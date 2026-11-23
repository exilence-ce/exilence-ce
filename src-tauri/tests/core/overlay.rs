//! The Net worth overlay (F14): its window, and the slices it shows reaching
//! it after Snapshots, on currency change and when income changes (D12).

use std::sync::mpsc;
use std::time::Duration;

use exilence_ce_ng_lib::shell::OVERLAY_WINDOW;
use exilence_ce_ng_lib::state;
use ipc::command::{
    CloseOverlay, NoArgs, OpenOverlay, ResetIncome, SettingsPatch, UpdateSettings,
    UpdateSettingsArgs,
};
use ipc::slices::{Currency, NetWorth, Settings};
use ipc::{Event, SliceName};
use tauri::test::MockRuntime;
use tauri::{Manager, WebviewWindow};

use super::fake_ggg::START;
use super::snapshot::{Setup, ggg_with_stash, insert_snapshot};

const MINUTE: i64 = 60_000;

/// Collects the events the overlay window receives under `slice`'s name.
fn overlay_events(overlay: &WebviewWindow<MockRuntime>, slice: SliceName) -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    state::listen(overlay, slice.event_name(), move |event| {
        // The test may have ended.
        let _ = tx.send(serde_json::from_str(event.payload()).unwrap());
    });
    rx
}

/// The first event from `events` that `wanted` accepts.
fn next_matching(events: &mpsc::Receiver<Event>, wanted: impl Fn(&Event) -> bool) -> Event {
    loop {
        let event = events
            .recv_timeout(Duration::from_secs(10))
            .expect("the overlay receives the event");
        if wanted(&event) {
            return event;
        }
    }
}

fn overlay(setup: &Setup) -> Option<WebviewWindow<MockRuntime>> {
    setup.harness.app.get_webview_window(OVERLAY_WINDOW)
}

#[test]
fn f14_d12_overlay_values_update_after_snapshots_on_currency_change_and_income_reset() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let clock = setup.harness.fakes.clock.clone();
    insert_snapshot(&setup, "after-start", START + MINUTE / 2, 100.0);

    setup.harness.invoke::<OpenOverlay>(&NoArgs {}).unwrap();
    let window = overlay(&setup).expect("the overlay is open");
    // Opening it again keeps the one window.
    setup.harness.invoke::<OpenOverlay>(&NoArgs {}).unwrap();
    assert_eq!(setup.harness.app.webview_windows().len(), 2);
    let net_worth = overlay_events(&window, SliceName::NetWorth);
    let settings = overlay_events(&window, SliceName::Settings);

    // After a Snapshot: the new total and income.
    clock.advance(MINUTE);
    setup.take_and_wait();
    let pushed = next_matching(
        &net_worth,
        |e| matches!(e, Event::NetWorth(n) if n.income != 0.0),
    );
    let Event::NetWorth(pushed) = pushed else {
        unreachable!()
    };
    assert_eq!((pushed.total, pushed.income), (329.0, 229.0));
    assert_eq!(pushed, setup.read(NetWorth::clone));

    // When income changes without a Snapshot (D12).
    setup.harness.invoke::<ResetIncome>(&NoArgs {}).unwrap();
    let Event::NetWorth(reset) = next_matching(&net_worth, |_| true) else {
        unreachable!()
    };
    assert_eq!((reset.total, reset.income), (329.0, 0.0));

    // On currency change.
    setup
        .harness
        .invoke::<UpdateSettings>(&UpdateSettingsArgs {
            patch: SettingsPatch {
                currency: Some(Currency::Divine),
                ..SettingsPatch::default()
            },
        })
        .unwrap();
    let Event::Settings(changed) = next_matching(&settings, |_| true) else {
        unreachable!()
    };
    assert_eq!(changed.currency, Currency::Divine);
    assert_eq!(changed, setup.read(Settings::clone));

    // The mock runtime never destroys a window, so only the request shows.
    setup.harness.invoke::<CloseOverlay>(&NoArgs {}).unwrap();
}
