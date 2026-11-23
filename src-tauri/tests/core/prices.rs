//! Prices (F5): every poe.ninja request legacy makes, mapped into one table,
//! conditional requests, 404 as no prices, the minute check with its
//! 60-minute age on a fake clock, and failures, against a `wiremock`
//! poe.ninja.

use std::sync::Arc;

use domain::pricing::PriceEntry;
use domain::pricing::ninja::{OverviewKind, overviews};
use exilence_ce_ng_lib::prices::{self, CHECK_MS, MAX_AGE_MS, PriceTables};
use exilence_ce_ng_lib::profiles;
use exilence_ce_ng_lib::scheduler::{self, Deadline, Scheduler};
use exilence_ce_ng_lib::slot::OperationSlot;
use exilence_ce_ng_lib::state::Store;
use ipc::slices::{PriceAvailability, Prices, Session, Status};
use tauri::Manager;
use tauri::async_runtime::block_on;
use wiremock::matchers::{any, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::fake_backend::{ACCESS_TOKEN, ACCOUNT_ID, FakeBackend, signed_in};
use super::fake_ggg::START;
use super::session_start::ggg_accepting;
use super::{Harness, mock_server, poll};

pub(super) const LEAGUE: &str = "Settlers";
pub(super) const CURRENCY: &str = "/poe1/api/economy/stash/current/currency/overview";
const ITEM: &str = "/poe1/api/economy/stash/current/item/overview";
const EXCHANGE: &str = "/poe1/api/economy/exchange/current/overview";
const DIVINE_ICON: &str = "https://web.poecdn.com/divine.png";

/// Divine Orb with its trend and page (D3), and Chaos Shard.
pub(super) fn currency_overview() -> serde_json::Value {
    serde_json::json!({
        "lines": [
            { "currencyTypeName": "Divine Orb", "receive": { "value": 150.0, "count": 400 },
              "detailsId": "divine-orb",
              "receiveSparkLine": { "data": [0, null, 4.5], "totalChange": 4.5 } },
            { "currencyTypeName": "Chaos Shard", "receive": { "value": 0.05, "count": 20 } },
        ],
        "currencyDetails": [{ "name": "Divine Orb", "icon": DIVINE_ICON }],
    })
}

fn map_overview() -> serde_json::Value {
    serde_json::json!({ "lines": [
        { "name": "Map (Tier 16)", "icon": "map.png", "chaosValue": 3.0, "count": 50 },
    ]})
}

/// Divine Orb again (fewer trades than the stash overview) and Exalted Orb,
/// which only the exchange overview has.
fn exchange_overview() -> serde_json::Value {
    serde_json::json!({
        "lines": [
            { "id": "divine", "primaryValue": 155.0, "volumePrimaryValue": 1550.0 },
            { "id": "exalted", "primaryValue": 12.0, "volumePrimaryValue": 2400.0 },
        ],
        "items": [
            { "id": "divine", "name": "Divine Orb", "image": "/divine.png" },
            { "id": "exalted", "name": "Exalted Orb", "image": "/exalted.png" },
        ],
    })
}

/// poe.ninja with prices for `Currency` (both overviews) and `Map`; every
/// other overview 404s, as a category poe.ninja dropped.
async fn mount_prices(server: &MockServer) {
    for (url, overview_type, body) in [
        (CURRENCY, "Currency", currency_overview()),
        (ITEM, "Map", map_overview()),
        (EXCHANGE, "Currency", exchange_overview()),
    ] {
        Mock::given(method("GET"))
            .and(path(url))
            .and(query_param("league", LEAGUE))
            .and(query_param("type", overview_type))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .with_priority(2)
            .mount(server)
            .await;
    }
    Mock::given(any())
        .respond_with(ResponseTemplate::new(404))
        .with_priority(10)
        .mount(server)
        .await;
}

pub(super) fn ninja_with_prices() -> MockServer {
    block_on(async {
        let server = mock_server().await;
        mount_prices(&server).await;
        server
    })
}

pub(super) fn requests(server: &MockServer) -> Vec<Request> {
    block_on(server.received_requests()).unwrap()
}

impl Harness {
    /// Signed in, poe.ninja at `ninja`, an active Profile pricing in
    /// `LEAGUE`.
    pub(super) fn for_prices(ninja: &MockServer) -> Self {
        let harness = Self::with_login().ninja(&ninja.uri()).build();
        harness.seed_active_price_league(LEAGUE);
        harness
    }

    fn seed_active_price_league(&self, league: &str) {
        let profile = wire::v1::Profile {
            id: "p1".to_owned(),
            name: "profile 1".to_owned(),
            league_id: "Settlers".to_owned(),
            price_league_id: league.to_owned(),
            ..Default::default()
        };
        block_on(profiles::replace(
            &self.pool(),
            ACCOUNT_ID,
            &[profile],
            "p1",
        ))
        .unwrap();
        block_on(profiles::push(self.app.handle()));
    }

    fn ensure(&self, league: &str) -> Result<Arc<Vec<PriceEntry>>, prices::LoadError> {
        block_on(prices::ensure(self.app.handle(), league))
    }

    pub(super) fn table(&self, league: &str) -> Option<(Arc<Vec<PriceEntry>>, i64)> {
        self.app.state::<PriceTables>().get(league)
    }

    pub(super) fn prices_slice(&self) -> Prices {
        self.app.state::<Store>().read(Prices::clone)
    }

    /// Fires the price check `ms` after now.
    fn check_after(&self, ms: i64) {
        self.fakes.clock.advance(ms);
        self.app
            .state::<Scheduler>()
            .set(Deadline::PriceCheck, self.app.state::<Scheduler>().now());
        assert!(scheduler::tick(self.app.handle()).contains(&Deadline::PriceCheck));
    }
}

/// The table once the background load started by the profile push is done.
#[track_caller]
pub(super) fn loaded(harness: &Harness) -> Prices {
    poll(|| {
        Some(harness.prices_slice()).filter(|p| {
            p.price_league_id.as_deref() == Some(LEAGUE)
                && matches!(
                    p.availability,
                    PriceAvailability::Available | PriceAvailability::Unavailable
                )
        })
    })
}

#[test]
fn f5_every_legacy_request_is_mapped_into_one_table_in_legacy_order_with_dedupe() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);

    let (table, fetched_at) = harness.table(LEAGUE).unwrap();
    let names: Vec<&str> = table.iter().map(|p| p.key.name.as_str()).collect();
    // Currency overviews first, then items, then exchange; the exchange's
    // Divine Orb merged into the first one, which keeps its place.
    assert_eq!(
        names,
        ["Divine Orb", "Chaos Shard", "Map (Tier 16)", "Exalted Orb"]
    );
    assert_eq!(table[0].calculated, 150.0, "most listings win");
    assert_eq!(table[2].key.tier, Some(16));
    assert_eq!(table[3].key.icon, "https://web.poecdn.com/exalted.png");
    assert_eq!(fetched_at, START);

    let sent: Vec<(String, String, String)> = requests(&ninja)
        .iter()
        .map(|r| {
            let query = |name: &str| {
                r.url
                    .query_pairs()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.into_owned())
                    .unwrap()
            };
            (r.url.path().to_owned(), query("league"), query("type"))
        })
        .collect();
    let expected: Vec<(String, String, String)> = overviews()
        .map(|(kind, overview_type)| {
            let url = match kind {
                OverviewKind::Currency => CURRENCY,
                OverviewKind::Item => ITEM,
                OverviewKind::Exchange => EXCHANGE,
            };
            (url.to_owned(), LEAGUE.to_owned(), overview_type.to_owned())
        })
        .collect();
    assert_eq!(sent, expected, "every overview legacy requests, once");

    let slice = harness.prices_slice();
    assert_eq!(slice.availability, PriceAvailability::Available);
    assert_eq!(slice.fetched_at, Some(START));
    assert_eq!(slice.divine_rate, Some(150.0));
    assert_eq!(slice.exalted_rate, Some(12.0));
}

#[test]
fn f5_f6_rates_use_the_custom_price_if_set() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    block_on(
        sqlx::query(
            "INSERT INTO custom_prices (price_league_id, name, icon, frame_type, variant, \
             links, quality, level, ilvl, tier, corrupted, elder, shaper, value) \
             VALUES (?1, 'Exalted Orb', 'https://web.poecdn.com/exalted.png', 5, '', \
             0, 0, 0, 0, 0, 0, 0, 0, 20.0)",
        )
        .bind(LEAGUE)
        .execute(&harness.pool()),
    )
    .unwrap();

    block_on(prices::push(harness.app.handle()));

    assert_eq!(harness.prices_slice().exalted_rate, Some(20.0));
}

#[test]
fn f5_a_league_poe_ninja_answers_404_for_has_no_prices() {
    let ninja = block_on(async {
        let server = mock_server().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        server
    });
    let harness = Harness::for_prices(&ninja);

    let slice = loaded(&harness);

    assert_eq!(slice.availability, PriceAvailability::Unavailable);
    assert_eq!(slice.fetched_at, Some(START));
    assert_eq!(slice.divine_rate, None);
    assert!(harness.table(LEAGUE).unwrap().0.is_empty());
}

#[test]
fn f5_requests_are_conditional_and_a_304_counts_as_fetched_now() {
    let ninja = block_on(async {
        let server = mock_server().await;
        // The currency overview has an ETag; its revalidation is a 304.
        Mock::given(path(CURRENCY))
            .and(query_param("type", "Currency"))
            .and(header("if-none-match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(path(CURRENCY))
            .and(query_param("type", "Currency"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .set_body_json(currency_overview()),
            )
            .with_priority(2)
            .mount(&server)
            .await;
        mount_prices(&server).await;
        server
    });
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    let (first, _) = harness.table(LEAGUE).unwrap();
    let currency_requests = || -> Vec<Option<String>> {
        requests(&ninja)
            .iter()
            .filter(|r| {
                r.url.path() == CURRENCY && r.url.query() == Some("league=Settlers&type=Currency")
            })
            .map(|r| {
                r.headers
                    .get("if-none-match")
                    .map(|v| v.to_str().unwrap().to_owned())
            })
            .collect()
    };

    harness.fakes.clock.advance(MAX_AGE_MS);
    let second = harness.ensure(LEAGUE).unwrap();

    assert_eq!(currency_requests(), [None, Some("\"v1\"".to_owned())]);
    assert_eq!(second, first, "the 304 keeps the answer it revalidated");
    assert!(
        Arc::ptr_eq(&second, &first),
        "the same prices keep the held table, so nothing rebuilds it (D3)"
    );
    assert_eq!(harness.table(LEAGUE).unwrap().1, START + MAX_AGE_MS);
    assert_eq!(harness.prices_slice().fetched_at, Some(START + MAX_AGE_MS));
}

#[test]
fn f5_the_minute_check_refetches_after_60_minutes_and_skips_while_a_snapshot_runs() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    let per_load = overviews().count();
    assert_eq!(requests(&ninja).len(), per_load);
    let wait_for_requests = |count: usize| poll(|| (requests(&ninja).len() == count).then_some(()));

    let due = || {
        harness
            .app
            .state::<Scheduler>()
            .due_at(Deadline::PriceCheck)
    };

    // 60 minutes old while a Snapshot runs: skipped before `check`
    // returns, no load started.
    let slot = harness.app.state::<OperationSlot>();
    let held = block_on(slot.hold());
    harness
        .app
        .state::<Store>()
        .update::<Status, _>(harness.app.handle(), |s| s.snapshot_running = true)
        .unwrap();
    harness.check_after(MAX_AGE_MS);
    assert_eq!(
        requests(&ninja).len(),
        per_load,
        "skipped during a Snapshot"
    );
    assert_eq!(
        due(),
        Some(START + MAX_AGE_MS + CHECK_MS),
        "the next tick is set"
    );
    harness
        .app
        .state::<Store>()
        .update::<Status, _>(harness.app.handle(), |s| s.snapshot_running = false)
        .unwrap();
    drop(held);

    // The next tick catches up.
    harness.fakes.clock.advance(CHECK_MS);
    assert!(scheduler::tick(harness.app.handle()).contains(&Deadline::PriceCheck));
    wait_for_requests(2 * per_load);
    let refetched_at = START + MAX_AGE_MS + CHECK_MS;
    let fetched_at = poll(|| {
        harness
            .prices_slice()
            .fetched_at
            .filter(|&at| at == refetched_at)
    });
    assert_eq!(harness.table(LEAGUE).unwrap().1, fetched_at);

    // Younger than 60 minutes: the next check is set, nothing is fetched.
    // The clock stays put from here, so the load the check started finds
    // the table fresh as this one, run to its end, does.
    harness.check_after(MAX_AGE_MS - 1);
    assert_eq!(due(), Some(refetched_at + MAX_AGE_MS - 1 + CHECK_MS));
    block_on(prices::ensure_active(harness.app.handle()));
    assert_eq!(requests(&ninja).len(), 2 * per_load);
}

#[test]
fn f5_a_failure_keeps_the_previous_prices_and_fetched_at_time() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    let (before, fetched_at) = harness.table(LEAGUE).unwrap();
    block_on(async {
        ninja.reset().await;
        Mock::given(path(ITEM))
            .and(query_param("type", "Beast"))
            .respond_with(ResponseTemplate::new(503))
            .with_priority(1)
            .mount(&ninja)
            .await;
        mount_prices(&ninja).await;
    });

    harness.fakes.clock.advance(MAX_AGE_MS);
    let refreshed = harness.ensure(LEAGUE);
    assert!(
        matches!(refreshed, Err(prices::LoadError::Ninja(_))),
        "{refreshed:?}"
    );

    let (after, still) = harness.table(LEAGUE).unwrap();
    assert!(Arc::ptr_eq(&before, &after), "the same table");
    assert_eq!(still, fetched_at);
    let slice = harness.prices_slice();
    assert_eq!(slice.availability, PriceAvailability::Available);
    assert_eq!(slice.fetched_at, Some(fetched_at));
}

#[test]
fn f5_a_refresh_that_comes_back_empty_keeps_the_previous_prices_and_fetched_at_time() {
    let no_lines = serde_json::json!({ "lines": [], "currencyDetails": [], "items": [] });
    for empty in [
        ResponseTemplate::new(404),
        ResponseTemplate::new(200).set_body_json(no_lines),
    ] {
        let ninja = ninja_with_prices();
        let harness = Harness::for_prices(&ninja);
        loaded(&harness);
        let (before, fetched_at) = harness.table(LEAGUE).unwrap();
        block_on(async {
            ninja.reset().await;
            Mock::given(any()).respond_with(empty).mount(&ninja).await;
        });

        harness.fakes.clock.advance(MAX_AGE_MS);
        let refreshed = harness.ensure(LEAGUE);
        assert!(
            matches!(refreshed, Err(prices::LoadError::Empty)),
            "{refreshed:?}"
        );

        let (after, still) = harness.table(LEAGUE).unwrap();
        assert!(Arc::ptr_eq(&before, &after), "the same table");
        assert_eq!(still, fetched_at);
        let slice = harness.prices_slice();
        assert_eq!(slice.availability, PriceAvailability::Available);
        assert_eq!(slice.fetched_at, Some(fetched_at));

        // Still old: the next minute check retries and loads the prices back.
        block_on(async {
            ninja.reset().await;
            mount_prices(&ninja).await;
        });
        assert!(harness.ensure(LEAGUE).is_ok());
        let (reloaded, now) = harness.table(LEAGUE).unwrap();
        assert!(!reloaded.is_empty());
        assert_eq!(now, fetched_at + MAX_AGE_MS);
    }
}

#[test]
fn f5_switching_to_another_price_league_loads_its_prices_and_others_load_on_demand() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);

    harness.seed_active_price_league("Standard");

    let slice = poll(|| {
        Some(harness.prices_slice()).filter(|p| p.availability == PriceAvailability::Unavailable)
    });
    assert_eq!(slice.price_league_id.as_deref(), Some("Standard"));
    // Another price league, e.g. for the custom-price table: still fresh,
    // not fetched again.
    let before = requests(&ninja).len();
    assert_eq!(harness.ensure(LEAGUE).unwrap().len(), 4);
    assert_eq!(requests(&ninja).len(), before);
}

#[test]
fn f3_f5_the_session_is_ready_once_prices_of_the_active_price_league_loaded() {
    let backend = FakeBackend::start(signed_in());
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let ninja = ninja_with_prices();
    let harness = Harness::with_login()
        .backend(&backend.url)
        .ggg(&ggg.uri())
        .ninja(&ninja.uri())
        .build();
    harness.seed_active_price_league(LEAGUE);
    assert!(!harness.app.state::<Store>().read(|s: &Session| s.ready));

    harness.start_session();
    poll(|| {
        harness
            .app
            .state::<Store>()
            .read(|s: &Session| s.ready)
            .then_some(())
    });
    assert_eq!(loaded(&harness).availability, PriceAvailability::Available);
    assert_eq!(
        harness
            .app
            .state::<Scheduler>()
            .due_at(Deadline::PriceCheck),
        Some(START + CHECK_MS)
    );
}
