//! The refuter-agreement instrument (`rdm-measure refuter-agreement` and
//! `rdm-measure mine-refuter-corpus`) against the checked-in
//! `tests/fixtures/refuter-agreement` corpus, sidecars and trial files, plus
//! the goldens captured from the JavaScript tools before they were deleted.
//!
//! Workflow-dependent (need Node; they reach the canonical `refutePrompt` in
//! `.claude/workflows/lib/review.mjs` only through the phase-2 binding via
//! `NodeReviewRules`): `corpus_prompts_regenerate_as_recorded`,
//! `mined_prompts_exceed_sidecar_preview_length`,
//! `refute_prompt_edit_is_reported_as_drift`,
//! `dry_run_matches_golden_and_dispatches_nothing`, `fake_claude_drives_full_path`
//! and `concurrency_does_not_change_output_order`. Node resolves from
//! `RDM_TEST_NODE` or `PATH`; a missing runtime fails them.
//!
//! Every other test needs no JavaScript runtime (several run the binary with an
//! empty `PATH` to prove it). No test dispatches a real agent: `claude` is
//! always the `rdm-devtools-fixture` fake, reached through `--claude-bin`.

mod measure_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use measure_support::{
    edited_doc, fails, fixture, golden, json, json_without_instrument, normalize, ok, repo_root,
    run, run_with, stderr,
};
use rdm_devtools::measure::jsjson::JsValue;
use rdm_devtools::measure::refuter_agreement::audit::find_blended_accuracy_keys;
use rdm_devtools::measure::refuter_agreement::corpus::{
    self, CorpusItem, DIVERGENCE_CLASS, HISTORICAL_ONLY_SEVERITIES, MIN_AUTHORITATIVE_SHARE,
    MIN_CORPUS_SIZE, MIN_DIVERGENCE_CLASS_SHARE, MIN_MINED_SHARE, authoritative_evidence,
    load_corpus, summarize_corpus,
};
use rdm_devtools::measure::refuter_agreement::dispatch::Dispatcher;
use rdm_devtools::measure::refuter_agreement::prompt::{
    build_batch_prompt, check_prompt_fidelity, regenerate_prompt,
};
use rdm_devtools::measure::refuter_agreement::report::{format_json, format_text};
use rdm_devtools::measure::refuter_agreement::score::{
    ScoreOptions, bucket_key_for, score_anchoring, score_trials,
};
use rdm_devtools::measure::refuter_agreement::trials::{
    MIN_BATCH_GROUP_SIZE, MIN_QUALIFYING_BATCH_GROUPS, MIN_QUALIFYING_BATCH_ITEMS,
    batch_group_key_for, build_batch_trials, expand_batch_results, format_batch_power,
    group_corpus_for_batching, unit_ident_of,
};
use rdm_devtools::measure::review_rules::NodeReviewRules;
use rdm_devtools::workflow::MutantTree;
use serde_json::{Value, json as j};

fn dir() -> PathBuf {
    fixture("refuter-agreement")
}

fn expected(name: &str) -> PathBuf {
    dir().join("expected").join(name)
}

fn sidecars() -> PathBuf {
    dir().join("mine-sidecars")
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

fn corpus_items() -> Vec<CorpusItem> {
    let (items, errors) = load_corpus(&golden(&dir().join("corpus.jsonl")));
    assert_eq!(errors, Vec::<String>::new());
    items
}

fn trials_file(name: &str) -> (Vec<JsValue>, Option<String>) {
    let v = JsValue::parse(&golden(&dir().join(name))).expect("trials JSON");
    let trials = v
        .get("trials")
        .and_then(JsValue::as_array)
        .cloned()
        .unwrap_or_default();
    (
        trials,
        v.get("baselineTier")
            .and_then(JsValue::as_str)
            .map(str::to_owned),
    )
}

fn no_js() -> Vec<(&'static str, String)> {
    let home = std::env::temp_dir();
    vec![
        ("PATH", String::new()),
        ("RDM_TEST_NODE", String::new()),
        ("HOME", home.display().to_string()),
    ]
}

fn run_no_js(args: &[&str]) -> std::process::Output {
    let env = no_js();
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    run_with(args, &pairs)
}

fn rules() -> NodeReviewRules {
    NodeReviewRules::start_default().unwrap_or_else(|e| panic!("{e}"))
}

/// A fake `claude` (the fixture binary under that name) with its scenario
/// beside it; returns (bin path, record path, temp dir guard).
fn fake_claude(scenario: Value) -> (PathBuf, PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin = tmp.path().join("claude");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_rdm-devtools-fixture"), &bin).expect("symlink");
    let record = tmp.path().join("calls.jsonl");
    let mut scenario = scenario;
    scenario["record"] = Value::String(s(&record).to_owned());
    std::fs::write(
        tmp.path().join("claude.scenario.json"),
        scenario.to_string(),
    )
    .expect("scenario");
    (bin, record, tmp)
}

fn calls(record: &Path) -> Vec<Value> {
    std::fs::read_to_string(record)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).expect("record line"))
        .collect()
}

fn claude_body(refuted: &str, tool_calls: usize, session: Option<&str>) -> String {
    let mut content: Vec<Value> = (1..tool_calls)
        .map(|_| j!({"type": "tool_use", "name": "Read", "input": {}}))
        .collect();
    if tool_calls > 0 {
        content.push(j!({"type": "tool_use", "name": "StructuredOutput",
            "input": {"refuted": serde_json::from_str::<Value>(refuted).expect("refuted"), "confidence": 90, "rationale": "fake"}}));
    }
    let mut body = j!({
        "usage": {"output_tokens": 10, "input_tokens": 20, "cache_creation_input_tokens": 5, "cache_read_input_tokens": 100},
        "messages": [{"message": {"content": content}}]
    });
    if tool_calls == 0 {
        body["result"] = Value::String(format!("{{\"refuted\": {refuted}, \"confidence\": 70}}"));
    }
    if let Some(id) = session {
        body["session_id"] = Value::String(id.to_owned());
    }
    body.to_string()
}

// --- corpus ------------------------------------------------------------------

#[test]
fn corpus_loads_with_floors_enums_and_adjudication_commits() {
    let items = corpus_items();
    assert!(items.len() >= MIN_CORPUS_SIZE);
    let mut ids: Vec<&str> = items.iter().map(CorpusItem::id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), items.len(), "duplicate corpus ids");
    let summary = summarize_corpus(&items);
    assert!(summary.divergence_class_share >= MIN_DIVERGENCE_CLASS_SHARE * 100.0);
    assert!(summary.mined_share >= MIN_MINED_SHARE * 100.0);
    assert!(summary.authoritative_share >= MIN_AUTHORITATIVE_SHARE * 100.0);
    for i in &items {
        assert!(
            corpus::GROUND_TRUTH_CLASSES.contains(&i.class()),
            "{}",
            i.id()
        );
        assert!(corpus::AUTHORITIES.contains(&i.authority()), "{}", i.id());
        assert!(
            corpus::PROVENANCE_KINDS.contains(&i.provenance_kind()),
            "{}",
            i.id()
        );
        let evidence = i
            .at(&["groundTruth", "evidence"])
            .and_then(JsValue::as_str)
            .unwrap_or("");
        if i.authority() == "authoritative" {
            assert!(
                authoritative_evidence(evidence),
                "{}: uncited authoritative evidence",
                i.id()
            );
        }
        let commit = i
            .at(&["groundTruth", "adjudicatedAgainstCommit"])
            .and_then(JsValue::as_str)
            .unwrap_or("");
        assert!(
            (7..=40).contains(&commit.len())
                && commit
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{}",
            i.id()
        );
        // No item may embed an absolute path outside this repo.
        let blob = i.value().stringify();
        for (at, _) in blob.match_indices("/Users/") {
            let tail = &blob[at..];
            if let Some(p) = tail.find("/Projects/") {
                assert!(
                    tail[p..].starts_with("/Projects/rdm"),
                    "{}: foreign path",
                    i.id()
                );
            }
        }
    }
    for sev in ["blocking", "concern"] {
        assert!(
            summary.by_severity.get(sev).is_some_and(|n| *n > 0),
            "no {sev} items"
        );
    }
    for mode in ["code", "plan"] {
        assert!(
            summary.by_mode.get(mode).is_some_and(|n| *n > 0),
            "no {mode} items"
        );
    }
    assert!(
        items
            .iter()
            .any(|i| HISTORICAL_ONLY_SEVERITIES.contains(&i.severity()))
    );
    // Relabelling the divergence class away breaks the share floor.
    let relabelled: Vec<CorpusItem> = items
        .iter()
        .map(|i| {
            let text = i.value().stringify().replace(
                &format!("\"class\":\"{DIVERGENCE_CLASS}\""),
                "\"class\":\"style-preference\"",
            );
            CorpusItem(JsValue::parse(&text).expect("json"))
        })
        .collect();
    assert!(
        summarize_corpus(&relabelled).divergence_class_share < MIN_DIVERGENCE_CLASS_SHARE * 100.0
    );
}

#[test]
fn invalid_corpus_items_rejected_with_named_errors() {
    let first = golden(&dir().join("corpus.jsonl"))
        .lines()
        .next()
        .expect("line")
        .to_owned();
    let edit = |f: &dyn Fn(&mut Value)| -> Vec<String> {
        let mut v: Value = serde_json::from_str(&first).expect("json");
        f(&mut v);
        load_corpus(&v.to_string()).1
    };
    let has = |errs: &[String], needle: &str| errs.iter().any(|e| e.contains(needle));
    let e = edit(&|v| v["groundTruth"]["authority"] = j!("probably"));
    assert!(
        has(
            &e,
            "groundTruth.authority must be one of authoritative|judgement-call, got \"probably\""
        ),
        "{e:?}"
    );
    let e = edit(&|v| v["typo"] = j!(1));
    assert!(has(&e, "unknown top-level key \"typo\""), "{e:?}");
    let e = edit(&|v| {
        v.as_object_mut().expect("obj").remove("promptDrift");
    });
    assert!(has(&e, "missing required key \"promptDrift\""), "{e:?}");
    let e = edit(&|v| v["groundTruth"]["class"] = j!("real-defect"));
    assert!(has(&e, "contradicts class \"real-defect\""), "{e:?}");
    let e = edit(&|v| {
        v["groundTruth"]["authority"] = j!("authoritative");
        v["groundTruth"]["evidence"] = j!("it is obviously not a bug");
    });
    assert!(has(&e, "must cite a concrete artifact"), "{e:?}");
    let e = edit(&|v| v["groundTruth"]["adjudicatedAgainstCommit"] = j!("HEAD"));
    assert!(
        has(
            &e,
            "adjudicatedAgainstCommit must be a 7-40 char hex commit sha"
        ),
        "{e:?}"
    );
    let e = edit(&|v| v["schemaVersion"] = j!(2));
    assert!(has(&e, "schemaVersion must be 1, got 2"), "{e:?}");
    let e = edit(&|v| v["groundTruth"] = Value::Null);
    assert!(has(&e, "groundTruth must be an object"), "{e:?}");
    let e = edit(&|v| {
        v["finding"]
            .as_object_mut()
            .expect("obj")
            .remove("what_fails");
    });
    assert!(has(&e, "finding.what_fails is required"), "{e:?}");
    let (_, dup) = load_corpus(&format!("{first}\n\n{first}\n"));
    assert!(has(&dup, "line 3: duplicate id"), "{dup:?}");
    let (_, bad) = load_corpus("{not json\n");
    assert!(has(&bad, "line 1: not parseable JSON"), "{bad:?}");
    // Through the CLI: a corpus with an invalid item exits 1, naming it.
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let mut v: Value = serde_json::from_str(&first).expect("json");
    v["mode"] = j!("yolo");
    std::fs::write(tmp.path(), format!("{v}\n")).expect("write");
    let err = fails(&run_no_js(&[
        "refuter-agreement",
        "--corpus",
        s(tmp.path()),
        "--batch-power",
    ]));
    assert!(
        err.contains("has 1 validation error(s)") && err.contains("mode must be one of code|plan"),
        "{err}"
    );
}

#[test]
fn corpus_prompts_regenerate_as_recorded() {
    let items = corpus_items();
    let mut rules = rules();
    for i in &items {
        let f = check_prompt_fidelity(i, &mut rules).expect("fidelity");
        assert_eq!(
            f.drifted,
            i.prompt_drift(),
            "{}: promptDrift {} but regeneration says {}",
            i.id(),
            i.prompt_drift(),
            f.drifted
        );
    }
    // A mutated recorded sha is reported as drift, never silently accepted.
    let text = items[0]
        .value()
        .stringify()
        .replace(items[0].prompt_sha256(), &"0".repeat(64));
    let mutated = CorpusItem(JsValue::parse(&text).expect("json"));
    assert!(
        check_prompt_fidelity(&mutated, &mut rules)
            .expect("fidelity")
            .drifted
    );
    let _ = rules.shutdown();
}

#[test]
fn mined_prompts_exceed_sidecar_preview_length() {
    let mut rules = rules();
    let mut checked = 0;
    for i in corpus_items()
        .iter()
        .filter(|i| i.provenance_kind() == "mined")
    {
        let p = regenerate_prompt(i, &mut rules).expect("prompt");
        assert!(
            p.encode_utf16().count() > 401,
            "{}: a sidecar-preview-length prompt",
            i.id()
        );
        checked += 1;
    }
    assert!(checked > 0);
    let _ = rules.shutdown();
}

#[test]
fn refute_prompt_edit_is_reported_as_drift() {
    let tree = MutantTree::copy(&repo_root(), &[".claude/workflows/lib/review.mjs"]).expect("copy");
    tree.replace_once(
        ".claude/workflows/lib/review.mjs",
        "reworded-stance",
        "'Start from the stance: this is NOT a real issue unless the ' +",
        "'Start from the stance: this is NOT a genuine issue unless the ' +",
    )
    .expect("plant mutant");
    let lib = tree.root().join(".claude/workflows/lib/review.mjs");
    let items = corpus_items();
    let mut mutant =
        NodeReviewRules::start(&lib, std::time::Duration::from_secs(120)).expect("start");
    for i in &items {
        assert!(
            check_prompt_fidelity(i, &mut mutant)
                .expect("fidelity")
                .drifted,
            "{}",
            i.id()
        );
    }
    let _ = mutant.shutdown();
    let out = run(&[
        "refuter-agreement",
        "--tiers",
        "opus",
        "--dry-run",
        "--review-lib",
        s(&lib),
    ]);
    let err = stderr(&out);
    assert!(out.status.success(), "{err}");
    assert!(
        err.contains(&format!("WARNING: {} corpus item(s) no longer regenerate byte-identically through refutePrompt (promptDrift)", items.len())),
        "{err}"
    );
}

// --- batch power -----------------------------------------------------------

fn synth(id: &str, target: &str, extra: Value) -> JsValue {
    let mut v = j!({
        "id": id, "schemaVersion": 1, "mode": "code", "dim": {"key": "correctness"}, "target": target,
        "finding": {"id": "f", "concern": "correctness", "location": "x", "severity": "concern", "confidence": 90, "what_fails": "y"},
        "promptSha256": "0".repeat(64), "promptDrift": false,
        "provenance": {"kind": "mined", "runId": "wf_same", "projectSlug": "p", "sessionId": "s", "agentId": format!("a{id}"), "workflow": "w"},
        "groundTruth": {"defect": false, "class": "stale-fact", "authority": "judgement-call", "evidence": "e", "adjudicatedAgainstCommit": "abcdef1"}
    });
    if let Value::Object(more) = extra {
        v["provenance"].as_object_mut().expect("obj").extend(more);
    }
    JsValue::from_json(&v)
}

#[test]
fn batch_power_under_unit_scoped_key_matches_golden() {
    let out = ok(&run_no_js(&["refuter-agreement", "--batch-power"]));
    assert_eq!(out, golden(&expected("batch-power.txt")));
    // Two review units in one run/mode/dim never merge; one unit does.
    let two = [
        synth("u1", "roadmap-a/phase-1", j!({})),
        synth("u2", "roadmap-a/phase-2", j!({})),
    ];
    let refs: Vec<&JsValue> = two.iter().collect();
    assert_eq!(
        group_corpus_for_batching(&refs, 2)
            .expect("power")
            .group_count,
        2
    );
    let k1 = batch_group_key_for(&two[0]).expect("key");
    assert_ne!(Some(k1.clone()), batch_group_key_for(&two[1]));
    assert_eq!(k1.split('|').count(), 4);
    let same = [
        synth("s1", "roadmap-a/phase-1", j!({})),
        synth("s2", "roadmap-a/phase-1", j!({})),
    ];
    let refs: Vec<&JsValue> = same.iter().collect();
    assert_eq!(
        group_corpus_for_batching(&refs, 2)
            .expect("power")
            .group_count,
        1
    );
    // Unit identity: the first line, under the shared rule.
    let ident = |t: &str| unit_ident_of(&JsValue::from_json(&j!({"target": t})));
    assert_eq!(
        ident("phase r/p-1\n\nbody with { braces }").as_deref(),
        Some("phase r/p-1")
    );
    assert_eq!(ident("{\n \"steps\": []\n}"), None);
    assert_eq!(ident("a \"quoted\" thing"), None);
    assert_eq!(ident(&"x".repeat(201)), None);
    assert_eq!(ident(""), None);
    // The committed corpus's power matches docs/token-baseline.json.
    let items = corpus_items();
    let refs: Vec<&JsValue> = items.iter().map(CorpusItem::value).collect();
    let p = group_corpus_for_batching(&refs, MIN_BATCH_GROUP_SIZE).expect("power");
    let doc = json(&golden(&repo_root().join("docs/token-baseline.json")));
    let want = &doc["refuterBatching"]["corpusPower"];
    let got = serde_json::to_value(&p).expect("value");
    for f in [
        "totalItems",
        "constructedExcluded",
        "nonGatingExcluded",
        "unrecoverableUnitExcluded",
        "groupableItems",
        "groupCount",
        "sizeHistogram",
        "qualifyingGroups",
        "qualifyingItems",
        "meetsMinimum",
    ] {
        assert_eq!(got[f], want[f], "corpusPower.{f}");
    }
    assert_eq!(
        p.constructed_excluded
            + p.non_gating_excluded
            + p.unrecoverable_unit_excluded
            + p.groupable_items,
        p.total_items
    );
    assert!(
        p.groups
            .iter()
            .filter(|g| g.size >= p.min_group_size)
            .all(|g| g.size > 1)
    );
    assert_eq!(
        (p.min_qualifying_groups, p.min_qualifying_items),
        (MIN_QUALIFYING_BATCH_GROUPS, MIN_QUALIFYING_BATCH_ITEMS)
    );
    assert!(
        format_batch_power(&p)
            .lines()
            .any(|l| l == "POWER: INSUFFICIENT")
    );
    // Round splitting on agentIndex.
    let rounds = [
        synth("r1", "roadmap-a/phase-1", j!({"agentIndex": 3})),
        synth("r2", "roadmap-a/phase-1", j!({"agentIndex": 4})),
        synth("r3", "roadmap-a/phase-1", j!({"agentIndex": 90})),
    ];
    let refs: Vec<&JsValue> = rounds.iter().collect();
    let r = group_corpus_for_batching(&refs, 2).expect("power");
    assert_eq!((r.group_count, r.round_splits), (2, 1));
}

#[test]
fn batch_power_dispatches_nothing() {
    let (bin, record, _tmp) = fake_claude(j!({"default": {"stdout": "{}", "exit": 0}}));
    let out = ok(&run_no_js(&[
        "refuter-agreement",
        "--batch-power",
        "--tiers",
        "opus",
        "--claude-bin",
        s(&bin),
    ]));
    assert!(out.lines().any(|l| l == "POWER: INSUFFICIENT"));
    assert!(!record.exists(), "--batch-power reached the dispatcher");
}

#[test]
fn underpowered_batched_arm_throws_or_forces_no_measurement() {
    let singletons: Vec<JsValue> = (0..20)
        .map(|i| {
            let mut v = synth(&format!("s{i}"), &format!("roadmap/phase-{i}"), j!({})).to_json();
            v["provenance"]["runId"] = j!(format!("wf_s{i}"));
            JsValue::from_json(&v)
        })
        .collect();
    let refs: Vec<&JsValue> = singletons.iter().collect();
    let power = group_corpus_for_batching(&refs, MIN_BATCH_GROUP_SIZE).expect("power");
    assert_eq!((power.qualifying_groups, power.meets_minimum), (0, false));
    let tiers = vec!["opus".to_owned()];
    let err = build_batch_trials(&power, &tiers, 2, false).expect_err("must refuse");
    assert!(err.contains("UNDERPOWERED"), "{err}");
    let plan = build_batch_trials(&power, &tiers, 2, true).expect("allowed");
    assert!(plan.no_measurement && plan.underpowered);
    let items: Vec<CorpusItem> = singletons.into_iter().map(CorpusItem).collect();
    let report = score_trials(
        &items,
        &[],
        ScoreOptions {
            baseline_tier: Some("opus".to_owned()),
            no_measurement: plan.no_measurement,
            decision: Some("ship-batched".to_owned()),
            ..ScoreOptions::default()
        },
    );
    let text = format_text(&report);
    assert!(
        text.lines()
            .any(|l| l == "NO MEASUREMENT — batched arm was underpowered")
    );
    assert!(
        !text.lines().any(|l| l.starts_with("DECISION:")),
        "no decision under NO MEASUREMENT"
    );
}

// --- miner -------------------------------------------------------------------

fn mine(extra: &[&str]) -> (String, String) {
    let root = sidecars();
    let mut args = vec!["mine-refuter-corpus", "--root", s(&root)];
    args.extend_from_slice(extra);
    let out = run_no_js(&args);
    (normalize(&ok(&out), &root), stderr(&out))
}

fn mine_json(extra: &[&str]) -> Value {
    let mut args = extra.to_vec();
    args.extend_from_slice(&["--format", "json"]);
    json(&mine(&args).0)
}

#[test]
fn miner_matches_goldens() {
    let (jsonl, err) = mine(&[]);
    assert_eq!(
        jsonl,
        golden(&expected("mine.jsonl")),
        "byte-identical JSONL, key order included"
    );
    assert_eq!(err, golden(&expected("mine.stderr.txt")));
    let (j, _) = mine(&["--format", "json"]);
    assert_eq!(json(&j)["instrument"], "rdm-measure mine-refuter-corpus");
    assert_eq!(
        json_without_instrument(&j),
        json_without_instrument(&golden(&expected("mine.json")))
    );
    assert_eq!(
        mine(&["--severity", "concern"]).0,
        golden(&expected("mine-severity-concern.jsonl"))
    );
    assert_eq!(
        mine(&["--min-group-size", "3"]).0,
        golden(&expected("mine-min-group-3.jsonl"))
    );
    assert_eq!(
        json_without_instrument(&mine(&["--min-group-size", "3", "--format", "json"]).0),
        json_without_instrument(&golden(&expected("mine-min-group-3.json")))
    );
    let exclude = dir().join("exclude-one.jsonl");
    assert_eq!(
        mine(&["--exclude-corpus", s(&exclude)]).0,
        golden(&expected("mine-exclude-corpus.jsonl"))
    );
    assert_eq!(
        json_without_instrument(&mine(&["--exclude-corpus", s(&exclude), "--format", "json"]).0),
        json_without_instrument(&golden(&expected("mine-exclude-corpus.json")))
    );
    // The miner really reads the transcript: editing a fixture finding moves
    // the mined record.
    let tmp = tempfile::tempdir().expect("tmp");
    copy_tree(&sidecars(), tmp.path());
    let t = tmp.path().join(
        "-Users-edward-Projects-rdm/sess-mine/subagents/workflows/wf_mine001/agent-refute-ok.jsonl",
    );
    let text = std::fs::read_to_string(&t).expect("read");
    let mutated = text.replace(
        "\\\"severity\\\": \\\"blocking\\\"",
        "\\\"severity\\\": \\\"concern\\\"",
    );
    assert_ne!(text, mutated, "the fixture edit applied");
    std::fs::write(&t, mutated).expect("write");
    let out = json(&ok(&run_no_js(&[
        "mine-refuter-corpus",
        "--root",
        s(tmp.path()),
        "--format",
        "json",
    ])));
    let item = out["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|i| i["id"] == "mined-wf_mine001-refute-ok")
        .expect("item");
    assert_eq!(item["finding"]["severity"], "concern");
}

#[test]
fn miner_six_skip_reasons_and_accounting_identity() {
    let m = mine_json(&[]);
    assert_eq!(m["recovered"], 3);
    for reason in [
        "unparseable-finding",
        "no-verdict",
        "no-transcript",
        "no-prompt",
        "unrecoverable-mode",
        "unrecoverable-dim",
    ] {
        assert_eq!(m["skips"][reason], 1, "{reason}");
    }
    let skipped: u64 = m["skips"]
        .as_object()
        .expect("skips")
        .values()
        .filter_map(Value::as_u64)
        .sum();
    assert_eq!(
        m["recovered"].as_u64().unwrap_or(0) + skipped,
        m["refuterRecordCount"].as_u64().unwrap_or(0)
    );
    let by_id = |id: &str| {
        m["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|i| i["id"] == id)
            .cloned()
            .expect(id)
    };
    let ok_item = by_id("mined-wf_mine001-refute-ok");
    assert_eq!(
        (ok_item["mode"].as_str(), ok_item["dim"]["key"].as_str()),
        (Some("code"), Some("correctness"))
    );
    assert_eq!(
        ok_item["groundTruth"],
        Value::Null,
        "the miner never assigns ground truth"
    );
    assert_eq!(ok_item["provenance"]["historicalVerdict"]["refuted"], true);
    assert!(ok_item["promptLength"].as_u64().unwrap_or(0) > 401);
    let plan = by_id("mined-wf_mine001-refute-plan");
    assert_eq!(
        plan["finding"]["id"], "f-2",
        "not mis-extracted from the braced target"
    );
    let target = plan["target"].as_str().unwrap_or("");
    assert!(
        target.contains('\n') && target.contains("steps_per_ac"),
        "the multi-line target is whole"
    );
    for i in m["items"].as_array().expect("items") {
        assert!(
            i["provenance"]["agentIndex"].is_u64(),
            "agentIndex recorded"
        );
        assert_eq!(i["promptSha256"].as_str().map(str::len), Some(64));
    }
}

#[test]
fn miner_slug_filter_excludes_foreign_project() {
    let m = mine_json(&[]);
    assert!(m["items"].as_array().expect("items").iter().all(|i| {
        i["provenance"]["projectSlug"]
            .as_str()
            .is_some_and(|p| p.starts_with("-Users-edward-Projects-rdm"))
    }));
    let open = mine_json(&["--project-slug", "-Users-edward-Projects-"]);
    let foreign = open["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|i| {
            i["provenance"]["projectSlug"]
                .as_str()
                .is_some_and(|p| p.contains("bowling"))
        })
        .count();
    assert_eq!(
        foreign, 1,
        "widening --project-slug surfaces the out-of-scope item"
    );
}

#[test]
fn miner_severity_until_limit_out_flags() {
    let sev = mine_json(&["--severity", "blocking"]);
    assert_eq!(sev["recovered"], 1);
    assert_eq!(sev["skips"]["severity-filtered"], 2);
    let both = mine_json(&["--severity", "blocking,concern"]);
    assert_eq!(both["recovered"], 3);
    assert!(both["skips"].get("severity-filtered").is_none());
    let before = mine_json(&["--until", "2026-07-24T00:00:00Z"]);
    assert_eq!(
        (
            before["recovered"].as_u64(),
            before["refuterRecordCount"].as_u64()
        ),
        (Some(0), Some(0))
    );
    let after = mine_json(&["--until", "2026-07-26T00:00:00Z"]);
    assert_eq!(after["recovered"], 3);
    assert_eq!(after["until"], "2026-07-26T00:00:00Z");
    let limited = mine_json(&["--limit", "1"]);
    assert_eq!(limited["items"].as_array().map(Vec::len), Some(1));
    let tmp = tempfile::tempdir().expect("tmp");
    let out_path = tmp.path().join("mined.jsonl");
    let (stdout, _) = mine(&["--out", s(&out_path)]);
    assert!(stdout.is_empty(), "--out leaves stdout empty");
    let written = std::fs::read_to_string(&out_path).expect("written");
    assert_eq!(
        normalize(&written, &sidecars()),
        golden(&expected("mine.jsonl"))
    );
}

#[test]
fn miner_min_group_size_and_exclude_corpus() {
    let g = mine_json(&["--min-group-size", "2"]);
    assert_eq!(g["recovered"], 0, "an all-singleton fixture keeps nothing");
    assert_eq!(g["skips"]["below-min-group-size"], 3);
    assert_eq!(g["batchGrouping"]["minGroupSize"], 2);
    assert_eq!(g["batchGrouping"]["qualifyingGroups"], 0);
    let skipped: u64 = g["skips"]
        .as_object()
        .expect("skips")
        .values()
        .filter_map(Value::as_u64)
        .sum();
    assert_eq!(
        skipped,
        g["refuterRecordCount"].as_u64().unwrap_or(0),
        "no silent drop"
    );
    let all = mine_json(&[]);
    let exclude = dir().join("exclude-one.jsonl");
    let ex = mine_json(&["--exclude-corpus", s(&exclude)]);
    assert_eq!(
        ex["recovered"].as_u64().unwrap_or(0),
        all["recovered"].as_u64().unwrap_or(0) - 1
    );
    assert_eq!(ex["skips"]["already-adjudicated"], 1);
    let missing = fails(&run_no_js(&[
        "mine-refuter-corpus",
        "--root",
        s(&sidecars()),
        "--exclude-corpus",
        "/nonexistent/corpus.jsonl",
    ]));
    assert!(
        missing.contains("--exclude-corpus could not read"),
        "{missing}"
    );
}

#[test]
fn miner_bad_arguments_rejected() {
    let root = sidecars();
    for (flag, args) in [
        ("--until", vec!["--root", s(&root), "--until", "not-a-date"]),
        ("--limit", vec!["--root", s(&root), "--limit", "0"]),
        ("--limit", vec!["--root", s(&root), "--limit", "abc"]),
        (
            "--min-group-size",
            vec!["--root", s(&root), "--min-group-size", "1"],
        ),
        ("--format", vec!["--root", s(&root), "--format", "yaml"]),
        ("--root", vec!["--root"]),
        ("--nonsense", vec!["--nonsense"]),
    ] {
        let mut full = vec!["mine-refuter-corpus"];
        full.extend(args);
        let err = fails(&run_no_js(&full));
        assert!(err.contains(flag), "{flag}: {err}");
        assert!(!err.contains("panicked"), "{err}");
    }
}

// --- scoring -----------------------------------------------------------------

#[test]
fn score_sample_matches_goldens() {
    for name in ["trials-sample", "trials-bounded-8x2x2"] {
        let rel = format!("tests/fixtures/refuter-agreement/{name}.json");
        let golden_name = if name == "trials-sample" {
            "score-sample"
        } else {
            "score-bounded-8x2x2"
        };
        let text = ok(&run_no_js(&["refuter-agreement", "--score-only", &rel]));
        assert_eq!(
            text,
            golden(&expected(&format!("{golden_name}.txt"))),
            "{name} text"
        );
        let js = ok(&run_no_js(&[
            "refuter-agreement",
            "--score-only",
            &rel,
            "--format",
            "json",
        ]));
        assert_eq!(
            json(&js),
            json(&golden(&expected(&format!("{golden_name}.json")))),
            "{name} json"
        );
    }
}

fn sample_report() -> rdm_devtools::measure::refuter_agreement::score::ScoreReport {
    let (trials, baseline) = trials_file("trials-sample.json");
    score_trials(
        &corpus_items(),
        &trials,
        ScoreOptions {
            baseline_tier: baseline,
            ..ScoreOptions::default()
        },
    )
}

#[test]
fn fn_fp_on_separate_denominators_with_ungraded_bucket() {
    let r = sample_report();
    let opus = r.tiers.iter().find(|t| t.bucket == "opus").expect("opus");
    let sonnet = r
        .tiers
        .iter()
        .find(|t| t.bucket == "sonnet")
        .expect("sonnet");
    assert_eq!(
        (
            opus.all.false_negatives,
            opus.all.defect_trials,
            opus.all.false_negative_rate
        ),
        (1, 4, Some(25.0))
    );
    assert_eq!(
        (sonnet.all.false_negatives, sonnet.all.false_negative_rate),
        (2, Some(50.0))
    );
    assert_eq!(
        (
            opus.all.false_positives,
            opus.all.non_defect_trials,
            opus.all.false_positive_rate
        ),
        (2, 5, Some(40.0))
    );
    assert_ne!(
        opus.all.defect_trials, opus.all.non_defect_trials,
        "the denominators differ, so pooling would be a different number"
    );
    assert_eq!(opus.all.ungraded, 1);
    assert_eq!(
        opus.all.defect_trials + opus.all.non_defect_trials + opus.all.ungraded,
        opus.all.trials
    );
    assert_eq!(r.historical_only_trials, 4);
    for t in [opus, sonnet] {
        assert_eq!(t.historical_only_trials, 2);
        assert!(t.by_class.iter().all(|c| c.class != "suggestion"));
    }
}

#[test]
fn flip_rate_per_class_and_authoritative_splits() {
    let r = sample_report();
    let opus = r.tiers.iter().find(|t| t.bucket == "opus").expect("opus");
    let sonnet = r
        .tiers
        .iter()
        .find(|t| t.bucket == "sonnet")
        .expect("sonnet");
    for t in [opus, sonnet] {
        let (a, jc, all) = (&t.authoritative_only, &t.judgement_call_only, &t.all);
        assert_eq!(a.trials + jc.trials, all.trials);
        assert_eq!(a.ungraded + jc.ungraded, all.ungraded);
        assert_eq!(a.defect_trials + jc.defect_trials, all.defect_trials);
        assert_eq!(
            a.non_defect_trials + jc.non_defect_trials,
            all.non_defect_trials
        );
        assert_eq!(a.false_negatives + jc.false_negatives, all.false_negatives);
        assert_eq!(a.false_positives + jc.false_positives, all.false_positives);
    }
    assert_eq!(
        (
            opus.authoritative_only.false_positives,
            opus.judgement_call_only.false_positives
        ),
        (0, 2)
    );
    assert_eq!(sonnet.authoritative_only.false_positives, 2);
    assert_eq!(
        (
            opus.self_consistency.replicate_flips,
            opus.self_consistency.flip_rate
        ),
        (1, Some(25.0))
    );
    assert_eq!(sonnet.self_consistency.replicate_flips, 1);
    let class = |t: &rdm_devtools::measure::refuter_agreement::score::TierScore, c: &str| {
        t.by_class
            .iter()
            .find(|r| r.class == c)
            .map(|r| r.rates.clone())
            .expect(c)
    };
    assert_eq!(class(opus, "real-defect").false_negatives, 1);
    assert_eq!(class(opus, DIVERGENCE_CLASS).false_positives, 2);
    assert_eq!(class(sonnet, DIVERGENCE_CLASS).false_positives, 3);
}

#[test]
fn token_and_tool_columns() {
    let r = sample_report();
    let opus = r.tiers.iter().find(|t| t.bucket == "opus").expect("opus");
    let sonnet = r
        .tiers
        .iter()
        .find(|t| t.bucket == "sonnet")
        .expect("sonnet");
    assert_eq!(
        (opus.cost.output, opus.cost.dispatched_trials),
        (11_500.0, 12)
    );
    assert_eq!(sonnet.cost.output, 24_000.0);
    assert!(
        opus.cost.mean_tokens_per_trial.is_some() && opus.cost.mean_tool_calls_per_trial.is_some()
    );
    assert_eq!(
        opus.token_delta, None,
        "the baseline carries tokenDelta: null"
    );
    assert!(sonnet.token_delta.is_some());
    let text = format_text(&r);
    let fn_block = text.split("## FALSE POSITIVES").next().unwrap_or("");
    let opus_row = fn_block
        .lines()
        .find(|l| l.starts_with("| opus |"))
        .expect("FN row");
    assert!(
        opus_row.contains("25.0%") && opus_row.contains("11,500") && opus_row.contains("13.6"),
        "{opus_row}"
    );
}

fn batched_fixture() -> (
    Vec<CorpusItem>,
    rdm_devtools::measure::refuter_agreement::trials::BatchPlan,
    Vec<JsValue>,
    rdm_devtools::measure::refuter_agreement::trials::Expanded,
) {
    let items = corpus_items();
    let refs: Vec<&JsValue> = items.iter().map(CorpusItem::value).collect();
    let power = group_corpus_for_batching(&refs, MIN_BATCH_GROUP_SIZE).expect("power");
    let plan = build_batch_trials(&power, &["opus".to_owned()], 2, true).expect("plan");
    let group = plan.groups[0].clone();
    let ids = group.ids.clone();
    let dispatched = [
        JsValue::from_json(&j!({
            "dispatchId": "d1", "groupKey": group.key, "tier": "opus", "replicate": 1, "corpusIds": ids,
            "verdicts": [
                {"id": ids[0], "refuted": true, "confidence": 90},
                {"id": "zz-not-in-this-batch", "refuted": true, "confidence": 90},
                {"id": ids[2], "refuted": false, "confidence": 80}
            ],
            "usage": {"output": 100, "uncachedInput": 200, "cacheWrite": 300, "cacheRead": 400},
            "toolCalls": 11
        })),
        JsValue::from_json(
            &j!({"dispatchId": "d2", "groupKey": group.key, "tier": "opus", "replicate": 2, "corpusIds": ids, "verdicts": null, "error": "boom", "usage": {}, "toolCalls": 0}),
        ),
    ];
    let expanded = expand_batch_results(&dispatched);
    let per_finding: Vec<JsValue> = ids
        .iter()
        .enumerate()
        .flat_map(|(i, id)| {
            [1, 2].map(|rep| {
                JsValue::from_json(&j!({
                    "trialId": format!("{id}|opus|{rep}"), "corpusId": id, "tier": "opus", "replicate": rep,
                    "verdict": {"refuted": i == 0, "confidence": 90},
                    "usage": {"output": 100, "uncachedInput": 200, "cacheWrite": 300, "cacheRead": 400}, "toolCalls": 11
                }))
            })
        })
        .collect();
    (items, plan, per_finding, expanded)
}

#[test]
fn batched_scoring_expansion_arm_buckets_and_dispatch_count() {
    let (items, plan, per_finding, expanded) = batched_fixture();
    let group = &plan.groups[0];
    let ids = &group.ids;
    // The batched prompt is a minimal delta from refutePrompt's stance.
    let members: Vec<&CorpusItem> = ids
        .iter()
        .filter_map(|id| items.iter().find(|i| i.id() == id))
        .collect();
    let keyed: Vec<JsValue> = members
        .iter()
        .map(|m| JsValue::from_json(&j!({"refute_id": m.id()})))
        .collect();
    let prompt = build_batch_prompt(
        members[0].mode(),
        &group.dim,
        &keyed,
        Some(members[0].target()),
    );
    assert!(prompt.starts_with("You are a READ-ONLY refuter. Do not edit any files."));
    assert!(prompt.contains("this is NOT a real issue unless the code proves otherwise"));
    for m in &members {
        assert!(prompt.contains(&format!("\"{}\"", m.id())));
    }
    assert_eq!(expanded.rows.len(), ids.len() * 2);
    assert_eq!(expanded.unknown_verdict_ids, ["d1:zz-not-in-this-batch"]);
    assert!(expanded.omitted_ids.contains(&format!("d1:{}", ids[1])));
    let row = |d: &str, id: &str| {
        expanded
            .rows
            .iter()
            .find(|r| {
                r.get("dispatchId").and_then(JsValue::as_str) == Some(d)
                    && r.get("corpusId").and_then(JsValue::as_str) == Some(id)
            })
            .expect("row")
            .to_json()
    };
    assert_eq!(row("d1", &ids[0])["verdict"]["refuted"], true);
    assert_eq!(
        row("d1", &ids[1])["verdict"],
        Value::Null,
        "an omitted id stays ungraded"
    );
    assert_eq!(row("d1", &ids[2])["verdict"]["refuted"], false);
    for id in ids {
        assert_eq!(
            row("d2", id)["verdict"],
            Value::Null,
            "a crashed batch leaves every id ungraded"
        );
    }
    assert_eq!(
        (
            row("d1", &ids[0])["usage"]["output"].as_u64(),
            row("d1", &ids[0])["toolCalls"].as_u64()
        ),
        (Some(100), Some(11))
    );
    assert_eq!(row("d1", &ids[1])["usage"], j!({}));
    assert_eq!(
        (
            row("d1", &ids[0])["positionInBatch"].as_u64(),
            row("d1", &ids[2])["positionInBatch"].as_u64()
        ),
        (Some(1), Some(3))
    );
    assert_eq!(
        bucket_key_for(&JsValue::from_json(&j!({"tier": "opus"}))),
        "opus"
    );
    assert_eq!(
        bucket_key_for(&JsValue::from_json(&j!({"tier": "opus", "arm": "batched"}))),
        "opus|batched"
    );
    let mut rows = per_finding.clone();
    rows.extend(expanded.rows.iter().cloned());
    let report = score_trials(
        &items,
        &rows,
        ScoreOptions {
            baseline_tier: Some("opus".to_owned()),
            ..ScoreOptions::default()
        },
    );
    let bucket = |b: &str| report.tiers.iter().find(|t| t.bucket == b).expect(b);
    let (pf, bt) = (bucket("opus"), bucket("opus|batched"));
    assert_eq!(
        (pf.arm.as_deref(), bt.arm.as_deref()),
        (None, Some("batched"))
    );
    assert_eq!(
        bt.cost.dispatches, 2,
        "expanded rows are not separate dispatches"
    );
    assert_eq!(pf.cost.dispatches, per_finding.len());
    assert_eq!(bt.cost.total_tokens, 1000.0);
    assert_eq!(bt.cost.mean_tokens_per_dispatch, Some(500.0));
    #[allow(clippy::cast_precision_loss)]
    let per_graded = (1000.0 / bt.cost.graded_findings as f64 * 10.0).round() / 10.0;
    assert_eq!(bt.cost.mean_tokens_per_graded_finding, Some(per_graded));
    assert!(bt.cost.graded_findings > bt.cost.dispatches);
    assert!(bt.all.ungraded > 0);
}

#[test]
fn anchoring_over_qualifying_groups_only() {
    let (items, plan, per_finding, expanded) = batched_fixture();
    let mut arms = BTreeMap::new();
    arms.insert("per-finding".to_owned(), per_finding.clone());
    arms.insert("batched".to_owned(), expanded.rows.clone());
    let a = score_anchoring(&plan.groups, &arms, MIN_BATCH_GROUP_SIZE);
    assert_eq!(a.qualifying_groups, plan.groups.len());
    assert_eq!(
        a.arms.iter().map(|x| x.arm.as_str()).collect::<Vec<_>>(),
        ["batched", "per-finding"]
    );
    for arm in &a.arms {
        assert!(
            arm.refutation_rate_by_position
                .by_position
                .iter()
                .all(|p| p.position >= 1)
        );
    }
    let pf = a.arms.iter().find(|x| x.arm == "per-finding").expect("arm");
    assert_eq!(
        (pf.dispatches_considered, pf.all_same_verdict),
        (2, 0),
        "two replicates, each mixing verdicts"
    );
    let mut singleton = plan.groups[0].clone();
    singleton.size = 1;
    singleton.ids.truncate(1);
    let none = score_anchoring(&[singleton], &BTreeMap::new(), MIN_BATCH_GROUP_SIZE);
    assert_eq!(
        none.qualifying_groups, 0,
        "a size-1 group never reaches the anchoring measurement"
    );
    let mut rows = per_finding;
    rows.extend(expanded.rows);
    let report = score_trials(
        &items,
        &rows,
        ScoreOptions {
            baseline_tier: Some("opus".to_owned()),
            anchoring: Some(a),
            ..ScoreOptions::default()
        },
    );
    let text = format_text(&report);
    assert!(text.contains("## ANCHORING") && text.contains("TOKENS PER GRADED FINDING"));
}

#[test]
fn no_blended_accuracy_key_in_any_report() {
    let (items, plan, per_finding, expanded) = batched_fixture();
    let mut arms = BTreeMap::new();
    arms.insert("per-finding".to_owned(), per_finding.clone());
    arms.insert("batched".to_owned(), expanded.rows.clone());
    let anchoring = score_anchoring(&plan.groups, &arms, MIN_BATCH_GROUP_SIZE);
    let mut rows = per_finding;
    rows.extend(expanded.rows);
    for report in [
        sample_report(),
        score_trials(
            &items,
            &rows,
            ScoreOptions {
                baseline_tier: Some("opus".to_owned()),
                anchoring: Some(anchoring),
                ..ScoreOptions::default()
            },
        ),
    ] {
        let value = serde_json::to_value(&report).expect("value");
        assert_eq!(
            find_blended_accuracy_keys(&value, "$"),
            Vec::<String>::new()
        );
        assert_eq!(
            find_blended_accuracy_keys(&json(&format_json(&report).expect("json")), "$"),
            Vec::<String>::new()
        );
        for line in format_text(&report).lines() {
            let lower = line.to_lowercase();
            for word in ["accuracy", "overall correct", "combined rate"] {
                if let Some(at) = lower.find(word) {
                    let tail = &lower[at..];
                    let clause = tail.split(['.', '\n']).next().unwrap_or("");
                    assert!(
                        !clause.chars().any(|c| c.is_ascii_digit()),
                        "a blended figure: {line}"
                    );
                }
            }
        }
    }
    // The detector is not vacuous.
    let planted = j!({"tiers": [{"anchoring": {"combinedRate": 0.5}}], "accuracy": 1});
    assert_eq!(
        find_blended_accuracy_keys(&planted, "$"),
        ["$.accuracy", "$.tiers[0].anchoring.combinedRate"]
    );
}

// --- dry run and dispatch ----------------------------------------------------

#[test]
fn dry_run_matches_golden_and_dispatches_nothing() {
    let (bin, record, _tmp) = fake_claude(j!({"default": {"stdout": "{}", "exit": 0}}));
    let out = ok(&run(&[
        "refuter-agreement",
        "--tiers",
        "opus,sonnet",
        "--replicates",
        "2",
        "--limit",
        "3",
        "--dry-run",
        "--claude-bin",
        s(&bin),
    ]));
    assert_eq!(out, golden(&expected("dry-run-opus-sonnet.txt")));
    let out = ok(&run(&[
        "refuter-agreement",
        "--shape",
        "both",
        "--tiers",
        "opus",
        "--replicates",
        "2",
        "--allow-underpowered",
        "--dry-run",
        "--claude-bin",
        s(&bin),
    ]));
    assert_eq!(
        out,
        golden(&expected("dry-run-both-allow-underpowered.txt"))
    );
    let refused = run(&[
        "refuter-agreement",
        "--shape",
        "batched",
        "--tiers",
        "opus",
        "--dry-run",
        "--claude-bin",
        s(&bin),
    ]);
    assert!(fails(&refused).contains("UNDERPOWERED"));
    assert!(!record.exists(), "a dry run reached the dispatcher");
}

#[test]
fn fake_claude_drives_full_path() {
    let (bin, record, tmp) = fake_claude(j!({
        "responses": [
            {"match": "BATCH_VERDICT", "stdout": "__BATCH__"},
            {"model": "opus", "stdout": claude_body("true", 15, None)},
            {"model": "sonnet", "stdout": claude_body("false", 0, Some("sess-fake"))}
        ]
    }));
    // The sonnet body carries no tool_use: the count is recovered from the
    // session transcript under $HOME/.claude/projects/<slug(checkout)>/.
    let home = tmp.path().join("home");
    let slug =
        rdm_devtools::measure::refuter_agreement::dispatch::project_slug_for(s(&repo_root()));
    let session_dir = home.join(".claude/projects").join(slug);
    std::fs::create_dir_all(&session_dir).expect("mkdir");
    std::fs::write(
        session_dir.join("sess-fake.jsonl"),
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\"},{\"type\":\"tool_use\"},{\"type\":\"tool_use\"},{\"type\":\"tool_use\"},{\"type\":\"tool_use\"},{\"type\":\"tool_use\"},{\"type\":\"tool_use\"},{\"type\":\"tool_use\"}]}}\n",
    )
    .expect("transcript");
    let out_path = tmp.path().join("stub-results.json");
    let out = run_with(
        &[
            "refuter-agreement",
            "--tiers",
            "opus,sonnet",
            "--replicates",
            "2",
            "--limit",
            "4",
            "--label",
            "stubbed",
            "--claude-bin",
            s(&bin),
            "--out",
            s(&out_path),
            "--format",
            "json",
        ],
        &[("HOME", s(&home))],
    );
    let stdout = ok(&out);
    let payload = json(&std::fs::read_to_string(&out_path).expect("payload"));
    assert_eq!(payload["instrument"], "rdm-measure refuter-agreement");
    assert_eq!(payload["trials"].as_array().map(Vec::len), Some(16));
    assert_eq!(payload["label"], "stubbed");
    assert_eq!(payload["corpusSha256"].as_str().map(str::len), Some(64));
    assert_eq!(payload["baselineTier"], "opus");
    let tier = |t: &str| {
        payload["report"]["tiers"]
            .as_array()
            .expect("tiers")
            .iter()
            .find(|x| x["tier"] == t)
            .cloned()
            .expect(t)
    };
    assert_eq!(
        tier("opus")["cost"]["meanToolCallsPerTrial"],
        15,
        "tool calls threaded through the spawn path"
    );
    assert_eq!(
        tier("sonnet")["cost"]["meanToolCallsPerTrial"],
        8,
        "recounted from the session transcript"
    );
    assert_eq!(
        json(&stdout),
        payload["report"],
        "stdout is the report the payload carries"
    );
    assert_eq!(
        find_blended_accuracy_keys(&payload, "$"),
        Vec::<String>::new()
    );
    let seen = calls(&record);
    assert_eq!(seen.len(), 16);
    for c in &seen {
        let argv: Vec<&str> = c["argv"]
            .as_array()
            .expect("argv")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(argv[..2], ["-p", "--model"]);
        assert!(argv.ends_with(&["--output-format", "json"]));
        assert!(
            c["promptBytes"].as_u64().unwrap_or(0) > 401,
            "the prompt arrived on stdin"
        );
        assert_eq!(c["cwd"].as_str().map(PathBuf::from), Some(repo_root()));
    }
    // --score-only re-scores the saved file with no dispatch at all.
    let rescored = ok(&run_no_js(&[
        "refuter-agreement",
        "--score-only",
        s(&out_path),
        "--format",
        "json",
    ]));
    assert_eq!(json(&rescored)["tiers"], payload["report"]["tiers"]);
    assert_eq!(calls(&record).len(), 16);

    // --shape both: two arms over the same items, NO MEASUREMENT, no decision.
    let items = corpus_items();
    let refs: Vec<&JsValue> = items.iter().map(CorpusItem::value).collect();
    let power = group_corpus_for_batching(&refs, MIN_BATCH_GROUP_SIZE).expect("power");
    let ids = power
        .groups
        .iter()
        .find(|g| g.size >= MIN_BATCH_GROUP_SIZE)
        .expect("group")
        .ids
        .clone();
    let graded: Vec<Value> = ids[..ids.len() - 1]
        .iter()
        .enumerate()
        .map(|(i, id)| j!({"id": id, "refuted": i == 0, "confidence": 88, "rationale": "fake"}))
        .collect();
    let batch_body = j!({
        "usage": {"output_tokens": 30, "input_tokens": 60, "cache_creation_input_tokens": 15, "cache_read_input_tokens": 300},
        "messages": [{"message": {"content": [{"type": "tool_use", "name": "StructuredOutput", "input": {"verdicts": graded}}]}}]
    });
    let scenario_path = tmp.path().join("claude.scenario.json");
    let mut scenario = json(&std::fs::read_to_string(&scenario_path).expect("scenario"));
    scenario["responses"][0]["stdout"] = Value::String(batch_body.to_string());
    std::fs::write(&scenario_path, scenario.to_string()).expect("scenario");
    let two = tmp.path().join("two-arm.json");
    let text = ok(&run_with(
        &[
            "refuter-agreement",
            "--shape",
            "both",
            "--tiers",
            "opus",
            "--replicates",
            "2",
            "--allow-underpowered",
            "--label",
            "two-arm",
            "--claude-bin",
            s(&bin),
            "--out",
            s(&two),
        ],
        &[("HOME", s(&home))],
    ));
    assert!(
        text.lines()
            .any(|l| l == "NO MEASUREMENT — batched arm was underpowered")
    );
    assert!(!text.lines().any(|l| l.starts_with("DECISION:")));
    let r = json(&std::fs::read_to_string(&two).expect("two-arm"));
    assert_eq!(
        (r["shape"].as_str(), r["noMeasurement"].as_bool()),
        (Some("both"), Some(true))
    );
    let ids_of = |batched: bool| -> Vec<String> {
        let mut v: Vec<String> = r["trials"]
            .as_array()
            .expect("trials")
            .iter()
            .filter(|t| (t["arm"] == "batched") == batched)
            .filter_map(|t| t["corpusId"].as_str().map(str::to_owned))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    assert_eq!(ids_of(false), ids_of(true), "both arms over the same items");
    let bucket = r["report"]["tiers"]
        .as_array()
        .expect("tiers")
        .iter()
        .find(|t| t["bucket"] == "opus|batched")
        .cloned()
        .expect("batched");
    assert!(
        bucket["all"]["ungraded"].as_u64().unwrap_or(0) > 0,
        "the omitted id is ungraded"
    );
    assert!(
        r["omittedVerdictIds"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    );
    assert_eq!(
        r["report"]["anchoring"]["arms"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(r["report"]["anchoring"]["minGroupSize"], 3);
}

#[test]
fn claude_dispatch_ok_fail_garbage_empty_enoent() {
    let prompt = "X".repeat(500);
    let make = |scenario: Value| {
        let (bin, record, tmp) = fake_claude(scenario);
        let d = Dispatcher {
            claude_bin: bin,
            cwd: tmp.path().to_owned(),
            projects_root: tmp.path().join("projects"),
        };
        (d, record, tmp)
    };
    let (d, record, _t1) = make(j!({"default": {"stdout": claude_body("true", 3, None)}}));
    let ok_out = d.dispatch("sonnet", &prompt).expect("dispatch");
    assert_eq!(
        ok_out
            .verdict
            .as_ref()
            .and_then(|v| v.get("refuted"))
            .and_then(JsValue::as_bool),
        Some(true)
    );
    assert_eq!(ok_out.tool_calls, 3.0);
    assert_eq!(
        ok_out.usage.get("cacheRead").and_then(JsValue::as_f64),
        Some(100.0)
    );
    let c = &calls(&record)[0];
    assert_eq!(c["model"], "sonnet");
    assert_eq!(c["promptBytes"], 500, "the prompt arrived on stdin");
    let (d, _, _t2) =
        make(j!({"default": {"stderr": "Invalid API key - please run /login\n", "exit": 3}}));
    let failed = d.dispatch("sonnet", &prompt).expect("dispatch");
    assert_eq!(
        failed.verdict, None,
        "a failure is ungraded, not refuted:false"
    );
    let e = failed.error.unwrap_or_default();
    assert!(
        e.contains("exited 3") && e.contains("Invalid API key"),
        "{e}"
    );
    let (d, _, _t3) = make(j!({"default": {"stdout": "Rate limit reached. Try again later.\n"}}));
    assert!(
        d.dispatch("sonnet", &prompt)
            .expect("dispatch")
            .error
            .unwrap_or_default()
            .contains("non-JSON body")
    );
    let (d, _, _t4) = make(j!({"default": {"stdout": ""}}));
    assert!(
        d.dispatch("sonnet", &prompt)
            .expect("dispatch")
            .error
            .unwrap_or_default()
            .contains("non-JSON body")
    );
    let missing = Dispatcher {
        claude_bin: PathBuf::from("/nonexistent/claude"),
        cwd: std::env::temp_dir(),
        projects_root: std::env::temp_dir(),
    };
    let fatal = missing
        .dispatch("sonnet", &prompt)
        .expect_err("a missing binary is fatal");
    assert!(
        fatal.contains("`claude` binary was not found") && fatal.contains("--dry-run"),
        "{fatal}"
    );
    // Through the CLI the fatal error aborts the run with exit 1.
    let err = fails(&run(&[
        "refuter-agreement",
        "--tiers",
        "opus",
        "--limit",
        "1",
        "--claude-bin",
        "/nonexistent/claude",
    ]));
    assert!(err.contains("`claude` binary was not found"), "{err}");
}

#[test]
fn concurrency_does_not_change_output_order() {
    let items = corpus_items();
    let slow: Vec<Value> = items
        .iter()
        .take(6)
        .enumerate()
        .map(|(i, it)| {
            let snippet: String = it.finding().get("id").and_then(JsValue::as_str).unwrap_or("").to_owned();
            j!({"match": snippet, "sleepMs": (6 - i) * 40, "stdout": claude_body(if i % 2 == 0 { "true" } else { "false" }, 2, None)})
        })
        .collect();
    let (bin, _record, tmp) =
        fake_claude(j!({"responses": slow, "default": {"stdout": claude_body("true", 1, None)}}));
    let run_at = |n: &str| {
        let out = tmp.path().join(format!("c{n}.json"));
        ok(&run(&[
            "refuter-agreement",
            "--tiers",
            "opus",
            "--replicates",
            "1",
            "--limit",
            "6",
            "--concurrency",
            n,
            "--claude-bin",
            s(&bin),
            "--out",
            s(&out),
            "--format",
            "json",
        ]));
        json(&std::fs::read_to_string(out).expect("payload"))
    };
    let serial = run_at("1");
    let parallel = run_at("4");
    assert_eq!(
        serial, parallel,
        "concurrency must not change the recorded order or figures"
    );
    let ids: Vec<&str> = serial["trials"]
        .as_array()
        .expect("trials")
        .iter()
        .filter_map(|t| t["corpusId"].as_str())
        .collect();
    let want: Vec<&str> = items.iter().take(6).map(CorpusItem::id).collect();
    assert_eq!(ids, want, "trial-plan order");
}

// --- plan-time validation and audits ----------------------------------------

#[test]
fn illegal_tier_rejected_at_plan_time() {
    let err = fails(&run_no_js(&[
        "refuter-agreement",
        "--tiers",
        "not-a-real-tier",
        "--dry-run",
    ]));
    assert!(err.contains("not a tier alias"), "{err}");
    let err = fails(&run_no_js(&[
        "refuter-agreement",
        "--filter-class",
        "nonsense",
        "--batch-power",
    ]));
    assert!(
        err.contains("--filter-class \"nonsense\" is not a known ground-truth class"),
        "{err}"
    );
    for (flag, value) in [
        ("--replicates", "0"),
        ("--concurrency", "x"),
        ("--limit", "1.5"),
        ("--min-batch-group", "1"),
    ] {
        let err = fails(&run_no_js(&[
            "refuter-agreement",
            flag,
            value,
            "--batch-power",
        ]));
        assert!(err.contains(flag), "{flag}: {err}");
    }
    for (flag, value) in [
        ("--shape", "sideways"),
        ("--audit-section", "other"),
        ("--format", "yaml"),
    ] {
        let err = fails(&run_no_js(&["refuter-agreement", flag, value]));
        assert!(err.contains(flag), "{flag}: {err}");
    }
    let err = fails(&run_no_js(&[
        "refuter-agreement",
        "--score-only",
        "tests/fixtures/refuter-agreement/trials-sample.json",
        "--tiers",
    ]));
    assert!(err.contains("--tiers"), "{err}");
}

#[test]
fn only_unknown_id_fails() {
    let err = fails(&run_no_js(&[
        "refuter-agreement",
        "--only",
        "no-such-id,also-missing",
        "--batch-power",
    ]));
    assert!(
        err.contains("--only names 2 id(s) not in the corpus: no-such-id, also-missing"),
        "{err}"
    );
    let first = corpus_items()[0].id().to_owned();
    ok(&run_no_js(&[
        "refuter-agreement",
        "--only",
        &first,
        "--batch-power",
    ]));
}

#[test]
fn audit_committed_tiering_ok() {
    let out = ok(&run_no_js(&[
        "refuter-agreement",
        "--audit",
        "docs/token-baseline.json",
    ]));
    assert!(
        out.contains("refuterModelTiering figures are internally consistent"),
        "{out}"
    );
}

#[test]
fn audit_committed_batching_ok() {
    let out = ok(&run_no_js(&[
        "refuter-agreement",
        "--audit",
        "docs/token-baseline.json",
        "--audit-section",
        "refuterBatching",
    ]));
    assert!(
        out.contains("refuterBatching figures are internally consistent"),
        "{out}"
    );
}

#[test]
fn audit_rejects_edited_tiering_figure() {
    let doc = edited_doc(&repo_root().join("docs/token-baseline.json"), |d| {
        let t = &mut d["refuterModelTiering"]["tiers"][0]["cost"]["totalTokens"];
        *t = j!(t.as_u64().unwrap_or(0) + 1);
    });
    let err = fails(&run_no_js(&["refuter-agreement", "--audit", s(doc.path())]));
    assert!(err.contains("cost.totalTokens"), "{err}");
}

/// One planted edit to a doc section.
type Mutation = Box<dyn Fn(&mut Value)>;

#[test]
fn audit_rejects_edited_batching_figure() {
    let mutations: Vec<(&str, Mutation)> = vec![
        (
            "exclusion count",
            Box::new(|s: &mut Value| {
                let v = &mut s["corpusPower"]["unrecoverableUnitExcluded"];
                *v = j!(v.as_u64().unwrap_or(0) + 1);
            }),
        ),
        (
            "qualifying groups",
            Box::new(|s: &mut Value| {
                let v = &mut s["corpusPower"]["qualifyingGroups"];
                *v = j!(v.as_u64().unwrap_or(0) + 1);
            }),
        ),
        (
            "size-1 folded in",
            Box::new(|s: &mut Value| {
                let v = &mut s["corpusPower"]["qualifyingItems"];
                *v = j!(v.as_u64().unwrap_or(0) + 24);
            }),
        ),
        (
            "meetsMinimum",
            Box::new(|s: &mut Value| s["corpusPower"]["meetsMinimum"] = j!(true)),
        ),
        (
            "decision on an underpowered arm",
            Box::new(|s: &mut Value| s["decision"] = j!("ship-batched")),
        ),
        (
            "decision outside the closed set",
            Box::new(|s: &mut Value| s["decision"] = j!("looks-fine")),
        ),
        (
            "per-mode partition",
            Box::new(|s: &mut Value| s["corpusPower"]["sizeHistogramByMode"]["plan"]["3"] = j!(1)),
        ),
    ];
    for (label, mutate) in mutations {
        let doc = edited_doc(&repo_root().join("docs/token-baseline.json"), |d| {
            mutate(&mut d["refuterBatching"])
        });
        let out = run_no_js(&[
            "refuter-agreement",
            "--audit",
            s(doc.path()),
            "--audit-section",
            "refuterBatching",
        ]);
        assert_eq!(out.status.code(), Some(1), "--audit missed: {label}");
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).expect("copy");
        }
    }
}
