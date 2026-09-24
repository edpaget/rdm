//! JavaScript-compatible numerics for the measurement tools.
//!
//! The Rust tools replace JavaScript instruments whose committed outputs
//! (golden files, `docs/token-baseline.json` figures) were produced by V8, so
//! every place a number is rounded, printed or parsed follows the JavaScript
//! rule rather than Rust's:
//!
//! - [`fmt_number`] is `String(n)` / `Number.prototype.toString()` (shortest
//!   round-trip digits, exponent form outside `1e-7 .. 1e21`).
//! - [`to_fixed`] is `Number.prototype.toFixed`, which rounds the double's
//!   exact decimal value and breaks an exact tie towards the larger magnitude
//!   (Rust's `{:.1}` breaks it to even).
//! - [`to_locale_en_us`] is `toLocaleString('en-US')` (thousands separators,
//!   at most three fraction digits, half-expand rounding).
//! - [`js_round`] is `Math.round` (half towards +∞).
//! - [`percentile`] is the linear-interpolation percentile the JS tools used.
//! - [`to_number`] is `Number(string)`, for argument validation.
//! - [`serialize_js_number`] makes a serde `f64` field serialize as a JSON
//!   integer when it is integral (JS prints `50`, not `50.0`).
//!
//! Golden comparisons over the tools' real outputs catch any divergence here.

use serde::Serializer;

/// Largest integer every `f64` below it represents exactly (2^53).
const MAX_SAFE: f64 = 9_007_199_254_740_992.0;

/// `Math.round`: rounds half towards positive infinity.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let r = x.round();
    // Rust rounds half away from zero; JS rounds a negative half up.
    if x < 0.0 && (r - x) == -0.5 {
        r + 1.0
    } else {
        r
    }
}

/// `whole ? Math.round((part / whole) * 1000) / 10 : 0` — the one-decimal
/// percentage every JS instrument reports.
pub fn pct(part: f64, whole: f64) -> f64 {
    if whole == 0.0 || whole.is_nan() {
        0.0
    } else {
        js_round((part / whole) * 1000.0) / 10.0
    }
}

/// Splits a finite, positive, non-zero double into its shortest round-trip
/// decimal digits and the exponent `n` such that the value is
/// `0.<digits> × 10^n`.
fn shortest_digits(x: f64) -> (String, i32) {
    // `{:e}` prints the shortest digits that round-trip, as `d.ddddde<exp>`.
    let s = format!("{x:e}");
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    (digits.to_owned(), exp + 1)
}

/// `String(n)` for a JavaScript number.
pub fn fmt_number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if x == 0.0 {
        return "0".to_owned();
    }
    if x < 0.0 {
        return format!("-{}", fmt_number(-x));
    }
    let (digits, n) = shortest_digits(x);
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if k <= n && n <= 21 {
        let zeros = usize::try_from(n - k).unwrap_or(0);
        return format!("{digits}{}", "0".repeat(zeros));
    }
    if 0 < n && n <= 21 {
        let at = usize::try_from(n).unwrap_or(0);
        return format!("{}.{}", &digits[..at], &digits[at..]);
    }
    if -6 < n && n <= 0 {
        let zeros = usize::try_from(-n).unwrap_or(0);
        return format!("0.{}{digits}", "0".repeat(zeros));
    }
    let e = n - 1;
    let sign = if e < 0 { '-' } else { '+' };
    if digits.len() == 1 {
        format!("{digits}e{sign}{}", e.abs())
    } else {
        format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
    }
}

/// Adds one unit in the last place of a string of ASCII digits, returning
/// whether the addition carried out of the most significant digit.
fn increment_digits(digits: &mut [u8]) -> bool {
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d += 1;
            return false;
        }
    }
    true
}

/// `Number.prototype.toFixed(fraction_digits)`.
pub fn to_fixed(x: f64, fraction_digits: usize) -> String {
    if !x.is_finite() {
        return fmt_number(x);
    }
    if x.abs() >= 1e21 {
        return fmt_number(x);
    }
    let negative = x < 0.0;
    // Every double below 1e21 has a finite exact decimal expansion; 1100
    // fraction digits is always enough to hold it exactly.
    let exact = format!("{:.1100}", x.abs());
    let (int_part, frac_part) = exact.split_once('.').unwrap_or((&exact, ""));
    let frac = frac_part.as_bytes();
    let keep = &frac[..fraction_digits.min(frac.len())];
    let rest = &frac[fraction_digits.min(frac.len())..];
    // JS picks the larger n on an exact tie, i.e. round half up in magnitude.
    let round_up = rest.first().is_some_and(|&d| d >= b'5');
    let mut digits: Vec<u8> = int_part.bytes().chain(keep.iter().copied()).collect();
    if round_up && increment_digits(&mut digits) {
        digits.insert(0, b'1');
    }
    let split = digits.len() - fraction_digits;
    let int_str = String::from_utf8_lossy(&digits[..split]).into_owned();
    let frac_str = String::from_utf8_lossy(&digits[split..]).into_owned();
    let body = if fraction_digits == 0 {
        int_str
    } else {
        format!("{int_str}.{frac_str}")
    };
    let is_zero = digits.iter().all(|&d| d == b'0');
    if negative && !is_zero {
        format!("-{body}")
    } else {
        body
    }
}

/// `Number.prototype.toLocaleString('en-US')`: grouped integer part, at most
/// three fraction digits (half-expand), trailing zeros dropped.
pub fn to_locale_en_us(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "∞" } else { "-∞" }.to_owned();
    }
    let negative = x < 0.0;
    let abs = x.abs();
    // Round the shortest decimal representation (what ICU formats) to three
    // fraction digits, half away from zero.
    let plain = plain_decimal(abs);
    let (int_part, frac_part) = plain.split_once('.').unwrap_or((&plain, ""));
    let frac = frac_part.as_bytes();
    let keep = &frac[..3.min(frac.len())];
    let round_up = frac.get(3).is_some_and(|&d| d >= b'5');
    let mut digits: Vec<u8> = int_part.bytes().chain(keep.iter().copied()).collect();
    if round_up && increment_digits(&mut digits) {
        digits.insert(0, b'1');
    }
    let split = digits.len() - keep.len();
    let int_digits = &digits[..split];
    let mut frac_digits: Vec<u8> = digits[split..].to_vec();
    while frac_digits.last() == Some(&b'0') {
        frac_digits.pop();
    }
    let mut grouped = String::new();
    for (i, d) in int_digits.iter().enumerate() {
        if i > 0 && (int_digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(char::from(*d));
    }
    if grouped.is_empty() {
        grouped.push('0');
    }
    let body = if frac_digits.is_empty() {
        grouped
    } else {
        format!("{grouped}.{}", String::from_utf8_lossy(&frac_digits))
    };
    let is_zero = body.bytes().all(|b| b == b'0' || b == b'.' || b == b',');
    if negative && !is_zero {
        format!("-{body}")
    } else {
        body
    }
}

/// The shortest round-trip digits of a finite non-negative double, written
/// out without an exponent.
fn plain_decimal(x: f64) -> String {
    if x == 0.0 {
        return "0".to_owned();
    }
    let (digits, n) = shortest_digits(x);
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if n >= k {
        let zeros = usize::try_from(n - k).unwrap_or(0);
        format!("{digits}{}", "0".repeat(zeros))
    } else if n > 0 {
        let at = usize::try_from(n).unwrap_or(0);
        format!("{}.{}", &digits[..at], &digits[at..])
    } else {
        let zeros = usize::try_from(-n).unwrap_or(0);
        format!("0.{}{digits}", "0".repeat(zeros))
    }
}

/// Linear-interpolation percentile (NumPy `'linear'`, Excel
/// `PERCENTILE.INC`) of `values`, which need not be sorted. `p` is in
/// `[0, 1]`. An empty slice yields `NaN`.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    if n == 1 {
        return sorted[0];
    }
    #[allow(clippy::cast_precision_loss)]
    let idx = (n - 1) as f64 * p;
    let lower = idx.floor();
    let upper = idx.ceil();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (lo, hi) = (lower as usize, upper as usize);
    if lo == hi {
        return sorted[lo];
    }
    let weight = idx - lower;
    sorted[lo] + (sorted[hi] - sorted[lo]) * weight
}

/// JavaScript's `StrWhiteSpaceChar`: what `String.prototype.trim` and
/// `Number()` strip.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// `Number(string)`: the JS string-to-number conversion, used where the JS
/// tools validated an argument with `Number(raw)`.
pub fn to_number(raw: &str) -> f64 {
    let s = js_trim(raw);
    if s.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if rest.is_empty() || !rest.chars().all(|c| c.is_digit(radix)) {
                return f64::NAN;
            }
            return rest.chars().fold(0.0, |acc, c| {
                acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
            });
        }
    }
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    if unsigned == "Infinity" {
        return if s.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    // StrDecimalLiteral: digits [. digits] [e[+-]digits] | . digits [...]
    let bytes = unsigned.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits_seen = i > int_start;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        digits_seen |= i > frac_start;
    }
    if !digits_seen {
        return f64::NAN;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return f64::NAN;
        }
    }
    if i != bytes.len() {
        return f64::NAN;
    }
    s.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Number.isInteger(x)`.
pub fn is_integer(x: f64) -> bool {
    x.is_finite() && x.trunc() == x
}

/// Converts an integral `f64` to `i64` when it is exactly representable.
pub fn as_exact_i64(x: f64) -> Option<i64> {
    if is_integer(x) && x.abs() < MAX_SAFE {
        #[allow(clippy::cast_possible_truncation)]
        Some(x as i64)
    } else {
        None
    }
}

/// Serde helper: serializes an `f64` the way `JSON.stringify` prints it for
/// every value these tools produce — an integral value as a JSON integer.
///
/// # Errors
///
/// Whatever the serializer returns.
pub fn serialize_js_number<S: Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error> {
    match as_exact_i64(*x) {
        Some(i) => s.serialize_i64(i),
        None if x.is_finite() => s.serialize_f64(*x),
        // JSON.stringify prints a non-finite number as null.
        None => s.serialize_none(),
    }
}

/// Serde helper for `Option<f64>` (`null` when absent).
///
/// # Errors
///
/// Whatever the serializer returns.
pub fn serialize_opt_js_number<S: Serializer>(x: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match x {
        Some(v) => serialize_js_number(v, s),
        None => s.serialize_none(),
    }
}

/// A `serde_json::Value` number for an `f64`, integral values as integers.
pub fn json_number(x: f64) -> serde_json::Value {
    match as_exact_i64(x) {
        Some(i) => serde_json::Value::from(i),
        None => serde_json::Number::from_f64(x).map_or(serde_json::Value::Null, Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsnum_integral_and_to_fixed() {
        assert_eq!(fmt_number(50.0), "50");
        assert_eq!(fmt_number(-0.0), "0");
        assert_eq!(fmt_number(2.5), "2.5");
        assert_eq!(fmt_number(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(fmt_number(1e21), "1e+21");
        assert_eq!(fmt_number(123e-20), "1.23e-18");
        assert_eq!(fmt_number(0.000001), "0.000001");
        assert_eq!(fmt_number(1e-7), "1e-7");
        assert_eq!(fmt_number(57.1), "57.1");
        assert_eq!(fmt_number(1e20), "100000000000000000000");
        // toFixed breaks an exact tie upward (0.25 and 12.25 are exact doubles).
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(12.25, 1), "12.3");
        assert_eq!(to_fixed(57.1, 1), "57.1");
        assert_eq!(to_fixed(0.0, 1), "0.0");
        assert_eq!(to_fixed(100.0, 1), "100.0");
        assert_eq!(to_fixed(9.96, 1), "10.0");
        // 1.005 is below the tie in binary, so it rounds down, as in JS.
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(-2.5, 0), "-3");
        let json = serde_json::to_string(&json_number(50.0)).unwrap_or_default();
        assert_eq!(json, "50");
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.499_999_999_999_999_94), 0.0);
        assert_eq!(pct(4.0, 7.0), 57.1);
        assert_eq!(pct(1.0, 0.0), 0.0);
    }

    #[test]
    fn locale_grouping_and_fraction() {
        assert_eq!(to_locale_en_us(0.0), "0");
        assert_eq!(to_locale_en_us(999.0), "999");
        assert_eq!(to_locale_en_us(1000.0), "1,000");
        assert_eq!(to_locale_en_us(54_310_983.0), "54,310,983");
        assert_eq!(to_locale_en_us(12_345.5), "12,345.5");
        assert_eq!(to_locale_en_us(1.23456), "1.235");
        assert_eq!(to_locale_en_us(-1234.0), "-1,234");
    }

    #[test]
    fn percentile_linear_interpolation() {
        let v = [40.0, 10.0, 30.0, 20.0];
        assert_eq!(percentile(&v, 0.1), 13.0);
        assert_eq!(percentile(&v, 0.5), 25.0);
        assert_eq!(percentile(&[7.0], 0.9), 7.0);
        assert_eq!(percentile(&[1.0, 2.0], 0.9), 1.9);
    }

    #[test]
    fn number_conversion_matches_js() {
        assert_eq!(to_number("5"), 5.0);
        assert_eq!(to_number(" 5 "), 5.0);
        assert_eq!(to_number(""), 0.0);
        assert_eq!(to_number("1e1"), 10.0);
        assert_eq!(to_number("0x10"), 16.0);
        assert!(to_number("abc").is_nan());
        assert!(to_number("inf").is_nan());
        assert!(to_number("1.5.2").is_nan());
        assert!(is_integer(to_number("3")));
        assert!(!is_integer(to_number("2.5")));
    }
}
