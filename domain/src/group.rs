//! The Group-combined Net worth widgets (F13, D15): the selected players'
//! Snapshot totals of their active Profiles, combined.

use crate::net_worth::{self, SPARKLINE_SNAPSHOTS, SnapshotTotal};

/// One selected player's Snapshots as the client holds them.
#[derive(Clone, Copy, Debug)]
pub struct PlayerTotals<'a> {
    /// Newest first: at least the newest [`SPARKLINE_SNAPSHOTS`] + 1 and
    /// those of the last hour.
    pub newest_first: &'a [SnapshotTotal],
    /// All the player's Snapshots held, for the Snapshot count.
    pub count: u32,
}

/// The Net worth widgets over several players.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Combined {
    /// The sum of each player's latest total.
    pub total: f64,
    /// The newest combined total minus the one before it.
    pub change: f64,
    /// The newest [`SPARKLINE_SNAPSHOTS`] combined totals, oldest first.
    pub sparkline: Vec<f64>,
    /// The sum of each player's income over the last hour; a Group has no
    /// Reset.
    pub income: f64,
    pub snapshot_count: u32,
    pub last_snapshot_at: Option<i64>,
}

/// Combines `players` at `now`. The combined total after each Snapshot is
/// the sum of every player's latest total at that time; it starts once every
/// player with Snapshots has one, so a player whose earlier values are
/// unknown never shows as 0. Its last two values give the change, as legacy
/// diffs the latest totals with and without the newest Snapshot
/// (`exilence-ce-legacy/src/store/domains/group.ts`, `lastSnapshotChange`).
pub fn combine(players: &[PlayerTotals<'_>], now: i64) -> Combined {
    let start = players
        .iter()
        .filter_map(|p| p.newest_first.last())
        .map(|s| s.created_at)
        .max();
    let mut snapshots: Vec<(i64, usize, f64)> = players
        .iter()
        .enumerate()
        .flat_map(|(player, p)| {
            p.newest_first
                .iter()
                .map(move |s| (s.created_at, player, s.total))
        })
        .collect();
    snapshots.sort_by_key(|&(created_at, player, _)| (created_at, player));
    let first_in_range = start.map_or(snapshots.len(), |s| {
        snapshots.partition_point(|&(created_at, _, _)| created_at < s)
    });
    let emit_from = snapshots
        .len()
        .saturating_sub(SPARKLINE_SNAPSHOTS)
        .max(first_in_range);
    let mut latest = vec![0.0; players.len()];
    let mut sparkline = Vec::with_capacity(snapshots.len().saturating_sub(emit_from));
    for (i, (_, player, total)) in snapshots.into_iter().enumerate() {
        latest[player] = total;
        if i >= emit_from {
            sparkline.push(latest.iter().sum::<f64>());
        }
    }
    let change = match sparkline.as_slice() {
        [.., previous, newest] => newest - previous,
        _ => 0.0,
    };
    Combined {
        total: sparkline.last().copied().unwrap_or(0.0),
        change,
        sparkline,
        income: players
            .iter()
            .map(|p| net_worth::income(p.newest_first, now, i64::MIN))
            .sum(),
        snapshot_count: players.iter().map(|p| p.count).sum(),
        last_snapshot_at: players
            .iter()
            .filter_map(|p| p.newest_first.first())
            .map(|s| s.created_at)
            .max(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const MINUTE: i64 = 60_000;

    fn totals(snapshots: &[(i64, f64)]) -> Vec<SnapshotTotal> {
        snapshots
            .iter()
            .map(|&(minutes_ago, total)| SnapshotTotal {
                created_at: NOW - minutes_ago * MINUTE,
                total,
            })
            .collect()
    }

    fn player(newest_first: &[SnapshotTotal]) -> PlayerTotals<'_> {
        PlayerTotals {
            newest_first,
            count: newest_first.len() as u32,
        }
    }

    #[test]
    fn f13_no_selected_player_combines_to_nothing() {
        assert_eq!(combine(&[], NOW), Combined::default());
        assert_eq!(combine(&[player(&[])], NOW), Combined::default());
    }

    #[test]
    fn f13_total_sums_each_players_latest_snapshot() {
        // Mine: three Snapshots; a member's latest only, as after joining (D4).
        let mine = totals(&[(10, 300.0), (50, 200.0), (90, 100.0)]);
        let member = totals(&[(70, 1000.0)]);
        let combined = combine(&[player(&mine), player(&member)], NOW);
        assert_eq!(combined.total, 1300.0);
        // Combined after the member's Snapshot: 1100 (70 min ago), 1200, 1300;
        // nothing before it, where the member's value is unknown.
        assert_eq!(combined.sparkline, [1100.0, 1200.0, 1300.0]);
        assert_eq!(combined.change, 100.0, "my newest Snapshot changed it");
        assert_eq!(combined.snapshot_count, 4);
        assert_eq!(combined.last_snapshot_at, Some(NOW - 10 * MINUTE));
    }

    #[test]
    fn f13_change_follows_the_groups_newest_snapshot() {
        let mine = totals(&[(30, 300.0), (60, 200.0)]);
        let member = totals(&[(5, 900.0), (40, 1000.0)]);
        let combined = combine(&[player(&mine), player(&member)], NOW);
        assert_eq!(combined.total, 1200.0);
        assert_eq!(combined.change, -100.0, "the member lost 100 last");
        assert_eq!(combined.sparkline, [1200.0, 1300.0, 1200.0]);
    }

    #[test]
    fn f13_sparkline_holds_the_newest_10_combined_totals() {
        let mine: Vec<SnapshotTotal> = totals(
            &(0..15)
                .map(|i| (i64::from(i) * 2, f64::from(100 - i)))
                .collect::<Vec<_>>(),
        );
        let member = totals(&[(100, 1000.0)]);
        let combined = combine(&[player(&mine), player(&member)], NOW);
        assert_eq!(
            combined.sparkline,
            [
                1091.0, 1092.0, 1093.0, 1094.0, 1095.0, 1096.0, 1097.0, 1098.0, 1099.0, 1100.0
            ]
        );
    }

    #[test]
    fn f13_d15_income_sums_each_selected_players_last_hour() {
        let mine = totals(&[(0, 500.0), (30, 300.0), (59, 200.0), (61, 50.0)]);
        // One Snapshot in the hour: no income.
        let member = totals(&[(10, 1000.0), (120, 100.0)]);
        let other = totals(&[(5, 80.0), (20, 100.0)]);
        let combined = combine(&[player(&mine), player(&member), player(&other)], NOW);
        assert_eq!(combined.income, 300.0 + 0.0 - 20.0);
    }
}
