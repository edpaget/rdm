//! `rdm-measure refuter-agreement` and `rdm-measure mine-refuter-corpus`: the
//! on-demand refuter model-tiering (and refuter-shape) instrument.
//!
//! Replaces `scripts/run-refuter-agreement.mjs`, `scripts/lib/refuter-agreement.mjs`
//! and `scripts/mine-refuter-corpus.mjs`. It dispatches the checked-in finding
//! corpus through the REAL `refutePrompt` on two or more model tiers, with
//! replicates, and reports FALSE-NEGATIVE and FALSE-POSITIVE rates separately
//! alongside token volume and tool calls.
//!
//! COST: a real run DISPATCHES PAID AGENTS (items × tiers × replicates), and
//! happens only on an explicit run with `--tiers` and without `--dry-run`.
//! `--dry-run`, `--score-only`, `--audit` and `--batch-power` dispatch nothing,
//! and the last three need no JavaScript runtime. `--dry-run` and real runs
//! regenerate each prompt through the canonical `refutePrompt` (Node, via the
//! phase-2 binding) and report `promptDrift`.
//!
//! Nothing under `.claude/workflows/` depends on this; it only calls into it.
//! Determinism: no clock, no RNG; the run label defaults to the corpus sha.

pub mod audit;
pub mod corpus;
pub mod dispatch;
pub mod miner;
pub mod prompt;
pub mod report;
pub mod score;
pub mod trials;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use corpus::{CorpusItem, GROUND_TRUTH_CLASSES, load_corpus, sha256};
use dispatch::Dispatcher;
use score::{ScoreOptions, score_anchoring, score_trials};
use trials::{BatchTrial, Trial, build_batch_trials, build_trials, group_corpus_for_batching};

use crate::measure::jsjson::{JsValue, js_len, obj};
use crate::measure::review_rules::ReviewRules;

/// The default corpus, relative to the checkout.
pub const DEFAULT_CORPUS: &str = "tests/fixtures/refuter-agreement/corpus.jsonl";
/// The `instrument` value the `--out` payload carries.
pub const INSTRUMENT: &str = "rdm-measure refuter-agreement";

const KNOWN_TIER_ALIASES: [&str; 3] = ["opus", "sonnet", "haiku"];

/// A tier alias `agent()` accepts, or a `claude-*` model id.
pub fn is_legal_tier(tier: &str) -> bool {
    if KNOWN_TIER_ALIASES.contains(&tier) {
        return true;
    }
    let lower = tier.to_ascii_lowercase();
    lower.strip_prefix("claude-").is_some_and(|rest| {
        !rest.is_empty()
            && rest.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '[' | ']')
            })
    })
}

/// The refutation shape to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One refuter per finding (production today).
    PerFinding,
    /// One refuter per unit-scoped group.
    Batched,
    /// Both arms over the same items.
    Both,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::PerFinding => "per-finding",
            Self::Batched => "batched",
            Self::Both => "both",
        }
    }
}

/// Every runner option (the CLI's flags).
#[derive(Debug, Clone)]
pub struct RunnerOptions {
    /// Corpus JSONL, relative to the checkout unless absolute.
    pub corpus: String,
    /// Tiers; the first is the baseline.
    pub tiers: Vec<String>,
    /// Replicates per (item, tier).
    pub replicates: usize,
    /// `groundTruth.class` filter.
    pub filter_class: Vec<String>,
    /// `finding.severity` filter.
    pub filter_severity: Vec<String>,
    /// Exactly these ids (an unknown id fails).
    pub only: Vec<String>,
    /// Cap after filtering.
    pub limit: Option<usize>,
    /// Run label (default: the corpus sha's first 12 hex digits).
    pub label: Option<String>,
    /// Results JSON, relative to the checkout unless absolute.
    pub out: Option<String>,
    /// JSON instead of text.
    pub json: bool,
    /// Print the plan; dispatch nothing.
    pub dry_run: bool,
    /// The `claude` executable.
    pub claude_bin: PathBuf,
    /// Parallel dispatches.
    pub concurrency: usize,
    /// Score a saved results file.
    pub score_only: Option<String>,
    /// Audit a doc section.
    pub audit: Option<String>,
    /// `refuterModelTiering` or `refuterBatching`.
    pub audit_section: String,
    /// The shape.
    pub shape: Shape,
    /// Print the batch-power analysis.
    pub batch_power: bool,
    /// Anchoring floor.
    pub min_batch_group: usize,
    /// Build an underpowered batched arm (stamped NO MEASUREMENT).
    pub allow_underpowered: bool,
}

/// Validates the options that must fail at plan time, before any dispatch.
///
/// # Errors
///
/// An unknown class or an illegal tier.
pub fn validate(o: &RunnerOptions) -> Result<(), String> {
    for c in &o.filter_class {
        if !GROUND_TRUTH_CLASSES.contains(&c.as_str()) {
            return Err(format!(
                "--filter-class \"{c}\" is not a known ground-truth class ({})",
                GROUND_TRUTH_CLASSES.join(", ")
            ));
        }
    }
    for t in &o.tiers {
        if !is_legal_tier(t) {
            return Err(format!(
                "--tiers value \"{t}\" is not a tier alias ({}) or a claude-* model id. An unknown model id makes every dispatch return null, which looks like a refuter crash rather than a typo.",
                KNOWN_TIER_ALIASES.join("|")
            ));
        }
    }
    Ok(())
}

/// Selects the corpus items the options name.
///
/// # Errors
///
/// When `--only` names an id not in the corpus.
pub fn select_items(all: Vec<CorpusItem>, o: &RunnerOptions) -> Result<Vec<CorpusItem>, String> {
    let mut items = all;
    if !o.filter_class.is_empty() {
        items.retain(|i| o.filter_class.iter().any(|c| c == i.class()));
    }
    if !o.filter_severity.is_empty() {
        items.retain(|i| o.filter_severity.iter().any(|s| s == i.severity()));
    }
    if !o.only.is_empty() {
        items.retain(|i| o.only.iter().any(|x| x == i.id()));
        let missing: Vec<&String> = o
            .only
            .iter()
            .filter(|x| !items.iter().any(|i| i.id() == x.as_str()))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "--only names {} id(s) not in the corpus: {}",
                missing.len(),
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if let Some(l) = o.limit {
        items.truncate(l);
    }
    Ok(items)
}

/// The `--out` payload.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload<'a> {
    /// The producing command.
    pub instrument: &'static str,
    /// The run label.
    pub label: String,
    /// The shape.
    pub shape: &'static str,
    /// The corpus path as given.
    pub corpus_path: String,
    /// The corpus file's sha256.
    pub corpus_sha256: String,
    /// The tiers.
    pub tiers: Vec<String>,
    /// Replicates.
    pub replicates: usize,
    /// The baseline.
    pub baseline_tier: String,
    /// Ids whose regenerated prompt drifted.
    pub prompt_drift: Vec<String>,
    /// The batched arm was underpowered.
    pub underpowered: bool,
    /// The run carries no decision.
    pub no_measurement: bool,
    /// Batched verdicts for ids a dispatch did not contain.
    pub unknown_verdict_ids: Vec<String>,
    /// Batched members a response did not grade.
    pub omitted_verdict_ids: Vec<String>,
    /// Every scoring row.
    pub trials: Vec<JsValue>,
    /// The raw batched dispatches.
    pub batch_dispatches: Vec<JsValue>,
    /// The score report.
    pub report: &'a score::ScoreReport,
}

fn write_line(w: &mut dyn Write, text: &str) {
    let _ = writeln!(w, "{text}");
}

/// Runs the refuter-agreement command. `rules` is started lazily (only
/// `--dry-run` and real runs reach the canonical `refutePrompt`).
///
/// # Errors
///
/// A fatal setup error (unreadable corpus, bad `--only`, underpowered arm,
/// missing `claude`, a failed canonical call); the caller prints it and exits
/// with status 1. A handled failure (corpus validation, audit problems)
/// returns `Ok(1)`.
pub fn run(
    o: &RunnerOptions,
    checkout: &Path,
    rules: &mut dyn FnMut() -> Result<Box<dyn ReviewRules>, String>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<u8, String> {
    validate(o)?;
    if let Some(doc_arg) = &o.audit {
        let path = checkout.join(doc_arg);
        let parsed: serde_json::Value = match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
        {
            Ok(v) => v,
            Err(e) => {
                write_line(
                    stderr,
                    &format!(
                        "{doc_arg} is not parseable JSON (--audit expects docs/token-baseline.json): {e}"
                    ),
                );
                return Ok(1);
            }
        };
        let Some(section) = parsed
            .get(&o.audit_section)
            .filter(|s| crate::measure::sidecar::truthy(Some(s)))
        else {
            write_line(
                stderr,
                &format!("{doc_arg} has no \"{}\" section", o.audit_section),
            );
            return Ok(1);
        };
        let problems = if o.audit_section == "refuterBatching" {
            audit::audit_batching_section(section)
        } else {
            audit::audit_tiering_section(section)
        };
        if problems.is_empty() {
            write_line(
                stdout,
                &format!(
                    "run-refuter-agreement --audit OK: {doc_arg}'s {} figures are internally consistent",
                    o.audit_section
                ),
            );
            return Ok(0);
        }
        write_line(
            stderr,
            &format!("run-refuter-agreement --audit FAILED against {doc_arg}:"),
        );
        for p in problems {
            write_line(stderr, &format!("  {p}"));
        }
        return Ok(1);
    }

    let corpus_path = checkout.join(&o.corpus);
    let corpus_text = crate::measure::sidecar::read_text(&corpus_path).map_err(|e| {
        format!(
            "cannot read the corpus {} ({}): {}",
            o.corpus,
            corpus_path.display(),
            crate::measure::sidecar::io_message(&e)
        )
    })?;
    let (all, errors) = load_corpus(&corpus_text);
    if !errors.is_empty() {
        write_line(
            stderr,
            &format!(
                "corpus {} has {} validation error(s):",
                o.corpus,
                errors.len()
            ),
        );
        for e in errors.iter().take(25) {
            write_line(stderr, &format!("  {e}"));
        }
        return Ok(1);
    }
    let items = select_items(all, o)?;
    let item_refs: Vec<&JsValue> = items.iter().map(CorpusItem::value).collect();

    if o.batch_power {
        let summary = group_corpus_for_batching(&item_refs, o.min_batch_group)?;
        write_line(stdout, &trials::format_batch_power(&summary));
        return Ok(0);
    }

    if let Some(saved_path) = &o.score_only {
        let path = checkout.join(saved_path);
        let text = crate::measure::sidecar::read_text(&path).map_err(|e| {
            format!(
                "cannot read --score-only {saved_path}: {}",
                crate::measure::sidecar::io_message(&e)
            )
        })?;
        let saved = JsValue::parse(&text)
            .map_err(|e| format!("--score-only {saved_path} is not JSON: {e}"))?;
        let rows = saved
            .get("trials")
            .and_then(JsValue::as_array)
            .cloned()
            .unwrap_or_default();
        let report = score_trials(
            &items,
            &rows,
            ScoreOptions {
                baseline_tier: saved
                    .get("baselineTier")
                    .filter(|b| b.truthy())
                    .map(JsValue::js_string),
                ..ScoreOptions::default()
            },
        );
        let body = if o.json {
            report::format_json(&report)?
        } else {
            report::format_text(&report)
        };
        write_line(stdout, &body);
        return Ok(0);
    }

    if o.tiers.is_empty() {
        return Err("--tiers is required (e.g. --tiers opus,sonnet)".to_owned());
    }

    // Regenerate every prompt through the REAL refutePrompt; a sha mismatch is
    // reported (promptDrift), never silently accepted.
    let mut review = rules()?;
    let mut prompts: Vec<(String, String)> = Vec::new();
    let mut drifted: Vec<String> = Vec::new();
    for item in &items {
        let p = prompt::regenerate_prompt(item, review.as_mut())?;
        if sha256(&p) != item.prompt_sha256() {
            drifted.push(item.id().to_owned());
        }
        prompts.push((item.id().to_owned(), p));
    }
    drop(review);
    if !drifted.is_empty() {
        let shown: Vec<&str> = drifted.iter().take(8).map(String::as_str).collect();
        write_line(
            stderr,
            &format!(
                "WARNING: {} corpus item(s) no longer regenerate byte-identically through refutePrompt (promptDrift): {}{}",
                drifted.len(),
                shown.join(", "),
                if drifted.len() > 8 { ", …" } else { "" }
            ),
        );
    }

    let wants_batched = matches!(o.shape, Shape::Batched | Shape::Both);
    let wants_per_finding = matches!(o.shape, Shape::PerFinding | Shape::Both);
    let mut power = None;
    let mut plan = None;
    let mut batch_prompts: Vec<(String, String)> = Vec::new();
    if wants_batched {
        let summary = group_corpus_for_batching(&item_refs, o.min_batch_group)?;
        let built = build_batch_trials(&summary, &o.tiers, o.replicates, o.allow_underpowered)?;
        for g in &built.groups {
            let members: Vec<&CorpusItem> = g
                .ids
                .iter()
                .filter_map(|id| items.iter().find(|i| i.id() == id))
                .collect();
            // The batch-local grading key: corpus ids double as refute_id.
            let keyed: Vec<JsValue> = members
                .iter()
                .map(|m| {
                    let mut o = crate::measure::jsjson::JsObject::new();
                    o.insert("refute_id", JsValue::String(m.id().to_owned()));
                    if let Some(f) = m.finding().as_object() {
                        for (k, v) in f.iter() {
                            o.insert(k.clone(), v.clone());
                        }
                    }
                    JsValue::Object(o)
                })
                .collect();
            let first = members.first();
            batch_prompts.push((
                g.key.clone(),
                prompt::build_batch_prompt(
                    first.map_or("", |m| m.mode()),
                    &g.dim,
                    &keyed,
                    first.map(|m| m.target()),
                ),
            ));
        }
        power = Some(summary);
        plan = Some(built);
    }
    let per_finding_items: Vec<&JsValue> = match (&plan, o.shape) {
        (Some(p), Shape::Both) => item_refs
            .iter()
            .copied()
            .filter(|i| {
                let id = i.get("id").and_then(JsValue::as_str).unwrap_or("");
                p.corpus_ids.iter().any(|c| c == id)
            })
            .collect(),
        _ => item_refs.clone(),
    };
    let trials: Vec<Trial> = if wants_per_finding {
        build_trials(&per_finding_items, &o.tiers, o.replicates)?
    } else {
        Vec::new()
    };
    let batch_trials: Vec<BatchTrial> = plan.as_ref().map(|p| p.trials.clone()).unwrap_or_default();
    let label = o
        .label
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| sha256(&corpus_text)[..12].to_owned());
    let prompt_of = |id: &str| {
        prompts
            .iter()
            .find(|(k, _)| k == id)
            .map_or("", |(_, p)| p.as_str())
    };
    let batch_prompt_of = |key: &str| {
        batch_prompts
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, p)| p.as_str())
    };

    if o.dry_run {
        write_line(
            stdout,
            &format!(
                "DRY RUN — label {label}, shape {}: {} item(s) x {} tier(s) x {} replicate(s) = {} trial(s), plus {} batched dispatch(es). Nothing dispatched.",
                o.shape.name(),
                per_finding_items.len(),
                o.tiers.len(),
                o.replicates,
                trials.len(),
                batch_trials.len()
            ),
        );
        for t in &trials {
            write_line(
                stdout,
                &format!(
                    "  {}  ({} prompt chars)",
                    t.trial_id,
                    js_len(prompt_of(&t.corpus_id))
                ),
            );
        }
        for t in &batch_trials {
            write_line(
                stdout,
                &format!(
                    "  {}  [batched x{}]  ({} prompt chars)",
                    t.trial_id,
                    t.corpus_ids.len(),
                    js_len(batch_prompt_of(&t.group_key))
                ),
            );
        }
        return Ok(0);
    }

    let dispatcher = Dispatcher {
        claude_bin: o.claude_bin.clone(),
        cwd: checkout.to_owned(),
        projects_root: crate::measure::sidecar::default_projects_root(),
    };
    // Progress goes straight to stderr as each dispatch finishes.
    let log = |line: String| {
        let _ = writeln!(std::io::stderr().lock(), "{line}");
    };
    let results: Vec<JsValue> = dispatch::run_pool(trials.len(), o.concurrency, &|i| {
        let t = &trials[i];
        let out = dispatcher.dispatch(&t.tier, prompt_of(&t.corpus_id))?;
        let graded = out
            .verdict
            .as_ref()
            .and_then(|v| v.get("refuted"))
            .and_then(JsValue::as_bool);
        log(format!(
            "trial {}: {}",
            t.trial_id,
            graded.map_or_else(|| "ungraded".to_owned(), |r| format!("refuted={r}"))
        ));
        Ok(obj([
            ("trialId", t.trial_id.clone().into()),
            ("corpusId", t.corpus_id.clone().into()),
            ("tier", t.tier.clone().into()),
            ("replicate", t.replicate.into()),
            ("verdict", out.verdict.unwrap_or(JsValue::Null)),
            ("error", out.error.into()),
            ("usage", out.usage),
            ("toolCalls", JsValue::Number(out.tool_calls)),
        ]))
    })?;
    let batch_results: Vec<JsValue> =
        dispatch::run_pool(batch_trials.len(), o.concurrency, &|i| {
            let t = &batch_trials[i];
            let out =
                dispatcher.dispatch_batch(&t.tier, batch_prompt_of(&t.group_key), &t.corpus_ids)?;
            let graded = out.verdicts.as_ref().map_or(0, Vec::len);
            log(format!(
                "batch {}: {graded}/{} graded",
                t.trial_id,
                t.corpus_ids.len()
            ));
            Ok(obj([
                ("trialId", t.trial_id.clone().into()),
                ("dispatchId", t.dispatch_id.clone().into()),
                ("groupKey", t.group_key.clone().into()),
                ("tier", t.tier.clone().into()),
                ("replicate", t.replicate.into()),
                (
                    "corpusIds",
                    JsValue::Array(t.corpus_ids.iter().cloned().map(JsValue::String).collect()),
                ),
                (
                    "verdicts",
                    out.verdicts.map_or(JsValue::Null, JsValue::Array),
                ),
                (
                    "unknownVerdictIds",
                    JsValue::Array(
                        out.unknown_verdict_ids
                            .into_iter()
                            .map(JsValue::String)
                            .collect(),
                    ),
                ),
                ("error", out.error.into()),
                ("usage", out.usage),
                ("toolCalls", JsValue::Number(out.tool_calls)),
            ]))
        })?;
    let expanded = trials::expand_batch_results(&batch_results);
    let mut all_rows = results.clone();
    all_rows.extend(expanded.rows.iter().cloned());
    let anchoring = plan.as_ref().map(|p| {
        let mut arms = BTreeMap::new();
        arms.insert("per-finding".to_owned(), results.clone());
        arms.insert("batched".to_owned(), expanded.rows.clone());
        score_anchoring(&p.groups, &arms, o.min_batch_group)
    });
    let no_measurement = plan.as_ref().is_some_and(|p| p.no_measurement);
    let report = score_trials(
        &items,
        &all_rows,
        ScoreOptions {
            baseline_tier: o.tiers.first().cloned(),
            no_measurement,
            anchoring,
            decision: None,
            batch_power: power,
        },
    );
    if let Some(out_path) = &o.out {
        let payload = Payload {
            instrument: INSTRUMENT,
            label,
            shape: o.shape.name(),
            corpus_path: o.corpus.clone(),
            corpus_sha256: sha256(&corpus_text),
            tiers: o.tiers.clone(),
            replicates: o.replicates,
            baseline_tier: o.tiers.first().cloned().unwrap_or_default(),
            prompt_drift: drifted,
            underpowered: plan.as_ref().is_some_and(|p| p.underpowered),
            no_measurement,
            unknown_verdict_ids: expanded.unknown_verdict_ids.clone(),
            omitted_verdict_ids: expanded.omitted_ids.clone(),
            trials: all_rows,
            batch_dispatches: batch_results,
            report: &report,
        };
        let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        let path = checkout.join(out_path);
        std::fs::write(&path, format!("{text}\n"))
            .map_err(|e| format!("cannot write --out \"{}\": {e}", path.display()))?;
    }
    let body = if o.json {
        report::format_json(&report)?
    } else {
        report::format_text(&report)
    };
    write_line(stdout, &body);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_legality() {
        for ok in [
            "opus",
            "sonnet",
            "haiku",
            "claude-opus-5",
            "Claude-Sonnet-4.5[1m]",
        ] {
            assert!(is_legal_tier(ok), "{ok}");
        }
        for bad in ["not-a-real-tier", "claude-", "gpt-4", "claude-x y", "Opus"] {
            assert!(!is_legal_tier(bad), "{bad}");
        }
    }
}
