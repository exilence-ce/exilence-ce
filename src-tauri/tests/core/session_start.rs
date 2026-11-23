//! The session start (F1, D25, D26), token refresh (D6), the operation slot
//! and the scheduler, against the fake backend and a `wiremock` GGG on a
//! fake clock.

use std::sync::mpsc;
use std::time::Duration;

use exilence_ce_ng_lib::auth;
use exilence_ce_ng_lib::ggg::{self, Ggg, Request};
use exilence_ce_ng_lib::scheduler::{self, Deadline, Scheduler};
use exilence_ce_ng_lib::session_start;
use exilence_ce_ng_lib::slot::OperationSlot;
use exilence_ce_ng_lib::state::{self, Store};
use ipc::slices::{LoginState, NotificationKind, Notifications, Profiles, RateLimit, Session};
use ipc::{Event, IpcError, MessageKey, SliceName, TOAST_EVENT};
use tauri::Manager;
use tauri::async_runtime::block_on;
use tonic::{Code, Status};
use wire::v1::ErrorReason;
use wiremock::matchers::{any, header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::fake_backend::{
    ACCESS_TOKEN, ACCESS_TOKEN_EXPIRES_AT, FakeBackend, SESSION_TOKEN, refreshed, refused, session,
    signed_in,
};
use super::fake_ggg::{FakeClock, START};
use super::{Harness, NO_BACKEND, WAIT, mock_server, next_toast, poll, received};

const DAY: i64 = 24 * 60 * 60 * 1000;
const FRESH_TOKEN: &str = "ggg-access-token-fresh";

impl Harness {
    /// With the backend at `backend_url`, GGG at `ggg`, and a stored Login
    /// whose GGG token expires at `ggg_expires_at`.
    pub(super) fn launched(backend_url: &str, ggg: &MockServer, ggg_expires_at: i64) -> Self {
        Self::with_login()
            .backend(backend_url)
            .ggg(&ggg.uri())
            .ggg_expires_at(ggg_expires_at)
            .build()
    }

    pub(super) fn clock(&self) -> &FakeClock {
        &self.fakes.clock
    }

    /// Starts the session as at a launch; returns once the backend part
    /// has run, while the GGG part may still run.
    pub(super) fn start_session(&self) {
        block_on(session_start::run(self.app.handle()));
    }

    pub(super) fn tick(&self) -> Vec<Deadline> {
        scheduler::tick(self.app.handle())
    }

    pub(super) fn due(&self, deadline: Deadline) -> Option<i64> {
        self.app.state::<Scheduler>().due_at(deadline)
    }

    pub(super) fn session_slice(&self) -> Session {
        self.app.state::<Store>().read(Session::clone)
    }

    pub(super) fn notifications(&self) -> Notifications {
        self.app.state::<Store>().read(Notifications::clone)
    }

    /// The stored GGG token and its expiry, if a Login is stored.
    fn stored_token(&self) -> Option<(String, i64)> {
        block_on(
            sqlx::query_as("SELECT ggg_access_token, ggg_expires_at FROM login")
                .fetch_optional(&self.pool()),
        )
        .unwrap()
    }
}

fn characters() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "characters": [
        { "id": "c1", "name": "Hero", "class": "Witch", "league": "Settlers", "level": 90 },
    ]}))
}

/// GGG answering `/character` and the `Settlers` stash tab list for
/// `token` and 401 to anything else.
pub(super) fn ggg_accepting(token: &str) -> MockServer {
    block_on(async {
        let server = mock_server().await;
        Mock::given(path("/character"))
            .and(header("authorization", format!("Bearer {token}").as_str()))
            .respond_with(characters())
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(path("/stash/Settlers"))
            .and(header("authorization", format!("Bearer {token}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stashes": [{ "id": "t1", "name": "Currency", "index": 0 }],
            })))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        server
    })
}

pub(super) fn logins(harness: &Harness) -> i64 {
    block_on(sqlx::query_scalar("SELECT count(*) FROM login").fetch_one(&harness.pool())).unwrap()
}

#[test]
fn f1_a_failed_ggg_load_shows_a_toast_and_retries_after_45_s() {
    let backend = FakeBackend::start(signed_in());
    let ggg = block_on(async {
        let server = mock_server().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(any())
            .respond_with(characters())
            .mount(&server)
            .await;
        server
    });
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    let toasts = harness.events(TOAST_EVENT);

    harness.start_session();

    let toast = next_toast(&toasts);
    assert_eq!(toast.kind, NotificationKind::Error);
    assert_eq!(
        toast.message.key,
        MessageKey::NotificationSessionStartFailed
    );
    assert_eq!(
        poll(|| harness.due(Deadline::GggSessionRetry)),
        START + 45_000
    );
    harness.clock().advance(44_999);
    assert!(!harness.tick().contains(&Deadline::GggSessionRetry));
    harness.clock().advance(1);
    assert!(harness.tick().contains(&Deadline::GggSessionRetry));
    assert_eq!(harness.wait_for_characters(), ["Settlers"]);
    harness.wait_for_ready();
    assert_eq!(received(&ggg, "/character"), 2);
    assert_eq!(harness.due(Deadline::GggSessionRetry), None);
    assert!(toasts.try_recv().is_err(), "one toast per failure");
}

#[test]
fn d6_a_token_expiring_within_7_days_is_refreshed_before_the_session_starts() {
    let backend = FakeBackend::start(signed_in());
    backend.script_refreshes([refreshed(FRESH_TOKEN, START + 28 * DAY)]);
    let ggg = ggg_accepting(FRESH_TOKEN);
    let harness = Harness::launched(&backend.url, &ggg, START + 6 * DAY);

    harness.start_session();

    harness.wait_for_characters();
    assert_eq!(
        received(&ggg, "/character"),
        1,
        "GGG saw only the refreshed token"
    );
    assert_eq!(
        backend.calls.lock().unwrap().refreshes,
        [Some(format!("Bearer {SESSION_TOKEN}"))]
    );
    assert_eq!(
        harness.stored_token(),
        Some((FRESH_TOKEN.to_owned(), START + 28 * DAY))
    );

    // 8 days left: no refresh.
    let backend = FakeBackend::start(signed_in());
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = Harness::launched(&backend.url, &ggg, START + 8 * DAY);
    harness.start_session();
    harness.wait_for_characters();
    assert_eq!(backend.refresh_count(), 0);
}

#[test]
fn d6_a_401_refreshes_the_token_once_and_sends_the_request_again() {
    let backend = FakeBackend::start(signed_in());
    backend.script_refreshes([refreshed(FRESH_TOKEN, START + 28 * DAY)]);
    let ggg = ggg_accepting(FRESH_TOKEN);
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    let toasts = harness.events(TOAST_EVENT);

    let characters = block_on(harness.app.state::<Ggg>().characters(harness.app.handle())).unwrap();

    assert_eq!(characters.len(), 1);
    assert_eq!(received(&ggg, "/character"), 2);
    assert_eq!(backend.refresh_count(), 1);
    assert_eq!(harness.session_slice().login, LoginState::SignedIn);
    assert!(toasts.try_recv().is_err());
}

/// Runs the session start's GGG part to its end against a GGG refusing the
/// stored token, with the refresh answered by `refresh`; checks that the
/// user was sent to login with "Session expired, please login again" and
/// nothing retries. Returns the GGG requests sent.
fn sent_to_login_after_refresh(
    refresh: Result<wire::v1::RefreshGggTokenResponse, Status>,
) -> usize {
    let backend = FakeBackend::start(signed_in());
    backend.script_refreshes([refresh]);
    let ggg = ggg_accepting("a token GGG would accept");
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    let toasts = harness.events(TOAST_EVENT);

    block_on(session_start::retry_ggg(harness.app.handle()));

    let toast = next_toast(&toasts);
    assert_eq!(toast.kind, NotificationKind::Error);
    assert_eq!(toast.message.key, MessageKey::ErrorSessionExpired);
    let session = harness.session_slice();
    assert_eq!(session.login, LoginState::SignedOut);
    assert_eq!(session.account_name, None);
    assert_eq!(logins(&harness), 0);
    assert!(toasts.try_recv().is_err(), "no session start toast");
    assert_eq!(harness.due(Deadline::GggSessionRetry), None, "no retry");
    assert_eq!(backend.refresh_count(), 1);
    received(&ggg, "/character")
}

#[test]
fn d6_a_second_401_after_the_refresh_sends_to_login() {
    let requests = sent_to_login_after_refresh(refreshed(FRESH_TOKEN, START + 28 * DAY));
    assert_eq!(requests, 2, "sent once more after the refresh, no more");
}

#[test]
fn d6_a_refused_refresh_sends_to_login() {
    let requests = sent_to_login_after_refresh(Err(refused(
        Code::FailedPrecondition,
        ErrorReason::GggRefreshFailed,
    )));
    assert_eq!(requests, 1, "not sent again");
}

#[test]
fn d6_one_refresh_at_a_time() {
    let backend = FakeBackend::start(signed_in());
    backend.script_refreshes([refreshed(FRESH_TOKEN, START + 28 * DAY)]);
    backend.script.lock().unwrap().refresh_delay = Duration::from_millis(300);
    let harness = Harness::launched(&backend.url, &ggg_accepting(FRESH_TOKEN), START + DAY);
    let refresh = || {
        let app = harness.app.handle().clone();
        tauri::async_runtime::spawn(async move { auth::refresh_ggg_token(&app).await })
    };
    let (first, second) = (refresh(), refresh());
    let (first, second) = block_on(async { (first.await.unwrap(), second.await.unwrap()) });

    assert_eq!((first, second), (Ok(()), Ok(())));
    assert_eq!(backend.refresh_count(), 1);
}

#[test]
fn d6_an_unreachable_backend_keeps_a_token_that_is_still_valid() {
    let ggg = ggg_accepting(ACCESS_TOKEN);
    // A stored Profile: the GGG part does not wait for `StartSession`.
    let harness = Harness::with_login()
        .backend(NO_BACKEND)
        .ggg(&ggg.uri())
        .ggg_expires_at(START + DAY)
        .profiles(&profiles(&["p1"]), "p1")
        .build();
    let toasts = harness.events(TOAST_EVENT);

    harness.start_session();

    harness.wait_for_ready();
    assert_eq!(
        harness.stored_token(),
        Some((ACCESS_TOKEN.to_owned(), START + DAY))
    );
    assert!(toasts.try_recv().is_err());
}

#[test]
fn d6_an_expired_token_that_cannot_be_refreshed_fails_with_a_toast_and_stays_signed_in() {
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = Harness::launched(NO_BACKEND, &ggg, START - 1);
    let toasts = harness.events(TOAST_EVENT);

    harness.start_session();

    let toast = next_toast(&toasts);
    assert_eq!(
        toast.message.key,
        MessageKey::NotificationSessionStartFailed
    );
    assert_eq!(
        toast.description.map(|d| d.key),
        Some(MessageKey::ErrorGggTokenExpired)
    );
    assert_eq!(harness.session_slice().login, LoginState::SignedIn);
    assert_eq!(received(&ggg, "/character"), 0);
    assert_eq!(
        poll(|| harness.due(Deadline::GggSessionRetry)),
        START + 45_000
    );
}

#[test]
fn d25_d26_nothing_is_visible_while_the_backend_is_down_solo_and_the_retries_back_off() {
    let backend = FakeBackend::start(signed_in());
    let down = || Err(Status::unavailable("backend down"));
    backend.script_start_sessions([down(), down(), down(), session(profiles(&["p1"]), "p1")]);
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    let toasts = harness.events(TOAST_EVENT);
    let sessions = harness.events(SliceName::Session.event_name());
    let notifications = harness.events(SliceName::Notifications.event_name());

    harness.start_session();
    harness.wait_for_characters();

    // min(60 s, 5 s × 2ⁿ) × r after the n-th failed attempt: `wait` is due
    // now; `r` is drawn for the next failure.
    let mut now = START;
    for (attempt, (wait, r)) in [(5_000, 0.5), (5_000, 1.0), (20_000, 1.0)]
        .into_iter()
        .enumerate()
    {
        assert_eq!(backend.start_session_count(), attempt + 1);
        assert_eq!(
            poll(|| harness.due(Deadline::BackendSessionRetry)),
            now + wait
        );
        harness.fakes.jitter.set(r);
        harness.clock().advance(wait);
        now += wait;
        assert_eq!(harness.tick(), [Deadline::BackendSessionRetry]);
        poll(|| (backend.start_session_count() == attempt + 2).then_some(()));
    }
    let profiles = poll(|| {
        Some(harness.app.state::<Store>().read(Profiles::clone)).filter(|p| !p.profiles.is_empty())
    });
    assert_eq!(profiles.active_profile_id.as_deref(), Some("p1"));
    assert_eq!(harness.due(Deadline::BackendSessionRetry), None);
    harness.wait_for_ready();

    assert!(toasts.try_recv().is_err(), "no toast");
    assert!(notifications.try_recv().is_err(), "no notification");
    let session = harness.session_slice();
    assert!(!session.offline && !session.superseded, "{session:?}");
    assert!(
        sessions
            .try_iter()
            .all(|e| matches!(e, Event::Session(s) if !s.offline)),
        "never offline"
    );
}

pub(super) fn profiles(ids: &[&str]) -> Vec<wire::v1::Profile> {
    ids.iter()
        .map(|id| wire::v1::Profile {
            id: (*id).to_owned(),
            name: format!("profile {id}"),
            league_id: "Settlers".to_owned(),
            price_league_id: "Settlers".to_owned(),
            stash_tab_ids: vec!["t1".to_owned(), "t2".to_owned()],
            character_name: None,
            include_equipment: false,
            include_inventory: true,
        })
        .collect()
}

#[test]
fn f2_f3_without_a_local_profile_the_session_is_ready_once_start_session_brought_one() {
    let backend = FakeBackend::start(signed_in());
    let down = || Err(Status::unavailable("backend down"));
    backend.script_start_sessions([down(), session(profiles(&["p1"]), "p1")]);
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    let sessions = harness.events(SliceName::Session.event_name());

    harness.start_session();
    harness.wait_for_characters();
    let due = poll(|| harness.due(Deadline::BackendSessionRetry));
    harness.clock().advance(due - START);
    assert_eq!(harness.tick(), [Deadline::BackendSessionRetry]);

    // Ready only with the stash tab list of the Profile `StartSession`
    // brought (F3), loaded by the GGG part once.
    let ready = loop {
        match sessions.recv_timeout(WAIT).expect("a session event") {
            Event::Session(session) if session.ready => break session,
            _ => {}
        }
    };
    assert!(ready.league_has_tabs, "{ready:?}");
    assert_eq!(received(&ggg, "/stash/Settlers"), 1);
}

#[test]
fn d25_superseded_stays_offline_until_restarted() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([Err(refused(Code::Unauthenticated, ErrorReason::Superseded))]);
    let ggg = ggg_accepting(ACCESS_TOKEN);
    // A stored Profile: the GGG part does not wait for `StartSession`.
    let harness = Harness::with_login()
        .backend(&backend.url)
        .ggg(&ggg.uri())
        .profiles(&profiles(&["p1"]), "p1")
        .build();
    let toasts = harness.events(TOAST_EVENT);

    harness.start_session();
    harness.wait_for_ready();

    let session = harness.session_slice();
    assert!(session.superseded);
    assert!(!session.offline, "outside a Group nothing shows offline");
    assert_eq!(session.login, LoginState::SignedIn);
    let notifications = harness.notifications();
    assert_eq!(notifications.newest[0].kind, NotificationKind::Warning);
    assert_eq!(
        notifications.newest[0].message.key,
        MessageKey::ErrorSuperseded
    );
    assert!(toasts.try_recv().is_err(), "noted in the list only");
    assert_eq!(harness.due(Deadline::BackendSessionRetry), None);

    // No further backend call: later ones fail at once.
    assert_eq!(
        block_on(auth::refresh_ggg_token(harness.app.handle())),
        Err(IpcError::Superseded)
    );
    harness.start_session();
    assert_eq!(backend.start_session_count(), 1);
    assert_eq!(backend.refresh_count(), 0);
    assert_eq!(harness.notifications().newest.len(), 1, "noted once");

    let harness = harness.restart();
    harness.start_session();
    assert_eq!(backend.start_session_count(), 2);
    assert!(!harness.session_slice().superseded);
}

#[test]
fn f1_session_expired_on_start_session_sends_to_login() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([Err(refused(
        Code::Unauthenticated,
        ErrorReason::SessionExpired,
    ))]);
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    let toasts = harness.events(TOAST_EVENT);

    harness.start_session();

    assert_eq!(
        next_toast(&toasts).message.key,
        MessageKey::ErrorSessionExpired
    );
    assert_eq!(harness.session_slice().login, LoginState::SignedOut);
    assert_eq!(logins(&harness), 0);
    assert_eq!(harness.due(Deadline::BackendSessionRetry), None);
}

#[test]
fn start_session_answer_replaces_the_profile_list_under_the_operation_slot() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([
        session(profiles(&["p1", "p2"]), "p2"),
        session(profiles(&["p2"]), "p2"),
    ]);
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    // For each `profiles` event listing Profiles: whether the operation
    // slot was held as it went out.
    let (under_slot, pushed) = mpsc::channel();
    let handle = harness.app.handle().clone();
    state::listen(
        &harness.app,
        SliceName::Profiles.event_name(),
        move |event| {
            if let Ok(Event::Profiles(profiles)) = serde_json::from_str(event.payload())
                && !profiles.profiles.is_empty()
            {
                let held = handle
                    .state::<Store>()
                    .read(|s: &ipc::slices::Status| s.operation_running);
                let _ = under_slot.send(held);
            }
        },
    );
    let slot = harness.app.state::<OperationSlot>();
    let held = slot.try_hold().unwrap();
    let handle = harness.app.handle().clone();
    tauri::async_runtime::spawn(async move { session_start::run(&handle).await });

    poll(|| (backend.start_session_count() == 1).then_some(()));
    let stored = || -> Vec<(String, i64, String)> {
        block_on(
            sqlx::query_as("SELECT id, position, stash_tab_ids FROM profiles ORDER BY position")
                .fetch_all(&harness.pool()),
        )
        .unwrap()
    };
    drop(held);

    // The answer waited for the slot and went out under it.
    assert_eq!(pushed.recv_timeout(WAIT), Ok(true), "under the slot");

    let applied = poll(|| {
        Some(harness.app.state::<Store>().read(Profiles::clone)).filter(|p| !p.profiles.is_empty())
    });
    let ids: Vec<&str> = applied.profiles.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["p1", "p2"]);
    assert_eq!(applied.active_profile_id.as_deref(), Some("p2"));
    assert_eq!(applied.profiles[0].stash_tab_ids, ["t1", "t2"]);
    assert_eq!(
        stored(),
        [
            ("p1".to_owned(), 0, r#"["t1","t2"]"#.to_owned()),
            ("p2".to_owned(), 1, r#"["t1","t2"]"#.to_owned()),
        ]
    );

    harness.start_session();
    assert_eq!(
        stored(),
        [("p2".to_owned(), 0, r#"["t1","t2"]"#.to_owned())]
    );
    let harness = harness.restart();
    let profiles = harness.app.state::<Store>().read(Profiles::clone);
    assert_eq!(profiles.profiles.len(), 1);
    assert_eq!(profiles.active_profile_id.as_deref(), Some("p2"));
}

#[test]
fn f7_overdue_deadlines_fire_once_after_a_clock_jump() {
    let backend = FakeBackend::start(signed_in());
    let ggg = block_on(async {
        let server = mock_server().await;
        Mock::given(path("/stash/Standard/a"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("X-Rate-Limit-Policy", "stash-request-limit")
                    .insert_header("X-Rate-Limit-Rules", "Account")
                    .insert_header("X-Rate-Limit-Account", "5:10:60")
                    .insert_header("X-Rate-Limit-Account-State", "6:10:60")
                    .insert_header("Retry-After", "60"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/character"))
            .respond_with(characters())
            .mount(&server)
            .await;
        server
    });
    let harness = Harness::launched(&backend.url, &ggg, ACCESS_TOKEN_EXPIRES_AT);
    let rate_limits = harness.events(SliceName::RateLimit.event_name());
    let stash = block_on(harness.app.state::<Ggg>().get::<serde_json::Value, _>(
        harness.app.handle(),
        Request::Stash {
            league: "Standard",
            id: "a",
        },
    ));
    assert!(matches!(
        stash,
        Err(ggg::Error::RateLimited { seconds_left: 60 })
    ));
    assert_eq!(
        harness.due(Deadline::RateLimitCooldownEnd),
        Some(START + 60_000)
    );
    let scheduler = harness.app.state::<Scheduler>();
    scheduler.set(Deadline::GggSessionRetry, START + 45_000);
    scheduler.set(Deadline::BackendSessionRetry, START + 5_000);
    assert_eq!(harness.tick(), []);

    // The machine slept for an hour.
    harness.clock().advance(60 * 60 * 1000);

    assert_eq!(
        harness.tick(),
        [
            Deadline::GggSessionRetry,
            Deadline::BackendSessionRetry,
            Deadline::RateLimitCooldownEnd
        ]
    );
    assert_eq!(harness.tick(), []);
    let cleared = loop {
        match rate_limits.recv_timeout(WAIT).expect("a rate limit") {
            Event::RateLimit(slice) if slice.cooldown_until.is_none() => break slice,
            _ => {}
        }
    };
    assert_eq!(cleared, harness.app.state::<Store>().read(RateLimit::clone));
    harness.wait_for_characters();
    poll(|| (backend.start_session_count() == 1).then_some(()));
    // Only `tick` starts the retries here, and what they started sets its
    // next deadlines ahead of now.
    assert_eq!(harness.tick(), [], "each fired once");
    assert_eq!(backend.start_session_count(), 1);
    let character_requests = block_on(ggg.received_requests())
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/character")
        .count();
    assert_eq!(character_requests, 1);
}
