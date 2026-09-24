//! Negative controls: the legacy suites' opt-in mutation modes
//! (`CODEX_QUEUE_TEST_MUTATION`, `CODEX_REVIEW_TEST_MUTATION`), run on every
//! `cargo nextest run`. Each copies the runtime's dependency closure into a
//! private temp tree ([`MutantTree`]; the checkout is never written), plants
//! one edit in the copy's `scripts/lib/codex-runtime.mjs`, runs the same
//! observation its positive test uses, and requires that test's check to
//! fail. A missing or ambiguous anchor, or a run that never reaches the
//! planted logic, is reported as "negative control not run", never a pass.

use rdm_devtools::workflow::MutantTree;
use serde_json::Value;

use crate::runner::{Fixture, check_bounded, check_models, observe_preview, observe_runtime};
use crate::support::repo;
use crate::workflow_support::{Failure, Lib, infra};

/// Everything `scripts/rdm-codex.mjs` imports, with relative paths kept so the
/// copy's own imports resolve inside the copy.
const CLOSURE: &[&str] = &[
    "scripts/rdm-codex.mjs",
    "scripts/lib/codex-runtime.mjs",
    "scripts/lib/codex-runtime-state.mjs",
    "scripts/lib/codex-process.mjs",
    "scripts/lib/codex-runtime-estimate.mjs",
    ".claude/workflows/lib/review.mjs",
    ".claude/workflows/lib/plan-review.mjs",
    ".claude/workflows/lib/estimate.mjs",
];

/// A private copy of the closure with one edit planted in the runtime.
fn mutant(name: &str, from: &str, to: &str) -> Result<(MutantTree, Lib), Failure> {
    let tree = MutantTree::copy(&repo(), CLOSURE).map_err(infra)?;
    tree.replace_once(crate::runner::RUNTIME, name, from, to)
        .map_err(infra)?;
    let lib = Lib::at(tree.root());
    Ok((tree, lib))
}

fn not_run(f: impl std::fmt::Display) -> ! {
    panic!("negative control not run: {f}")
}

#[test]
fn unbounded_agent_semaphore_exceeds_two() {
    let (_tree, lib) = mutant(
        "unbounded-agent-semaphore",
        "if(active>=concurrency)await new Promise(r=>queue.push(r));else active++;",
        "active++;",
    )
    .unwrap_or_else(|f| not_run(f));
    let fx = Fixture::queue().unwrap_or_else(|f| not_run(f));
    let run = observe_preview(&lib, &fx).unwrap_or_else(|f| not_run(f));
    // Inconclusive unless the mutant run completed the same workload.
    let proposals = serde_json::from_str::<Value>(&run.output)
        .ok()
        .and_then(|v| v["result"]["proposed"].as_array().map(Vec::len));
    let launched = run.events.iter().filter(|e| e.kind == "start").count();
    if !run.success || proposals != Some(5) || launched != 5 {
        not_run(format!(
            "the mutant preview did not complete its five judgments (exit ok: {}, proposals: {proposals:?}, launched: {launched}):\n{}",
            run.success, run.output
        ));
    }
    match check_bounded(&run) {
        Err(Failure::Check(m)) => eprintln!("mutant caught: {m}"),
        Err(other) => not_run(other),
        Ok(()) => panic!("the planted mutant survived: live children stayed bounded"),
    }
    let peak = crate::support::peak(&run.events);
    assert!(peak > 2, "an unbounded semaphore launched {peak} at once");
}

#[test]
fn refuter_mapped_to_review_find_is_detected() {
    let (_tree, lib) = mutant(
        "refuter-on-review-find",
        "role === 'refuter' ? 'review-verify'",
        "role === 'refuter' ? 'review-find'",
    )
    .unwrap_or_else(|f| not_run(f));
    let fx = Fixture::review("code-review", false, None).unwrap_or_else(|f| not_run(f));
    let (find, verify) = (&fx.expected["review-find"], &fx.expected["review-verify"]);
    if find["model"] == verify["model"] && find["effort"] == verify["effort"] {
        not_run(format!(
            "review-find and review-verify resolve to the same profile, so the control cannot discriminate: {find} / {verify}"
        ));
    }
    let run = observe_runtime(&lib, &fx).unwrap_or_else(|f| not_run(f));
    let refuters: Vec<_> = run
        .events
        .iter()
        .filter(|e| e.kind == "start" && e.role == "refuter")
        .collect();
    if refuters.len() != 1 {
        not_run(format!("expected one refuter call, saw {:?}", run.events));
    }
    match check_models(&fx, &run) {
        Err(Failure::Check(m)) => eprintln!("mutant caught: {m}"),
        Err(other) => not_run(other),
        Ok(()) => panic!("the planted mutant survived: every role ran on its own profile"),
    }
    let call = run
        .calls
        .iter()
        .find(|c| c.pid == refuters[0].pid)
        .expect("the refuter call was recorded");
    assert_eq!(call.after("-m"), find["model"].as_str(), "{:?}", call.argv);
    let effort = format!(
        "model_reasoning_effort=\"{}\"",
        find["effort"].as_str().unwrap_or_default()
    );
    assert!(call.configs().contains(&effort.as_str()), "{:?}", call.argv);
}
