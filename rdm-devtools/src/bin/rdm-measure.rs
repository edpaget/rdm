//! `rdm-measure` — repository-only measurement tools (see
//! `rdm_devtools::measure`).
//!
//! ```text
//! rdm-measure lane-tokens [--since ISO] [--workflow NAME]... [--format text|json]
//!                         [--out PATH] [--root DIR]
//! rdm-measure refuter-severity [--root DIR] [--until ISO] [--format text|json]
//!                              [--check DOC | --audit DOC]
//!                              [--review-lib PATH] [--host-timeout-secs N]
//! rdm-measure mine-refuter-corpus [--root DIR] [--project-slug P]... [--until ISO]
//!                                 [--severity S]... [--limit N] [--min-group-size N]
//!                                 [--exclude-corpus PATH] [--out PATH] [--format jsonl|json]
//! rdm-measure refuter-agreement [--corpus PATH] [--tiers T,...] [--replicates N]
//!                               [--filter-class C]... [--filter-severity S]... [--only ID,...]
//!                               [--limit N] [--label S] [--out PATH] [--format text|json]
//!                               [--dry-run] [--claude-bin PATH] [--concurrency N]
//!                               [--score-only PATH] [--audit DOC [--audit-section S]]
//!                               [--shape per-finding|batched|both] [--batch-power]
//!                               [--min-batch-group N] [--allow-underpowered]
//!                               [--review-lib PATH] [--host-timeout-secs N]
//! ```
//!
//! COST WARNING: `refuter-agreement` with `--tiers` and without `--dry-run`
//! dispatches paid agents (items x tiers x replicates) through `claude -p`.
//!
//! Run through Cargo from the checkout:
//! `cargo run -q -p rdm-devtools --bin rdm-measure -- <subcommand> ...`.
//! Argument errors exit 1 and `--help` exits 0. Nothing reads the real
//! `~/.claude/projects` unless `--root` is omitted.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use rdm_devtools::measure::args::{
    self, comma_split, flag_value, int_at_least_two, parse_or_exit, positive_int,
};
use rdm_devtools::measure::refuter_agreement::{self, RunnerOptions, Shape, miner};
use rdm_devtools::measure::refuter_severity::{
    self, INSTRUMENT, InstrumentReport, MeasureOptions, doc, render,
};
use rdm_devtools::measure::review_rules::{
    DEFAULT_HOST_TIMEOUT, NodeReviewRules, checkout_root, default_review_lib,
};
use rdm_devtools::measure::sidecar::{RunFilters, default_projects_root};
use rdm_devtools::measure::{jsdate, lane_tokens};

#[derive(Parser)]
#[command(
    name = "rdm-measure",
    about = "Token and refuter measurement tools (repository-only; not part of the rdm release)",
    args_override_self = true
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum TextOrJson {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Cmd {
    /// Token usage across Workflow runs by token class, grouped by agent class,
    /// label, model and workflow, with the never-reconciled sidecar discrepancy.
    LaneTokens(LaneTokensArgs),
    /// Refuter spend by graded finding severity, the review fan-out
    /// distributions and the determining-finding rank (needs Node, except
    /// --audit).
    RefuterSeverity(RefuterSeverityArgs),
    /// Mine historical refuter findings verbatim from agent transcripts as
    /// candidate corpus records (groundTruth: null; no JavaScript runtime).
    MineRefuterCorpus(MineArgs),
    /// Replay the adjudicated corpus through the real refutePrompt on model
    /// tiers and score FN/FP separately. COST WARNING: a run with --tiers and
    /// without --dry-run dispatches paid agents.
    RefuterAgreement(AgreementArgs),
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum JsonlOrJson {
    Jsonl,
    Json,
}

#[derive(clap::Args)]
struct MineArgs {
    /// Session-sidecar root (default: ~/.claude/projects).
    #[arg(long, value_name = "DIR", value_parser = flag_value)]
    root: Option<String>,
    /// Project-slug prefix filter; repeatable and comma-separated (default:
    /// this repo's slugs, including --worktrees- variants). Slugs start with
    /// `-`, so a hyphen-leading value is accepted (but never a `--flag`).
    #[arg(long, value_name = "PREFIX", value_parser = flag_value, allow_hyphen_values = true)]
    project_slug: Vec<String>,
    /// Ignore runs starting after this instant.
    #[arg(long, value_name = "ISO", value_parser = flag_value)]
    until: Option<String>,
    /// Finding-severity filter; repeatable and comma-separated.
    #[arg(long, value_name = "SEVERITY", value_parser = flag_value)]
    severity: Vec<String>,
    /// Stop after N recovered records.
    #[arg(long, value_name = "N", value_parser = positive_int)]
    limit: Option<u64>,
    /// Emit only candidates whose unit-scoped batch group has >= N members.
    #[arg(long, value_name = "N", value_parser = int_at_least_two)]
    min_group_size: Option<u64>,
    /// Skip candidates whose id is already in this corpus JSONL.
    #[arg(long, value_name = "PATH", value_parser = flag_value)]
    exclude_corpus: Option<String>,
    /// Write the output here instead of stdout.
    #[arg(long, value_name = "PATH", value_parser = flag_value)]
    out: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value = "jsonl")]
    format: JsonlOrJson,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ShapeArg {
    PerFinding,
    Batched,
    Both,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
#[value(rename_all = "verbatim")]
enum AuditSection {
    #[value(name = "refuterModelTiering")]
    RefuterModelTiering,
    #[value(name = "refuterBatching")]
    RefuterBatching,
}

#[derive(clap::Args)]
struct AgreementArgs {
    /// Corpus JSONL, relative to the checkout.
    #[arg(long, value_name = "PATH", value_parser = flag_value, default_value = refuter_agreement::DEFAULT_CORPUS)]
    corpus: String,
    /// Model tiers (aliases opus|sonnet|haiku or claude-* ids); repeatable and
    /// comma-separated; the first is the baseline.
    #[arg(long, value_name = "TIERS", value_parser = flag_value)]
    tiers: Vec<String>,
    /// Replicates per (item, tier).
    #[arg(long, value_name = "N", value_parser = positive_int, default_value = "2")]
    replicates: u64,
    /// groundTruth.class filter; repeatable and comma-separated.
    #[arg(long, value_name = "CLASS", value_parser = flag_value)]
    filter_class: Vec<String>,
    /// finding.severity filter; repeatable and comma-separated.
    #[arg(long, value_name = "SEVERITY", value_parser = flag_value)]
    filter_severity: Vec<String>,
    /// Run exactly these corpus ids (an unknown id fails).
    #[arg(long, value_name = "IDS", value_parser = flag_value)]
    only: Vec<String>,
    /// Cap the corpus after filtering.
    #[arg(long, value_name = "N", value_parser = positive_int)]
    limit: Option<u64>,
    /// Run label (default: the corpus sha, never the clock).
    #[arg(long, value_name = "LABEL", value_parser = flag_value)]
    label: Option<String>,
    /// Write the results JSON here (relative to the checkout).
    #[arg(long, value_name = "PATH", value_parser = flag_value)]
    out: Option<String>,
    /// Report format.
    #[arg(long, value_enum, default_value = "text")]
    format: TextOrJson,
    /// Print the trial plan; dispatch nothing.
    #[arg(long)]
    dry_run: bool,
    /// The `claude` executable to dispatch through.
    #[arg(long, value_name = "PATH", value_parser = flag_value, default_value = "claude")]
    claude_bin: String,
    /// Dispatch N trials at a time (output order is unaffected).
    #[arg(long, value_name = "N", value_parser = positive_int, default_value = "1")]
    concurrency: u64,
    /// Score a saved results JSON; dispatch nothing.
    #[arg(long, value_name = "PATH", value_parser = flag_value)]
    score_only: Option<String>,
    /// Corpus-free arithmetic audit of a docs/token-baseline.json section.
    #[arg(long, value_name = "DOC", value_parser = flag_value)]
    audit: Option<String>,
    /// Which section --audit checks.
    #[arg(long, value_enum, default_value = "refuterModelTiering")]
    audit_section: AuditSection,
    /// Refutation shape to drive.
    #[arg(long, value_enum, default_value = "per-finding")]
    shape: ShapeArg,
    /// Report the unit-scoped batch-size distribution and a POWER verdict;
    /// dispatches nothing.
    #[arg(long)]
    batch_power: bool,
    /// Minimum unit-scoped group size for the anchoring measurement.
    #[arg(long, value_name = "N", value_parser = int_at_least_two, default_value = "3")]
    min_batch_group: u64,
    /// Build a batched arm below the pre-registered floor anyway (stamped NO
    /// MEASUREMENT; never carries a decision).
    #[arg(long)]
    allow_underpowered: bool,
    /// The canonical review source (default: the checkout's review.mjs).
    #[arg(long, value_name = "PATH", value_parser = flag_value)]
    review_lib: Option<String>,
    /// Overall deadline for the Node review host.
    #[arg(long, value_name = "N", value_parser = positive_int)]
    host_timeout_secs: Option<u64>,
}

#[derive(clap::Args)]
struct LaneTokensArgs {
    /// Only include runs starting on/after this date.
    #[arg(long, value_name = "ISO", value_parser = flag_value)]
    since: Option<String>,
    /// Only include runs with this workflowName. Repeatable; repeats are OR'd
    /// (any match), unlike rdm's AND-ed --tag.
    #[arg(long, value_name = "NAME", value_parser = flag_value)]
    workflow: Vec<String>,
    /// Output format.
    #[arg(long, value_enum, default_value = "text")]
    format: TextOrJson,
    /// Write the output here instead of stdout (the parent directory must exist).
    #[arg(long, value_name = "PATH", value_parser = flag_value)]
    out: Option<String>,
    /// Session-sidecar root (default: ~/.claude/projects).
    #[arg(long, value_name = "DIR", value_parser = flag_value)]
    root: Option<String>,
}

#[derive(clap::Args)]
struct RefuterSeverityArgs {
    /// Session-sidecar root (default: ~/.claude/projects).
    #[arg(long, value_name = "DIR", value_parser = flag_value)]
    root: Option<String>,
    /// Ignore runs starting after this instant.
    #[arg(long, value_name = "ISO", value_parser = flag_value)]
    until: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value = "text")]
    format: TextOrJson,
    /// Recompute over the corpus and assert DOC's nonGatingRefutationSkip,
    /// refuterFanout and determiningFindingRank figures match exactly (applies
    /// the doc's measurementWindow.until unless --until overrides it).
    #[arg(long, value_name = "DOC", value_parser = flag_value)]
    check: Option<String>,
    /// Corpus-free arithmetic audit of DOC's own figures (no JavaScript runtime).
    #[arg(long, value_name = "DOC", value_parser = flag_value)]
    audit: Option<String>,
    /// The canonical review source (default: the checkout's
    /// .claude/workflows/lib/review.mjs).
    #[arg(long, value_name = "PATH", value_parser = flag_value)]
    review_lib: Option<String>,
    /// Overall deadline for the Node review host.
    #[arg(long, value_name = "N", value_parser = positive_int)]
    host_timeout_secs: Option<u64>,
}

/// Writes to stdout ignoring a closed pipe.
fn out(text: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{text}");
}

fn err(text: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{text}");
}

fn root_of(root: Option<&str>) -> (PathBuf, String) {
    match root {
        Some(r) => (PathBuf::from(r), r.to_owned()),
        None => {
            let d = default_projects_root();
            let shown = d.display().to_string();
            (d, shown)
        }
    }
}

fn lane_tokens(a: &LaneTokensArgs) -> Result<(), String> {
    let (root, shown) = root_of(a.root.as_deref());
    let filters = RunFilters {
        since: a.since.clone(),
        workflow_names: a.workflow.clone(),
    };
    let report = lane_tokens::build_report(&root, &shown, &filters)?;
    let body = match a.format {
        TextOrJson::Json => lane_tokens::format_json(&report)?,
        TextOrJson::Text => lane_tokens::format_text(&report),
    };
    match &a.out {
        Some(path) => {
            let dir = args::dirname(path);
            if !std::path::Path::new(&dir).exists() {
                return Err(format!("--out parent directory does not exist: \"{dir}\""));
            }
            fs::write(path, format!("{body}\n"))
                .map_err(|e| format!("cannot write --out \"{path}\": {e}"))
        }
        None => {
            out(&body);
            Ok(())
        }
    }
}

fn start_rules(a: &RefuterSeverityArgs) -> Result<NodeReviewRules, String> {
    let lib = a
        .review_lib
        .as_ref()
        .map_or_else(default_review_lib, PathBuf::from);
    let timeout = a
        .host_timeout_secs
        .map_or(DEFAULT_HOST_TIMEOUT, Duration::from_secs);
    NodeReviewRules::start(&lib, timeout)
}

fn refuter_severity_cmd(a: &RefuterSeverityArgs) -> Result<ExitCode, String> {
    let repo = checkout_root();
    // Validate the window before anything starts a JavaScript runtime.
    if let Some(until) = a.until.as_deref().filter(|u| !u.is_empty())
        && jsdate::parse(until).is_none()
    {
        return Err(format!(
            "--until value is not a parseable date: \"{until}\""
        ));
    }
    if let Some(doc_arg) = &a.audit {
        let doc = doc::read_doc(doc_arg, &repo)?;
        let problems = doc::audit_all(&doc, &refuter_severity::rank::CAP_VERDICT_RULE);
        if problems.is_empty() {
            out(&format!(
                "measure-refuter-severity --audit OK: {doc_arg}'s figures are internally consistent"
            ));
            return Ok(ExitCode::SUCCESS);
        }
        err(&format!(
            "measure-refuter-severity --audit FAILED against {doc_arg}:"
        ));
        for p in problems {
            err(&format!("  {p}"));
        }
        return Ok(ExitCode::from(1));
    }
    let (root, shown) = root_of(a.root.as_deref());
    if let Some(doc_arg) = &a.check {
        let doc = doc::read_doc(doc_arg, &repo)?;
        let until = a.until.clone().or_else(|| {
            doc.section
                .get("measurementWindow")
                .and_then(|w| w.get("until"))
                .and_then(serde_json::Value::as_str)
                .filter(|u| !u.is_empty())
                .map(str::to_owned)
        });
        let mut rules = start_rules(a)?;
        let report = refuter_severity::measure(
            &MeasureOptions {
                root,
                root_display: shown,
                until,
            },
            &mut rules,
        )?;
        let _ = rules.shutdown();
        let value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        let missing = doc::check_all(&value, &doc);
        if missing.is_empty() {
            out(&format!(
                "measure-refuter-severity --check OK: {doc_arg} matches the measured corpus"
            ));
            return Ok(ExitCode::SUCCESS);
        }
        err(&format!(
            "measure-refuter-severity --check FAILED against {doc_arg}:"
        ));
        for m in missing {
            err(&format!("  {m}"));
        }
        err(
            "\nRe-run `cargo run -q -p rdm-devtools --bin rdm-measure -- refuter-severity --format json` and update the doc.",
        );
        return Ok(ExitCode::from(1));
    }
    let mut rules = start_rules(a)?;
    let report = refuter_severity::measure(
        &MeasureOptions {
            root,
            root_display: shown,
            until: a.until.clone(),
        },
        &mut rules,
    )?;
    let _ = rules.shutdown();
    match a.format {
        TextOrJson::Json => {
            let body = serde_json::to_string_pretty(&InstrumentReport {
                instrument: INSTRUMENT,
                report: &report,
            })
            .map_err(|e| e.to_string())?;
            out(&body);
        }
        TextOrJson::Text => out(&render::render_text(&report)),
    }
    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::cast_possible_truncation)]
fn mine_cmd(a: &MineArgs) -> Result<(), String> {
    let (root, shown) = root_of(a.root.as_deref());
    let exclude_ids = match &a.exclude_corpus {
        Some(p) => {
            miner::read_corpus_ids(&std::path::absolute(p).unwrap_or_else(|_| PathBuf::from(p)))?
        }
        None => Vec::new(),
    };
    let result = miner::mine(&miner::MineOptions {
        root,
        root_display: shown,
        project_slug_prefixes: comma_split(&a.project_slug),
        until: a.until.clone(),
        severities: comma_split(&a.severity),
        limit: a.limit.map(|n| n as usize),
        min_group_size: a.min_group_size.map(|n| n as usize),
        exclude_ids,
    })?;
    let body = match a.format {
        JsonlOrJson::Json => serde_json::to_string_pretty(&miner::InstrumentMine {
            instrument: miner::INSTRUMENT,
            result: &result,
        })
        .map_err(|e| e.to_string())?,
        JsonlOrJson::Jsonl => miner::jsonl(&result.items),
    };
    match &a.out {
        Some(path) => {
            fs::write(path, &body).map_err(|e| format!("cannot write --out \"{path}\": {e}"))?
        }
        None => {
            let _ = std::io::stdout().lock().write_all(body.as_bytes());
        }
    }
    err(&miner::summary_line(&result));
    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn agreement_cmd(a: &AgreementArgs) -> Result<ExitCode, String> {
    let options = RunnerOptions {
        corpus: a.corpus.clone(),
        tiers: comma_split(&a.tiers),
        replicates: a.replicates as usize,
        filter_class: comma_split(&a.filter_class),
        filter_severity: comma_split(&a.filter_severity),
        only: comma_split(&a.only),
        limit: a.limit.map(|n| n as usize),
        label: a.label.clone(),
        out: a.out.clone(),
        json: a.format == TextOrJson::Json,
        dry_run: a.dry_run,
        claude_bin: PathBuf::from(&a.claude_bin),
        concurrency: a.concurrency as usize,
        score_only: a.score_only.clone(),
        audit: a.audit.clone(),
        audit_section: match a.audit_section {
            AuditSection::RefuterModelTiering => "refuterModelTiering",
            AuditSection::RefuterBatching => "refuterBatching",
        }
        .to_owned(),
        shape: match a.shape {
            ShapeArg::PerFinding => Shape::PerFinding,
            ShapeArg::Batched => Shape::Batched,
            ShapeArg::Both => Shape::Both,
        },
        batch_power: a.batch_power,
        min_batch_group: a.min_batch_group as usize,
        allow_underpowered: a.allow_underpowered,
    };
    let lib = a
        .review_lib
        .as_ref()
        .map_or_else(default_review_lib, PathBuf::from);
    let timeout = a
        .host_timeout_secs
        .map_or(DEFAULT_HOST_TIMEOUT, Duration::from_secs);
    let mut start =
        || -> Result<Box<dyn rdm_devtools::measure::review_rules::ReviewRules>, String> {
            Ok(Box::new(NodeReviewRules::start(&lib, timeout)?))
        };
    let code = refuter_agreement::run(
        &options,
        &checkout_root(),
        &mut start,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )?;
    Ok(ExitCode::from(code))
}

fn main() -> ExitCode {
    let cli = match parse_or_exit::<Cli>() {
        Ok(cli) => cli,
        Err(code) => return code,
    };
    let result = match &cli.command {
        Cmd::LaneTokens(a) => lane_tokens(a).map(|()| ExitCode::SUCCESS),
        Cmd::RefuterSeverity(a) => refuter_severity_cmd(a),
        Cmd::MineRefuterCorpus(a) => mine_cmd(a).map(|()| ExitCode::SUCCESS),
        Cmd::RefuterAgreement(a) => agreement_cmd(a),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            err(&format!("error: {e}"));
            ExitCode::from(1)
        }
    }
}
