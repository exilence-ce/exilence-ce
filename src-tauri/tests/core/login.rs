//! Sign-in (F1, D18, D20) and its notifications and toasts (F18), against
//! the fake backend.

use std::sync::mpsc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use exilence_ce_ng_lib::auth;
use ipc::command::{CopyLoginLink, NoArgs, StartLogin, StartLoginArgs};
use ipc::slices::{LoginState, NotificationKind, Notifications, Platform, Session, Settings};
use ipc::{BackendReason, Event, IpcError, MessageKey, SliceName, TOAST_EVENT};
use sha2::{Digest, Sha256};
use tauri::{Manager, Url};
use tonic::{Code, Status};
use wire::v1::{ErrorDetail, ErrorReason};

use super::fake_backend::{
    ACCESS_TOKEN, ACCESS_TOKEN_EXPIRES_AT, ACCOUNT_ID, ACCOUNT_NAME, FakeBackend, SESSION_TOKEN,
    signed_in,
};
use super::fake_ggg::{FakeClock, FakeGgg, Policy};
use super::{Fakes, Harness, WAIT, next_toast, poll};
use exilence_ce_ng_lib::state::Store;

/// The query of an authorization URL the core opened.
fn query(url: &str) -> Vec<(String, String)> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

pub(super) fn param(url: &str, name: &str) -> String {
    query(url)
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("{name} missing in {url}"))
}

impl Harness {
    pub(super) fn start_login(&self, platform: Platform) -> String {
        self.invoke::<StartLogin>(&StartLoginArgs { platform })
            .unwrap();
        self.desktop.opened.lock().unwrap().last().unwrap().clone()
    }

    pub(super) fn deep_link(&self, link: &str) {
        auth::open_deep_link(self.app.handle(), link);
    }

    fn session(&self) -> Session {
        self.app.state::<Store>().read(Session::clone)
    }
}

/// The deep link the backend's redirect forwards for `state` and `code`.
pub(super) fn callback(state: &str, code: &str) -> String {
    let mut url = Url::parse("exilence-ce-ng://").unwrap();
    url.query_pairs_mut()
        .append_pair("code", code)
        .append_pair("state", state);
    url.into()
}

/// Waits for a session whose login state is `login`.
pub(super) fn wait_for_login(events: &mpsc::Receiver<Event>, login: LoginState) -> Session {
    loop {
        match events.recv_timeout(WAIT).expect("a session event") {
            Event::Session(session) if session.login == login => return session,
            _ => {}
        }
    }
}

#[test]
fn d18_d20_each_login_attempt_has_pkce_s256_and_a_fresh_ng_state() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_backend(&backend.url);
    let sessions = harness.events(SliceName::Session.event_name());

    let first = harness.start_login(Platform::Xbox);
    let second = harness.start_login(Platform::Xbox);
    harness.invoke::<CopyLoginLink>(&NoArgs {}).unwrap();

    for url in [&first, &second] {
        assert!(
            url.starts_with("https://www.pathofexile.com/oauth/authorize?"),
            "{url}"
        );
        let names: Vec<String> = query(url).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            names,
            [
                "client_id",
                "response_type",
                "scope",
                "state",
                "redirect_uri",
                "code_challenge",
                "code_challenge_method"
            ]
        );
        assert_eq!(param(url, "client_id"), "exilencece");
        assert_eq!(param(url, "response_type"), "code");
        assert_eq!(
            param(url, "scope"),
            "account:stashes account:profile account:characters"
        );
        assert_eq!(
            param(url, "redirect_uri"),
            "https://api.exilence.de/api/authentication/redirect"
        );
        assert_eq!(param(url, "code_challenge_method"), "S256");
        assert!(param(url, "state").starts_with("ng-"), "{url}");
    }
    assert_ne!(param(&first, "state"), param(&second, "state"));
    assert_ne!(
        param(&first, "code_challenge"),
        param(&second, "code_challenge")
    );
    assert_eq!(
        *harness.desktop.copied.lock().unwrap(),
        std::slice::from_ref(&second),
        "the copy-link control copies the same URL"
    );
    // The platform is remembered.
    assert_eq!(
        harness.app.state::<Store>().read(|s: &Settings| s.platform),
        Platform::Xbox
    );
    assert_eq!(harness.session().platform, Platform::Xbox);

    // The first attempt was replaced: its state no longer signs in.
    harness.deep_link(&callback(&param(&first, "state"), "old-code"));
    harness.deep_link(&callback(&param(&second, "state"), "code-123"));
    wait_for_login(&sessions, LoginState::SignedIn);

    let calls = backend.calls.lock().unwrap();
    assert_eq!(calls.sign_ins.len(), 1);
    let sign_in = &calls.sign_ins[0];
    assert_eq!(sign_in.code, "code-123");
    assert_eq!(
        BASE64URL.encode(Sha256::digest(sign_in.code_verifier.as_bytes())),
        param(&second, "code_challenge"),
        "PKCE S256"
    );
    let harness = harness.restart();
    assert_eq!(harness.session().platform, Platform::Xbox);
}

#[test]
fn f1_deep_link_signs_in_stores_the_login_and_starts_the_session() {
    let backend = FakeBackend::start(signed_in());
    let clock = FakeClock::new();
    let ggg = FakeGgg::start(
        clock.clone(),
        vec![
            Policy {
                prefix: "/character",
                name: "character-list-request-limit",
                limits: vec![(10, 10, 60)],
            },
            // The session start's stash tab list of the active Profile.
            Policy {
                prefix: "/stash",
                name: "stash-list-request-limit",
                limits: vec![(10, 10, 60)],
            },
        ],
    );
    let harness = Harness::with(Fakes {
        backend_url: backend.url.clone(),
        ggg_url: ggg.url(),
        clock,
        ..Fakes::default()
    });
    let sessions = harness.events(SliceName::Session.event_name());
    let toasts = harness.events(TOAST_EVENT);
    let url = harness.start_login(Platform::Pc);

    harness.deep_link(&callback(&param(&url, "state"), "code-123"));

    assert_eq!(
        wait_for_login(&sessions, LoginState::SigningIn).account_name,
        None
    );
    let session = wait_for_login(&sessions, LoginState::SignedIn);
    assert_eq!(session.account_name.as_deref(), Some(ACCOUNT_NAME));
    let login: (String, String, String, i64) = tauri::async_runtime::block_on(
        sqlx::query_as(
            "SELECT account_id, session_token, ggg_access_token, ggg_expires_at FROM login",
        )
        .fetch_one(&harness.pool()),
    )
    .unwrap();
    assert_eq!(
        login,
        (
            ACCOUNT_ID.to_owned(),
            SESSION_TOKEN.to_owned(),
            ACCESS_TOKEN.to_owned(),
            ACCESS_TOKEN_EXPIRES_AT
        )
    );
    let started = poll(|| {
        backend
            .calls
            .lock()
            .unwrap()
            .start_sessions
            .first()
            .cloned()
    });
    assert_eq!(
        started,
        (
            Some(format!("Bearer {SESSION_TOKEN}")),
            env!("CARGO_PKG_VERSION").to_owned()
        )
    );
    // F20: every backend call carries a fresh UUIDv7 correlation ID.
    let ids = backend.calls.lock().unwrap().correlation_ids.clone();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    for id in &ids {
        assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 7);
    }
    // F18: a success is list-only.
    let notifications = harness.app.state::<Store>().read(Notifications::clone);
    assert_eq!(notifications.unread, 1);
    assert_eq!(notifications.newest[0].kind, NotificationKind::Success);
    assert_eq!(
        notifications.newest[0].message.key,
        MessageKey::NotificationLoginSucceeded
    );
    assert!(toasts.try_recv().is_err(), "no toast for a success");

    // The stored Login brings the next start straight to the Net worth view.
    let harness = harness.restart();
    let session = harness.session();
    assert_eq!(session.login, LoginState::SignedIn);
    assert_eq!(session.account_name.as_deref(), Some(ACCOUNT_NAME));
}

#[test]
fn d18_mismatching_deep_link_is_rejected() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_backend(&backend.url);
    let sessions = harness.events(SliceName::Session.event_name());
    let toasts = harness.events(TOAST_EVENT);
    let url = harness.start_login(Platform::Pc);

    harness.deep_link(&callback("ng-forged", "stolen-code"));
    harness.deep_link(&callback(
        &format!("{}x", param(&url, "state")),
        "stolen-code",
    ));
    harness.deep_link("exilence-ce-ng://?code=stolen-code");

    for _ in 0..3 {
        let toast = next_toast(&toasts);
        assert_eq!(toast.kind, NotificationKind::Error);
        assert_eq!(toast.message.key, MessageKey::NotificationLoginFailed);
        assert_eq!(
            toast.description.map(|d| d.key),
            Some(MessageKey::ErrorLoginRejected)
        );
    }
    assert_eq!(harness.session().login, LoginState::SignedOut);
    assert!(backend.calls.lock().unwrap().sign_ins.is_empty());
    let notifications = harness.app.state::<Store>().read(Notifications::clone);
    assert_eq!(notifications.unread, 3);

    // The pending attempt survives a forged link.
    harness.deep_link(&callback(&param(&url, "state"), "code-123"));
    wait_for_login(&sessions, LoginState::SignedIn);
    assert_eq!(backend.calls.lock().unwrap().sign_ins[0].code, "code-123");
}

#[test]
fn f1_failed_sign_in_shows_a_toast_and_returns_to_login() {
    let detail = prost::Message::encode_to_vec(&ErrorDetail {
        reason: ErrorReason::GggAuthFailed.into(),
    });
    let refused = Status::with_details(Code::Unauthenticated, "refused", detail.into());
    let backend = FakeBackend::start(Err(refused));
    let harness = Harness::with_backend(&backend.url);
    let sessions = harness.events(SliceName::Session.event_name());
    let toasts = harness.events(TOAST_EVENT);
    let url = harness.start_login(Platform::Pc);

    harness.deep_link(&callback(&param(&url, "state"), "code-123"));

    wait_for_login(&sessions, LoginState::SigningIn);
    wait_for_login(&sessions, LoginState::SignedOut);
    let toast = next_toast(&toasts);
    assert_eq!(toast.message.key, MessageKey::NotificationLoginFailed);
    assert_eq!(
        toast.description,
        Some(
            IpcError::Backend {
                reason: BackendReason::GggAuthFailed
            }
            .message()
        )
    );
    let notifications = harness.app.state::<Store>().read(Notifications::clone);
    assert_eq!(notifications.newest[0].kind, NotificationKind::Error);
    assert_eq!(notifications.unread, 1);
    let logins: i64 = tauri::async_runtime::block_on(
        sqlx::query_scalar("SELECT count(*) FROM login").fetch_one(&harness.pool()),
    )
    .unwrap();
    assert_eq!(logins, 0);

    // The attempt is used up: the same link again is rejected.
    harness.deep_link(&callback(&param(&url, "state"), "code-123"));
    assert_eq!(
        next_toast(&toasts).description.map(|d| d.key),
        Some(MessageKey::ErrorLoginRejected)
    );
}

#[test]
fn f1_ggg_unavailable_during_sign_in_shows_not_connected_and_returns_to_login() {
    // The backend answers UNAVAILABLE when GGG rate-limits, fails (5xx) or
    // cannot be reached during the code exchange or the profile check.
    let backend = FakeBackend::start(Err(Status::unavailable("GGG unreachable")));
    let harness = Harness::with_backend(&backend.url);
    let sessions = harness.events(SliceName::Session.event_name());
    let toasts = harness.events(TOAST_EVENT);
    let url = harness.start_login(Platform::Pc);

    harness.deep_link(&callback(&param(&url, "state"), "code-123"));

    wait_for_login(&sessions, LoginState::SignedOut);
    let toast = next_toast(&toasts);
    assert_eq!(toast.message.key, MessageKey::NotificationLoginFailed);
    assert_eq!(toast.description, Some(IpcError::Offline.message()));
    assert_eq!(super::session_start::logins(&harness), 0);
}

#[test]
fn f1_refused_authorization_or_unreachable_backend_shows_a_toast() {
    let harness = Harness::new();
    let toasts = harness.events(TOAST_EVENT);

    let url = harness.start_login(Platform::Pc);
    harness.deep_link(&format!(
        "exilence-ce-ng://?error=access_denied&state={}",
        param(&url, "state")
    ));
    assert_eq!(
        next_toast(&toasts).description.map(|d| d.key),
        Some(MessageKey::ErrorLoginRefused)
    );

    let url = harness.start_login(Platform::Pc);
    harness.deep_link(&callback(&param(&url, "state"), "code-123"));
    assert_eq!(
        next_toast(&toasts).description,
        Some(IpcError::Offline.message())
    );
    assert_eq!(harness.session().login, LoginState::SignedOut);
}
