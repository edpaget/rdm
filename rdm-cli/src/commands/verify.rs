//! `rdm verify` — the CLI surface over the `dispatch.verify` key, resolved
//! for the command's project (env override, then the project's own override,
//! then the plan-repo-wide value).
//!
//! `docs/verify-gate.md` is canonical for the key itself: one project-supplied
//! command, never decomposed or partially run, whose exit code gates a phase.
//! This module adds two ways to reach it from outside the dispatch pipeline:
//! `rdm verify resolve` (what is configured?) and `rdm verify run` (run it in
//! the item's worktree and report the result).
//!
//! Discovery — inferring a command from CI config, `docs/principles.md` or
//! `CLAUDE.md` when the key is unset — is deliberately **not** implemented
//! here. It stays an agent step in the orchestrator, whose result is written
//! into the plan document rather than into config, so a guess never silently
//! becomes a project's durable verification contract.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use rdm_core::config::{Config, ConfigSource, GlobalConfig, ResolvedValue};

use crate::OutputFormat;
use crate::VerifyCommand;
use crate::commands;
use crate::paths;

/// How many characters of merged command output `run` reports.
///
/// The **last** 4000, not the first: a failing command prints its diagnosis
/// at the end. Matches the bound `docs/verify-gate.md` § 6 states, so the two
/// surfaces cannot disagree.
const TAIL_LIMIT: usize = 4000;

/// Exit code for "no verification command is configured".
///
/// Distinct from 0/1 so a shell caller can tell an unconfigured project from a
/// failing one — but note a command that legitimately exits 2 is
/// indistinguishable by exit code alone, which is why the JSON payload's
/// `resolved` field is the contract callers should key off.
const EXIT_UNRESOLVED: i32 = 2;

/// Exit code for a command that could not produce an exit status at all
/// (killed by a signal). Fail-closed: an unrunnable verification is never a
/// pass.
const EXIT_UNRUNNABLE: i32 = 1;

/// Exit code for `--item` naming an item that does not resolve to a
/// worktree: the configured command is known, but there is nowhere to run
/// it, so it never ran at all.
///
/// Must be distinct from [`EXIT_UNRUNNABLE`] (1): "the command failed" and
/// "the command never ran" are different facts, and a caller (the dispatch
/// orchestrator) charges them differently — one is rework, the other an
/// escalation. Must also be distinct from [`EXIT_UNRESOLVED`] (2): 2 means
/// "nothing is configured, fall back to a caller-supplied command", and
/// falling back here would run the *right* command in the wrong (or an
/// arbitrary) directory, since a command genuinely is configured.
const EXIT_ITEM_UNRESOLVED: i32 = 3;

/// Dispatches an `rdm verify` subcommand.
///
/// # Errors
///
/// Returns an error when the project cannot be resolved, when the configured
/// command is multi-line, or when the configured command cannot be spawned.
///
/// Two other failure cases exit the process directly rather than returning
/// an error, each after emitting a result payload: no command is configured
/// (exit [`EXIT_UNRESOLVED`]), and `--item` names an item with no worktree
/// (exit [`EXIT_ITEM_UNRESOLVED`]).
pub fn run(
    command: VerifyCommand,
    root: &Path,
    repo_config: &Config,
    raw_repo_config: &Config,
    global_config: &GlobalConfig,
    format: OutputFormat,
) -> Result<()> {
    let configs = Configs {
        merged: repo_config,
        raw_repo: raw_repo_config,
        global: global_config,
    };
    match command {
        VerifyCommand::Resolve { project } => resolve(&configs, format, project),
        VerifyCommand::Run { item, project } => run_command(root, &configs, format, item, project),
    }
}

/// The configs `verify` reads: `merged` (repo + global defaults) only for
/// project resolution, `raw_repo`/`global` for the scoped resolver.
struct Configs<'a> {
    merged: &'a Config,
    raw_repo: &'a Config,
    global: &'a GlobalConfig,
}

/// The configured verification command, if any.
///
/// Reads through `rdm_core::config::resolve_scoped_value` — the same
/// project-aware resolver `rdm config get dispatch.verify --raw --project <p>`
/// uses — so the two surfaces can never disagree about what is configured.
/// The source is returned too, so the multi-line refusal can name the fix.
fn configured_command(project: &str, configs: &Configs) -> Result<Option<ResolvedValue<String>>> {
    Ok(rdm_core::config::resolve_scoped_value(
        "dispatch.verify",
        Some(project),
        configs.raw_repo,
        configs.global,
        |k| std::env::var(k).ok(),
    )?)
}

fn resolve(configs: &Configs, format: OutputFormat, project: Option<String>) -> Result<()> {
    // Resolving the project also keeps an unknown `--project` an error here,
    // matching every other subcommand's behavior.
    let project = paths::resolve_project(project, configs.merged)?;
    let cmd = configured_command(&project, configs)?.map(|r| r.value);
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "resolved": cmd.is_some(),
                "command": cmd,
            }))?
        ),
        _ => match &cmd {
            Some(c) => println!("{c}"),
            None => println!("unresolved"),
        },
    }
    Ok(())
}

fn run_command(
    root: &Path,
    configs: &Configs,
    format: OutputFormat,
    item: Option<String>,
    project: Option<String>,
) -> Result<()> {
    // `--item` is resolved in this same project, so the command always comes
    // from the project the item lives in.
    let project = paths::resolve_project(project, configs.merged)?;
    let Some(ResolvedValue { value: cmd, source }) = configured_command(&project, configs)? else {
        emit(format, false, None, None, None)?;
        std::process::exit(EXIT_UNRESOLVED);
    };
    // `docs/verify-gate.md` § 6: a multi-line value is refused rather than
    // guessed at. rdm never decomposes the command — ordering, parallelism and
    // output formatting belong to whatever task runner it invokes — so a value
    // that looks like a script is a configuration mistake, not a shell to run.
    if cmd.contains('\n') {
        let fix = match source {
            ConfigSource::Project => format!(
                "set that as the value: rdm config set dispatch.verify \"bash scripts/ci.sh\" --project {project}"
            ),
            ConfigSource::Env => {
                "set RDM_DISPATCH_VERIFY to that single command (e.g. 'bash scripts/ci.sh')"
                    .to_string()
            }
            _ => "set that as the value: rdm config set dispatch.verify \"bash scripts/ci.sh\""
                .to_string(),
        };
        bail!(
            "'dispatch.verify' is multi-line, which rdm refuses to run — it executes ONE command \
             and never decomposes it. Put the steps in a script (e.g. 'bash scripts/ci.sh') and \
             {fix}"
        );
    }

    let dir = match item {
        Some(raw) => match item_worktree(root, &project, &raw) {
            Ok(dir) => dir,
            Err(e) => {
                // Mirror `main.rs`'s top-level error formatting so the
                // actionable `rdm worktree add <item>` message a caller
                // greps stderr for is preserved verbatim.
                eprintln!("error: {e:#}");
                // `resolved: true` because the command IS configured — only
                // the checkout is unknown. Combined with a null `exit` this
                // is the same payload shape `EXIT_UNRUNNABLE` produces for a
                // different reason (a signal-killed command); the exit
                // *code* (1 vs 3), not the payload, is what disambiguates
                // "ran and failed to finish" from "never ran at all".
                emit(format, true, Some(&cmd), None, None)?;
                std::process::exit(EXIT_ITEM_UNRESOLVED);
            }
        },
        None => std::env::current_dir().context("cannot determine current directory")?,
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(&dir)
        .output()
        .with_context(|| {
            format!(
                "failed to run the configured verification command in {}",
                dir.display()
            )
        })?;

    // Merge stdout and stderr: a verification command's diagnosis routinely
    // lands on stderr, and a caller reading one stream would miss it.
    let mut merged = String::from_utf8_lossy(&output.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&output.stderr));
    let tail = tail_of(&merged);

    let code = output.status.code();
    emit(format, true, Some(&cmd), code, Some(&tail))?;
    std::process::exit(code.unwrap_or(EXIT_UNRUNNABLE));
}

/// The last [`TAIL_LIMIT`] characters of `text`, split on a character boundary.
fn tail_of(text: &str) -> String {
    let count = text.chars().count();
    if count <= TAIL_LIMIT {
        return text.to_string();
    }
    text.chars().skip(count - TAIL_LIMIT).collect()
}

/// The actionable message naming the grammar `--item` accepts, for a
/// reference that names no worktree at all: either `raw` parsed cleanly as
/// `plan/<slug>` or `change/<sha>` (neither is a worktree kind), or it
/// looked like a malformed `roadmap`/`phase`-prefixed reference (see
/// [`looks_like_malformed_roadmap_or_phase_ref`]) whose normal resolution
/// then genuinely failed.
fn no_worktree_grammar_error(raw: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "'{raw}' names no worktree — --item accepts <roadmap>, <roadmap>/<phase>, \
         task/<slug>, roadmap/<slug>, or phase/<roadmap>/<stem>"
    )
}

/// True when `raw` looks like a malformed kind-prefixed `roadmap`/`phase`
/// reference: its leading segment (before the first `/`) parses as
/// [`rdm_core::model::ReviewTargetKind::Roadmap`] or
/// [`rdm_core::model::ReviewTargetKind::Phase`], but the full string does
/// **not** parse as a well-formed `rdm_core::model::ReviewTarget` — the
/// shape a truncated reference like `phase/<roadmap>` (stem omitted) or
/// `roadmap/a/b` (extra segment) takes.
///
/// `roadmap` and `phase` are deliberately excluded from the "refuse up
/// front" treatment `plan`/`change` get in [`normalize_item_grammar`]:
/// unlike `task`, `plan`, `src`, and `change`
/// (`rdm_core::link::RESERVED_ROADMAP_SLUGS`), `roadmap` and `phase` are
/// **not** reserved roadmap slugs, so a roadmap literally named `roadmap`
/// or `phase` must still resolve through the ordinary `<roadmap>/<stem>`
/// grammar `rdm_git::worktree::ItemRef::parse` implements. This predicate
/// exists only so [`item_worktree`] can replace the resulting garbled
/// nested error (e.g. "phase 'auth' of roadmap 'phase' not found") with an
/// actionable one naming the accepted grammar — and only once that ordinary
/// resolution has actually failed, never up front.
fn looks_like_malformed_roadmap_or_phase_ref(raw: &str) -> bool {
    if raw.parse::<rdm_core::model::ReviewTarget>().is_ok() {
        return false;
    }
    let prefix = raw.split('/').next().unwrap_or(raw);
    matches!(
        prefix.parse::<rdm_core::model::ReviewTargetKind>(),
        Ok(rdm_core::model::ReviewTargetKind::Roadmap | rdm_core::model::ReviewTargetKind::Phase)
    )
}

/// True when a roadmap literally named `roadmap` exists in `project` AND
/// `identifier` resolves as one of ITS phases (by stem or number).
///
/// This is the narrow condition under which [`normalize_item_grammar`]
/// prefers reading `roadmap/<identifier>` as `phase/roadmap/<identifier>`
/// instead of an ordinary roadmap reference. It is never true merely because
/// a roadmap named `roadmap` exists: an ordinary `roadmap/<slug>` reference
/// to a *different* roadmap must keep resolving as that roadmap even in a
/// project that also happens to have one literally named `roadmap` — only an
/// `identifier` that is actually a phase of it flips the reading.
fn roadmap_named_roadmap_has_phase(
    store: &impl rdm_core::store::Store,
    project: &str,
    identifier: &str,
) -> bool {
    const LITERAL_ROADMAP_SLUG: &str = "roadmap";
    if rdm_core::io::load_roadmap(store, project, LITERAL_ROADMAP_SLUG).is_err() {
        return false;
    }
    let Ok(stem) =
        rdm_core::ops::phase::resolve_phase_stem(store, project, LITERAL_ROADMAP_SLUG, identifier)
    else {
        return false;
    };
    rdm_core::io::load_phase(store, project, LITERAL_ROADMAP_SLUG, &stem).is_ok()
}

/// Rewrites `raw` from the kind-prefixed reference grammar
/// (`roadmap/<slug>`, `phase/<roadmap>/<stem>`, `task/<slug>`) that
/// `--on`/`--implements` use into the unprefixed worktree grammar
/// (`<roadmap>`, `<roadmap>/<stem>`, `task/<slug>`) that
/// `rdm_git::worktree::ItemRef::parse` accepts.
///
/// `plan/<slug>` and `change/<sha>` name no worktree and are refused here,
/// up front, naming the grammar `--item` actually accepts — both when they
/// parse cleanly and when they don't (e.g. `plan/a/b`, with an extra
/// segment): a malformed reference under either kind still names no
/// worktree, however it's spelled. The same is true of a malformed
/// `task/<slug>` reference (e.g. `task/a/b`): `task` is reserved too
/// (`rdm_core::link::is_reserved_roadmap_slug`), so it is refused up front
/// rather than silently reinterpreted. None of the three may be handed to
/// `ItemRef::parse` unchanged: it has no `plan`/`change` case and does not
/// validate a task slug's shape, so `plan/foo` would silently become phase
/// `foo` of a roadmap literally named `plan`, and `task/a/b` would become a
/// task whose slug is literally `a/b`, resolving (or failing to) as that
/// item instead of being refused as the non-worktree reference it is.
///
/// `roadmap` and `phase` are handled differently on a parse failure,
/// because — unlike `task`/`plan`/`change` — neither is a reserved roadmap
/// slug (see [`looks_like_malformed_roadmap_or_phase_ref`]): a malformed
/// `roadmap`/`phase`-prefixed reference is returned unchanged here, to fall
/// through to `ItemRef::parse`'s own resolution, and only replaced with an
/// actionable message by [`item_worktree`] if that resolution then fails
/// **and** the prefix does not itself name an existing roadmap in the
/// project (a roadmap literally named `roadmap` or `phase` must still
/// resolve its own unknown-stem errors normally).
///
/// A WELL-FORMED `roadmap/<rest>` reference is a separate case from that
/// fallthrough. `ReviewTarget::from_str` accepts it whenever `rest` has no
/// `/`, so ordinarily it is read as "the roadmap named `<rest>`" and `rest`
/// alone is returned. But a roadmap literally named `roadmap` (legal — it is
/// not in `rdm_core::link::RESERVED_ROADMAP_SLUGS`) then has no OTHER
/// two-segment spelling for its own phases: `roadmap/<stem>` always reads as
/// "the roadmap named `<stem>`" first. [`roadmap_named_roadmap_has_phase`]
/// is consulted here so that when `rest` genuinely resolves as a phase of a
/// roadmap named `roadmap`, `phase/roadmap/<rest>` is preferred over an
/// ordinary roadmap reference — and left alone (today's behavior) otherwise,
/// so an ordinary `roadmap/<slug>` reference to any other roadmap is
/// unaffected even in a project that also has one literally named `roadmap`.
///
/// Anything else that fails to parse as a `rdm_core::model::ReviewTarget`
/// at all — including the unprefixed `<roadmap>/<stem>` form, which has no
/// kind keyword — is also returned unchanged.
///
/// # Errors
///
/// Returns an error naming the accepted `--item` grammar when `raw` parses,
/// or looks like it was meant to parse, as `plan/<slug>` or `change/<sha>`.
fn normalize_item_grammar(
    raw: &str,
    store: &impl rdm_core::store::Store,
    project: &str,
) -> Result<String> {
    match raw.parse::<rdm_core::model::ReviewTarget>() {
        Ok(rdm_core::model::ReviewTarget::Roadmap { roadmap }) => {
            if roadmap_named_roadmap_has_phase(store, project, &roadmap) {
                Ok(format!("roadmap/{roadmap}"))
            } else {
                Ok(roadmap)
            }
        }
        Ok(rdm_core::model::ReviewTarget::Phase { roadmap, stem }) => {
            Ok(format!("{roadmap}/{stem}"))
        }
        Ok(rdm_core::model::ReviewTarget::Task { slug }) => Ok(format!("task/{slug}")),
        Ok(
            rdm_core::model::ReviewTarget::Plan { .. }
            | rdm_core::model::ReviewTarget::Change { .. },
        ) => Err(no_worktree_grammar_error(raw)),
        Err(_) => {
            let prefix = raw.split('/').next().unwrap_or(raw);
            match prefix.parse::<rdm_core::model::ReviewTargetKind>() {
                // Decide refuse-up-front vs fall-through by the same
                // reserved-slug set `rdm_core::link` enforces everywhere
                // else (`task`, `plan`, `src`, `change`), not a hand-picked
                // subset of `ReviewTargetKind` — so a malformed reference
                // under any reserved kind (`task/a/b` included) gets the
                // actionable grammar message instead of being silently
                // misread by `ItemRef::parse`.
                Ok(kind) if rdm_core::link::is_reserved_roadmap_slug(&kind.to_string()) => {
                    Err(no_worktree_grammar_error(raw))
                }
                // `Roadmap`/`Phase` (not reserved — fall through unchanged)
                // or an unrecognized prefix (the unprefixed form).
                _ => Ok(raw.to_string()),
            }
        }
    }
}

/// Resolves the worktree directory of `raw`.
///
/// Accepts both the unprefixed `rdm worktree add` grammar (`<roadmap>`,
/// `<roadmap>/<phase>`, `task/<slug>`) and the kind-prefixed
/// `roadmap/<slug>` / `phase/<roadmap>/<stem>` / `task/<slug>` grammar that
/// `--on`/`--implements` use, via [`normalize_item_grammar`], which also
/// refuses `plan/<slug>` and `change/<sha>` up front (neither names a
/// worktree).
///
/// A phase always resolves to its roadmap's shared worktree — never an
/// obsolete per-phase checkout — by routing through
/// [`rdm_git::worktree::registered_worktree_for`], the single existing
/// "which registered worktree serves this item?" policy
/// `GitWorktreeProbe::candidates`/`review_source` already share for the
/// `reviewed` gate.
///
/// # Errors
///
/// Returns an error when `raw` names no worktree grammar (`plan/<slug>`,
/// `change/<sha>`, or a malformed `roadmap`/`phase`-prefixed reference such
/// as `phase/<roadmap>` with the stem omitted — see
/// [`looks_like_malformed_roadmap_or_phase_ref`] — but only when the leading
/// segment does not itself name an existing roadmap in `project`; when it
/// does, e.g. `phase/<stem>` against a roadmap literally named `phase`, the
/// real "unknown phase" error from resolution is surfaced instead), when it
/// does not resolve to a known plan item, or when no worktree is registered
/// for the resolved item.
fn item_worktree(root: &Path, project: &str, raw: &str) -> Result<std::path::PathBuf> {
    use rdm_git::worktree;
    let store = commands::make_store(root)?;
    let normalized = normalize_item_grammar(raw, &store, project)?;
    let item = match worktree::resolve_item(&store, project, &normalized) {
        Ok(item) => item,
        Err(e) => {
            // A malformed `roadmap`/`phase`-prefixed reference falls
            // through `normalize_item_grammar` unchanged (neither kind is
            // reserved — see `looks_like_malformed_roadmap_or_phase_ref`),
            // so it reaches `ItemRef::parse` and is misread as a phase of a
            // roadmap literally named `phase`/`roadmap`. Only replace that
            // garbled nested error with the actionable grammar message once
            // resolution has genuinely failed — a real roadmap named
            // `phase`/`roadmap` must still resolve normally. But even then,
            // if the leading segment names an *existing* roadmap in this
            // project (checked with the same plan-repo lookup
            // `resolve_item` itself uses), the reference is well-formed for
            // that roadmap — just an unknown phase stem within it — so the
            // real, actionable error from `resolve_item` must be surfaced
            // instead of the generic grammar message.
            let prefix_names_existing_roadmap = raw
                .split_once('/')
                .map(|(prefix, _)| prefix)
                .is_some_and(|prefix| rdm_core::io::load_roadmap(&store, project, prefix).is_ok());
            return Err(
                if looks_like_malformed_roadmap_or_phase_ref(raw) && !prefix_names_existing_roadmap
                {
                    no_worktree_grammar_error(raw)
                } else {
                    anyhow::anyhow!("{e}").context(format!("cannot resolve item '{raw}'"))
                },
            );
        }
    };
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let repo =
        worktree::discover_distinct_project_repo(&cwd, root).map_err(|e| anyhow::anyhow!("{e}"))?;
    // The single existing "which registered worktree serves this item?"
    // policy (`rdm_core::worktree::review_worktree_item` composed with
    // `worktree::list`), shared with `GitWorktreeProbe`/`review_source`
    // rather than re-derived here. Every `ItemRef` variant yields a key, but
    // fall back to the item's own canonical string rather than panic if
    // that ever stops being true — this value is used for nothing but
    // interpolating into a "no worktree" message.
    let key = rdm_core::worktree::review_worktree_item(&item.as_review_target())
        .unwrap_or_else(|| item.canonical());
    worktree::registered_worktree_for(&repo, &item)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .map(|w| w.path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no rdm worktree for '{key}' — create one with `rdm worktree add {key}`, \
                 or omit --item to run in the current directory"
            )
        })
}

/// Prints the result payload in the requested format.
fn emit(
    format: OutputFormat,
    resolved: bool,
    command: Option<&str>,
    exit: Option<i32>,
    tail: Option<&str>,
) -> Result<()> {
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "resolved": resolved,
                "command": command,
                "exit": exit,
                "tail": tail,
            }))?
        ),
        _ => {
            if !resolved {
                println!("unresolved");
                return Ok(());
            }
            if let Some(t) = tail
                && !t.is_empty()
            {
                print!("{t}");
                if !t.ends_with('\n') {
                    println!();
                }
            }
            match exit {
                Some(0) => println!("verify: passed ({})", command.unwrap_or("")),
                Some(c) => println!("verify: FAILED with exit {c} ({})", command.unwrap_or("")),
                // `exit: None` with `resolved: true` covers two distinct
                // causes (a signal-killed command, or the item never
                // resolving to a worktree at all) that the exit *code*
                // disambiguates, not this payload shape — so the text here
                // stays cause-agnostic rather than claiming a signal.
                None => println!(
                    "verify: FAILED — the command was not run to completion ({})",
                    command.unwrap_or("")
                ),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_last_characters_not_the_first() {
        let text = format!("START{}END", "x".repeat(TAIL_LIMIT));
        let tail = tail_of(&text);
        assert_eq!(tail.chars().count(), TAIL_LIMIT);
        assert!(tail.ends_with("END"), "the tail must be the END of output");
        assert!(!tail.contains("START"), "the head must be dropped");
    }

    #[test]
    fn tail_passes_short_output_through_unchanged() {
        assert_eq!(tail_of("hello\n"), "hello\n");
        assert_eq!(tail_of(""), "");
    }

    /// An empty store: `normalize_item_grammar`'s new roadmap-named-`roadmap`
    /// carve-out consults the store only inside `roadmap_named_roadmap_has_phase`,
    /// which returns `false` the moment `load_roadmap` finds nothing — so every
    /// PURE grammar test below (asserting today's unaffected behavior) can use
    /// this with no seeding at all.
    fn no_roadmap_store() -> rdm_core::store::MemoryStore {
        rdm_core::store::MemoryStore::new()
    }

    const PROJ: &str = "demo";

    #[test]
    fn normalize_item_grammar_rewrites_the_three_worktree_kinds() {
        let store = no_roadmap_store();
        assert_eq!(
            normalize_item_grammar("roadmap/auth", &store, PROJ).unwrap(),
            "auth"
        );
        assert_eq!(
            normalize_item_grammar("phase/auth/phase-1-design", &store, PROJ).unwrap(),
            "auth/phase-1-design"
        );
        assert_eq!(
            normalize_item_grammar("task/fix-bug", &store, PROJ).unwrap(),
            "task/fix-bug"
        );
    }

    #[test]
    fn normalize_item_grammar_passes_through_the_unprefixed_form_unchanged() {
        // No kind keyword before the first `/` — not a `ReviewTarget` at
        // all — so it must fall through unchanged to `ItemRef::parse`.
        let store = no_roadmap_store();
        assert_eq!(
            normalize_item_grammar("auth/phase-1-design", &store, PROJ).unwrap(),
            "auth/phase-1-design"
        );
        assert_eq!(
            normalize_item_grammar("auth", &store, PROJ).unwrap(),
            "auth"
        );
    }

    #[test]
    fn normalize_item_grammar_refuses_plan_and_change_naming_the_grammar() {
        let store = no_roadmap_store();
        let err = normalize_item_grammar("plan/foo", &store, PROJ)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("names no worktree"),
            "must say the reference names no worktree: {err}"
        );
        assert!(
            err.contains("<roadmap>") && err.contains("task/<slug>"),
            "must name the accepted grammar: {err}"
        );

        let err = normalize_item_grammar("change/HEAD", &store, PROJ)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("names no worktree"),
            "must say the reference names no worktree: {err}"
        );
    }

    #[test]
    fn normalize_item_grammar_refuses_malformed_plan_and_change_too() {
        // A malformed reference under a reserved kind (`plan`/`change`)
        // still names no worktree, however it's spelled — refused up front
        // just like the well-formed case, not silently passed through.
        let store = no_roadmap_store();
        let err = normalize_item_grammar("plan/a/b", &store, PROJ)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("names no worktree"),
            "must say the reference names no worktree: {err}"
        );

        let err = normalize_item_grammar("change/a/b", &store, PROJ)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("names no worktree"),
            "must say the reference names no worktree: {err}"
        );
    }

    #[test]
    fn normalize_item_grammar_refuses_malformed_task_too() {
        // `task` is a reserved roadmap slug just like `plan`/`change`, so a
        // malformed `task/<slug>` reference (an extra segment) must be
        // refused up front here rather than silently reinterpreted by
        // `ItemRef::parse` as a task whose slug is literally `a/b`.
        let store = no_roadmap_store();
        let err = normalize_item_grammar("task/a/b", &store, PROJ)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("names no worktree"),
            "must say the reference names no worktree: {err}"
        );
    }

    #[test]
    fn normalize_item_grammar_passes_through_a_malformed_roadmap_or_phase_ref_unchanged() {
        // `roadmap`/`phase` are not reserved roadmap slugs (unlike
        // `task`/`plan`/`src`/`change`), so a malformed one must NOT be
        // refused up front here — a roadmap literally named `roadmap` or
        // `phase` still has to resolve through the ordinary grammar.
        // `item_worktree` is what replaces the garbled nested error with an
        // actionable one, and only once real resolution has failed.
        let store = no_roadmap_store();
        assert_eq!(
            normalize_item_grammar("phase/auth", &store, PROJ).unwrap(),
            "phase/auth"
        );
        assert_eq!(
            normalize_item_grammar("roadmap/a/b", &store, PROJ).unwrap(),
            "roadmap/a/b"
        );
    }

    #[test]
    fn normalize_item_grammar_prefers_a_roadmap_literally_named_roadmap_own_phase() {
        // A project roadmap literally named `roadmap` (legal — `roadmap` is
        // not in RESERVED_ROADMAP_SLUGS) with a real phase `design`. The
        // well-formed two-segment reference `roadmap/design` parses cleanly
        // as `ReviewTarget::Roadmap { roadmap: "design" }` (model.rs), so
        // without the carve-out this always collapsed to the bare roadmap
        // slug `design` — losing the literal `roadmap` prefix — and resolved
        // (or failed to) as a roadmap named `design` instead of the phase.
        let mut store = rdm_core::store::MemoryStore::new();
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, PROJ, "Demo").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: PROJ,
                slug: "roadmap",
                title: "Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: PROJ,
                roadmap: "roadmap",
                slug: "design",
                title: "Design",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            normalize_item_grammar("roadmap/phase-1-design", &store, PROJ).unwrap(),
            "roadmap/phase-1-design",
            "must prefer the <roadmap>/<stem> phase reading, not collapse to a bare roadmap slug"
        );
        // The numeric stem form must resolve too — `resolve_phase_stem`
        // handles the digit-to-stem translation inside the carve-out check.
        assert_eq!(
            normalize_item_grammar("roadmap/1", &store, PROJ).unwrap(),
            "roadmap/1",
            "a numeric identifier that resolves as a real phase must also take the phase reading"
        );

        // An identifier that is NOT a real phase of the `roadmap`-named
        // roadmap must fall back to today's behavior — an ordinary roadmap
        // reference — so this never over-fires for an unrelated slug.
        assert_eq!(
            normalize_item_grammar("roadmap/no-such-phase", &store, PROJ).unwrap(),
            "no-such-phase",
            "an identifier that is not a real phase must still read as an ordinary roadmap reference"
        );
    }

    #[test]
    fn normalize_item_grammar_still_resolves_an_ordinary_roadmap_even_when_roadmap_named_roadmap_exists()
     {
        // Backward compatibility: a project with BOTH a roadmap literally
        // named `roadmap` (with its own phase) AND an ordinary roadmap named
        // `auth`. `roadmap/auth` must still read as "the roadmap named
        // `auth`", never as a phase of the `roadmap`-named roadmap — `auth`
        // is not one of its phases, so the carve-out must not fire.
        let mut store = rdm_core::store::MemoryStore::new();
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, PROJ, "Demo").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: PROJ,
                slug: "roadmap",
                title: "Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: PROJ,
                roadmap: "roadmap",
                slug: "design",
                title: "Design",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: PROJ,
                slug: "auth",
                title: "Auth",
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            normalize_item_grammar("roadmap/auth", &store, PROJ).unwrap(),
            "auth",
            "an ordinary roadmap/<slug> reference to a DIFFERENT roadmap must keep working"
        );
    }

    #[test]
    fn looks_like_malformed_roadmap_or_phase_ref_flags_only_the_truncated_shapes() {
        assert!(looks_like_malformed_roadmap_or_phase_ref("phase/auth"));
        assert!(looks_like_malformed_roadmap_or_phase_ref("roadmap/a/b"));
        // Well-formed kind-prefixed references are not malformed.
        assert!(!looks_like_malformed_roadmap_or_phase_ref(
            "phase/auth/phase-1-design"
        ));
        assert!(!looks_like_malformed_roadmap_or_phase_ref("roadmap/auth"));
        // The unprefixed form has no kind keyword at all.
        assert!(!looks_like_malformed_roadmap_or_phase_ref(
            "auth/phase-1-design"
        ));
        assert!(!looks_like_malformed_roadmap_or_phase_ref("auth"));
        // `task`/`plan`/`change` are handled elsewhere, not by this
        // predicate, even when malformed.
        assert!(!looks_like_malformed_roadmap_or_phase_ref("task/a/b"));
        assert!(!looks_like_malformed_roadmap_or_phase_ref("plan/a/b"));
    }

    #[test]
    fn tail_splits_on_a_character_boundary() {
        // Multi-byte characters: truncating by bytes would panic or produce
        // invalid UTF-8; truncating by chars must not.
        let text = "é".repeat(TAIL_LIMIT + 10);
        let tail = tail_of(&text);
        assert_eq!(tail.chars().count(), TAIL_LIMIT);
    }
}
