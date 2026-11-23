//! GGG requests and rate limits (F7) against a `wiremock` GGG, on a fake
//! clock: pacing by learnt rules, hits spent elsewhere, each cooldown
//! source, restarts, the allowance hint, and the session start's character
//! load.

use std::sync::Arc;
use std::time::Duration;

use exilence_ce_ng_lib::clock::Clock;
use exilence_ce_ng_lib::ggg::{self, Ggg, Request};
use exilence_ce_ng_lib::session_start::Characters;
use exilence_ce_ng_lib::state::Store;
use ipc::slices::{Allowance, Platform, RateLimit};
use ipc::{Event, IpcError, SliceName};
use tauri::Manager;
use tauri::async_runtime::block_on;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::fake_backend::{ACCESS_TOKEN, FakeBackend, signed_in};
use super::fake_ggg::{FakeClock, FakeGgg, Limit, Policy, START};
use super::login::{callback, param};
use super::{Fakes, Harness, mock_server, poll};

const STASH: &str = "/stash/";

fn stash_policy(limits: Vec<Limit>) -> Policy {
    Policy {
        prefix: STASH,
        name: "stash-request-limit",
        limits,
    }
}

fn character_policy() -> Policy {
    Policy {
        prefix: "/character",
        name: "character-list-request-limit",
        limits: vec![(2, 10, 60)],
    }
}

impl Harness {
    /// With GGG at `url` on `clock`, and a stored Login.
    fn with_ggg(url: &str, clock: Arc<FakeClock>) -> Self {
        Self::with_login().ggg(url).clock(clock).build()
    }

    /// Sends `request` through the core's GGG queue.
    fn get(&self, request: Request<'_>) -> Result<serde_json::Value, ggg::Error> {
        block_on(self.app.state::<Ggg>().get(self.app.handle(), request))
    }

    fn stash(&self, id: &str) -> Result<serde_json::Value, ggg::Error> {
        self.get(Request::Stash {
            league: "Standard",
            id,
        })
    }

    fn rate_limit(&self) -> RateLimit {
        self.app.state::<Store>().read(RateLimit::clone)
    }
}

fn seconds_left(result: Result<serde_json::Value, ggg::Error>) -> u64 {
    match result {
        Err(ggg::Error::RateLimited { seconds_left }) => seconds_left,
        other => panic!("not rate limited: {other:?}"),
    }
}

#[test]
fn f7_requests_never_exceed_the_learnt_rules() {
    let rule_sets: [Vec<Limit>; 2] = [
        vec![(5, 10, 60), (12, 60, 300)],
        vec![(3, 5, 10), (8, 30, 60), (20, 300, 1800)],
    ];
    for limits in rule_sets {
        let clock = FakeClock::new();
        let ggg = FakeGgg::start(clock.clone(), vec![stash_policy(limits.clone())]);
        let harness = Harness::with_ggg(&ggg.url(), clock.clone());

        for i in 0..30 {
            harness.stash(&format!("tab{i}")).unwrap();
        }

        let requests = ggg.requests();
        assert_eq!(requests.len(), 30);
        assert!(requests.iter().all(|r| r.status == 200), "{requests:?}");
        // One hit always stays spare, within period + 2 s.
        let hits = ggg.hits(STASH);
        for (max, period, _) in limits {
            let span = i64::from(period) * 1000 + 2000;
            for &at in &hits {
                let held = hits.iter().filter(|&&h| h > at - span && h <= at).count();
                assert!(
                    held < max as usize,
                    "{held} hits within {period} s + 2 s at {at} under {max}:{period}"
                );
            }
        }
        assert!(clock.now() > START, "the queue paced");
    }
}

#[test]
fn f7_hits_spent_elsewhere_are_counted() {
    let clock = FakeClock::new();
    let ggg = FakeGgg::start(clock.clone(), vec![stash_policy(vec![(5, 10, 60)])]);
    let harness = Harness::with_ggg(&ggg.url(), clock);

    harness.stash("a").unwrap();
    ggg.spend_elsewhere(STASH, 2, START);
    harness.stash("b").unwrap();
    // GGG reported 4 hits, 2 of them spent elsewhere: 4 = max − 1, so the
    // next request waits until the oldest leaves the window (10 s + 2 s).
    // Counting only its own 2, the client would have sent it at once.
    harness.stash("c").unwrap();

    let requests = ggg.requests();
    let sent: Vec<i64> = requests.iter().map(|r| r.at).collect();
    assert_eq!(sent, [START, START, START + 12_000]);
    assert!(requests.iter().all(|r| r.status == 200));
}

/// A 429 with the stash policy's headers; rules restrict for 60 and 300 s.
fn too_many_requests(retry_after: Option<u32>, state: &str) -> ResponseTemplate {
    let response = ResponseTemplate::new(429)
        .insert_header("X-Rate-Limit-Policy", "stash-request-limit")
        .insert_header("X-Rate-Limit-Rules", "Account")
        .insert_header("X-Rate-Limit-Account", "5:10:60,20:300:300")
        .insert_header("X-Rate-Limit-Account-State", state);
    match retry_after {
        Some(seconds) => response.insert_header("Retry-After", seconds.to_string().as_str()),
        None => response,
    }
}

/// GGG answering `first` once, then successfully.
fn ggg_answering(first: ResponseTemplate) -> MockServer {
    let ok = ResponseTemplate::new(200)
        .insert_header("X-Rate-Limit-Policy", "character-list-request-limit")
        .insert_header("X-Rate-Limit-Rules", "Account")
        .insert_header("X-Rate-Limit-Account", "2:10:60")
        .insert_header("X-Rate-Limit-Account-State", "1:10:0")
        .set_body_json(serde_json::json!({ "characters": [] }));
    block_on(async {
        let server = mock_server().await;
        Mock::given(any())
            .respond_with(first)
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(any()).respond_with(ok).mount(&server).await;
        server
    })
}

fn received(server: &MockServer) -> usize {
    block_on(server.received_requests()).unwrap().len()
}

/// Answers a stash request with `first` and checks the cooldown it starts:
/// pushed, stored, failing every GGG request at once until it ends. Returns
/// its length in seconds.
fn cooldown_after(first: ResponseTemplate) -> u64 {
    let clock = FakeClock::new();
    let server = ggg_answering(first);
    let harness = Harness::with_ggg(&server.uri(), clock.clone());
    let pushed = harness.events(SliceName::RateLimit.event_name());

    let seconds = seconds_left(harness.stash("a"));
    let until = START + seconds as i64 * 1000;

    match pushed.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::RateLimit(slice) => assert_eq!(slice.cooldown_until, Some(until)),
        event => panic!("not the rate limit: {event:?}"),
    }
    let stored: Option<i64> = block_on(
        sqlx::query_scalar(
            "SELECT restricted_until FROM rate_limit_policies \
             WHERE policy = 'stash-request-limit'",
        )
        .fetch_one(&harness.pool()),
    )
    .unwrap();
    assert_eq!(stored, Some(until));

    // Any other GGG request fails at once, without reaching GGG.
    clock.advance(1_000);
    let error = harness.get(Request::Characters).unwrap_err();
    assert_eq!(
        IpcError::from(error),
        IpcError::RateLimited {
            seconds_left: seconds - 1
        }
    );
    assert_eq!(received(&server), 1);

    clock.advance(until - clock.now());
    harness.get(Request::Characters).unwrap();
    assert_eq!(received(&server), 2);
    seconds
}

#[test]
fn f7_retry_after_sets_the_cooldown() {
    assert_eq!(
        cooldown_after(too_many_requests(Some(30), "6:10:45,6:300:0")),
        30
    );
}

#[test]
fn f7_without_retry_after_the_active_restriction_sets_the_cooldown() {
    assert_eq!(
        cooldown_after(too_many_requests(None, "6:10:45,6:300:0")),
        45
    );
}

#[test]
fn f7_without_an_active_restriction_the_largest_rule_restriction_sets_the_cooldown() {
    assert_eq!(
        cooldown_after(too_many_requests(None, "6:10:0,6:300:0")),
        300
    );
}

#[test]
fn f7_a_restart_during_a_cooldown_keeps_the_remaining_wait() {
    let clock = FakeClock::new();
    let server = ggg_answering(too_many_requests(Some(60), "6:10:0,6:300:0"));
    let harness = Harness::with_ggg(&server.uri(), clock.clone());
    assert_eq!(seconds_left(harness.stash("a")), 60);

    clock.advance(20_000);
    let harness = harness.restart();

    assert_eq!(harness.rate_limit().cooldown_until, Some(START + 60_000));
    assert_eq!(seconds_left(harness.get(Request::Characters)), 40);
    assert_eq!(received(&server), 1);
    clock.advance(40_000);
    harness.get(Request::Characters).unwrap();
}

#[test]
fn f7_a_restart_restores_the_windows() {
    let clock = FakeClock::new();
    let ggg = FakeGgg::start(clock.clone(), vec![stash_policy(vec![(5, 10, 60)])]);
    let harness = Harness::with_ggg(&ggg.url(), clock.clone());
    for id in ["a", "b", "c", "d"] {
        harness.stash(id).unwrap();
    }

    clock.advance(1_000);
    let harness = harness.restart();
    // The 4 reported hits count as sent at the last response: the first
    // request after the restart is paced too.
    harness.stash("e").unwrap();

    let sent: Vec<i64> = ggg.requests().iter().map(|r| r.at).collect();
    assert_eq!(sent, [START, START, START, START, START + 12_000]);
}

#[test]
fn f7_allowance_hint_names_the_strictest_window_of_the_stash_tab_policy() {
    let clock = FakeClock::new();
    let ggg = FakeGgg::start(
        clock.clone(),
        vec![
            character_policy(),
            stash_policy(vec![(15, 10, 60), (30, 300, 1800)]),
        ],
    );
    let harness = Harness::with_ggg(&ggg.url(), clock);
    assert_eq!(harness.rate_limit().allowance, None, "no headers known yet");

    harness.get(Request::Characters).unwrap();
    assert_eq!(
        harness.rate_limit().allowance,
        None,
        "not the stash-tab policy"
    );

    harness.stash("a").unwrap();
    // 30 per 300 s (0.1/s) is stricter than 15 per 10 s (1.5/s).
    let expected = Some(Allowance {
        max: 30,
        period: 300,
    });
    assert_eq!(harness.rate_limit().allowance, expected);
    let harness = harness.restart();
    assert_eq!(harness.rate_limit().allowance, expected);
}

#[test]
fn f7_sign_in_loads_characters_and_leagues_through_the_ggg_queue() {
    let backend = FakeBackend::start(signed_in());
    let clock = FakeClock::new();
    // The session start goes on with the active Profile's stash tab list.
    let stash_list = Policy {
        prefix: "/stash",
        name: "stash-list-request-limit",
        limits: vec![(10, 10, 60)],
    };
    let ggg = FakeGgg::start(clock.clone(), vec![character_policy(), stash_list]);
    let harness = Harness::with(Fakes {
        backend_url: backend.url.clone(),
        ggg_url: ggg.url(),
        clock,
        ..Fakes::default()
    });
    let url = harness.start_login(Platform::Pc);

    harness.deep_link(&callback(&param(&url, "state"), "code-123"));

    let leagues =
        poll(|| Some(harness.app.state::<Characters>().leagues()).filter(|l| !l.is_empty()));
    assert_eq!(leagues, ["Settlers", "Standard"]);
    let requests: Vec<_> = block_on(ggg.server.received_requests())
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/character")
        .collect();
    assert_eq!(requests.len(), 1);
    let header = |name: &str| requests[0].headers[name].to_str().unwrap().to_owned();
    assert_eq!(requests[0].url.path(), "/character");
    assert_eq!(header("authorization"), format!("Bearer {ACCESS_TOKEN}"));
    assert_eq!(
        header("user-agent"),
        format!(
            "OAuth exilencece/{} (contact: exilencece@gmail.com) StrictMode",
            env!("CARGO_PKG_VERSION")
        )
    );
    let endpoints: String = block_on(
        sqlx::query_scalar("SELECT endpoints FROM rate_limit_policies WHERE policy = ?1")
            .bind("character-list-request-limit")
            .fetch_one(&harness.pool()),
    )
    .unwrap();
    assert_eq!(endpoints, r#"["characters"]"#);
}
