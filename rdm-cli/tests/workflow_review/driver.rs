//! The code engine's source-bound standalone path, driven and then executed.
//!
//! Executes the real `.claude/workflows/rdm-wf-review-refute-fix.js` driver
//! (loaded with `Host::load_driver`) under a Rust-scripted reviewer fleet,
//! then runs the `gateScript` / `persistScript` it returns under `/bin/bash`
//! (the system bash — bash 3.2 on macOS) against `CARGO_BIN_EXE_rdm` in a
//! per-test plan repo, and asserts on the plan state read back through the
//! binary. The lib cases import `.claude/workflows/lib/review.mjs`.
//!
//! Each test owns its fixture: a plan repo whose default project is the
//! decoy (so a dropped `--project` misses), project `rev-verify` with roadmap
//! `rm-rev` (phases `phase-1-clean`, `phase-2-dirty`), task
//! `standalone-task` and plan `rev-verify-plan`; a source repo with the
//! roadmap and task worktrees registered through `rdm worktree add` and one
//! pinned commit on each. Scripts run with `PATH=/usr/bin:/bin`, so only the
//! binary the engine was handed can run. Ported from
//! `scripts/lib/review-driver.test.mjs`; see
//! `docs/test-migration-inventory.md` § "Phase 3".

use std::path::{Path, PathBuf};
use std::time::Duration;

use rdm_core::anchor::QuoteOccurrence;
use rdm_core::error::Error as CoreError;
use rdm_core::source::SourceObjectKind;
use rdm_devtools::workflow::Host;
use serde_json::{Value, json};

use crate::plan_fixture::{
    OpenStdin, Pin, PlanRepo, Run, Shell, SourceRepo, branch, commit_file, pin, rdm_bin, rev,
    review_id,
};
use crate::support::{
    Agent, Failure, Js, Lib, Outcome, REVIEW_ENGINE, Reply, infra, run_driver, run_real,
};

const PROJECT: &str = "rev-verify";
const ROADMAP: &str = "rm-rev";
const TASK: &str = "standalone-task";
const PLAN: &str = "rev-verify-plan";
const AC_BODY: &str = "## Acceptance criteria\n\n- AC1: it works";
/// Finding dimensions a planted finding list is spread across, in order.
const DIMS: [&str; 6] = [
    "correctness",
    "tests",
    "architecture",
    "api-docs",
    "changelog",
    "security",
];

struct Fx {
    repo: PlanRepo,
    roadmap_pin: Pin,
    task_pin: Pin,
}

impl Fx {
    fn roadmap_wt(&self) -> PathBuf {
        PathBuf::from(&self.roadmap_pin.source)
    }
}

fn phase_seed(slug: &'static str, n: &'static str) -> Vec<&'static str> {
    vec![
        "phase",
        "create",
        slug,
        "--title",
        slug,
        "--number",
        n,
        "--body",
        AC_BODY,
        "--no-edit",
        "--roadmap",
        ROADMAP,
        "--project",
        PROJECT,
    ]
}

fn fixture() -> Result<Fx, Failure> {
    let repo = PlanRepo::with_decoy(&[PROJECT])?;
    let implements = format!("phase/{ROADMAP}/phase-2-dirty");
    repo.seed(&[
        &[
            "roadmap",
            "create",
            ROADMAP,
            "--title",
            "Review RM",
            "--body",
            "seed",
            "--no-edit",
            "--project",
            PROJECT,
        ],
        &phase_seed("clean", "1"),
        &phase_seed("dirty", "2"),
        &[
            "task",
            "create",
            TASK,
            "--title",
            "Standalone",
            "--body",
            AC_BODY,
            "--no-edit",
            "--project",
            PROJECT,
        ],
        &[
            "plan",
            "create",
            PLAN,
            "--title",
            "Review Verify Plan",
            "--implements",
            &implements,
            "--body",
            "the approved plan",
            "--no-edit",
            "--project",
            PROJECT,
        ],
    ])?;
    let src = SourceRepo::init(&repo)?;
    let rw = src.add_worktree(&repo, ROADMAP, PROJECT)?;
    let roadmap_pin = pin(&rw, &["roadmap-work.txt", "dir/roadmap-work.txt"])?;
    let tw = src.add_worktree(&repo, &format!("task/{TASK}"), PROJECT)?;
    let task_pin = pin(&tw, &["task-work.txt"])?;
    Ok(Fx {
        repo,
        roadmap_pin,
        task_pin,
    })
}

/// Shallow-merges objects left to right.
fn merge(parts: &[Value]) -> Value {
    let mut out = serde_json::Map::new();
    for p in parts {
        if let Some(o) = p.as_object() {
            for (k, v) in o {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// The engine arguments every source-bound case shares, plus `extra`.
fn args(pin: &Pin, extra: Value) -> Value {
    merge(&[
        json!({ "mode": "code", "rdmBin": rdm_bin(), "project": PROJECT, "gate": true }),
        pin.args(),
        extra,
    ])
}

/// Phase-target arguments for `phase` on the roadmap pin.
fn phase_args(fx: &Fx, phase: &str, extra: Value) -> Value {
    args(
        &fx.roadmap_pin,
        merge(&[json!({ "roadmap": ROADMAP, "phase": phase }), extra]),
    )
}

/// Persist-only arguments naming the approved plan, on `phase-2-dirty`.
fn persist_args(pin: &Pin) -> Value {
    args(
        pin,
        json!({ "gate": false, "persist": true, "implements": format!("plan/{PLAN}"),
                "roadmap": ROADMAP, "phase": "phase-2-dirty" }),
    )
}

/// A fleet whose `ac` finder reports one PASS row, whose other finders each
/// plant the next of `findings` (spread across [`DIMS`]), and whose refuters
/// never refute.
fn fleet(findings: Vec<Value>) -> Agent {
    Agent::scripted(move |call| {
        let label = call.label.as_str();
        if label == "find:code:ac" {
            return Reply::Value(json!({
                "ac": [{ "criterion": "AC1: it works", "status": "PASS", "evidence": "the tests cover it" }],
                "findings": []
            }));
        }
        for (dim, f) in DIMS.iter().zip(&findings) {
            if label == format!("find:code:{dim}") {
                return Reply::Value(json!({ "findings": [f] }));
            }
        }
        if label.starts_with("refute:") {
            return Reply::Value(json!({ "refuted": false, "confidence": 95 }));
        }
        Reply::Value(json!({ "findings": [] }))
    })
}

/// Runs the engine with `args` under [`fleet`]; a throw is a failure.
fn drive(lib: &Lib, args: Value, findings: Vec<Value>) -> Result<(Value, Agent), Failure> {
    let agent = fleet(findings);
    let out = run_driver(lib, REVIEW_ENGINE, args, &agent)?.map_err(Failure::Js)?;
    Ok((out, agent))
}

fn strict() -> Run {
    Run::bare().shell(Shell::BashStrict)
}

fn plain() -> Run {
    Run::bare().shell(Shell::Bash)
}

fn phase_status(fx: &Fx, stem: &str) -> Result<Value, Failure> {
    Ok(fx.repo.phase(PROJECT, ROADMAP, stem)?["status"].clone())
}

fn task_status(fx: &Fx) -> Result<Value, Failure> {
    Ok(fx.repo.task(PROJECT, TASK)?["status"].clone())
}

/// Runs the returned persist ladder to success and returns `(review, stdout)`.
fn land(fx: &Fx, result: &Value) -> Result<(Value, String), Failure> {
    let script = result["persistScript"]
        .as_str()
        .ok_or_else(|| Failure::Check(format!("a persist:true run emits a ladder: {result}")))?;
    let out = fx.repo.run_ok(&strict(), script)?;
    let id = review_id(&out)
        .ok_or_else(|| Failure::Check(format!("the ladder prints the id it created: {out}")))?;
    Ok((fx.repo.review(PROJECT, &id)?, out))
}

fn has_line(out: &str, line: &str) -> bool {
    out.lines().any(|l| l == line)
}

fn comments(review: &Value) -> Vec<Value> {
    review["comments"].as_array().cloned().unwrap_or_default()
}

/// The comment whose body carries `needle` (a finding id the test planted).
fn comment_with(review: &Value, needle: &str) -> Result<Value, Failure> {
    comments(review)
        .into_iter()
        .find(|c| c["body"].as_str().is_some_and(|b| b.contains(needle)))
        .ok_or_else(|| Failure::Check(format!("no comment carries {needle}: {review}")))
}

/// The ladder's own degradation note comment, if any.
fn note(review: &Value) -> Result<Value, Failure> {
    comments(review)
        .into_iter()
        .find(|c| {
            c["body"]
                .as_str()
                .is_some_and(|b| b.starts_with("persist-note: anchors-degraded"))
        })
        .ok_or_else(|| Failure::Check(format!("the review carries a degradation note: {review}")))
}

fn no_anchor(c: &Value) -> bool {
    c.get("anchor").is_none_or(Value::is_null)
}

/// The `anchor` header the persisted comment body parses back to.
fn header_anchor(js: &mut Js, c: &Value) -> Result<Value, Failure> {
    let h = js.call("parseCommentHeader", vec![c["body"].clone()])?;
    Ok(h.get("anchor").cloned().unwrap_or(Value::Null))
}

fn note_body(js: &mut Js, degraded: u32, requested: u32) -> Result<Value, Failure> {
    js.call(
        "persistDegradationNoteBody",
        vec![json!(degraded), json!(requested)],
    )
}

fn finding(id: &str, severity: &str, extra: Value) -> Value {
    merge(&[
        json!({ "id": id, "concern": "correctness", "severity": severity, "confidence": 90,
                "what_fails": format!("{id} fails") }),
        extra,
    ])
}

/// Commits on `wt`'s branch: `before` ends at the pinned base, `after` at the
/// pinned head.
fn range(wt: &Path, before: &[(&str, &str)], after: &[(&str, &str)]) -> Pin {
    for (f, c) in before {
        commit_file(wt, f, c, &format!("feat: {f}"));
    }
    let base = rev(wt, "HEAD");
    for (f, c) in after {
        commit_file(wt, f, c, &format!("feat: change {f}"));
    }
    Pin {
        source: wt.to_string_lossy().into_owned(),
        base,
        expected_head: rev(wt, "HEAD"),
        expected_branch: branch(wt),
    }
}

fn make_executable(path: &Path) -> Outcome {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(infra)
}

// --- Gate ladders ------------------------------------------------------------

#[test]
fn clean_review_gates_phase_to_reviewed() {
    run_real(|lib| {
        let fx = fixture()?;
        let (r, _) = drive(lib, phase_args(&fx, "phase-1-clean", json!({})), vec![])?;
        check_eq!(r["outcome"], json!("reviewed"), "a clean review");
        check_eq!(r["status"], json!("reviewed"), "maps to reviewed");
        check_eq!(
            r["writesCompletion"],
            json!(true),
            "a reviewed outcome tells the lander it may write the trailer"
        );
        check_eq!(
            phase_status(&fx, "phase-1-clean")?,
            json!("not-started"),
            "nothing is written before the ladder runs"
        );
        let script = r["gateScript"].as_str().unwrap_or_default();
        check!(!script.is_empty(), "gate:true emits a ladder: {r}");
        fx.repo.run_ok(&strict(), script)?;
        check_eq!(
            phase_status(&fx, "phase-1-clean")?,
            json!("reviewed"),
            "the executed ladder drove the phase to reviewed"
        );
        Ok(())
    });
}

#[test]
fn rework_gates_phase_to_in_progress() {
    run_real(|lib| {
        let fx = fixture()?;
        let (r, _) = drive(
            lib,
            phase_args(&fx, "phase-2-dirty", json!({})),
            vec![finding(
                "real-bug",
                "blocking",
                json!({ "confidence": 95, "what_fails": "it drops a write" }),
            )],
        )?;
        check_eq!(r["outcome"], json!("rework"), "a live blocker reworks");
        check_eq!(
            r["writesCompletion"],
            json!(false),
            "rework never authorises a completion trailer"
        );
        check_eq!(
            r["findings"].as_array().map(Vec::len),
            Some(1),
            "the blocker survived"
        );
        fx.repo
            .run_ok(&strict(), r["gateScript"].as_str().unwrap_or_default())?;
        check_eq!(
            phase_status(&fx, "phase-2-dirty")?,
            json!("in-progress"),
            "the ladder parked the phase for rework"
        );
        Ok(())
    });
}

#[test]
fn task_target_gates_via_task_update() {
    run_real(|lib| {
        let fx = fixture()?;
        let (r, _) = drive(lib, args(&fx.task_pin, json!({ "task": TASK })), vec![])?;
        check_eq!(r["task"], json!(TASK), "a task result names the task");
        check!(
            r.get("roadmap").is_none() && r.get("phase").is_none(),
            "a task result carries no roadmap/phase identity: {r}"
        );
        check_eq!(r["outcome"], json!("reviewed"), "clean");
        check_eq!(task_status(&fx)?, json!("open"), "nothing written yet");
        fx.repo
            .run_ok(&strict(), r["gateScript"].as_str().unwrap_or_default())?;
        check_eq!(
            task_status(&fx)?,
            json!("reviewed"),
            "the ladder drove the task to reviewed"
        );
        Ok(())
    });
}

#[test]
fn refused_gate_write_fails_plain_shell() {
    run_real(|lib| {
        let fx = fixture()?;
        let (r, _) = drive(lib, phase_args(&fx, "phase-1-clean", json!({})), vec![])?;
        // Fault injection: move the branch on, so the pinned head is no longer
        // the checkout's and the binding makes the real binary refuse.
        commit_file(
            &fx.roadmap_wt(),
            "moved-on.txt",
            "later\n",
            "feat: moved on",
        );
        let out = fx
            .repo
            .run(&plain(), r["gateScript"].as_str().unwrap_or_default())?;
        check!(
            !out.status.success(),
            "a refused write fails the ladder in a plain shell, not masked by the trailing read-back"
        );
        check!(
            phase_status(&fx, "phase-1-clean")? != json!("reviewed"),
            "the refused ladder did not stamp reviewed"
        );
        Ok(())
    });
}

#[test]
fn gate_false_writes_nothing() {
    run_real(|lib| {
        let fx = fixture()?;
        let before = fx.repo.snapshot()?;
        let (r, _) = drive(
            lib,
            phase_args(&fx, "phase-1-clean", json!({ "gate": false })),
            vec![],
        )?;
        check_eq!(r["outcome"], json!("reviewed"), "the verdict is returned");
        check!(
            r.get("gateCommands").is_none() && r.get("gateScript").is_none(),
            "no ladder when the caller keeps its own gate: {r}"
        );
        check_eq!(
            fx.repo.snapshot()?,
            before,
            "the plan repo is untouched by the call"
        );
        Ok(())
    });
}

/// Ported from main's review-driver.test.mjs (0f83454): `findEffort` /
/// `verifyEffort` reach every finder and refuter dispatch, and a run without
/// them dispatches no `effort` key at all.
#[test]
fn find_and_verify_effort_reach_every_finder_and_refuter() {
    run_real(|lib| {
        let fx = fixture()?;
        let blocker = || vec![finding("c-1", "blocking", json!({}))];
        let (_, agent) = drive(
            lib,
            phase_args(
                &fx,
                "phase-1-clean",
                json!({ "gate": false, "findModel": "m-find", "findEffort": "low",
                        "verifyModel": "m-verify", "verifyEffort": "high" }),
            ),
            blocker(),
        )?;
        let (finds, refutes) = (agent.calls_with("find:"), agent.calls_with("refute:"));
        check!(
            !finds.is_empty() && !refutes.is_empty(),
            "the engine dispatched finders and refuters: {:?}",
            agent.labels()
        );
        for c in &finds {
            check_eq!(
                (c.opts["model"].clone(), c.opts["effort"].clone()),
                (json!("m-find"), json!("low")),
                "{}",
                c.label
            );
        }
        for c in &refutes {
            check_eq!(
                (c.opts["model"].clone(), c.opts["effort"].clone()),
                (json!("m-verify"), json!("high")),
                "{}",
                c.label
            );
        }

        let (_, bare) = drive(
            lib,
            phase_args(&fx, "phase-1-clean", json!({ "gate": false })),
            blocker(),
        )?;
        check!(!bare.calls().is_empty(), "the bare run dispatched agents");
        for c in bare.calls() {
            check!(
                c.opts.get("effort").is_none(),
                "{} carries no effort key without effort args: {}",
                c.label,
                c.opts
            );
        }
        Ok(())
    });
}

// --- The persist ladder: anchors -------------------------------------------------

#[test]
fn persist_ladder_records_change_review_anchors() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        let (r, _) = drive(
            lib,
            persist_args(&fx.roadmap_pin),
            vec![
                // The explicit `path` field is used directly; the messy
                // `location` defeats pathFromLocation's end-anchored strip.
                finding(
                    "anchored-bug",
                    "blocking",
                    json!({ "confidence": 95, "location": "roadmap-work.txt:1 (mirrored at README.md:9-12)",
                            "path": "roadmap-work.txt", "quote": "shipped" }),
                ),
                // Prose-only location and no path: the requested anchor is
                // dropped at build time.
                finding(
                    "degraded-bug",
                    "concern",
                    json!({ "concern": "tests", "location": "throughout the gate step", "quote": "shipped" }),
                ),
                // No quote: this finding never asked for an anchor.
                finding(
                    "whole-doc-note",
                    "suggestion",
                    json!({ "concern": "architecture", "confidence": 80, "location": "general" }),
                ),
            ],
        )?;
        let (review, out) = land(&fx, &r)?;
        check_eq!(review["state"], json!("submitted"), "submitted");
        check_eq!(
            review["verdict"],
            json!("request-changes"),
            "rework persists request-changes"
        );
        check_eq!(
            review["target"]["kind"],
            json!("change"),
            "the reviewed artifact is the pinned change"
        );
        check_eq!(
            comments(&review).len(),
            4,
            "each survivor once, plus one degradation note"
        );

        let anchored = comment_with(&review, "anchored-bug")?;
        check_eq!(
            anchored["anchor"]["anchor_type"],
            json!("file-quote"),
            "a change review anchors into the source file"
        );
        check_eq!(
            anchored["anchor"]["path"],
            json!("roadmap-work.txt"),
            "the finder's `path` field landed the anchor"
        );
        check_eq!(anchored["anchor"]["quote"], json!("shipped"), "the quote");
        check_eq!(
            header_anchor(&mut js, &anchored)?,
            json!("path"),
            "header-marked path"
        );

        let degraded = comment_with(&review, "degraded-bug")?;
        check!(no_anchor(&degraded), "landed whole-document: {degraded}");
        check_eq!(
            header_anchor(&mut js, &degraded)?,
            json!("degraded"),
            "a dropped anchor is header-marked degraded"
        );

        let whole = comment_with(&review, "whole-doc-note")?;
        check!(no_anchor(&whole), "whole-document by design: {whole}");
        check_eq!(
            header_anchor(&mut js, &whole)?,
            json!("wholeDocumentIntended"),
            "never recorded as a degraded anchor"
        );

        let n = note(&review)?;
        check!(no_anchor(&n), "the note is whole-document");
        check_eq!(
            n["body"],
            note_body(&mut js, 1, 2)?,
            "the note names the real total against what was requested"
        );
        check!(
            review["body"].as_str().is_some_and(|b| b.contains(
                "1 of 2 requested anchor(s) could not be placed at persist time (path-missing x1); see the `anchor` header on each comment."
            )),
            "the review's own summary names the build-time degradation: {}",
            review["body"]
        );
        check_eq!(
            r["persistDegraded"],
            json!({ "requested": 2, "degraded": 1, "all": false }),
            "a partially-degraded review does not set all"
        );
        check!(
            has_line(&out, "anchorsDegraded=partial"),
            "the ladder prints the partial disposition: {out}"
        );
        Ok(())
    });
}

#[test]
fn apostrophe_finding_runs_under_system_bash() {
    run_real(|lib| {
        let fx = fixture()?;
        let text = "it's dropped on the fallback path";
        let (r, _) = drive(
            lib,
            persist_args(&fx.roadmap_pin),
            vec![finding(
                "apostrophe-bug",
                "blocking",
                json!({ "confidence": 95, "what_fails": text, "location": "general" }),
            )],
        )?;
        let (review, _) = land(&fx, &r)?;
        check_eq!(review["state"], json!("submitted"), "submitted");
        let c = comment_with(&review, "apostrophe-bug")?;
        check!(
            c["body"].as_str().is_some_and(|b| b.contains(text)),
            "the apostrophe-bearing text rides through verbatim: {c}"
        );
        Ok(())
    });
}

#[test]
fn path_with_line_suffix_lands_anchor() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        // A slash-bearing suffixed path: accepted whole by the path check, so
        // only the strip makes it name a real file; the prose location cannot
        // rescue a broken strip.
        let (r, _) = drive(
            lib,
            persist_args(&fx.roadmap_pin),
            vec![finding(
                "suffixed-path-bug",
                "blocking",
                json!({ "location": "see the gate step", "path": "dir/roadmap-work.txt:1", "quote": "shipped" }),
            )],
        )?;
        let (review, out) = land(&fx, &r)?;
        check!(
            has_line(&out, "anchorsDegraded=none"),
            "nothing degraded: {out}"
        );
        let cs = comments(&review);
        check_eq!(cs.len(), 1, "one comment");
        check_eq!(
            cs[0]["anchor"]["anchor_type"],
            json!("file-quote"),
            "a real file anchor"
        );
        check_eq!(
            cs[0]["anchor"]["path"],
            json!("dir/roadmap-work.txt"),
            "the `:1` suffix is stripped"
        );
        check_eq!(header_anchor(&mut js, &cs[0])?, json!("path"), "anchored");
        check_eq!(
            r["persistDegraded"],
            json!({ "requested": 1, "degraded": 0, "all": false }),
            "nothing degraded at build time"
        );
        Ok(())
    });
}

// --- Run-time refusals: retried whole-document, classified by cause -------------

/// One finding over a custom range, persisted by the executed ladder; returns
/// `(review, stdout, result)`.
fn refused(
    lib: &Lib,
    fx: &Fx,
    before: &[(&str, &str)],
    after: &[(&str, &str)],
    f: Value,
) -> Result<(Value, String, Value), Failure> {
    let pin = range(&fx.roadmap_wt(), before, after);
    let (r, _) = drive(lib, persist_args(&pin), vec![f])?;
    let (review, out) = land(fx, &r)?;
    Ok((review, out, r))
}

fn assert_whole_doc_degraded(js: &mut Js, review: &Value, id: &str) -> Outcome {
    let c = comment_with(review, id)?;
    check!(
        no_anchor(&c),
        "the refused anchor lands whole-document: {c}"
    );
    check_eq!(
        header_anchor(js, &c)?,
        json!("degraded"),
        "a run-time refusal is header-marked degraded"
    );
    Ok(())
}

#[test]
fn runtime_refusal_outside_hunk_blocking_parks() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        // `--unified=0` hunks: only the second line changes, so the first is
        // outside every hunk.
        let (review, out, r) = refused(
            lib,
            &fx,
            &[("outside-hunk.txt", "keepme\nchangeme\n")],
            &[("outside-hunk.txt", "keepme\nchanged\n")],
            finding(
                "outside-hunk-bug",
                "blocking",
                json!({ "location": "see the gate step", "path": "outside-hunk.txt", "quote": "keepme" }),
            ),
        )?;
        check!(has_line(&out, "anchorsDegraded=all"), "all: {out}");
        check!(
            has_line(&out, "anchorsParkRequired=yes"),
            "a blocking finding losing its anchor requires a park: {out}"
        );
        check_eq!(comments(&review).len(), 2, "the finding plus the note");
        assert_whole_doc_degraded(&mut js, &review, "outside-hunk-bug")?;
        check_eq!(
            note(&review)?["body"],
            note_body(&mut js, 1, 1)?,
            "the note reports the run-time result"
        );
        check_eq!(
            r["persistDegraded"],
            json!({ "requested": 1, "degraded": 0, "all": false }),
            "the build-time preview cannot see a run-time refusal"
        );
        Ok(())
    });
}

#[test]
fn runtime_refusal_outside_hunk_suggestion_no_park() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        let (review, out, _) = refused(
            lib,
            &fx,
            &[("outside-hunk-suggestion.txt", "keepme\nchangeme\n")],
            &[("outside-hunk-suggestion.txt", "keepme\nchanged\n")],
            finding(
                "outside-hunk-suggestion",
                "suggestion",
                json!({ "location": "see the gate step", "path": "outside-hunk-suggestion.txt", "quote": "keepme" }),
            ),
        )?;
        check!(has_line(&out, "anchorsDegraded=all"), "all: {out}");
        check!(
            has_line(&out, "anchorsParkRequired=no"),
            "a non-blocking finding on an untouched line is not a park signal: {out}"
        );
        assert_whole_doc_degraded(&mut js, &review, "outside-hunk-suggestion")
    });
}

#[test]
fn runtime_refusal_quote_not_found_parks() {
    run_real(|lib| {
        let fx = fixture()?;
        let (_, out, _) = refused(
            lib,
            &fx,
            &[("missing-quote.txt", "keepme\nchangeme\n")],
            &[("missing-quote.txt", "keepme\nchanged\n")],
            finding(
                "quote-does-not-exist",
                "concern",
                json!({ "concern": "tests", "confidence": 70, "location": "see the gate step",
                        "path": "missing-quote.txt", "quote": "this text appears nowhere in the file" }),
            ),
        )?;
        check!(
            has_line(&out, "anchorsParkRequired=yes"),
            "a systemic refusal parks regardless of severity: {out}"
        );
        Ok(())
    });
}

#[test]
fn runtime_refusal_spoofed_does_not_touch_parks() {
    run_real(|lib| {
        let fx = fixture()?;
        // The quote itself contains the benign cause's wording and does not
        // exist in the file: the classifier must key on rdm's own message.
        let (_, out, _) = refused(
            lib,
            &fx,
            &[("spoofed-quote.txt", "keepme\nchangeme\n")],
            &[("spoofed-quote.txt", "keepme\nchanged\n")],
            finding(
                "quote-spoofs-the-old-marker",
                "concern",
                json!({ "concern": "tests", "confidence": 70, "location": "see the gate step",
                        "path": "spoofed-quote.txt",
                        "quote": "this code does not touch the validation path at all" }),
            ),
        )?;
        check!(
            has_line(&out, "anchorsParkRequired=yes"),
            "a QuoteNotFound echoing benign-looking text still parks: {out}"
        );
        Ok(())
    });
}

#[test]
fn runtime_refusal_unmodified_file_benign() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        let (review, out, _) = refused(
            lib,
            &fx,
            &[("never-modified.txt", "existing content\n")],
            &[("other-change.txt", "something else entirely\n")],
            finding(
                "whole-file-untouched",
                "suggestion",
                json!({ "location": "see the gate step", "path": "never-modified.txt", "quote": "existing content" }),
            ),
        )?;
        check!(has_line(&out, "anchorsDegraded=all"), "all: {out}");
        check!(
            has_line(&out, "anchorsParkRequired=no"),
            "a real, in-range, untouched file is benign: {out}"
        );
        assert_whole_doc_degraded(&mut js, &review, "whole-file-untouched")
    });
}

#[test]
fn runtime_refusal_path_absent_at_head_parks() {
    run_real(|lib| {
        let fx = fixture()?;
        let (_, out, _) = refused(
            lib,
            &fx,
            &[("real-file.txt", "first line\nsecond line\n")],
            &[("real-file.txt", "first line\nsecond line, changed\n")],
            finding(
                "path-does-not-exist-at-head",
                "concern",
                json!({ "concern": "tests", "confidence": 70, "location": "see the gate step",
                        "path": "this-path-was-never-committed.txt", "quote": "second line" }),
            ),
        )?;
        check!(
            has_line(&out, "anchorsParkRequired=yes"),
            "a path absent at head is systemic: {out}"
        );
        Ok(())
    });
}

#[test]
fn mixed_anchor_run_reports_partial() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        let pin = range(
            &fx.roadmap_wt(),
            &[("untouched.txt", "never touched\n")],
            &[("landed.txt", "shipped-anchor\n")],
        );
        let (r, _) = drive(
            lib,
            persist_args(&pin),
            vec![
                finding(
                    "lands-fine",
                    "blocking",
                    json!({ "location": "see the gate step", "path": "landed.txt", "quote": "shipped-anchor" }),
                ),
                finding(
                    "refused-at-runtime",
                    "concern",
                    json!({ "concern": "tests", "confidence": 85, "location": "see the gate step",
                            "path": "untouched.txt", "quote": "never touched" }),
                ),
            ],
        )?;
        let (review, out) = land(&fx, &r)?;
        check!(
            has_line(&out, "anchorsDegraded=partial"),
            "one landed, one refused: {out}"
        );
        check!(
            has_line(&out, "anchorsParkRequired=no"),
            "the benign untouched-file cause is not a park signal: {out}"
        );
        check_eq!(comments(&review).len(), 3, "two findings plus the note");
        let landed = comment_with(&review, "lands-fine")?;
        check_eq!(
            landed["anchor"]["anchor_type"],
            json!("file-quote"),
            "landed"
        );
        check_eq!(header_anchor(&mut js, &landed)?, json!("path"), "anchored");
        assert_whole_doc_degraded(&mut js, &review, "refused-at-runtime")?;
        check_eq!(note(&review)?["body"], note_body(&mut js, 1, 2)?, "1 of 2");
        check_eq!(
            r["persistDegraded"],
            json!({ "requested": 2, "degraded": 0, "all": false }),
            "the build-time preview sees neither run-time outcome"
        );
        Ok(())
    });
}

#[test]
fn whole_document_by_design_never_degrades() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        let (r, _) = drive(
            lib,
            phase_args(
                &fx,
                "phase-1-clean",
                json!({ "persist": true, "implements": format!("plan/{PLAN}") }),
            ),
            vec![finding(
                "note-only",
                "suggestion",
                json!({ "confidence": 80, "location": "general" }),
            )],
        )?;
        check_eq!(
            r["outcome"],
            json!("reviewed"),
            "a lone suggestion gates cleanly"
        );
        let (review, out) = land(&fx, &r)?;
        let cs = comments(&review);
        check_eq!(cs.len(), 1, "one comment");
        check!(no_anchor(&cs[0]), "no anchor to land");
        check_eq!(
            header_anchor(&mut js, &cs[0])?,
            json!("wholeDocumentIntended"),
            "never confused with a dropped anchor"
        );
        // Negative control for the degradation clause asserted in
        // `persist_ladder_records_change_review_anchors`.
        check!(
            !review["body"].as_str().is_some_and(
                |b| b.contains("requested anchor(s) could not be placed at persist time")
            ),
            "nothing degraded, so the summary carries no degradation clause: {}",
            review["body"]
        );
        check_eq!(
            r["persistDegraded"],
            json!({ "requested": 0, "degraded": 0, "all": false }),
            "zero requested never reports all"
        );
        check!(
            has_line(&out, "anchorsDegraded=none"),
            "none when nothing was requested: {out}"
        );
        Ok(())
    });
}

#[test]
fn all_anchors_degraded_sets_all() {
    run_real(|lib| {
        let fx = fixture()?;
        let mut js = Js::open(lib)?;
        let (r, _) = drive(
            lib,
            persist_args(&fx.roadmap_pin),
            vec![
                finding(
                    "first-degraded",
                    "blocking",
                    json!({ "location": "throughout the gate step", "quote": "shipped" }),
                ),
                finding(
                    "second-degraded",
                    "concern",
                    json!({ "concern": "tests", "confidence": 85, "location": "elsewhere, also prose-only", "quote": "shipped" }),
                ),
            ],
        )?;
        let (review, out) = land(&fx, &r)?;
        check_eq!(
            r["persistDegraded"],
            json!({ "requested": 2, "degraded": 2, "all": true }),
            "every requested anchor degrading sets all"
        );
        check!(has_line(&out, "anchorsDegraded=all"), "all: {out}");
        check!(
            has_line(&out, "anchorsParkRequired=yes"),
            "a build-time degradation is systemic: {out}"
        );
        check_eq!(comments(&review).len(), 3, "both findings plus the note");
        for id in ["first-degraded", "second-degraded"] {
            assert_whole_doc_degraded(&mut js, &review, id)?;
        }
        check_eq!(note(&review)?["body"], note_body(&mut js, 2, 2)?, "2 of 2");
        Ok(())
    });
}

// --- The park-required guard ------------------------------------------------------

/// A clean task review with `persist` and `gate`, so the gate ladder carries
/// the park-required guard; returns the gate script.
fn guarded(lib: &Lib, fx: &Fx) -> Result<String, Failure> {
    let (r, _) = drive(
        lib,
        args(&fx.task_pin, json!({ "task": TASK, "persist": true })),
        vec![],
    )?;
    check_eq!(r["outcome"], json!("reviewed"), "clean");
    check!(
        r["persistCommands"].is_array(),
        "persist:true builds a persist ladder: {r}"
    );
    Ok(r["gateScript"].as_str().unwrap_or_default().to_owned())
}

fn gate_in_task(
    fx: &Fx,
    script: &str,
    park: Option<&str>,
) -> Result<std::process::Output, Failure> {
    let mut run = plain().cwd(Path::new(&fx.task_pin.source));
    if let Some(v) = park {
        run = run.env("RDM_PERSIST_ANCHORS_PARK_REQUIRED", v);
    }
    fx.repo.run(&run, script)
}

#[test]
fn gate_refuses_reviewed_when_park_required() {
    run_real(|lib| {
        let fx = fixture()?;
        let script = guarded(lib, &fx)?;
        let refused = gate_in_task(&fx, &script, Some("yes"))?;
        check!(
            !refused.status.success(),
            "the gate exits non-zero when a park is required"
        );
        check!(
            String::from_utf8_lossy(&refused.stderr)
                .contains("RDM_PERSIST_ANCHORS_PARK_REQUIRED=yes"),
            "the refusal names its cause on stderr: {}",
            String::from_utf8_lossy(&refused.stderr)
        );
        check!(
            task_status(&fx)? != json!("reviewed"),
            "the item does not reach reviewed on a park-required run"
        );
        for v in [Some("no"), None] {
            let ok = gate_in_task(&fx, &script, v)?;
            check!(
                ok.status.success(),
                "park-required={v:?} is not refused: {}",
                String::from_utf8_lossy(&ok.stderr)
            );
            check_eq!(
                task_status(&fx)?,
                json!("reviewed"),
                "the write reaches reviewed for park-required={v:?}"
            );
        }
        Ok(())
    });
}

#[test]
fn park_guard_is_what_refuses() {
    run_real(|lib| {
        let fx = fixture()?;
        let script = guarded(lib, &fx)?;
        let mut js = Js::open(lib)?;
        let guard = js.call("persistDegradationGateLines", vec![])?;
        let guard = format!("{}\n", guard.as_str().unwrap_or_default());
        check!(
            script.matches(&guard).count() == 1,
            "the emitted gate carries the guard exactly once, so stripping it is a real mutation"
        );
        let stripped = script.replacen(&guard, "", 1);
        let out = gate_in_task(&fx, &stripped, Some("yes"))?;
        check!(
            out.status.success(),
            "with the guard stripped the same park-required run succeeds: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        check_eq!(
            task_status(&fx)?,
            json!("reviewed"),
            "so the guard, not something else, is what refuses"
        );
        Ok(())
    });
}

#[test]
fn refused_review_start_fails_persist_plain_shell() {
    run_real(|lib| {
        let fx = fixture()?;
        let (r, _) = drive(
            lib,
            args(
                &fx.task_pin,
                json!({ "task": TASK, "gate": false, "persist": true }),
            ),
            vec![],
        )?;
        let script = r["persistScript"].as_str().unwrap_or_default();
        // Fault injection: refuse `review start` and nothing else. The
        // ladder's last line is a printf, so only per-line failure handling
        // can surface the refusal.
        let stub = fx.repo.dir.path().join("refusing-rdm");
        std::fs::write(
            &stub,
            "#!/bin/sh\n[ \"$2\" = start ] || exit 0\necho \"error: refused\" >&2\nexit 1\n",
        )
        .map_err(infra)?;
        make_executable(&stub)?;
        let refusing = script.replace(rdm_bin(), &stub.to_string_lossy());
        check!(
            refusing != script,
            "the stub replaced the binary in the ladder"
        );
        let out = fx.repo.run(&plain(), &refusing)?;
        check!(
            !out.status.success(),
            "a refused review start fails the ladder in a plain shell"
        );
        Ok(())
    });
}

// --- Escalation without a verdict -------------------------------------------------

#[test]
fn no_ac_reviewer_escalates() {
    run_real(|lib| {
        let fx = fixture()?;
        let (r, _) = drive(
            lib,
            phase_args(
                &fx,
                "phase-1-clean",
                json!({ "reviewers": ["correctness", "tests"] }),
            ),
            vec![],
        )?;
        check_eq!(
            r["outcome"],
            json!("escalated"),
            "no acceptance-criteria evidence"
        );
        check!(
            r.get("gateCommands").is_none(),
            "an evidence-incomplete run emits no ladder: {r}"
        );
        let summary = r["summary"].as_str().unwrap_or_default();
        check!(
            summary.contains("ac") && summary.contains("NO AC TABLE"),
            "the summary names the missing reviewer and the absent table: {summary}"
        );
        check_eq!(
            r["reviewCoverage"]["acTableAbsent"],
            json!(true),
            "an unselected ac reviewer counts as absent"
        );
        Ok(())
    });
}

#[test]
fn unresolvable_source_escalates_without_dispatch() {
    run_real(|lib| {
        let fx = fixture()?;
        let broken: [(&str, Value); 5] = [
            ("a short head", json!({ "expectedHead": "abc123" })),
            ("a non-hex head", json!({ "expectedHead": "z".repeat(40) })),
            ("a missing branch", json!({ "expectedBranch": "" })),
            ("a missing base", json!({ "base": Host::undefined() })),
            ("a missing source path", json!({ "source": "   " })),
        ];
        for (name, patch) in broken {
            let a = merge(&[
                phase_args(&fx, "phase-1-clean", json!({ "persist": true })),
                patch,
            ]);
            let (r, agent) = drive(lib, a, vec![])?;
            check_eq!(r["outcome"], json!("escalated"), "{name}: no verdict");
            for key in ["gateCommands", "gateScript", "persistCommands"] {
                check!(r.get(key).is_none(), "{name}: no {key}: {r}");
            }
            check!(
                agent.calls().is_empty(),
                "{name}: the refusal precedes every agent dispatch"
            );
        }
        Ok(())
    });
}

// --- Review-core helpers the driver's decisions rest on ---------------------------

#[test]
fn anchor_refusal_classification_uses_real_error_text() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let spoof = "this code does not touch the validation path";
        let occurrences = |q: &str| {
            vec![
                QuoteOccurrence {
                    occurrence: 1,
                    context: format!("...{q}..."),
                },
                QuoteOccurrence {
                    occurrence: 2,
                    context: format!("...{q}..."),
                },
            ]
        };
        // Each refusal text is rdm-core's own Display for the error the
        // binary raises, never a hand-copied string.
        let cases: Vec<(&str, CoreError, bool)> = vec![
            (
                "QuoteOutsideChangedHunks, untouched line",
                CoreError::QuoteOutsideChangedHunks {
                    path: "src/lib.rs".into(),
                    start_line: 12,
                    end_line: 12,
                    nearest: Some((8, 9)),
                    range: "a1b2c3d..e4f5678".into(),
                },
                true,
            ),
            (
                "QuoteOutsideChangedHunks, untouched file",
                CoreError::QuoteOutsideChangedHunks {
                    path: "src/never-touched.rs".into(),
                    start_line: 1,
                    end_line: 1,
                    nearest: None,
                    range: "a1b2c3d..e4f5678".into(),
                },
                true,
            ),
            (
                "QuoteNotFound",
                CoreError::QuoteNotFound {
                    quote: "shipped".into(),
                    commit: None,
                },
                false,
            ),
            (
                "QuoteAmbiguous",
                CoreError::QuoteAmbiguous {
                    quote: "shipped".into(),
                    commit: None,
                    occurrences: occurrences("shipped"),
                },
                false,
            ),
            (
                "QuoteNotFound echoing the benign wording (spoofed)",
                CoreError::QuoteNotFound {
                    quote: spoof.into(),
                    commit: None,
                },
                false,
            ),
            (
                "QuoteAmbiguous echoing the benign wording (spoofed)",
                CoreError::QuoteAmbiguous {
                    quote: spoof.into(),
                    commit: None,
                    occurrences: occurrences(spoof),
                },
                false,
            ),
            (
                "QuoteOccurrenceOutOfRange",
                CoreError::QuoteOccurrenceOutOfRange {
                    quote: "shipped".into(),
                    occurrence: 3,
                    available: 2,
                },
                false,
            ),
            (
                "ChangePathNotInRevision",
                CoreError::ChangePathNotInRevision {
                    path: "src/does-not-exist.rs".into(),
                    rev: "e4f5678".into(),
                },
                false,
            ),
            (
                "ChangePathNotAFile",
                CoreError::ChangePathNotAFile {
                    path: "src/a-directory".into(),
                    rev: "e4f5678".into(),
                    found: SourceObjectKind::Tree,
                },
                false,
            ),
            (
                "ChangeHeadNotInSource",
                CoreError::ChangeHeadNotInSource {
                    head: "a".repeat(40),
                    location: None,
                },
                false,
            ),
        ];
        for (name, err, benign) in cases {
            let text = err.to_string();
            check_eq!(
                js.call("isAnchorRefusalBenign", vec![json!(text)])?,
                json!(benign),
                "{name}: {text}"
            );
        }
        for absent in [Host::undefined(), json!("")] {
            check_eq!(
                js.call("isAnchorRefusalBenign", vec![absent.clone()])?,
                json!(false),
                "{absent} is never benign"
            );
        }
        Ok(())
    });
}

#[test]
fn has_blocking_in_scope_matrix() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let tiers: [(&str, Value, &[&str]); 2] = [
            ("default", Host::undefined(), &["blocking"]),
            ("large", json!("large"), &["blocking", "concern"]),
        ];
        for (tier_name, tier, severities) in tiers {
            for sev in severities {
                for (scope, gates) in [
                    (json!({ "inScope": false }), false),
                    (json!({ "inScope": true }), true),
                    (json!({}), true),
                ] {
                    let f = merge(&[json!({ "severity": sev, "confidence": 90 }), scope.clone()]);
                    check_eq!(
                        js.call("hasBlocking", vec![json!([f]), tier.clone()])?,
                        json!(gates),
                        "{tier_name} tier, {sev}, {scope}"
                    );
                }
            }
        }
        Ok(())
    });
}

#[test]
fn refute_prompt_plan_command_follows_mode() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let dim = json!({ "key": "correctness" });
        let f = json!({ "id": "f1", "concern": "correctness", "severity": "blocking", "confidence": 90,
                        "what_fails": "it drops a write" });
        let sentinel = "rdm plan show sentinel-plan-7f3a --format json";
        let mut prompt = |mode: &str, ctx: Value| -> Result<String, Failure> {
            Ok(js
                .call(
                    "refutePrompt",
                    vec![json!(mode), dim.clone(), f.clone(), ctx],
                )?
                .as_str()
                .unwrap_or_default()
                .to_owned())
        };
        let code_with = prompt("code", json!({ "target": "T", "planCommand": sentinel }))?;
        let code_none = prompt("code", json!({ "target": "T" }))?;
        let code_undef = prompt(
            "code",
            json!({ "target": "T", "planCommand": Host::undefined() }),
        )?;
        let code_empty = prompt("code", json!({ "target": "T", "planCommand": "" }))?;
        let plan_with = prompt("plan", json!({ "target": "T", "planCommand": sentinel }))?;
        let plan_none = prompt("plan", json!({ "target": "T" }))?;
        check!(
            code_with.contains(sentinel),
            "the injected plan command reaches the code refuter"
        );
        check!(!code_none.contains(sentinel), "no plan, no plan command");
        check_eq!(
            code_none,
            code_undef,
            "absent and undefined are one spelling"
        );
        check_eq!(code_none, code_empty, "absent and '' are one spelling");
        // Supplying a plan adds lines in both modes (the plan-read line); in
        // code mode it also adds a clause plan mode never gets.
        let added = |with: &str, without: &str| -> Vec<String> {
            let base: std::collections::HashSet<&str> = without.lines().collect();
            with.lines()
                .filter(|l| !base.contains(l))
                .map(str::to_owned)
                .collect()
        };
        let code_added = added(&code_with, &code_none);
        let plan_added = added(&plan_with, &plan_none);
        check!(
            !plan_added.is_empty() && plan_added.iter().all(|l| code_added.contains(l)),
            "plan mode adds only what code mode also adds: {plan_added:?} vs {code_added:?}"
        );
        check!(
            code_added.len() > plan_added.len(),
            "code mode adds a clause plan mode does not: {code_added:?}"
        );
        Ok(())
    });
}

#[test]
fn out_of_scope_blocker_reviewed_in_scope_reworks() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for (in_scope, want) in [(false, "reviewed"), (true, "rework")] {
            let agent = Agent::scripted(move |call| {
                let l = call.label.as_str();
                if l == "find:code:ac" {
                    Reply::Value(
                        json!({ "ac": [{ "criterion": "AC1: it works", "status": "PASS", "evidence": "covered" }],
                                         "findings": [] }),
                    )
                } else if l == "find:code:correctness" {
                    Reply::Value(
                        json!({ "findings": [{ "id": "scope-bug", "concern": "correctness", "severity": "blocking",
                                                        "confidence": 90, "what_fails": "it drops a write" }] }),
                    )
                } else if l.starts_with("refute:") {
                    Reply::Value(json!({ "refuted": false, "confidence": 90, "inScope": in_scope }))
                } else {
                    Reply::Value(json!({ "findings": [] }))
                }
            });
            let out = js.review_ok(
                "code",
                &agent,
                json!({ "reviewers": ["ac", "correctness"], "planCommand": "rdm plan show scope-flip --format json",
                        "target": "the change under review" }),
            )?;
            let survivors = out["survivors"].clone();
            check_eq!(
                survivors.as_array().map(Vec::len),
                Some(1),
                "inScope {in_scope}: the blocker survives refutation"
            );
            check_eq!(
                survivors[0]["inScope"],
                json!(in_scope),
                "the verdict's scope rides on the survivor"
            );
            check_eq!(
                js.call(
                    "classifyOutcome",
                    vec![json!({ "codeReviews": [survivors], "acTable": out["acTable"] })]
                )?,
                json!(want),
                "inScope {in_scope} classifies {want}"
            );
        }
        Ok(())
    });
}

#[test]
fn comment_header_in_scope_round_trip() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let base = json!({ "id": "f1", "concern": "correctness", "severity": "blocking", "confidence": 90,
                           "what_fails": "it drops a write" });
        for (scope, want) in [
            (json!({ "inScope": true }), json!(true)),
            (json!({ "inScope": false }), json!(false)),
            (json!({}), Value::Null),
        ] {
            let body = js.call(
                "formatCommentBody",
                vec![merge(&[base.clone(), scope.clone()])],
            )?;
            let h = js.call("parseCommentHeader", vec![body])?;
            check_eq!(
                h["inScope"],
                want,
                "{scope}: inScope round-trips (never graded parses back null)"
            );
        }
        Ok(())
    });
}

fn legacy_body(with_in_scope: bool) -> String {
    let mut lines = vec![
        "severity: blocking",
        "confidence: 90",
        "refuted: false",
        "unrefutedReason: none",
        "dimension: correctness",
        "finding-id: f1",
    ];
    if with_in_scope {
        lines.push("inScope: n/a");
    }
    lines.extend(["", "correctness", "What fails: it drops a write"]);
    lines.join("\n")
}

fn assert_legacy(js: &mut Js, body: String) -> Outcome {
    let h = js.call("parseCommentHeader", vec![json!(body)])?;
    check!(!h.is_null(), "a legacy body is still machine-written: {h}");
    check_eq!(h["severity"], json!("blocking"), "severity");
    check_eq!(h["confidence"], json!(90), "confidence");
    check_eq!(h["refuted"], json!(false), "refuted");
    check_eq!(h["unrefutedReason"], json!("none"), "unrefutedReason");
    check_eq!(h["dimension"], json!("correctness"), "dimension");
    check_eq!(h["findingId"], json!("f1"), "finding id");
    check_eq!(h["inScope"], Value::Null, "inScope unknown, not guessed");
    check!(
        rdm_devtools::workflow::member(&h, "anchor").is_none(),
        "anchor unknown, not guessed: {h}"
    );
    check_eq!(h["whatFails"], json!("it drops a write"), "whatFails");
    Ok(())
}

#[test]
fn parse_comment_header_legacy_seven_key() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        assert_legacy(&mut js, legacy_body(true))?;
        // The current eight-key shape is still preferred.
        let current = js.call(
            "formatCommentBody",
            vec![
                json!({ "id": "f1", "concern": "correctness", "severity": "blocking", "confidence": 90,
                        "what_fails": "it drops a write" }),
                json!("path"),
            ],
        )?;
        check_eq!(
            js.call("parseCommentHeader", vec![current])?["anchor"],
            json!("path"),
            "the current shape parses its anchor"
        );
        Ok(())
    });
}

#[test]
fn parse_comment_header_legacy_six_key() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        assert_legacy(&mut js, legacy_body(false))
    });
}

#[test]
fn persist_anchor_invalid_path_falls_back() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let opts = json!({ "pathAnchors": true, "source": { "noCode": false } });
        for bad in ["/abs/path.rs", "../escape.rs", "   "] {
            let d = js.call(
                "persistAnchorFor",
                vec![
                    json!({ "id": "f1", "concern": "correctness", "quote": "shipped", "path": bad,
                            "location": "roadmap-work.txt:1" }),
                    json!("change/abc123"),
                    opts.clone(),
                ],
            )?;
            check_eq!(
                d["path"],
                json!("roadmap-work.txt"),
                "{bad:?} falls back to the location"
            );
            check_eq!(d["quote"], json!(true), "a usable fallback still anchors");
            check_eq!(d["reason"], Value::Null, "not a degradation");
        }
        // Negative control: no usable fallback still degrades.
        let d = js.call(
            "persistAnchorFor",
            vec![
                json!({ "id": "f2", "concern": "correctness", "quote": "shipped", "path": "/abs/path.rs",
                        "location": "throughout the gate step" }),
                json!("change/abc123"),
                opts,
            ],
        )?;
        check_eq!(d["path"], Value::Null, "no path");
        check_eq!(d["reason"], json!("path-missing"), "degraded");
        Ok(())
    });
}

#[test]
fn persist_anchor_strips_slashed_suffix() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let d = js.call(
            "persistAnchorFor",
            vec![
                json!({ "id": "f1", "concern": "correctness", "quote": "shipped", "path": "src/foo.rs:12-18",
                        "location": "prose" }),
                json!("change/abc123"),
                json!({ "pathAnchors": true, "source": { "noCode": false } }),
            ],
        )?;
        check_eq!(
            d["path"],
            json!("src/foo.rs"),
            "the range suffix is stripped"
        );
        check_eq!(d["quote"], json!(true), "anchored");
        check_eq!(d["reason"], Value::Null, "not a degradation");
        Ok(())
    });
}

// --- Held-open stdin -----------------------------------------------------------

fn stdin_ladder(lib: &Lib, fx: &Fx, id: &str) -> Result<String, Failure> {
    let (r, _) = drive(
        lib,
        args(
            &fx.task_pin,
            json!({ "task": TASK, "gate": false, "persist": true, "implements": format!("plan/{PLAN}") }),
        ),
        vec![finding(
            id,
            "concern",
            json!({ "location": "general", "what_fails": "a stub finding for a non-empty ladder" }),
        )],
    )?;
    Ok(r["persistScript"].as_str().unwrap_or_default().to_owned())
}

#[test]
fn persist_ladder_completes_under_open_stdin() {
    run_real(|lib| {
        let fx = fixture()?;
        let script = stdin_ladder(lib, &fx, "stdin-hang-regression")?;
        match fx
            .repo
            .run_open_stdin(&Run::bare(), &script, Duration::from_secs(8))?
        {
            OpenStdin::Exited { code, stdout } => {
                check_eq!(code, 0, "the ladder exits 0 with stdin held open: {stdout}");
                check!(
                    review_id(&stdout).is_some(),
                    "and reports its review: {stdout}"
                );
                Ok(())
            }
            OpenStdin::TimedOut { stdout, stderr } => Err(Failure::Check(format!(
                "the ladder hung with stdin held open: {stdout}{stderr}"
            ))),
        }
    });
}

#[test]
fn dev_null_redirects_prevent_stdin_hang() {
    run_real(|lib| {
        let fx = fixture()?;
        let script = stdin_ladder(lib, &fx, "stdin-hang-regression-stub")?;
        // A stand-in for `rdm` that reads stdin to EOF before delegating, so
        // only the ladder's own per-line redirects can let it finish.
        let stub = fx.repo.dir.path().join("stdin-blocking-rdm");
        std::fs::write(
            &stub,
            format!("#!/bin/sh\ncat >/dev/null\nexec {} \"$@\"\n", rdm_bin()),
        )
        .map_err(infra)?;
        make_executable(&stub)?;
        let blocking = script.replace(rdm_bin(), &stub.to_string_lossy());
        check!(
            blocking != script,
            "every rdm invocation now goes through the stub"
        );
        match fx
            .repo
            .run_open_stdin(&Run::bare(), &blocking, Duration::from_secs(8))?
        {
            OpenStdin::Exited { code, stdout } => {
                check_eq!(
                    code,
                    0,
                    "the redirects feed the blocking read EOF: {stdout}"
                );
                check!(
                    review_id(&stdout).is_some(),
                    "the ladder completed: {stdout}"
                );
            }
            OpenStdin::TimedOut { stdout, stderr } => {
                return Err(Failure::Check(format!(
                    "the ladder hung despite its redirects: {stdout}{stderr}"
                )));
            }
        }
        // The redirect-strip mutant must hang: the Session deadline expires and
        // the process group is killed.
        let stripped = blocking.replace(" < /dev/null", "");
        check!(stripped != blocking, "the mutant removes the redirects");
        match fx
            .repo
            .run_open_stdin(&Run::bare(), &stripped, Duration::from_millis(1500))?
        {
            OpenStdin::TimedOut { .. } => Ok(()),
            OpenStdin::Exited { code, stdout } => Err(Failure::Check(format!(
                "without the redirects the stub must block on stdin, but the ladder exited {code}: {stdout}"
            ))),
        }
    });
}
