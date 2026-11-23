//! The `charts` slice of the active Profile (F10): its net worth history
//! within the chosen time span, named by the Profile as in legacy, and the
//! per-tab chart of its newest 50 Snapshots. In a Group the net worth chart
//! holds one series per selected player instead, named by account (D15);
//! the per-tab chart stays the player's own. Pushed with the `net_worth`
//! slice and whenever the time span changes.

use domain::charts::{self, PlayerPoints, Point as DomainPoint, SnapshotTabs, TabValue};
use ipc::slices::{Charts, Point, Profile, Profiles, Series, Settings, TimeSpan};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::group::Groups;
use crate::scheduler::Scheduler;
use crate::state::{Pushing, Store};

/// The `charts` slice of `profile` of `account_id` for `time_span` at `now`.
pub async fn load(
    pool: &SqlitePool,
    account_id: &str,
    profile: &Profile,
    time_span: TimeSpan,
    now: i64,
) -> sqlx::Result<Charts> {
    let totals: Vec<(i64, f64)> = sqlx::query_as(
        "SELECT created_at, total_value FROM snapshots \
         WHERE account_id = ?1 AND profile_id = ?2",
    )
    .bind(account_id)
    .bind(&profile.id)
    .fetch_all(pool)
    .await?;
    let totals: Vec<DomainPoint> = totals
        .into_iter()
        .map(|(at, value)| DomainPoint { at, value })
        .collect();
    let rows: Vec<(String, i64, String, String, f64)> = sqlx::query_as(
        "SELECT s.id, s.created_at, t.tab_id, t.name, t.value \
         FROM (SELECT id, created_at FROM snapshots \
               WHERE account_id = ?1 AND profile_id = ?2 \
               ORDER BY created_at DESC, id DESC LIMIT ?3) s \
         JOIN snapshot_tabs t ON t.account_id = ?1 AND t.snapshot_id = s.id \
         ORDER BY s.created_at DESC, s.id DESC, t.position",
    )
    .bind(account_id)
    .bind(&profile.id)
    .bind(charts::TAB_CHART_SNAPSHOTS as i64)
    .fetch_all(pool)
    .await?;
    let mut snapshots: Vec<SnapshotTabs> = Vec::new();
    let mut last_id = None;
    for (id, at, tab_id, name, value) in rows {
        if last_id.as_ref() != Some(&id) {
            snapshots.push(SnapshotTabs {
                at,
                tabs: Vec::new(),
            });
            last_id = Some(id);
        }
        if let Some(snapshot) = snapshots.last_mut() {
            snapshot.tabs.push(TabValue {
                tab_id,
                name,
                value,
            });
        }
    }
    let history = charts::net_worth_history(&totals, domain_span(time_span), now);
    Ok(Charts {
        time_span,
        net_worth: vec![Series {
            name: profile.name.clone(),
            points: points(&history),
        }],
        tabs: charts::tab_series(&snapshots, &profile.stash_tab_ids)
            .into_iter()
            .map(|s| Series {
                name: s.name,
                points: points(&s.points),
            })
            .collect(),
    })
}

/// The `charts` slice of the active Profile of the signed-in account, from
/// the stored Profiles and settings, as at a start; empty without one.
pub async fn load_active(
    pool: &SqlitePool,
    account_id: Option<&str>,
    profiles: &Profiles,
    time_span: TimeSpan,
    now: i64,
) -> sqlx::Result<Charts> {
    let active = profiles
        .active_profile_id
        .as_ref()
        .and_then(|id| profiles.profiles.iter().find(|p| p.id == *id));
    match (account_id, active) {
        (Some(account_id), Some(profile)) => load(pool, account_id, profile, time_span, now).await,
        _ => Ok(Charts {
            time_span,
            ..Charts::default()
        }),
    }
}

/// Pushes the `charts` slice of the signed-in account's active Profile for
/// the time span setting; in a Group, with the selected players' net worth
/// series (D15). A failure is only logged.
pub async fn push<R: Runtime>(app: &AppHandle<R>) {
    let pushing = app.state::<Pushing<Charts>>();
    // From before the Profiles, the time span and the players are read.
    let _pushing = pushing.lock().await;
    let pool = app.state::<SqlitePool>();
    let store = app.state::<Store>();
    let profiles = store.read(Profiles::clone);
    let time_span = store.read(|s: &Settings| s.chart_time_span);
    let now = app.state::<Scheduler>().now();
    // In a Group: each player's name, whether selected and the account's
    // own, with the totals held (none for the account's own).
    let players = app.state::<Groups>().read_members(|m| {
        let totals: Vec<DomainPoint> = m
            .snapshots
            .iter()
            .map(|s| DomainPoint {
                at: s.created_at,
                value: s.total,
            })
            .collect();
        (m.account_name.clone(), m.selected, m.me, totals)
    });
    let loaded = async {
        let account_id: Option<String> = sqlx::query_scalar("SELECT account_id FROM login")
            .fetch_optional(&*pool)
            .await?;
        load_active(&pool, account_id.as_deref(), &profiles, time_span, now).await
    }
    .await;
    let loaded = loaded.map(|mut charts| {
        if let Some(players) = players {
            // The account's own history, as solo; filtering it by the span
            // again keeps it.
            let mine: Vec<DomainPoint> = charts
                .net_worth
                .first()
                .map(|s| {
                    s.points
                        .iter()
                        .map(|p| DomainPoint {
                            at: p.at,
                            value: p.value,
                        })
                        .collect()
                })
                .unwrap_or_default();
            let players: Vec<PlayerPoints<'_>> = players
                .iter()
                .map(|(name, selected, me, totals)| PlayerPoints {
                    name,
                    selected: *selected,
                    snapshots: if *me { &mine } else { totals },
                })
                .collect();
            charts.net_worth = charts::group_net_worth(&players, domain_span(time_span), now)
                .into_iter()
                .map(|s| Series {
                    name: s.name,
                    points: points(&s.points),
                })
                .collect();
        }
        charts
    });
    match loaded {
        Ok(charts) => {
            if let Err(error) = store.update::<Charts, _>(app, |c| *c = charts) {
                log::error!("cannot push the charts: {error}");
            }
        }
        Err(error) => log::error!("cannot load the charts: {error}"),
    }
}

const fn domain_span(span: TimeSpan) -> charts::TimeSpan {
    match span {
        TimeSpan::Day => charts::TimeSpan::Day,
        TimeSpan::Week => charts::TimeSpan::Week,
        TimeSpan::Month => charts::TimeSpan::Month,
        TimeSpan::All => charts::TimeSpan::All,
    }
}

fn points(points: &[DomainPoint]) -> Vec<Point> {
    points
        .iter()
        .map(|p| Point {
            at: p.at,
            value: p.value,
        })
        .collect()
}
