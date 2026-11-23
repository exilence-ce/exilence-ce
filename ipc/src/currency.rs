//! The currency format helper (F9): converts pushed chaos values into the
//! chosen currency with the `prices` slice's rates and formats them with 2
//! decimals in the system locale, as legacy `formatValue` does.

use crate::message::{Message, MessageKey};
use crate::slices::{Currency, Prices};

/// The number separators of a locale. The UI reads the system locale's once;
/// formatting itself is plain Rust, so it is tested natively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberFormat {
    pub decimal: char,
    /// Between groups of 3 integer digits; `None` without grouping.
    pub group: Option<char>,
}

impl NumberFormat {
    /// `1,234.56`.
    pub const ENGLISH: Self = Self {
        decimal: '.',
        group: Some(','),
    };

    /// `value` rounded to exactly `decimals` decimals, with a leading `-`
    /// when negative. Never `-0`.
    pub fn format(&self, value: f64, decimals: usize) -> String {
        let fixed = format!("{:.*}", decimals, value.abs());
        let (integer, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
        let negative = value < 0.0 && fixed.bytes().any(|b| matches!(b, b'1'..=b'9'));
        let mut text = String::with_capacity(fixed.len() + integer.len() / 3 + 1);
        if negative {
            text.push('-');
        }
        for (i, digit) in integer.chars().enumerate() {
            if i > 0 && (integer.len() - i) % 3 == 0 {
                text.extend(self.group);
            }
            text.push(digit);
        }
        if !fraction.is_empty() {
            text.push(self.decimal);
            text.push_str(fraction);
        }
        text
    }
}

/// Chaos per unit of `currency`: 1 for chaos, else the active price
/// league's rate (the custom price if set); `None` without a positive rate.
pub fn rate(currency: Currency, prices: &Prices) -> Option<f64> {
    match currency {
        Currency::Chaos => Some(1.0),
        Currency::Exalted => prices.exalted_rate,
        Currency::Divine => prices.divine_rate,
    }
    .filter(|rate| *rate > 0.0)
}

/// `chaos` in `currency`; `None` without a rate.
pub fn convert(chaos: f64, currency: Currency, prices: &Prices) -> Option<f64> {
    rate(currency, prices).map(|rate| chaos / rate)
}

/// `chaos` in `currency` with 2 decimals: `0` without a rate or when it
/// rounds to 0; negative values as `- 1.50` (legacy).
pub fn format_value(
    chaos: f64,
    currency: Currency,
    prices: &Prices,
    number: NumberFormat,
) -> String {
    format(chaos, currency, prices, number, false)
}

/// Like [`format_value`], with a `+ ` prefix for a positive change.
pub fn format_change(
    chaos: f64,
    currency: Currency,
    prices: &Prices,
    number: NumberFormat,
) -> String {
    format(chaos, currency, prices, number, true)
}

fn format(
    chaos: f64,
    currency: Currency,
    prices: &Prices,
    number: NumberFormat,
    change: bool,
) -> String {
    let Some(value) = convert(chaos, currency, prices) else {
        return "0".to_owned();
    };
    let text = number.format(value.abs(), 2);
    if (value * 100.0).round() == 0.0 {
        "0".to_owned()
    } else if value < 0.0 {
        format!("- {text}")
    } else if change {
        format!("+ {text}")
    } else {
        text
    }
}

/// The short label of `currency`: `c`, `ex` or `div`.
pub fn short(currency: Currency) -> MessageKey {
    match currency {
        Currency::Chaos => MessageKey::CurrencyChaos,
        Currency::Exalted => MessageKey::CurrencyExalted,
        Currency::Divine => MessageKey::CurrencyDivine,
    }
}

/// The currency the switch selects after `currency`: exalted → divine →
/// chaos → exalted.
pub fn next(currency: Currency) -> Currency {
    match currency {
        Currency::Exalted => Currency::Divine,
        Currency::Divine => Currency::Chaos,
        Currency::Chaos => Currency::Exalted,
    }
}

/// The switch's tooltip rates, `1 ex = X.X c / 1 divine = X.X c`; a missing
/// rate shows as `0`.
pub fn rates(prices: &Prices, number: NumberFormat) -> Message {
    let rate = |currency| rate(currency, prices).map_or("0".to_owned(), |r| number.format(r, 1));
    Message::new(MessageKey::CurrencySwitchRates)
        .arg("exalted", rate(Currency::Exalted))
        .arg("divine", rate(Currency::Divine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Locale;

    const GERMAN: NumberFormat = NumberFormat {
        decimal: ',',
        group: Some('.'),
    };

    fn prices(exalted: Option<f64>, divine: Option<f64>) -> Prices {
        Prices {
            exalted_rate: exalted,
            divine_rate: divine,
            ..Prices::default()
        }
    }

    #[test]
    fn f9_numbers_have_2_decimals_in_the_locale() {
        let en = NumberFormat::ENGLISH;
        assert_eq!(en.format(1234567.891, 2), "1,234,567.89");
        assert_eq!(en.format(123.0, 2), "123.00");
        assert_eq!(en.format(0.005, 2), "0.01");
        assert_eq!(en.format(-1234.5, 2), "-1,234.50");
        assert_eq!(en.format(-0.001, 2), "0.00", "never -0");
        assert_eq!(GERMAN.format(1234.5, 2), "1.234,50");
        let swiss = NumberFormat {
            decimal: '.',
            group: Some('\u{2019}'),
        };
        assert_eq!(swiss.format(1234.5, 1), "1\u{2019}234.5");
        let ungrouped = NumberFormat {
            decimal: ',',
            group: None,
        };
        assert_eq!(ungrouped.format(1234.5, 2), "1234,50");
    }

    #[test]
    fn f9_converts_with_the_price_leagues_rates() {
        let prices = prices(Some(200.0), Some(50.0));
        assert_eq!(convert(500.0, Currency::Chaos, &prices), Some(500.0));
        assert_eq!(convert(500.0, Currency::Exalted, &prices), Some(2.5));
        assert_eq!(convert(500.0, Currency::Divine, &prices), Some(10.0));
        let none = self::prices(None, Some(0.0));
        assert_eq!(convert(500.0, Currency::Exalted, &none), None);
        assert_eq!(
            convert(500.0, Currency::Divine, &none),
            None,
            "0 is no rate"
        );
        assert_eq!(convert(500.0, Currency::Chaos, &none), Some(500.0));
    }

    #[test]
    fn f9_values_and_changes() {
        let prices = prices(Some(200.0), Some(150.0));
        let en = NumberFormat::ENGLISH;
        assert_eq!(
            format_value(1234.5, Currency::Chaos, &prices, en),
            "1,234.50"
        );
        assert_eq!(format_value(300.0, Currency::Divine, &prices, en), "2.00");
        assert_eq!(
            format_value(300.0, Currency::Exalted, &prices, GERMAN),
            "1,50"
        );
        assert_eq!(
            format_change(12.345, Currency::Chaos, &prices, en),
            "+ 12.35"
        );
        assert_eq!(format_change(-3.0, Currency::Chaos, &prices, en), "- 3.00");
        assert_eq!(format_value(-3.0, Currency::Chaos, &prices, en), "- 3.00");
        assert_eq!(format_change(0.0, Currency::Chaos, &prices, en), "0");
        assert_eq!(
            format_change(0.004, Currency::Chaos, &prices, en),
            "0",
            "rounds to 0"
        );
        assert_eq!(
            format_value(150.0, Currency::Chaos, &prices, en),
            "150.00",
            "no + on values"
        );
    }

    #[test]
    fn f9_values_show_0_without_a_rate() {
        let none = prices(None, None);
        let en = NumberFormat::ENGLISH;
        assert_eq!(format_value(500.0, Currency::Divine, &none, en), "0");
        assert_eq!(format_change(500.0, Currency::Exalted, &none, en), "0");
    }

    #[test]
    fn f9_switch_cycles_exalted_divine_chaos_with_its_rates() {
        assert_eq!(next(Currency::Exalted), Currency::Divine);
        assert_eq!(next(Currency::Divine), Currency::Chaos);
        assert_eq!(next(Currency::Chaos), Currency::Exalted);
        let locale = Locale::english();
        let labels: Vec<String> = [Currency::Exalted, Currency::Divine, Currency::Chaos]
            .map(|c| locale.key(short(c)))
            .to_vec();
        assert_eq!(labels, ["ex", "div", "c"]);
        assert_eq!(
            locale.text(&rates(
                &prices(Some(12.34), Some(180.0)),
                NumberFormat::ENGLISH
            )),
            "1 ex = 12.3 c / 1 divine = 180.0 c"
        );
        assert_eq!(
            locale.text(&rates(&prices(None, None), NumberFormat::ENGLISH)),
            "1 ex = 0 c / 1 divine = 0 c"
        );
    }
}
