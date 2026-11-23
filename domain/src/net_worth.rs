//! Net worth widget math (F8): change, sparkline and income from the
//! Profile's Snapshot totals in chaos.

/// How many of the newest Snapshots the sparkline shows.
pub const SPARKLINE_SNAPSHOTS: usize = 10;

/// The longest income window, in ms: one hour.
pub const INCOME_WINDOW_MS: i64 = 60 * 60 * 1000;

/// A Snapshot's total value in chaos and when it was taken (Unix ms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotTotal {
    pub created_at: i64,
    pub total: f64,
}

/// The newest total minus the second newest; 0 with fewer than 2 Snapshots.
/// `newest_first` is ordered newest first.
pub fn change(newest_first: &[SnapshotTotal]) -> f64 {
    match newest_first {
        [newest, previous, ..] => newest.total - previous.total,
        _ => 0.0,
    }
}

/// The totals of the newest [`SPARKLINE_SNAPSHOTS`], oldest first.
pub fn sparkline(newest_first: &[SnapshotTotal]) -> Vec<f64> {
    newest_first
        .iter()
        .take(SPARKLINE_SNAPSHOTS)
        .rev()
        .map(|s| s.total)
        .collect()
}

/// Where the income window starts: the later of one hour before `now` and
/// the last reset. Only Snapshots taken after it count.
pub fn income_window_start(now: i64, reset_at: i64) -> i64 {
    (now - INCOME_WINDOW_MS).max(reset_at)
}

/// Income per hour: the newest minus the oldest total of the Snapshots
/// taken after [`income_window_start`]; 0 with fewer than 2. Like legacy,
/// the difference is not scaled to the window's length.
pub fn income(newest_first: &[SnapshotTotal], now: i64, reset_at: i64) -> f64 {
    let start = income_window_start(now, reset_at);
    let mut in_window = newest_first.iter().filter(|s| s.created_at > start);
    match (in_window.next(), in_window.next_back()) {
        (Some(newest), Some(oldest)) => newest.total - oldest.total,
        _ => 0.0,
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
            .map(|&(created_at, total)| SnapshotTotal { created_at, total })
            .collect()
    }

    #[test]
    fn f8_change_is_newest_minus_second_newest() {
        assert_eq!(change(&[]), 0.0);
        assert_eq!(change(&totals(&[(NOW, 100.0)])), 0.0, "fewer than 2");
        let three = totals(&[(NOW, 90.0), (NOW - MINUTE, 120.0), (NOW - 2 * MINUTE, 10.0)]);
        assert_eq!(change(&three), -30.0);
    }

    #[test]
    fn f8_sparkline_holds_the_newest_10_oldest_first() {
        assert!(sparkline(&[]).is_empty());
        let newest_first: Vec<SnapshotTotal> = (0..15)
            .map(|i| SnapshotTotal {
                created_at: NOW - i * MINUTE,
                total: (100 - i) as f64,
            })
            .collect();
        assert_eq!(
            sparkline(&newest_first),
            [91.0, 92.0, 93.0, 94.0, 95.0, 96.0, 97.0, 98.0, 99.0, 100.0]
        );
    }

    #[test]
    fn f8_income_spans_the_last_hour() {
        let never_reset = 0;
        let snapshots = totals(&[
            (NOW, 500.0),
            (NOW - 30 * MINUTE, 300.0),
            (NOW - 59 * MINUTE, 200.0),
            // Outside the hour: not the oldest of the window.
            (NOW - 60 * MINUTE, 50.0),
            (NOW - 90 * MINUTE, 10.0),
        ]);
        assert_eq!(income(&snapshots, NOW, never_reset), 300.0);
        // Not scaled to the window: two Snapshots 10 minutes apart.
        let short = totals(&[(NOW, 150.0), (NOW - 10 * MINUTE, 100.0)]);
        assert_eq!(income(&short, NOW, never_reset), 50.0);
        // Losses are negative.
        let loss = totals(&[(NOW, 100.0), (NOW - MINUTE, 160.0)]);
        assert_eq!(income(&loss, NOW, never_reset), -60.0);
    }

    #[test]
    fn f8_income_needs_2_snapshots_in_the_window() {
        assert_eq!(income(&[], NOW, 0), 0.0);
        let one_in_window = totals(&[(NOW, 500.0), (NOW - 2 * 60 * MINUTE, 100.0)]);
        assert_eq!(income(&one_in_window, NOW, 0), 0.0);
    }

    #[test]
    fn f8_income_reset_starts_the_window_at_the_reset() {
        let snapshots = totals(&[
            (NOW, 500.0),
            (NOW - 10 * MINUTE, 400.0),
            (NOW - 20 * MINUTE, 100.0),
        ]);
        // Reset 15 minutes ago: only the newest two count.
        assert_eq!(income(&snapshots, NOW, NOW - 15 * MINUTE), 100.0);
        // Right after a reset nothing counts, until 2 newer Snapshots.
        assert_eq!(income(&snapshots, NOW, NOW), 0.0);
        let after = totals(&[(NOW + 2 * MINUTE, 530.0), (NOW + MINUTE, 510.0)]);
        let mut all = after.clone();
        all.extend(&snapshots);
        assert_eq!(income(&all, NOW + 2 * MINUTE, NOW), 20.0);
        // A reset older than an hour leaves the hour.
        assert_eq!(
            income_window_start(NOW, NOW - 2 * 60 * MINUTE),
            NOW - 60 * MINUTE
        );
        assert_eq!(income_window_start(NOW, NOW - MINUTE), NOW - MINUTE);
    }
}
