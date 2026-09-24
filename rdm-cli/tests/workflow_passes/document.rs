//! The roadmap-documentation pass.
//!
//! Imports the canonical `.claude/workflows/lib/document.mjs` for the
//! decision helpers and loads the real `.claude/workflows/rdm-wf-document.js`
//! driver for the engine cases. Real `roadmap show` / `phase show` JSON from
//! a seeded plan repo, and real commits in a source repo, drive the
//! decisions; the read commands, git range commands and the write script the
//! pass returns are executed. Ported from `scripts/verify-workflow-document.sh`
//! and the document half of `scripts/lib/workflow-env-args.test.mjs`; the two
//! engine cases are new (the shell never ran the driver).

use serde_json::{Value, json};

use rdm_devtools::workflow::Host;

use crate::generators::Generator;
use crate::plan_fixture::{PlanRepo, Run, SourceRepo, commit_file, rdm_bin};
use crate::workflow_support::{Agent, Failure, Module, Reply, run_driver, run_real};

const LIB: &str = ".claude/workflows/lib/document.mjs";
const ENGINE: &str = ".claude/workflows/rdm-wf-document.js";
const PROJECT: &str = "doc-verify";

const GEN: Generator = Generator {
    script: "scripts/gen-workflow-document.sh",
    lib: LIB,
    engine: ENGINE,
    template: "rdm-core/src/templates/workflows/rdm-wf-document.js",
    plugin: "plugins/rdm/workflows/rdm-wf-document.js",
    begin: ">>> document-core:begin",
};

fn rejects(r: &Result<Value, rdm_devtools::workflow::JsError>, needle: &str) -> bool {
    r.as_ref().is_err_and(|e| e.message.contains(needle))
}

/// A plan repo with three roadmaps — `rm-done` (two done phases with real
/// commits), `rm-incomplete` (one in-progress phase) and `rm-no-sha` (a done
/// phase with no commit) — and the source repo holding those commits.
struct Fx {
    repo: PlanRepo,
    src: SourceRepo,
    sha_one: String,
    sha_two: String,
}

fn fixture(default_project: Option<&str>) -> Result<Fx, Failure> {
    let repo = match default_project {
        Some(p) => PlanRepo::init(p)?,
        None => PlanRepo::with_decoy(&[PROJECT])?,
    };
    let src = SourceRepo::init(&repo)?;
    let sha_one = commit_file(&src.root, "a.txt", "one\n", "feat: add a.txt");
    let sha_two = commit_file(&src.root, "b.txt", "two\n", "feat: add b.txt");
    let roadmap = |slug: &'static str, body: &'static str| {
        vec![
            "roadmap",
            "create",
            slug,
            "--title",
            slug,
            "--body",
            body,
            "--no-edit",
            "--project",
            PROJECT,
        ]
    };
    let phase = |slug: &'static str, n: &'static str, rm: &'static str| {
        vec![
            "phase",
            "create",
            slug,
            "--title",
            slug,
            "--number",
            n,
            "--body",
            "Phase body.",
            "--no-edit",
            "--roadmap",
            rm,
            "--project",
            PROJECT,
        ]
    };
    let done = |stem: &'static str, rm: &'static str, sha: Option<&str>| {
        let mut v = vec![
            "phase".to_owned(),
            "update".to_owned(),
            stem.to_owned(),
            "--status".to_owned(),
            "done".to_owned(),
            "--no-edit".to_owned(),
            "--roadmap".to_owned(),
            rm.to_owned(),
            "--project".to_owned(),
            PROJECT.to_owned(),
        ];
        if let Some(s) = sha {
            v.extend(["--commit".to_owned(), s.to_owned()]);
        }
        v
    };
    let d1 = done("phase-1-one", "rm-done", Some(&sha_one));
    let d2 = done("phase-2-two", "rm-done", Some(&sha_two));
    let d3 = done("phase-1-bare", "rm-no-sha", None);
    repo.seed(&[
        &roadmap("rm-done", "All phases complete."),
        &phase("one", "1", "rm-done"),
        &phase("two", "2", "rm-done"),
        &refs(&d1),
        &refs(&d2),
        &roadmap("rm-incomplete", "Still in flight."),
        &phase("started", "1", "rm-incomplete"),
        &[
            "phase",
            "update",
            "phase-1-started",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "rm-incomplete",
            "--project",
            PROJECT,
        ],
        &roadmap("rm-no-sha", "Done, but no commit recorded."),
        &phase("bare", "1", "rm-no-sha"),
        &refs(&d3),
    ])?;
    Ok(Fx {
        repo,
        src,
        sha_one,
        sha_two,
    })
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn roadmap_json(fx: &Fx, slug: &str) -> Result<Value, Failure> {
    fx.repo.json(&[
        "roadmap",
        "show",
        slug,
        "--project",
        PROJECT,
        "--format",
        "json",
        "--no-body",
    ])
}

fn stems(v: &Value) -> Vec<Value> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|p| p["stem"].clone())
        .collect()
}

// --- Pure decisions ---------------------------------------------------------------

#[test]
fn parse_document_args() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let want = |roadmap: &str, out: &str| json!({ "roadmap": roadmap, "out": out, "rdmBin": "rdm", "project": "" });
        for (args, expected) in [
            (
                json!({ "roadmap": "rm", "out": "x.md" }),
                want("rm", "x.md"),
            ),
            (json!({ "roadmap": "rm" }), want("rm", "")),
            (json!({}), want("", "")),
            (Host::undefined(), want("", "")),
            (Value::Null, want("", "")),
            (json!("not json"), want("", "")),
            (
                json!(json!({ "roadmap": "stringified-rm", "out": "docs/z.md" }).to_string()),
                want("stringified-rm", "docs/z.md"),
            ),
            (json!(42), want("", "")),
        ] {
            check_eq!(
                m.call("parseDocumentArgs", vec![args.clone()])?,
                expected,
                "{args}"
            );
        }
        Ok(())
    });
}

#[test]
fn out_path_resolution() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        check_eq!(
            m.call("defaultOutPath", vec![json!("my-roadmap")])?,
            json!("docs/my-roadmap.md"),
            "default"
        );
        for (args, want) in [
            (
                json!({ "roadmap": "my-roadmap", "out": "" }),
                "docs/my-roadmap.md",
            ),
            (
                json!({ "roadmap": "my-roadmap", "out": "custom/path.md" }),
                "custom/path.md",
            ),
            (json!({ "roadmap": "my-roadmap" }), "docs/my-roadmap.md"),
        ] {
            check_eq!(
                m.call("resolveOutPath", vec![args.clone()])?,
                json!(want),
                "{args}"
            );
        }
        Ok(())
    });
}

#[test]
fn compute_incomplete_phases() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        for (phases, want) in [
            (
                json!([{ "stem": "a", "status": "done" }, { "stem": "b", "status": "done" }]),
                json!([]),
            ),
            (
                json!([{ "stem": "a", "status": "done" }, { "stem": "b", "status": "in-progress" }]),
                json!([{ "stem": "b", "status": "in-progress" }]),
            ),
            (json!([]), json!([])),
            (Host::undefined(), json!([])),
            (Value::Null, json!([])),
        ] {
            check_eq!(
                m.call("computeIncompletePhases", vec![phases.clone()])?,
                want,
                "{phases}"
            );
        }
        Ok(())
    });
}

#[test]
fn git_range_commands() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let r = m.call("buildGitRangeCommands", vec![json!("abc123")])?;
        check_eq!(r["hasSha"], json!(true), "a SHA yields commands");
        check!(
            r["log"].is_string() && r["diffStat"].is_string(),
            "both commands are built: {r}"
        );
        for bad in [
            json!(""),
            Host::undefined(),
            Value::Null,
            json!(0),
            json!(42),
        ] {
            check_eq!(
                m.call("buildGitRangeCommands", vec![bad.clone()])?,
                json!({ "hasSha": false, "log": null, "diffStat": null }),
                "{bad} has no SHA"
            );
        }
        Ok(())
    });
}

// --- Real rdm JSON and real commits -------------------------------------------------

#[test]
fn real_roadmap_json_drives_decisions() {
    run_real(|lib| {
        let fx = fixture(None)?;
        let mut m = Module::open(lib, LIB)?;
        let done = roadmap_json(&fx, "rm-done")?;
        check_eq!(
            done["phases"].as_array().map(Vec::len),
            Some(2),
            "two phases"
        );
        check_eq!(
            m.call("computeIncompletePhases", vec![done["phases"].clone()])?,
            json!([]),
            "a real fully-done roadmap has no incomplete phase"
        );
        for (stem, sha) in [("phase-1-one", &fx.sha_one), ("phase-2-two", &fx.sha_two)] {
            let phase = fx.repo.phase(PROJECT, "rm-done", stem)?;
            check_eq!(
                phase["commit"],
                json!(sha),
                "{stem} records its real commit"
            );
            check_eq!(
                m.call("buildGitRangeCommands", vec![phase["commit"].clone()])?["hasSha"],
                json!(true),
                "{stem}: the real commit field drives the has-SHA branch"
            );
        }
        check_eq!(
            m.call(
                "resolveOutPath",
                vec![json!({ "roadmap": done["slug"], "out": "" })]
            )?,
            json!("docs/rm-done.md"),
            "the real slug names the default out path"
        );
        let incomplete = m.call(
            "computeIncompletePhases",
            vec![roadmap_json(&fx, "rm-incomplete")?["phases"].clone()],
        )?;
        check_eq!(
            stems(&incomplete),
            vec![json!("phase-1-started")],
            "the in-flight phase"
        );
        check_eq!(
            incomplete[0]["status"],
            json!("in-progress"),
            "with its status"
        );
        let bare = fx.repo.phase(PROJECT, "rm-no-sha", "phase-1-bare")?;
        check_eq!(bare["status"], json!("done"), "done");
        check!(bare.get("commit").is_none(), "no commit recorded: {bare}");
        check_eq!(
            m.call(
                "buildGitRangeCommands",
                vec![bare.get("commit").cloned().unwrap_or(Host::undefined())]
            )?["hasSha"],
            json!(false),
            "a missing commit falls back to body-only"
        );
        Ok(())
    });
}

#[test]
fn git_range_commands_execute_in_source_repo() {
    run_real(|lib| {
        let fx = fixture(None)?;
        let mut m = Module::open(lib, LIB)?;
        let run = Run::bare().cwd(&fx.src.root);
        for (sha, file, subject) in [
            (&fx.sha_one, "a.txt", "feat: add a.txt"),
            (&fx.sha_two, "b.txt", "feat: add b.txt"),
        ] {
            let r = m.call("buildGitRangeCommands", vec![json!(sha)])?;
            let log = fx
                .repo
                .run_ok(&run, r["log"].as_str().unwrap_or_default())?;
            check!(
                log.lines().count() == 1
                    && sha.starts_with(log.split(' ').next().unwrap_or("-"))
                    && log.contains(subject),
                "the log names exactly the phase's commit: {log}"
            );
            let stat = fx
                .repo
                .run_ok(&run, r["diffStat"].as_str().unwrap_or_default())?;
            check!(
                stat.contains(file) && stat.contains("1 file changed"),
                "the diff stat names the phase's file: {stat}"
            );
        }
        Ok(())
    });
}

#[test]
fn read_commands_execute_with_injected_axes() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let check_reads =
            |m: &mut Module, fx: &Fx, args: Value, run: &Run| -> Result<(), Failure> {
                let cfg = m.call("parseDocumentArgs", vec![args])?;
                let rm = m.call(
                    "documentRoadmapCommand",
                    vec![json!("rm-done"), cfg.clone()],
                )?;
                let out = fx.repo.run_ok(run, rm.as_str().unwrap_or_default())?;
                let v: Value = serde_json::from_str(&out)
                    .map_err(|e| Failure::Check(format!("{e}: {out}")))?;
                check_eq!(
                    v["slug"],
                    json!("rm-done"),
                    "the roadmap command reads the roadmap"
                );
                let ph = m.call(
                    "documentPhaseCommand",
                    vec![json!("rm-done"), json!("phase-1-one"), cfg],
                )?;
                let out = fx.repo.run_ok(run, ph.as_str().unwrap_or_default())?;
                let v: Value = serde_json::from_str(&out)
                    .map_err(|e| Failure::Check(format!("{e}: {out}")))?;
                check_eq!(
                    v["stem"],
                    json!("phase-1-one"),
                    "the phase command reads the phase"
                );
                Ok(())
            };
        let fx = fixture(None)?;
        check_reads(
            &mut m,
            &fx,
            json!({ "roadmap": "rm-done", "rdmBin": rdm_bin(), "project": PROJECT }),
            &Run::bare(),
        )?;
        // Omitted axes: a plain `rdm` on PATH and the default project.
        let home = fixture(Some(PROJECT))?;
        let run = Run::bare().path_prepend(&home.repo.rdm_on_path()?);
        check_reads(&mut m, &home, json!({ "roadmap": "rm-done" }), &run)?;
        let cfg = m.call("parseDocumentArgs", vec![json!({ "roadmap": "rm-done" })])?;
        let rm = m.call("documentRoadmapCommand", vec![json!("rm-done"), cfg])?;
        check!(
            !home
                .repo
                .run(&Run::bare(), rm.as_str().unwrap_or_default())?
                .status
                .success(),
            "the omitted-axes command names a plain `rdm`"
        );
        Ok(())
    });
}

#[test]
fn invalid_axes_refused_at_parse() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        for (args, needle) in [
            (
                json!({ "roadmap": "r", "rdmBin": 42 }),
                "rdmBin must be a string path",
            ),
            (
                json!({ "roadmap": "r", "project": "a b" }),
                "plain project name",
            ),
            (
                json!({ "roadmap": "r", "project": "a;rm -rf /" }),
                "plain project name",
            ),
        ] {
            let r = m.try_call("parseDocumentArgs", vec![args.clone()])?;
            check!(rejects(&r, needle), "{args}: {r:?}");
        }
        check_eq!(
            m.call("resolveRdmBin", vec![Host::undefined()])?,
            json!("rdm"),
            "default binary"
        );
        check_eq!(
            m.call("parseProjectArg", vec![Host::undefined()])?,
            json!(""),
            "no project"
        );
        check_eq!(
            m.call("projectFlag", vec![json!({ "project": "demo" })])?,
            json!(" --project demo"),
            "the flag"
        );
        check_eq!(
            m.call("projectFlag", vec![json!({})])?,
            json!(""),
            "no flag"
        );
        Ok(())
    });
}

// --- The engine --------------------------------------------------------------------

fn engine_args(extra: Value) -> Value {
    let mut out = json!({ "rdmBin": rdm_bin(), "project": PROJECT });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        out[k] = v;
    }
    out
}

/// The caller's side of the contract: run the engine with no `roadmapMeta`,
/// execute the `roadmap show` command it names, and hand the JSON back.
fn roadmap_meta(lib: &crate::workflow_support::Lib, fx: &Fx, slug: &str) -> Result<Value, Failure> {
    let agent = Agent::scripted(|_| Reply::Throw("no agent may run".into()));
    let r = run_driver(lib, ENGINE, engine_args(json!({ "roadmap": slug })), &agent)?
        .map_err(Failure::Js)?;
    check_eq!(r["fetchError"], json!(true), "no meta, no run: {r}");
    check!(agent.calls().is_empty(), "no agent without the phase list");
    let out = fx.repo.run_ok(
        &Run::bare(),
        r["roadmapCommand"].as_str().unwrap_or_default(),
    )?;
    let mut meta: Value =
        serde_json::from_str(&out).map_err(|e| Failure::Check(format!("{e}: {out}")))?;
    meta["found"] = json!(true);
    Ok(meta)
}

#[test]
fn engine_aborts_incomplete_before_any_agent() {
    run_real(|lib| {
        let fx = fixture(None)?;
        let meta = roadmap_meta(lib, &fx, "rm-incomplete")?;
        let agent = Agent::scripted(|_| Reply::Throw("no agent may run".into()));
        let r = run_driver(
            lib,
            ENGINE,
            engine_args(json!({ "roadmap": "rm-incomplete", "roadmapMeta": meta })),
            &agent,
        )?
        .map_err(Failure::Js)?;
        check_eq!(r["aborted"], json!(true), "an incomplete roadmap aborts");
        check_eq!(
            stems(&r["incompletePhases"]),
            vec![json!("phase-1-started")],
            "naming the phase"
        );
        check_eq!(r["path"], Value::Null, "nothing to write");
        check!(agent.calls().is_empty(), "before any gather or synthesis");
        Ok(())
    });
}

#[test]
fn engine_gathers_then_synthesizes_and_writes() {
    run_real(|lib| {
        let fx = fixture(None)?;
        let meta = roadmap_meta(lib, &fx, "rm-done")?;
        let shipped = "SENTINEL-SHIPPED-phase-one-4b2e";
        let draft =
            "# Feature\n\nIt's `done` — costs $HOME and \"quotes\".\n\n## Usage\n\n    rdm thing";
        let agent = Agent::scripted(move |call| match call.label.as_str() {
            "gather:phase-1-one" => Reply::Value(
                json!({ "stem": "phase-1-one", "title": "one", "shipped": shipped,
                                                         "hasSha": true, "fallback": false }),
            ),
            "gather:phase-2-two" => Reply::Throw("gatherer crashed".into()),
            "synthesize:draft" => Reply::Value(json!({ "draft": draft })),
            other => Reply::Throw(format!("unexpected agent {other}")),
        });
        let out_rel = "out dir/it's-the-doc.md";
        let r = run_driver(
            lib,
            ENGINE,
            engine_args(json!({ "roadmap": "rm-done", "roadmapMeta": meta, "out": out_rel })),
            &agent,
        )?
        .map_err(Failure::Js)?;
        check_eq!(r["aborted"], json!(false), "a done roadmap is drafted: {r}");
        check_eq!(r["path"], json!(out_rel), "to the requested path");
        check_eq!(
            r["draft"],
            json!(draft),
            "the synthesized draft is returned"
        );

        let calls = agent.calls();
        let synth: Vec<_> = calls
            .iter()
            .filter(|c| c.label == "synthesize:draft")
            .collect();
        let gathers: Vec<_> = calls
            .iter()
            .filter(|c| c.label.starts_with("gather:"))
            .collect();
        check_eq!(synth.len(), 1, "one synthesis");
        check_eq!(gathers.len(), 2, "one gatherer per phase");
        check!(
            gathers.iter().all(|g| g.seq < synth[0].seq),
            "every gather precedes synthesis"
        );
        for g in &gathers {
            let stem = g.label.trim_start_matches("gather:");
            check!(
                g.prompt.contains(rdm_bin()) && g.prompt.contains(stem),
                "{}: the gatherer is told how to read its own phase",
                g.label
            );
        }
        let s = &synth[0].prompt;
        check!(
            s.contains(shipped),
            "the gathered account reaches the synthesizer"
        );
        let mut m = Module::open(lib, LIB)?;
        let cfg = m.call(
            "parseDocumentArgs",
            vec![engine_args(json!({ "roadmap": "rm-done" }))],
        )?;
        let show = m.call(
            "documentPhaseCommand",
            vec![json!("rm-done"), json!("phase-2-two"), cfg],
        )?;
        let show = show.as_str().unwrap_or_default();
        check!(
            s.contains(show),
            "the failed gatherer's phase still carries its read command to the synthesizer"
        );
        let phase = fx.repo.run_ok(&Run::bare(), show)?;
        check!(
            phase.contains("\"phase-2-two\""),
            "and that command reads the phase: {phase}"
        );

        // The orchestrator runs the returned write script.
        let work = fx.repo.tmpdir("work")?;
        fx.repo.run_ok(
            &Run::bare().cwd(&work),
            r["writeScript"].as_str().unwrap_or_default(),
        )?;
        let written =
            std::fs::read_to_string(work.join(out_rel)).map_err(crate::workflow_support::infra)?;
        check_eq!(
            written,
            format!("{draft}\n"),
            "the executed write lands the draft verbatim"
        );
        Ok(())
    });
}

#[test]
fn generator_in_sync() {
    GEN.in_sync();
}

#[test]
fn generator_drift_detected_then_healed() {
    GEN.drift_detected_then_healed();
}
