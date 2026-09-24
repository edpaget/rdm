//! Opt-in paid dispatch: one `claude -p --model <tier> --output-format json`
//! per trial with the prompt on stdin, through a bounded worker pool whose
//! concurrency never changes the recorded order, plus the pure parsers for
//! its response bodies.
//!
//! A per-trial failure (non-zero exit, non-JSON body, no boolean `refuted`)
//! records `verdict: null` — ungraded, never coerced to `refuted: false` —
//! and never aborts the run. A missing `claude` binary is a setup error and
//! is fatal, naming the fix.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde_json::Value;

use crate::measure::jsjson::{JsValue, obj};
use crate::measure::refuter_severity::extract::match_brace;
use crate::measure::sidecar::{read_text, transcript_entries, truthy};

/// The project-slug directory name Claude Code uses for a working directory
/// (every non-alphanumeric UTF-16 unit becomes `-`).
pub fn project_slug_for(cwd: &str) -> String {
    let units: Vec<u16> = cwd
        .encode_utf16()
        .map(|u| {
            if u < 0x80 && (u as u8).is_ascii_alphanumeric() {
                u
            } else {
                u16::from(b'-')
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Counts `tool_use` blocks in a finished session's transcript
/// (`<projects_root>/<slug(cwd)>/<session>.jsonl`); `None` when unreadable.
pub fn count_session_tool_uses(session_id: &str, cwd: &str, projects_root: &Path) -> Option<usize> {
    let file = projects_root
        .join(project_slug_for(cwd))
        .join(format!("{session_id}.jsonl"));
    let raw = read_text(&file).ok()?;
    let mut n = 0;
    for entry in transcript_entries(&raw) {
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(content) = entry
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        {
            n += content
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .count();
        }
    }
    Some(n)
}

fn try_parse_json(text: Option<&Value>) -> Option<Value> {
    serde_json::from_str(text?.as_str()?).ok()
}

/// The first balanced `{...}` span inside a prose or fenced answer, parsed.
pub fn try_parse_embedded_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = match_brace(text, start)?;
    serde_json::from_str(&text[start..=end]).ok()
}

fn parse_result_field(body: &Value) -> Option<Value> {
    let result = body.get("result");
    try_parse_json(result)
        .filter(|v| truthy(Some(v)))
        .or_else(|| {
            result
                .and_then(Value::as_str)
                .and_then(try_parse_embedded_json)
        })
}

fn usage_of(body: &Value) -> JsValue {
    let u = body.get("usage");
    let n = |k: &str| -> JsValue {
        let v = u.and_then(|u| u.get(k));
        JsValue::Number(if truthy(v) {
            crate::measure::refuter_severity::doc::num(v)
        } else {
            0.0
        })
    };
    obj([
        ("output", n("output_tokens")),
        ("uncachedInput", n("input_tokens")),
        ("cacheWrite", n("cache_creation_input_tokens")),
        ("cacheRead", n("cache_read_input_tokens")),
    ])
}

fn verdict_from(input: &Value) -> Option<JsValue> {
    let refuted = input.get("refuted").and_then(Value::as_bool)?;
    Some(obj([
        ("refuted", JsValue::Bool(refuted)),
        (
            "confidence",
            input.get("confidence").and_then(Value::as_f64).into(),
        ),
        (
            "rationale",
            input
                .get("rationale")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .into(),
        ),
    ]))
}

fn tool_blocks(body: &Value) -> Vec<&Value> {
    body.get("messages")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .filter_map(|m| {
                    m.get("message")
                        .and_then(|x| x.get("content"))
                        .and_then(Value::as_array)
                })
                .flatten()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .collect()
        })
        .unwrap_or_default()
}

/// One per-finding dispatch outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// `{refuted, confidence, rationale}`, or `None` (ungraded).
    pub verdict: Option<JsValue>,
    /// The four token classes.
    pub usage: JsValue,
    /// Tool calls.
    pub tool_calls: f64,
    /// Why it is ungraded.
    pub error: Option<String>,
}

/// Parses a `claude -p --output-format json` body: the LAST `StructuredOutput`
/// with a boolean `refuted` wins; failing that, the `result` string (bare,
/// fenced or in prose); a non-boolean `refuted` stays ungraded.
pub fn parse_claude_result(body: &Value) -> Outcome {
    let blocks = tool_blocks(body);
    let mut verdict = None;
    for b in &blocks {
        if b.get("name").and_then(Value::as_str) == Some("StructuredOutput")
            && let Some(v) = b
                .get("input")
                .filter(|i| truthy(Some(i)))
                .and_then(verdict_from)
        {
            verdict = Some(v);
        }
    }
    if verdict.is_none() {
        verdict = parse_result_field(body).as_ref().and_then(verdict_from);
    }
    #[allow(clippy::cast_precision_loss)]
    let mut tool_calls = blocks.len() as f64;
    if let Some(n) = body.get("num_tool_uses").and_then(Value::as_f64)
        && tool_calls == 0.0
    {
        tool_calls = n;
    }
    Outcome {
        error: verdict
            .is_none()
            .then(|| "no boolean `refuted` in the response".to_owned()),
        verdict,
        usage: usage_of(body),
        tool_calls,
    }
}

/// One batched dispatch outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchOutcome {
    /// Per-id verdicts (`None`: no `verdicts` array at all — a crash).
    pub verdicts: Option<Vec<JsValue>>,
    /// Verdict ids the dispatch did not contain (dropped).
    pub unknown_verdict_ids: Vec<String>,
    /// The four token classes.
    pub usage: JsValue,
    /// Tool calls.
    pub tool_calls: f64,
    /// Why it failed.
    pub error: Option<String>,
}

/// Parses a batched body: unknown ids are dropped and recorded, a
/// non-boolean `refuted` stays `null`, and a body with no `verdicts` array is
/// a crash (every id stays ungraded).
pub fn parse_claude_batch_result(body: &Value, expected_ids: &[String]) -> BatchOutcome {
    let blocks = tool_blocks(body);
    let mut raw: Option<Vec<Value>> = None;
    for b in &blocks {
        if b.get("name").and_then(Value::as_str) == Some("StructuredOutput")
            && let Some(v) = b
                .get("input")
                .and_then(|i| i.get("verdicts"))
                .and_then(Value::as_array)
        {
            raw = Some(v.clone());
        }
    }
    if raw.is_none() {
        raw = parse_result_field(body)
            .and_then(|p| p.get("verdicts").and_then(Value::as_array).cloned());
    }
    #[allow(clippy::cast_precision_loss)]
    let mut tool_calls = blocks.len() as f64;
    if let Some(n) = body.get("num_tool_uses").and_then(Value::as_f64)
        && tool_calls == 0.0
    {
        tool_calls = n;
    }
    let usage = usage_of(body);
    let Some(raw) = raw else {
        return BatchOutcome {
            verdicts: None,
            unknown_verdict_ids: Vec::new(),
            usage,
            tool_calls,
            error: Some("no `verdicts` array in the response".to_owned()),
        };
    };
    let mut verdicts = Vec::new();
    let mut unknown = Vec::new();
    for v in raw.iter().filter(|v| v.is_object()) {
        let id = crate::measure::sidecar::js_display(v.get("id"));
        if !expected_ids.is_empty() && !expected_ids.contains(&id) {
            unknown.push(id);
            continue;
        }
        verdicts.push(obj([
            ("id", JsValue::String(id)),
            ("refuted", v.get("refuted").and_then(Value::as_bool).into()),
            (
                "confidence",
                v.get("confidence").and_then(Value::as_f64).into(),
            ),
            (
                "rationale",
                v.get("rationale")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .into(),
            ),
        ]));
    }
    BatchOutcome {
        verdicts: Some(verdicts),
        unknown_verdict_ids: unknown,
        usage,
        tool_calls,
        error: None,
    }
}

/// How to reach `claude`.
#[derive(Debug, Clone)]
pub struct Dispatcher {
    /// The executable (default `claude` on `PATH`).
    pub claude_bin: PathBuf,
    /// The working directory the agent runs in (the checkout).
    pub cwd: PathBuf,
    /// Where finished session transcripts live, for the tool-call recount
    /// (`$HOME/.claude/projects`).
    pub projects_root: PathBuf,
}

struct Raw {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Dispatcher {
    fn run(&self, tier: &str, prompt: &str) -> Result<Result<Raw, String>, String> {
        let spawned = Command::new(&self.claude_bin)
            .args(["-p", "--model", tier, "--output-format", "json"])
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "the `claude` binary was not found ({}). The refuter-agreement runner dispatches real agents through `claude -p`; install/authenticate the CLI or pass --claude-bin, or use --dry-run / --score-only.",
                    self.claude_bin.display()
                ));
            }
            Err(e) => return Ok(Err(format!("claude failed to start: {e}"))),
        };
        if let Some(mut stdin) = child.stdin.take() {
            let input = prompt.to_owned();
            // A child that exits without reading must not wedge the writer.
            std::thread::spawn(move || {
                let _ = stdin.write_all(input.as_bytes());
            });
        }
        match child.wait_with_output() {
            Ok(out) => Ok(Ok(Raw {
                status: out.status.code(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })),
            Err(e) => Ok(Err(format!("claude failed to start: {e}"))),
        }
    }

    fn recount(&self, body: &Value, tool_calls: &mut f64) {
        if *tool_calls == 0.0
            && let Some(session) = body.get("session_id").and_then(Value::as_str)
            && let Some(n) =
                count_session_tool_uses(session, &self.cwd.to_string_lossy(), &self.projects_root)
        {
            #[allow(clippy::cast_precision_loss)]
            {
                *tool_calls = n as f64;
            }
        }
    }

    /// Dispatches one per-finding trial.
    ///
    /// # Errors
    ///
    /// Only when the binary cannot be found (fatal); every other failure is an
    /// ungraded outcome.
    pub fn dispatch(&self, tier: &str, prompt: &str) -> Result<Outcome, String> {
        let failed = |error: String| Outcome {
            verdict: None,
            usage: JsValue::empty_object(),
            tool_calls: 0.0,
            error: Some(error),
        };
        let raw = match self.run(tier, prompt)? {
            Ok(raw) => raw,
            Err(e) => return Ok(failed(e)),
        };
        if raw.status != Some(0) {
            let tail: String = raw.stderr.trim().chars().take(400).collect();
            return Ok(failed(format!(
                "claude exited {}: {tail}",
                raw.status
                    .map_or_else(|| "null".to_owned(), |c| c.to_string())
            )));
        }
        let body: Value = match serde_json::from_str(&raw.stdout) {
            Ok(b) => b,
            Err(e) => return Ok(failed(format!("claude returned a non-JSON body: {e}"))),
        };
        let mut out = parse_claude_result(&body);
        self.recount(&body, &mut out.tool_calls);
        Ok(out)
    }

    /// Dispatches one batched trial.
    ///
    /// # Errors
    ///
    /// Only when the binary cannot be found (fatal).
    pub fn dispatch_batch(
        &self,
        tier: &str,
        prompt: &str,
        ids: &[String],
    ) -> Result<BatchOutcome, String> {
        let failed = |error: String| BatchOutcome {
            verdicts: None,
            unknown_verdict_ids: Vec::new(),
            usage: JsValue::empty_object(),
            tool_calls: 0.0,
            error: Some(error),
        };
        let raw = match self.run(tier, prompt)? {
            Ok(raw) => raw,
            Err(e) => return Ok(failed(e)),
        };
        if raw.status != Some(0) {
            return Ok(failed(format!(
                "claude exited {}",
                raw.status
                    .map_or_else(|| "null".to_owned(), |c| c.to_string())
            )));
        }
        let body: Value = match serde_json::from_str(&raw.stdout) {
            Ok(b) => b,
            Err(e) => return Ok(failed(format!("claude returned a non-JSON body: {e}"))),
        };
        let mut out = parse_claude_batch_result(&body, ids);
        self.recount(&body, &mut out.tool_calls);
        Ok(out)
    }
}

/// Runs `task(i)` for every `i < n` on at most `concurrency` threads and
/// returns the results in index order (completion order never matters). The
/// first fatal error stops new tasks and is returned.
///
/// # Errors
///
/// The first task error.
pub fn run_pool<T: Send>(
    n: usize,
    concurrency: usize,
    task: &(dyn Fn(usize) -> Result<T, String> + Sync),
) -> Result<Vec<T>, String> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let slots: Mutex<Vec<Option<T>>> = Mutex::new((0..n).map(|_| None).collect());
    let error: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..concurrency.max(1).min(n.max(1)) {
            scope.spawn(|| {
                loop {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= n {
                        return;
                    }
                    match task(i) {
                        Ok(v) => {
                            if let Ok(mut s) = slots.lock() {
                                s[i] = Some(v);
                            }
                        }
                        Err(e) => {
                            stop.store(true, Ordering::SeqCst);
                            if let Ok(mut slot) = error.lock()
                                && slot.is_none()
                            {
                                *slot = Some(e);
                            }
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = error.into_inner().ok().flatten() {
        return Err(e);
    }
    let slots = slots
        .into_inner()
        .map_err(|_| "a dispatch worker panicked".to_owned())?;
    slots
        .into_iter()
        .map(|s| s.ok_or_else(|| "a dispatch produced no result".to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn verdict(o: &Outcome) -> Option<bool> {
        o.verdict
            .as_ref()
            .and_then(|v| v.get("refuted"))
            .and_then(JsValue::as_bool)
    }

    #[test]
    fn parse_claude_result_last_structured_output_wins_fenced_bare_and_non_boolean_ungraded() {
        let two = json!({"messages": [
            {"message": {"content": [{"type": "tool_use", "name": "StructuredOutput", "input": {"refuted": true}}]}},
            {"message": {"content": [{"type": "tool_use", "name": "Read", "input": {}},
                                     {"type": "tool_use", "name": "StructuredOutput", "input": {"refuted": false, "confidence": 80, "rationale": "r"}}]}}
        ], "usage": {"output_tokens": 5, "input_tokens": 6, "cache_creation_input_tokens": 7, "cache_read_input_tokens": 8}});
        let o = parse_claude_result(&two);
        assert_eq!(verdict(&o), Some(false), "the LAST StructuredOutput wins");
        assert_eq!(o.tool_calls, 3.0);
        assert_eq!(
            o.usage.stringify(),
            r#"{"output":5,"uncachedInput":6,"cacheWrite":7,"cacheRead":8}"#
        );
        assert_eq!(o.error, None);
        let bare = parse_claude_result(
            &json!({"result": "{\"refuted\": true, \"confidence\": 70}", "num_tool_uses": 4}),
        );
        assert_eq!(verdict(&bare), Some(true));
        assert_eq!(
            bare.tool_calls, 4.0,
            "num_tool_uses fills in when no tool_use was seen"
        );
        let fenced = parse_claude_result(
            &json!({"result": "Here:\n```json\n{\"refuted\": false, \"rationale\": \"a \u{7d} in a string\"}\n```"}),
        );
        assert_eq!(verdict(&fenced), Some(false));
        let non_bool = parse_claude_result(&json!({"result": "{\"refuted\": \"yes\"}"}));
        assert_eq!(
            non_bool.verdict, None,
            "a non-boolean refuted is ungraded, never false"
        );
        assert_eq!(
            non_bool.error.as_deref(),
            Some("no boolean `refuted` in the response")
        );
        let nothing = parse_claude_result(&json!({}));
        assert_eq!(nothing.verdict, None);
        assert_eq!(
            nothing.usage.stringify(),
            r#"{"output":0,"uncachedInput":0,"cacheWrite":0,"cacheRead":0}"#
        );
    }

    #[test]
    fn parse_claude_batch_result_unknown_ids_non_boolean_missing_array() {
        let ids = vec!["a".to_owned(), "b".to_owned()];
        let body = json!({"messages": [{"message": {"content": [{"type": "tool_use", "name": "StructuredOutput",
            "input": {"verdicts": [{"id": "a", "refuted": true}, {"id": "zzz", "refuted": false}, {"id": "b", "refuted": "maybe"}, 7]}}]}}]});
        let o = parse_claude_batch_result(&body, &ids);
        assert_eq!(
            o.unknown_verdict_ids,
            ["zzz"],
            "an id the dispatch did not contain is dropped"
        );
        let v = o.verdicts.unwrap_or_default();
        assert_eq!(v.len(), 2);
        assert_eq!(
            v[1].get("refuted"),
            Some(&JsValue::Null),
            "non-boolean stays ungraded"
        );
        let missing = parse_claude_batch_result(&json!({"result": "{\"nope\": 1}"}), &ids);
        assert_eq!(
            missing.verdicts, None,
            "no verdicts array is a crash, not all-omitted"
        );
        assert_eq!(
            missing.error.as_deref(),
            Some("no `verdicts` array in the response")
        );
        let from_result = parse_claude_batch_result(
            &json!({"result": "{\"verdicts\": [{\"id\": \"b\", \"refuted\": false}]}"}),
            &ids,
        );
        assert_eq!(from_result.verdicts.map(|v| v.len()), Some(1));
    }

    #[test]
    fn slug_and_tool_recount() {
        assert_eq!(
            project_slug_for("/Users/me/Projects/rdm"),
            "-Users-me-Projects-rdm"
        );
        assert_eq!(project_slug_for("/a b/\u{1f600}"), "-a-b---");
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let slug = dir.path().join(project_slug_for("/w"));
        std::fs::create_dir_all(&slug).unwrap_or_default();
        std::fs::write(
            slug.join("s1.jsonl"),
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\"},{\"type\":\"text\"},{\"type\":\"tool_use\"}]}}\nnot json\n",
        )
        .unwrap_or_default();
        assert_eq!(count_session_tool_uses("s1", "/w", dir.path()), Some(2));
        assert_eq!(count_session_tool_uses("missing", "/w", dir.path()), None);
    }

    #[test]
    fn pool_preserves_order_and_stops_on_fatal() {
        let out = run_pool(20, 4, &|i| {
            std::thread::sleep(std::time::Duration::from_millis(((20 - i) % 5) as u64));
            Ok(i * 2)
        });
        assert_eq!(out, Ok((0..20).map(|i| i * 2).collect::<Vec<_>>()));
        let err = run_pool(5, 2, &|i| {
            if i == 3 {
                Err("fatal".to_owned())
            } else {
                Ok(i)
            }
        });
        assert_eq!(err, Err("fatal".to_owned()));
    }
}
