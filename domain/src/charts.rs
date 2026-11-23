//! Chart series (F10): the net worth history of a player within a time span,
//! one such series per selected player in a Group (D15), and the per-tab
//! chart over the newest 50 Snapshots. Ports
//! `exilence-ce-legacy/src/store/domains/profile.ts:160-266`,
//! `src/store/domains/group.ts:185-204` and `src/utils/snapshot.utils.ts:102-116`.

/// How far back the net worth history reaches (legacy `chartTimeSpan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSpan {
    Day,
    Week,
    Month,
    All,
}

const HOUR: i64 = 60 * 60 * 1000;
const DAY: i64 = 24 * HOUR;

impl TimeSpan {
    /// The span's length in ms: 24 hours, 7 days, 30 days; `None` for all
    /// time.
    pub const fn millis(self) -> Option<i64> {
        match self {
            Self::Day => Some(DAY),
            Self::Week => Some(7 * DAY),
            Self::Month => Some(30 * DAY),
            Self::All => None,
        }
    }
}

/// A value in chaos at a time (Unix ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub at: i64,
    pub value: f64,
}

/// How many of the newest Snapshots the per-tab chart covers.
pub const TAB_CHART_SNAPSHOTS: usize = 50;

/// The net worth history of one player: the totals of `snapshots` (any
/// order) taken after `now` minus `span`, oldest first. A Snapshot exactly
/// at the span's start is outside it, as in legacy (`isBefore`).
pub fn net_worth_history(snapshots: &[Point], span: TimeSpan, now: i64) -> Vec<Point> {
    let start = span.millis().map(|length| now - length);
    let mut points: Vec<Point> = snapshots
        .iter()
        .filter(|p| start.is_none_or(|start| p.at > start))
        .copied()
        .collect();
    points.sort_by_key(|p| p.at);
    points
}

/// A Group player's Snapshot totals of their active Profile (F13).
#[derive(Debug, Clone, Copy)]
pub struct PlayerPoints<'a> {
    /// The account name, which names the player's series.
    pub name: &'a str,
    /// In the player selection.
    pub selected: bool,
    /// Any order.
    pub snapshots: &'a [Point],
}

/// One player's line of the Group net worth chart.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSeries {
    pub name: String,
    /// Oldest first.
    pub points: Vec<Point>,
}

/// The Group net worth chart (D15): one series per selected player of
/// `players`, in their order, each the player's [`net_worth_history`] within
/// `span`. A player without a Snapshot in the span has no line, as legacy
/// leaves out players without Snapshots.
pub fn group_net_worth(
    players: &[PlayerPoints<'_>],
    span: TimeSpan,
    now: i64,
) -> Vec<PlayerSeries> {
    players
        .iter()
        .filter(|p| p.selected)
        .map(|p| PlayerSeries {
            name: p.name.to_owned(),
            points: net_worth_history(p.snapshots, span, now),
        })
        .filter(|s| !s.points.is_empty())
        .collect()
}

/// A tab's value in one Snapshot, with the tab's name as of that Snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct TabValue {
    /// GGG stash tab id; empty for the Character pseudo-tab.
    pub tab_id: String,
    pub name: String,
    pub value: f64,
}

/// One Snapshot's tab values.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotTabs {
    pub at: i64,
    pub tabs: Vec<TabValue>,
}

/// One line of the per-tab chart.
#[derive(Debug, Clone, PartialEq)]
pub struct TabSeries {
    pub tab_id: String,
    /// The tab's name in the newest Snapshot that holds it.
    pub name: String,
    /// One point per Snapshot that holds the tab, oldest first.
    pub points: Vec<Point>,
}

/// The per-tab chart: one series per tab of `selected` (the Profile's stash
/// tab ids, in selection order) over the newest [`TAB_CHART_SNAPSHOTS`] of
/// `snapshots` (any order). The Character pseudo-tab is not charted; a
/// selected tab none of those Snapshots holds has no line.
pub fn tab_series(snapshots: &[SnapshotTabs], selected: &[String]) -> Vec<TabSeries> {
    let mut newest: Vec<&SnapshotTabs> = snapshots.iter().collect();
    newest.sort_by_key(|s| std::cmp::Reverse(s.at));
    newest.truncate(TAB_CHART_SNAPSHOTS);
    selected
        .iter()
        .filter(|id| !id.is_empty())
        .filter_map(|id| {
            // Newest first, so the first match names the series.
            let mut held = newest
                .iter()
                .filter_map(|s| s.tabs.iter().find(|t| t.tab_id == *id).map(|t| (s.at, t)));
            let (at, newest_tab) = held.next()?;
            let mut points: Vec<Point> = std::iter::once((at, newest_tab))
                .chain(held)
                .map(|(at, t)| Point { at, value: t.value })
                .collect();
            points.reverse();
            Some(TabSeries {
                tab_id: id.clone(),
                name: newest_tab.name.clone(),
                points,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn point(ago: i64, value: f64) -> Point {
        Point {
            at: NOW - ago,
            value,
        }
    }

    #[test]
    fn f10_net_worth_history_keeps_the_snapshots_within_the_time_span_oldest_first() {
        let snapshots = [
            point(HOUR, 1.0),
            point(40 * DAY, 2.0),
            point(DAY, 3.0),
            point(DAY - 1, 4.0),
            point(7 * DAY - 1, 5.0),
            point(30 * DAY - 1, 6.0),
            point(30 * DAY, 7.0),
        ];
        let values = |span| -> Vec<f64> {
            net_worth_history(&snapshots, span, NOW)
                .iter()
                .map(|p| p.value)
                .collect()
        };
        assert_eq!(values(TimeSpan::Day), [4.0, 1.0], "exactly 24 h ago is out");
        assert_eq!(values(TimeSpan::Week), [5.0, 3.0, 4.0, 1.0]);
        assert_eq!(values(TimeSpan::Month), [6.0, 5.0, 3.0, 4.0, 1.0]);
        assert_eq!(values(TimeSpan::All), [2.0, 7.0, 6.0, 5.0, 3.0, 4.0, 1.0]);
        assert!(net_worth_history(&[], TimeSpan::All, NOW).is_empty());
    }

    #[test]
    fn d15_group_net_worth_has_a_series_per_selected_player_within_the_time_span() {
        let foo = [
            point(2 * DAY, 10.0),
            point(HOUR, 30.0),
            point(3 * HOUR, 20.0),
        ];
        let bar = [point(HOUR, 7.0)];
        let baz = [point(HOUR, 5.0)];
        let old = [point(40 * DAY, 1.0)];
        let players = [
            PlayerPoints {
                name: "Foo#1234",
                selected: true,
                snapshots: &foo,
            },
            PlayerPoints {
                name: "Bar#5678",
                selected: false,
                snapshots: &bar,
            },
            PlayerPoints {
                name: "Baz#9012",
                selected: true,
                snapshots: &baz,
            },
            PlayerPoints {
                name: "Old#0000",
                selected: true,
                snapshots: &old,
            },
        ];
        let lines = |span| -> Vec<(String, Vec<f64>)> {
            group_net_worth(&players, span, NOW)
                .into_iter()
                .map(|s| (s.name, s.points.iter().map(|p| p.value).collect()))
                .collect()
        };
        let line = |name: &str, values: &[f64]| (name.to_owned(), values.to_vec());

        assert_eq!(
            lines(TimeSpan::Day),
            [line("Foo#1234", &[20.0, 30.0]), line("Baz#9012", &[5.0])],
            "selected players only, each within the span; none without points in it"
        );
        assert_eq!(
            lines(TimeSpan::All),
            [
                line("Foo#1234", &[10.0, 20.0, 30.0]),
                line("Baz#9012", &[5.0]),
                line("Old#0000", &[1.0]),
            ]
        );
        assert!(group_net_worth(&[], TimeSpan::All, NOW).is_empty());
    }

    fn tab(id: &str, name: &str, value: f64) -> TabValue {
        TabValue {
            tab_id: id.to_owned(),
            name: name.to_owned(),
            value,
        }
    }

    #[test]
    fn f10_tab_series_cover_the_newest_50_snapshots_per_selected_tab() {
        // 60 Snapshots, an hour apart, given newest last; `t1` is renamed in
        // the newest, `t3` only in the oldest 10, the Character pseudo-tab
        // in each.
        let snapshots: Vec<SnapshotTabs> = (0..60)
            .map(|i| {
                let mut tabs = vec![
                    tab("t1", if i == 59 { "Loot" } else { "Currency" }, i as f64),
                    tab("t2", "Maps", 100.0 + i as f64),
                    tab("", "Character", 5.0),
                ];
                if i < 10 {
                    tabs.push(tab("t3", "Old", 1.0));
                }
                SnapshotTabs {
                    at: NOW + i * HOUR,
                    tabs,
                }
            })
            .collect();
        let selected = ["t2", "t1", "t3", ""].map(str::to_owned);

        let series = tab_series(&snapshots, &selected);

        let names: Vec<&str> = series.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Maps", "Loot"], "selection order, newest name");
        for s in &series {
            assert_eq!(s.points.len(), TAB_CHART_SNAPSHOTS);
            assert_eq!(s.points[0].at, NOW + 10 * HOUR, "oldest of the newest 50");
            assert_eq!(s.points[49].at, NOW + 59 * HOUR);
        }
        assert_eq!(series[1].points[0].value, 10.0);
        assert_eq!(series[0].points[49].value, 159.0);
    }

    #[test]
    fn f10_a_tab_missing_from_some_snapshots_has_points_only_where_held() {
        let snapshots = [
            SnapshotTabs {
                at: NOW,
                tabs: vec![tab("t1", "A", 1.0)],
            },
            SnapshotTabs {
                at: NOW + HOUR,
                tabs: vec![tab("t2", "B", 2.0)],
            },
            SnapshotTabs {
                at: NOW + 2 * HOUR,
                tabs: vec![tab("t1", "A", 3.0), tab("t2", "B", 4.0)],
            },
        ];
        let series = tab_series(&snapshots, &["t1".to_owned(), "t9".to_owned()]);
        assert_eq!(
            series,
            [TabSeries {
                tab_id: "t1".to_owned(),
                name: "A".to_owned(),
                points: vec![
                    Point {
                        at: NOW,
                        value: 1.0
                    },
                    Point {
                        at: NOW + 2 * HOUR,
                        value: 3.0
                    },
                ],
            }],
            "no line for a tab no Snapshot holds"
        );
    }
}
