//! A `wiremock` GGG that enforces rate-limit rules the way GGG announces
//! them, on a clock the test advances, and the backoff's random factor as
//! the test sets it (parity verification, fakes).

use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use exilence_ce_ng_lib::clock::Clock;
use exilence_ce_ng_lib::scheduler::Jitter;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

/// 2027-01-15T08:00:00Z.
pub const START: i64 = 1_800_000_000_000;

/// Wall-clock time that only moves when a test or a sleep moves it: a
/// sleep jumps to its deadline at once.
pub struct FakeClock(AtomicI64);

impl FakeClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(START)))
    }

    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    fn sleep_until(&self, at: i64) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.0.fetch_max(at, Ordering::SeqCst);
        Box::pin(async {})
    }
}

/// The backoff's random factor, as the test sets it; 1 until then.
pub struct FakeJitter(AtomicU64);

impl FakeJitter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(1f64.to_bits())))
    }

    pub fn set(&self, factor: f64) {
        self.0.store(factor.to_bits(), Ordering::SeqCst);
    }
}

impl Jitter for FakeJitter {
    fn factor(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::SeqCst))
    }
}

/// `max:period:restriction`.
pub type Limit = (u32, u32, u32);

/// One policy of the fake: the requests whose path starts with `prefix`
/// count against `limits` of rule `Account`.
pub struct Policy {
    pub prefix: &'static str,
    pub name: &'static str,
    pub limits: Vec<Limit>,
}

/// A request the fake answered.
#[derive(Clone, Debug)]
pub struct Hit {
    pub at: i64,
    pub status: u16,
}

struct PolicyState {
    policy: Policy,
    /// Every hit GGG counts: the client's and those spent elsewhere.
    hits: Vec<i64>,
}

#[derive(Default)]
struct Ledger {
    policies: Vec<PolicyState>,
    requests: Vec<Hit>,
}

#[derive(Clone)]
struct Responder {
    clock: Arc<FakeClock>,
    ledger: Arc<Mutex<Ledger>>,
}

impl Respond for Responder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let now = self.clock.now();
        let path = request.url.path().to_owned();
        let mut ledger = self.ledger.lock().unwrap();
        let Some(state) = ledger
            .policies
            .iter_mut()
            .find(|p| path.starts_with(p.policy.prefix))
        else {
            panic!("no fake policy for {path}");
        };
        let within = |hits: &[i64], period: u32| {
            hits.iter()
                .filter(|&&h| h > now - i64::from(period) * 1000)
                .count() as u32
        };
        let exceeded = state
            .policy
            .limits
            .iter()
            .any(|&(max, period, _)| within(&state.hits, period) + 1 > max);
        if !exceeded {
            state.hits.push(now);
        }
        let rules: Vec<String> = state
            .policy
            .limits
            .iter()
            .map(|(m, p, r)| format!("{m}:{p}:{r}"))
            .collect();
        let states: Vec<String> = state
            .policy
            .limits
            .iter()
            .map(|&(max, period, restriction)| {
                let hits = within(&state.hits, period) + u32::from(exceeded);
                let active = if hits > max { restriction } else { 0 };
                format!("{hits}:{period}:{active}")
            })
            .collect();
        let response = ResponseTemplate::new(if exceeded { 429 } else { 200 })
            .insert_header("X-Rate-Limit-Policy", state.policy.name)
            .insert_header("X-Rate-Limit-Rules", "Account")
            .insert_header("X-Rate-Limit-Account", rules.join(","))
            .insert_header("X-Rate-Limit-Account-State", states.join(","))
            .set_body_json(body(&path));
        ledger.requests.push(Hit {
            at: now,
            status: if exceeded { 429 } else { 200 },
        });
        response
    }
}

/// What the fake answers: three characters in two leagues for `/character`,
/// an empty object otherwise.
fn body(path: &str) -> serde_json::Value {
    if path == "/character" {
        serde_json::json!({ "characters": [
            { "id": "c1", "name": "Hero", "class": "Witch", "league": "Settlers", "level": 90 },
            { "id": "c2", "name": "Old", "class": "Duelist", "league": "Standard", "level": 70 },
            { "id": "c3", "name": "Alt", "class": "Ranger", "league": "Settlers", "level": 12 },
        ]})
    } else {
        serde_json::json!({})
    }
}

pub struct FakeGgg {
    pub server: MockServer,
    ledger: Arc<Mutex<Ledger>>,
}

impl FakeGgg {
    /// GGG enforcing `policies` on `clock`.
    pub fn start(clock: Arc<FakeClock>, policies: Vec<Policy>) -> Self {
        let ledger = Arc::new(Mutex::new(Ledger {
            policies: policies
                .into_iter()
                .map(|policy| PolicyState {
                    policy,
                    hits: Vec::new(),
                })
                .collect(),
            requests: Vec::new(),
        }));
        let responder = Responder {
            clock,
            ledger: Arc::clone(&ledger),
        };
        let server = tauri::async_runtime::block_on(async {
            let server = super::mock_server().await;
            Mock::given(any())
                .respond_with(responder)
                .mount(&server)
                .await;
            server
        });
        Self { server, ledger }
    }

    pub fn url(&self) -> String {
        self.server.uri()
    }

    /// Spends `count` hits of the policy of `prefix` now, as another tool
    /// using the same account would.
    pub fn spend_elsewhere(&self, prefix: &str, count: usize, now: i64) {
        let mut ledger = self.ledger.lock().unwrap();
        let state = ledger
            .policies
            .iter_mut()
            .find(|p| p.policy.prefix == prefix)
            .unwrap();
        state.hits.extend(std::iter::repeat_n(now, count));
    }

    /// Every request answered so far.
    pub fn requests(&self) -> Vec<Hit> {
        self.ledger.lock().unwrap().requests.clone()
    }

    /// Every hit of the policy of `prefix`, the client's and others'.
    pub fn hits(&self, prefix: &str) -> Vec<i64> {
        let ledger = self.ledger.lock().unwrap();
        let state = ledger
            .policies
            .iter()
            .find(|p| p.policy.prefix == prefix)
            .unwrap();
        state.hits.clone()
    }
}
