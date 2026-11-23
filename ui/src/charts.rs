//! The chart sections of the Net worth view (F10): the net worth history
//! with its time span and the per-tab chart with its legend, drawn in-house
//! as SVG with a nearest-point tooltip and drag zoom; and the sparkline the
//! widgets share (D24). The series come whole in the `charts` slice; zoom,
//! tooltip and legend are UI state only.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use ipc::MessageKey;
use ipc::command::{SettingsPatch, UpdateSettings, UpdateSettingsArgs};
use ipc::slices::{Series, TimeSpan};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

use crate::LOCALE;
use crate::currency::number_format;
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri;

/// Legacy `highchartsColors`: the primary colour, then Material 300 shades.
const PALETTE: [&str; 9] = [
    "#a31545", "#ff8a65", "#ffb74d", "#dce775", "#4db6ac", "#ffd54f", "#9575cd", "#f06292",
    "#64b5f6",
];
/// The net worth area's gradient: primary at 25 % down to primary-darker at
/// 0 % (legacy `SnapshotHistoryChart`).
const AREA_TOP: &str = "#a31545";
const AREA_BOTTOM: &str = "#4a091f";
/// Legacy `primaryLighter`, the sparkline colour.
const SPARKLINE_COLOR: &str = "#ff2e75";
const SPARKLINE_WIDTH: f64 = 90.0;
const SPARKLINE_HEIGHT: f64 = 25.0;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// What resets the zoom: the time span and the Group player selection, not
/// a new Snapshot (F10).
type ZoomKey = (TimeSpan, Vec<String>);

/// The net worth chart and the per-tab chart side by side, each in a
/// collapsible section whose open state is remembered.
#[component]
pub fn ChartSections(state: State) -> impl IntoView {
    let zoom_key = Memo::new(move |_| {
        let players = state.group.with(|g| {
            g.members
                .iter()
                .filter(|m| m.selected)
                .map(|m| m.account_name.clone())
                .collect()
        });
        (state.charts.with(|c| c.time_span), players)
    });
    let net_worth_open = Signal::derive(move || state.settings.with(|s| s.net_worth_chart_open));
    let tab_open = Signal::derive(move || state.settings.with(|s| s.tab_chart_open));
    let toggle_net_worth = move || {
        save(SettingsPatch {
            net_worth_chart_open: Some(!net_worth_open.get_untracked()),
            ..SettingsPatch::default()
        })
    };
    let toggle_tabs = move || {
        save(SettingsPatch {
            tab_chart_open: Some(!tab_open.get_untracked()),
            ..SettingsPatch::default()
        })
    };
    view! {
        <div class="chart-sections">
            <ChartSection
                class="net-worth-chart"
                title=MessageKey::ChartNetWorth
                icon=icon::SHOW_CHART
                open=net_worth_open
                toggle=toggle_net_worth
            >
                <Chart
                    series=Signal::derive(move || state.charts.with(|c| c.net_worth.clone()))
                    area=true
                    width=700.0
                    height=200.0
                    zoom_key
                />
                <TimeSpans state />
            </ChartSection>
            <ChartSection
                class="tab-chart"
                title=MessageKey::ChartTabs
                icon=icon::STACKED_LINE_CHART
                open=tab_open
                toggle=toggle_tabs
            >
                <Chart
                    series=Signal::derive(move || state.charts.with(|c| c.tabs.clone()))
                    area=false
                    width=500.0
                    height=222.0
                    zoom_key
                    legend=true
                />
            </ChartSection>
        </div>
    }
}

fn save(patch: SettingsPatch) {
    tauri::send::<UpdateSettings>(UpdateSettingsArgs { patch });
}

/// A collapsible section: its header toggles it, `aside` shown at the
/// header's right before the chevron (legacy `AccordionSummary`). Also
/// holds the item table.
#[component]
pub fn ChartSection(
    class: &'static str,
    title: MessageKey,
    icon: &'static str,
    #[prop(optional, into)] aside: Option<ViewFn>,
    open: Signal<bool>,
    toggle: impl Fn() + Copy + 'static,
    children: ChildrenFn,
) -> impl IntoView {
    view! {
        <section class=format!("chart-section {class}") class:open=open>
            // A div, so the aside may hold a link; keys work as on a button.
            <div
                class="chart-section-header"
                role="button"
                tabindex="0"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| toggle()
                on:keydown=move |ev| {
                    if matches!(ev.key().as_str(), "Enter" | " ") {
                        ev.prevent_default();
                        toggle();
                    }
                }
            >
                <Icon path=icon />
                <span class="chart-section-title">{LOCALE.key(title)}</span>
                {aside.map(|aside| view! { <span class="chart-section-aside">{aside.run()}</span> })}
                <span class="chart-section-chevron">
                    <Icon path=icon::EXPAND_MORE />
                </span>
            </div>
            <Show when=move || open.get()>
                <div class="chart-section-body">{children()}</div>
            </Show>
        </section>
    }
}

/// The time span options under the net worth chart (legacy `ChartToolbox`).
#[component]
fn TimeSpans(state: State) -> impl IntoView {
    const SPANS: [(TimeSpan, MessageKey); 4] = [
        (TimeSpan::Day, MessageKey::ChartTimeSpanDay),
        (TimeSpan::Week, MessageKey::ChartTimeSpanWeek),
        (TimeSpan::Month, MessageKey::ChartTimeSpanMonth),
        (TimeSpan::All, MessageKey::ChartTimeSpanAll),
    ];
    let options = SPANS.map(|(span, key)| {
        view! {
            <button
                class="time-span"
                class:selected=move || state.settings.with(|s| s.chart_time_span == span)
                on:click=move |_| {
                    save(SettingsPatch {
                        chart_time_span: Some(span),
                        ..SettingsPatch::default()
                    })
                }
            >
                {LOCALE.key(key)}
            </button>
        }
    });
    view! { <div class="time-spans">{options}</div> }
}

/// One chart: an area per series (net worth) or a line per series with a
/// legend (tabs), straight between points and without load animation.
#[component]
fn Chart(
    series: Signal<Vec<Series>>,
    area: bool,
    width: f64,
    height: f64,
    #[prop(into)] zoom_key: Signal<ZoomKey>,
    #[prop(optional)] legend: bool,
) -> impl IntoView {
    let frame = Frame {
        left: 56.0,
        top: 10.0,
        width: width - 56.0 - 12.0,
        height: height - 10.0 - 24.0,
    };
    let gradient = unique_id("chart-gradient");
    let clip = unique_id("chart-clip");
    // Legend toggles, by series name; not remembered.
    let hidden = RwSignal::new(HashSet::<String>::new());
    let zoom = RwSignal::new(None::<Range>);
    Effect::watch(
        move || zoom_key.track(),
        move |_, _, _| zoom.set(None),
        false,
    );
    // Drag start and current x; the pointer for the tooltip.
    let drag = RwSignal::new(None::<(f64, f64)>);
    let pointer = RwSignal::new(None::<(f64, f64)>);

    let shown = Memo::new(move |_| {
        series.with(|s| hidden.with(|h| s.iter().map(|s| !h.contains(&s.name)).collect::<Vec<_>>()))
    });
    let full = Memo::new(move |_| series.with(|s| shown.with(|v| full_range(s, v))));
    let scale = Memo::new(move |_| {
        let full = full.get()?;
        let range = zoom.get().unwrap_or(full);
        let (lo, hi) = series.with(|s| shown.with(|v| value_extent(s, v, range)))?;
        Some(Scale {
            range,
            axis: value_axis(lo, hi),
            frame,
        })
    });

    let on_down = move |ev: leptos::ev::MouseEvent| {
        if ev.button() != 0 {
            return;
        }
        if let Some((x, y)) = svg_point(&ev, width, height)
            && frame.contains(x, y)
        {
            ev.prevent_default();
            drag.set(Some((x, x)));
        }
    };
    let on_move = move |ev: leptos::ev::MouseEvent| {
        let at = svg_point(&ev, width, height);
        if let (Some((x, _)), Some((start, _))) = (at, drag.get_untracked()) {
            drag.set(Some((start, x)));
        }
        pointer.set(at);
    };
    let on_up = move |_| {
        let Some((from, to)) = drag.get_untracked() else {
            return;
        };
        drag.set(None);
        let (Some(scale), Some(full)) = (scale.get_untracked(), full.get_untracked()) else {
            return;
        };
        let min = series.with_untracked(|s| shown.with_untracked(|v| min_zoom(s, v)));
        if let Some(range) = zoom_range(&scale, from, to, min, full) {
            zoom.set(Some(range));
        }
    };
    let on_leave = move |_| {
        drag.set(None);
        pointer.set(None);
    };

    let grid = move || {
        let scale = scale.get()?;
        let offset = local_offset();
        let values = value_ticks(scale.axis)
            .into_iter()
            .map(|v| {
                let y = scale.y(v);
                view! {
                    <line class="chart-grid" x1=frame.left x2=frame.right() y1=y y2=y />
                    <text class="chart-label" x=frame.left - 6.0 y=y + 4.0 text-anchor="end">
                        {value_label(v, scale.axis)}
                    </text>
                }
            })
            .collect_view();
        let (interval, ticks) = time_ticks(scale.range, (frame.width / 90.0) as usize, offset);
        let times = ticks
            .into_iter()
            .map(|at| {
                let x = scale.x(at);
                view! {
                    <line class="chart-tick" x1=x x2=x y1=frame.bottom() y2=frame.bottom() + 5.0 />
                    <text class="chart-label" x=x y=frame.bottom() + 17.0 text-anchor="middle">
                        {time_label(at, interval)}
                    </text>
                }
            })
            .collect_view();
        Some(view! {
            {values}
            <line class="chart-axis" x1=frame.left x2=frame.right() y1=frame.bottom() y2=frame.bottom() />
            {times}
        })
    };

    let gradient_fill = format!("url(#{gradient})");
    let lines = move || {
        let scale = scale.get()?;
        let shown = shown.get();
        Some(series.with(|all| {
            all.iter()
                .enumerate()
                .filter(|(i, _)| shown[*i])
                .map(|(i, s)| {
                    let color = PALETTE[i % PALETTE.len()];
                    let points: Vec<(f64, f64)> = s
                        .points
                        .iter()
                        .map(|p| (scale.x(p.at), scale.y(p.value)))
                        .collect();
                    let in_view = s
                        .points
                        .iter()
                        .filter(|p| scale.range.contains(p.at))
                        .count();
                    // Markers only where points are far enough apart to see,
                    // so a lone Snapshot still shows.
                    let markers = (in_view as f64 * 4.0 <= frame.width).then(|| {
                        points
                            .iter()
                            .map(|&(x, y)| view! { <circle cx=x cy=y r="2" fill=color /> })
                            .collect_view()
                    });
                    let fill = area.then(|| {
                        view! {
                            <path d=area_path(&points, frame.bottom()) fill=gradient_fill.clone() />
                        }
                    });
                    view! {
                        {fill}
                        <path
                            d=line_path(&points)
                            fill="none"
                            stroke=color
                            stroke-width=if area { "2" } else { "1" }
                        />
                        {markers}
                    }
                })
                .collect_view()
        }))
    };

    let tooltip = move || {
        if drag.get().is_some() {
            return None;
        }
        let (x, y) = pointer.get()?;
        let scale = scale.get()?;
        let shown = shown.get();
        let (si, pi) = series.with(|s| nearest_point(s, &shown, &scale, x, y))?;
        let (name, point) = series.with(|s| (s[si].name.clone(), s[si].points[pi]));
        let color = PALETTE[si % PALETTE.len()];
        let (px, py) = (scale.x(point.at), scale.y(point.value));
        let when = local_date_time(point.at);
        // Chaos with exactly 2 decimals in the system locale, whatever the
        // currency switch shows (D24).
        let what = format!("{name}: {}", number_format().format(point.value, 2));
        let box_width = when.chars().count().max(what.chars().count()) as f64 * 6.2 + 16.0;
        let box_height = 38.0;
        let left = if px + 12.0 + box_width > frame.right() {
            px - 12.0 - box_width
        } else {
            px + 12.0
        };
        let top = (py - box_height - 8.0).max(frame.top);
        Some(view! {
            <circle class="chart-hover" cx=px cy=py r="4" fill=color />
            <g class="chart-tooltip" transform=format!("translate({left},{top})")>
                <rect width=box_width height=box_height rx="3" stroke=color />
                <text x="8" y="15">{when}</text>
                <text x="8" y="30">{what}</text>
            </g>
        })
    };

    let selection = move || {
        let (from, to) = drag.get()?;
        let a = from.min(to).clamp(frame.left, frame.right());
        let b = from.max(to).clamp(frame.left, frame.right());
        Some(view! {
            <rect class="chart-selection" x=a y=frame.top width=b - a height=frame.height />
        })
    };

    let legend = legend.then(|| {
        view! {
            <ul class="chart-legend">
                <For
                    each=move || series.with(|s| s.iter().map(|s| s.name.clone()).enumerate().collect::<Vec<_>>())
                    key=|entry| entry.clone()
                    let:entry
                >
                    {
                        let (i, name) = entry;
                        let toggled = name.clone();
                        let is_hidden = {
                            let name = name.clone();
                            move || hidden.with(|h| h.contains(&name))
                        };
                        view! {
                            <li>
                                <button
                                    class="chart-legend-entry"
                                    class:hidden=is_hidden
                                    on:click=move |_| {
                                        hidden.update(|h| {
                                            if !h.remove(&toggled) {
                                                h.insert(toggled.clone());
                                            }
                                        })
                                    }
                                >
                                    <svg class="chart-legend-swatch" viewBox="0 0 10 10">
                                        <circle cx="5" cy="5" r="4" fill=PALETTE[i % PALETTE.len()] />
                                    </svg>
                                    {name}
                                </button>
                            </li>
                        }
                    }
                </For>
            </ul>
        }
    });

    view! {
        <div class="chart">
            <Show when=move || zoom.with(Option::is_some)>
                <button class="reset-zoom" on:click=move |_| zoom.set(None)>
                    {LOCALE.key(MessageKey::ChartResetZoom)}
                </button>
            </Show>
            <svg
                class="chart-svg"
                viewBox=format!("0 0 {width} {height}")
                on:mousedown=on_down
                on:mousemove=on_move
                on:mouseup=on_up
                on:mouseleave=on_leave
            >
                <defs>
                    <linearGradient id=gradient x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0" stop-color=AREA_TOP stop-opacity="0.25" />
                        <stop offset="1" stop-color=AREA_BOTTOM stop-opacity="0" />
                    </linearGradient>
                    <clipPath id=clip.clone()>
                        <rect x=frame.left y=frame.top - 4.0 width=frame.width height=frame.height + 4.0 />
                    </clipPath>
                </defs>
                {grid}
                <g clip-path=format!("url(#{clip})")>{lines}</g>
                {tooltip}
                {selection}
            </svg>
            {legend}
        </div>
    }
}

/// A sparkline (F8 total value, F11 "Price last 7 days", F6): legacy's
/// smoothed line with its gradient area to the zero line, no animation
/// (D24); `None` days break the line.
#[component]
pub fn Sparkline(#[prop(into)] values: Signal<Vec<Option<f64>>>) -> impl IntoView {
    let gradient = unique_id("sparkline-gradient");
    let fill = format!("url(#{gradient})");
    let paths =
        Memo::new(move |_| values.with(|v| sparkline_paths(v, SPARKLINE_WIDTH, SPARKLINE_HEIGHT)));
    view! {
        <svg
            class="sparkline"
            viewBox=format!("0 0 {SPARKLINE_WIDTH} {SPARKLINE_HEIGHT}")
            width=SPARKLINE_WIDTH
            height=SPARKLINE_HEIGHT
            aria-hidden="true"
        >
            <defs>
                <linearGradient id=gradient x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0" stop-color=SPARKLINE_COLOR stop-opacity="0.9" />
                    <stop offset="1" stop-color=SPARKLINE_COLOR stop-opacity="0" />
                </linearGradient>
            </defs>
            {move || {
                paths
                    .get()
                    .map(|(line, area)| {
                        view! {
                            <path d=area fill=fill.clone() fill-opacity="0.4" />
                            <path d=line fill="none" stroke=SPARKLINE_COLOR stroke-width="2" />
                        }
                    })
            }}
        </svg>
    }
}

/// An id unique in the document, for gradients and clip paths.
fn unique_id(prefix: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!("{prefix}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The pointer of `ev` in the viewBox (`width` × `height`) of the SVG that
/// handles it.
fn svg_point(ev: &leptos::ev::MouseEvent, width: f64, height: f64) -> Option<(f64, f64)> {
    let svg = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
    let rect = svg.get_bounding_client_rect();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    Some((
        (f64::from(ev.client_x()) - rect.left()) * width / rect.width(),
        (f64::from(ev.client_y()) - rect.top()) * height / rect.height(),
    ))
}

/// Local time minus UTC now, in ms.
fn local_offset() -> i64 {
    -(js_sys::Date::new_0().get_timezone_offset() * MINUTE as f64) as i64
}

/// Formats with `Intl` in the system locale.
fn intl_format(format: &js_sys::Function, value: &JsValue) -> String {
    format
        .call1(&JsValue::UNDEFINED, value)
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_default()
}

fn intl_options(entries: &[(&str, JsValue)]) -> js_sys::Object {
    let options = js_sys::Object::new();
    for (key, value) in entries {
        let _ = js_sys::Reflect::set(&options, &JsValue::from_str(key), value);
    }
    options
}

fn date_time_format(entries: &[(&str, JsValue)]) -> js_sys::Function {
    js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &intl_options(entries)).format()
}

/// The tooltip's local date and time.
fn local_date_time(at: i64) -> String {
    thread_local! {
        static FORMAT: js_sys::Function =
            date_time_format(&[("dateStyle", "medium".into()), ("timeStyle", "short".into())]);
    }
    FORMAT.with(|f| intl_format(f, &JsValue::from_f64(at as f64)))
}

/// A time-axis label in local time: the time of day below a day's ticks,
/// else the day, else the month.
fn time_label(at: i64, interval: i64) -> String {
    thread_local! {
        static TIME: js_sys::Function =
            date_time_format(&[("hour", "2-digit".into()), ("minute", "2-digit".into())]);
        static DAY: js_sys::Function =
            date_time_format(&[("day", "numeric".into()), ("month", "short".into())]);
        static MONTH: js_sys::Function =
            date_time_format(&[("month", "short".into()), ("year", "2-digit".into())]);
    }
    let format = if interval < self::DAY {
        &TIME
    } else if interval < 90 * self::DAY {
        &DAY
    } else {
        &MONTH
    };
    format.with(|f| intl_format(f, &JsValue::from_f64(at as f64)))
}

// ---------------------------------------------------------------- geometry

/// A range on the time axis, Unix ms, `start < end`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Range {
    start: i64,
    end: i64,
}

impl Range {
    fn contains(self, at: i64) -> bool {
        (self.start..=self.end).contains(&at)
    }

    fn len(self) -> i64 {
        self.end - self.start
    }
}

/// The plot area inside a chart's viewBox.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

impl Frame {
    fn right(self) -> f64 {
        self.left + self.width
    }

    fn bottom(self) -> f64 {
        self.top + self.height
    }

    fn contains(self, x: f64, y: f64) -> bool {
        (self.left..=self.right()).contains(&x) && (self.top..=self.bottom()).contains(&y)
    }
}

/// The value axis: `lo` to `hi` in steps of `step`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ValueAxis {
    lo: f64,
    hi: f64,
    step: f64,
}

/// Maps times and values into the plot.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scale {
    range: Range,
    axis: ValueAxis,
    frame: Frame,
}

impl Scale {
    fn x(&self, at: i64) -> f64 {
        self.frame.left
            + (at - self.range.start) as f64 / self.range.len() as f64 * self.frame.width
    }

    fn y(&self, value: f64) -> f64 {
        self.frame.bottom()
            - (value - self.axis.lo) / (self.axis.hi - self.axis.lo) * self.frame.height
    }

    /// The time at viewBox `x`.
    fn at(&self, x: f64) -> i64 {
        self.range.start
            + ((x - self.frame.left) / self.frame.width * self.range.len() as f64).round() as i64
    }
}

/// The shown series' points, with their series index.
fn shown_points<'a>(
    series: &'a [Series],
    shown: &'a [bool],
) -> impl Iterator<Item = (usize, usize, &'a ipc::slices::Point)> {
    series
        .iter()
        .enumerate()
        .filter(|(i, _)| shown.get(*i).copied().unwrap_or(false))
        .flat_map(|(i, s)| s.points.iter().enumerate().map(move |(j, p)| (i, j, p)))
}

/// The time range of the shown series; a lone time sits in the middle of
/// two hours. `None` without points.
fn full_range(series: &[Series], shown: &[bool]) -> Option<Range> {
    let (start, end) = shown_points(series, shown).fold(None, |range, (_, _, p)| match range {
        None => Some((p.at, p.at)),
        Some((start, end)) => Some((p.at.min(start), p.at.max(end))),
    })?;
    Some(if start == end {
        Range {
            start: start - HOUR,
            end: end + HOUR,
        }
    } else {
        Range { start, end }
    })
}

/// The lowest and highest shown value within `range`, else of all shown
/// points (a zoom between two Snapshots). `None` without points.
fn value_extent(series: &[Series], shown: &[bool], range: Range) -> Option<(f64, f64)> {
    let extent = |within: bool| {
        shown_points(series, shown)
            .filter(|(_, _, p)| !within || range.contains(p.at))
            .fold(None, |extent, (_, _, p)| match extent {
                None => Some((p.value, p.value)),
                Some((lo, hi)) => Some((p.value.min(lo), p.value.max(hi))),
            })
    };
    extent(true).or_else(|| extent(false))
}

/// A value axis around `lo..=hi` with about 5 round steps of 1, 2, 2.5 or 5
/// times a power of ten.
fn value_axis(lo: f64, hi: f64) -> ValueAxis {
    let (lo, hi) = if hi > lo {
        (lo, hi)
    } else {
        let pad = (lo.abs() * 0.1).max(1.0);
        (lo - pad, hi + pad)
    };
    let raw = (hi - lo) / 5.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|f| f * magnitude)
        .find(|&s| s >= raw)
        .unwrap_or(10.0 * magnitude);
    ValueAxis {
        lo: (lo / step).floor() * step,
        hi: (hi / step).ceil() * step,
        step,
    }
}

fn value_ticks(axis: ValueAxis) -> Vec<f64> {
    let count = ((axis.hi - axis.lo) / axis.step).round() as usize;
    (0..=count)
        .map(|i| axis.lo + i as f64 * axis.step)
        .collect()
}

/// A value-axis label: thousands as `k`, millions as `M` (Highcharts'
/// numeric symbols) once the axis reaches them, with the decimals the step
/// needs.
fn value_label(value: f64, axis: ValueAxis) -> String {
    let top = axis.lo.abs().max(axis.hi.abs());
    let (divisor, symbol) = if top >= 1e6 {
        (1e6, "M")
    } else if top >= 1e3 {
        (1e3, "k")
    } else {
        (1.0, "")
    };
    let unit = axis.step / divisor;
    let decimals = (0..3)
        .find(|&d| {
            let scaled = unit * 10f64.powi(d);
            (scaled - scaled.round()).abs() < 1e-9
        })
        .unwrap_or(3) as usize;
    let mut text = format!("{:.*}", decimals, value / divisor);
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if text == "-0" {
        text = "0".to_owned();
    }
    format!("{text}{symbol}")
}

/// Time-axis intervals, shortest first.
const TICK_INTERVALS: [i64; 15] = [
    MINUTE,
    5 * MINUTE,
    15 * MINUTE,
    30 * MINUTE,
    HOUR,
    3 * HOUR,
    6 * HOUR,
    12 * HOUR,
    DAY,
    2 * DAY,
    7 * DAY,
    14 * DAY,
    30 * DAY,
    91 * DAY,
    365 * DAY,
];

/// The time-axis ticks over `range`: the shortest interval that gives at
/// most `max` ticks, at its multiples in local time (`offset`: local minus
/// UTC, ms). Returns the interval and the ticks.
fn time_ticks(range: Range, max: usize, offset: i64) -> (i64, Vec<i64>) {
    let max = max.max(2) as i64;
    let interval = TICK_INTERVALS
        .into_iter()
        .find(|&i| range.len() / i < max)
        .unwrap_or(TICK_INTERVALS[TICK_INTERVALS.len() - 1]);
    let first = (range.start + offset).div_euclid(interval) * interval - offset;
    let first = if first < range.start {
        first + interval
    } else {
        first
    };
    let ticks = std::iter::successors(Some(first), |t| Some(t + interval))
        .take_while(|&t| t <= range.end)
        .collect();
    (interval, ticks)
}

/// The shortest zoom: 5 times the closest distance between two points of a
/// shown series (Highcharts' `minRange`).
fn min_zoom(series: &[Series], shown: &[bool]) -> i64 {
    series
        .iter()
        .zip(shown)
        .filter(|(_, shown)| **shown)
        .flat_map(|(s, _)| s.points.windows(2).map(|w| w[1].at - w[0].at))
        .filter(|&gap| gap > 0)
        .min()
        .map_or(0, |gap| 5 * gap)
}

/// A drag shorter than this (viewBox px) is a click, not a zoom.
const MIN_DRAG: f64 = 10.0;

/// The zoom a drag from `from` to `to` (viewBox x, either direction)
/// selects: the time range under it, clamped to the plot. A narrower range
/// than `min` widens to `min` around its centre, shifted to stay inside
/// `full`. `None` for a drag shorter than [`MIN_DRAG`].
fn zoom_range(scale: &Scale, from: f64, to: f64, min: i64, full: Range) -> Option<Range> {
    if (to - from).abs() < MIN_DRAG {
        return None;
    }
    let frame = scale.frame;
    let a = from.min(to).clamp(frame.left, frame.right());
    let b = from.max(to).clamp(frame.left, frame.right());
    let (mut start, mut end) = (scale.at(a), scale.at(b));
    let min = min.min(full.len());
    if end - start < min {
        start += (end - start) / 2 - min / 2;
        end = start + min;
        if start < full.start {
            (start, end) = (full.start, full.start + min);
        } else if end > full.end {
            (start, end) = (full.end - min, full.end);
        }
    }
    (end > start).then_some(Range { start, end })
}

/// The point the tooltip shows for the pointer at viewBox (`x`, `y`): among
/// the shown series' points inside the zoom, the horizontally nearest, then
/// the nearest overall (Highcharts' `findNearestPointBy: 'x'`). Returns the
/// series and point index.
fn nearest_point(
    series: &[Series],
    shown: &[bool],
    scale: &Scale,
    x: f64,
    y: f64,
) -> Option<(usize, usize)> {
    if !scale.frame.contains(x, y) {
        return None;
    }
    shown_points(series, shown)
        .filter(|(_, _, p)| scale.range.contains(p.at))
        .map(|(i, j, p)| {
            let dx = (scale.x(p.at) - x).abs();
            let dy = scale.y(p.value) - y;
            ((dx, dx * dx + dy * dy), (i, j))
        })
        .min_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, found)| found)
}

/// An SVG path straight through `points`.
fn line_path(points: &[(f64, f64)]) -> String {
    let mut d = String::with_capacity(points.len() * 16);
    for (i, (x, y)) in points.iter().enumerate() {
        d.push_str(if i == 0 { "M" } else { "L" });
        d.push_str(&format!("{x:.1},{y:.1}"));
    }
    d
}

/// The area under `points` down to `bottom`.
fn area_path(points: &[(f64, f64)], bottom: f64) -> String {
    match (points.first(), points.last()) {
        (Some(&(first, _)), Some(&(last, _))) => {
            format!(
                "{}L{last:.1},{bottom:.1}L{first:.1},{bottom:.1}Z",
                line_path(points)
            )
        }
        _ => String::new(),
    }
}

/// d3's `curveBasis` (Victory's `basis`, legacy's sparklines) through
/// `points`: a B-spline from the first point to the last; two points run
/// straight.
fn basis_path(points: &[(f64, f64)]) -> String {
    let at = |(x, y): (f64, f64)| format!("{x:.1},{y:.1}");
    let bezier = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        format!(
            "C{} {} {}",
            at(((2.0 * a.0 + b.0) / 3.0, (2.0 * a.1 + b.1) / 3.0)),
            at(((a.0 + 2.0 * b.0) / 3.0, (a.1 + 2.0 * b.1) / 3.0)),
            at(((a.0 + 4.0 * b.0 + c.0) / 6.0, (a.1 + 4.0 * b.1 + c.1) / 6.0)),
        )
    };
    match points {
        [] => String::new(),
        [only] => format!("M{}", at(*only)),
        [first, last] => format!("M{}L{}", at(*first), at(*last)),
        [first, second, ..] => {
            let mut d = format!(
                "M{}L{}",
                at(*first),
                at((
                    (5.0 * first.0 + second.0) / 6.0,
                    (5.0 * first.1 + second.1) / 6.0
                ))
            );
            for w in points.windows(3) {
                d.push_str(&bezier(w[0], w[1], w[2]));
            }
            let (before, last) = (points[points.len() - 2], points[points.len() - 1]);
            d.push_str(&bezier(before, last, last));
            d.push_str(&format!("L{}", at(last)));
            d
        }
    }
}

/// A sparkline's line and area paths in a `width` × `height` box (legacy
/// `SparklineChart`): one smoothed segment per run of days with data (a day
/// without breaks the line), x by day, y scaled to the values' lowest and
/// highest with 1 px to spare, all zeros along the bottom. Each segment's
/// area reaches to the zero line, so it fills under a rise and over a fall;
/// a zero line outside the values is the box's edge. `None` when no two
/// days with data follow each other.
fn sparkline_paths(values: &[Option<f64>], width: f64, height: f64) -> Option<(String, String)> {
    let known = || values.iter().flatten().copied();
    let lo = known().fold(f64::INFINITY, f64::min);
    let hi = known().fold(f64::NEG_INFINITY, f64::max);
    if values.len() < 2 || lo > hi {
        return None;
    }
    let (top, bottom) = (1.0, height - 1.0);
    let step = (width - 2.0) / (values.len() - 1) as f64;
    let y = |v: f64| {
        let share = if hi > lo {
            (v - lo) / (hi - lo)
        } else if v == 0.0 {
            // Victory's domain of only zeros: [0, ε].
            0.0
        } else {
            0.5
        };
        bottom - share * (bottom - top)
    };
    let zero = if hi < 0.0 {
        0.0
    } else if lo > 0.0 {
        height
    } else {
        y(0.0)
    };
    let (mut line, mut area) = (String::new(), String::new());
    let points: Vec<Option<(f64, f64)>> = values
        .iter()
        .enumerate()
        .map(|(i, v)| v.map(|v| (1.0 + i as f64 * step, y(v))))
        .collect();
    for segment in points.split(Option::is_none) {
        let segment: Vec<(f64, f64)> = segment.iter().flatten().copied().collect();
        if let [(first, _), .., (last, _)] = segment[..] {
            let path = basis_path(&segment);
            area.push_str(&format!("{path}L{last:.1},{zero:.1}L{first:.1},{zero:.1}Z"));
            line.push_str(&path);
        }
    }
    (!line.is_empty()).then_some((line, area))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::slices::Point;

    const T: i64 = 1_800_000_000_000;

    fn series(name: &str, points: &[(i64, f64)]) -> Series {
        Series {
            name: name.to_owned(),
            points: points
                .iter()
                .map(|&(at, value)| Point { at, value })
                .collect(),
        }
    }

    /// A 100 px wide plot from x 0 over `T..T + 100 h`, values 0..=100.
    fn scale() -> Scale {
        Scale {
            range: Range {
                start: T,
                end: T + 100 * HOUR,
            },
            axis: ValueAxis {
                lo: 0.0,
                hi: 100.0,
                step: 20.0,
            },
            frame: Frame {
                left: 0.0,
                top: 0.0,
                width: 100.0,
                height: 100.0,
            },
        }
    }

    #[test]
    fn f10_zoom_range_is_the_dragged_time_range_inside_the_plot() {
        let scale = scale();
        let full = scale.range;
        let range = |start, end| Some(Range { start, end });
        assert_eq!(
            zoom_range(&scale, 20.0, 60.0, 0, full),
            range(T + 20 * HOUR, T + 60 * HOUR)
        );
        assert_eq!(
            zoom_range(&scale, 60.0, 20.0, 0, full),
            range(T + 20 * HOUR, T + 60 * HOUR),
            "either direction"
        );
        assert_eq!(
            zoom_range(&scale, 80.0, 140.0, 0, full),
            range(T + 80 * HOUR, T + 100 * HOUR),
            "clamped to the plot"
        );
        assert_eq!(zoom_range(&scale, 20.0, 29.0, 0, full), None, "a click");
    }

    #[test]
    fn f10_zoom_range_is_at_least_five_point_distances_and_stays_inside() {
        let scale = scale();
        let full = scale.range;
        let points = series("a", &[(T, 1.0), (T + 4 * HOUR, 2.0), (T + 10 * HOUR, 3.0)]);
        let min = min_zoom(std::slice::from_ref(&points), &[true]);
        assert_eq!(min, 20 * HOUR, "5 × the 4 h gap");
        assert_eq!(
            zoom_range(&scale, 40.0, 50.0, min, full),
            Some(Range {
                start: T + 35 * HOUR,
                end: T + 55 * HOUR
            }),
            "widened around the centre"
        );
        assert_eq!(
            zoom_range(&scale, 0.0, 10.0, min, full),
            Some(Range {
                start: T,
                end: T + 20 * HOUR
            }),
            "shifted inside the full range"
        );
        assert_eq!(
            zoom_range(&scale, 90.0, 100.0, min, full),
            Some(Range {
                start: T + 80 * HOUR,
                end: T + 100 * HOUR
            })
        );
        assert_eq!(
            zoom_range(&scale, 0.0, 50.0, 1000 * HOUR, full),
            Some(full),
            "never wider than the full range"
        );
    }

    #[test]
    fn f10_nearest_point_is_the_horizontally_nearest_then_the_closest() {
        let scale = scale();
        let all = [
            series("low", &[(T + 10 * HOUR, 10.0), (T + 50 * HOUR, 20.0)]),
            series("high", &[(T + 10 * HOUR, 90.0), (T + 52 * HOUR, 80.0)]),
            series("hidden", &[(T + 30 * HOUR, 50.0)]),
        ];
        let shown = [true, true, false];
        let nearest = |x, y| nearest_point(&all, &shown, &scale, x, y);
        // y 100 is value 0, y 0 is value 100.
        assert_eq!(
            nearest(48.0, 10.0),
            Some((0, 1)),
            "nearest in x wins over y"
        );
        assert_eq!(
            nearest(10.0, 85.0),
            Some((0, 0)),
            "same x: the closer value"
        );
        assert_eq!(nearest(10.0, 15.0), Some((1, 0)));
        assert_eq!(nearest(25.0, 70.0), Some((0, 0)), "hidden series skipped");
        assert_eq!(nearest(150.0, 50.0), None, "outside the plot");

        let zoomed = Scale {
            range: Range {
                start: T + 40 * HOUR,
                end: T + 100 * HOUR,
            },
            ..scale
        };
        assert_eq!(
            nearest_point(&all, &shown, &zoomed, 0.0, 85.0),
            Some((0, 1)),
            "only points inside the zoom"
        );
    }

    #[test]
    fn f10_ranges_axes_and_ticks() {
        let a = series("a", &[(T, 5.0)]);
        assert_eq!(
            full_range(std::slice::from_ref(&a), &[true]),
            Some(Range {
                start: T - HOUR,
                end: T + HOUR
            }),
            "a lone Snapshot in the middle"
        );
        assert_eq!(full_range(std::slice::from_ref(&a), &[false]), None);
        assert_eq!(
            value_axis(1012.0, 1480.0),
            ValueAxis {
                lo: 1000.0,
                hi: 1500.0,
                step: 100.0
            }
        );
        let axis = value_axis(0.0, 2400.0);
        assert_eq!(axis.step, 500.0);
        assert_eq!(
            value_ticks(axis),
            [0.0, 500.0, 1000.0, 1500.0, 2000.0, 2500.0]
        );
        assert_eq!(value_label(2500.0, axis), "2.5k");
        assert_eq!(value_label(2000.0, axis), "2k");
        assert_eq!(value_label(0.75, value_axis(0.0, 1.2)), "0.75");
        let (interval, ticks) = time_ticks(
            Range {
                start: T + 5 * MINUTE,
                end: T + 5 * DAY,
            },
            6,
            2 * HOUR,
        );
        assert_eq!(interval, DAY);
        assert_eq!(ticks.len(), 5);
        assert_eq!((ticks[0] + 2 * HOUR) % DAY, 0, "local midnight");
    }

    #[test]
    fn d24_sparklines_are_smoothed_with_their_area_to_the_zero_line() {
        assert_eq!(sparkline_paths(&[Some(1.0)], 90.0, 25.0), None);
        assert_eq!(
            sparkline_paths(&[Some(1.0), None, Some(2.0)], 90.0, 25.0),
            None
        );
        let (line, area) =
            sparkline_paths(&[Some(0.0), Some(10.0), Some(0.0)], 90.0, 25.0).unwrap();
        assert_eq!(
            line,
            "M1.0,24.0L8.3,20.2C15.7,16.3 30.3,8.7 45.0,8.7C59.7,8.7 74.3,16.3 81.7,20.2L89.0,24.0"
        );
        assert_eq!(area, format!("{line}L89.0,24.0L1.0,24.0Z"));
        // A fall fills over the line, up to its start at 0.
        let (_, area) = sparkline_paths(&[Some(0.0), Some(-10.0)], 90.0, 25.0).unwrap();
        assert_eq!(area, "M1.0,1.0L89.0,24.0L89.0,1.0L1.0,1.0Z");
        // Values above 0 fill down to the box's bottom.
        let (_, area) = sparkline_paths(&[Some(5.0), Some(10.0)], 90.0, 25.0).unwrap();
        assert_eq!(area, "M1.0,24.0L89.0,1.0L89.0,25.0L1.0,25.0Z");
        // A day without data breaks the line; days keep their place.
        let (line, _) =
            sparkline_paths(&[Some(0.0), None, Some(5.0), Some(10.0)], 90.0, 25.0).unwrap();
        assert_eq!(line, "M59.7,12.5L89.0,1.0");
        // Only zeros run along the bottom.
        let (line, _) = sparkline_paths(&[Some(0.0), Some(0.0)], 90.0, 25.0).unwrap();
        assert_eq!(line, "M1.0,24.0L89.0,24.0");
    }
}
