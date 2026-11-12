//! The persist writer: its pure helpers, and the command ladder it emits run
//! under a real `sh` against the real `rdm` binary in a per-test plan repo.

use std::process::{Command, Output};

use rdm_devtools::workflow::Host;
use serde_json::{Value, json};

use crate::git_test_support::git;
use crate::plan_fixture::{PlanRepo, hermetic, rdm_bin};
use crate::support::{Agent, Failure, Js, Lib, Outcome, REVIEW_LIB, Reply, run_mutant, run_real};

pub(crate) const PROJECT: &str = "persist-verify";
const TASK_BODY: &str = "Alpha opening line.\nThe retry backoff strategy is unspecified here.\nA repeated sentence.\nA repeated sentence.\nTrailing \"quoted\" $dollar `backtick` — em-dash line.";

/// The persist fixture: [`PlanRepo`] with default project [`PROJECT`], a task
/// whose body carries quotable, repeated and shell-hostile lines, and a
/// roadmap with one phase.
pub(crate) fn seeded() -> Result<PlanRepo, Failure> {
    let repo = PlanRepo::init(PROJECT)?;
    repo.seed(&[
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
    ])?;
    Ok(repo)
}

/// The ladder `persistReviewCommands` emits, as one script.
pub(crate) fn ladder(
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
pub(crate) fn land(repo: &PlanRepo, script: &str, session: &str) -> Result<String, Failure> {
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

pub(crate) fn comments(review: &Value) -> Vec<Value> {
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
            json!({ "id": "g1", "concern": "specification", "severity": "blocking", "confidence": 90, "what_fails": "plain" }),
            json!({ "id": "n1", "concern": "scope", "severity": "suggestion", "confidence": 75, "what_fails": "ng", "unrefuted": true, "unrefutedReason": "non-gating" }),
            json!({ "id": "b1", "concern": "architectural-fit", "severity": "concern", "confidence": 80, "what_fails": "bg", "unrefuted": true, "unrefutedReason": "budget" }),
            json!({ "id": "e1", "concern": "specification", "severity": "blocking", "confidence": 99, "what_fails": "line one\nline two", "refuterError": true }),
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
        let absent = json!({ "$undefined": true });
        let on = json!({ "on": null });
        // A target with an rdm item behind it: an absent `persist` means on.
        let backed = [
            json!({ "kind": "roadmap" }),
            json!({ "kind": "phase" }),
            json!({ "kind": "task" }),
            json!({ "kind": "implementation-plan", "planSlug": "p" }),
        ];
        for target in backed {
            let cases = [
                (absent.clone(), on.clone()),
                (json!(null), on.clone()),
                (json!(false), json!(null)),
                (json!(true), on.clone()),
                (json!({}), on.clone()),
                (json!({ "on": "plan/x" }), json!({ "on": "plan/x" })),
            ];
            for (arg, want) in cases {
                check_eq!(
                    js.plan_call("resolvePersistArg", vec![arg.clone(), target.clone()])?,
                    want,
                    "resolvePersistArg({arg}, {target})"
                );
            }
        }
        // A plan with no slug has nothing to attach a review to, and so does a
        // call that names no target at all: absent means off.
        for target in [
            json!({ "kind": "implementation-plan" }),
            json!({ "kind": "implementation-plan", "planSlug": "" }),
            absent.clone(),
        ] {
            let cases = [
                (absent.clone(), json!(null)),
                (json!(null), json!(null)),
                (json!(false), json!(null)),
                (json!(true), on.clone()),
                (json!({}), on.clone()),
                (json!({ "on": "plan/x" }), json!({ "on": "plan/x" })),
            ];
            for (arg, want) in cases {
                check_eq!(
                    js.plan_call("resolvePersistArg", vec![arg.clone(), target.clone()])?,
                    want,
                    "resolvePersistArg({arg}, {target})"
                );
            }
        }
        for bad in [json!("task/x"), json!(["a"])] {
            let r = js.plan_try_call(
                "resolvePersistArg",
                vec![bad.clone(), json!({ "kind": "task" })],
            )?;
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
        let flagged = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "target": "--task t --persist" })],
        )?;
        let plain = js.plan_call("parsePlanArgs", vec![json!({ "target": "--task t" })])?;
        check_eq!(
            plain["persist"],
            json!({ "on": null }),
            "a task target persists by default"
        );
        check_eq!(
            flagged["persist"],
            plain["persist"],
            "persist is never tokenized out of the flag string"
        );
        let off = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "task": "t", "persist": false })],
        )?;
        check_eq!(off["persist"], json!(null), "an explicit false is off");
        let b = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "task": "t", "persist": true })],
        )?;
        check_eq!(
            b["persist"],
            json!({ "on": null }),
            "the structured key is honoured"
        );
        let free = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "implementationPlan": true, "planFile": "/tmp/x.md" })],
        )?;
        check_eq!(
            free["persist"],
            json!(null),
            "a plan given as a file is report-only"
        );
        check_eq!(
            free["persistIgnored"],
            json!(false),
            "and nothing was asked for, so nothing was ignored"
        );
        let slugged = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "implementationPlan": true, "planSlug": "p" })],
        )?;
        check_eq!(
            slugged["persist"],
            json!({ "on": null }),
            "a plan named by slug persists by default"
        );
        check_eq!(
            slugged["persistIgnored"],
            json!(false),
            "and nothing was ignored"
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
            vec![json!({ "id": "p1", "concern": "specification", "severity": "blocking", "confidence": 91, "what_fails": "the backoff is unspecified" })],
        )?;
        let reviews = json!([
            { "id": "2026-01-01-0000-aaaa", "state": "submitted", "created": "2026-01-01T00:00:00Z", "comments": [{ "body": "a human note" }] },
            { "id": "2026-01-02-0000-bbbb", "state": "submitted", "created": "2026-01-02T00:00:00Z",
              "comments": [{ "body": body }, { "body": "a human note with no header" }] }
        ]);
        let prior = js.plan_call("priorFindingsFromReviews", vec![reviews.clone()])?;
        check_eq!(
            prior,
            json!([{ "severity": "blocking", "concern": "specification", "what_fails": "the backoff is unspecified" }]),
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
            if call.label.contains("specification") {
                Reply::Value(
                    json!({ "findings": [{ "id": "q1", "concern": "specification", "severity": "blocking", "confidence": 90,
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
        let no_quote = json!({ "id": "f1", "concern": "specification", "severity": "blocking", "confidence": 90, "what_fails": "x" });
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
        let repo = seeded()?;
        let survivors = json!([
            { "id": "a1", "concern": "specification", "severity": "blocking", "confidence": 90, "what_fails": "the backoff is unspecified",
              "why": "no rule", "recommendation": "state it", "quote": "The retry backoff strategy is unspecified here." },
            { "id": "w1", "concern": "scope", "severity": "concern", "confidence": 80, "what_fails": "scope creep" },
            { "id": "c1", "concern": "architectural-fit", "severity": "concern", "confidence": 85, "what_fails": "a stale quote was cleared by the refuter" }
        ]);
        let script = ladder(&mut js, "plan", "rework", survivors, "task/persist-target")?;
        let id = land(&repo, &script, "ladder")?;
        let review = repo.review(PROJECT, &id)?;
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
        check_eq!(
            by_id["a1"]["dimension"],
            json!("specification"),
            "a1 dimension"
        );
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
        let repo = seeded()?;
        // Zero survivors still submits (the start body is never empty).
        let script = ladder(
            &mut js,
            "plan",
            "reviewed",
            json!([]),
            "task/persist-target",
        )?;
        let clean = repo.review(PROJECT, &land(&repo, &script, "clean")?)?;
        check_eq!(clean["verdict"], json!("approve"), "reviewed -> approve");
        check_eq!(
            clean["state"],
            json!("submitted"),
            "a zero-survivor review is submitted"
        );

        let esc = json!([{ "id": "e1", "concern": "specification", "severity": "blocking", "confidence": 95, "what_fails": "the goal is wrong" }]);
        let script = ladder(
            &mut js,
            "plan",
            "escalated",
            esc.clone(),
            "task/persist-target",
        )?;
        let escalated = repo.review(PROJECT, &land(&repo, &script, "esc")?)?;
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
        let code = repo.review(PROJECT, &land(&repo, &script, "esc-code")?)?;
        check!(
            code.to_string().contains("[code] escalated"),
            "an escalated code review carries the [code] prefix"
        );

        let script = ladder(&mut js, "plan", "rework", json!([]), "task/persist-target")?;
        let rework = repo.review(PROJECT, &land(&repo, &script, "rework")?)?;
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
        let repo = seeded()?;
        let survivors = json!([{ "id": "p1", "concern": "specification", "severity": "blocking", "confidence": 90,
                                 "what_fails": "phase gap", "quote": "a unique phase sentence" }]);
        let script = ladder(
            &mut js,
            "plan",
            "rework",
            survivors,
            "phase/persist-rm/phase-1-target",
        )?;
        let review = repo.review(PROJECT, &land(&repo, &script, "stem")?)?;
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
        let numeric = repo.review(PROJECT, &land(&repo, &script, "numeric")?)?;
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
        let repo = seeded()?;
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
            repo.reviews_on(PROJECT, "phase/persist-rm/phase-1-target")?
                .len(),
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
        let repo = seeded()?;
        let survivors = json!([{ "id": "amb", "concern": "specification", "severity": "blocking", "confidence": 90,
                                 "what_fails": "ambiguous", "quote": "A repeated sentence." }]);
        let script = ladder(&mut js, "plan", "rework", survivors, "task/persist-target")?;
        let tmp = repo.tmpdir("amb")?;
        let out = repo.sh(&script, "amb", &tmp)?;
        check!(
            !out.status.success(),
            "an ambiguous quote stops the ladder rather than guessing an occurrence"
        );
        let reviews = repo.reviews_on(PROJECT, "task/persist-target")?;
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
        let repo = seeded()?;
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
    let repo = seeded()?;
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
    let repo = seeded()?;
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
    let repo = seeded()?;
    let survivors = json!([{ "id": "f1", "concern": "specification", "severity": "blocking", "confidence": 90, "what_fails": "x" }]);
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
        let repo = seeded()?;
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
                repo.review(PROJECT, &id)?["state"],
                json!("submitted"),
                "review {id} is submitted"
            );
        }
        Ok(())
    });
}

// --- Line-span anchors (plan mode) ---------------------------------------------

/// A plan-mode run whose specification finder reports a line span on the task
/// body, and whose refuter answers `lines_ok` with `lines_ok`.
fn spanned_agent(lines_ok: Option<bool>) -> Agent {
    Agent::scripted(move |call| {
        if call.label.starts_with("find:") {
            if call.label.contains("specification") {
                Reply::Value(
                    json!({ "findings": [{ "id": "s1", "concern": "specification", "severity": "blocking", "confidence": 90,
                    "what_fails": "the retry backoff strategy is unspecified", "start_line": 2, "end_line": 3 }] }),
                )
            } else {
                Reply::Value(json!({ "findings": [] }))
            }
        } else {
            let mut v = json!({ "refuted": false, "confidence": 95 });
            if let Some(ok) = lines_ok {
                v["lines_ok"] = json!(ok);
            }
            Reply::Value(v)
        }
    })
}

#[test]
fn lines_ok_preserves_or_clears_span() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = seeded()?;
        let ctx = json!({ "target": "the plan" });
        for (ok, keeps) in [(Some(true), true), (None, true), (Some(false), false)] {
            let out = js.review_ok("plan", &spanned_agent(ok), ctx.clone())?;
            let f = crate::support::find(&out, "survivors", "s1")
                .ok_or_else(|| Failure::Check(format!("lines_ok {ok:?} dropped the finding")))?
                .clone();
            if keeps {
                check_eq!(
                    (f["start_line"].clone(), f["end_line"].clone()),
                    (json!(2), json!(3)),
                    "lines_ok {ok:?} preserves the span"
                );
                continue;
            }
            check!(
                f.get("start_line").is_none() && f.get("end_line").is_none(),
                "lines_ok false removes both span keys entirely: {f}"
            );
            // The stripped survivor, through the real ladder, lands whole-document.
            let script = ladder(&mut js, "plan", "rework", json!([f]), "task/persist-target")?;
            let review = repo.review(PROJECT, &land(&repo, &script, "stripped")?)?;
            let cs = comments(&review);
            check_eq!(cs.len(), 1, "one comment");
            check!(
                cs[0].get("anchor").is_none_or(Value::is_null),
                "a stripped span carries no anchor: {}",
                cs[0]
            );
            check_eq!(
                cs[0]["anchor_state"],
                json!("whole-document"),
                "recorded as whole-document on purpose"
            );
        }
        Ok(())
    });
}

#[test]
fn strip_span_is_pure() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let out = js.call(
            "stripSpan",
            vec![json!({ "id": "x", "start_line": 2, "end_line": 4, "quote": "q" })],
        )?;
        check_eq!(
            out,
            json!({ "id": "x", "quote": "q" }),
            "both span keys are removed, the rest kept"
        );
        let none = js.call("stripSpan", vec![json!({ "id": "y" })])?;
        check_eq!(
            none,
            json!({ "id": "y" }),
            "a span-less finding comes back unchanged"
        );
        Ok(())
    });
}

#[test]
fn span_changes_refute_prompt() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let dims = js.call("resolveReviewers", vec![json!("plan")])?;
        let dim = dims[0].clone();
        let no_span = json!({ "id": "f1", "concern": "specification", "severity": "blocking", "confidence": 90, "what_fails": "x" });
        let mut with_span = no_span.clone();
        with_span["start_line"] = json!(3);
        with_span["end_line"] = json!(4);
        let ctx = json!({ "target": "t" });
        let a = js.call(
            "refutePrompt",
            vec![json!("plan"), dim.clone(), no_span, ctx.clone()],
        )?;
        let b = js.call("refutePrompt", vec![json!("plan"), dim, with_span, ctx])?;
        check!(
            a != b,
            "a span-carrying finding gets a different refuter prompt"
        );
        Ok(())
    });
}

#[test]
fn reflowed_wrap_persists_with_its_line_anchor() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = seeded()?;
        let body = "Opening line.\nComments anchor on line ranges rather than quotes, since line numbers\nshift less often than a re-flowed quote does.\nClosing line.";
        repo.seed(&[&[
            "task",
            "create",
            "wrapped",
            "--title",
            "Wrapped",
            "--body",
            body,
            "--no-edit",
            "--project",
            PROJECT,
        ]])?;
        // Control: the re-flowed quote of the wrapped sentence does not
        // anchor, which is the failure the line span replaces.
        let start = repo.json(&[
            "review",
            "start",
            "--on",
            "task/wrapped",
            "--no-edit",
            "--format",
            "json",
            "--project",
            PROJECT,
        ])?;
        let id = start["id"].as_str().unwrap_or_default().to_owned();
        let refused = repo.rdm(
            "control",
            &[
                "review",
                "comment",
                &id,
                "--quote",
                "since line numbers shift less often",
                "--body",
                "control",
                "--no-edit",
                "--project",
                PROJECT,
            ],
        )?;
        check!(
            !refused.status.success(),
            "the re-flowed quote is refused: {}",
            String::from_utf8_lossy(&refused.stdout)
        );

        let survivors = json!([{ "id": "w1", "concern": "specification", "severity": "blocking", "confidence": 90,
                                 "what_fails": "the claim is unsupported", "start_line": 2, "end_line": 3 }]);
        let script = ladder(&mut js, "plan", "rework", survivors, "task/wrapped")?;
        let review = repo.review(PROJECT, &land(&repo, &script, "wrapped")?)?;
        let cs = comments(&review);
        check_eq!(cs.len(), 1, "one comment");
        let c = &cs[0];
        check_eq!(
            c["anchor"]["anchor_type"],
            json!("line-range"),
            "a line-range anchor"
        );
        check_eq!(
            (
                c["anchor"]["start_line"].clone(),
                c["anchor"]["end_line"].clone()
            ),
            (json!(2), json!(3)),
            "the requested span"
        );
        check_eq!(
            c["anchor"]["snippet"],
            json!(
                "Comments anchor on line ranges rather than quotes, since line numbers\nshift less often than a re-flowed quote does."
            ),
            "the snippet is the two original lines, wrap intact"
        );
        check_eq!(
            c["resolution"]["state"],
            json!("resolved"),
            "and it resolves"
        );
        let h = js.call("parseCommentHeader", vec![c["body"].clone()])?;
        check_eq!(
            h["anchor"],
            json!("line"),
            "the header records a line anchor"
        );
        Ok(())
    });
}

#[test]
fn refused_line_anchor_lands_whole_document_and_is_counted() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = seeded()?;
        let survivors = json!([{ "id": "o1", "concern": "specification", "severity": "blocking", "confidence": 90,
                                 "what_fails": "past the end", "start_line": 40, "end_line": 41 }]);
        let script = ladder(&mut js, "plan", "rework", survivors, "task/persist-target")?;
        let tmp = repo.tmpdir("oor")?;
        let out = repo.sh(&script, "oor", &tmp)?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        check!(
            out.status.success(),
            "a refused line anchor does not abort the ladder: {stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        check!(
            stdout.lines().any(|l| l == "anchorsDegraded=all"),
            "the lost anchor is counted: {stdout}"
        );
        check!(
            stdout.lines().any(|l| l == "anchorsParkRequired=yes"),
            "and line-out-of-range is not benign: {stdout}"
        );
        let id = review_id(&out).ok_or_else(|| Failure::Check(format!("no reviewId: {stdout}")))?;
        let review = repo.review(PROJECT, &id)?;
        let cs = comments(&review);
        check_eq!(cs.len(), 1, "the finding, and no note comment beside it");
        check_eq!(
            review["anchors_degraded"],
            json!(1),
            "the lost anchor is a review-level count"
        );
        let finding = cs
            .iter()
            .find(|c| {
                c["body"]
                    .as_str()
                    .is_some_and(|b| b.contains("finding-id: o1"))
            })
            .ok_or_else(|| Failure::Check(format!("no finding comment: {review}")))?;
        check!(
            finding.get("anchor").is_none_or(Value::is_null),
            "stored whole-document: {finding}"
        );
        check_eq!(
            finding["anchor_state"],
            json!("line-out-of-range"),
            "with core's reason"
        );
        check!(
            !cs.iter().any(|c| c["body"]
                .as_str()
                .is_some_and(|b| b.starts_with("persist-note"))),
            "no note comment is written: {review}"
        );
        Ok(())
    });
}

#[test]
fn change_target_ignores_a_span() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cfg = json!({ "rdmBin": "/fake/bin/rdm", "project": "demo" });
        let opts = json!({ "pathAnchors": true });
        let plain = json!({ "id": "c1", "concern": "correctness", "severity": "blocking", "confidence": 90,
                            "what_fails": "x", "quote": "shipped", "path": "src/foo.rs" });
        let mut spanned = plain.clone();
        spanned["start_line"] = json!(3);
        spanned["end_line"] = json!(5);
        let build = |js: &mut Js, f: &Value| {
            js.call(
                "persistReviewCommands",
                vec![
                    json!({ "mode": "code", "outcome": "rework", "survivors": [f] }),
                    json!("change/abc123"),
                    cfg.clone(),
                    opts.clone(),
                ],
            )
        };
        let a = build(&mut js, &plain)?;
        let b = build(&mut js, &spanned)?;
        check_eq!(
            b,
            a,
            "a span on a change target changes nothing (no --line)"
        );
        Ok(())
    });
}

#[test]
fn finding_line_span_normalizes_or_rejects() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cases = [
            (json!({ "start_line": 3 }), json!({ "start": 3, "end": 3 })),
            (
                json!({ "start_line": 3, "end_line": null }),
                json!({ "start": 3, "end": 3 }),
            ),
            (
                json!({ "start_line": 2, "end_line": 4 }),
                json!({ "start": 2, "end": 4 }),
            ),
            (
                json!({ "start_line": 1, "end_line": 1 }),
                json!({ "start": 1, "end": 1 }),
            ),
            (json!({ "start_line": 5, "end_line": 3 }), Value::Null),
            (json!({ "start_line": 0 }), Value::Null),
            (json!({ "start_line": -1 }), Value::Null),
            (json!({ "start_line": 2.5 }), Value::Null),
            (json!({ "start_line": "2" }), Value::Null),
            (json!({ "start_line": 2, "end_line": 3.5 }), Value::Null),
            (json!({ "start_line": 2, "end_line": "4" }), Value::Null),
            (
                json!({ "start_line": 9_007_199_254_740_992_u64 }),
                Value::Null,
            ),
            (json!({ "end_line": 4 }), Value::Null),
            (json!({ "quote": "q" }), Value::Null),
            (Value::Null, Value::Null),
        ];
        for (finding, want) in cases {
            let got = js.call("findingLineSpan", vec![finding.clone()])?;
            check_eq!(got, want, "findingLineSpan({finding})");
        }
        Ok(())
    });
}

#[test]
fn persist_requests_anchor_is_target_aware() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let span = json!({ "start_line": 2, "end_line": 3 });
        let bad_span = json!({ "start_line": 5, "end_line": 3 });
        let quote = json!({ "quote": "shipped" });
        let blank_quote = json!({ "quote": "   " });
        let cases = [
            (span.clone(), "task/x", true),
            (span.clone(), "phase/r/phase-1-a", true),
            (span, "change/abc123", false),
            (bad_span.clone(), "task/x", false),
            (quote.clone(), "task/x", true),
            (quote, "change/abc123", true),
            (blank_quote, "task/x", false),
            (json!({}), "task/x", false),
        ];
        for (finding, target, want) in cases {
            let got = js.call(
                "persistRequestsAnchor",
                vec![finding.clone(), json!(target)],
            )?;
            check_eq!(
                got,
                json!(want),
                "persistRequestsAnchor({finding}, {target})"
            );
        }
        Ok(())
    });
}

#[test]
fn plan_target_span_beats_quote() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cfg = json!({ "rdmBin": "/fake/bin/rdm", "project": "demo" });
        let base = json!({ "id": "p1", "concern": "specification", "severity": "blocking", "confidence": 90,
                           "what_fails": "x" });
        let mut span_only = base.clone();
        span_only["start_line"] = json!(2);
        span_only["end_line"] = json!(3);
        let mut span_and_quote = span_only.clone();
        span_and_quote["quote"] = json!("shipped");
        let mut quote_only = base.clone();
        quote_only["quote"] = json!("shipped");
        let mut bad_span_and_quote = quote_only.clone();
        bad_span_and_quote["start_line"] = json!(5);
        bad_span_and_quote["end_line"] = json!(3);
        let build = |js: &mut Js, f: &Value| {
            js.call(
                "persistReviewCommands",
                vec![
                    json!({ "mode": "plan", "outcome": "rework", "survivors": [f] }),
                    json!("task/x"),
                    cfg.clone(),
                ],
            )
        };
        let spanned = build(&mut js, &span_and_quote)?;
        let text = spanned.to_string();
        check!(
            text.contains("--line") && text.contains("2-3"),
            "a span-and-quote survivor emits --line 2-3: {text}"
        );
        check!(
            !text.contains("--quote"),
            "a span-and-quote survivor emits no --quote: {text}"
        );
        check_eq!(
            spanned,
            build(&mut js, &span_only)?,
            "the quote changes nothing once a span is present"
        );
        let inverted = build(&mut js, &bad_span_and_quote)?;
        let inverted_text = inverted.to_string();
        check!(
            inverted_text.contains("--quote") && !inverted_text.contains("--line"),
            "an end<start span falls back to the quote: {inverted_text}"
        );
        check_eq!(
            inverted,
            build(&mut js, &quote_only)?,
            "an invalid span persists exactly as the quote-only survivor"
        );
        Ok(())
    });
}

// --- Document-directed findings -------------------------------------------------

const PHASE_DOC: &str = "phase/rm/phase-2-dirty";
const PLAN_DOC: &str = "plan/my-plan";

fn document_opts() -> Value {
    json!({ "pathAnchors": true, "documents": [PHASE_DOC, PLAN_DOC] })
}

#[test]
fn persist_document_for_matches_the_consumers_refs() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let opts = document_opts();
        let numbered = json!({ "documents": ["phase/rm/2"] });
        let cases: Vec<(Value, Value, Value, &str)> = vec![
            (
                json!({ "path": PHASE_DOC }),
                opts.clone(),
                json!(PHASE_DOC),
                "an exact item ref",
            ),
            (
                json!({ "path": "rdm:phase/rm/phase-2-dirty" }),
                opts.clone(),
                json!(PHASE_DOC),
                "a leading rdm: is dropped",
            ),
            (
                json!({ "path": "phase/rm/phase-2-dirty:12" }),
                opts.clone(),
                json!(PHASE_DOC),
                "a line suffix is dropped",
            ),
            (
                json!({ "path": "  rdm:phase/rm/phase-2-dirty:3-9  " }),
                opts.clone(),
                json!(PHASE_DOC),
                "trimmed, prefix and range suffix dropped",
            ),
            (
                json!({ "path": "phase/rm/2" }),
                opts.clone(),
                json!(PHASE_DOC),
                "a phase number matches the entry's stem",
            ),
            (
                json!({ "path": PHASE_DOC }),
                numbered.clone(),
                json!("phase/rm/2"),
                "a stem matches a numbered entry, and the ENTRY is returned",
            ),
            (
                json!({ "path": "phase/rm/phase-20-later" }),
                numbered.clone(),
                Value::Null,
                "phase-20- is not phase 2",
            ),
            (
                json!({ "path": "phase/rm/22" }),
                opts.clone(),
                Value::Null,
                "another number",
            ),
            (
                json!({ "path": "phase/rm/phase-1-clean" }),
                opts.clone(),
                Value::Null,
                "another phase of the same roadmap",
            ),
            (
                json!({ "path": "phase/other/phase-2-dirty" }),
                opts.clone(),
                Value::Null,
                "the same stem in another roadmap",
            ),
            (
                json!({ "path": "phase/other/2" }),
                opts.clone(),
                Value::Null,
                "the same number in another roadmap",
            ),
            (
                json!({ "path": PLAN_DOC }),
                opts.clone(),
                json!(PLAN_DOC),
                "the plan ref",
            ),
            (
                json!({ "path": "rdm:plan/my-plan" }),
                opts.clone(),
                json!(PLAN_DOC),
                "the plan ref as an rdm: link",
            ),
            (
                json!({ "path": "plan/another-plan" }),
                opts.clone(),
                Value::Null,
                "another plan",
            ),
            (
                json!({ "path": "src/lib.rs" }),
                opts.clone(),
                Value::Null,
                "an ordinary source path",
            ),
            (
                json!({ "path": 42 }),
                opts.clone(),
                Value::Null,
                "a non-string path and no location",
            ),
            (
                json!({ "location": "phase/rm/phase-2-dirty:4" }),
                opts.clone(),
                json!(PHASE_DOC),
                "no path: the location is the candidate",
            ),
            (
                json!({ "path": "   ", "location": PLAN_DOC }),
                opts.clone(),
                json!(PLAN_DOC),
                "a blank path falls to the location",
            ),
            (
                json!({ "path": "src/lib.rs", "location": PHASE_DOC }),
                opts.clone(),
                Value::Null,
                "a non-blank path wins over the location",
            ),
            (
                json!({ "location": "the acceptance criteria section" }),
                opts.clone(),
                Value::Null,
                "prose",
            ),
            (
                json!({ "path": PHASE_DOC }),
                json!({ "pathAnchors": true }),
                Value::Null,
                "no documents option",
            ),
            (
                json!({ "path": PHASE_DOC }),
                json!({ "documents": "phase/rm/phase-2-dirty" }),
                Value::Null,
                "documents that is not a list",
            ),
            (
                json!({ "path": PHASE_DOC }),
                json!({ "documents": [42, null] }),
                Value::Null,
                "non-string entries",
            ),
            (Value::Null, opts.clone(), Value::Null, "no finding at all"),
        ];
        for (finding, opts, want, why) in cases {
            check_eq!(
                js.call("persistDocumentFor", vec![finding.clone(), opts])?,
                want,
                "{why}: {finding}"
            );
        }
        check_eq!(
            js.call("persistDocumentFor", vec![json!({ "path": PHASE_DOC })])?,
            Value::Null,
            "no opts at all"
        );
        Ok(())
    });
}

#[test]
fn a_document_finding_is_not_a_requested_anchor() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let target = json!("change/abc123");
        let opts = document_opts();
        let about_doc = json!({ "id": "d1", "concern": "ac", "severity": "concern", "confidence": 90,
                                "what_fails": "the criterion is vague", "path": PHASE_DOC,
                                "quote": "- AC1: it works" });
        let lost = json!({ "id": "l1", "concern": "tests", "severity": "concern", "confidence": 90,
                           "what_fails": "x", "location": "throughout", "quote": "shipped" });

        let states = js.get("PERSIST_ANCHOR_STATES")?;
        check!(
            states
                .as_array()
                .is_some_and(|a| a.contains(&json!("document"))),
            "document is in the closed header vocabulary: {states}"
        );
        for noisy in [opts.clone(), {
            // An empty reviewed range changes nothing for a document finding.
            let mut o = opts.clone();
            o["source"] = json!({ "noCode": true });
            o
        }] {
            let decision = js.call(
                "persistAnchorFor",
                vec![about_doc.clone(), target.clone(), noisy.clone()],
            )?;
            check_eq!(
                decision,
                json!({ "quote": false, "path": null, "reason": null, "intendedPath": null,
                        "line": null, "document": PHASE_DOC }),
                "a document decision"
            );
            check_eq!(
                js.call(
                    "persistAnchorState",
                    vec![about_doc.clone(), target.clone(), noisy.clone()]
                )?,
                json!("document"),
                "the header records the request"
            );
            check_eq!(
                js.call(
                    "persistRequestsAnchor",
                    vec![about_doc.clone(), target.clone(), noisy]
                )?,
                json!(false),
                "a document finding requests no source anchor"
            );
        }
        // Without the option the same finding is an ordinary anchor request.
        check_eq!(
            js.call(
                "persistRequestsAnchor",
                vec![about_doc.clone(), target.clone()]
            )?,
            json!(true),
            "default-off"
        );
        check_eq!(
            js.call(
                "persistAnchorFor",
                vec![lost.clone(), target.clone(), opts.clone()]
            )?["document"],
            Value::Null,
            "every other decision carries document: null"
        );
        // A plan-repo document target never consults the option.
        check_eq!(
            js.call(
                "persistAnchorFor",
                vec![
                    about_doc.clone(),
                    json!("task/x"),
                    json!({ "documents": [PHASE_DOC] })
                ]
            )?["document"],
            Value::Null,
            "only a change target has document findings"
        );

        let only = json!({ "mode": "code", "outcome": "rework", "survivors": [about_doc] });
        check_eq!(
            js.call(
                "persistDegradedSummary",
                vec![only.clone(), target.clone(), opts.clone()]
            )?,
            json!({ "requested": 0, "degraded": 0, "all": false }),
            "left out of requested and degraded"
        );
        check_eq!(
            js.call(
                "persistPreDegradedAnchors",
                vec![only.clone(), target.clone(), opts.clone()]
            )?,
            json!([]),
            "not a build-time drop"
        );
        check_eq!(
            js.call(
                "persistDegradationClause",
                vec![only, target.clone(), opts.clone()]
            )?,
            json!(""),
            "no degradation clause"
        );
        let mixed = json!({ "mode": "code", "outcome": "rework",
                            "survivors": [{ "id": "d1", "path": PLAN_DOC, "quote": "q" }, lost] });
        check_eq!(
            js.call("persistDegradedSummary", vec![mixed, target, opts])?,
            json!({ "requested": 1, "degraded": 1, "all": true }),
            "only the real anchor request is counted"
        );
        Ok(())
    });
}

#[test]
fn a_document_finding_emits_one_document_comment_naming_the_consumers_ref() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cfg = json!({ "rdmBin": "/fake/bin/rdm", "project": "demo" });
        // The finder spells the ref its own way; the ladder passes the
        // consumer's entry.
        let survivors = json!([{ "id": "d1", "concern": "ac", "severity": "blocking", "confidence": 90,
                                 "what_fails": "x", "path": "rdm:phase/rm/2:14", "quote": "- AC1: it works" }]);
        let cmds = js.call(
            "persistReviewCommands",
            vec![
                json!({ "mode": "code", "outcome": "rework", "survivors": survivors }),
                json!("change/abc123"),
                cfg,
                document_opts(),
            ],
        )?;
        let lines: Vec<String> = cmds
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .flat_map(|c| c.lines().map(str::to_owned).collect::<Vec<_>>())
            .collect();
        let comment_lines: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains(" review comment "))
            .collect();
        check_eq!(comment_lines.len(), 1, "one comment command: {lines:?}");
        check_eq!(
            comment_lines[0].trim(),
            "/fake/bin/rdm review comment \"$RDM_REVIEW_ID\" --document 'phase/rm/phase-2-dirty' --body \"$RDM_PERSIST_BODY\" --no-edit --project demo < /dev/null || exit 1",
            "the document comment line"
        );
        check!(
            lines.iter().any(|l| l == "RDM_PERSIST_PARK_REQUIRED=0"),
            "a document finding never seeds a park: {lines:?}"
        );
        Ok(())
    });
}

#[test]
fn documents_are_refused_on_a_document_target_and_for_bad_entries() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cfg = json!({ "rdmBin": "/fake/bin/rdm", "project": "demo" });
        let result = json!({ "mode": "code", "outcome": "reviewed", "survivors": [] });
        let on_task = js.try_call(
            "persistReviewCommands",
            vec![
                result.clone(),
                json!("task/x"),
                cfg.clone(),
                json!({ "documents": ["task/x"] }),
            ],
        )?;
        check!(
            on_task
                .as_ref()
                .is_err_and(|e| e.message.contains("opts.documents")),
            "documents on a plan-repo document target throws: {on_task:?}"
        );
        for bad in [
            json!([""]),
            json!([42]),
            json!(["no-slash"]),
            json!(["change/abc123"]),
            json!([PHASE_DOC, "  "]),
            json!("phase/rm/phase-2-dirty"),
        ] {
            let r = js.try_call(
                "persistReviewCommands",
                vec![
                    result.clone(),
                    json!("change/abc123"),
                    cfg.clone(),
                    json!({ "documents": bad }),
                ],
            )?;
            check!(
                r.as_ref()
                    .is_err_and(|e| e.message.contains("opts.documents")),
                "documents {bad} throws: {r:?}"
            );
        }
        let ok = js.try_call(
            "persistReviewCommands",
            vec![
                result,
                json!("change/abc123"),
                cfg,
                json!({ "documents": [PHASE_DOC, PLAN_DOC] }),
            ],
        )?;
        check!(ok.is_ok(), "well-formed documents are accepted: {ok:?}");
        Ok(())
    });
}

#[test]
fn follows_is_refused_on_a_document_target_and_for_bad_values() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cfg = json!({ "rdmBin": "/fake/bin/rdm", "project": "demo" });
        let result = json!({ "mode": "code", "outcome": "reviewed", "survivors": [] });
        let mut persist = |target: &str, opts: Value| {
            js.try_call(
                "persistReviewCommands",
                vec![result.clone(), json!(target), cfg.clone(), opts],
            )
        };
        let on_task = persist("task/x", json!({ "follows": "2026-11-11-0900-abcd" }))?;
        check!(
            on_task
                .as_ref()
                .is_err_and(|e| e.message.contains("opts.follows")),
            "follows on a plan-repo document target throws: {on_task:?}"
        );
        for bad in [
            json!(""),
            json!("   "),
            json!(42),
            json!(true),
            json!(["id"]),
        ] {
            let r = persist("change/abc123", json!({ "follows": bad }))?;
            check!(
                r.as_ref()
                    .is_err_and(|e| e.message.contains("opts.follows")),
                "follows {bad} throws: {r:?}"
            );
        }
        let ok = persist(
            "change/abc123",
            json!({ "follows": "2026-11-11-0900-abcd" }),
        )?;
        check!(ok.is_ok(), "a review id is accepted: {ok:?}");
        let absent = persist("change/abc123", json!({}))?;
        let undefined = persist("change/abc123", json!({ "follows": Host::undefined() }))?;
        check!(
            absent.is_ok() && absent.as_ref().ok() == undefined.as_ref().ok(),
            "an undefined follows emits what an absent one does"
        );
        Ok(())
    });
}

// --- The gate comment of a rework with nothing to act on -------------------------

fn complete_evidence(ac: Value) -> Value {
    json!({ "coverage": { "complete": true, "acDimensionRan": true }, "budget": {},
            "acTable": ac, "survivors": [] })
}

#[test]
fn persist_gate_finding_names_what_gated() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let pass =
            json!({ "criterion": "AC1: it works", "status": "PASS", "evidence": "tests cover it" });
        let fail = json!({ "criterion": "AC2: it is documented", "status": "FAIL",
                           "evidence": "no entry in\nCHANGELOG.md" });
        let partial = json!({ "criterion": "AC3: both flavors render", "status": "PARTIAL",
                              "evidence": "markdown only" });
        let rework = |evidence: Value| json!({ "mode": "code", "outcome": "rework", "survivors": [], "evidence": evidence });

        let g = js.call(
            "persistGateFinding",
            vec![rework(complete_evidence(json!([pass, fail])))],
        )?;
        check_eq!(g["severity"], json!("blocking"), "blocking");
        check_eq!(g["concern"], json!("ac"), "an acceptance-criteria gate");
        check!(
            g["id"].as_str().is_some_and(|s| !s.is_empty()),
            "a fixed id: {g}"
        );
        let what = g["what_fails"].as_str().unwrap_or_default().to_owned();
        check!(
            what.contains("AC2: it is documented") && what.contains("FAIL"),
            "names the unmet criterion and its status: {what}"
        );
        check!(
            what.contains("no entry in CHANGELOG.md"),
            "carries the row's evidence on one line: {what}"
        );
        check!(!what.contains('\n'), "one line: {what}");
        check!(
            !what.contains("AC1: it works"),
            "a PASS row is not named: {what}"
        );

        let g = js.call(
            "persistGateFinding",
            vec![rework(complete_evidence(json!([pass, partial, fail])))],
        )?;
        let what = g["what_fails"].as_str().unwrap_or_default().to_owned();
        check!(
            what.contains("AC3: both flavors render")
                && what.contains("PARTIAL")
                && what.contains("markdown only")
                && what.contains("AC2: it is documented"),
            "every FAIL and PARTIAL row is named: {what}"
        );

        // No gap in the table, and evidence that is not complete.
        let g = js.call(
            "persistGateFinding",
            vec![rework(json!({ "acTable": [pass] }))],
        )?;
        check!(
            g["what_fails"]
                .as_str()
                .is_some_and(|s| s.contains("evidence is incomplete")),
            "an evidence gap is named: {g}"
        );
        // No gap and complete evidence: nothing more specific to say.
        let g = js.call(
            "persistGateFinding",
            vec![rework(complete_evidence(json!([pass])))],
        )?;
        check!(
            g["what_fails"]
                .as_str()
                .is_some_and(|s| !s.contains("evidence is incomplete") && s.contains("rework")),
            "the bare outcome is stated: {g}"
        );

        for none in [
            json!({ "mode": "code", "outcome": "reviewed", "survivors": [],
                    "evidence": complete_evidence(json!([pass, fail])) }),
            json!({ "mode": "code", "outcome": "escalated", "survivors": [],
                    "evidence": complete_evidence(json!([pass, fail])) }),
            json!({ "mode": "code", "outcome": "rework",
                    "survivors": [{ "id": "s1", "severity": "blocking", "what_fails": "x" }],
                    "evidence": complete_evidence(json!([pass, fail])) }),
            Value::Null,
        ] {
            check_eq!(
                js.call("persistGateFinding", vec![none.clone()])?,
                Value::Null,
                "no gate finding for {none}"
            );
        }
        Ok(())
    });
}

/// A plan-mode rework with no survivors closes the same way: the gate comment
/// is an ordinary headered whole-document comment, written before submit.
#[test]
fn a_survivorless_rework_lands_one_gate_comment() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let repo = seeded()?;
        let script = ladder(&mut js, "plan", "rework", json!([]), "task/persist-target")?;
        let review = repo.review(PROJECT, &land(&repo, &script, "gate")?)?;
        check_eq!(review["verdict"], json!("request-changes"), "the verdict");
        let cs = comments(&review);
        check_eq!(cs.len(), 1, "exactly the gate comment: {review}");
        check_eq!(
            cs[0]["anchor_state"],
            json!("whole-document"),
            "whole-document on purpose"
        );
        check_eq!(review["anchors_degraded"], json!(0), "and not counted");
        let h = js.call("parseCommentHeader", vec![cs[0]["body"].clone()])?;
        check_eq!(h["severity"], json!("blocking"), "headered like a finding");
        check_eq!(h["anchor"], json!("wholeDocumentIntended"), "no anchor");

        // A reviewed outcome with no survivors still carries no comment.
        let script = ladder(
            &mut js,
            "plan",
            "reviewed",
            json!([]),
            "task/persist-target",
        )?;
        let clean = repo.review(PROJECT, &land(&repo, &script, "clean")?)?;
        check_eq!(comments(&clean).len(), 0, "a clean review has no comment");
        Ok(())
    });
}
