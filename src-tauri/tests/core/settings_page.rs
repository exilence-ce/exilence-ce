//! The Settings page (F17): setting ranges and Reset data.

use std::time::Duration;

use exilence_ce_ng_lib::reset::{self, Locations};
use exilence_ce_ng_lib::slot::OperationSlot;
use exilence_ce_ng_lib::state::Store;
use ipc::command::{SettingsPatch, UpdateSettings, UpdateSettingsArgs};
use ipc::slices::{Currency, LoginState, Session, Settings};
use ipc::{Field, IpcError, MessageKey};
use tauri::Manager;
use tauri::async_runtime::block_on;

use super::fake_backend::{FakeBackend, signed_in};
use super::{Fakes, Harness, settings_slice};

fn patch(harness: &Harness, patch: SettingsPatch) -> Result<(), IpcError> {
    harness.invoke::<UpdateSettings>(&UpdateSettingsArgs { patch })
}

fn out_of_range(field: Field) -> Result<(), IpcError> {
    Err(IpcError::Validation {
        field,
        key: MessageKey::ErrorSettingOutOfRange,
    })
}

/// The settings stored, but the crash marker (F20).
fn stored_keys(harness: &Harness) -> Vec<String> {
    block_on(
        sqlx::query_scalar("SELECT key FROM settings WHERE key != 'clean_exit' ORDER BY key")
            .fetch_all(&harness.pool()),
    )
    .unwrap()
}

#[test]
fn f17_settings_accept_their_bounds_and_refuse_values_outside() {
    let harness = Harness::new();
    let minutes = |m| SettingsPatch {
        auto_snapshot_minutes: Some(m),
        ..SettingsPatch::default()
    };
    let item = |t| SettingsPatch {
        price_threshold: Some(t),
        ..SettingsPatch::default()
    };
    let total = |t| SettingsPatch {
        total_price_threshold: Some(t),
        ..SettingsPatch::default()
    };
    let scale = |s| SettingsPatch {
        ui_scale: Some(s),
        ..SettingsPatch::default()
    };

    for refused in [minutes(4), minutes(1001)] {
        assert_eq!(
            patch(&harness, refused),
            out_of_range(Field::AutoSnapshotMinutes)
        );
    }
    for refused in [item(-0.01), item(100.01)] {
        assert_eq!(
            patch(&harness, refused),
            out_of_range(Field::PriceThreshold)
        );
    }
    for refused in [total(-0.01), total(5000.01)] {
        assert_eq!(
            patch(&harness, refused),
            out_of_range(Field::TotalPriceThreshold)
        );
    }
    for refused in [scale(40), scale(160), scale(55)] {
        assert_eq!(patch(&harness, refused), out_of_range(Field::UiScale));
    }
    // A refused patch stores nothing, not even its valid settings.
    let mixed = SettingsPatch {
        auto_snapshot: Some(true),
        ui_scale: Some(155),
        ..SettingsPatch::default()
    };
    assert_eq!(patch(&harness, mixed), out_of_range(Field::UiScale));
    assert_eq!(settings_slice(&harness), Settings::default());
    assert!(stored_keys(&harness).is_empty());

    for accepted in [minutes(5), minutes(1000), item(0.0), item(100.0)] {
        patch(&harness, accepted).unwrap();
    }
    for accepted in [total(0.0), total(5000.0), scale(50), scale(150)] {
        patch(&harness, accepted).unwrap();
    }
    patch(
        &harness,
        SettingsPatch {
            auto_snapshot: Some(true),
            low_confidence_pricing: Some(true),
            hardware_acceleration: Some(false),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    let expected = Settings {
        auto_snapshot: true,
        auto_snapshot_minutes: 1000,
        low_confidence_pricing: true,
        price_threshold: 100.0,
        total_price_threshold: 5000.0,
        ui_scale: 150,
        hardware_acceleration: false,
        ..Settings::default()
    };
    assert_eq!(settings_slice(&harness), expected);
    let harness = harness.restart();
    assert_eq!(settings_slice(&harness), expected);
}

#[test]
fn f9_the_chosen_currency_is_remembered() {
    let harness = Harness::new();
    assert_eq!(
        settings_slice(&harness).currency,
        Currency::Chaos,
        "default"
    );
    patch(
        &harness,
        SettingsPatch {
            currency: Some(Currency::Divine),
            ..SettingsPatch::default()
        },
    )
    .unwrap();
    assert_eq!(settings_slice(&harness).currency, Currency::Divine);
    let harness = harness.restart();
    assert_eq!(settings_slice(&harness).currency, Currency::Divine);
}

#[test]
fn f17_stored_out_of_range_settings_load_as_their_default() {
    let harness = Harness::new();
    block_on(
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES \
             ('ui_scale', '30'), ('auto_snapshot_minutes', '2000'), ('price_threshold', '7.5')",
        )
        .execute(&harness.pool()),
    )
    .unwrap();
    let harness = harness.restart();
    assert_eq!(
        settings_slice(&harness),
        Settings {
            price_threshold: 7.5,
            ..Settings::default()
        }
    );
}

#[test]
fn f17_reset_data_cancels_the_holder_signs_out_and_deletes_ng_data_but_logs() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login().backend(&backend.url).build();
    let dir = harness.dir.path().to_owned();
    let logs = dir.join("logs").join("exilence-ce-ng.log");
    std::fs::create_dir_all(logs.parent().unwrap()).unwrap();
    std::fs::write(&logs, b"line").unwrap();

    // A running Snapshot: holds the slot until asked to cancel.
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let handle = harness.app.handle().clone();
    let snapshot = tauri::async_runtime::spawn(async move {
        let slot = handle.state::<OperationSlot>();
        let _held = slot.hold().await;
        let mut cancellation = slot.cancellation();
        held_tx.send(()).unwrap();
        cancellation.wait_for(|c| *c).await.unwrap();
    });
    held_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    block_on(reset::prepare(harness.app.handle(), &dir)).unwrap();
    block_on(snapshot).unwrap();
    assert_eq!(backend.sign_outs().len(), 1, "SignOut, best effort");
    assert!(dir.join(reset::PENDING_FILE).exists());

    // The restart.
    let Harness {
        app, window, dir, ..
    } = harness;
    drop(window);
    drop(app);
    let locations = Locations {
        roots: vec![dir.path().to_owned()],
        keep: vec![dir.path().join("logs")],
    };
    reset::finish_pending(dir.path(), &locations).unwrap();
    assert!(logs.exists());
    let harness = Harness::open(dir, Fakes::default());
    let session = harness.app.state::<Store>().read(Session::clone);
    assert_eq!(session.login, LoginState::SignedOut);
    assert_eq!(settings_slice(&harness), Settings::default());
}

/// Reset data closes the database before the restart also while background
/// work is in the middle of a query: the query ends first, and once
/// `prepare` returns no file of the data directory is open, so the next
/// start can delete it (Windows refuses to delete an open file).
#[cfg(target_os = "linux")]
#[test]
fn f17_reset_data_leaves_no_data_file_open_while_queries_run() {
    const QUERIES: usize = 2;
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login().backend(&backend.url).build();
    let dir = harness.dir.path().to_owned();
    let pool = harness.pool();
    // Each holds a connection and, once the pool is closing, runs a query
    // that takes a while.
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let queries: Vec<_> = (0..QUERIES)
        .map(|_| {
            let pool = pool.clone();
            let acquired = acquired_tx.clone();
            tauri::async_runtime::spawn(async move {
                let mut connection = pool.acquire().await?;
                let closing = pool.close_event();
                acquired.send(()).unwrap();
                closing.await;
                sqlx::query_scalar::<_, i64>(
                    "WITH RECURSIVE n(i) AS \
                     (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 500000) \
                     SELECT count(*) FROM n",
                )
                .fetch_one(&mut *connection)
                .await
            })
        })
        .collect();
    for _ in 0..QUERIES {
        acquired_rx.recv_timeout(super::WAIT).unwrap();
    }

    block_on(reset::prepare(harness.app.handle(), &dir)).unwrap();

    let open: Vec<_> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok())
        .filter(|file| file.starts_with(&dir))
        .collect();
    assert_eq!(open, Vec::<std::path::PathBuf>::new());
    for query in queries {
        assert_eq!(block_on(query).unwrap().unwrap(), 500_000);
    }
}
