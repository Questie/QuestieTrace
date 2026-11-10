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
}
