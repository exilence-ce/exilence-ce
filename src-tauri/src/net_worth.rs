//! The `net_worth` slice of the active Profile (F8): the newest Snapshot's
//! total, its change against the one before, the sparkline of the newest
//! 10, the income per hour, the Snapshot count and when the newest was
//! taken. Pushed after a Snapshot, on Reset income and whenever the active
//! Profile changes. In a Group the slice combines the selected players'
//! active Profiles instead (F13, D15): the account's own Snapshots from the
//! database, the other members' as the Group stream brought them; a Group
//! has no Reset income.

use std::collections::HashMap;
use std::sync::Mutex;

use domain::group::{self as group_math, PlayerTotals};
use domain::net_worth::{self as math, INCOME_WINDOW_MS, SPARKLINE_SNAPSHOTS, SnapshotTotal};
use ipc::slices::{NetWorth, Profiles};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::group::{Groups, Selection};
use crate::scheduler::Scheduler;
use crate::state::{Pushing, Store};

/// The income of each Profile in this run (F8): kept in memory only, so
/// after a restart every income window starts at the start and shows 0.
pub struct Income {
    started_at: i64,
    windows: Mutex<HashMap<String, IncomeWindow>>,
}

/// A Profile's last Reset income and its income after its last Snapshot.
#[derive(Clone, Copy)]
struct IncomeWindow {
    reset_at: i64,
    income: f64,
}

impl Income {
    /// Every income window starts at `started_at` (Unix ms).
    pub fn new(started_at: i64) -> Self {
        Self {
            started_at,
            windows: Mutex::default(),
        }
    }

    fn window(&self, profile_id: &str) -> IncomeWindow {
        self.lock()
            .get(profile_id)
            .copied()
            .unwrap_or(IncomeWindow {
                reset_at: self.started_at,
                income: 0.0,
            })
    }

    fn set(&self, profile_id: &str, window: IncomeWindow) {
        self.lock().insert(profile_id.to_owned(), window);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, IncomeWindow>> {
        self.windows.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The `net_worth` slice of `profile_id` of `account_id`, with `income`.
pub async fn load(
    pool: &SqlitePool,
    account_id: &str,
    profile_id: &str,
    income: f64,
) -> sqlx::Result<NetWorth> {
    let newest = totals(pool, account_id, profile_id, None).await?;
    Ok(NetWorth {
        total: newest.first().map_or(0.0, |s| s.total),
        change: math::change(&newest),
        sparkline: math::sparkline(&newest),
        income,
        snapshot_count: count(pool, account_id, profile_id).await?,
        last_snapshot_at: newest.first().map(|s| s.created_at),
        ..NetWorth::default()
    })
}

/// How many Snapshots the Profile has.
async fn count(pool: &SqlitePool, account_id: &str, profile_id: &str) -> sqlx::Result<u32> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM snapshots WHERE account_id = ?1 AND profile_id = ?2",
    )
    .bind(account_id)
    .bind(profile_id)
    .fetch_one(pool)
    .await?;
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

/// The Group-combined slice of the `selection` at `now`, the signed-in
/// account counting with its `active` Profile (F13, D15).
async fn combined(
    pool: &SqlitePool,
    active: Option<(String, String)>,
    selection: Selection,
    now: i64,
) -> sqlx::Result<NetWorth> {
    let mut mine = None;
    if selection.me
        && let Some((account_id, profile_id)) = &active
    {
        // What the combination reads: the newest of the sparkline, the one
        // before them and the last hour's.
        let rows: Vec<(i64, f64)> = sqlx::query_as(
            "SELECT created_at, total_value FROM ( \
                 SELECT created_at, total_value, id, \
                        row_number() OVER (ORDER BY created_at DESC, id DESC) AS rn \
                 FROM snapshots WHERE account_id = ?1 AND profile_id = ?2) \
             WHERE rn <= ?3 OR created_at > ?4 \
             ORDER BY created_at DESC, id DESC",
        )
        .bind(account_id)
        .bind(profile_id)
        .bind(SPARKLINE_SNAPSHOTS as i64 + 1)
        .bind(now - INCOME_WINDOW_MS)
        .fetch_all(pool)
        .await?;
        let newest_first: Vec<SnapshotTotal> = rows
            .into_iter()
            .map(|(created_at, total)| SnapshotTotal { created_at, total })
            .collect();
        mine = Some((newest_first, count(pool, account_id, profile_id).await?));
    }
    let players: Vec<PlayerTotals<'_>> = mine
        .iter()
        .map(|(newest_first, count)| PlayerTotals {
            newest_first,
            count: *count,
        })
        .chain(selection.others.iter().map(|newest_first| PlayerTotals {
            newest_first,
            count: u32::try_from(newest_first.len()).unwrap_or(u32::MAX),
        }))
        .collect();
    let combined = group_math::combine(&players, now);
    Ok(NetWorth {
        total: combined.total,
        change: combined.change,
        sparkline: combined.sparkline,
        income: combined.income,
        snapshot_count: combined.snapshot_count,
        last_snapshot_at: combined.last_snapshot_at,
        group_combined: true,
    })
}

/// The totals of the Profile's Snapshots, newest first: those taken after
/// `after`, or the newest [`SPARKLINE_SNAPSHOTS`] without it.
async fn totals(
    pool: &SqlitePool,
    account_id: &str,
    profile_id: &str,
    after: Option<i64>,
) -> sqlx::Result<Vec<SnapshotTotal>> {
    let rows: Vec<(i64, f64)> = sqlx::query_as(
        "SELECT created_at, total_value FROM snapshots \
         WHERE account_id = ?1 AND profile_id = ?2 AND created_at > ?3 \
         ORDER BY created_at DESC, id DESC LIMIT ?4",
    )
    .bind(account_id)
    .bind(profile_id)
    .bind(after.unwrap_or(i64::MIN))
    .bind(if after.is_some() {
        -1
    } else {
        SPARKLINE_SNAPSHOTS as i64
    })
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(created_at, total)| SnapshotTotal { created_at, total })
        .collect())
}

/// The signed-in account and its active Profile, if both exist.
async fn active<R: Runtime>(app: &AppHandle<R>) -> sqlx::Result<Option<(String, String)>> {
    let active = app
        .state::<Store>()
        .read(|p: &Profiles| p.active_profile_id.clone());
    let Some(profile_id) = active else {
        return Ok(None);
    };
    let account_id: Option<String> = sqlx::query_scalar("SELECT account_id FROM login")
        .fetch_optional(&*app.state::<SqlitePool>())
        .await?;
    Ok(account_id.map(|account_id| (account_id, profile_id)))
}

/// Pushes the `net_worth` slice of the signed-in account's active Profile
/// with the income after its last Snapshot, empty without one; in a Group,
/// the selected players combined. A failure is only logged.
pub async fn push<R: Runtime>(app: &AppHandle<R>) {
    let pushing = app.state::<Pushing<NetWorth>>();
    // From before the active Profile and the player selection are read: a
    // push that read the selection before `set_player_selected` changed it
    // never lands after the push of that change.
    let _pushing = pushing.lock().await;
    let pool = app.state::<SqlitePool>();
    let loaded = async {
        let active = active(app).await?;
        if let Some(selection) = app.state::<Groups>().selection() {
            let now = app.state::<Scheduler>().now();
            return combined(&pool, active, selection, now).await;
        }
        match active {
            Some((account_id, profile_id)) => {
                let income = app.state::<Income>().window(&profile_id).income;
                load(&pool, &account_id, &profile_id, income).await
            }
            None => Ok(NetWorth::default()),
        }
    }
    .await;
    match loaded {
        Ok(net_worth) => {
            if let Err(error) = app
                .state::<Store>()
                .update::<NetWorth, _>(app, |n| *n = net_worth)
            {
                log::error!("cannot push the net worth: {error}");
            }
        }
        Err(error) => log::error!("cannot load the net worth: {error}"),
    }
}

/// After a Snapshot: recalculates the active Profile's income over its
/// window, then pushes the slice. A failure is only logged.
pub async fn snapshot_taken<R: Runtime>(app: &AppHandle<R>) {
    let calculated = async {
        let pushing = app.state::<Pushing<NetWorth>>();
        // A reset meanwhile is not undone by writing back the window read
        // before it.
        let _pushing = pushing.lock().await;
        let Some((account_id, profile_id)) = active(app).await? else {
            return Ok(());
        };
        let now = app.state::<Scheduler>().now();
        let income = app.state::<Income>();
        let IncomeWindow { reset_at, .. } = income.window(&profile_id);
        let since = math::income_window_start(now, reset_at);
        let pool = app.state::<SqlitePool>();
        let in_window = totals(&pool, &account_id, &profile_id, Some(since)).await?;
        let window = IncomeWindow {
            reset_at,
            income: math::income(&in_window, now, reset_at),
        };
        income.set(&profile_id, window);
        Ok::<_, sqlx::Error>(())
    }
    .await;
    if let Err(error) = calculated {
        log::error!("cannot calculate the income: {error}");
    }
    push(app).await;
}

/// `reset_income`: sets the active Profile's income to 0 and starts its
/// window now (F8).
pub async fn reset_income<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let pushing = app.state::<Pushing<NetWorth>>();
    // A push that read the income before the reset never lands after it.
    let _pushing = pushing.lock().await;
    let Some(profile_id) = app
        .state::<Store>()
        .read(|p: &Profiles| p.active_profile_id.clone())
    else {
        return Ok(());
    };
    let window = IncomeWindow {
        reset_at: app.state::<Scheduler>().now(),
        income: 0.0,
    };
    app.state::<Income>().set(&profile_id, window);
    app.state::<Store>()
        .update::<NetWorth, _>(app, |n| n.income = 0.0)
}
