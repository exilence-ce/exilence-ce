//! Legacy's MUI select (`SelectField.tsx`, `LeagueDropdown.tsx`): an outlined
//! field that opens a menu of its options under it instead of the webview's
//! native popup, and stays highlighted while the menu is open.

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::shell::{Icon, icon};

/// Space MUI's popover keeps between a menu and the window's edges.
const MARGIN: f64 = 16.0;

/// Calls `update` every frame while `open` holds and its owner lives, so a
/// menu placed in the window follows its field when the dialog scrolls or
/// the field grows.
pub fn every_frame_while(open: RwSignal<bool>, update: impl Fn() + Copy + 'static) {
    if open.try_get_untracked() != Some(true) {
        return;
    }
    update();
    request_animation_frame(move || every_frame_while(open, update));
}

/// Where the open menu goes (MUI `Popover`): its top edge under the field,
/// moved up to stay `MARGIN` inside the window; centered on the field, at
/// least as wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuPlace {
    pub left: f64,
    pub top: f64,
    pub min_width: f64,
    pub max_height: f64,
}

/// The menu's place under `field` (its viewport box: left, bottom, width)
/// for a menu of `menu` width × height in a window `height` tall.
pub fn menu_place(field: (f64, f64, f64), menu: (f64, f64), height: f64) -> MenuPlace {
    let (left, bottom, width) = field;
    // MUI's menu paper: at most the window less 96 px.
    let max_height = height - 96.0;
    let menu_width = menu.0.max(width);
    let menu_height = menu.1.min(max_height);
    let top = bottom.min(height - MARGIN - menu_height).max(MARGIN);
    MenuPlace {
        left: left + width / 2.0 - menu_width / 2.0,
        top,
        min_width: width,
        max_height,
    }
}

/// A select: `options` as `(value, text)` pairs, `value` the chosen one,
/// `on_change` called with a newly chosen value. Outlined with its `label`
/// in the border, or `standard`: underlined, as in legacy's toolbar.
#[component]
pub fn OutlinedSelect(
    id: &'static str,
    #[prop(optional, into)] label: Option<String>,
    /// The hover text, for a select without a label.
    #[prop(optional, into)]
    title: Option<String>,
    #[prop(optional)] standard: bool,
    options: Signal<Vec<(String, String)>>,
    value: Signal<String>,
    on_change: Callback<String>,
    disabled: Signal<bool>,
    error: Signal<bool>,
) -> impl IntoView {
    let labelled_by = label.as_ref().map(|_| format!("{id}-label"));
    let open = RwSignal::new(false);
    let place = RwSignal::new(None::<MenuPlace>);
    // The option the keyboard is on.
    let active = RwSignal::new(0usize);
    let root = NodeRef::<html::Div>::new();
    let display = NodeRef::<html::Div>::new();
    let menu = NodeRef::<html::Ul>::new();

    let measure = move || {
        let (Some(Some(root)), Some(Some(menu))) =
            (root.try_get_untracked(), menu.try_get_untracked())
        else {
            return;
        };
        let field = root.get_bounding_client_rect();
        let size = menu.get_bounding_client_rect();
        let height = window()
            .inner_height()
            .ok()
            .and_then(|h| h.as_f64())
            .unwrap_or_default();
        let next = Some(menu_place(
            (field.left(), field.bottom(), field.width()),
            (size.width(), menu.scroll_height() as f64),
            height,
        ));
        if place.try_get_untracked() != Some(next) {
            place.try_set(next);
        }
    };
    let focus_option = move |at: usize| {
        active.set(at);
        let item = menu
            .get_untracked()
            .and_then(|menu| menu.children().item(at as u32))
            .and_then(|item| item.dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(item) = item {
            _ = item.focus();
        }
    };
    let show = move || {
        if disabled.get_untracked() {
            return;
        }
        let chosen = value.get_untracked();
        let at = options.with_untracked(|o| o.iter().position(|(v, _)| *v == chosen));
        active.set(at.unwrap_or_default());
        place.set(None);
        open.set(true);
    };
    // Back to the field, which keeps the focus, as MUI's does.
    let close = move || {
        open.set(false);
        if let Some(display) = display.get_untracked() {
            _ = display.focus();
        }
    };
    let choose = move |chosen: String| {
        close();
        if chosen != value.get_untracked() {
            on_change.run(chosen);
        }
    };
    Effect::new(move |_| {
        if open.get() {
            every_frame_while(open, measure);
            request_animation_frame(move || focus_option(active.get_untracked()));
        }
    });

    let menu_keydown = move |ev: ev::KeyboardEvent| {
        let count = options.with_untracked(Vec::len);
        let at = active.get_untracked();
        match ev.key().as_str() {
            "ArrowDown" => focus_option((at + 1).min(count.saturating_sub(1))),
            "ArrowUp" => focus_option(at.saturating_sub(1)),
            "Home" => focus_option(0),
            "End" => focus_option(count.saturating_sub(1)),
            "Enter" | " " => {
                if let Some((v, _)) = options.with_untracked(|o| o.get(at).cloned()) {
                    choose(v);
                }
            }
            "Escape" => {
                ev.stop_propagation();
                close();
            }
            "Tab" => close(),
            // Jumps to the next option starting with the typed character.
            key if key.chars().count() == 1 => {
                let key = key.to_lowercase();
                let next = options.with_untracked(|o| {
                    (1..=count)
                        .map(|step| (at + step) % count)
                        .find(|&i| o[i].1.to_lowercase().starts_with(&key))
                });
                if let Some(next) = next {
                    focus_option(next);
                }
            }
            _ => return,
        }
        if ev.key() != "Tab" {
            ev.prevent_default();
        }
    };
    let text = move || {
        let chosen = value.get();
        options
            .with(|o| o.iter().find(|(v, _)| *v == chosen).map(|(_, t)| t.clone()))
            // Keeps the line's height when nothing is chosen (MUI).
            .unwrap_or_else(|| "\u{200b}".to_owned())
    };
    let items = move || {
        let chosen = value.get();
        options
            .get()
            .into_iter()
            .enumerate()
            .map(|(at, (v, t))| {
                let selected = v == chosen;
                view! {
                    <li
                        role="option"
                        tabindex="-1"
                        aria-selected=selected.to_string()
                        class:selected=selected
                        on:focus=move |_| active.set(at)
                        on:click=move |_| choose(v.clone())
                    >
                        {t}
                    </li>
                }
            })
            .collect_view()
    };

    view! {
        <div
            class=if standard { "standard-select select" } else { "outlined-field select" }
            class:open=open
            class:error=move || error.get()
            class:disabled=move || disabled.get()
            node_ref=root
        >
            {label.map(|label| view! { <label id=format!("{id}-label")>{label}</label> })}
            <div
                id=id
                class="select-display"
                role="button"
                title=title
                aria-haspopup="listbox"
                aria-labelledby=format!("{id}-label")
                aria-expanded=move || open.get().to_string()
                aria-disabled=move || disabled.get().to_string()
                node_ref=display
                // MUI opens on the press, not the click.
                on:mousedown=move |ev| {
                    if ev.button() == 0 {
                        ev.prevent_default();
                        if let Some(display) = display.get_untracked() {
                            _ = display.focus();
                        }
                        show();
                    }
                }
                on:keydown=move |ev| {
                    if matches!(ev.key().as_str(), " " | "ArrowUp" | "ArrowDown" | "Enter") {
                        ev.prevent_default();
                        show();
                    }
                }
            >
                {text}
            </div>
            <Icon path=icon::ARROW_DROP_DOWN />
            <Show when=move || open.get()>
                // Catches presses outside the menu, as MUI's backdrop does.
                <div
                    class="menu-backdrop"
                    on:mousedown=move |ev| {
                        ev.prevent_default();
                        close();
                    }
                ></div>
                <ul
                    class="menu"
                    role="listbox"
                    aria-labelledby=labelled_by.clone()
                    node_ref=menu
                    class:placed=move || place.get().is_some()
                    style:left=move || place.get().map(|p| format!("{}px", p.left))
                    style:top=move || place.get().map(|p| format!("{}px", p.top))
                    style:min-width=move || place.get().map(|p| format!("{}px", p.min_width))
                    style:max-height=move || place.get().map(|p| format!("{}px", p.max_height))
                    on:keydown=menu_keydown
                >
                    {items}
                </ul>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_opens_under_its_field_and_moves_up_to_stay_in_the_window() {
        // Under a field ending at 300 in an 800 px window.
        let place = menu_place((100.0, 300.0, 452.0), (452.0, 200.0), 800.0);
        assert_eq!(
            (place.left, place.top, place.min_width),
            (100.0, 300.0, 452.0)
        );
        // Too tall to fit under it: moved up to end 16 px above the bottom.
        assert_eq!(
            menu_place((100.0, 700.0, 452.0), (452.0, 200.0), 800.0).top,
            584.0
        );
        // Taller than the window allows: capped, and kept 16 px from the top.
        let tall = menu_place((100.0, 300.0, 452.0), (452.0, 2000.0), 800.0);
        assert_eq!((tall.top, tall.max_height), (80.0, 704.0));
        // Wider than the field: centered on it.
        assert_eq!(
            menu_place((100.0, 300.0, 200.0), (300.0, 100.0), 800.0).left,
            50.0
        );
    }
}
