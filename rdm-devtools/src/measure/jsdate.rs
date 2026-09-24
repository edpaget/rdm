//! `Date.parse` for the date strings the measurement tools accept.
//!
//! The JS tools compared `--since`/`--until` (and a sidecar's string
//! `startTime`/`timestamp`) with `Date.parse`. This implements the ECMAScript
//! date-time string format V8 parses, which is what every recorded window and
//! every sidecar uses:
//!
//! - `YYYY`, `YYYY-MM`, `YYYY-MM-DD` (and `±YYYYYY` extended years) — UTC;
//! - any of those followed by `THH:mm`, `THH:mm:ss` or `THH:mm:ss.fff…`, then
//!   an optional `Z` or `±HH:mm` offset. A date-time with no offset is **local
//!   time**, exactly as in JavaScript. V8 also accepts a space instead of `T`,
//!   so this does too.
//!
//! V8's legacy fallback formats (`"Jul 24 2026"`, `"2026/07/24"`, RFC 2822)
//! are not accepted: such a value is rejected as unparseable rather than
//! guessed at. Day-of-month is range-checked to 1–31 and rolls over like V8
//! (`2026-02-30` is 2 March).

use chrono::{Local, NaiveDate, TimeZone};

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    // Howard Hinnant's algorithm; d may exceed the month length (rolls over).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn take_digits(s: &[u8], at: &mut usize, n: usize) -> Option<i64> {
    let end = at.checked_add(n)?;
    let slice = s.get(*at..end)?;
    if !slice.iter().all(u8::is_ascii_digit) {
        return None;
    }
    *at = end;
    std::str::from_utf8(slice).ok()?.parse().ok()
}

/// Milliseconds since the epoch, or `None` where `Date.parse` yields `NaN`.
pub fn parse(input: &str) -> Option<f64> {
    let s = input.as_bytes();
    let mut i = 0;
    let year = match s.first() {
        Some(b'+') | Some(b'-') => {
            let negative = s[0] == b'-';
            i = 1;
            let y = take_digits(s, &mut i, 6)?;
            if negative && y == 0 {
                return None;
            }
            if negative { -y } else { y }
        }
        _ => take_digits(s, &mut i, 4)?,
    };
    let mut month = 1;
    let mut day = 1;
    if s.get(i) == Some(&b'-') {
        i += 1;
        month = take_digits(s, &mut i, 2)?;
        if s.get(i) == Some(&b'-') {
            i += 1;
            day = take_digits(s, &mut i, 2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date_ms = days_from_civil(year, month, day) * 86_400_000;
    if i == s.len() {
        #[allow(clippy::cast_precision_loss)]
        return Some(date_ms as f64);
    }
    if s.get(i) != Some(&b'T') && s.get(i) != Some(&b't') && s.get(i) != Some(&b' ') {
        return None;
    }
    i += 1;
    let hour = take_digits(s, &mut i, 2)?;
    if s.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    let minute = take_digits(s, &mut i, 2)?;
    let mut second = 0;
    let mut millis = 0;
    if s.get(i) == Some(&b':') {
        i += 1;
        second = take_digits(s, &mut i, 2)?;
        if s.get(i) == Some(&b'.') {
            i += 1;
            let start = i;
            while s.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == start {
                return None;
            }
            let frac = std::str::from_utf8(&s[start..i]).ok()?;
            let three: String = frac.chars().chain("000".chars()).take(3).collect();
            millis = three.parse().ok()?;
        }
    }
    let valid_time = (hour < 24 && minute < 60 && second < 60)
        || (hour == 24 && minute == 0 && second == 0 && millis == 0);
    if !valid_time {
        return None;
    }
    let time_ms = ((hour * 60 + minute) * 60 + second) * 1000 + millis;
    let offset_ms = match s.get(i) {
        None => {
            // No offset on a date-time: local time.
            let date = NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, 1, 1)?;
            let base = date.and_hms_opt(0, 0, 0)?;
            let naive = base
                + chrono::Duration::milliseconds(
                    date_ms - days_from_civil(year, 1, 1) * 86_400_000 + time_ms,
                );
            let local = Local.from_local_datetime(&naive).earliest()?;
            #[allow(clippy::cast_precision_loss)]
            return Some(local.timestamp_millis() as f64);
        }
        Some(b'Z') | Some(b'z') => {
            i += 1;
            0
        }
        Some(b'+') | Some(b'-') => {
            let negative = s[i] == b'-';
            i += 1;
            let oh = take_digits(s, &mut i, 2)?;
            // V8 accepts both `+02:00` and `+0200`.
            if s.get(i) == Some(&b':') {
                i += 1;
            }
            let om = take_digits(s, &mut i, 2)?;
            if oh > 23 || om > 59 {
                return None;
            }
            let off = (oh * 60 + om) * 60_000;
            if negative { -off } else { off }
        }
        Some(_) => return None,
    };
    if i != s.len() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some((date_ms + time_ms - offset_ms) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_forms_parse_like_v8() {
        assert_eq!(parse("2026-07-24T00:00:00Z"), Some(1_784_851_200_000.0));
        assert_eq!(parse("2026-07-24"), Some(1_784_851_200_000.0));
        assert_eq!(parse("2026-07"), Some(1_782_864_000_000.0));
        assert_eq!(parse("2026"), Some(1_767_225_600_000.0));
        assert_eq!(parse("2026-07-25T09:00:00.000Z"), Some(1_784_970_000_000.0));
        assert_eq!(
            parse("2026-07-25T11:00:00+02:00"),
            Some(1_784_970_000_000.0)
        );
        assert_eq!(parse("2026-02-30"), parse("2026-03-02"));
        assert_eq!(parse("not-a-date"), None);
        assert_eq!(parse("2026-13-01"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("2026-07-24T25:00Z"), None);
        assert!(parse("2026-07-24T00:00").is_some());
        assert_eq!(parse("2026-07-24t00:00:00z"), Some(1_784_851_200_000.0));
        assert_eq!(parse("+002026-07-24"), Some(1_784_851_200_000.0));
        assert_eq!(
            parse("2026-07-24T00:00:00.123456Z"),
            Some(1_784_851_200_123.0)
        );
        assert_eq!(parse("2026-07-24T24:00:00Z"), Some(1_784_937_600_000.0));
        assert_eq!(parse("2026-07-24T00:00:00+0200"), Some(1_784_844_000_000.0));
    }
}
