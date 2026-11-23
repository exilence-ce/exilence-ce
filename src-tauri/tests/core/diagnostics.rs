//! Debug mode, the crash notice, the Diagnostics bundle, Open log folder and
//! the log's redaction (F20), against the fake backend, a wiremock GGG and
//! poe.ninja.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, Once, PoisonError};
use std::time::Duration;

use exilence_ce_ng_lib::ggg::Ggg;
use exilence_ce_ng_lib::state::Store;
use exilence_ce_ng_lib::{db, diagnostics, logging};
use ipc::command::{
    ExportDiagnostics, Log, LogArgs, LogLevel, NoArgs, OpenLogFolder, SettingsPatch, SignOut,
};
use ipc::slices::{LoginState, NotificationKind, Notifications, Platform, Status};
use ipc::{Message, MessageKey, SliceName};
use ninja::Ninja;
use serde_json::Value;
use tauri::Manager;
use tauri::async_runtime::block_on;
use tonic::Status as GrpcStatus;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::fake_backend::{ACCESS_TOKEN, ACCOUNT_NAME, FakeBackend, SESSION_TOKEN, signed_in};
use super::fake_ggg::START;
use super::login::{callback, param, wait_for_login};
use super::snapshot::{Setup, ggg_with_stash};
use super::{Fakes, Harness, log_dir, mock_server, poll, update_settings};

/// Debug mode is process-wide: tests that turn it on or rely on it take
/// turns, and turn it off again when done.
struct DebugMode(#[allow(dead_code)] MutexGuard<'static, ()>);

static DEBUG_MODE: Mutex<()> = Mutex::new(());

impl DebugMode {
    fn take() -> Self {
        Self(DEBUG_MODE.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Drop for DebugMode {
    fn drop(&mut self) {
        logging::set_debug_mode(false);
    }
}

fn debug_mode(harness: &Harness, on: bool) {
    update_settings(
        harness,
        SettingsPatch {
            debug_mode: Some(on),
            ..SettingsPatch::default()
        },
    );
}

/// Every log file, oldest first.
fn log_text() -> String {
    log::logger().flush();
    let mut text = String::new();
    for n in (0..logging::FILES).rev() {
        if let Ok(file) = std::fs::read_to_string(log_dir().join(logging::file_name(n))) {
            text.push_str(&file);
        }
    }
    text
}

/// A UI line for the log at `level`.
fn ui_log(harness: &Harness, level: LogLevel, message: &str) {
    harness
        .invoke::<Log>(&LogArgs {
            level,
            message: message.to_owned(),
        })
        .unwrap();
}

/// Every entry of the zip at `path`: name → text.
fn bundle_entries(path: &Path) -> BTreeMap<String, String> {
    let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut entry = zip.by_index(i).unwrap();
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            (entry.name().unwrap().into_owned(), text)
        })
        .collect()
}

impl Harness {
    /// Exports the Diagnostics bundle, saving it where the user chooses;
    /// its entries.
    fn export_bundle(&self) -> BTreeMap<String, String> {
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("bundle.zip");
        *self.desktop.save_to.lock().unwrap() = Some(path.clone());
        self.invoke::<ExportDiagnostics>(&NoArgs {}).unwrap();
        bundle_entries(&path)
    }

    fn status(&self) -> Status {
        self.app.state::<Store>().read(Status::clone)
    }

    fn notification_list(&self) -> Notifications {
        self.app.state::<Store>().read(Notifications::clone)
    }
}

/// A GGG that refuses every request with rate-limit headers and a body over
/// 4 KB that echoes the access token.
fn ggg_refusing(marker: &str) -> MockServer {
    block_on(async {
        let server = mock_server().await;
        let body = format!(
            "{{\"error\":\"{marker}\",\"access_token\":\"{ACCESS_TOKEN}\",\
             \"authorization\":\"Bearer {ACCESS_TOKEN}\",\"padding\":\"{}\"}}",
            "p".repeat(5000)
        );
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(400)
                    .insert_header("x-rate-limit-policy", "character-request-limit")
                    .insert_header("x-rate-limit-rules", "Account")
                    .insert_header("x-rate-limit-account", "10:60:60")
                    .insert_header("x-rate-limit-account-state", "1:60:0")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        server
    })
}

#[test]
fn f20_debug_mode_toggles_at_once_and_stays_on_across_a_restart() {
    let _debug = DebugMode::take();
    let harness = Harness::new();
    let statuses = harness.events(SliceName::Status.event_name());
    assert!(!harness.status().debug_mode);
    let id = uuid::Uuid::now_v7();

    ui_log(&harness, LogLevel::Debug, &format!("{id} before"));
    debug_mode(&harness, true);
    assert!(harness.status().debug_mode, "the toolbar's debug marker");
    assert!(matches!(
        statuses.recv_timeout(std::time::Duration::from_secs(5)),
        Ok(ipc::Event::Status(Status {
            debug_mode: true,
            ..
        }))
    ));
    assert_eq!(log::max_level(), log::LevelFilter::Debug);
    ui_log(&harness, LogLevel::Debug, &format!("{id} while on"));

    // Stays on until turned off, restarts included.
    let harness = harness.restart();
    assert!(harness.status().debug_mode);
    assert_eq!(log::max_level(), log::LevelFilter::Debug);

    debug_mode(&harness, false);
    assert!(!harness.status().debug_mode);
    assert_eq!(log::max_level(), log::LevelFilter::Info);
    ui_log(&harness, LogLevel::Debug, &format!("{id} after"));

    let log = log_text();
    assert!(log.contains(&format!("{id} while on")), "{log}");
    assert!(!log.contains(&format!("{id} before")));
    assert!(!log.contains(&format!("{id} after")));
}

#[test]
fn f20_debug_mode_logs_every_ggg_ninja_and_backend_request() {
    let _debug = DebugMode::take();
    let marker = uuid::Uuid::now_v7().to_string();
    let ggg = ggg_refusing(&marker);
    let ninja = block_on(async {
        let server = mock_server().await;
        Mock::given(path("/poe1/api/economy/leagues"))
            .respond_with(
                ResponseTemplate::new(503).set_body_string(format!("{marker} ninja is down")),
            )
            .mount(&server)
            .await;
        server
    });
    let backend = FakeBackend::start(signed_in());
    backend
        .script
        .lock()
        .unwrap()
        .sign_outs
        .push_back(Err(GrpcStatus::internal(format!(
            "{marker} backend refused"
        ))));
    let harness = Harness::with_login()
        .backend(&backend.url)
        .ggg(&ggg.uri())
        .ninja(&ninja.uri())
        .build();
    debug_mode(&harness, true);

    let handle = harness.app.handle();
    assert!(block_on(handle.state::<Ggg>().characters(handle)).is_err());
    assert!(block_on(handle.state::<Ninja>().price_leagues()).is_err());
    harness.invoke::<SignOut>(&NoArgs {}).unwrap();

    let log = log_text();
    let lines: Vec<&str> = log.lines().filter(|l| l.contains(" DEBUG ")).collect();
    let find = |needle: &str| {
        lines
            .iter()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no debug line with {needle}:\n{log}"))
            .to_string()
    };

    // GGG: method, URL, status, duration and rate-limit headers; the failed
    // body cut at 4 KB, redacted.
    let request = find(&format!(
        "GGG GET {}/character: 400 Bad Request after",
        ggg.uri()
    ));
    assert!(request.contains(" ms ["), "{request}");
    assert!(
        request.contains("x-rate-limit-policy: character-request-limit"),
        "{request}"
    );
    assert!(
        request.contains("x-rate-limit-account: 10:60:60"),
        "{request}"
    );
    let body = find(&format!("{}/character: 400 Bad Request body: ", ggg.uri()));
    assert!(body.contains(&marker), "{body}");
    assert!(body.contains("(cut at 4096 of "), "{body}");
    assert!(!body.contains(ACCESS_TOKEN), "{body}");

    // poe.ninja.
    find(&format!(
        "poe.ninja GET {}/poe1/api/economy/leagues: 503 Service Unavailable after",
        ninja.uri()
    ));
    find(&format!("{marker} ninja is down"));

    // The backend: URL, correlation ID, gRPC status and duration, and the
    // failure's message.
    let id = backend
        .calls
        .lock()
        .unwrap()
        .correlation_ids
        .last()
        .cloned()
        .unwrap();
    let call = find(&format!(
        "backend POST {}/exilence.v1.AuthService/SignOut, correlation ID {id}: Internal: {marker} backend refused after",
        backend.url
    ));
    assert!(call.contains(" ms"), "{call}");

    // Rate-limit state changes per policy, rule and window (the limiter).
    find("rate limit policy character-request-limit: rules");
}

#[test]
fn f20_debug_mode_logs_each_snapshot_step_with_its_duration_and_counts() {
    let _debug = DebugMode::take();
    let backend = FakeBackend::start(signed_in());
    let setup = Setup::ready(Some(&backend), ggg_with_stash(Duration::ZERO));
    debug_mode(&setup.harness, true);

    setup.take_and_wait();

    let id = setup.snapshots()[0].0.clone();
    let log = log_text();
    // The log is process-wide: while Debug mode is on, Snapshots of tests
    // running alongside log their steps too, so a step's first line may be
    // another Snapshot's. Each step is found with the counts this fixture's
    // Snapshot logs.
    let step = |prefix: &str, counts: &str| {
        log.lines()
            .find(|l| l.contains(" DEBUG ") && l.contains(prefix) && l.contains(counts))
            .unwrap_or_else(|| panic!("no debug line with {prefix}…{counts}:\n{log}"))
            .to_owned()
    };
    for (prefix, counts) in [
        ("Snapshot fetched: ", "2 tabs, 4 items in "),
        ("Snapshot priced: ", "definitions against "),
        ("Snapshot committed: ", "3 new definitions in "),
        (&format!("Snapshot {id} uploaded in "), ""),
    ] {
        let line = step(prefix, counts);
        assert!(line.trim_end().ends_with(" ms"), "{line}");
    }
}

#[test]
fn f20_correlation_id_of_a_backend_request_is_in_the_client_log_and_at_the_backend() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login().backend(&backend.url).build();

    harness.invoke::<SignOut>(&NoArgs {}).unwrap();

    let received = backend.calls.lock().unwrap().correlation_ids.clone();
    assert_eq!(received.len(), 1, "one SignOut");
    let id = &received[0];
    assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 7);
    let log = log_text();
    assert!(
        log.lines().any(|l| l.contains(" INFO ")
            && l.contains(&format!("backend SignOut, correlation ID {id}"))),
        "{id} not in the client log:\n{log}"
    );
}

#[test]
fn f20_tokens_code_state_authorization_and_group_passwords_never_reach_the_log_or_the_bundle() {
    let _debug = DebugMode::take();
    for debug in [false, true] {
        let marker = uuid::Uuid::now_v7().to_string();
        let ggg = ggg_refusing(&marker);
        let backend = FakeBackend::start(signed_in());
        backend.script_start_sessions([Err(GrpcStatus::internal(format!(
            "{marker}: token {SESSION_TOKEN} refused, Authorization: Bearer {SESSION_TOKEN}"
        )))]);
        let harness = Harness::with(Fakes {
            backend_url: backend.url.clone(),
            ggg_url: ggg.uri(),
            ..Fakes::default()
        });
        debug_mode(&harness, debug);
        let sessions = harness.events(SliceName::Session.event_name());

        // Sign-in: the deep link's code and state, the verifier sent with
        // them, and the session and access tokens answered.
        let url = harness.start_login(Platform::Pc);
        let state = param(&url, "state");
        let code = format!("code-SECRET-{marker}");
        let link = callback(&state, &code);
        log::info!("deep link received: {link}");
        harness.deep_link(&link);
        wait_for_login(&sessions, LoginState::SignedIn);
        let verifier = backend.calls.lock().unwrap().sign_ins[0]
            .code_verifier
            .clone();
        // The session start: GGG echoes the access token in a failed body,
        // the backend the session token in a failure.
        poll(|| {
            log_text()
                .contains(&format!("{marker}: token"))
                .then_some(())
        });
        let handle = harness.app.handle();
        assert!(block_on(handle.state::<Ggg>().characters(handle)).is_err());
        // UI-side lines and the forms a secret may take.
        ui_log(
            &harness,
            LogLevel::Error,
            "JoinGroupRequest { name: \"Mappers\", password: \"hunter2-group-pw\" }",
        );
        ui_log(
            &harness,
            LogLevel::Debug,
            "join failed for password=hunter2-group-pw",
        );
        log::info!("request headers: Authorization: Bearer {SESSION_TOKEN}");
        log::info!("GGG answered {{\"access_token\":\"{ACCESS_TOKEN}\",\"expires_in\":3600}}");
        log::error!("sign-in failed for verifier {verifier} and token {ACCESS_TOKEN}");

        let bundle = harness.export_bundle();
        let log = log_text();
        assert!(
            log.contains(&format!("signed in as {ACCOUNT_NAME}")),
            "account names stay"
        );
        assert!(log.contains("[redacted]"));
        let bundled_log = &bundle["logs/exilence-ce-ng.log"];
        assert!(bundled_log.contains(&format!("signed in as {ACCOUNT_NAME}")));
        if debug {
            assert!(bundled_log.contains(&format!("{marker}\",\"access_token\":\"[redacted]")));
        }
        for secret in [
            SESSION_TOKEN,
            ACCESS_TOKEN,
            &code,
            &state,
            &verifier,
            "hunter2-group-pw",
        ] {
            assert!(!log.contains(secret), "{secret} logged (debug {debug})");
            for (name, text) in &bundle {
                assert!(!text.contains(secret), "{secret} in {name} (debug {debug})");
            }
        }
    }
}

#[test]
fn f20_diagnostics_bundle_holds_logs_versions_system_settings_and_rate_limits() {
    let dir = tempfile::tempdir().unwrap();
    block_on(async {
        let pool = db::open(dir.path()).await.unwrap();
        sqlx::query(
            "INSERT INTO rate_limit_policies (policy, headers, endpoints, observed_at) \
             VALUES ('stash-request-limit', ?1, '[\"stash\"]', ?2)",
        )
        .bind(
            r#"{"x-rate-limit-policy":"stash-request-limit","x-rate-limit-rules":"Account","x-rate-limit-account":"30:60:60","x-rate-limit-account-state":"1:60:0"}"#,
        )
        .bind(START)
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    });
    let harness = Harness::open(dir, Fakes::default());
    update_settings(
        &harness,
        SettingsPatch {
            ui_scale: Some(120),
            ..SettingsPatch::default()
        },
    );
    let marker = uuid::Uuid::now_v7().to_string();
    ui_log(
        &harness,
        LogLevel::Error,
        &format!("{marker} before the export"),
    );

    // Cancelling the dialog saves nothing.
    harness.invoke::<ExportDiagnostics>(&NoArgs {}).unwrap();
    assert!(harness.notification_list().newest.is_empty());

    let bundle = harness.export_bundle();

    let suggested = harness.desktop.suggested.lock().unwrap().clone();
    assert_eq!(suggested.len(), 2);
    assert!(suggested[1].starts_with("exilence-ce-ng-diagnostics-"));
    assert!(suggested[1].ends_with(".zip"));
    assert!(bundle["logs/exilence-ce-ng.log"].contains(&format!("{marker} before the export")));
    for name in bundle.keys() {
        assert!(
            name == diagnostics::REPORT_FILE || name.starts_with("logs/exilence-ce-ng.log"),
            "{name}"
        );
    }
    let report: Value = serde_json::from_str(&bundle[diagnostics::REPORT_FILE]).unwrap();
    assert_eq!(report["app"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["app"]["channel"], "latest");
    assert!(report["app"]["debug_mode"].is_boolean());
    let system = &report["system"];
    assert_eq!(system["os"], std::env::consts::OS);
    assert_eq!(system["arch"], std::env::consts::ARCH);
    for key in ["os_version", "webview"] {
        assert!(
            system[key].as_str().is_some_and(|v| !v.is_empty()),
            "{key}: {system}"
        );
    }
    if cfg!(target_os = "linux") {
        let linux = &system["linux"];
        for key in ["session_type", "gdk_backend", "kernel"] {
            assert!(linux.get(key).is_some(), "{key}: {linux}");
        }
        assert!(linux["gpus"].is_array(), "{linux}");
        assert!(linux["webkit"].is_object(), "{linux}");
    }
    assert_eq!(report["settings"]["ui_scale"], 120);
    assert_eq!(report["settings"]["release_channel"], "latest");
    let policies = report["rate_limit"]["policies"].as_array().unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0]["policy"], "stash-request-limit");
    assert_eq!(policies[0]["headers"]["x-rate-limit-account"], "30:60:60");
    assert!(report["rate_limit"].get("cooldown_until").is_some());

    // Noted in the list, no toast.
    let saved = harness.notification_list().newest;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].kind, NotificationKind::Success);
    assert_eq!(
        saved[0].message.key,
        MessageKey::NotificationDiagnosticsExported
    );
}

#[test]
fn f20_crash_notice_after_an_unclean_exit() {
    let harness = Harness::new();
    assert!(
        harness.notification_list().newest.is_empty(),
        "a first launch"
    );

    let harness = harness.crash();
    let notifications = harness.notification_list();
    assert_eq!(notifications.newest.len(), 1);
    let notice = &notifications.newest[0];
    // List-only: a warning has no toast (F18).
    assert_eq!(notice.kind, NotificationKind::Warning);
    assert_eq!(
        notice.message,
        Message::new(MessageKey::NotificationLastSessionCrashed)
    );
    assert_eq!(
        notice.description,
        Some(Message::new(
            MessageKey::NotificationLastSessionCrashedDescription
        ))
    );

    // A clean exit leaves no notice for the next launch.
    let harness = harness.restart();
    assert!(harness.notification_list().newest.is_empty());
}

#[test]
fn f20_a_core_panic_is_logged() {
    static HOOK: Once = Once::new();
    HOOK.call_once(diagnostics::install_panic_hook);
    log_dir();
    let id = uuid::Uuid::now_v7();

    let panicked = std::thread::Builder::new()
        .name("f20-panic".to_owned())
        .spawn(move || panic!("{id} went wrong"))
        .unwrap()
        .join();

    assert!(panicked.is_err());
    let log = log_text();
    let line = log
        .lines()
        .find(|l| l.contains(&format!("{id} went wrong")))
        .unwrap_or_else(|| panic!("panic not logged:\n{log}"));
    assert!(line.contains(" ERROR "), "{line}");
    assert!(line.contains("panic in thread f20-panic"), "{line}");
}

#[test]
fn f20_open_log_folder_opens_the_log_directory() {
    let harness = Harness::new();
    harness.invoke::<OpenLogFolder>(&NoArgs {}).unwrap();
    assert_eq!(
        *harness.desktop.opened_dirs.lock().unwrap(),
        [log_dir().to_owned()]
    );
}
