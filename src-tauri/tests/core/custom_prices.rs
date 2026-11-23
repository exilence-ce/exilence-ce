//! Custom prices (F6, F9): the price table of a price league, custom prices
//! stored per price league across restarts, and the ex/div rates they
//! change, against a `wiremock` poe.ninja.

use exilence_ce_ng_lib::prices;
use ipc::command::{
    GetPriceTable, GetPriceTableArgs, PriceKey, PriceTableRow, RemoveCustomPrice,
    RemoveCustomPriceArgs, SetCustomPrice, SetCustomPriceArgs,
};
use ipc::{Field, IpcError, MessageKey};
use tauri::async_runtime::block_on;

use super::Harness;
use super::prices::{LEAGUE, loaded, ninja_with_prices, requests};

/// Another price league, which poe.ninja has no prices for.
const OTHER_LEAGUE: &str = "Standard";

impl Harness {
    fn price_table(&self, league: &str) -> Result<Vec<PriceTableRow>, IpcError> {
        self.invoke::<GetPriceTable>(&GetPriceTableArgs {
            price_league: league.to_owned(),
        })
    }

    /// The key of `name` in `LEAGUE`'s price table.
    fn key_of(&self, name: &str) -> PriceKey {
        self.price_table(LEAGUE)
            .unwrap()
            .into_iter()
            .find(|row| row.key.name == name)
            .unwrap()
            .key
    }

    /// The custom prices of `league` as their item names and values.
    fn stored(&self, league: &str) -> Vec<(String, f64)> {
        let mut stored: Vec<_> = block_on(prices::custom_prices(&self.pool(), league))
            .unwrap()
            .into_iter()
            .map(|c| (c.key.name, c.value))
            .collect();
        stored.sort_by(|a, b| a.0.cmp(&b.0));
        stored
    }

    fn set_custom_price(&self, league: &str, key: &PriceKey, value: f64) -> Result<(), IpcError> {
        self.invoke::<SetCustomPrice>(&SetCustomPriceArgs {
            price_league: league.to_owned(),
            key: key.clone(),
            value,
        })
    }

    fn remove_custom_price(&self, league: &str, key: &PriceKey) -> Result<(), IpcError> {
        self.invoke::<RemoveCustomPrice>(&RemoveCustomPriceArgs {
            price_league: league.to_owned(),
            key: key.clone(),
        })
    }
}

/// The custom price the table shows for `name` in `league`.
fn shown(harness: &Harness, league: &str, name: &str) -> Option<f64> {
    harness
        .price_table(league)
        .unwrap()
        .into_iter()
        .find(|row| row.key.name == name)
        .unwrap()
        .custom_price
}

#[test]
fn f6_price_table_loads_a_price_league_on_demand() {
    let ninja = ninja_with_prices();
    // Signed in without a Profile: nothing loads prices but the table.
    let harness = Harness::with_login().ninja(&ninja.uri()).build();
    assert!(harness.table(LEAGUE).is_none());

    let rows = harness.price_table(LEAGUE).unwrap();

    let mut names: Vec<_> = rows.iter().map(|row| row.key.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["Chaos Shard", "Divine Orb", "Exalted Orb", "Map (Tier 16)"]
    );
    assert!(rows.iter().all(|row| row.custom_price.is_none()));
    let divine = rows
        .iter()
        .find(|row| row.key.name == "Divine Orb")
        .unwrap();
    assert_eq!(divine.price, 150.0);
    // Still fresh: not fetched again (the price leagues are, while poe.ninja
    // has none for the selector).
    let price_requests = || {
        requests(&ninja)
            .iter()
            .filter(|r| !r.url.path().ends_with("/leagues"))
            .count()
    };
    let before = price_requests();
    harness.price_table(LEAGUE).unwrap();
    assert_eq!(price_requests(), before);
}

#[test]
fn f6_custom_price_is_set_overwritten_and_removed_per_price_league() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    let divine = harness.key_of("Divine Orb");

    harness.set_custom_price(LEAGUE, &divine, 120.0).unwrap();
    assert_eq!(shown(&harness, LEAGUE, "Divine Orb"), Some(120.0));
    assert_eq!(shown(&harness, LEAGUE, "Exalted Orb"), None);

    harness.set_custom_price(LEAGUE, &divine, 130.0).unwrap();
    assert_eq!(shown(&harness, LEAGUE, "Divine Orb"), Some(130.0));
    assert_eq!(harness.stored(LEAGUE), [("Divine Orb".to_owned(), 130.0)]);

    // The same entry in another price league is another custom price.
    harness
        .set_custom_price(OTHER_LEAGUE, &divine, 90.0)
        .unwrap();
    assert_eq!(harness.stored(LEAGUE), [("Divine Orb".to_owned(), 130.0)]);
    assert_eq!(
        harness.stored(OTHER_LEAGUE),
        [("Divine Orb".to_owned(), 90.0)]
    );

    harness.remove_custom_price(LEAGUE, &divine).unwrap();
    assert_eq!(shown(&harness, LEAGUE, "Divine Orb"), None);
    assert!(harness.stored(LEAGUE).is_empty());
    assert_eq!(
        harness.stored(OTHER_LEAGUE),
        [("Divine Orb".to_owned(), 90.0)]
    );
    // Removing what is not set changes nothing.
    harness.remove_custom_price(LEAGUE, &divine).unwrap();
}

#[test]
fn f6_custom_price_identifies_its_entry_by_every_field() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    let divine = harness.key_of("Divine Orb");
    let other = PriceKey {
        quality: Some(20),
        ..divine.clone()
    };

    harness.set_custom_price(LEAGUE, &divine, 120.0).unwrap();
    harness.set_custom_price(LEAGUE, &other, 1.0).unwrap();
    assert_eq!(shown(&harness, LEAGUE, "Divine Orb"), Some(120.0));
    harness.remove_custom_price(LEAGUE, &other).unwrap();

    assert_eq!(harness.stored(LEAGUE), [("Divine Orb".to_owned(), 120.0)]);
}

#[test]
fn f6_custom_price_must_be_a_number_of_0_or_more() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    let divine = harness.key_of("Divine Orb");

    assert_eq!(
        harness.set_custom_price(LEAGUE, &divine, -1.0),
        Err(IpcError::Validation {
            field: Field::CustomPrice,
            key: MessageKey::ErrorCustomPriceInvalid,
        })
    );
    assert!(harness.stored(LEAGUE).is_empty());
    harness.set_custom_price(LEAGUE, &divine, 0.0).unwrap();
    assert_eq!(shown(&harness, LEAGUE, "Divine Orb"), Some(0.0));
}

#[test]
fn f6_custom_prices_survive_a_restart() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    loaded(&harness);
    let exalted = harness.key_of("Exalted Orb");
    harness.set_custom_price(LEAGUE, &exalted, 20.0).unwrap();

    let harness = harness.restart();

    assert_eq!(harness.stored(LEAGUE), [("Exalted Orb".to_owned(), 20.0)]);
    assert_eq!(shown(&harness, LEAGUE, "Exalted Orb"), Some(20.0));
}

#[test]
fn f6_f9_a_custom_price_change_pushes_the_rates() {
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    let before = loaded(&harness);
    assert_eq!(before.exalted_rate, Some(12.0));
    assert_eq!(before.divine_rate, Some(150.0));
    let exalted = harness.key_of("Exalted Orb");
    let divine = harness.key_of("Divine Orb");

    harness.set_custom_price(LEAGUE, &exalted, 20.0).unwrap();
    harness.set_custom_price(LEAGUE, &divine, 140.0).unwrap();
    let slice = harness.prices_slice();
    assert_eq!(slice.exalted_rate, Some(20.0));
    assert_eq!(slice.divine_rate, Some(140.0));

    // A custom price of another price league leaves the rates alone.
    harness
        .set_custom_price(OTHER_LEAGUE, &exalted, 50.0)
        .unwrap();
    assert_eq!(harness.prices_slice().exalted_rate, Some(20.0));

    harness.remove_custom_price(LEAGUE, &exalted).unwrap();
    assert_eq!(harness.prices_slice().exalted_rate, Some(12.0));
    // A custom price of 0 keeps poe.ninja's rate.
    harness.set_custom_price(LEAGUE, &divine, 0.0).unwrap();
    assert_eq!(harness.prices_slice().divine_rate, Some(150.0));
}
