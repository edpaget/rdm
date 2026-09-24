//! Timestamp-based id generation and collision probing, shared by the
//! id-keyed document kinds (reviews and runs).
//!
//! An id is `YYYY-MM-DD-HHMM-xxxx`: the mint instant to the minute, plus a
//! 4-hex-digit suffix hashed from the instant's nanoseconds, a process-local
//! counter, and the process id. The caller probes candidates against its own
//! store paths with [`next_available_id`] and names its own exhaustion error.

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Utc};

use crate::error::{Error, Result};

/// Maximum attempts to find a non-colliding id before giving up.
pub(crate) const MAX_ID_ATTEMPTS: u32 = 20;

/// Process-local counter mixed into id suffixes so two ids generated at the
/// same instant within one process still differ.
static ID_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generates a timestamp-based id: `YYYY-MM-DD-HHMM-xxxx` where `xxxx` is a
/// 4-hex-digit suffix derived from the timestamp, a process-local counter,
/// and the process id.
pub(crate) fn generate_timestamp_id(now: DateTime<Utc>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    now.timestamp_nanos_opt()
        .unwrap_or_default()
        .hash(&mut hasher);
    ID_COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    let suffix = (hasher.finish() & 0xffff) as u16;
    format!("{}-{suffix:04x}", now.format("%Y-%m-%d-%H%M"))
}

/// Returns the first candidate id for which `exists` is `false`, calling
/// `candidate` up to [`MAX_ID_ATTEMPTS`] times.
///
/// # Errors
///
/// Returns `on_exhausted` when every attempt collided.
pub(crate) fn next_available_id(
    exists: impl Fn(&str) -> bool,
    mut candidate: impl FnMut() -> String,
    on_exhausted: Error,
) -> Result<String> {
    for _ in 0..MAX_ID_ATTEMPTS {
        let id = candidate();
        if !exists(&id) {
            return Ok(id);
        }
    }
    Err(on_exhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn generate_timestamp_id_format() {
        let now = Utc.with_ymd_and_hms(2026, 9, 24, 15, 30, 5).unwrap();
        let id = generate_timestamp_id(now);
        assert!(id.starts_with("2026-09-24-1530-"), "got: {id}");
        let suffix = id.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 4);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn next_available_id_reports_the_callers_exhaustion_error() {
        let mut calls = 0;
        let result = next_available_id(
            |_| true,
            || {
                calls += 1;
                "stuck".to_string()
            },
            Error::RunIdExhausted,
        );
        assert!(matches!(result, Err(Error::RunIdExhausted)));
        assert_eq!(calls, MAX_ID_ATTEMPTS);
    }
}
