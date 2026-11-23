//! The guided tour (F19): legacy's steps without bulk sell, shown as
//! legacy's reactour stepper (`ToolbarStepper.tsx`, reactour 1.18). It runs
//! once after the first sign-in and again from the support panel's Re-take
//! tour; What's New opens by itself after the first one. Each step dims the
//! window but for the element marked `data-tour="<target>"`, puts a numbered
//! card beside it and blocks clicks on the whole window.

use std::time::Duration;

use ipc::MessageKey;
use ipc::command::{CompleteTour, LogLevel, NoArgs};
use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;
use wasm_bindgen::JsCast;
use web_sys::{ScrollIntoViewOptions, ScrollLogicalPosition};

use crate::LOCALE;
use crate::state::State;
use crate::tauri;

/// One step: the element it points at (none: centered), its title and text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub target: Option<&'static str>,
    pub title: MessageKey,
    pub body: MessageKey,
    pub body2: Option<MessageKey>,
}

const fn step(
    target: Option<&'static str>,
    title: MessageKey,
    body: MessageKey,
    body2: Option<MessageKey>,
) -> Step {
    Step {
        target,
        title,
        body,
        body2,
    }
}

impl Step {
    /// Space around the target inside the highlight: none for the
    /// navigation's views and the support panel (legacy `maskSpace`).
    fn space(&self) -> f64 {
        match self.target {
            Some("net-worth" | "settings" | "support-panel") => 0.0,
            _ => 16.0,
        }
    }

    /// Extra space above the card: the support panel's (legacy `marginTop`).
    fn card_offset(&self) -> f64 {
        match self.target {
            Some("support-panel") => 20.0,
            _ => 0.0,
        }
    }
}

/// Legacy's steps in legacy order (`stepper.utils.ts`), without bulk sell.
pub const STEPS: [Step; 11] = [
    step(
        None,
        MessageKey::TourInitialTitle,
        MessageKey::TourInitialBody,
        None,
    ),
    step(
        Some("net-worth"),
        MessageKey::TourNetWorthTitle,
        MessageKey::TourNetWorthBody,
        None,
    ),
    step(
        Some("settings"),
        MessageKey::TourSettingsTitle,
        MessageKey::TourSettingsBody,
        None,
    ),
    step(
        Some("profile"),
        MessageKey::TourProfileTitle,
        MessageKey::TourProfileBody,
        Some(MessageKey::TourProfileBody2),
    ),
    step(
        Some("snapshot"),
        MessageKey::TourSnapshotTitle,
        MessageKey::TourSnapshotBody,
        Some(MessageKey::TourSnapshotBody2),
    ),
    step(
        Some("overlay"),
        MessageKey::TourOverlayTitle,
        MessageKey::TourOverlayBody,
        None,
    ),
    step(
        Some("group"),
        MessageKey::TourGroupTitle,
        MessageKey::TourGroupBody,
        None,
    ),
    step(
        Some("notifications"),
        MessageKey::TourNotificationsTitle,
        MessageKey::TourNotificationsBody,
        None,
    ),
    step(
        Some("currency-switch"),
        MessageKey::TourCurrencySwitchTitle,
        MessageKey::TourCurrencySwitchBody,
        None,
    ),
    step(
        Some("reset-income"),
        MessageKey::TourResetIncomeTitle,
        MessageKey::TourResetIncomeBody,
        None,
    ),
    step(
        Some("support-panel"),
        MessageKey::TourSupportPanelTitle,
        MessageKey::TourSupportPanelBody,
        None,
    ),
];

/// A box in viewport pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// A width and height in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// Where the card's top left corner goes (reactour's `Guide`): beside the
/// target, 2 × `space` away, on the side with the most room that fits the
/// card, aligned with the target's edge that keeps it inside the window.
/// Without a target, or without a side that fits, in the middle.
pub fn card_position(target: Option<Rect>, space: f64, card: Size, window: Size) -> (f64, f64) {
    let center = (
        (window.width - card.width) / 2.0,
        (window.height - card.height) / 2.0,
    );
    let Some(target) = target else {
        return center;
    };
    let right = target.left + target.width;
    let bottom = target.top + target.height;
    // Along a side: from the target's start edge, else up to its end edge.
    let align = |start: f64, end: f64, size: f64, room: f64| {
        let at = if start + size <= room {
            start - space
        } else if end + space <= room {
            end - size + space
        } else {
            end - size
        };
        at.max(space)
    };
    let x = align(target.left, right, card.width, window.width);
    let y = align(target.top, bottom, card.height, window.height);
    // Room on each side, the card's size across it, and the card there.
    let mut sides = [
        (
            target.left,
            card.width,
            (target.left - card.width - 2.0 * space, y),
        ),
        (window.width - right, card.width, (right + 2.0 * space, y)),
        (
            target.top,
            card.height,
            (x, target.top - card.height - 2.0 * space),
        ),
        (
            window.height - bottom,
            card.height,
            (x, bottom + 2.0 * space),
        ),
    ];
    // Most room first; equal room keeps this order.
    sides.sort_by(|a, b| b.0.total_cmp(&a.0));
    sides
        .into_iter()
        .find(|&(room, size, _)| room > size + 2.0 * space)
        .map_or(center, |(_, _, at)| at)
}

/// Starts the tour once the main window is signed in with its state loaded,
/// unless it ran before; while the legacy import dialog asks (F21), it
/// waits. Once per run: a slice changing before `complete_tour` is stored
/// does not start it again.
pub fn start_on_first_run(state: State, open: RwSignal<bool>) {
    let started = StoredValue::new(false);
    Effect::new(move |_| {
        let first_run = state.loaded.get()
            && state
                .session
                .with(|s| s.login == ipc::slices::LoginState::SignedIn)
            && !state.settings.with(|s| s.tour_completed)
            && state.legacy_import.with(|l| l.offer.is_none());
        if first_run && !started.get_value() {
            started.set_value(true);
            open.set(true);
        }
    });
}

/// The tour while `open`; finishing or Escape ends it, and What's New opens
/// when the core says so (once, after the first tour).
#[component]
pub fn Tour(open: RwSignal<bool>, whats_new: RwSignal<bool>) -> impl IntoView {
    view! {
        <Show when=move || open.get()>
            <TourSteps open whats_new />
        </Show>
    }
}

/// How long after the first tour ends What's New opens (legacy).
const WHATS_NEW_DELAY: Duration = Duration::from_secs(1);

/// The arrow buttons' icons (reactour `Arrow`).
const BACK_ARROW: &str = "M1.4 7.2h16M7.6 1L1.4 7.2l6.2 6.2";
const NEXT_ARROW: &str = "M17 7.2H1M10.8 1L17 7.2l-6.2 6.2";

#[component]
fn TourSteps(open: RwSignal<bool>, whats_new: RwSignal<bool>) -> impl IntoView {
    // The currency switch and Reset income are in the Net worth view.
    let navigate = use_navigate();
    Effect::new(move |_| navigate("/", Default::default()));
    let index = RwSignal::new(0usize);
    // The highlight around the step's target, its space included.
    let highlight = RwSignal::new(None::<Rect>);
    // The card's top left corner, once measured.
    let card_at = RwSignal::new(None::<(f64, f64)>);
    let card = NodeRef::<html::Section>::new();
    let measure = move || {
        let Some(element) = card.get_untracked() else {
            return;
        };
        let step = STEPS[index.get_untracked()];
        let space = step.space();
        let target = step.target.and_then(target_rect);
        let size = element.get_bounding_client_rect();
        let size = Size {
            width: size.width(),
            height: size.height(),
        };
        let (x, y) = card_position(target, space, size, viewport());
        highlight.set(target.map(|t| Rect {
            left: (t.left - space).max(0.0),
            top: (t.top - space).max(0.0),
            width: t.width + 2.0 * space,
            height: t.height + 2.0 * space,
        }));
        if card_at.get_untracked().is_none() {
            _ = element.focus();
        }
        card_at.set(Some((x, y + step.card_offset())));
    };
    // After the step (and a changed route) has rendered.
    Effect::new(move |_| {
        index.track();
        request_animation_frame(measure);
    });
    let finish = move || {
        open.set(false);
        spawn_local(async move {
            match tauri::invoke::<CompleteTour>(&NoArgs {}).await {
                Ok(true) => set_timeout(move || whats_new.set(true), WHATS_NEW_DELAY),
                Ok(false) => {}
                Err(error) => tauri::log(LogLevel::Error, format!("complete_tour: {error}")),
            }
        });
    };
    let last = STEPS.len() - 1;
    let next = move || index.update(|i| *i = (*i + 1).min(last));
    let back = move || index.update(|i| *i = i.saturating_sub(1));
    let resized = window_event_listener(ev::resize, move |_| measure());
    let keys = window_event_listener(ev::keydown, move |ev| {
        match ev.key().as_str() {
            "Escape" => finish(),
            "ArrowRight" => next(),
            "ArrowLeft" => back(),
            _ => return,
        }
        ev.prevent_default();
    });
    // Keyboard focus stays on the card, as legacy's focus lock kept it.
    let focus = window_event_listener(ev::focusin, move |ev| {
        let Some(element) = card.get_untracked() else {
            return;
        };
        let inside = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
            .is_some_and(|node| element.contains(Some(&node)));
        if !inside {
            _ = element.focus();
        }
    });
    on_cleanup(move || {
        resized.remove();
        keys.remove();
        focus.remove();
    });

    let step = move || STEPS[index.get()];
    let px = |v: f64| format!("{v}px");
    let arrow = |path: &'static str| {
        view! {
            <svg viewBox="0 0 18.4 14.4">
                <path
                    d=path
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-miterlimit="10"
                />
            </svg>
        }
    };
    view! {
        // Blocks clicks on the window, the highlighted element included.
        <div class="tour-blocker"></div>
        {move || match highlight.get() {
            Some(rect) => view! {
                <div
                    class="tour-spotlight"
                    style:left=px(rect.left)
                    style:top=px(rect.top)
                    style:width=px(rect.width)
                    style:height=px(rect.height)
                ></div>
            }
                .into_any(),
            None => view! { <div class="tour-mask"></div> }.into_any(),
        }}
        <section
            class="tour-card"
            node_ref=card
            tabindex="-1"
            role="dialog"
            aria-modal="true"
            style:opacity=move || card_at.get().is_none().then_some("0")
            style:transform=move || {
                card_at.get().map(|(x, y)| format!("translate({}px, {}px)", x.round(), y.round()))
            }
        >
            <h2>{move || LOCALE.key(step().title)}</h2>
            <p>{move || LOCALE.key(step().body)}</p>
            {move || step().body2.map(|body2| view! { <p>{LOCALE.key(body2)}</p> })}
            <span class="tour-badge">{move || index.get() + 1}</span>
            <div class="tour-controls">
                <button
                    class="tour-arrow back"
                    aria-label=LOCALE.key(MessageKey::TourBack)
                    disabled=move || index.get() == 0
                    on:click=move |_| back()
                >
                    {arrow(BACK_ARROW)}
                </button>
                <nav class="tour-dots">
                    {(0..STEPS.len())
                        .map(|i| {
                            view! {
                                <button
                                    class="tour-dot"
                                    class:current=move || index.get() == i
                                    disabled=move || index.get() == i
                                    on:click=move |_| index.set(i)
                                ></button>
                            }
                        })
                        .collect_view()}
                </nav>
                {move || {
                    if index.get() == last {
                        view! {
                            <button class="tour-begin" on:click=move |_| finish()>
                                {LOCALE.key(MessageKey::TourLetsBegin)}
                            </button>
                        }
                            .into_any()
                    } else {
                        view! {
                            <button
                                class="tour-arrow next"
                                aria-label=LOCALE.key(MessageKey::TourNext)
                                on:click=move |_| next()
                            >
                                {arrow(NEXT_ARROW)}
                            </button>
                        }
                            .into_any()
                    }
                }}
            </div>
        </section>
    }
}

/// The window's inner size.
fn viewport() -> Size {
    let window = window();
    let size = |v: Result<wasm_bindgen::JsValue, _>| v.ok().and_then(|v| v.as_f64());
    Size {
        width: size(window.inner_width()).unwrap_or(0.0),
        height: size(window.inner_height()).unwrap_or(0.0),
    }
}

/// The viewport box of the element marked `data-tour="<target>"`, if it is
/// shown; scrolled to the middle first when not wholly in view (legacy).
fn target_rect(target: &str) -> Option<Rect> {
    let element = document()
        .query_selector(&format!("[data-tour=\"{target}\"]"))
        .ok()??;
    let rect = element.get_bounding_client_rect();
    let window = viewport();
    let in_view = rect.top() >= 0.0
        && rect.left() >= 0.0
        && rect.bottom() <= window.height
        && rect.right() <= window.width;
    let rect = if in_view {
        rect
    } else {
        let options = ScrollIntoViewOptions::new();
        options.set_block(ScrollLogicalPosition::Center);
        element.scroll_into_view_with_scroll_into_view_options(&options);
        element.get_bounding_client_rect()
    };
    (rect.width() > 0.0 || rect.height() > 0.0).then(|| Rect {
        left: rect.left(),
        top: rect.top(),
        width: rect.width(),
        height: rect.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f19_tour_has_the_eleven_legacy_steps_without_bulk_sell() {
        let targets: Vec<_> = STEPS.iter().map(|s| s.target).collect();
        assert_eq!(
            targets,
            [
                None,
                Some("net-worth"),
                Some("settings"),
                Some("profile"),
                Some("snapshot"),
                Some("overlay"),
                Some("group"),
                Some("notifications"),
                Some("currency-switch"),
                Some("reset-income"),
                Some("support-panel"),
            ]
        );
        assert_eq!(LOCALE.key(STEPS[10].title), "Support panel");
    }

    #[test]
    fn f19_tour_card_goes_beside_its_target_where_there_is_most_room() {
        let window = Size {
            width: 1150.0,
            height: 800.0,
        };
        let card = Size {
            width: 360.0,
            height: 150.0,
        };
        let rect = |left, top| {
            Some(Rect {
                left,
                top,
                width: 40.0,
                height: 36.0,
            })
        };
        // A toolbar button: below it, aligned with its left edge less space.
        assert_eq!(
            card_position(rect(500.0, 40.0), 16.0, card, window),
            (484.0, 108.0)
        );
        // The title bar's support button at the right edge: left of it.
        assert_eq!(
            card_position(rect(1100.0, 0.0), 0.0, card, window),
            (740.0, 0.0)
        );
        // Near the bottom right corner: left of it, aligned with its bottom
        // edge plus space so the card stays inside.
        assert_eq!(
            card_position(rect(1000.0, 700.0), 16.0, card, window),
            (608.0, 602.0)
        );
        // No target, or no side with room: in the middle.
        assert_eq!(card_position(None, 16.0, card, window), (395.0, 325.0));
        let everywhere = Some(Rect {
            left: 0.0,
            top: 0.0,
            width: 1150.0,
            height: 800.0,
        });
        assert_eq!(
            card_position(everywhere, 16.0, card, window),
            (395.0, 325.0)
        );
    }
}
