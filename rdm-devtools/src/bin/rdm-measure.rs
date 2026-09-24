//! `rdm-measure` — repository-only measurement tools (see
//! `rdm_devtools::measure`).
//!
//! ```text
//! rdm-measure lane-tokens [--since ISO] [--workflow NAME]... [--format text|json]
//!                         [--out PATH] [--root DIR]
//! rdm-measure refuter-severity [--root DIR] [--until ISO] [--format text|json]
//!                              [--check DOC | --audit DOC]
//!                              [--review-lib PATH] [--host-timeout-secs N]
//! ```
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
use rdm_devtools::measure::args::{self, flag_value, parse_or_exit, positive_int};
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

fn main() -> ExitCode {
    let cli = match parse_or_exit::<Cli>() {
        Ok(cli) => cli,
        Err(code) => return code,
    };
    let result = match &cli.command {
        Cmd::LaneTokens(a) => lane_tokens(a).map(|()| ExitCode::SUCCESS),
        Cmd::RefuterSeverity(a) => refuter_severity_cmd(a),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            err(&format!("error: {e}"));
            ExitCode::from(1)
        }
    }
}
