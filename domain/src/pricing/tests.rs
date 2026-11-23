use super::ninja::{self, CurrencyOverview, ExchangeOverview, ItemOverview, OverviewKind};
use super::*;
use crate::ggg::{ItemMod, ItemProperty, ItemSocket};

// ---------------------------------------------------------------- builders

/// A price entry with every field set, as the item overview sends it.
fn entry(name: &str, calculated: f64) -> PriceEntry {
    PriceEntry {
        key: PriceKey {
            name: name.to_owned(),
            icon: String::new(),
            frame_type: Some(0),
            variant: Some(String::new()),
            links: Some(0),
            quality: Some(0),
            level: Some(0),
            ilvl: Some(0),
            tier: Some(0),
            corrupted: Some(false),
            elder: Some(false),
            shaper: Some(false),
        },
        calculated,
        count: 100,
        ..PriceEntry::default()
    }
}

/// A price entry as the currency overview sends it: no fields but name,
/// icon and frame type.
fn currency_entry(name: &str, calculated: f64) -> PriceEntry {
    PriceEntry {
        key: PriceKey {
            name: name.to_owned(),
            frame_type: Some(5),
            ..PriceKey::default()
        },
        calculated,
        count: 100,
        ..PriceEntry::default()
    }
}

fn with(mut entry: PriceEntry, edit: impl FnOnce(&mut PriceKey)) -> PriceEntry {
    edit(&mut entry.key);
    entry
}

fn item(name: &str, type_line: &str, frame_type: u32) -> Item {
    Item {
        id: format!("{name} {type_line}"),
        name: name.to_owned(),
        type_line: type_line.to_owned(),
        base_type: type_line.to_owned(),
        frame_type,
        ..Item::default()
    }
}

fn currency(type_line: &str) -> Item {
    item("", type_line, frame_type::CURRENCY)
}

fn prop(name: &str, value: &str) -> ItemProperty {
    ItemProperty {
        name: name.to_owned(),
        values: vec![(value.to_owned(), 0)],
    }
}

fn mods(descriptions: &[&str]) -> Vec<ItemMod> {
    descriptions
        .iter()
        .map(|d| ItemMod {
            description: (*d).to_owned(),
        })
        .collect()
}

fn linked(groups: &[u32]) -> Vec<ItemSocket> {
    groups.iter().map(|&group| ItemSocket { group }).collect()
}

fn default_options() -> PricingOptions {
    PricingOptions::default()
}

/// The unit price of `item` against `entries` with default options.
fn price_of(item: &Item, entries: &[PriceEntry]) -> Option<f64> {
    PriceTable::new(entries, &default_options(), &[]).unit_price(&ItemDefinition::from_item(item))
}

fn stash(items: &[Item]) -> TabItems<'_> {
    TabItems {
        id: TabId::Stash("tab".to_owned()),
        items: items.iter().collect(),
    }
}

fn price_tab(items: &[Item], entries: &[PriceEntry], options: &PricingOptions) -> PricedTab {
    let table = PriceTable::new(entries, options, &[]);
    table.price_tabs(&[stash(items)]).tabs.remove(0)
}

// ---------------------------------------------------------------- matching (F5)

#[test]
fn f5_chaos_orb_is_always_1c() {
    let entries = [currency_entry("Chaos Orb", 7.0)];
    assert_eq!(price_of(&currency("Chaos Orb"), &entries), Some(1.0));
    assert_eq!(price_of(&currency("Chaos Orb"), &[]), Some(1.0));
    let custom = [CustomPrice {
        key: entries[0].key.clone(),
        value: 3.0,
    }];
    let table = PriceTable::new(&entries, &default_options(), &custom);
    let chaos = ItemDefinition::from_item(&currency("Chaos Orb"));
    assert_eq!(table.unit_price(&chaos), Some(1.0));
}

#[test]
fn f5_currency_matches_by_name() {
    let entries = [currency_entry("Divine Orb", 150.0)];
    assert_eq!(price_of(&currency("Divine Orb"), &entries), Some(150.0));
    assert_eq!(price_of(&currency("Exalted Orb"), &entries), None);
}

#[test]
fn f5_priced_only_when_exactly_one_entry_matches() {
    let entries = [
        currency_entry("Orb of Annulment", 10.0),
        with(entry("Orb of Annulment", 11.0), |k| k.tier = Some(1)),
    ];
    assert_eq!(price_of(&currency("Orb of Annulment"), &entries), None);
}

#[test]
fn f5_exchange_only_currency_and_v1_3_additions_match_by_name() {
    let entries = [
        currency_entry("Runegraft of Stability", 4.0),
        currency_entry("Coin of Power", 2.0),
        currency_entry("Timeless Astrolabe", 6.0),
        currency_entry("Exalted Orb", 12.0),
    ];
    for (type_line, price) in [
        ("Runegraft of Stability", 4.0),
        ("Coin of Power", 2.0),
        ("Timeless Astrolabe", 6.0),
        ("Exalted Orb", 12.0),
    ] {
        assert_eq!(
            price_of(&currency(type_line), &entries),
            Some(price),
            "{type_line}"
        );
    }
}

#[test]
fn f5_fragment_and_scarab_match_by_name() {
    let entries = [currency_entry("Sacrifice at Dusk", 1.5)];
    assert_eq!(
        price_of(&item("", "Sacrifice at Dusk", 0), &entries),
        Some(1.5)
    );
}

#[test]
fn f5_map_matches_by_tier() {
    let mut map = item("", "Strand Map", frame_type::RARE);
    map.properties = vec![prop("Map Tier", "5")];
    let entries = [
        with(entry("Strand Map", 3.0), |k| k.tier = Some(4)),
        with(entry("Strand Map", 5.0), |k| k.tier = Some(5)),
    ];
    assert_eq!(price_of(&map, &entries), Some(5.0));
    map.properties = vec![prop("Map Tier", "6")];
    assert_eq!(price_of(&map, &entries), None);
}

#[test]
fn f5_map_name_is_base_type() {
    let mut map = item("Doom Spires", "Map (Tier 16)", frame_type::RARE);
    map.base_type = "Map (Tier 16)".to_owned();
    map.properties = vec![prop("Map Tier", "16")];
    let definition = ItemDefinition::from_item(&map);
    assert_eq!(definition.name, "Map (Tier 16)");
    assert_eq!(definition.tier, 16);
    assert_eq!(price_of(&map, &[entry("Map (Tier 16)", 2.0)]), Some(2.0));
}

#[test]
fn f5_only_newest_legacy_map_generation_is_priced() {
    let mut map = item("", "Strand Map", 0);
    map.properties = vec![prop("Map Tier", "5")];
    let generation = |n: u32, price: f64| {
        with(entry("Strand Map", price), |k| {
            k.tier = Some(5);
            k.variant = Some(format!(", Gen-{n}"));
        })
    };
    // Gen-10 entries are dropped, so the one Gen-11 entry matches.
    let entries = [generation(10, 1.0), generation(11, 2.0)];
    assert_eq!(price_of(&map, &entries), Some(2.0));
    // An older generation alone is the newest present.
    assert_eq!(price_of(&map, &[generation(10, 1.0)]), Some(1.0));
}

#[test]
fn f5_charged_compass_is_never_priced() {
    let entries = [currency_entry("Charged Compass", 5.0)];
    assert_eq!(price_of(&currency("Charged Compass"), &entries), None);
}

#[test]
fn f5_beast_matches_by_type_line() {
    let mut beast = item("Vile Gnaw", "Fenumal Plagued Arachnid", frame_type::RARE);
    beast.descr_text = Some("Right-click to add this to your bestiary.".to_owned());
    let entries = [entry("Fenumal Plagued Arachnid", 40.0)];
    assert!(ItemDefinition::from_item(&beast).beast);
    assert_eq!(price_of(&beast, &entries), Some(40.0));
}

fn unique_entry(name: &str, calculated: f64) -> PriceEntry {
    with(entry(name, calculated), |k| k.frame_type = Some(3))
}

#[test]
fn f5_unique_matches_by_links() {
    let mut body = item("Tabula Rasa", "Simple Robe", frame_type::UNIQUE);
    let entries = [
        unique_entry("Tabula Rasa", 10.0),
        with(unique_entry("Tabula Rasa", 50.0), |k| k.links = Some(6)),
    ];
    body.sockets = linked(&[0, 0, 0, 1]);
    assert_eq!(price_of(&body, &entries), Some(10.0));
    body.sockets = linked(&[0, 0, 0, 0, 0, 0]);
    assert_eq!(price_of(&body, &entries), Some(50.0));
    // A 5-link matches neither the under-5 nor the 6-link entry.
    body.sockets = linked(&[0, 0, 0, 0, 0, 1]);
    assert_eq!(price_of(&body, &entries), None);
}

#[test]
fn f5_unique_matches_by_variant() {
    let mut jewel = item("Watcher's Eye", "Prismatic Jewel", frame_type::UNIQUE);
    let entries = [
        with(unique_entry("Watcher's Eye", 5.0), |k| {
            k.variant = Some("A".into())
        }),
        with(unique_entry("Watcher's Eye", 9.0), |k| {
            k.variant = Some("B".into())
        }),
    ];
    assert_eq!(price_of(&jewel, &entries), None);
    // An entry without a variant matches any item variant.
    let plain = [unique_entry("Watcher's Eye", 7.0)];
    jewel.explicit_mods = mods(&["3 uses remaining"]);
    assert_eq!(price_of(&jewel, &plain), Some(7.0));
}

#[test]
fn f5_unique_matches_by_quality_with_fallback_to_quality_0() {
    let mut flask = item("Dying Sun", "Ruby Flask", frame_type::UNIQUE);
    flask.properties = vec![prop("Quality", "+20%")];
    let entries = [
        unique_entry("Dying Sun", 30.0),
        with(unique_entry("Dying Sun", 40.0), |k| k.quality = Some(20)),
    ];
    // Both entries survive the quality rule; the quality-0 one is the fallback.
    assert_eq!(price_of(&flask, &entries), Some(30.0));
    let only_20 = [with(unique_entry("Dying Sun", 40.0), |k| {
        k.quality = Some(20)
    })];
    assert_eq!(price_of(&flask, &only_20), Some(40.0));
    flask.properties = vec![prop("Quality", "+10%")];
    assert_eq!(price_of(&flask, &only_20), None);
}

#[test]
fn f5_unique_tincture_and_relic_match() {
    let tincture = item(
        "Grasping Nightshade",
        "Rosethorn Tincture",
        frame_type::UNIQUE,
    );
    assert_eq!(
        price_of(&tincture, &[unique_entry("Grasping Nightshade", 3.0)]),
        Some(3.0)
    );
    // A relic is not a unique frame, so its name is joined with its type line.
    let relic = item("Kaom's Heart", "Glorious Plate", frame_type::RELIC);
    let entries = [unique_entry("Kaom's Heart Glorious Plate", 900.0)];
    assert_eq!(price_of(&relic, &entries), Some(900.0));
    assert_eq!(
        price_of(&relic, &[unique_entry("Kaom's Heart", 900.0)]),
        None
    );
}

fn gem(level: &str, quality: Option<&str>, corrupted: bool) -> Item {
    let mut gem = item("", "Vaal Grace", frame_type::GEM);
    gem.properties = vec![prop("Level", level)];
    gem.properties.extend(quality.map(|q| prop("Quality", q)));
    gem.corrupted = corrupted;
    gem
}

fn gem_entry(level: u32, quality: u32, corrupted: bool, calculated: f64) -> PriceEntry {
    with(entry("Vaal Grace", calculated), |k| {
        k.frame_type = Some(4);
        k.level = Some(level);
        k.quality = Some(quality);
        k.corrupted = Some(corrupted);
    })
}

#[test]
fn f5_gem_matches_by_level_quality_and_corruption() {
    let entries = [
        gem_entry(1, 0, false, 1.0),
        gem_entry(20, 20, false, 20.0),
        gem_entry(21, 20, true, 80.0),
    ];
    assert_eq!(
        price_of(&gem("20 (Max)", Some("+20%"), false), &entries),
        Some(20.0)
    );
    assert_eq!(
        price_of(&gem("21", Some("+20%"), true), &entries),
        Some(80.0)
    );
    // Levels and qualities below 20 match any entry below 20.
    assert_eq!(
        price_of(&gem("13", Some("+7%"), false), &entries),
        Some(1.0)
    );
    assert_eq!(price_of(&gem("13", None, true), &entries), None);
    assert_eq!(price_of(&gem("20", Some("+23%"), false), &entries), None);
}

fn seed(tier: &str) -> Item {
    let mut seed = currency("Wild Ursaling Seed");
    seed.ilvl = 83;
    seed.properties = vec![prop("Spawns a Level {0} Monster when Harvested", tier)];
    seed
}

#[test]
fn f5_seed_matches_by_item_level_band() {
    let entries = [
        with(currency_entry("Wild Ursaling Seed", 1.0), |k| {
            k.ilvl = Some(1)
        }),
        with(currency_entry("Wild Ursaling Seed", 9.0), |k| {
            k.ilvl = Some(76)
        }),
    ];
    assert_eq!(ItemDefinition::from_item(&seed("68")).ilvl, 68);
    assert_eq!(price_of(&seed("68"), &entries), Some(1.0));
    assert_eq!(price_of(&seed("80"), &entries), Some(9.0));
}

#[test]
fn f5_allflame_ember_matches_highest_item_level_not_above() {
    let mut ember = item("", "Allflame Ember of Flesh", frame_type::ALLFLAME_EMBER);
    let level = |ilvl: u32, price: f64| {
        with(entry("Allflame Ember of Flesh", price), |k| {
            k.ilvl = Some(ilvl)
        })
    };
    let entries = [level(68, 2.0), level(83, 8.0), level(73, 4.0)];
    ember.ilvl = 80;
    assert_eq!(price_of(&ember, &entries), Some(4.0));
    ember.ilvl = 86;
    assert_eq!(price_of(&ember, &entries), Some(8.0));
    ember.ilvl = 60;
    assert_eq!(price_of(&ember, &entries), None);
}

#[test]
fn f5_divination_card_needs_inventory_icon() {
    let card = item("", "The Doctor", frame_type::DIVINATION_CARD);
    let art = with(currency_entry("The Doctor", 900.0), |k| {
        k.icon = "x/Divination/Art.png".into()
    });
    let inventory = with(currency_entry("The Doctor", 1000.0), |k| {
        k.icon = "x/Divination/InventoryIcon.png".into()
    });
    assert_eq!(price_of(&card, &[art.clone(), inventory]), Some(1000.0));
    assert_eq!(price_of(&card, &[art]), None);
}

// ---------------------------------------------------------------- options (F5)

#[test]
fn f5_low_confidence_off_ignores_10_or_fewer_listings() {
    let mut thin = currency_entry("Divine Orb", 150.0);
    thin.count = 10;
    let divine = ItemDefinition::from_item(&currency("Divine Orb"));
    let entries = [thin];
    let off = PriceTable::new(&entries, &default_options(), &[]);
    assert_eq!(off.unit_price(&divine), None);
    let on = PricingOptions {
        low_confidence_pricing: true,
        ..default_options()
    };
    assert_eq!(
        PriceTable::new(&entries, &on, &[]).unit_price(&divine),
        Some(150.0)
    );
}

#[test]
fn f5_price_threshold_applies_only_while_total_threshold_is_0() {
    let entries = [currency_entry("Orb of Alchemy", 0.5)];
    let alchemy = ItemDefinition::from_item(&currency("Orb of Alchemy"));
    let per_item = PricingOptions {
        price_threshold: 1.0,
        ..default_options()
    };
    assert_eq!(
        PriceTable::new(&entries, &per_item, &[]).unit_price(&alchemy),
        None
    );
    let both = PricingOptions {
        total_price_threshold: 0.2,
        ..per_item
    };
    assert_eq!(
        PriceTable::new(&entries, &both, &[]).unit_price(&alchemy),
        Some(0.5)
    );
}

#[test]
fn f5_exempt_lifeforce_and_markers_bypass_price_threshold() {
    let entries = [
        currency_entry("Vivid Crystallised Lifeforce", 0.05),
        currency_entry("Wild Crystallised Lifeforce", 0.04),
        currency_entry("Primal Crystallised Lifeforce", 0.03),
        currency_entry("Rogue's Marker", 0.002),
        currency_entry("Orb of Alchemy", 0.5),
    ];
    let vivid = ItemDefinition::from_item(&currency("Vivid Crystallised Lifeforce"));
    let marker = ItemDefinition::from_item(&currency("Rogue's Marker"));
    let alchemy = ItemDefinition::from_item(&currency("Orb of Alchemy"));

    let off = PricingOptions {
        price_threshold: 1.1,
        ..default_options()
    };
    let table_off = PriceTable::new(&entries, &off, &[]);
    assert_eq!(table_off.unit_price(&vivid), None);
    assert_eq!(table_off.unit_price(&marker), None);
    assert_eq!(table_off.unit_price(&alchemy), None);

    let on = PricingOptions {
        exempt_lifeforce_and_markers: true,
        ..off
    };
    let table_on = PriceTable::new(&entries, &on, &[]);
    assert_eq!(table_on.unit_price(&vivid), Some(0.05));
    assert_eq!(table_on.unit_price(&marker), Some(0.002));
    assert_eq!(table_on.unit_price(&alchemy), None);
}

#[test]
fn f5_total_threshold_drops_merged_rows_below_it() {
    let mut alchemy = currency("Orb of Alchemy");
    alchemy.stack_size = Some(3);
    let mut chance = currency("Orb of Chance");
    chance.stack_size = Some(1);
    let items = [alchemy.clone(), alchemy, chance];
    let entries = [
        currency_entry("Orb of Alchemy", 1.0),
        currency_entry("Orb of Chance", 2.0),
    ];
    let options = PricingOptions {
        total_price_threshold: 5.0,
        ..default_options()
    };
    let tab = price_tab(&items, &entries, &options);
    // Two stacks of 3 merge to 6c, which survives; the 2c row does not.
    assert_eq!(tab.rows.len(), 1);
    assert_eq!((tab.rows[0].stack_size, tab.rows[0].total), (6, 6.0));
    assert_eq!(tab.value, 6.0);
}

#[test]
fn f5_tab_value_sums_rows_and_drops_unpriced_items() {
    let mut divine = currency("Divine Orb");
    divine.stack_size = Some(2);
    let items = [divine, currency("Chaos Orb"), currency("Mirror Shard")];
    let entries = [currency_entry("Divine Orb", 150.5)];
    let tab = price_tab(&items, &entries, &default_options());
    assert_eq!(tab.rows.len(), 2);
    assert_eq!(tab.value, 302.0);
}

// ---------------------------------------------------------------- tabs (F3)

#[test]
fn f3_identical_items_merge_with_summed_quantity_and_total() {
    let mut a = currency("Divine Orb");
    a.stack_size = Some(4);
    let mut b = a.clone();
    b.stack_size = Some(6);
    let other = currency("Chaos Orb");
    let items = [a, other, b];
    let tab = price_tab(
        &items,
        &[currency_entry("Divine Orb", 100.0)],
        &default_options(),
    );
    assert_eq!(tab.rows.len(), 2);
    assert_eq!((tab.rows[0].stack_size, tab.rows[0].total), (10, 1000.0));
    assert_eq!((tab.rows[1].stack_size, tab.rows[1].total), (1, 1.0));
}

#[test]
fn f3_character_pseudo_tab_counts_selected_parts() {
    let inventory = [currency("Chaos Orb")];
    let mut equipment = [currency("Chaos Orb")];
    equipment[0].stack_size = Some(2);
    let entries: [PriceEntry; 0] = [];
    let table = PriceTable::new(&entries, &default_options(), &[]);
    let value = |inv, eq| {
        let tab = TabItems::character(&inventory, &equipment, inv, eq);
        assert_eq!(tab.id, TabId::Character);
        table.price_tabs(&[tab]).tabs[0].value
    };
    assert_eq!(value(true, true), 3.0);
    assert_eq!(value(true, false), 1.0);
    assert_eq!(value(false, true), 2.0);
    assert_eq!(value(false, false), 0.0);
}

#[test]
fn f3_snapshot_references_each_surviving_definition_once() {
    let first = [currency("Divine Orb"), currency("Mirror Shard")];
    let second = [currency("Divine Orb"), currency("Chaos Orb")];
    let entries = [currency_entry("Divine Orb", 100.0)];
    let table = PriceTable::new(&entries, &default_options(), &[]);
    let snapshot = table.price_tabs(&[stash(&first), stash(&second)]);
    let names: Vec<_> = snapshot
        .definitions
        .iter()
        .map(|d| (d.definition.name.as_str(), d.price))
        .collect();
    assert_eq!(names, [("Divine Orb", 100.0), ("Chaos Orb", 1.0)]);
    assert_eq!(snapshot.tabs[1].rows[0].definition, 0);
    assert_eq!(snapshot.total_value(), 201.0);
}

// ---------------------------------------------------------------- custom prices (F6)

#[test]
fn f6_custom_price_replaces_a_surviving_price() {
    let entries = [currency_entry("Divine Orb", 150.0)];
    let custom = [CustomPrice {
        key: entries[0].key.clone(),
        value: 120.0,
    }];
    let table = PriceTable::new(&entries, &default_options(), &custom);
    let divine = ItemDefinition::from_item(&currency("Divine Orb"));
    assert_eq!(table.unit_price(&divine), Some(120.0));
}

#[test]
fn f6_custom_price_needs_the_price_to_survive_the_options() {
    let mut thin = currency_entry("Divine Orb", 150.0);
    thin.count = 3;
    let custom = [CustomPrice {
        key: thin.key.clone(),
        value: 120.0,
    }];
    let entries = [thin];
    let table = PriceTable::new(&entries, &default_options(), &custom);
    let divine = ItemDefinition::from_item(&currency("Divine Orb"));
    assert_eq!(table.unit_price(&divine), None);
}

#[test]
fn f6_custom_price_identifies_its_entry_by_every_field() {
    let entries = [currency_entry("Divine Orb", 150.0)];
    // Same name, but the custom price's entry has a quality the entry lacks.
    let custom = [CustomPrice {
        key: with(entries[0].clone(), |k| k.quality = Some(0)).key,
        value: 120.0,
    }];
    let table = PriceTable::new(&entries, &default_options(), &custom);
    let divine = ItemDefinition::from_item(&currency("Divine Orb"));
    assert_eq!(table.unit_price(&divine), Some(150.0));
}

#[test]
fn f6_custom_price_of_0_keeps_the_poe_ninja_price() {
    let entries = [currency_entry("Divine Orb", 150.0)];
    let custom = [CustomPrice {
        key: entries[0].key.clone(),
        value: 0.0,
    }];
    let table = PriceTable::new(&entries, &default_options(), &custom);
    let divine = ItemDefinition::from_item(&currency("Divine Orb"));
    assert_eq!(table.unit_price(&divine), Some(150.0));
}

#[test]
fn f6_f9_currency_rate_is_the_custom_price_if_set() {
    let entries = [
        currency_entry("Divine Orb", 150.0),
        currency_entry("Divine Orb", 140.0),
        currency_entry("Exalted Orb", 12.0),
    ];
    let custom = [CustomPrice {
        key: entries[2].key.clone(),
        value: 15.0,
    }];
    assert_eq!(currency_rate(&entries, &[], "Divine Orb"), Some(150.0));
    assert_eq!(currency_rate(&entries, &custom, "Exalted Orb"), Some(15.0));
    let zero = [CustomPrice {
        key: entries[2].key.clone(),
        value: 0.0,
    }];
    assert_eq!(currency_rate(&entries, &zero, "Exalted Orb"), Some(12.0));
    assert_eq!(currency_rate(&entries, &[], "Mirror of Kalandra"), None);
}

#[test]
fn f6_f9_custom_rate_applies_whatever_the_pricing_options() {
    // Too few listings to survive the options: not priced, yet its custom
    // price is the rate (legacy rates read the unfiltered price list).
    let mut thin = currency_entry("Divine Orb", 150.0);
    thin.count = 3;
    let custom = [CustomPrice {
        key: thin.key.clone(),
        value: 120.0,
    }];
    let entries = [thin];
    let table = PriceTable::new(&entries, &default_options(), &custom);
    let divine = ItemDefinition::from_item(&currency("Divine Orb"));
    assert_eq!(table.unit_price(&divine), None);
    assert_eq!(currency_rate(&entries, &custom, "Divine Orb"), Some(120.0));
}

#[test]
fn f6_price_table_lists_only_the_newest_map_generation() {
    let generation = |n: u32| {
        with(entry("Strand Map", 1.0), |k| {
            k.variant = Some(format!(", Gen-{n}"))
        })
    };
    let entries = [generation(10), entry("Divine Orb", 150.0), generation(11)];
    let listed: Vec<_> = newest_map_generation(&entries)
        .into_iter()
        .map(|p| p.key.variant.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(listed, ["", ", Gen-11"]);
    // Low-confidence and below-threshold entries stay listed.
    let mut thin = entry("Mirror Shard", 0.0);
    thin.count = 1;
    assert_eq!(newest_map_generation(&[thin]).len(), 1);
}

// ---------------------------------------------------------------- deviations

#[test]
fn d1_definition_attributes_come_from_the_ggg_item() {
    let mut amulet = item("Astramentis", "Onyx Amulet", frame_type::UNIQUE);
    amulet.ilvl = 84;
    amulet.properties = vec![prop("Quality", "+12%")];
    amulet.corrupted = true;
    let entry = with(unique_entry("Astramentis", 60.0), |k| {
        k.ilvl = Some(40);
        k.level = Some(1);
    });
    let entries = [entry];
    let table = PriceTable::new(&entries, &default_options(), &[]);
    let items = [amulet];
    let snapshot = table.price_tabs(&[stash(&items)]);
    let definition = &snapshot.definitions[0].definition;
    assert_eq!(snapshot.definitions[0].price, 60.0);
    assert_eq!(
        (definition.ilvl, definition.quality, definition.level),
        (84, 12, 0)
    );
    assert!(definition.corrupted);
}

#[test]
fn d2_zero_price_values_zero() {
    // With a total threshold, 0-priced entries survive the price filter.
    let entries = [currency_entry("Scroll of Wisdom", 0.0)];
    let options = PricingOptions {
        total_price_threshold: 0.1,
        ..default_options()
    };
    let mut wisdom = currency("Scroll of Wisdom");
    wisdom.stack_size = Some(40);
    let table = PriceTable::new(&entries, &options, &[]);
    let definition = ItemDefinition::from_item(&wisdom);
    assert_eq!(table.unit_price(&definition), Some(0.0));
    let tab = table
        .price_tabs(&[stash(std::slice::from_ref(&wisdom))])
        .tabs
        .remove(0);
    assert!(tab.rows.is_empty());
    assert_eq!(tab.value, 0.0);
}

#[test]
fn d29_filled_coffin_is_currency_named_by_type_line() {
    let mut coffin = currency("Filled Coffin");
    coffin.implicit_mods = mods(&["Level 84 Bloodline Corpse"]);
    let definition = ItemDefinition::from_item(&coffin);
    assert_eq!(definition.name, "Filled Coffin");
    assert_eq!(
        price_of(&coffin, &[currency_entry("Filled Coffin", 3.0)]),
        Some(3.0)
    );
    assert_eq!(
        price_of(&coffin, &[entry("Level 84 Bloodline Corpse", 3.0)]),
        None
    );
}

#[test]
fn d30_watchstone_variant_is_its_uses_remaining() {
    let mut watchstone = item("", "Ivory Watchstone", 0);
    watchstone.explicit_mods = mods(&["Your Maps contain 2 additional Packs", "15 uses remaining"]);
    assert_eq!(ItemDefinition::from_item(&watchstone).variant, "15");
}

#[test]
fn d31_impresence_variant_is_the_damage_type_its_mods_name() {
    let mut impresence = item("Impresence", "Onyx Amulet", frame_type::UNIQUE);
    impresence.explicit_mods = mods(&["Adds 9 to 18 Cold Damage", "+60 to maximum Life"]);
    assert_eq!(ItemDefinition::from_item(&impresence).variant, "Cold");
    let variant = |v: &str, price: f64| {
        with(unique_entry("Impresence", price), |k| {
            k.variant = Some(v.to_owned())
        })
    };
    let entries = [
        variant("Fire", 31.0),
        variant("Cold", 30.0),
        variant("Chaos", 115.0),
    ];
    assert_eq!(price_of(&impresence, &entries), Some(30.0));
}

// ---------------------------------------------------------------- poe.ninja mapping (F5)

#[test]
fn f5_ninja_overviews_in_legacy_order() {
    let all: Vec<_> = ninja::overviews().collect();
    assert_eq!(all.len(), 35);
    assert_eq!(all[0], (OverviewKind::Currency, "Currency"));
    assert_eq!(all[2], (OverviewKind::Item, "Incubator"));
    assert_eq!(all[19], (OverviewKind::Exchange, "Currency"));
}

#[test]
fn f5_ninja_currency_line_has_only_name_icon_and_frame_type() {
    let overview: CurrencyOverview = serde_json::from_value(serde_json::json!({
        "lines": [
            {"currencyTypeName": "Divine Orb", "receive": {"value": 150.5, "count": 40}},
            {"currencyTypeName": "Mirror Shard", "receive": null}
        ],
        "currencyDetails": [{"name": "Divine Orb", "icon": "divine.png"}]
    }))
    .unwrap();
    let entries: Vec<_> = overview.price_entries("Currency").collect();
    let mut divine = currency_entry("Divine Orb", 150.5);
    divine.key.icon = "divine.png".into();
    divine.count = 40;
    let mut shard = currency_entry("Mirror Shard", 0.0);
    shard.count = 0;
    assert_eq!(entries, [divine, shard]);
}

#[test]
fn f5_ninja_item_line_reads_tier_from_name_and_elder_from_variant() {
    let overview: ItemOverview = serde_json::from_value(serde_json::json!({
        "lines": [
            {"name": "Map (Tier 16)", "icon": "m.png", "variant": "Elder", "itemClass": 0,
             "chaosValue": 2.5, "count": 12, "sparkLine": {"data": [null, 1.0]}},
            {"name": "Vaal Grace", "variant": null, "itemClass": 4, "gemLevel": 21,
             "gemQuality": 20, "corrupted": true, "levelRequired": 72, "links": 0}
        ]
    }))
    .unwrap();
    let entries: Vec<_> = overview.price_entries("Map").collect();
    let map = &entries[0].key;
    assert_eq!(
        (map.tier, map.elder, map.shaper),
        (Some(16), Some(true), Some(false))
    );
    assert_eq!((entries[0].calculated, entries[0].count), (2.5, 12));
    let gem = &entries[1].key;
    assert_eq!(gem.variant.as_deref(), Some(""));
    assert_eq!(
        (gem.level, gem.quality, gem.ilvl),
        (Some(21), Some(20), Some(72))
    );
    assert_eq!(
        (gem.corrupted, gem.frame_type, gem.tier),
        (Some(true), Some(4), Some(0))
    );
    assert_eq!((entries[1].calculated, entries[1].count), (0.0, 0));
}

#[test]
fn f5_ninja_exchange_line_approximates_count_and_completes_icon() {
    let overview: ExchangeOverview = serde_json::from_value(serde_json::json!({
        "core": {},
        "lines": [
            {"id": "divine", "primaryValue": 150.0, "volumePrimaryValue": 300000.0},
            {"id": "exalted", "primaryValue": 10.0, "volumePrimaryValue": 20.0},
            {"id": "the-doctor", "primaryValue": 0.0},
        ],
        "items": [
            {"id": "divine", "name": "Divine Orb", "image": "/gen/divine.png"},
            {"id": "exalted", "name": "Exalted Orb", "image": "https://x/exalted.png"},
        ]
    }))
    .unwrap();
    let entries: Vec<_> = overview.price_entries("DivinationCard").collect();
    assert_eq!(entries[0].key.icon, "https://web.poecdn.com/gen/divine.png");
    assert_eq!(entries[0].count, 2000);
    assert_eq!(
        (entries[1].key.icon.as_str(), entries[1].count),
        ("https://x/exalted.png", 11)
    );
    assert_eq!(entries[2].key.name, "the-doctor");
    assert!(entries[2].key.icon.ends_with("/InventoryIcon.png"));
    assert_eq!(entries[2].count, 0);
    assert_eq!(
        entries[2].key,
        with(entry("the-doctor", 0.0), |k| {
            k.frame_type = Some(5);
            k.icon = entries[2].key.icon.clone();
        })
        .key
    );
}

#[test]
fn f5_ninja_dedupe_keeps_first_position_and_most_listings() {
    let mut stash_divine = currency_entry("Divine Orb", 150.0);
    stash_divine.count = 20;
    let chaos = currency_entry("Chaos Orb", 1.0);
    let mut exchange_divine = with(entry("Divine Orb", 155.0), |k| k.frame_type = Some(5));
    exchange_divine.count = 900;
    let deduped = ninja::dedupe([stash_divine, chaos.clone(), exchange_divine.clone()]);
    assert_eq!(deduped, [exchange_divine, chaos]);
}

// ---------------------------------------------------------------- trend and poe.ninja page (D3)

fn sparkline(total_change: f64) -> Sparkline {
    Sparkline {
        data: vec![Some(0.0), None, Some(total_change)],
        total_change,
    }
}

#[test]
fn d3_currency_line_trend_follows_confidence_and_links_its_page() {
    let overview: CurrencyOverview = serde_json::from_value(serde_json::json!({
        "lines": [
            {"currencyTypeName": "Divine Orb", "detailsId": "divine-orb",
             "receive": {"value": 150.0, "count": 40},
             "receiveSparkLine": {"data": [0, null, 5.5], "totalChange": 5.5},
             "lowConfidenceReceiveSparkLine": {"data": [0, null, 9], "totalChange": 9}},
            {"currencyTypeName": "Mirror Shard", "detailsId": "mirror-shard",
             "receive": {"value": 1.0, "count": 3},
             "receiveSparkLine": {"data": [0, null, 5.5], "totalChange": 5.5},
             "lowConfidenceReceiveSparkLine": {"data": [0, null, 9], "totalChange": 9}},
            {"currencyTypeName": "Chaos Shard"}
        ]
    }))
    .unwrap();
    let entries: Vec<_> = overview.price_entries("Fragment").collect();
    assert_eq!(entries[0].sparkline, Some(sparkline(5.5)));
    assert_eq!(
        entries[0].details_page.as_deref(),
        Some("fragments/divine-orb")
    );
    assert_eq!(entries[1].sparkline, Some(sparkline(9.0)), "low confidence");
    assert_eq!(
        (&entries[2].sparkline, &entries[2].details_page),
        (&None, &None)
    );
}

#[test]
fn d3_item_line_trend_follows_confidence_and_links_its_page() {
    let overview: ItemOverview = serde_json::from_value(serde_json::json!({
        "lines": [
            {"name": "Headhunter", "count": 50, "detailsId": "headhunter",
             "sparkLine": {"data": [0, null, 5.5], "totalChange": 5.5},
             "lowConfidenceSparkline": {"data": [0, null, 9], "totalChange": 9}},
            {"name": "Mageblood", "count": 2, "detailsId": "mageblood",
             "sparkline": {"data": [0, null, 5.5], "totalChange": 5.5},
             "lowConfidenceSparkline": {"data": [0, null, 9], "totalChange": 9}},
            {"name": "Tabula Rasa", "count": 2,
             "sparkline": {"data": [0, null, 5.5], "totalChange": 5.5}}
        ]
    }))
    .unwrap();
    let entries: Vec<_> = overview.price_entries("UniqueAccessory").collect();
    assert_eq!(entries[0].sparkline, Some(sparkline(5.5)));
    assert_eq!(
        entries[0].details_page.as_deref(),
        Some("unique-accessories/headhunter")
    );
    assert_eq!(entries[1].sparkline, Some(sparkline(9.0)), "low confidence");
    assert_eq!(
        entries[2].sparkline,
        Some(sparkline(5.5)),
        "no low-confidence trend"
    );
    assert_eq!(entries[2].details_page, None);
}

#[test]
fn d3_exchange_line_links_the_items_details_id_else_its_own() {
    let overview: ExchangeOverview = serde_json::from_value(serde_json::json!({
        "lines": [
            {"id": "divine", "primaryValue": 150.0,
             "sparkline": {"data": [0, null, 5.5], "totalChange": 5.5}},
            {"id": "the-doctor", "primaryValue": 900.0}
        ],
        "items": [{"id": "divine", "name": "Divine Orb", "detailsId": "divine-orb"}]
    }))
    .unwrap();
    let entries: Vec<_> = overview.price_entries("DivinationCard").collect();
    assert_eq!(entries[0].sparkline, Some(sparkline(5.5)));
    assert_eq!(
        entries[0].details_page.as_deref(),
        Some("divination-cards/divine-orb")
    );
    assert_eq!(entries[1].sparkline, None);
    assert_eq!(
        entries[1].details_page.as_deref(),
        Some("divination-cards/the-doctor")
    );
}

#[test]
fn d3_details_url_uses_legacy_league_and_type_slugs() {
    assert_eq!(
        ninja::details_url("Standard", "currency/divine-orb"),
        "https://poe.ninja/poe1/economy/standard/currency/divine-orb"
    );
    assert_eq!(
        ninja::details_url("Hardcore Keepers of the Flame", "x/y"),
        "https://poe.ninja/poe1/economy/keepersoftheflamehc/x/y"
    );
    let page = |overview_type| {
        let overview: ItemOverview = serde_json::from_value(serde_json::json!({
            "lines": [{"name": "x", "detailsId": "x"}]
        }))
        .unwrap();
        overview
            .price_entries(overview_type)
            .next()
            .unwrap()
            .details_page
            .unwrap()
    };
    assert_eq!(page("Currency"), "currency/x");
    assert_eq!(page("Fragment"), "fragments/x");
    assert_eq!(page("Prophecy"), "prophecies/x");
    assert_eq!(page("BlightRavagedMap"), "blight-ravaged-maps/x");
    assert_eq!(page("SkillGem"), "skill-gems/x");
}

#[test]
fn d3_matching_price_is_the_entry_that_prices_the_item() {
    let mut map = with(entry("Strand Map", 3.0), |k| k.tier = Some(16));
    map.sparkline = Some(sparkline(-2.0));
    let other_tier = with(entry("Strand Map", 1.0), |k| k.tier = Some(15));
    let entries = [other_tier, map.clone()];
    let table = PriceTable::new(&entries, &PricingOptions::default(), &[]);
    let mut strand = item("", "Strand Map", frame_type::NORMAL);
    strand.properties = vec![prop("Map Tier", "16")];
    let definition = ItemDefinition::from_item(&strand);
    assert_eq!(table.matching_price(&definition), Some(&map));
    assert_eq!(table.unit_price(&definition), Some(3.0));

    let unknown = ItemDefinition::from_item(&item("", "Nothing", frame_type::CURRENCY));
    assert_eq!(table.matching_price(&unknown), None);
}
