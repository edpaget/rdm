//! Per-model, per-token-class token accounting over Claude Code transcript
//! text.
//!
//! This module is pure: every entry point takes already-read `&str` or
//! in-memory values and returns numbers. Locating and reading transcript files
//! is the caller's job.
//!
//! # Token classes
//!
//! A [`TokenUsage`] counts five classes: uncached `input`, `output`,
//! `cache_write_5m`, `cache_write_1h` and `cache_read`. They are read from an
//! assistant line's `message.usage` object:
//!
//! - `input_tokens` becomes [`TokenUsage::input`].
//! - `output_tokens` becomes [`TokenUsage::output`].
//! - `cache_read_input_tokens` becomes [`TokenUsage::cache_read`].
//! - The cache write is split by TTL. When `usage.cache_creation` is an
//!   object, its `ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`
//!   fill the two write classes, and any remainder of
//!   `cache_creation_input_tokens` above their sum is added to
//!   [`TokenUsage::cache_write_5m`]. With no split, all of
//!   `cache_creation_input_tokens` goes to `cache_write_5m`, because 5 minutes
//!   is the default TTL. So `cache_write_5m + cache_write_1h ==
//!   cache_creation_input_tokens` whenever the aggregate is at least the split
//!   sum.
//!
//! A missing or non-integer field counts as 0.
//!
//! # Parsing and dedupe rules
//!
//! [`parse_transcript`] never fails. Blank lines, lines that are not JSON,
//! and lines that are not `type: "assistant"` with a non-empty string
//! `requestId` and an object `message.usage` are skipped. Unknown fields are
//! ignored.
//!
//! - **Dedupe by `requestId`, last write wins, first position kept.** A request
//!   streams as several assistant lines sharing one `requestId`, and only the
//!   last carries the final `output_tokens`. A later line replaces the
//!   request's usage and model; the request keeps its first position and its
//!   first line's `timestamp`.
//! - **All-zero usage is not a measurement.** A request whose final usage is
//!   zero in every class is removed and reported as a [`UsageWarning`] instead
//!   of being counted as a zero observation. The check runs on the final,
//!   deduped record.
//! - **The model is the raw `message.model` string**, unnormalized
//!   (`<synthetic>` included). A missing or non-string model is keyed
//!   [`UNKNOWN_MODEL`].
//!
//! # Time windows
//!
//! [`UsageLedger::from_requests_in_window`] counts only the requests whose
//! timestamp `ts` satisfies `start <= ts < end`. A request without a timestamp
//! falls in no window.

use std::collections::BTreeMap;
use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign};

use chrono::{DateTime, Utc};
use serde_json::Value;

/// The ledger key used for a request whose `message.model` is missing or not a
/// string.
pub const UNKNOWN_MODEL: &str = "unknown";

/// Token counts split into the five token classes.
///
/// Addition saturates at `u64::MAX` per class rather than wrapping or
/// panicking, so summing any number of transcripts is total.
///
/// # Examples
///
/// ```
/// use rdm_core::usage::TokenUsage;
///
/// let a = TokenUsage { input: 10, output: 5, cache_write_5m: 100, cache_write_1h: 20, cache_read: 1000 };
/// let b = TokenUsage { output: 7, ..TokenUsage::default() };
/// let sum: TokenUsage = [a, b].iter().sum();
/// assert_eq!(sum.output, 12);
/// assert_eq!(sum.cache_write(), 120);
/// assert_eq!(sum.total(), 10 + 12 + 120 + 1000);
/// assert_eq!(a + b, sum);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// Uncached input tokens (`input_tokens`).
    pub input: u64,
    /// Output tokens (`output_tokens`).
    pub output: u64,
    /// Cache-write tokens with the 5-minute TTL.
    pub cache_write_5m: u64,
    /// Cache-write tokens with the 1-hour TTL.
    pub cache_write_1h: u64,
    /// Cache-read tokens (`cache_read_input_tokens`).
    pub cache_read: u64,
}

impl TokenUsage {
    /// Both cache-write classes: `cache_write_5m + cache_write_1h`.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_core::usage::TokenUsage;
    ///
    /// let u = TokenUsage { cache_write_5m: 3, cache_write_1h: 4, ..TokenUsage::default() };
    /// assert_eq!(u.cache_write(), 7);
    /// ```
    #[must_use]
    pub fn cache_write(&self) -> u64 {
        self.cache_write_5m.saturating_add(self.cache_write_1h)
    }

    /// All five classes summed.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_core::usage::TokenUsage;
    ///
    /// let u = TokenUsage { input: 1, output: 2, cache_write_5m: 3, cache_write_1h: 4, cache_read: 5 };
    /// assert_eq!(u.total(), 15);
    /// ```
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_write())
            .saturating_add(self.cache_read)
    }

    /// `true` when every class is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_core::usage::TokenUsage;
    ///
    /// assert!(TokenUsage::default().is_zero());
    /// assert!(!TokenUsage { cache_read: 1, ..TokenUsage::default() }.is_zero());
    /// ```
    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

impl Add for TokenUsage {
    type Output = TokenUsage;

    fn add(self, rhs: TokenUsage) -> TokenUsage {
        TokenUsage {
            input: self.input.saturating_add(rhs.input),
            output: self.output.saturating_add(rhs.output),
            cache_write_5m: self.cache_write_5m.saturating_add(rhs.cache_write_5m),
            cache_write_1h: self.cache_write_1h.saturating_add(rhs.cache_write_1h),
            cache_read: self.cache_read.saturating_add(rhs.cache_read),
        }
    }
}

impl Add<&TokenUsage> for TokenUsage {
    type Output = TokenUsage;

    fn add(self, rhs: &TokenUsage) -> TokenUsage {
        self + *rhs
    }
}

impl AddAssign for TokenUsage {
    fn add_assign(&mut self, rhs: TokenUsage) {
        *self = *self + rhs;
    }
}

impl AddAssign<&TokenUsage> for TokenUsage {
    fn add_assign(&mut self, rhs: &TokenUsage) {
        *self = *self + *rhs;
    }
}

impl Sum for TokenUsage {
    fn sum<I: Iterator<Item = TokenUsage>>(iter: I) -> TokenUsage {
        iter.fold(TokenUsage::default(), Add::add)
    }
}

impl<'a> Sum<&'a TokenUsage> for TokenUsage {
    fn sum<I: Iterator<Item = &'a TokenUsage>>(iter: I) -> TokenUsage {
        iter.fold(TokenUsage::default(), Add::add)
    }
}

/// One deduped request from a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestUsage {
    /// The raw `requestId` string.
    pub request_id: String,
    /// The raw `message.model` of the request's last line, or
    /// [`UNKNOWN_MODEL`].
    pub model: String,
    /// The RFC 3339 `timestamp` of the request's first line; `None` when
    /// missing or unparsable.
    pub timestamp: Option<DateTime<Utc>>,
    /// The usage of the request's last line.
    pub usage: TokenUsage,
}

/// A report about a transcript that parsing tolerated. It is not an error:
/// parsing still succeeds, and the affected data is left out of the result.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UsageWarning {
    /// A request whose final usage was zero in every class. It is a
    /// no-measurement case, excluded rather than counted as a zero
    /// observation.
    AllZeroUsage {
        /// The request's `requestId`.
        request_id: String,
        /// The request's model.
        model: String,
    },
}

impl fmt::Display for UsageWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsageWarning::AllZeroUsage { request_id, model } => write!(
                f,
                "request {request_id} (model {model}) reported all-zero usage; excluded as unmeasured"
            ),
        }
    }
}

/// The result of [`parse_transcript`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedTranscript {
    /// Deduped requests with non-zero usage, in first-appearance order.
    pub requests: Vec<RequestUsage>,
    /// Everything the parse tolerated, in the order it was found.
    pub warnings: Vec<UsageWarning>,
}

fn int_field(obj: &Value, key: &str) -> u64 {
    obj.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn usage_from(usage: &Value) -> TokenUsage {
    let aggregate = int_field(usage, "cache_creation_input_tokens");
    let (cache_write_5m, cache_write_1h) = match usage.get("cache_creation") {
        Some(split @ Value::Object(_)) => {
            let m5 = int_field(split, "ephemeral_5m_input_tokens");
            let h1 = int_field(split, "ephemeral_1h_input_tokens");
            let remainder = aggregate.saturating_sub(m5.saturating_add(h1));
            (m5.saturating_add(remainder), h1)
        }
        _ => (aggregate, 0),
    };
    TokenUsage {
        input: int_field(usage, "input_tokens"),
        output: int_field(usage, "output_tokens"),
        cache_write_5m,
        cache_write_1h,
        cache_read: int_field(usage, "cache_read_input_tokens"),
    }
}

/// Parses transcript text (JSON lines) into deduped per-request usage.
///
/// See the [module docs](self) for the full rules. Parsing never fails:
/// malformed and non-usage lines are skipped, and all-zero requests are
/// excluded with a [`UsageWarning`].
///
/// # Examples
///
/// ```
/// use rdm_core::usage::parse_transcript;
///
/// let raw = concat!(
///     r#"{"type":"assistant","requestId":"r1","message":{"model":"claude-opus","usage":{"input_tokens":3,"output_tokens":1}}}"#, "\n",
///     r#"{"type":"user","message":{"content":"hi"}}"#, "\n",
///     r#"{"type":"assistant","requestId":"r1","message":{"model":"claude-opus","usage":{"input_tokens":3,"output_tokens":40}}}"#, "\n",
/// );
/// let parsed = parse_transcript(raw);
/// assert_eq!(parsed.requests.len(), 1);
/// assert_eq!(parsed.requests[0].usage.output, 40);
/// assert!(parsed.warnings.is_empty());
/// ```
#[must_use]
pub fn parse_transcript(raw: &str) -> ParsedTranscript {
    let mut requests: Vec<RequestUsage> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(request_id) = entry
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let message = entry.get("message");
        let Some(usage @ Value::Object(_)) = message.and_then(|m| m.get("usage")) else {
            continue;
        };
        let usage = usage_from(usage);
        let model = message
            .and_then(|m| m.get("model"))
            .and_then(Value::as_str)
            .unwrap_or(UNKNOWN_MODEL)
            .to_owned();
        match index.get(request_id) {
            Some(&i) => {
                let slot = &mut requests[i];
                slot.usage = usage;
                slot.model = model;
            }
            None => {
                let timestamp = entry
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&Utc));
                index.insert(request_id.to_owned(), requests.len());
                requests.push(RequestUsage {
                    request_id: request_id.to_owned(),
                    model,
                    timestamp,
                    usage,
                });
            }
        }
    }
    let mut parsed = ParsedTranscript::default();
    for request in requests {
        if request.usage.is_zero() {
            parsed.warnings.push(UsageWarning::AllZeroUsage {
                request_id: request.request_id,
                model: request.model,
            });
        } else {
            parsed.requests.push(request);
        }
    }
    parsed
}

/// One model's share of a [`UsageLedger`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelUsage {
    /// Deduped requests counted.
    pub requests: u64,
    /// Their usage summed per class.
    pub usage: TokenUsage,
}

/// Token usage aggregated per model, keyed by the raw `message.model` string.
///
/// Iteration order is the models' byte-wise string order, so output built from
/// a ledger is deterministic.
///
/// # Examples
///
/// ```
/// use rdm_core::usage::{parse_transcript, UsageLedger};
///
/// let raw = concat!(
///     r#"{"type":"assistant","requestId":"a","message":{"model":"opus","usage":{"output_tokens":5}}}"#, "\n",
///     r#"{"type":"assistant","requestId":"b","message":{"model":"haiku","usage":{"output_tokens":2}}}"#, "\n",
///     r#"{"type":"assistant","requestId":"c","message":{"model":"opus","usage":{"output_tokens":7}}}"#, "\n",
/// );
/// let ledger = UsageLedger::from_requests(&parse_transcript(raw).requests);
/// let opus = ledger.get("opus").copied().unwrap_or_default();
/// assert_eq!((opus.requests, opus.usage.output), (2, 12));
/// assert_eq!(ledger.total().usage.output, 14);
/// assert_eq!(ledger.iter().map(|(m, _)| m).collect::<Vec<_>>(), ["haiku", "opus"]);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageLedger {
    models: BTreeMap<String, ModelUsage>,
}

impl UsageLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A ledger counting every request in `requests`.
    #[must_use]
    pub fn from_requests(requests: &[RequestUsage]) -> Self {
        let mut ledger = Self::new();
        for r in requests {
            ledger.record(r);
        }
        ledger
    }

    /// A ledger counting only the requests whose timestamp lies in the
    /// half-open window `[start, end)`. Requests without a timestamp are in no
    /// window and are never counted.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{DateTime, Utc};
    /// use rdm_core::usage::{parse_transcript, UsageLedger};
    ///
    /// let raw = concat!(
    ///     r#"{"type":"assistant","requestId":"a","timestamp":"2026-01-01T00:00:00Z","message":{"model":"m","usage":{"output_tokens":1}}}"#, "\n",
    ///     r#"{"type":"assistant","requestId":"b","timestamp":"2026-01-01T01:00:00Z","message":{"model":"m","usage":{"output_tokens":10}}}"#, "\n",
    /// );
    /// let start: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap_or_default();
    /// let end: DateTime<Utc> = "2026-01-01T01:00:00Z".parse().unwrap_or_default();
    /// let ledger = UsageLedger::from_requests_in_window(&parse_transcript(raw).requests, start, end);
    /// assert_eq!(ledger.total().usage.output, 1, "the request at `end` is outside");
    /// ```
    #[must_use]
    pub fn from_requests_in_window(
        requests: &[RequestUsage],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Self {
        let mut ledger = Self::new();
        for r in requests {
            if r.timestamp.is_some_and(|ts| start <= ts && ts < end) {
                ledger.record(r);
            }
        }
        ledger
    }

    /// Counts one request against its model.
    pub fn record(&mut self, request: &RequestUsage) {
        let entry = self.models.entry(request.model.clone()).or_default();
        entry.requests = entry.requests.saturating_add(1);
        entry.usage += request.usage;
    }

    /// The usage recorded for `model`, matched exactly against the raw model
    /// string.
    #[must_use]
    pub fn get(&self, model: &str) -> Option<&ModelUsage> {
        self.models.get(model)
    }

    /// Every model and its usage, in model-string order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ModelUsage)> {
        self.models.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// `true` when no request has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// All models summed: total requests and per-class usage.
    #[must_use]
    pub fn total(&self) -> ModelUsage {
        self.models
            .values()
            .fold(ModelUsage::default(), |acc, m| ModelUsage {
                requests: acc.requests.saturating_add(m.requests),
                usage: acc.usage + m.usage,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        s.parse()
            .unwrap_or_else(|e| panic!("bad test timestamp {s}: {e}"))
    }

    fn lines(ls: &[&str]) -> String {
        ls.join("\n")
    }

    #[test]
    fn parses_all_five_classes_with_cache_split() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"r1","message":{"model":"claude-opus-4","usage":{"input_tokens":11,"output_tokens":22,"cache_creation_input_tokens":300,"cache_read_input_tokens":4000,"cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":200}}}}"#,
            r#"{"type":"assistant","requestId":"r2","message":{"model":"claude-haiku","usage":{"input_tokens":1,"output_tokens":2,"cache_creation_input_tokens":3,"cache_read_input_tokens":4}}}"#,
        ]);
        let ledger = UsageLedger::from_requests(&parse_transcript(&raw).requests);
        assert_eq!(
            ledger.get("claude-opus-4").map(|m| m.usage),
            Some(TokenUsage {
                input: 11,
                output: 22,
                cache_write_5m: 100,
                cache_write_1h: 200,
                cache_read: 4000,
            })
        );
        assert_eq!(
            ledger.get("claude-haiku").map(|m| m.usage),
            Some(TokenUsage {
                input: 1,
                output: 2,
                cache_write_5m: 3,
                cache_write_1h: 0,
                cache_read: 4,
            })
        );
    }

    #[test]
    fn dedupe_last_write_wins_keeps_first_position() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"req-A","message":{"model":"m","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            r#"{"type":"user","message":{"content":"x"}}"#,
            r#"{"type":"assistant","requestId":"req-B","message":{"model":"m","usage":{"input_tokens":80,"output_tokens":120}}}"#,
            r#"{"type":"assistant","requestId":"req-A","message":{"model":"m","usage":{"input_tokens":100,"output_tokens":50}}}"#,
        ]);
        let parsed = parse_transcript(&raw);
        assert_eq!(parsed.requests.len(), 2, "two requestIds, not three lines");
        assert_eq!(
            parsed.requests[0].request_id, "req-A",
            "first position kept"
        );
        assert_eq!(parsed.requests[0].usage.output, 50, "later line wins");
        assert_eq!(parsed.requests[1].usage.output, 120);
        let ledger = UsageLedger::from_requests(&parsed.requests);
        assert_eq!(
            ledger.get("m").map(|m| (m.requests, m.usage.output)),
            Some((2, 170))
        );
    }

    #[test]
    fn tolerates_non_usage_lines_and_synthetic_model() {
        let raw = lines(&[
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"system","subtype":"init","content":"x"}"#,
            r#"{"type":"attachment","attachment":{"kind":"file"}}"#,
            "not json at all",
            "",
            r#"{"type":"assistant","requestId":"r1","extra":{"nested":true},"message":{"model":"claude-opus","id":"msg","usage":{"input_tokens":5,"output_tokens":6,"service_tier":"standard"}}}"#,
            r#"{"type":"assistant","requestId":"syn-1","message":{"model":"<synthetic>","usage":{"output_tokens":3}}}"#,
            r#"{"type":"assistant","requestId":"syn-0","message":{"model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#,
            r#"{"type":"assistant","message":{"model":"m","usage":{"output_tokens":9}}}"#,
            r#"{"type":"assistant","requestId":7,"message":{"model":"m","usage":{"output_tokens":9}}}"#,
        ]);
        let parsed = parse_transcript(&raw);
        let ids: Vec<&str> = parsed
            .requests
            .iter()
            .map(|r| r.request_id.as_str())
            .collect();
        assert_eq!(ids, ["r1", "syn-1"]);
        let ledger = UsageLedger::from_requests(&parsed.requests);
        assert_eq!(
            ledger
                .get("<synthetic>")
                .map(|m| (m.requests, m.usage.output)),
            Some((1, 3))
        );
        assert_eq!(ledger.get("claude-opus").map(|m| m.usage.input), Some(5));
        assert_eq!(
            parsed.warnings,
            [UsageWarning::AllZeroUsage {
                request_id: "syn-0".to_owned(),
                model: "<synthetic>".to_owned(),
            }]
        );
    }

    #[test]
    fn all_zero_usage_is_excluded_and_warned() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"real","message":{"model":"m","usage":{"output_tokens":4}}}"#,
            r#"{"type":"assistant","requestId":"zero","message":{"model":"m","usage":{"input_tokens":0,"output_tokens":0}}}"#,
            r#"{"type":"assistant","requestId":"empty","message":{"model":"m","usage":{}}}"#,
        ]);
        let parsed = parse_transcript(&raw);
        assert_eq!(parsed.requests.len(), 1);
        assert_eq!(parsed.requests[0].request_id, "real");
        let ledger = UsageLedger::from_requests(&parsed.requests);
        assert_eq!(ledger.get("m").map(|m| m.requests), Some(1));
        assert_eq!(parsed.warnings.len(), 2);
        assert!(parsed.warnings[0].to_string().contains("zero"));
        assert!(parsed.warnings[1].to_string().contains("empty"));
    }

    #[test]
    fn all_zero_is_judged_on_the_final_line() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"r","message":{"model":"m","usage":{"output_tokens":0}}}"#,
            r#"{"type":"assistant","requestId":"r","message":{"model":"m","usage":{"output_tokens":8}}}"#,
            r#"{"type":"assistant","requestId":"s","message":{"model":"m","usage":{"output_tokens":8}}}"#,
            r#"{"type":"assistant","requestId":"s","message":{"model":"m","usage":{"output_tokens":0}}}"#,
        ]);
        let parsed = parse_transcript(&raw);
        assert_eq!(parsed.requests.len(), 1);
        assert_eq!(parsed.requests[0].request_id, "r");
        assert_eq!(parsed.warnings.len(), 1);
    }

    #[test]
    fn token_usage_add_and_sum_per_class() {
        let a = TokenUsage {
            input: 1,
            output: 2,
            cache_write_5m: 3,
            cache_write_1h: 4,
            cache_read: 5,
        };
        let b = TokenUsage {
            input: 10,
            output: 20,
            cache_write_5m: 30,
            cache_write_1h: 40,
            cache_read: 50,
        };
        let expected = TokenUsage {
            input: 11,
            output: 22,
            cache_write_5m: 33,
            cache_write_1h: 44,
            cache_read: 55,
        };
        assert_eq!(a + b, expected);
        #[allow(clippy::op_ref)] // exercising the by-reference impl
        {
            assert_eq!(a + &b, expected);
        }
        let mut c = a;
        c += b;
        assert_eq!(c, expected);
        let mut d = a;
        d += &b;
        assert_eq!(d, expected);
        assert_eq!([a, b].into_iter().sum::<TokenUsage>(), expected);
        assert_eq!([a, b].iter().sum::<TokenUsage>(), expected);
        assert_eq!(expected.cache_write(), 77);
        assert_eq!(expected.total(), 165);
        let max = TokenUsage {
            input: u64::MAX,
            ..TokenUsage::default()
        };
        assert_eq!((max + a).input, u64::MAX, "addition saturates");
    }

    #[test]
    fn unsplit_cache_write_goes_to_5m_and_remainder_rule() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"unsplit","message":{"model":"a","usage":{"cache_creation_input_tokens":500}}}"#,
            r#"{"type":"assistant","requestId":"remainder","message":{"model":"b","usage":{"cache_creation_input_tokens":500,"cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":300}}}}"#,
            r#"{"type":"assistant","requestId":"only1h","message":{"model":"c","usage":{"cache_creation_input_tokens":60,"cache_creation":{"ephemeral_1h_input_tokens":60}}}}"#,
        ]);
        let parsed = parse_transcript(&raw);
        let split: Vec<(u64, u64)> = parsed
            .requests
            .iter()
            .map(|r| (r.usage.cache_write_5m, r.usage.cache_write_1h))
            .collect();
        assert_eq!(split, [(500, 0), (200, 300), (0, 60)]);
        let writes: Vec<u64> = parsed
            .requests
            .iter()
            .map(|r| r.usage.cache_write())
            .collect();
        assert_eq!(writes, [500, 500, 60], "5m + 1h equals the aggregate");
    }

    #[test]
    fn ledger_aggregates_multiple_models_with_request_counts() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"1","message":{"model":"opus","usage":{"input_tokens":1,"output_tokens":10}}}"#,
            r#"{"type":"assistant","requestId":"2","message":{"model":"sonnet","usage":{"input_tokens":2,"cache_read_input_tokens":7}}}"#,
            r#"{"type":"assistant","requestId":"3","message":{"model":"opus","usage":{"input_tokens":3,"output_tokens":30}}}"#,
            r#"{"type":"assistant","requestId":"4","message":{"usage":{"output_tokens":1}}}"#,
        ]);
        let ledger = UsageLedger::from_requests(&parse_transcript(&raw).requests);
        let opus = ledger.get("opus").copied().unwrap_or_default();
        assert_eq!(opus.requests, 2);
        assert_eq!((opus.usage.input, opus.usage.output), (4, 40));
        let sonnet = ledger.get("sonnet").copied().unwrap_or_default();
        assert_eq!(sonnet.requests, 1);
        assert_eq!(sonnet.usage.cache_read, 7);
        assert_eq!(ledger.get(UNKNOWN_MODEL).map(|m| m.requests), Some(1));
        let models: Vec<&str> = ledger.iter().map(|(m, _)| m).collect();
        assert_eq!(models, ["opus", "sonnet", UNKNOWN_MODEL]);
    }

    #[test]
    fn ledger_total_sums_models() {
        let mut ledger = UsageLedger::new();
        assert!(ledger.is_empty());
        assert_eq!(ledger.total(), ModelUsage::default());
        for (id, model, out) in [("a", "x", 1), ("b", "y", 2), ("c", "y", 4)] {
            ledger.record(&RequestUsage {
                request_id: id.to_owned(),
                model: model.to_owned(),
                timestamp: None,
                usage: TokenUsage {
                    output: out,
                    cache_read: 100,
                    ..TokenUsage::default()
                },
            });
        }
        let total = ledger.total();
        assert_eq!(total.requests, 3);
        assert_eq!(total.usage.output, 7);
        assert_eq!(total.usage.cache_read, 300);
    }

    #[test]
    fn ledger_key_is_raw_model_string() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"1","message":{"model":"claude-opus-4-1","usage":{"output_tokens":1}}}"#,
            r#"{"type":"assistant","requestId":"2","message":{"model":"claude-opus-4-1-20250805","usage":{"output_tokens":2}}}"#,
            r#"{"type":"assistant","requestId":"3","message":{"model":"Claude-Opus-4-1","usage":{"output_tokens":4}}}"#,
        ]);
        let ledger = UsageLedger::from_requests(&parse_transcript(&raw).requests);
        let keys: Vec<&str> = ledger.iter().map(|(m, _)| m).collect();
        assert_eq!(
            keys,
            [
                "Claude-Opus-4-1",
                "claude-opus-4-1",
                "claude-opus-4-1-20250805"
            ]
        );
        assert_eq!(
            ledger
                .get("claude-opus-4-1-20250805")
                .map(|m| m.usage.output),
            Some(2)
        );
    }

    #[test]
    fn deduped_request_keeps_first_timestamp() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"r","timestamp":"2026-03-01T10:00:00.123Z","message":{"model":"m","usage":{"output_tokens":1}}}"#,
            r#"{"type":"assistant","requestId":"r","timestamp":"2026-03-01T10:00:05Z","message":{"model":"m","usage":{"output_tokens":9}}}"#,
            r#"{"type":"assistant","requestId":"bad","timestamp":"yesterday","message":{"model":"m","usage":{"output_tokens":1}}}"#,
            r#"{"type":"assistant","requestId":"none","message":{"model":"m","usage":{"output_tokens":1}}}"#,
        ]);
        let parsed = parse_transcript(&raw);
        assert_eq!(
            parsed.requests[0].timestamp,
            Some(ts("2026-03-01T10:00:00.123Z"))
        );
        assert_eq!(parsed.requests[0].usage.output, 9);
        assert_eq!(parsed.requests[1].timestamp, None);
        assert_eq!(parsed.requests[2].timestamp, None);
    }

    #[test]
    fn window_ledger_is_half_open() {
        let raw = lines(&[
            r#"{"type":"assistant","requestId":"before","timestamp":"2026-01-01T09:59:59Z","message":{"model":"m","usage":{"output_tokens":1}}}"#,
            r#"{"type":"assistant","requestId":"at-start","timestamp":"2026-01-01T10:00:00Z","message":{"model":"m","usage":{"output_tokens":10}}}"#,
            r#"{"type":"assistant","requestId":"offset-before","timestamp":"2026-01-01T10:30:00+02:00","message":{"model":"m","usage":{"output_tokens":100}}}"#,
            r#"{"type":"assistant","requestId":"inside","timestamp":"2026-01-01T10:30:00Z","message":{"model":"m","usage":{"output_tokens":1000}}}"#,
            r#"{"type":"assistant","requestId":"at-end","timestamp":"2026-01-01T11:00:00Z","message":{"model":"m","usage":{"output_tokens":10000}}}"#,
            r#"{"type":"assistant","requestId":"after","timestamp":"2026-01-01T12:00:00Z","message":{"model":"m","usage":{"output_tokens":100000}}}"#,
            r#"{"type":"assistant","requestId":"untimed","message":{"model":"m","usage":{"output_tokens":1000000}}}"#,
        ]);
        let parsed = parse_transcript(&raw);
        let ledger = UsageLedger::from_requests_in_window(
            &parsed.requests,
            ts("2026-01-01T10:00:00Z"),
            ts("2026-01-01T11:00:00Z"),
        );
        // "offset-before" is 08:30Z, before the window; start is in, end is out.
        let total = ledger.total();
        assert_eq!(total.requests, 2);
        assert_eq!(total.usage.output, 10 + 1000);
        assert_eq!(
            UsageLedger::from_requests(&parsed.requests)
                .total()
                .requests,
            7
        );
    }
}
