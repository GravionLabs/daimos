//! Property tests of the calculator: precedence and unit round trips.

use daimos_core::calc::evaluate;
use daimos_core::calc::expr::{eval, parse};
use daimos_core::calc::rates::Rates;
use daimos_core::calc::units::{convert, currency, lookup};
use proptest::prelude::*;

fn calc(input: &str) -> f64 {
    eval(&parse(input).unwrap()).unwrap()
}

proptest! {
    #[test]
    fn multiplication_binds_tighter_than_addition(a in -1000i32..1000, b in -1000i32..1000, c in -1000i32..1000) {
        prop_assert_eq!(calc(&format!("{a}+{b}*{c}")), f64::from(a) + f64::from(b) * f64::from(c));
        prop_assert_eq!(calc(&format!("{a}*{b}+{c}")), f64::from(a) * f64::from(b) + f64::from(c));
        prop_assert_eq!(calc(&format!("({a}+{b})*{c}")), (f64::from(a) + f64::from(b)) * f64::from(c));
    }

    #[test]
    fn subtraction_and_division_associate_to_the_left(a in -500i32..500, b in 1i32..50, c in 1i32..50) {
        prop_assert_eq!(calc(&format!("{a}-{b}-{c}")), f64::from(a) - f64::from(b) - f64::from(c));
        let expected = f64::from(a) / f64::from(b) / f64::from(c);
        let got = calc(&format!("{a}/{b}/{c}"));
        prop_assert!((got - expected).abs() < 1e-9);
    }

    #[test]
    fn power_is_right_associative_and_binds_tighter_than_a_minus(a in 1u32..4, b in 1u32..4, c in 1u32..3) {
        let expected = f64::from(a).powf(f64::from(b).powf(f64::from(c)));
        let got = calc(&format!("{a}^{b}^{c}"));
        prop_assert!((got - expected).abs() < 1e-9 * expected.max(1.0));
        prop_assert_eq!(calc(&format!("-{a}^2")), -f64::from(a * a));
    }

    #[test]
    fn a_percentage_added_is_a_share_of_the_left_operand(a in 1u32..1000, p in 1u32..100) {
        let expected = f64::from(a) * (1.0 + f64::from(p) / 100.0);
        let got = calc(&format!("{a} + {p}%"));
        prop_assert!((got - expected).abs() < 1e-9);
    }

    #[test]
    fn the_calculator_never_panics_on_text(input in ".{0,40}") {
        let _ = evaluate(&input, None);
    }
}

/// Units of one dimension, to convert between all pairs.
const GROUPS: &[&[&str]] = &[
    &["mm", "cm", "m", "km", "in", "ft", "yd", "mi"],
    &["mg", "g", "kg", "oz", "lb", "t"],
    &["ms", "s", "min", "h", "d", "w"],
    &["ml", "l", "tsp", "cup", "gal", "qt"],
    &["b", "kb", "mb", "gb", "kib", "gib"],
    &["m/s", "km/h", "mph", "kn"],
    &["c", "f", "k"],
];

proptest! {
    #[test]
    fn converting_there_and_back_gives_the_start(group in 0..GROUPS.len(), i in 0usize..8, j in 0usize..8, value in -1.0e6f64..1.0e6) {
        let units = GROUPS[group];
        let (from, to) = (lookup(units[i % units.len()]).unwrap(), lookup(units[j % units.len()]).unwrap());
        let there = convert(value, from, to).unwrap();
        let back = convert(there, to, from).unwrap();
        prop_assert!((back - value).abs() <= 1e-9 * value.abs().max(1.0), "{value} -> {there} -> {back}");
    }

    #[test]
    fn currencies_round_trip_through_the_euro(rate_a in 0.01f64..500.0, rate_b in 0.01f64..500.0, value in 0.0f64..1.0e6) {
        let (a, b) = (currency(rate_a), currency(rate_b));
        let back = convert(convert(value, a, b).unwrap(), b, a).unwrap();
        prop_assert!((back - value).abs() <= 1e-9 * value.max(1.0));
    }

    #[test]
    fn a_conversion_text_agrees_with_the_unit_table(value in 1u32..10_000) {
        let rates = Rates { date: "d".into(), per_eur: Default::default(), fetched_at: 0 };
        let answer = evaluate(&format!("{value} km in m"), Some(&rates)).unwrap();
        prop_assert_eq!(answer.title, format!("{} m", value * 1000));
    }
}
