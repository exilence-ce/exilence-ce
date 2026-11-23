//! Currency display (F9): the system locale's number format, values in the
//! chosen currency and the ex → div → c switch. Switching applies at once
//! in this window and is stored through `update_settings`.

use std::sync::LazyLock;

use ipc::MessageKey;
use ipc::command::{SettingsPatch, UpdateSettings, UpdateSettingsArgs};
use ipc::currency::{self, NumberFormat};
use ipc::slices::Currency;
use leptos::prelude::*;
use wasm_bindgen::JsValue;

use crate::LOCALE;
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri;

/// The system locale's number separators, read once from
/// `Intl.NumberFormat`.
static NUMBER_FORMAT: LazyLock<NumberFormat> = LazyLock::new(|| {
    let intl = js_sys::Intl::NumberFormat::new(&js_sys::Array::new(), &js_sys::Object::new());
    let part = |part: &JsValue, field: &str| {
        js_sys::Reflect::get(part, &field.into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    };
    separators(
        intl.format_to_parts(1_234_567.5)
            .iter()
            .map(|p| (part(&p, "type"), part(&p, "value"))),
    )
});

/// The system locale's number format.
pub fn number_format() -> NumberFormat {
    *NUMBER_FORMAT
}

/// The separators in the `(type, value)` parts of a formatted number with a
/// fraction; English for those it lacks.
fn separators(parts: impl Iterator<Item = (String, String)>) -> NumberFormat {
    let mut format = NumberFormat {
        decimal: NumberFormat::ENGLISH.decimal,
        group: None,
    };
    for (kind, value) in parts {
        let mut chars = value.chars();
        match (kind.as_str(), chars.next(), chars.next()) {
            ("decimal", Some(c), None) => format.decimal = c,
            ("group", Some(c), None) => format.group = Some(c),
            _ => {}
        }
    }
    format
}

/// `chaos` in the chosen currency with its label, e.g. `- 1.50 div`; with a
/// `+ ` prefix for a positive `change`.
pub fn amount(state: State, chaos: f64, change: bool) -> String {
    let chosen = state.settings.with(|s| s.currency);
    let value = state.prices.with(|prices| {
        if change {
            currency::format_change(chaos, chosen, prices, number_format())
        } else {
            currency::format_value(chaos, chosen, prices, number_format())
        }
    });
    format!("{value} {}", LOCALE.key(currency::short(chosen)))
}

/// Cycles exalt → divine → chaos; its tooltip shows the rates.
#[component]
pub fn CurrencySwitch(state: State) -> impl IntoView {
    let tooltip = move || {
        let rates = state
            .prices
            .with(|p| LOCALE.text(&currency::rates(p, number_format())));
        format!("{rates}\n{}", LOCALE.key(MessageKey::CurrencySwitchHint))
    };
    let switch = move |_| {
        let next = currency::next(state.settings.with_untracked(|s| s.currency));
        state.settings.update(|s| s.currency = next);
        save(next);
    };
    view! {
        <button class="adornment" data-tour="currency-switch" title=tooltip on:click=switch>
            <Icon path=icon::CHANGE_CIRCLE />
        </button>
    }
}

fn save(currency: Currency) {
    tauri::send::<UpdateSettings>(UpdateSettingsArgs {
        patch: SettingsPatch {
            currency: Some(currency),
            ..SettingsPatch::default()
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(parts: &[(&str, &str)]) -> impl Iterator<Item = (String, String)> {
        parts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn f9_system_separators_come_from_intl_parts() {
        let german = parts(&[
            ("integer", "1"),
            ("group", "."),
            ("integer", "234"),
            ("group", "."),
            ("integer", "567"),
            ("decimal", ","),
            ("fraction", "5"),
        ]);
        assert_eq!(
            separators(german),
            NumberFormat {
                decimal: ',',
                group: Some('.')
            }
        );
        let french = parts(&[
            ("integer", "1"),
            ("group", "\u{202f}"),
            ("integer", "234"),
            ("decimal", ","),
            ("fraction", "5"),
        ]);
        assert_eq!(
            separators(french),
            NumberFormat {
                decimal: ',',
                group: Some('\u{202f}')
            }
        );
        let ungrouped = parts(&[("integer", "1234567"), ("decimal", "."), ("fraction", "5")]);
        assert_eq!(
            separators(ungrouped),
            NumberFormat {
                decimal: '.',
                group: None
            }
        );
    }
}
