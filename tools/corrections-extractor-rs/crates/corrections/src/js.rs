//! JavaScript semantics the TypeScript tool relies on, where Rust's own differ. Ported code
//! uses these instead of the Rust equivalents so the output stays the same.

pub use trace_data::format_number;

/// JS `Math.round`: halves round toward +infinity (`-2.5` becomes `-2`), unlike `f64::round`.
pub fn round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    let rounded = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    // Math.round(-0.4) is -0.
    if rounded == 0.0 {
        0f64.copysign(x)
    } else {
        rounded
    }
}

/// JS `\s` and `String.prototype.trim`'s whitespace (Rust's `trim` misses U+FEFF).
pub fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// JS `String.prototype.trim`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_whitespace)
}

/// JS `Number.isInteger`.
pub fn is_integer(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0
}

/// JS `Number(s)` for the strings trace data holds: decimal numbers, surrounding whitespace
/// ignored, empty means 0; anything else is NaN. (Hex, octal and binary literals are not
/// expected in trace data and read as NaN.)
pub fn number(s: &str) -> f64 {
    let s = trim(s);
    if s.is_empty() {
        return 0.0;
    }
    match s {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust also accepts "inf", "nan" and friends, which are NaN in JS.
    if s.bytes().any(|b| b.is_ascii_alphabetic() && b != b'e' && b != b'E') {
        return f64::NAN;
    }
    s.parse().unwrap_or(f64::NAN)
}

/// JS `String(x)` for a decoded value; None is `undefined`.
pub fn string(value: Option<&trace_data::Value>) -> String {
    use trace_data::Value;
    let Some(value) = value else { return "undefined".to_owned() };
    match value.resolved() {
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => format_number(*n),
        Value::Str(s) => s.to_string(),
        // Array.prototype.join: decoded arrays hold no nil, so every item is stringified.
        Value::Array(items) => items.iter().map(|item| string(Some(item))).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Lazy(_) => unreachable!("resolved() never returns a lazy value"),
    }
}

/// JS `Math.min(...values)`: NaN when any value is NaN, Infinity for none.
pub fn min(values: &[f64]) -> f64 {
    values
        .iter()
        .fold(f64::INFINITY, |min, &v| if min.is_nan() || v.is_nan() { f64::NAN } else { min.min(v) })
}

/// JS `Math.max(...values)`: NaN when any value is NaN, -Infinity for none.
pub fn max(values: &[f64]) -> f64 {
    values
        .iter()
        .fold(f64::NEG_INFINITY, |max, &v| if max.is_nan() || v.is_nan() { f64::NAN } else { max.max(v) })
}

/// JS `Math.sign`: ±0 and NaN keep themselves.
pub fn sign(x: f64) -> f64 {
    if x == 0.0 || x.is_nan() {
        x
    } else {
        x.signum()
    }
}

/// JS `Math.hypot(a, b)` as V8 computes it (normalized Kahan sum); it differs from
/// `f64::hypot` in the last bit for about a third of inputs.
pub fn hypot(a: f64, b: f64) -> f64 {
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let (a, b) = (a.abs(), b.abs());
    let max = a.max(b);
    if max == 0.0 {
        return 0.0;
    }
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for value in [a, b] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// JS `x.toFixed(4)` for finite numbers below 1e21 (coordinates): Rust rounds an exact tie to
/// even, JS away from zero; with 4 decimals the ties are the odd multiples of 1/32. JS prints
/// -0 as "0.0000".
pub fn to_fixed_4(x: f64) -> String {
    let x = x + 0.0;
    let thirty_seconds = x.abs() * 32.0;
    let tie = thirty_seconds.fract() == 0.0 && thirty_seconds % 2.0 == 1.0;
    let away_from_zero = if x > 0.0 { x.next_up() } else { x.next_down() };
    format!("{:.4}", if tie { away_from_zero } else { x })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_matches_math_round_on_halves_and_negatives() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert!(round(-0.4).is_sign_negative());
        assert_eq!(round(1234.5678 * 100.0) / 100.0, 1234.57);
    }

    #[test]
    fn number_parses_like_js_number() {
        assert_eq!(number(" 42 "), 42.0);
        assert_eq!(number(""), 0.0);
        assert_eq!(number("1e3"), 1000.0);
        assert!(number("inf").is_nan());
        assert!(number("12abc").is_nan());
    }

    #[test]
    fn trim_removes_the_byte_order_mark_like_js() {
        assert_eq!(trim("\u{feff} Elwynn \n"), "Elwynn");
    }

    #[test]
    fn stringifies_types_like_js_string() {
        let types = serde_json::json!({ "t": [true, 1.5, "event", ["event"], [1, ["a", 2]], { "k": 1 }] });
        let strings = crate::test_support::with_session(&crate::test_support::session_json(types, vec![]), |s| {
            let items = s.function("t").and_then(trace_data::Value::as_array).expect("an array");
            items.iter().map(|item| string(Some(item))).collect::<Vec<_>>()
        });
        assert_eq!(strings, ["true", "1.5", "event", "event", "1,a,2", "[object Object]"]);
        assert_eq!(string(None), "undefined");
    }

    #[test]
    fn to_fixed_4_rounds_ties_away_from_zero_like_js() {
        // Expected values printed by node: (x).toFixed(4).
        assert_eq!(to_fixed_4(30.03125), "30.0313");
        assert_eq!(to_fixed_4(-0.03125), "-0.0313");
        assert_eq!(to_fixed_4(0.09375), "0.0938");
        assert_eq!(to_fixed_4(62.5625), "62.5625");
        assert_eq!(to_fixed_4(-0.0), "0.0000");
    }

    #[test]
    fn hypot_matches_v8_where_libm_differs() {
        // node: Math.hypot(-40.17, 61.260000000000005) === 73.25582911959975; f64::hypot gives ...77.
        assert_eq!(hypot(-40.17, 61.260000000000005), 73.25582911959975);
    }

    #[test]
    fn min_max_and_sign_keep_nan_like_js() {
        assert_eq!((min(&[3.0, 1.0, 2.0]), max(&[3.0, 1.0, 2.0])), (1.0, 3.0));
        assert!(min(&[1.0, f64::NAN]).is_nan() && max(&[f64::NAN, 1.0]).is_nan());
        assert_eq!((min(&[]), max(&[])), (f64::INFINITY, f64::NEG_INFINITY));
        assert!(sign(-0.0).is_sign_negative() && sign(-0.0) == 0.0);
        assert_eq!((sign(-7.5), sign(2.0)), (-1.0, 1.0));
    }
}
