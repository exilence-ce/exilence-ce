//! Pricing golden test (F5): the committed GGG and poe.ninja fixtures replayed
//! through `domain` pricing must match what legacy v1.3.0 priced them at
//! (`fixtures/pricing/Standard/oracle/`, recorded by running the legacy code)
//! per GGG item (matched or not, unit price) and per tab (value within 1e-6
//! relative), except for the listed deviations:
//!
//! - D1 changes only the attributes shown for an item, so rows are not compared.
//! - D2 is the only allowed difference to the plain oracle outputs; the `-d2`
//!   outputs must match exactly.
//! - D29–D31 items make legacy fail or never price them, so their hand-made tab
//!   is compared against `hand-made/deviations-expected.json` instead.

#[path = "../../fixtures/pricing/snapshot.rs"]
mod snapshot;

use std::collections::HashMap;
use std::path::PathBuf;

use domain::item_definition::ItemDefinition;
use domain::pricing::ninja::{
    self, CurrencyOverview, ExchangeOverview, ItemOverview, OverviewKind,
};
use domain::pricing::{PriceEntry, PriceTable, PricingOptions, TabId, TabItems};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use snapshot::{LEAGUE, Snapshot, StashResponse};

/// The oracle's setting combinations; the total threshold is always 0.
const DEFAULT: PricingOptions = PricingOptions {
    low_confidence_pricing: false,
    price_threshold: 0.0,
    total_price_threshold: 0.0,
};
const LOW_CONFIDENCE: PricingOptions = PricingOptions {
    low_confidence_pricing: true,
    ..DEFAULT
};
const THRESHOLD_5C: PricingOptions = PricingOptions {
    price_threshold: 5.0,
    ..DEFAULT
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/pricing")
}

fn fixture<T: DeserializeOwned>(path: &str) -> T {
    snapshot::fixture(&root(), path)
}

/// Every captured overview, mapped and deduped in legacy order; a `null`
/// body is a 404, "no prices".
fn captured_prices() -> Vec<PriceEntry> {
    let mut entries = Vec::new();
    for (kind, overview_type) in ninja::overviews() {
        let dir = match kind {
            OverviewKind::Currency => "currency",
            OverviewKind::Item => "item",
            OverviewKind::Exchange => "exchange",
        };
        let path = format!("{LEAGUE}/ninja/{dir}/{overview_type}.json");
        match kind {
            OverviewKind::Currency => {
                if let Some(o) = fixture::<Option<CurrencyOverview>>(&path) {
                    entries.extend(o.price_entries(overview_type));
                }
            }
            OverviewKind::Item => {
                if let Some(o) = fixture::<Option<ItemOverview>>(&path) {
                    entries.extend(o.price_entries(overview_type));
                }
            }
            OverviewKind::Exchange => {
                if let Some(o) = fixture::<Option<ExchangeOverview>>(&path) {
                    entries.extend(o.price_entries(overview_type));
                }
            }
        }
    }
    ninja::dedupe(entries)
}

// ---------------------------------------------------------------- oracle output

#[derive(Debug, Deserialize)]
struct Oracle {
    tabs: Vec<OracleTab>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleTab {
    stash_tab_id: String,
    value: f64,
    items: Vec<OracleItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleItem {
    item_id: String,
    matched: bool,
    price: Option<f64>,
}

fn tab_name(id: &TabId) -> &str {
    match id {
        TabId::Stash(id) => id,
        TabId::Character => "Character",
    }
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    let tolerance = 1e-6 * expected.abs().max(actual.abs());
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: NG {actual}, legacy {expected}"
    );
}

/// Prices the Snapshot with `options` and compares it with the oracle output
/// `<setting>.json` and `<setting>-d2.json`. The `-d2` output must match
/// exactly; the plain one may differ only by D2: an item NG values at 0 that
/// legacy values at 1c.
fn assert_matches_oracle(setting: &str, options: &PricingOptions) {
    let snapshot = Snapshot::load(&root());
    let entries = captured_prices();
    let table = PriceTable::new(&entries, options, &[]);
    let tabs = snapshot.tabs();
    let priced = table.price_tabs(&tabs);

    for d2 in [true, false] {
        let file = if d2 {
            format!("{setting}-d2")
        } else {
            setting.to_owned()
        };
        let oracle: Oracle = fixture(&format!("{LEAGUE}/oracle/{file}.json"));
        let mut oracle_tabs: HashMap<&str, &OracleTab> = oracle
            .tabs
            .iter()
            .map(|t| (t.stash_tab_id.as_str(), t))
            .collect();
        assert_eq!(oracle_tabs.len(), tabs.len(), "{file}: tab count");

        for (tab, priced_tab) in tabs.iter().zip(&priced.tabs) {
            let name = tab_name(&tab.id);
            let expected = oracle_tabs
                .remove(name)
                .unwrap_or_else(|| panic!("{file}: tab {name} missing from the oracle"));
            let mut expected_items: HashMap<&str, &OracleItem> = expected
                .items
                .iter()
                .map(|i| (i.item_id.as_str(), i))
                .collect();
            assert_eq!(
                expected_items.len(),
                tab.items.len(),
                "{file} {name}: item count"
            );

            // Chaos legacy adds for D2 items: 1c per item in the stack.
            let mut d2_value = 0.0;
            for item in &tab.items {
                let want = expected_items
                    .remove(item.id.as_str())
                    .unwrap_or_else(|| panic!("{file} {name}: item {} missing", item.id));
                let price = table.unit_price(&ItemDefinition::from_item(item));
                let d2_deviation = !d2 && price == Some(0.0) && want.price == Some(1.0);
                if d2_deviation {
                    d2_value += f64::from(item.stack_size.filter(|s| *s > 0).unwrap_or(1));
                    continue;
                }
                assert_eq!(
                    (price.is_some(), price),
                    (want.matched, want.price),
                    "{file} {name}: item {} (matched, unit price)",
                    item.id
                );
            }
            assert_close(
                priced_tab.value + d2_value,
                expected.value,
                &format!("{file} {name}: tab value"),
            );
        }
    }
}

#[test]
fn f5_d2_golden_default() {
    assert_matches_oracle("default", &DEFAULT);
}

#[test]
fn f5_d2_golden_low_confidence() {
    assert_matches_oracle("low-confidence", &LOW_CONFIDENCE);
}

#[test]
fn f5_d2_golden_threshold_5c() {
    assert_matches_oracle("threshold-5c", &THRESHOLD_5C);
}

// ---------------------------------------------------------------- D29–D31

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviationsExpected {
    stash_tab_id: String,
    value: f64,
    items: Vec<OracleItem>,
}

/// The hand-made D29–D31 tab against the captured poe.ninja prices, as
/// `hand-made/deviations-expected.json` expects in every setting (D2 cannot
/// change it: none of its matched prices is 0).
#[test]
fn d29_d30_d31_hand_made_tab_prices_as_expected() {
    let stash = fixture::<StashResponse>("hand-made/deviations.json").stash;
    let expected: DeviationsExpected = fixture("hand-made/deviations-expected.json");
    assert_eq!(stash.id, expected.stash_tab_id);
    assert_eq!(stash.items.len(), expected.items.len());
    let entries = captured_prices();

    for options in [DEFAULT, LOW_CONFIDENCE, THRESHOLD_5C] {
        let table = PriceTable::new(&entries, &options, &[]);
        for (item, want) in stash.items.iter().zip(&expected.items) {
            assert_eq!(item.id, want.item_id);
            let price = table.unit_price(&ItemDefinition::from_item(item));
            assert_eq!(
                (price.is_some(), price),
                (want.matched, want.price),
                "{options:?} {}",
                item.id
            );
        }
        let tab = TabItems {
            id: TabId::Stash(stash.id.clone()),
            items: stash.items.iter().collect(),
        };
        let value = table.price_tabs(&[tab]).tabs[0].value;
        assert_close(value, expected.value, &format!("{options:?} tab value"));
    }
}
