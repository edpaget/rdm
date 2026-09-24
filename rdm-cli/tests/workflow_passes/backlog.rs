//! The propose-only backlog-grooming pass.
//!
//! Imports the canonical `.claude/workflows/lib/backlog.mjs` for the helpers
//! and `buildBacklogPipeline`, and loads the real
//! `.claude/workflows/rdm-wf-backlog.js` driver for the engine cases. The
//! report command the pass hands back is executed against the real binary;
//! the engine case proves the plan repo is byte-identical afterwards. Ported
//! from `scripts/verify-workflow-backlog.sh` and the backlog half of
//! `scripts/lib/workflow-env-args.test.mjs`.

use std::cell::RefCell;
use std::rc::Rc;

use rdm_devtools::workflow::{Host, JsError, is_fn};
use serde_json::{Value, json};

use crate::generators::Generator;
use crate::plan_fixture::{PlanRepo, Run, rdm_bin};
use crate::workflow_support::{Agent, Failure, Module, Reply, load, run_driver, run_real, split};

const LIB: &str = ".claude/workflows/lib/backlog.mjs";
const ENGINE: &str = ".claude/workflows/rdm-wf-backlog.js";
const PROJECT: &str = "backlog-wf-proj";

const GEN: Generator = Generator {
    script: "scripts/gen-workflow-backlog.sh",
    lib: LIB,
    engine: ENGINE,
    template: "rdm-core/src/templates/workflows/rdm-wf-backlog.js",
    plugin: "plugins/rdm/workflows/rdm-wf-backlog.js",
    begin: ">>> backlog-groom:begin",
};

const TITLES: [&str; 4] = [
    "Stale tasks",
    "Duplicate clusters",
    "Tag clusters",
    "Archivable roadmaps",
];

fn populated() -> Value {
    json!({
        "stale_tasks": [{ "slug": "a", "title": "A", "status": "open" }],
        "duplicate_clusters": [{ "members": [{ "slug": "b", "title": "B" }] }],
        "tag_clusters": [{ "tag": "t", "tasks": [{ "slug": "c", "title": "C" }] }],
        "archivable_roadmaps": [{ "roadmap": "r", "title": "R", "phase_count": 2 }],
    })
}

fn empty_report() -> Value {
    json!({ "stale_tasks": [], "duplicate_clusters": [], "tag_clusters": [], "archivable_roadmaps": [] })
}

fn defaults() -> Value {
    json!({ "rdmBin": "rdm", "project": null, "olderThan": null, "tag": null })
}

fn rejects(r: &Result<Value, JsError>, needle: &str) -> bool {
    r.as_ref().is_err_and(|e| e.message.contains(needle))
}

fn keys(v: &Value) -> Vec<Value> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|c| c["key"].clone())
        .collect()
}

/// A seeded plan repo carrying all four signal categories.
fn seeded(default_project: Option<&str>) -> Result<PlanRepo, Failure> {
    let repo = match default_project {
        Some(p) => PlanRepo::init(p)?,
        None => PlanRepo::with_decoy(&[PROJECT])?,
    };
    let task = |slug: &'static str, title: &'static str, tags: &'static str| {
        vec![
            "task",
            "create",
            slug,
            "--title",
            title,
            "--tags",
            tags,
            "--no-edit",
            "--project",
            PROJECT,
        ]
    };
    repo.seed(&[
        &task("dup-a", "Fix login bug on mobile", "bug"),
        &task("dup-b", "Fix login bug on mobile devices", "mobile"),
        &task("tag-a", "Refactor the settings loader", "cluster-tag"),
        &task("tag-b", "Document the export pipeline", "cluster-tag"),
        &[
            "task",
            "create",
            "stale-one",
            "--title",
            "Stale One",
            "--body",
            "Retire me.",
            "--no-edit",
            "--project",
            PROJECT,
        ],
        &[
            "roadmap",
            "create",
            "terminal-rm",
            "--title",
            "Terminal Roadmap",
            "--body",
            "A finished roadmap.",
            "--no-edit",
            "--project",
            PROJECT,
        ],
        &[
            "phase",
            "create",
            "only",
            "--title",
            "Only",
            "--number",
            "1",
            "--body",
            "the only phase",
            "--no-edit",
            "--roadmap",
            "terminal-rm",
            "--project",
            PROJECT,
        ],
        &[
            "phase",
            "update",
            "phase-1-only",
            "--status",
            "done",
            "--no-edit",
            "--roadmap",
            "terminal-rm",
            "--project",
            PROJECT,
        ],
    ])?;
    Ok(repo)
}

/// `rdm backlog report --format json` run directly, with `flags`.
fn direct_report(repo: &PlanRepo, flags: &[&str]) -> Result<Value, Failure> {
    let mut args = vec!["backlog", "report", "--format", "json"];
    args.extend_from_slice(flags);
    repo.json(&args)
}

fn run_json(repo: &PlanRepo, run: &Run, cmd: &str) -> Result<Value, Failure> {
    let out = repo.run_ok(run, cmd)?;
    serde_json::from_str(&out).map_err(|e| Failure::Check(format!("{cmd}: non-JSON: {e}: {out}")))
}

// --- Pure helpers ------------------------------------------------------------------

#[test]
fn parse_backlog_args() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        for (args, want) in [
            (json!({}), defaults()),
            (Host::undefined(), defaults()),
            (json!("not json"), defaults()),
            (json!("null"), defaults()),
        ] {
            check_eq!(
                m.call("parseBacklogArgs", vec![args.clone()])?,
                want,
                "{args}"
            );
        }
        let field = |m: &mut Module, args: Value, key: &str| -> Result<Value, Failure> {
            Ok(m.call("parseBacklogArgs", vec![args])?[key].clone())
        };
        for (args, key, want) in [
            (json!({ "olderThan": 0 }), "olderThan", json!(0)),
            (json!({ "olderThan": "0" }), "olderThan", json!(0)),
            (json!({ "olderThan": 5 }), "olderThan", json!(5)),
            (json!({ "tag": "" }), "tag", json!("")),
            (json!({ "tag": Host::undefined() }), "tag", Value::Null),
            (json!({ "tag": "bug" }), "tag", json!("bug")),
            (json!({ "project": "rdm" }), "project", json!("rdm")),
            (json!({ "project": "" }), "project", Value::Null),
            (
                json!(r#"{"project":"p","olderThan":3}"#),
                "project",
                json!("p"),
            ),
            (
                json!(r#"{"project":"p","olderThan":3}"#),
                "olderThan",
                json!(3),
            ),
        ] {
            check_eq!(field(&mut m, args.clone(), key)?, want, "{args} → {key}");
        }
        for bad in [json!(-1), json!("abc")] {
            let r = m.try_call("parseBacklogArgs", vec![json!({ "olderThan": bad })])?;
            check!(
                rejects(&r, "non-negative integer"),
                "olderThan {bad}: {r:?}"
            );
        }
        Ok(())
    });
}

#[test]
fn category_order_and_schema() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let cats = m.get("CATEGORY")?;
        check_eq!(
            keys(&cats),
            vec![
                json!("stale_tasks"),
                json!("duplicate_clusters"),
                json!("tag_clusters"),
                json!("archivable_roadmaps")
            ],
            "fixed category order"
        );
        for c in cats.as_array().into_iter().flatten() {
            check!(is_fn(&c["analyzerPrompt"]), "{} builds a prompt", c["key"]);
        }
        let schema = m.get("ANALYSIS_SCHEMA")?;
        check_eq!(schema["type"], json!("object"), "an object schema");
        check_eq!(
            schema["required"],
            json!(["proposals", "openQuestions"]),
            "its required keys"
        );
        Ok(())
    });
}

#[test]
fn parse_backlog_report() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        check_eq!(
            m.call("parseBacklogReport", vec![json!({})])?,
            empty_report(),
            "missing arrays default to empty"
        );
        check_eq!(
            m.call("parseBacklogReport", vec![populated()])?,
            populated(),
            "a populated report round-trips"
        );
        check_eq!(
            m.call("parseBacklogReport", vec![json!(populated().to_string())])?,
            populated(),
            "a JSON string is parsed"
        );
        for bad in [Value::Null, Host::undefined(), json!(42)] {
            let r = m.try_call("parseBacklogReport", vec![bad.clone()])?;
            check!(
                rejects(&r, "returned no data"),
                "{bad} is a fetch error: {r:?}"
            );
        }
        let r = m.try_call("parseBacklogReport", vec![json!("not json")])?;
        check!(
            rejects(&r, "valid JSON"),
            "non-JSON is a fetch error: {r:?}"
        );
        Ok(())
    });
}

#[test]
fn select_categories() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let sel = |m: &mut Module, r: Value| -> Result<Vec<Value>, Failure> {
            Ok(keys(&m.call("selectCategories", vec![r])?))
        };
        check_eq!(
            sel(&mut m, json!({}))?,
            Vec::<Value>::new(),
            "empty selects nothing"
        );
        check_eq!(
            sel(&mut m, populated())?.len(),
            4,
            "populated selects all four"
        );
        let only = json!({ "stale_tasks": [], "duplicate_clusters": [], "tag_clusters": [{ "tag": "t", "tasks": [] }],
                           "archivable_roadmaps": [] });
        check_eq!(
            sel(&mut m, only)?,
            vec![json!("tag_clusters")],
            "only the populated one"
        );
        check_eq!(
            sel(&mut m, Value::Null)?,
            Vec::<Value>::new(),
            "null tolerated"
        );
        Ok(())
    });
}

#[test]
fn normalize_analysis() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        for absent in [Value::Null, Host::undefined()] {
            check_eq!(
                m.call("normalizeAnalysis", vec![absent])?,
                Value::Null,
                "absent stays null"
            );
        }
        check_eq!(
            m.call("normalizeAnalysis", vec![json!({})])?,
            json!({ "proposals": [], "openQuestions": [] }),
            "defaults"
        );
        let full =
            json!({ "proposals": [{ "command": "x", "rationale": "y" }], "openQuestions": ["q"] });
        check_eq!(
            m.call("normalizeAnalysis", vec![full.clone()])?,
            full,
            "passes through"
        );
        Ok(())
    });
}

fn subsections(summary: &str) -> Vec<String> {
    summary
        .lines()
        .filter_map(|l| l.strip_prefix("### "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn consolidate_batch() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let cats = m.get("CATEGORY")?;
        let batch = |m: &mut Module, entries: Value| -> Result<String, Failure> {
            Ok(m.call("consolidateBatch", vec![entries])?
                .as_str()
                .unwrap_or_default()
                .to_owned())
        };
        let empty = batch(&mut m, json!([]))?;
        check!(
            empty.contains("## Open questions") && empty.contains("None."),
            "the open-questions section is always present: {empty}"
        );
        check!(
            subsections(&empty).is_empty(),
            "no category subsection: {empty}"
        );

        let one = batch(
            &mut m,
            json!([{ "cat": cats[3], "result": { "proposals": [{ "command": "rdm roadmap archive terminal-rm",
                                                                "rationale": "all terminal" }], "openQuestions": [] } }]),
        )?;
        check_eq!(
            subsections(&one),
            vec![TITLES[3].to_owned()],
            "exactly the one category"
        );
        check!(
            one.contains("rdm roadmap archive terminal-rm"),
            "its proposal: {one}"
        );

        let mixed = batch(
            &mut m,
            json!([
                { "cat": cats[0], "result": { "proposals": [{ "command": "rdm task update s --status wont-fix", "rationale": "stale" }],
                                              "openQuestions": [] } },
                { "cat": cats[1], "result": { "proposals": [], "openQuestions": ["no clear survivor"] } },
                { "cat": cats[2], "result": null },
            ]),
        )?;
        check_eq!(
            subsections(&mixed),
            vec![TITLES[0].to_owned()],
            "only a category with a proposal gets a subsection"
        );
        check!(
            mixed.contains("no clear survivor"),
            "an open question is kept: {mixed}"
        );
        check!(
            mixed.contains("[Tag clusters] analysis failed for this category"),
            "a failed analyzer becomes an open question naming its category: {mixed}"
        );
        Ok(())
    });
}

// --- Executed commands and propagation ----------------------------------------------

#[test]
fn report_command_executes_with_threaded_filters() {
    run_real(|lib| {
        let repo = seeded(None)?;
        let mut m = Module::open(lib, LIB)?;
        let command = |m: &mut Module, args: Value| -> Result<String, Failure> {
            let cfg = m.call("parseBacklogArgs", vec![args])?;
            Ok(m.call("backlogReportCommand", vec![cfg])?
                .as_str()
                .unwrap_or_default()
                .to_owned())
        };
        let axes = json!({ "rdmBin": rdm_bin(), "project": PROJECT, "olderThan": 0 });
        let tagged = command(&mut m, with(&axes, json!({ "tag": "cluster-tag" })))?;
        let tagged_out = run_json(&repo, &Run::bare(), &tagged)?;
        check_eq!(
            tagged_out,
            direct_report(
                &repo,
                &[
                    "--older-than",
                    "0",
                    "--tag",
                    "cluster-tag",
                    "--project",
                    PROJECT
                ]
            )?,
            "the returned command is the tag-filtered report"
        );
        let unfiltered = direct_report(&repo, &["--older-than", "0", "--project", PROJECT])?;
        check!(
            tagged_out != unfiltered,
            "the tag filter is load-bearing in this fixture"
        );
        let empty_tag = command(&mut m, with(&axes, json!({ "tag": "" })))?;
        check_eq!(
            run_json(&repo, &Run::bare(), &empty_tag)?,
            unfiltered,
            "an empty tag filters nothing"
        );
        check!(
            unfiltered["duplicate_clusters"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
                && unfiltered["archivable_roadmaps"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty()),
            "the seeded signals surface: {unfiltered}"
        );

        // Omitted axes: a plain `rdm` on PATH and the default project.
        let home = seeded(Some(PROJECT))?;
        let plain = command(&mut m, json!({ "olderThan": 0 }))?;
        check!(
            !home.run(&Run::bare(), &plain)?.status.success(),
            "the command names a plain `rdm`"
        );
        check_eq!(
            run_json(
                &home,
                &Run::bare().path_prepend(&home.rdm_on_path()?),
                &plain
            )?,
            direct_report(&home, &["--older-than", "0"])?,
            "and reads the default project"
        );
        Ok(())
    });
}

fn with(base: &Value, extra: Value) -> Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        out.insert(k, v);
    }
    Value::Object(out)
}

#[test]
fn analyzer_prompts_carry_injected_axes_and_items() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let bin = "/opt/sentinel/bin/rdm-7c1e";
        let cfg = m.call(
            "parseBacklogArgs",
            vec![json!({ "rdmBin": bin, "project": "demo" })],
        )?;
        let report = json!({
            "stale_tasks": [{ "slug": "sentinel-stale-slug" }],
            "duplicate_clusters": [{ "members": [{ "slug": "sentinel-dup-slug" }] }],
            "tag_clusters": [{ "tag": "sentinel-tag", "tasks": [{ "slug": "sentinel-cluster-slug" }] }],
            "archivable_roadmaps": [{ "roadmap": "sentinel-roadmap-slug" }],
        });
        let cats = m.get("CATEGORY")?;
        for c in cats.as_array().cloned().unwrap_or_default() {
            let field = c["arrayField"].as_str().unwrap_or_default();
            let items = report[field].clone();
            let prompt = split(
                m.host
                    .call(&c["analyzerPrompt"], vec![items.clone(), cfg.clone()]),
            )?
            .map_err(Failure::Js)?;
            let prompt = prompt.as_str().unwrap_or_default();
            check!(prompt.contains(bin), "{field}: the injected binary");
            check!(
                prompt.contains(" --project demo"),
                "{field}: the injected project"
            );
            let slug = items.to_string();
            for s in [
                "sentinel-stale-slug",
                "sentinel-dup-slug",
                "sentinel-cluster-slug",
                "sentinel-roadmap-slug",
            ] {
                if slug.contains(s) {
                    check!(prompt.contains(s), "{field}: its report item {s}");
                }
            }
        }
        Ok(())
    });
}

#[test]
fn invalid_axes_refused_at_parse() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        for (args, needle) in [
            (json!({ "rdmBin": 42 }), "rdmBin must be a string path"),
            (json!({ "rdmBin": {} }), "rdmBin must be a string path"),
            (json!({ "project": "a b" }), "plain project name"),
            (json!({ "project": "a;rm -rf /" }), "plain project name"),
        ] {
            let r = m.try_call("parseBacklogArgs", vec![args.clone()])?;
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
        Ok(())
    });
}

// --- buildBacklogPipeline, driven ---------------------------------------------------

/// Builds the pipeline over `agent` and a Rust `fetchReport`, runs it with
/// `args`, and returns the result (or throw) plus every cfg `fetchReport`
/// received.
fn run_pipeline(
    m: &mut Module,
    agent: &Agent,
    mut fetch: impl FnMut() -> Result<Value, String> + 'static,
    args: Value,
) -> Result<(Result<Value, JsError>, Vec<Value>), Failure> {
    let mut deps = agent.install(&mut m.host);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&seen);
    deps["fetchReport"] = m.host.register_fn(move |a| {
        log.borrow_mut()
            .push(a.first().cloned().unwrap_or(Value::Null));
        fetch()
    });
    let run = m.call("buildBacklogPipeline", vec![deps])?;
    let result = m.try_invoke(&run, vec![args])?;
    let cfgs = seen.borrow().clone();
    Ok((result, cfgs))
}

fn proposing() -> Agent {
    Agent::scripted(|call| {
        Reply::Value(json!({
            "proposals": [{ "command": format!("rdm {}", call.label), "rationale": format!("r-{}", call.label) }],
            "openQuestions": [format!("q-{}", call.label)],
        }))
    })
}

#[test]
fn empty_report_short_circuits() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let agent = proposing();
        let (r, _) = run_pipeline(&mut m, &agent, || Ok(empty_report()), json!({}))?;
        check_eq!(
            r.map_err(Failure::Js)?,
            json!({ "groomed": false, "summary": "Nothing to groom — the backlog report returned no signals" }),
            "an empty report short-circuits"
        );
        check!(agent.calls().is_empty(), "no analyzer runs");
        Ok(())
    });
}

#[test]
fn full_report_one_analyzer_per_category() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let schema = m.get("ANALYSIS_SCHEMA")?;
        let agent = proposing();
        let (r, cfgs) = run_pipeline(&mut m, &agent, || Ok(populated()), json!({}))?;
        let r = r.map_err(Failure::Js)?;
        check_eq!(r["groomed"], json!(true), "groomed");
        check_eq!(cfgs.len(), 1, "the report is fetched once");
        check_eq!(cfgs[0]["project"], Value::Null, "with the parsed cfg");
        let mut labels = agent.labels();
        labels.sort();
        check_eq!(
            labels,
            vec![
                "analyze:archivable_roadmaps",
                "analyze:duplicate_clusters",
                "analyze:stale_tasks",
                "analyze:tag_clusters"
            ],
            "one uniquely-labelled analyzer per category"
        );
        for c in agent.calls() {
            check_eq!(
                c.opts["schema"],
                schema,
                "{} is forced to ANALYSIS_SCHEMA",
                c.label
            );
        }
        let summary = r["summary"].as_str().unwrap_or_default();
        check_eq!(
            subsections(summary),
            TITLES.map(str::to_owned).to_vec(),
            "a subsection per category, in order"
        );
        for c in agent.calls() {
            check!(
                summary.contains(&format!("rdm {}", c.label))
                    && summary.contains(&format!("q-{}", c.label)),
                "{}'s proposal and open question are consolidated",
                c.label
            );
        }
        Ok(())
    });
}

#[test]
fn fetch_error_propagates() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let agent = proposing();
        let (r, _) = run_pipeline(
            &mut m,
            &agent,
            || Err("rdm backlog report exited non-zero".to_owned()),
            json!({}),
        )?;
        check!(
            rejects(&r, "exited non-zero"),
            "a fetch failure propagates: {r:?}"
        );
        check!(agent.calls().is_empty(), "no analyzer runs");
        Ok(())
    });
}

#[test]
fn crashed_analyzer_degrades() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let agent = Agent::scripted(|call| {
            if call.label == "analyze:stale_tasks" {
                Reply::Throw("analyzer crashed".into())
            } else {
                Reply::Value(
                    json!({ "proposals": [{ "command": format!("rdm {}", call.label), "rationale": "ok" }],
                                     "openQuestions": [] }),
                )
            }
        });
        let (r, _) = run_pipeline(&mut m, &agent, || Ok(populated()), json!({}))?;
        let r = r.map_err(Failure::Js)?;
        check_eq!(
            r["groomed"],
            json!(true),
            "one crash does not abort the run"
        );
        let summary = r["summary"].as_str().unwrap_or_default();
        check_eq!(
            subsections(summary),
            TITLES[1..]
                .iter()
                .map(|s| (*s).to_owned())
                .collect::<Vec<_>>(),
            "the others keep their subsections; the crashed one gets none"
        );
        check!(
            summary.contains("[Stale tasks] analysis failed for this category"),
            "the crashed category becomes an open question: {summary}"
        );
        Ok(())
    });
}

#[test]
fn output_deterministic() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let (a, _) = run_pipeline(&mut m, &proposing(), || Ok(populated()), json!({}))?;
        let (b, _) = run_pipeline(&mut m, &proposing(), || Ok(populated()), json!({}))?;
        check_eq!(
            a.map_err(Failure::Js)?["summary"],
            b.map_err(Failure::Js)?["summary"],
            "identical input, identical output"
        );
        Ok(())
    });
}

#[test]
fn missing_fetch_report_throws() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let deps = proposing().install(&mut m.host);
        let r = m.try_call("buildBacklogPipeline", vec![deps])?;
        check!(rejects(&r, "deps.fetchReport is required"), "{r:?}");
        Ok(())
    });
}

// --- The engine --------------------------------------------------------------------

#[test]
fn engine_zero_mutation_over_real_report() {
    run_real(|lib| {
        let repo = seeded(None)?;
        let before = repo.snapshot()?;
        let args = json!({ "rdmBin": rdm_bin(), "project": PROJECT, "olderThan": 0 });

        // Without a report the engine reads nothing and names the command.
        let first = Agent::scripted(|_| Reply::Throw("no agent may run".into()));
        let r = run_driver(lib, ENGINE, args.clone(), &first)?.map_err(Failure::Js)?;
        check_eq!(r["fetchError"], json!(true), "no report, no run: {r}");
        check!(first.calls().is_empty(), "no agent without a report");
        let report = run_json(
            &repo,
            &Run::bare(),
            r["reportCommand"].as_str().unwrap_or_default(),
        )?;

        // The report it named, fed back, drives every category.
        let agent = Agent::scripted(|call| {
            Reply::Value(
                json!({ "proposals": [{ "command": format!("rdm {} <slug>", call.label),
                                                 "rationale": "deterministic fixture proposal" }], "openQuestions": [] }),
            )
        });
        let r = run_driver(
            lib,
            ENGINE,
            with(&args, json!({ "report": report })),
            &agent,
        )?
        .map_err(Failure::Js)?;
        check_eq!(
            r["groomed"],
            json!(true),
            "the seeded report is groomed: {r}"
        );
        check_eq!(
            subsections(r["summary"].as_str().unwrap_or_default()),
            TITLES.map(str::to_owned).to_vec(),
            "all four subsections"
        );
        check_eq!(agent.calls().len(), 4, "one analyzer per category");
        check!(
            agent
                .calls()
                .iter()
                .all(|c| c.opts["phase"] == json!("Analyze")),
            "every analyzer runs in the declared phase"
        );
        check_eq!(
            repo.snapshot()?,
            before,
            "HEAD, status and every file are unchanged"
        );
        Ok(())
    });
}

#[test]
fn engine_duplicate_meta_fails_to_load() {
    run_real(|lib| {
        let script = lib.read(ENGINE)?;
        let mut host = Host::start_default()?;
        load(host.load_driver(&script))?;
        match host.load_driver(&format!("{script}\nlet meta = null\n")) {
            Err(rdm_devtools::workflow::WorkflowError::Js(e)) => {
                check_eq!(
                    e.name,
                    "SyntaxError",
                    "a duplicate top-level meta fails to compile"
                );
                Ok(())
            }
            other => Err(Failure::Check(format!(
                "a duplicate top-level meta must fail to load: {other:?}"
            ))),
        }
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
