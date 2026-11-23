//! The `charts` slice (F10): pushed with the active Profile, again when the
//! time span changes, and loaded at a start with the stored time span.

use exilence_ce_ng_lib::profiles;
use exilence_ce_ng_lib::state::Store;
use ipc::command::{SettingsPatch, UpdateSettings, UpdateSettingsArgs};
use ipc::slices::{Charts, Point, Series, TimeSpan};
use sqlx::SqlitePool;
use tauri::Manager;
use tauri::async_runtime::block_on;

use super::Harness;
use super::fake_backend::ACCOUNT_ID;
use super::fake_ggg::START;

const PROFILE_ID: &str = "01a1cd36-0000-7000-8000-0000000000cc";
const HOUR: i64 = 60 * 60 * 1000;

fn profile() -> wire::v1::Profile {
    wire::v1::Profile {
        id: PROFILE_ID.to_owned(),
        name: "profile 1".to_owned(),
        league_id: "Settlers".to_owned(),
        price_league_id: "Settlers".to_owned(),
        stash_tab_ids: vec!["t2".to_owned(), "t1".to_owned()],
        character_name: Some("Hero".to_owned()),
        include_equipment: false,
        include_inventory: true,
    }
}

/// Stores a Snapshot taken at `at` with `tabs` (id, name, value) in order.
fn insert(pool: &SqlitePool, id: &str, at: i64, tabs: &[(&str, &str, f64)]) {
    block_on(async {
        let total: f64 = tabs.iter().map(|t| t.2).sum();
        sqlx::query(
            "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(ACCOUNT_ID)
        .bind(id)
        .bind(PROFILE_ID)
        .bind(at)
        .bind(total)
        .execute(pool)
        .await
        .unwrap();
        for (position, (tab_id, name, value)) in tabs.iter().enumerate() {
            sqlx::query(
                "INSERT INTO snapshot_tabs (account_id, snapshot_id, position, tab_id, name, value) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(ACCOUNT_ID)
            .bind(id)
            .bind(position as i64)
            .bind(tab_id)
            .bind(name)
            .bind(value)
            .execute(pool)
            .await
            .unwrap();
        }
    });
}

fn charts(harness: &Harness) -> Charts {
    harness.app.state::<Store>().read(Charts::clone)
}

fn point(at: i64, value: f64) -> Point {
    Point { at, value }
}

#[test]
fn f10_charts_follow_the_active_profile_and_the_remembered_time_span() {
    let harness = Harness::with_login()
        .profiles(&[profile()], PROFILE_ID)
        .build();
    let pool = harness.pool();
    let two_days_ago = START - 48 * HOUR;
    let an_hour_ago = START - HOUR;
    insert(
        &pool,
        "s1",
        two_days_ago,
        &[
            ("t1", "Currency", 10.0),
            ("t2", "Maps", 5.0),
            ("", "Character", 1.0),
        ],
    );
    insert(
        &pool,
        "s2",
        an_hour_ago,
        &[
            ("t1", "Loot", 20.0),
            ("t2", "Maps", 6.0),
            ("", "Character", 1.0),
        ],
    );

    block_on(profiles::push(harness.app.handle()));

    let all = charts(&harness);
    assert_eq!(all.time_span, TimeSpan::All);
    assert_eq!(
        all.net_worth,
        [Series {
            name: "profile 1".to_owned(),
            points: vec![point(two_days_ago, 16.0), point(an_hour_ago, 27.0)],
        }]
    );
    assert_eq!(
        all.tabs,
        [
            Series {
                name: "Maps".to_owned(),
                points: vec![point(two_days_ago, 5.0), point(an_hour_ago, 6.0)],
            },
            Series {
                name: "Loot".to_owned(),
                points: vec![point(two_days_ago, 10.0), point(an_hour_ago, 20.0)],
            },
        ],
        "selected tabs in selection order, no Character line"
    );

    harness
        .invoke::<UpdateSettings>(&UpdateSettingsArgs {
            patch: SettingsPatch {
                chart_time_span: Some(TimeSpan::Day),
                ..SettingsPatch::default()
            },
        })
        .unwrap();

    let day = charts(&harness);
    assert_eq!(day.time_span, TimeSpan::Day);
    assert_eq!(day.net_worth[0].points, [point(an_hour_ago, 27.0)]);
    assert_eq!(
        day.tabs, all.tabs,
        "the per-tab chart ignores the time span"
    );

    let harness = harness.restart();
    assert_eq!(charts(&harness), day, "the stored time span at a start");
}
