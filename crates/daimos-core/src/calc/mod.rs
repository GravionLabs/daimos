//! The calculator: arithmetic, unit conversion and currency conversion for text typed into the
//! search field, without a prefix. Anything that is not clearly a calculation gives nothing, so
//! ordinary searches never show a calculator result by accident.

pub mod expr;
pub mod provider;
pub mod rates;
pub mod units;

use rates::Rates;
use units::{Dim, Unit};

/// What the calculator found for an input.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The result, as shown in the list: `14` or `7.456454 mi`.
    pub title: String,
    /// What was calculated, or for currencies where the rate comes from.
    pub subtitle: String,
    /// What `Enter` copies.
    pub copy: String,
}

/// Evaluates `input` as a unit or currency conversion (`12 km in mi`) or as arithmetic
/// (`2*(3+4)`); `None` when it is neither.
pub fn evaluate(input: &str, rates: Option<&Rates>) -> Option<Answer> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    conversion(input, rates).or_else(|| arithmetic(input))
}

fn arithmetic(input: &str) -> Option<Answer> {
    let ast = expr::parse(input)?;
    if expr::is_trivial(&ast) {
        return None;
    }
    let value = format_number(expr::eval(&ast)?);
    Some(Answer { title: value.clone(), subtitle: input.to_owned(), copy: value })
}

/// The separators between a quantity and the unit it is converted to.
const SEPARATORS: [&str; 3] = [" in ", " to ", " as "];

fn conversion(input: &str, rates: Option<&Rates>) -> Option<Answer> {
    // The last separator: in `5 in in cm` the first `in` is the unit.
    let lower = input.to_lowercase();
    let (at, separator) = SEPARATORS
        .iter()
        .filter_map(|s| lower.rfind(s).map(|at| (at, *s)))
        .max_by_key(|(at, _)| *at)?;
    let (left, target) = (input.get(..at)?, input.get(at + separator.len()..)?.trim());

    let quantities = parse_quantities(left, rates)?;
    let target_unit = resolve(target, rates)?;
    let dim = quantities.first()?.1.dim;
    if quantities.iter().any(|(_, unit, _)| unit.dim != dim) || target_unit.dim != dim {
        return None;
    }
    if dim == Dim::Temperature && quantities.len() > 1 {
        return None;
    }
    let value = if dim == Dim::Temperature {
        let (v, unit, _) = &quantities[0];
        units::convert(*v, *unit, target_unit)?
    } else {
        let base: f64 = quantities.iter().map(|(v, unit, _)| v * unit.factor).sum();
        base / target_unit.factor
    };
    if !value.is_finite() {
        return None;
    }

    let shown_target = if dim == Dim::Currency { target.to_uppercase() } else { target.to_owned() };
    let number = if dim == Dim::Currency { format_money(value) } else { format_number(value) };
    let mut subtitle = quantities
        .iter()
        .map(|(v, _, name)| format!("{} {name}", format_number(*v)))
        .collect::<Vec<_>>()
        .join(" ");
    if dim == Dim::Currency
        && let Some(rates) = rates
    {
        subtitle.push_str(&format!(" · ECB rates of {}", rates.date));
    }
    Some(Answer {
        title: format!("{number} {shown_target}"),
        subtitle,
        copy: format!("{number} {shown_target}"),
    })
}

/// A unit by name: built in, or a currency of today's rates.
fn resolve(name: &str, rates: Option<&Rates>) -> Option<Unit> {
    let lower = name.trim().to_lowercase();
    if let Some(unit) = units::lookup(&lower) {
        return Some(unit);
    }
    if lower.len() == 3 && lower.chars().all(|c| c.is_ascii_alphabetic()) {
        return rates?.rate(&lower).map(units::currency);
    }
    None
}

/// `12 km`, `12km`, `2 h 30 min`: pairs of a number and a unit.
fn parse_quantities(text: &str, rates: Option<&Rates>) -> Option<Vec<(f64, Unit, String)>> {
    let mut pairs = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let end = rest
            .char_indices()
            .find(|(i, c)| {
                !(c.is_ascii_digit()
                    || *c == '.'
                    || *c == '_'
                    || (*i == 0 && (*c == '-' || *c == '+')))
            })
            .map_or(rest.len(), |(i, _)| i);
        let value: f64 = rest[..end].replace('_', "").parse().ok()?;
        rest = rest[end..].trim_start();
        // The unit is every word up to the next number (`fl oz` has two).
        let mut words = Vec::new();
        while let Some(word) = rest.split_whitespace().next() {
            if word.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
                break;
            }
            // A glued unit stops at a digit: `12km30m` is not supported, `12 km` and `12km` are.
            words.push(word);
            rest = rest[word.len()..].trim_start();
        }
        let name = words.join(" ");
        pairs.push((value, resolve(&name, rates)?, name));
    }
    (!pairs.is_empty()).then_some(pairs)
}

/// At most 10 significant digits and 8 decimals, no trailing zeros; scientific notation for very
/// large and very small values.
pub fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let magnitude = value.abs();
    if !(1e-4..1e15).contains(&magnitude) {
        let text = format!("{value:.9e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let mantissa = trim_zeros(mantissa);
        return format!("{mantissa}e{exponent}");
    }
    let digits_before = magnitude.log10().floor() as i32 + 1;
    let decimals = (10 - digits_before).clamp(0, 8) as usize;
    trim_zeros(&format!("{value:.decimals$}")).to_owned()
}

/// Money: two decimals (four below one unit), no trailing zeros.
fn format_money(value: f64) -> String {
    let decimals = if value.abs() >= 1.0 { 2 } else { 4 };
    trim_zeros(&format!("{value:.decimals$}")).to_owned()
}

fn trim_zeros(text: &str) -> &str {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rates() -> Rates {
        Rates {
            date: "2026-10-09".into(),
            per_eur: [("EUR", 1.0), ("USD", 1.25), ("JPY", 150.0)]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            fetched_at: 0,
        }
    }

    fn title(input: &str) -> Option<String> {
        evaluate(input, Some(&rates())).map(|a| a.title)
    }

    #[test]
    fn arithmetic_gives_the_result_and_the_expression() {
        let answer = evaluate("2*(3+4)", None).unwrap();
        assert_eq!(
            (answer.title.as_str(), answer.subtitle.as_str(), answer.copy.as_str()),
            ("14", "2*(3+4)", "14")
        );
        assert_eq!(title("sqrt 2").as_deref(), Some("1.41421356"));
        assert_eq!(title("2^10").as_deref(), Some("1024"));
        assert_eq!(title("10% of 80").as_deref(), Some("8"));
        assert_eq!(title("0xff").as_deref(), Some("255"));
        assert_eq!(title("1e6 / 7").as_deref(), Some("142857.1429"));
    }

    #[test]
    fn units_convert_with_sensible_rounding() {
        assert_eq!(title("12 km in mi").as_deref(), Some("7.45645431 mi"));
        assert_eq!(title("70 f in c").as_deref(), Some("21.11111111 c"));
        assert_eq!(title("3 gb in mb").as_deref(), Some("3000 mb"));
        assert_eq!(title("2 h 30 min in s").as_deref(), Some("9000 s"));
        assert_eq!(title("100 c to f").as_deref(), Some("212 f"));
        assert_eq!(title("5 in in cm").as_deref(), Some("12.7 cm"));
        assert_eq!(title("12km in m").as_deref(), Some("12000 m"));
        assert_eq!(title("1 fl oz in ml").as_deref(), Some("29.57352956 ml"));
    }

    #[test]
    fn a_conversion_names_its_input_in_the_subtitle() {
        let answer = evaluate("2 h 30 min in s", None).unwrap();
        assert_eq!(answer.subtitle, "2 h 30 min");
        assert_eq!(answer.copy, "9000 s");
    }

    #[test]
    fn currencies_use_the_rates_and_name_their_date() {
        let answer = evaluate("100 usd in eur", Some(&rates())).unwrap();
        assert_eq!(answer.title, "80 EUR");
        assert_eq!(answer.subtitle, "100 usd · ECB rates of 2026-10-09");
        assert_eq!(title("1 eur to jpy").as_deref(), Some("150 JPY"));
        assert_eq!(title("125 usd in jpy").as_deref(), Some("15000 JPY"));
    }

    #[test]
    fn currencies_without_rates_or_with_an_unknown_code_give_nothing() {
        assert_eq!(evaluate("100 usd in eur", None), None);
        assert_eq!(title("100 usd in xyz"), None);
        assert_eq!(title("100 chf in eur"), None);
    }

    #[test]
    fn mismatches_and_non_expressions_give_nothing() {
        for input in [
            "12 km in kg",
            "12 km in",
            "in km",
            "12 furlong in m",
            "3 c in m",
            "10 c + 5 c in f",
            "firefox",
            "42",
            "e",
            "hello in world",
        ] {
            assert_eq!(title(input), None, "{input:?}");
        }
    }

    #[test]
    fn money_has_two_decimals_and_four_below_one() {
        assert_eq!(format_money(89.237_908_26), "89.24");
        assert_eq!(format_money(0.123_456), "0.1235");
        assert_eq!(format_money(150.0), "150");
        let rates = rates();
        assert_eq!(evaluate("100 jpy in usd", Some(&rates)).unwrap().title, "0.8333 USD");
    }

    #[test]
    fn numbers_are_rounded_and_trimmed() {
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.33333333");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(123_456_789.123_456), "123456789.1");
        assert_eq!(format_number(1e15), "1e15");
        assert_eq!(format_number(1.5e-7), "1.5e-7");
        assert_eq!(format_number(2.0f64.powi(60)), "1.152921505e18");
    }
}
