//! Numbers in text (evidence, notes, the review, Lua comments) as the TypeScript pipeline prints
//! them, so both pipelines' text outputs read the same. The values never differ; only exact
//! rounding ties and a few spellings do between JS and Rust formatting.

/// JS `x.toFixed(digits)` for numbers below 1e21. Rust's `{:.N}` rounds an exact decimal tie to
/// even ("0.12" for 0.125), JS away from zero ("0.13"), and ratios of small counts hit those ties
/// often (2/16, 13/16). With `digits` decimals the exact ties are the odd multiples of
/// 2^-(digits + 1). Negative zero prints unsigned, infinities as JS words.
pub fn to_fixed(x: f64, digits: usize) -> String {
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    // Adding 0.0 turns -0.0 into 0.0.
    let x = x + 0.0;
    let scaled = x.abs() * 2f64.powi(i32::try_from(digits).unwrap_or(i32::MAX) + 1);
    let tie = scaled.fract() == 0.0 && scaled % 2.0 == 1.0;
    let away_from_zero = if x > 0.0 { x.next_up() } else { x.next_down() };
    format!("{:.digits$}", if tie { away_from_zero } else { x })
}

/// JS `x.toExponential(digits)`: like Rust's `{:.Ne}`, but with an explicit sign on non-negative
/// exponents ("1.2e-2", "1.0e+0").
pub fn to_exponential(x: f64, digits: usize) -> String {
    let formatted = format!("{x:.digits$e}");
    match formatted.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => format!("{mantissa}e+{exponent}"),
        _ => formatted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_prints_what_js_prints() {
        // Expected values from Node: (x).toFixed(digits).
        assert_eq!(to_fixed(2.0 / 16.0, 2), "0.13", "an exact tie rounds up, not to even");
        assert_eq!(to_fixed(0.8125, 3), "0.813");
        assert_eq!(to_fixed(3.0 / 8.0, 2), "0.38");
        assert_eq!(to_fixed(12.25, 1), "12.3");
        assert_eq!(to_fixed(100.25, 1), "100.3");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(9.5, 0), "10", "the carry runs through nines");
        assert_eq!(to_fixed(-0.125, 2), "-0.13", "a negative tie rounds away from zero");
        assert_eq!(to_fixed(-0.25, 1), "-0.3");
        // Not ties: these doubles lie just below the decimal tie.
        assert_eq!(to_fixed(0.15, 1), "0.1");
        assert_eq!(to_fixed(0.35, 1), "0.3");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(886.15, 1), "886.1");
        // Ordinary rounding.
        assert_eq!(to_fixed(1.0 / 3.0, 2), "0.33");
        assert_eq!(to_fixed(0.99523, 3), "0.995");
        assert_eq!(to_fixed(86.5533093999984, 1), "86.6");
        // Signs and words.
        assert_eq!(to_fixed(-0.04, 1), "-0.0");
        assert_eq!(to_fixed(-0.0, 1), "0.0");
        assert_eq!(to_fixed(f64::NEG_INFINITY, 1), "-Infinity");
    }

    #[test]
    fn to_exponential_signs_non_negative_exponents() {
        assert_eq!(to_exponential(0.0123, 1), "1.2e-2");
        assert_eq!(to_exponential(0.0, 1), "0.0e+0");
        assert_eq!(to_exponential(1234.0, 1), "1.2e+3");
    }
}
