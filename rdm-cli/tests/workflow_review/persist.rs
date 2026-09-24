//! The persist writer: its pure helpers, and the command ladder it emits run
//! under a real `sh` against the real `rdm` binary in a per-test plan repo.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::git_test_support::git;
use crate::support::{Agent, Failure, Js, Lib, Outcome, REVIEW_LIB, Reply, run_mutant, run_real};

const PROJECT: &str = "persist-verify";
const TASK_BODY: &str = "Alpha opening line.\nThe retry backoff strategy is unspecified here.\nA repeated sentence.\nA repeated sentence.\nTrailing \"quoted\" $dollar `backtick` — em-dash line.";

fn rdm_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rdm")
}

/// Applies the hermetic environment every `rdm` (and ladder) process runs
/// under: no inherited `RDM_*`, no real global/system git or rdm config.
fn hermetic(cmd: &mut Command, root: &Path, session: &str) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("RDM_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("RDM_ROOT", root)
        .env("RDM_SESSION", session)
        .env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
}

/// A seeded, committed plan repo in its own temp directory.
struct PlanRepo {
    dir: TempDir,
    root: PathBuf,
}

impl PlanRepo {
    fn new() -> Result<Self, Failure> {
        let dir = TempDir::new().map_err(|e| Failure::Infra(e.to_string()))?;
        let root = dir.path().join("plan");
        std::fs::create_dir_all(&root).map_err(|e| Failure::Infra(e.to_string()))?;
        let repo = Self { dir, root };
        repo.ok("seed", &["init", "--default-project", PROJECT])?;
        repo.ok(
            "seed",
            &[
                "task",
                "create",
                "persist-target",
                "--title",
                "Persist target",
                "--no-edit",
                "--project",
                PROJECT,
                "--body",
                TASK_BODY,
            ],
        )?;
        repo.ok(
            "seed",
            &[
                "roadmap",
                "create",
                "persist-rm",
                "--title",
                "Persist roadmap",
                "--body",
                "Roadmap body.",
                "--no-edit",
                "--project",
                PROJECT,
            ],
        )?;
        repo.ok(
            "seed",
            &[
                "phase",
                "create",
                "target",
                "--title",
                "Target phase",
                "--number",
                "1",
                "--body",
                "Phase body with a unique phase sentence.",
                "--no-edit",
                "--roadmap",
                "persist-rm",
                "--project",
                PROJECT,
            ],
        )?;
        repo.ok("seed", &["commit", "-m", "chore(plan): seed"])?;
        Ok(repo)
    }

    fn rdm(&self, session: &str, args: &[&str]) -> Result<Output, Failure> {
        let mut cmd = Command::new(rdm_bin());
        hermetic(&mut cmd, &self.root, session);
        cmd.args(args)
            .output()
            .map_err(|e| Failure::Infra(format!("running rdm: {e}")))
    }

    fn ok(&self, session: &str, args: &[&str]) -> Result<String, Failure> {
        let out = self.rdm(session, args)?;
        if !out.status.success() {
            return Err(Failure::Infra(format!(
                "rdm {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn review(&self, id: &str) -> Result<Value, Failure> {
        let text = self.ok(
            "reader",
            &[
                "review",
                "show",
                id,
                "--format",
                "json",
                "--project",
                PROJECT,
            ],
        )?;
        serde_json::from_str(&text)
            .map_err(|e| Failure::Infra(format!("review show json: {e}: {text}")))
    }

    fn reviews_on(&self, target: &str) -> Result<Vec<Value>, Failure> {
        let text = self.ok(
            "reader",
            &[
                "review",
                "list",
                "--on",
                target,
                "--format",
                "json",
                "--project",
                PROJECT,
            ],
        )?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| Failure::Infra(format!("review list json: {e}: {text}")))?;
        let list = v
            .as_array()
            .cloned()
            .or_else(|| v["reviews"].as_array().cloned())
            .unwrap_or_default();
        list.iter()
            .filter_map(|r| r["id"].as_str())
            .map(|id| self.review(id))
            .collect()
    }

    /// A fresh, empty directory to use as the ladder's `TMPDIR`.
    fn tmpdir(&self, name: &str) -> Result<PathBuf, Failure> {
        let p = self.dir.path().join(name);
        std::fs::create_dir_all(&p).map_err(|e| Failure::Infra(e.to_string()))?;
        Ok(p)
    }

    /// Runs `script` under `sh -c` with the hermetic environment.
    fn sh(&self, script: &str, session: &str, tmpdir: &Path) -> Result<Output, Failure> {
        let mut cmd = Command::new("sh");
        hermetic(&mut cmd, &self.root, session);
        cmd.arg("-c")
            .arg(script)
            .env("TMPDIR", tmpdir)
            .current_dir(self.dir.path())
            .output()
            .map_err(|e| Failure::Infra(format!("running sh: {e}")))
    }
}

/// The ladder `persistReviewCommands` emits, as one script.
fn ladder(
    js: &mut Js,
    mode: &str,
    outcome: &str,
    survivors: Value,
    target: &str,
) -> Result<String, Failure> {
    let cmds = js.call(
        "persistReviewCommands",
        vec![
            json!({ "mode": mode, "outcome": outcome, "survivors": survivors }),
            json!(target),
            json!({ "rdmBin": rdm_bin(), "project": PROJECT }),
        ],
    )?;
    let lines: Vec<&str> = cmds
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Ok(lines.join("\n") + "\n")
}

fn review_id(out: &Output) -> Option<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("reviewId="))
        .next_back()
        .map(str::to_owned)
}

/// Runs a ladder to success, returning the review id it reports.
fn land(repo: &PlanRepo, script: &str, session: &str) -> Result<String, Failure> {
    let tmp = repo.tmpdir(&format!("tmp-{session}"))?;
    let out = repo.sh(script, session, &tmp)?;
    check!(
        out.status.success(),
        "the emitted ladder exited {:?}: {}{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    review_id(&out).ok_or_else(|| {
        Failure::Check(format!(
            "no reviewId= line: {}",
            String::from_utf8_lossy(&out.stdout)
        ))
    })
}

fn comments(review: &Value) -> Vec<Value> {
    review["comments"].as_array().cloned().unwrap_or_default()
}

// --- Pure helpers ------------------------------------------------------------

#[test]
fn verdict_map_and_unknown_outcome_throws() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        check_eq!(
            js.get("PERSIST_VERDICT")?,
            json!({ "reviewed": "approve", "rework": "request-changes", "escalated": "request-changes" }),
            "the verdict map"
        );
        for (outcome, want) in [
            ("reviewed", "approve"),
            ("rework", "request-changes"),
            ("escalated", "request-changes"),
        ] {
            check_eq!(
                js.call("persistVerdictFor", vec![json!(outcome)])?,
                json!(want),
                "{outcome}"
            );
        }
        for bad in ["nonsense", "constructor"] {
            let r = js.try_call("persistVerdictFor", vec![json!(bad)])?;
            check!(
                r.as_ref()
                    .is_err_and(|e| e.message.contains("unrecognized outcome")),
                "{bad} must throw, never default to a verdict: {r:?}"
            );
        }
        Ok(())
    });
}

#[test]
fn comment_header_round_trip() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let keys = js.get("PERSIST_HEADER_KEYS")?;
        let keys: Vec<String> = keys
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str().map(str::to_owned))
            .collect();
        let matrix = [
            json!({ "id": "g1", "concern": "coherence", "severity": "blocking", "confidence": 90, "what_fails": "plain" }),
            json!({ "id": "n1", "concern": "restraint", "severity": "suggestion", "confidence": 75, "what_fails": "ng", "unrefuted": true, "unrefutedReason": "non-gating" }),
            json!({ "id": "b1", "concern": "architectural-fit", "severity": "concern", "confidence": 80, "what_fails": "bg", "unrefuted": true, "unrefutedReason": "budget" }),
            json!({ "id": "e1", "concern": "coherence", "severity": "blocking", "confidence": 99, "what_fails": "line one\nline two", "refuterError": true }),
        ];
        for f in matrix {
            let body = js.call("formatCommentBody", vec![f.clone()])?;
            let text = body.as_str().unwrap_or_default();
            let lines: Vec<&str> = text.split('\n').collect();
            for (i, key) in keys.iter().enumerate() {
                check!(
                    lines
                        .get(i)
                        .is_some_and(|l| l.starts_with(&format!("{key}: "))),
                    "{}: header line {i} is `{key}: `: {text:?}",
                    f["id"]
                );
            }
            check_eq!(
                lines.get(keys.len()).copied(),
                Some(""),
                "{}: a blank line separates header and prose",
                f["id"]
            );
            let h = js.call("parseCommentHeader", vec![body.clone()])?;
            check_eq!(h["severity"], f["severity"], "severity round-trips");
            check_eq!(h["confidence"], f["confidence"], "confidence round-trips");
            check_eq!(h["refuted"], json!(false), "refuted is always false");
            let reason = f.get("unrefutedReason").cloned().unwrap_or(json!("none"));
            check_eq!(
                h["unrefutedReason"],
                reason,
                "unrefutedReason uses the `none` sentinel"
            );
            check_eq!(h["dimension"], f["concern"], "dimension round-trips");
            check_eq!(h["findingId"], f["id"], "finding-id round-trips");
            let first = f["what_fails"]
                .as_str()
                .unwrap_or_default()
                .split('\n')
                .next()
                .unwrap_or_default()
                .to_owned();
            check_eq!(
                h["whatFails"],
                json!(first),
                "whatFails is recovered (first line)"
            );
        }
        Ok(())
    });
}

#[test]
fn parse_comment_header_rejects_non_headers() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for body in [
            json!("This plan looks fine to me."),
            json!(""),
            json!(null),
            json!("severity: blocking\nnope"),
        ] {
            check_eq!(
                js.call("parseCommentHeader", vec![body.clone()])?,
                json!(null),
                "{body} has no header"
            );
        }
        Ok(())
    });
}

#[test]
fn opaque_and_bare_ref_guards() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cfg = json!({ "rdmBin": "/fake/bin/rdm", "project": "demo" });
        let result = json!({ "mode": "plan", "outcome": "rework", "survivors": [] });
        for bad in [
            json!("nope"),
            json!(""),
            json!("   "),
            json!(null),
            json!({ "$undefined": true }),
            json!(42),
            json!({}),
        ] {
            let r = js.try_call(
                "persistReviewCommands",
                vec![result.clone(), bad.clone(), cfg.clone()],
            )?;
            check!(
                r.as_ref()
                    .is_err_and(|e| e.message.contains("well-formed rdm review ref")),
                "a ref with no '/' throws: {bad}: {r:?}"
            );
        }
        let r = js.try_call(
            "persistReviewCommands",
            vec![
                json!({ "mode": "bogus", "outcome": "rework", "survivors": [] }),
                json!("task/t"),
                cfg,
            ],
        )?;
        check!(
            r.as_ref()
                .is_err_and(|e| e.message.contains("unknown gate mode")),
            "an unknown mode throws: {r:?}"
        );
        Ok(())
    });
}

#[test]
fn refs_emitted_shell_quoted() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cfg = json!({ "rdmBin": "/fake/bin/rdm", "project": "demo" });
        let result = json!({ "mode": "plan", "outcome": "rework", "survivors": [] });
        for r in [
            "task/t",
            "phase/rm/phase-1-x",
            "phase/rm/1",
            "roadmap/rm",
            "plan/p",
            "change/abc123",
        ] {
            let cmds = js.call(
                "persistReviewCommands",
                vec![result.clone(), json!(r), cfg.clone()],
            )?;
            let joined: Vec<&str> = cmds
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            check!(
                joined
                    .iter()
                    .any(|c| c.contains(&format!(" review start --on '{r}' "))),
                "ref {r} is passed through, shell-quoted and unprefixed"
            );
        }
        let a = js.call(
            "persistReviewCommands",
            vec![result.clone(), json!("task/t"), cfg.clone()],
        )?;
        let b = js.call("persistReviewCommands", vec![result, json!("task/t"), cfg])?;
        check_eq!(a, b, "the emitted commands are deterministic");
        Ok(())
    });
}

#[test]
fn resolve_persist_arg() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cases = [
            (json!({ "$undefined": true }), json!(null)),
            (json!(null), json!(null)),
            (json!(false), json!(null)),
            (json!(true), json!({ "on": null })),
            (json!({}), json!({ "on": null })),
            (json!({ "on": "plan/x" }), json!({ "on": "plan/x" })),
        ];
        for (arg, want) in cases {
            check_eq!(
                js.plan_call("resolvePersistArg", vec![arg.clone()])?,
                want,
                "resolvePersistArg({arg})"
            );
        }
        for bad in [json!("task/x"), json!(["a"])] {
            let r = js.plan_try_call("resolvePersistArg", vec![bad.clone()])?;
            check!(
                r.as_ref()
                    .is_err_and(|e| e.message.contains("persist must be omitted")),
                "{bad} is not a persist arg: {r:?}"
            );
        }
        Ok(())
    });
}

#[test]
fn parse_plan_args_never_parses_persist() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let a = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "target": "--task t --persist" })],
        )?;
        check_eq!(
            a["persist"],
            json!(null),
            "persist is never tokenized out of the flag string"
        );
        let b = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "task": "t", "persist": true })],
        )?;
        check_eq!(
            b["persist"],
            json!({ "on": null }),
            "the structured key is honoured"
        );
        Ok(())
    });
}

#[test]
fn persist_target_for() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cases = [
            (
                json!({ "kind": "task", "ident": "t" }),
                json!(null),
                1,
                "task/t",
            ),
            (
                json!({ "kind": "phase", "roadmap": "rm", "ident": "phase-1-x" }),
                json!(null),
                1,
                "phase/rm/phase-1-x",
            ),
            (
                json!({ "kind": "roadmap", "ident": "rm" }),
                json!(null),
                1,
                "roadmap/rm",
            ),
            (
                json!({ "kind": "task", "ident": "t" }),
                json!({ "on": "plan/p" }),
                1,
                "plan/p",
            ),
            (
                json!({ "kind": "phase", "roadmap": "rm", "ident": "phase-2-y" }),
                json!({ "on": "plan/p" }),
                4,
                "phase/rm/phase-2-y",
            ),
        ];
        for (unit, persist, count, want) in cases {
            let got = js.plan_call(
                "persistTargetFor",
                vec![unit.clone(), persist, json!(count)],
            )?;
            check_eq!(got, json!(want), "persistTargetFor({unit}, units={count})");
        }
        Ok(())
    });
}

#[test]
fn prior_round_and_findings_from_reviews() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let rounds = [
            (json!(null), 0),
            (json!([]), 0),
            (json!([{ "state": "draft" }]), 0),
            (
                json!([{ "state": "submitted" }, { "state": "addressed" }, { "state": "draft" }]),
                2,
            ),
        ];
        for (reviews, want) in rounds {
            check_eq!(
                js.plan_call("priorRoundFromReviews", vec![reviews.clone()])?,
                json!(want),
                "{reviews}"
            );
        }
        check_eq!(
            js.plan_call("priorFindingsFromReviews", vec![json!(null)])?,
            json!([]),
            "unreadable -> no findings"
        );
        let body = js.call(
            "formatCommentBody",
            vec![json!({ "id": "p1", "concern": "coherence", "severity": "blocking", "confidence": 91, "what_fails": "the backoff is unspecified" })],
        )?;
        let reviews = json!([
            { "id": "2026-01-01-0000-aaaa", "state": "submitted", "created": "2026-01-01T00:00:00Z", "comments": [{ "body": "a human note" }] },
            { "id": "2026-01-02-0000-bbbb", "state": "submitted", "created": "2026-01-02T00:00:00Z",
              "comments": [{ "body": body }, { "body": "a human note with no header" }] }
        ]);
        let prior = js.plan_call("priorFindingsFromReviews", vec![reviews.clone()])?;
        check_eq!(
            prior,
            json!([{ "severity": "blocking", "concern": "coherence", "what_fails": "the backoff is unspecified" }]),
            "only header-carrying comments of the latest review"
        );
        let reversed = json!([reviews[1], reviews[0]]);
        check_eq!(
            js.plan_call("priorFindingsFromReviews", vec![reversed])?,
            prior,
            "latest is chosen by created, not order"
        );
        Ok(())
    });
}

fn quoted_agent(quote_ok: Option<bool>) -> Agent {
    Agent::scripted(move |call| {
        if call.label.starts_with("find:") {
            if call.label.contains("coherence") {
                Reply::Value(
                    json!({ "findings": [{ "id": "q1", "concern": "coherence", "severity": "blocking", "confidence": 90,
                    "what_fails": "the retry backoff strategy is unspecified", "quote": "retry with backoff" }] }),
                )
            } else {
                Reply::Value(json!({ "findings": [] }))
            }
        } else {
            let mut v = json!({ "refuted": false, "confidence": 95 });
            if let Some(ok) = quote_ok {
                v["quote_ok"] = json!(ok);
            }
            Reply::Value(v)
        }
    })
}

#[test]
fn quote_ok_preserves_or_clears_quote() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let ctx = json!({ "target": "the plan" });
        for (ok, keeps) in [(Some(true), true), (None, true), (Some(false), false)] {
            let out = js.review_ok("plan", &quoted_agent(ok), ctx.clone())?;
            let f = crate::support::find(&out, "survivors", "q1")
                .ok_or_else(|| Failure::Check(format!("quote_ok {ok:?} dropped the finding")))?;
            if keeps {
                check_eq!(
                    f["quote"],
                    json!("retry with backoff"),
                    "quote_ok {ok:?} preserves the quote"
                );
            } else {
                check!(
                    f.get("quote").is_none(),
                    "quote_ok false removes the quote key entirely: {f}"
                );
            }
        }
        Ok(())
    });
}

#[test]
fn strip_quote_is_pure() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let out = js.call(
            "stripQuote",
            vec![json!({ "id": "x", "quote": "q", "severity": "blocking" })],
        )?;
        check_eq!(
            out,
            json!({ "id": "x", "severity": "blocking" }),
            "the quote key is removed, the rest kept"
        );
        let none = js.call("stripQuote", vec![json!({ "id": "y" })])?;
        check_eq!(
            none,
            json!({ "id": "y" }),
            "a quote-less finding comes back unchanged"
        );
        Ok(())
    });
}

#[test]
fn quote_changes_refute_prompt() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let dims = js.call("resolveReviewers", vec![json!("plan")])?;
        let dim = dims[0].clone();
        let no_quote = json!({ "id": "f1", "concern": "coherence", "severity": "blocking", "confidence": 90, "what_fails": "x" });
        let mut with_quote = no_quote.clone();
        with_quote["quote"] = json!("a verbatim span");
        let ctx = json!({ "target": "t" });
        let a = js.call(
            "refutePrompt",
            vec![json!("plan"), dim.clone(), no_quote.clone(), ctx.clone()],
        )?;
        let b = js.call(
            "refutePrompt",
            vec![json!("plan"), dim.clone(), with_quote, ctx.clone()],
        )?;
        check!(
            a != b,
            "a quote-carrying finding gets a different refuter prompt"
        );
        let again = js.call("refutePrompt", vec![json!("plan"), dim, no_quote, ctx])?;
        check_eq!(a, again, "the refuter prompt is deterministic");
        Ok(())
    });
}

#[test]
fn path_from_location_guards() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cases = [
            (json!({ "$undefined": true }), json!(null), "non-string"),
            (json!(42), json!(null), "a number"),
            (json!(""), json!(null), "empty"),
            (json!("   "), json!(null), "whitespace"),
            (json!("throughout the gate step"), json!(null), "bare prose"),
            (json!("src/lib rs"), json!(null), "embedded space"),
            (json!("/src/lib.rs"), json!(null), "leading slash"),
            (json!("src\\lib.rs"), json!(null), "backslash"),
            (json!("src/../lib.rs"), json!(null), ".. segment"),
            (json!(".."), json!(null), "bare .."),
            (
                json!("src/lib.rs:42"),
                json!("src/lib.rs"),
                "a line suffix is stripped",
            ),
            (
                json!("src/lib.rs:10-20"),
                json!("src/lib.rs"),
                "a range suffix is stripped",
            ),
            (
                json!("rdm-core/src/lib.rs"),
                json!("rdm-core/src/lib.rs"),
                "a slash-bearing path",
            ),
            (
                json!("Cargo.toml"),
                json!("Cargo.toml"),
                "an extension-bearing file",
            ),
            (
                json!("Cargo.toml:notes"),
                json!(null),
                "a non-numeric colon suffix is not a line marker",
            ),
        ];
        for (loc, want, why) in cases {
            check_eq!(js.call("pathFromLocation", vec![loc])?, want, "{why}");
        }
        Ok(())
    });
}

// --- The ladder against the real binary -----------------------------------------

#[test]
fn ladder_lands_anchored_whole_doc_and_cleared_comments() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = PlanRepo::new()?;
        let survivors = json!([
            { "id": "a1", "concern": "coherence", "severity": "blocking", "confidence": 90, "what_fails": "the backoff is unspecified",
              "why": "no rule", "recommendation": "state it", "quote": "The retry backoff strategy is unspecified here." },
            { "id": "w1", "concern": "restraint", "severity": "concern", "confidence": 80, "what_fails": "scope creep" },
            { "id": "c1", "concern": "architectural-fit", "severity": "concern", "confidence": 85, "what_fails": "a stale quote was cleared by the refuter" }
        ]);
        let script = ladder(&mut js, "plan", "rework", survivors, "task/persist-target")?;
        let id = land(&repo, &script, "ladder")?;
        let review = repo.review(&id)?;
        check_eq!(
            review["verdict"],
            json!("request-changes"),
            "rework persists request-changes"
        );
        check_eq!(
            review["state"],
            json!("submitted"),
            "the review is submitted"
        );
        check_eq!(review["target"]["kind"], json!("task"), "target kind");
        check_eq!(
            review["target"]["slug"],
            json!("persist-target"),
            "target slug"
        );
        let cs = comments(&review);
        check_eq!(cs.len(), 3, "one comment per survivor");
        let resolved: Vec<&Value> = cs
            .iter()
            .filter(|c| c["resolution"]["state"] == json!("resolved"))
            .collect();
        check_eq!(resolved.len(), 1, "exactly the quoted survivor anchors");
        check_eq!(
            resolved[0]["resolution"]["quote"],
            json!("The retry backoff strategy is unspecified here."),
            "the anchor resolves to the quoted sentence"
        );
        check_eq!(
            resolved[0]["anchor"]["anchor_type"],
            json!("text-quote"),
            "a text-quote anchor"
        );
        let whole: Vec<&Value> = cs
            .iter()
            .filter(|c| c.get("anchor").is_none_or(Value::is_null))
            .collect();
        check_eq!(whole.len(), 2, "quote-less survivors land whole-document");
        check!(
            whole
                .iter()
                .all(|c| c["resolution"]["state"] != json!("resolved")),
            "a whole-document comment has no resolved anchor"
        );
        let mut by_id = std::collections::HashMap::new();
        for c in &cs {
            let h = js.call("parseCommentHeader", vec![c["body"].clone()])?;
            check!(
                !h.is_null(),
                "every persisted body parses back: {}",
                c["body"]
            );
            check_eq!(h["refuted"], json!(false), "refuted is false");
            check_eq!(h["unrefutedReason"], json!("none"), "no unrefuted reason");
            by_id.insert(h["findingId"].as_str().unwrap_or_default().to_owned(), h);
        }
        check_eq!(by_id["a1"]["severity"], json!("blocking"), "a1 severity");
        check_eq!(by_id["a1"]["confidence"], json!(90), "a1 confidence");
        check_eq!(by_id["a1"]["dimension"], json!("coherence"), "a1 dimension");
        check_eq!(by_id["w1"]["severity"], json!("concern"), "w1 severity");
        check_eq!(by_id["c1"]["confidence"], json!(85), "c1 confidence");
        let body = cs
            .iter()
            .find(|c| c["resolution"]["state"] == json!("resolved"))
            .map(|c| c["body"].clone());
        check!(
            body.is_some_and(|b| b
                .as_str()
                .is_some_and(|s| s.contains("the backoff is unspecified"))),
            "prose rides through"
        );
        Ok(())
    });
}

#[test]
fn verdict_mapping_via_real_binary() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = PlanRepo::new()?;
        // Zero survivors still submits (the start body is never empty).
        let script = ladder(
            &mut js,
            "plan",
            "reviewed",
            json!([]),
            "task/persist-target",
        )?;
        let clean = repo.review(&land(&repo, &script, "clean")?)?;
        check_eq!(clean["verdict"], json!("approve"), "reviewed -> approve");
        check_eq!(
            clean["state"],
            json!("submitted"),
            "a zero-survivor review is submitted"
        );

        let esc = json!([{ "id": "e1", "concern": "coherence", "severity": "blocking", "confidence": 95, "what_fails": "the goal is wrong" }]);
        let script = ladder(
            &mut js,
            "plan",
            "escalated",
            esc.clone(),
            "task/persist-target",
        )?;
        let escalated = repo.review(&land(&repo, &script, "esc")?)?;
        check_eq!(
            escalated["verdict"],
            json!("request-changes"),
            "escalated -> request-changes"
        );
        check!(
            escalated.to_string().contains("[plan] escalated"),
            "the escalated plan review carries the [plan] prefix"
        );

        let script = ladder(&mut js, "code", "escalated", esc, "task/persist-target")?;
        let code = repo.review(&land(&repo, &script, "esc-code")?)?;
        check!(
            code.to_string().contains("[code] escalated"),
            "an escalated code review carries the [code] prefix"
        );

        let script = ladder(&mut js, "plan", "rework", json!([]), "task/persist-target")?;
        let rework = repo.review(&land(&repo, &script, "rework")?)?;
        check_eq!(
            rework["verdict"],
            json!("request-changes"),
            "rework -> request-changes"
        );
        check!(
            !rework.to_string().contains("escalated:"),
            "a rework review is not marked escalated"
        );
        Ok(())
    });
}

#[test]
fn phase_stem_and_numeric_refs_resolve() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = PlanRepo::new()?;
        let survivors = json!([{ "id": "p1", "concern": "coherence", "severity": "blocking", "confidence": 90,
                                 "what_fails": "phase gap", "quote": "a unique phase sentence" }]);
        let script = ladder(
            &mut js,
            "plan",
            "rework",
            survivors,
            "phase/persist-rm/phase-1-target",
        )?;
        let review = repo.review(&land(&repo, &script, "stem")?)?;
        check_eq!(
            review["target"]["kind"],
            json!("phase"),
            "the stem ref resolves to a phase review"
        );
        check!(
            comments(&review)
                .iter()
                .any(|c| c["resolution"]["state"] == json!("resolved")),
            "the quote anchors in the phase body"
        );
        let script = ladder(&mut js, "plan", "reviewed", json!([]), "phase/persist-rm/1")?;
        let numeric = repo.review(&land(&repo, &script, "numeric")?)?;
        check_eq!(
            numeric["target"]["kind"],
            json!("phase"),
            "the numeric ref resolves too"
        );
        Ok(())
    });
}

#[test]
fn bare_ref_rejected() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = PlanRepo::new()?;
        let script = ladder(
            &mut js,
            "plan",
            "reviewed",
            json!([]),
            "persist-rm/phase-1-target",
        )?;
        let tmp = repo.tmpdir("bare")?;
        let out = repo.sh(&script, "bare", &tmp)?;
        check!(
            !out.status.success(),
            "the real binary refuses a bare <roadmap>/<phase> ref and the ladder stops"
        );
        check!(review_id(&out).is_none(), "no review id is reported");
        check_eq!(
            repo.reviews_on("phase/persist-rm/phase-1-target")?.len(),
            0,
            "nothing landed on the phase"
        );
        Ok(())
    });
}

#[test]
fn ambiguous_quote_not_silently_anchored() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = PlanRepo::new()?;
        let survivors = json!([{ "id": "amb", "concern": "coherence", "severity": "blocking", "confidence": 90,
                                 "what_fails": "ambiguous", "quote": "A repeated sentence." }]);
        let script = ladder(&mut js, "plan", "rework", survivors, "task/persist-target")?;
        let tmp = repo.tmpdir("amb")?;
        let out = repo.sh(&script, "amb", &tmp)?;
        check!(
            !out.status.success(),
            "an ambiguous quote stops the ladder rather than guessing an occurrence"
        );
        let reviews = repo.reviews_on("task/persist-target")?;
        check_eq!(
            reviews.len(),
            1,
            "the ladder started one review before it stopped"
        );
        for review in reviews {
            check!(
                review["state"] != json!("submitted"),
                "no review is submitted: {review}"
            );
            check!(
                comments(&review)
                    .iter()
                    .all(|c| c.get("anchor").is_none_or(Value::is_null)),
                "no comment is silently anchored at an arbitrary occurrence: {review}"
            );
        }
        Ok(())
    });
}

#[test]
fn ladder_commits_only_its_own_changeset() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = PlanRepo::new()?;
        repo.ok(
            "other",
            &[
                "task",
                "create",
                "unrelated-work",
                "--title",
                "Unrelated",
                "--body",
                "staged by another session",
                "--no-edit",
                "--project",
                PROJECT,
            ],
        )?;
        let script = ladder(
            &mut js,
            "plan",
            "reviewed",
            json!([]),
            "task/persist-target",
        )?;
        land(&repo, &script, "ladder")?;
        let head = git(&repo.root, &["show", "--name-only", "--format=%s", "HEAD"]);
        let head = String::from_utf8_lossy(&head.stdout).into_owned();
        check!(
            head.contains("record plan review of task/persist-target"),
            "the ladder's commit is HEAD: {head}"
        );
        check!(
            !head.contains("unrelated-work"),
            "the other session's change is not swept in: {head}"
        );
        let log = git(&repo.root, &["log", "--name-only", "--format="]);
        check!(
            !String::from_utf8_lossy(&log.stdout).contains("unrelated-work"),
            "the other session's change is in no commit"
        );
        let status = repo.ok("other", &["status"])?;
        check!(
            status.contains("unrelated-work"),
            "the other session's change is still staged: {status}"
        );
        let file = repo
            .root
            .join("projects")
            .join(PROJECT)
            .join("tasks")
            .join("unrelated-work.md");
        check!(file.exists(), "and still on disk");
        Ok(())
    });
}

// --- Shell safety and scratch-file hygiene ------------------------------------

fn injection_safe(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let repo = PlanRepo::new()?;
    let marks = repo.tmpdir("marks")?;
    let m = |n: &str| marks.join(n);
    let target = format!(
        "task/pwn$(touch {})`touch {}`;touch {}",
        m("cmdsub").display(),
        m("backtick").display(),
        m("semicolon").display()
    );
    let script = ladder(&mut js, "plan", "reviewed", json!([]), &target)?;
    let tmp = repo.tmpdir("inject")?;
    repo.sh(&script, "inject", &tmp)?;
    for name in ["cmdsub", "backtick", "semicolon"] {
        check!(
            !m(name).exists(),
            "the {name} injection vector in the target executed"
        );
    }
    Ok(())
}

#[test]
fn ladder_is_shell_injection_safe() {
    run_real(injection_safe);
}

#[test]
fn mutant_target_unquoted() {
    run_mutant(
        Lib::mutant(
            "target-unquoted",
            &[(REVIEW_LIB, ": shellQuote(target)) +", ": target) +")],
        ),
        injection_safe,
    );
}

fn symlink_not_followed(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let repo = PlanRepo::new()?;
    let victim = repo.dir.path().join("victim.txt");
    std::fs::write(&victim, "precious").map_err(|e| Failure::Infra(e.to_string()))?;
    let script = ladder(
        &mut js,
        "plan",
        "reviewed",
        json!([]),
        "task/persist-target",
    )?;
    // Plant a symlink at the predictable pid-named path from inside the SAME
    // shell, so `$$` is the ladder's own pid.
    let preamble = format!(
        "ln -s '{}' \"$TMPDIR/rdm-persist-start.$$.json\" || exit 99\n",
        victim.display()
    );
    let tmp = repo.tmpdir("symlink")?;
    let out = repo.sh(&(preamble + &script), "symlink", &tmp)?;
    check!(
        out.status.success(),
        "the ladder succeeds despite the planted symlink: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let now = std::fs::read_to_string(&victim).map_err(|e| Failure::Infra(e.to_string()))?;
    check_eq!(now, "precious", "the symlink target is untouched");
    Ok(())
}

#[test]
fn ladder_scratch_file_not_at_predictable_path() {
    run_real(symlink_not_followed);
}

#[test]
fn mutant_scratch_path_predictable() {
    run_mutant(
        Lib::mutant(
            "scratch-path-predictable",
            &[(
                REVIEW_LIB,
                "cmds.push('RDM_PERSIST_START_JSON=$(mktemp \"${TMPDIR:-/tmp}/rdm-persist-start.XXXXXX\") || exit 1');",
                "cmds.push('RDM_PERSIST_START_JSON=${TMPDIR:-/tmp}/rdm-persist-start.$$.json');",
            )],
        ),
        symlink_not_followed,
    );
}

fn leaves_no_scratch(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let repo = PlanRepo::new()?;
    let survivors = json!([{ "id": "f1", "concern": "coherence", "severity": "blocking", "confidence": 90, "what_fails": "x" }]);
    let script = ladder(&mut js, "plan", "rework", survivors, "task/persist-target")?;
    let tmp = repo.tmpdir("clean")?;
    let out = repo.sh(&script, "clean", &tmp)?;
    check!(
        out.status.success(),
        "the ladder succeeds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let left: Vec<String> = std::fs::read_dir(&tmp)
        .map_err(|e| Failure::Infra(e.to_string()))?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    check_eq!(
        left,
        Vec::<String>::new(),
        "TMPDIR is empty after the ladder"
    );
    Ok(())
}

#[test]
fn ladder_leaves_no_scratch_file() {
    run_real(leaves_no_scratch);
}

#[test]
fn mutant_scratch_rm_removed() {
    run_mutant(
        Lib::mutant(
            "scratch-rm-removed",
            &[(
                REVIEW_LIB,
                "      'rm -f \"$RDM_PERSIST_START_JSON\"' +\n",
                "      ':' +\n",
            )],
        ),
        leaves_no_scratch,
    );
}

#[test]
fn concurrent_ladders_do_not_collide() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = PlanRepo::new()?;
        let tmp = repo.tmpdir("shared")?;
        let script = ladder(
            &mut js,
            "plan",
            "reviewed",
            json!([]),
            "task/persist-target",
        )?;
        let spawn = |session: &str| {
            let mut cmd = Command::new("sh");
            hermetic(&mut cmd, &repo.root, session);
            cmd.arg("-c")
                .arg(&script)
                .env("TMPDIR", &tmp)
                .current_dir(repo.dir.path());
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
        };
        let a = spawn("one").map_err(|e| Failure::Infra(e.to_string()))?;
        let b = spawn("two").map_err(|e| Failure::Infra(e.to_string()))?;
        let a = a
            .wait_with_output()
            .map_err(|e| Failure::Infra(e.to_string()))?;
        let b = b
            .wait_with_output()
            .map_err(|e| Failure::Infra(e.to_string()))?;
        let (ida, idb) = (review_id(&a), review_id(&b));
        check!(
            a.status.success() && b.status.success(),
            "both ladders succeed: {} / {}",
            String::from_utf8_lossy(&a.stderr),
            String::from_utf8_lossy(&b.stderr)
        );
        check!(
            ida.is_some() && ida != idb,
            "they land distinct reviews: {ida:?} {idb:?}"
        );
        for id in [ida, idb].into_iter().flatten() {
            check_eq!(
                repo.review(&id)?["state"],
                json!("submitted"),
                "review {id} is submitted"
            );
        }
        Ok(())
    });
}
