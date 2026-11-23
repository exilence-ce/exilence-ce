//! The item table's rows (F11): a Snapshot's rows of every tab merged into
//! one row per item, as legacy `mergeItemStacks` merges with `findItem`
//! (`exilence-ce-legacy/src/utils/item.utils.ts:11-35,118-132`), the Comparison of
//! two Snapshots, and the Group Comparison merged over players (D14). In a
//! Group the selected players' newest Snapshots merge like the tabs of one.
//! Legacy merges looser than definitions differ: a map's quality, a
//! non-seed's item level and variants do not split rows.

use std::collections::HashMap;

use crate::item_definition::{ItemDefinition, frame_type};

/// One row of a Snapshot tab: all of the tab's items with one definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabRow<'a> {
    /// The tab's position in the Snapshot.
    pub tab: usize,
    pub definition: &'a ItemDefinition,
    pub stack_size: u32,
    /// Chaos per unit.
    pub price: f64,
}

/// The tab rows of one item, merged.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedRow<'a> {
    /// The first merged row's definition: legacy shows its fields.
    pub definition: &'a ItemDefinition,
    /// The first merged row's unit price, in chaos.
    pub price: f64,
    pub quantity: u64,
    /// Each tab holding the item with its quantity there, in the order the
    /// tabs were first seen.
    pub tabs: Vec<(usize, u64)>,
}

impl MergedRow<'_> {
    /// Quantity × price, as legacy recomputes it on every merge.
    pub fn total(&self) -> f64 {
        self.quantity as f64 * self.price
    }
}

/// Merges `rows` into one row per item, in the order items are first seen.
pub fn merge<'a>(rows: impl IntoIterator<Item = TabRow<'a>>) -> Vec<MergedRow<'a>> {
    let mut merged: Vec<MergedRow<'a>> = Vec::new();
    // Every legacy rule compares the name first.
    let mut by_name: HashMap<&'a str, Vec<usize>> = HashMap::new();
    for row in rows {
        let quantity = u64::from(row.stack_size);
        let named = by_name.entry(row.definition.name.as_str()).or_default();
        let found = named
            .iter()
            .copied()
            .find(|&i| same_item(merged[i].definition, row.definition));
        match found {
            Some(i) => {
                let item = &mut merged[i];
                item.quantity += quantity;
                match item.tabs.iter_mut().find(|(tab, _)| *tab == row.tab) {
                    Some((_, in_tab)) => *in_tab += quantity,
                    None => item.tabs.push((row.tab, quantity)),
                }
            }
            None => {
                named.push(merged.len());
                merged.push(MergedRow {
                    definition: row.definition,
                    price: row.price,
                    quantity,
                    tabs: vec![(row.tab, quantity)],
                });
            }
        }
    }
    merged
}

/// Legacy `findItem(merged, item)`: whether `item` merges into the row whose
/// first definition is `merged`. The rule is not symmetric.
fn same_item(merged: &ItemDefinition, item: &ItemDefinition) -> bool {
    let map = merged.type_line.contains(" Map");
    merged.name == item.name
        && (map || merged.quality == item.quality)
        && merged.links == item.links
        && merged.level == item.level
        && merged.corrupted == item.corrupted
        && (!merged.type_line.contains(" Seed") || map || merged.ilvl == item.ilvl)
        // The frame type counts, except for maps that are not unique.
        && (merged.frame_type == item.frame_type
            || (merged.name.contains(" Map") && merged.frame_type != frame_type::UNIQUE))
}

/// One row of the Comparison view: how an item changed between two
/// Snapshots.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference<'a> {
    pub definition: &'a ItemDefinition,
    /// Chaos per unit: the newer Snapshot's, the older's for a vanished item.
    pub price: f64,
    /// The change in quantity.
    pub quantity: i64,
    /// The change in total value, in chaos.
    pub total: f64,
}

/// Legacy `diffSnapshots(older, newer)` (`exilence-ce-legacy/src/utils/snapshot.utils.ts:118-162`)
/// over the merged rows of two Snapshots: changed items with the difference
/// valued at the newer price, then new items as they are, then vanished
/// items negated. Rows without a change in quantity or value are dropped.
/// A Snapshot without item detail passes no rows.
pub fn compare<'a>(older: &[MergedRow<'a>], newer: &[MergedRow<'a>]) -> Vec<Difference<'a>> {
    let find = |rows: &[MergedRow<'a>], item: &MergedRow<'a>| {
        rows.iter()
            .position(|r| same_item(r.definition, item.definition))
    };
    let signed = |quantity: u64| i64::try_from(quantity).unwrap_or(i64::MAX);
    let mut changed = Vec::new();
    let mut added = Vec::new();
    for item in newer {
        match find(older, item) {
            Some(i) if older[i].quantity != item.quantity => changed.push(Difference {
                definition: item.definition,
                price: item.price,
                quantity: signed(item.quantity) - signed(older[i].quantity),
                total: item.total() - item.price * older[i].quantity as f64,
            }),
            Some(_) => {}
            None => added.push(Difference {
                definition: item.definition,
                price: item.price,
                quantity: signed(item.quantity),
                total: item.total(),
            }),
        }
    }
    let vanished = older
        .iter()
        .filter(|item| find(newer, item).is_none())
        .map(|item| Difference {
            definition: item.definition,
            price: item.price,
            quantity: -signed(item.quantity),
            total: -item.total().abs(),
        });
    changed
        .into_iter()
        .chain(added)
        .chain(vanished)
        .filter(|d| d.quantity != 0 && d.total != 0.0)
        .collect()
}

/// The Group Comparison (D14): each selected player's [`compare`] of their
/// own two newest Snapshots, in selection order, merged into one table. The
/// rows of one item add up their changes in quantity and value and keep the
/// first row's definition and price; items stay in the order first seen.
/// Rows whose changes cancel out are dropped, as [`compare`] drops them.
pub fn merge_comparisons<'a>(
    players: impl IntoIterator<Item = Vec<Difference<'a>>>,
) -> Vec<Difference<'a>> {
    let mut merged: Vec<Difference<'a>> = Vec::new();
    for difference in players.into_iter().flatten() {
        match merged
            .iter_mut()
            .find(|m| same_item(m.definition, difference.definition))
        {
            Some(item) => {
                item.quantity += difference.quantity;
                item.total += difference.total;
            }
            None => merged.push(difference),
        }
    }
    merged.retain(|d| d.quantity != 0 && d.total != 0.0);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(name: &str, type_line: &str, frame_type: u32) -> ItemDefinition {
        ItemDefinition {
            name: name.to_owned(),
            type_line: type_line.to_owned(),
            frame_type,
            ..ItemDefinition::default()
        }
    }

    fn row(tab: usize, definition: &ItemDefinition, stack_size: u32, price: f64) -> TabRow<'_> {
        TabRow {
            tab,
            definition,
            stack_size,
            price,
        }
    }

    #[test]
    fn f11_latest_merges_an_item_across_tabs_with_its_quantity_per_tab() {
        let chaos = definition("Chaos Orb", "Chaos Orb", frame_type::CURRENCY);
        let divine = definition("Divine Orb", "Divine Orb", frame_type::CURRENCY);
        let merged = merge([
            row(0, &divine, 2, 150.0),
            row(0, &chaos, 7, 1.0),
            row(1, &chaos, 10, 1.0),
            row(2, &chaos, 50, 1.0),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].definition, &divine);
        assert_eq!((merged[0].quantity, merged[0].total()), (2, 300.0));
        assert_eq!(merged[1].quantity, 67);
        assert_eq!(merged[1].total(), 67.0);
        assert_eq!(merged[1].tabs, [(0, 7), (1, 10), (2, 50)]);
    }

    #[test]
    fn f11_merged_row_keeps_the_first_rows_fields_and_price() {
        let mut elder = definition("Strand Map", "Strand Map", frame_type::NORMAL);
        elder.tier = 16;
        elder.quality = 20;
        elder.elder = true;
        let mut plain = elder.clone();
        plain.quality = 0;
        plain.elder = false;
        plain.frame_type = frame_type::RARE;
        let merged = merge([row(0, &elder, 1, 5.0), row(0, &plain, 3, 2.0)]);
        assert_eq!(
            merged.len(),
            1,
            "maps ignore quality and non-unique frame type"
        );
        assert_eq!(merged[0].definition, &elder);
        assert_eq!((merged[0].quantity, merged[0].total()), (4, 20.0));
        assert_eq!(merged[0].tabs, [(0, 4)]);
    }

    #[test]
    fn f11_latest_keeps_items_apart_as_legacy_find_item_does() {
        let gem = |level, quality, corrupted| ItemDefinition {
            level,
            quality,
            corrupted,
            ..definition("Vaal Grace", "Vaal Grace", frame_type::GEM)
        };
        let (a, b, c, d) = (
            gem(20, 0, false),
            gem(21, 0, false),
            gem(20, 20, false),
            gem(20, 0, true),
        );
        let merged = merge([&a, &b, &c, &d].map(|g| row(0, g, 1, 1.0)));
        assert_eq!(merged.len(), 4, "level, quality and corruption split rows");

        let seed = |ilvl| ItemDefinition {
            ilvl,
            ..definition(
                "Wild Thornfruit",
                "Wild Thornfruit Seed",
                frame_type::CURRENCY,
            )
        };
        let (low, high) = (seed(60), seed(80));
        assert_eq!(
            merge([row(0, &low, 1, 1.0), row(1, &high, 1, 1.0)]).len(),
            2
        );

        let currency = definition("Ancient Orb", "Ancient Orb", frame_type::CURRENCY);
        let relic = ItemDefinition {
            frame_type: frame_type::RELIC,
            ..currency.clone()
        };
        assert_eq!(
            merge([row(0, &currency, 1, 1.0), row(0, &relic, 1, 1.0)]).len(),
            2,
            "frame type counts outside non-unique maps"
        );

        let linked = ItemDefinition {
            links: 6,
            ..definition("Tabula Rasa", "Simple Robe", frame_type::UNIQUE)
        };
        let unlinked = ItemDefinition {
            links: 0,
            ..linked.clone()
        };
        assert_eq!(
            merge([row(0, &linked, 1, 1.0), row(0, &unlinked, 1, 1.0)]).len(),
            2
        );
    }

    fn differences(rows: &[Difference]) -> Vec<(String, i64, f64, f64)> {
        rows.iter()
            .map(|d| (d.definition.name.clone(), d.quantity, d.price, d.total))
            .collect()
    }

    #[test]
    fn f11_comparison_diffs_changed_new_and_vanished_items() {
        let chaos = definition("Chaos Orb", "Chaos Orb", frame_type::CURRENCY);
        let divine = definition("Divine Orb", "Divine Orb", frame_type::CURRENCY);
        let exalted = definition("Exalted Orb", "Exalted Orb", frame_type::CURRENCY);
        let mirror = definition("Mirror of Kalandra", "Mirror", frame_type::CURRENCY);
        let ancient = definition("Ancient Orb", "Ancient Orb", frame_type::CURRENCY);
        let older = merge([
            row(0, &divine, 2, 140.0),
            row(0, &chaos, 17, 1.0),
            row(0, &exalted, 1, 12.0),
            row(1, &mirror, 1, 0.0),
        ]);
        let newer = merge([
            row(0, &chaos, 10, 1.0),
            row(0, &divine, 5, 150.0),
            row(1, &exalted, 1, 13.0),
            row(1, &ancient, 3, 2.0),
        ]);
        assert_eq!(
            differences(&compare(&older, &newer)),
            [
                // Changed: the difference at the newer price.
                ("Chaos Orb".to_owned(), -7, 1.0, -7.0),
                ("Divine Orb".to_owned(), 3, 150.0, 450.0),
                // New as it is; Exalted Orb kept its quantity and the
                // vanished mirror had no value: both dropped.
                ("Ancient Orb".to_owned(), 3, 2.0, 6.0),
            ]
        );
        assert_eq!(
            differences(&compare(&newer, &older)),
            [
                ("Divine Orb".to_owned(), -3, 140.0, -420.0),
                ("Chaos Orb".to_owned(), 7, 1.0, 7.0),
                // Vanished: negated.
                ("Ancient Orb".to_owned(), -3, 2.0, -6.0),
            ]
        );
    }

    #[test]
    fn f11_comparison_treats_a_snapshot_without_item_detail_as_empty() {
        let chaos = definition("Chaos Orb", "Chaos Orb", frame_type::CURRENCY);
        let rows = merge([row(0, &chaos, 4, 1.0)]);
        assert_eq!(
            differences(&compare(&[], &rows)),
            [("Chaos Orb".to_owned(), 4, 1.0, 4.0)]
        );
        assert_eq!(
            differences(&compare(&rows, &[])),
            [("Chaos Orb".to_owned(), -4, 1.0, -4.0)]
        );
        assert!(compare(&rows, &rows).is_empty(), "nothing changed");
    }

    #[test]
    fn d14_group_comparison_diffs_each_players_own_snapshots_then_merges() {
        let chaos = definition("Chaos Orb", "Chaos Orb", frame_type::CURRENCY);
        let divine = definition("Divine Orb", "Divine Orb", frame_type::CURRENCY);
        let exalted = definition("Exalted Orb", "Exalted Orb", frame_type::CURRENCY);
        let mirror = definition("Mirror of Kalandra", "Mirror", frame_type::CURRENCY);
        // The first player gained chaos and a divine; the second lost chaos,
        // gained exalts, and their divine change cancels the first's.
        let first = compare(
            &merge([row(0, &chaos, 10, 1.0)]),
            &merge([row(0, &chaos, 15, 1.0), row(0, &divine, 1, 150.0)]),
        );
        let second = compare(
            &merge([row(0, &chaos, 30, 1.0), row(0, &divine, 2, 140.0)]),
            &merge([
                row(0, &chaos, 28, 1.0),
                row(0, &exalted, 4, 12.0),
                row(0, &divine, 1, 150.0),
            ]),
        );
        // A player whose two newest Snapshots hold the same adds nothing;
        // legacy diffed one player's latest against another's instead.
        let third = compare(
            &merge([row(0, &mirror, 1, 9000.0)]),
            &merge([row(0, &mirror, 1, 9000.0)]),
        );

        assert_eq!(
            differences(&merge_comparisons([first, second, third])),
            [
                ("Chaos Orb".to_owned(), 3, 1.0, 3.0),
                ("Exalted Orb".to_owned(), 4, 12.0, 48.0),
            ],
            "chaos 5 - 2; the divine's +1 and -1 cancel out"
        );
        assert!(merge_comparisons(Vec::<Vec<Difference>>::new()).is_empty());
    }
}
