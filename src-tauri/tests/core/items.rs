//! The item table (F11): the `items` slice from the newest Snapshot, its
//! trends and poe.ninja pages (D3), the Comparison rows, the CSV export and
//! the remembered layout, against the `wiremock` GGG and poe.ninja of the
//! Snapshot tests.

use std::collections::BTreeMap;
use std::time::Duration;

use exilence_ce_ng_lib::scheduler::Scheduler;
use exilence_ce_ng_lib::{csv_export, items, links, prices, profiles};
use ipc::command::{ExportCsv, ExportCsvArgs, SettingsPatch, UpdateSettings, UpdateSettingsArgs};
use ipc::slices::{ColumnPreset, ItemColumn, ItemTab, ItemTableSort, Items, PriceTrend, Settings};
use ipc::{Field, IpcError, MessageKey};
use tauri::Manager;
use tauri::async_runtime::block_on;
use wiremock::Mock;
use wiremock::matchers::{path, query_param};

use super::fake_backend::ACCOUNT_ID;
use super::prices::{CURRENCY, currency_overview};
use super::snapshot::{LEAGUE, PROFILE_ID, Setup, currency, ggg_with_stash, ok, profile};
use super::{Harness, settings_slice, update_settings};

const DIVINE_PAGE: &str = "https://poe.ninja/poe1/economy/settlers/currency/divine-orb";

/// A row as (name, quantity, price, total, tabs as (tab, quantity)).
type Row = (String, i64, f64, f64, Vec<(u32, i64)>);

fn rows(items: &Items) -> Vec<Row> {
    items
        .rows
        .iter()
        .map(|r| {
            let tabs = r.tabs.iter().map(|t| (t.tab, t.quantity)).collect();
            (r.name.clone(), r.quantity, r.price, r.total, tabs)
        })
        .collect()
}

fn row(name: &str, quantity: i64, price: f64, tabs: &[(u32, i64)]) -> Row {
    let total = quantity as f64 * price;
    (name.to_owned(), quantity, price, total, tabs.to_vec())
}

#[test]
fn f11_d3_items_hold_the_newest_snapshots_rows_with_tab_names_trends_and_pages() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    assert_eq!(
        setup.read(Items::clone),
        Items::default(),
        "no Snapshot yet"
    );

    setup.take_and_wait();

    // Chaos Orb lies in `Currency` and in `Maps`' child tab: one row.
    let items = setup.read(Items::clone);
    let tab = |id: &str, name: &str| ItemTab {
        id: id.to_owned(),
        name: name.to_owned(),
    };
    assert_eq!(
        items.tabs,
        [
            tab("t1", "Currency"),
            tab("t2", "Maps"),
            tab("", "Character")
        ]
    );
    assert_eq!(
        rows(&items),
        [
            row("Divine Orb", 2, 150.0, &[(0, 2)]),
            row("Chaos Orb", 17, 1.0, &[(0, 7), (1, 10)]),
            row("Exalted Orb", 1, 12.0, &[(2, 1)]),
        ]
    );
    let divine = &items.rows[0];
    assert_eq!(
        (divine.icon.as_str(), divine.frame_type),
        ("https://web.poecdn.com/Divine Orb.png", 5)
    );
    // D3: from the current price table of the Snapshot's price league.
    assert_eq!(
        divine.trend,
        Some(PriceTrend {
            points: vec![Some(0.0), None, Some(4.5)],
            change: 4.5,
        })
    );
    assert_eq!(divine.ninja_url.as_deref(), Some(DIVINE_PAGE));
    assert_eq!(
        (&items.rows[1].trend, &items.rows[1].ninja_url),
        (&None, &None)
    );
    assert_eq!(items.rows[2].trend, None, "the exchange line has no trend");
    assert_eq!(
        items.rows[2].ninja_url.as_deref(),
        Some("https://poe.ninja/poe1/economy/settlers/currency/exalted")
    );
    assert!(links::allowed(DIVINE_PAGE).is_some(), "open_link opens it");

    // A newer Snapshot replaces the rows.
    block_on(
        Mock::given(path(format!("/stash/{LEAGUE}/t1")))
            .respond_with(ok(serde_json::json!({ "stash": {
                "id": "t1", "name": "Currency", "items": [currency("Divine Orb", 5)],
            }})))
            .with_priority(1)
            .mount(&setup.ggg),
    );
    setup.take_and_wait();
    let items = setup.read(Items::clone);
    assert_eq!(
        rows(&items),
        [
            row("Divine Orb", 5, 150.0, &[(0, 5)]),
            row("Chaos Orb", 10, 1.0, &[(1, 10)]),
            row("Exalted Orb", 1, 12.0, &[(2, 1)]),
        ]
    );

    // The Profile switch follows the active Profile.
    let other = wire::v1::Profile {
        id: "01a1cd36-0000-7000-8000-0000000000bb".to_owned(),
        name: "profile 2".to_owned(),
        ..profile()
    };
    let switch_to = |active: &str| {
        block_on(profiles::replace(
            &setup.harness.pool(),
            ACCOUNT_ID,
            &[profile(), other.clone()],
            active,
        ))
        .unwrap();
        block_on(profiles::push(setup.harness.app.handle()));
    };
    switch_to(&other.id);
    assert_eq!(setup.read(Items::clone), Items::default());
    switch_to(PROFILE_ID);
    assert_eq!(rows(&setup.read(Items::clone)), rows(&items));

    // After a restart the rows are there before prices load; trends and
    // pages follow them.
    let setup = setup.restart();
    let restored = setup.read(Items::clone);
    assert_eq!(rows(&restored), rows(&items));
    assert!(
        restored
            .rows
            .iter()
            .all(|r| r.trend.is_none() && r.ninja_url.is_none())
    );
}

/// How many Snapshot bodies the `items` pushes of `setup`'s app decoded and
/// item price tables they built.
fn items_work(setup: &Setup) -> (u64, u64) {
    let cache = setup.harness.app.state::<items::Cache>();
    (cache.decoded(), cache.tables_built())
}

#[test]
fn f11_a_snapshot_pushes_its_items_without_decoding_a_body() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    setup.take_and_wait();
    assert_eq!(items_work(&setup).0, 0, "the new body is held in memory");

    block_on(
        Mock::given(path(format!("/stash/{LEAGUE}/t1")))
            .respond_with(ok(serde_json::json!({ "stash": {
                "id": "t1", "name": "Currency", "items": [currency("Divine Orb", 5)],
            }})))
            .with_priority(1)
            .mount(&setup.ggg),
    );
    setup.take_and_wait();
    let items = setup.read(Items::clone);
    assert_eq!(
        comparison_rows(&items),
        [
            row("Divine Orb", 3, 150.0, &[]),
            row("Chaos Orb", -7, 1.0, &[]),
        ]
    );
    assert_eq!(items_work(&setup).0, 0, "the previous one is still held");
    block_on(profiles::push(setup.harness.app.handle()));
    assert_eq!(items_work(&setup).0, 0, "a later push decodes nothing");

    // A start decodes the two stored bodies, once.
    let setup = setup.restart();
    assert_eq!(items_work(&setup).0, 2);
    block_on(profiles::push(setup.harness.app.handle()));
    assert_eq!(items_work(&setup).0, 2);
    assert_eq!(
        comparison_rows(&setup.read(Items::clone)),
        comparison_rows(&items)
    );
}

#[test]
fn d3_the_item_price_table_is_rebuilt_only_when_prices_or_options_change() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    setup.take_and_wait();
    assert_eq!(items_work(&setup).1, 1);
    let app = setup.harness.app.handle();

    // Another price league's load pushes `items` with the same table.
    block_on(prices::ensure(app, "Standard")).unwrap();
    assert_eq!(items_work(&setup).1, 1);
    setup.take_and_wait();
    assert_eq!(items_work(&setup).1, 1, "a Snapshot keeps it");

    // A refresh of the Snapshot's price league with the same prices.
    setup.harness.fakes.clock.advance(prices::MAX_AGE_MS);
    block_on(prices::ensure(app, LEAGUE)).unwrap();
    assert_eq!(items_work(&setup).1, 1, "the same prices keep it");

    // New prices of the Snapshot's price league.
    let mut overview = currency_overview();
    overview["lines"][0]["receive"]["value"] = 160.0.into();
    block_on(
        Mock::given(path(CURRENCY))
            .and(query_param("type", "Currency"))
            .respond_with(ok(overview))
            .with_priority(1)
            .mount(&setup.ninja),
    );
    setup.harness.fakes.clock.advance(prices::MAX_AGE_MS);
    block_on(prices::ensure(app, LEAGUE)).unwrap();
    assert_eq!(items_work(&setup).1, 2);
    assert_eq!(
        setup.read(Items::clone).rows[0].ninja_url.as_deref(),
        Some(DIVINE_PAGE)
    );

    // Other pricing options.
    update_settings(
        &setup.harness,
        SettingsPatch {
            low_confidence_pricing: Some(true),
            ..SettingsPatch::default()
        },
    );
    block_on(profiles::push(app));
    assert_eq!(items_work(&setup).1, 3);
}

/// The comparison rows as (name, quantity, price, total, tabs).
fn comparison_rows(items: &Items) -> Vec<Row> {
    rows(&Items {
        rows: items.comparison_rows.clone(),
        ..Items::default()
    })
}

#[test]
fn f11_comparison_diffs_the_two_newest_snapshots() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    setup.take_and_wait();
    assert_eq!(
        setup.read(Items::clone).comparison_rows,
        [],
        "empty with fewer than 2 Snapshots"
    );

    block_on(
        Mock::given(path(format!("/stash/{LEAGUE}/t1")))
            .respond_with(ok(serde_json::json!({ "stash": {
                "id": "t1", "name": "Currency", "items": [currency("Divine Orb", 5)],
            }})))
            .with_priority(1)
            .mount(&setup.ggg),
    );
    setup.take_and_wait();
    let expected = [
        row("Divine Orb", 3, 150.0, &[]),
        row("Chaos Orb", -7, 1.0, &[]),
    ];
    assert_eq!(
        comparison_rows(&setup.read(Items::clone)),
        expected,
        "the unchanged Exalted Orb is dropped"
    );
    let items = setup.read(Items::clone);
    assert_eq!(
        items.comparison_rows[0].ninja_url.as_deref(),
        Some(DIVINE_PAGE)
    );
    let restored = setup.restart();
    assert_eq!(comparison_rows(&restored.read(Items::clone)), expected);
}

#[test]
fn f11_csv_export_saves_the_given_rows_with_title_bom_and_file_name() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    setup.take_and_wait();
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("export.csv");
    *setup.harness.desktop.save_to.lock().unwrap() = Some(file.clone());
    // The UI's filtered rows in its order: Chaos Orb, then Divine Orb.
    let keys = setup.read(|items: &Items| {
        ["Chaos Orb", "Divine Orb"].map(|name| {
            let row = items.rows.iter().find(|r| r.name == name).unwrap();
            row.key.clone()
        })
    });
    setup
        .harness
        .invoke::<ExportCsv>(&ExportCsvArgs {
            row_keys: keys.to_vec(),
        })
        .unwrap();

    let at = csv_export::local(setup.harness.app.state::<Scheduler>().now());
    assert_eq!(
        *setup.harness.desktop.suggested.lock().unwrap(),
        [at.format("Export_%Y-%m-%d-%H-%M.csv").to_string()]
    );
    let text = std::fs::read(&file).unwrap();
    assert!(text.starts_with(b"\xEF\xBB\xBF"), "UTF-8 BOM");
    let expected = format!(
        "\u{feff}Data from {}\r\n\n\
         name,frameType,ilvl,tier,corrupted,links,quality,level,stackSize,calculated,total,tab,icon,detailsUrl\r\n\
         \"Chaos Orb\",5,0,0,FALSE,0,0,0,17,1,17,\"Currency, Maps\",\"https://web.poecdn.com/Chaos Orb.png\",\"\"\r\n\
         \"Divine Orb\",5,0,0,FALSE,0,0,0,2,150,300,\"Currency\",\"https://web.poecdn.com/Divine Orb.png\",\"{DIVINE_PAGE}\"\r\n",
        at.format("%Y-%m-%d %H:%M")
    );
    assert_eq!(String::from_utf8(text).unwrap(), expected);

    // Cancelling the dialog saves nothing.
    std::fs::remove_file(&file).unwrap();
    *setup.harness.desktop.save_to.lock().unwrap() = None;
    setup
        .harness
        .invoke::<ExportCsv>(&ExportCsvArgs {
            row_keys: keys.to_vec(),
        })
        .unwrap();
    assert!(!file.exists());
}

#[test]
fn f11_item_table_layout_is_stored_and_survives_a_restart() {
    let harness = Harness::new();
    let mut presets = ColumnPreset::legacy();
    presets[1].hidden_columns = vec![ItemColumn::Tabs];
    let layout = SettingsPatch {
        item_table_sort: Some(ItemTableSort {
            column: ItemColumn::Name,
            descending: false,
        }),
        item_table_column_widths: Some(BTreeMap::from([
            (ItemColumn::Name, 320),
            (ItemColumn::Tabs, 90),
        ])),
        item_table_page_size: Some(10),
        item_table_presets: Some(presets.clone()),
        item_table_active_preset: Some(presets[1].name.clone()),
        ..SettingsPatch::default()
    };
    update_settings(&harness, layout.clone());
    let expected = Settings {
        item_table_sort: layout.item_table_sort.unwrap(),
        item_table_column_widths: layout.item_table_column_widths.clone().unwrap(),
        item_table_page_size: 10,
        item_table_presets: presets,
        item_table_active_preset: "breachstones".to_owned(),
        ..Settings::default()
    };
    assert_eq!(settings_slice(&harness), expected);

    for size in [0, 7, 100] {
        let refused = harness.invoke::<UpdateSettings>(&UpdateSettingsArgs {
            patch: SettingsPatch {
                item_table_page_size: Some(size),
                ..SettingsPatch::default()
            },
        });
        assert_eq!(
            refused,
            Err(IpcError::Validation {
                field: Field::ItemTablePageSize,
                key: MessageKey::ErrorSettingOutOfRange,
            }),
            "{size}"
        );
    }

    let harness = harness.restart();
    assert_eq!(settings_slice(&harness), expected);
}

#[test]
fn f10_item_table_section_open_state_is_stored_and_survives_a_restart() {
    let harness = Harness::new();
    assert!(settings_slice(&harness).items_open, "open by default");
    update_settings(
        &harness,
        SettingsPatch {
            items_open: Some(false),
            ..SettingsPatch::default()
        },
    );
    let expected = Settings {
        items_open: false,
        ..Settings::default()
    };
    assert_eq!(settings_slice(&harness), expected);
    assert_eq!(settings_slice(&harness.restart()), expected);
}

#[test]
fn f11_item_table_layout_defaults_to_legacy() {
    let settings = Settings::default();
    assert_eq!(
        settings.item_table_sort,
        ItemTableSort {
            column: ItemColumn::Total,
            descending: true,
        }
    );
    assert_eq!(settings.item_table_page_size, 25);
    assert!(settings.item_table_column_widths.is_empty());
    let active = settings
        .item_table_presets
        .iter()
        .find(|p| p.name == settings.item_table_active_preset)
        .unwrap();
    assert_eq!(
        active.hidden_columns,
        [
            ItemColumn::Quality,
            ItemColumn::Level,
            ItemColumn::Corrupted,
            ItemColumn::IlvlTier
        ]
    );
    assert_eq!(settings.item_table_presets.len(), 11);
}
