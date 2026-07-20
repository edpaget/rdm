//! dispatch-phase — pure decision logic for the keystone dispatch pipeline.
//!
//! This is the **single source of truth** for the deterministic decision core of
//! the dispatch-phase workflow: given the surviving findings from the plan-review
//! and code-review gates, decide the phase OUTCOME. Because the Claude Code
//! Workflow runtime cannot `import`/`require` (see docs/workflow-schemas.md
//! § "Import spike"), the marked block below is copied BYTE-IDENTICAL into
//! `.claude/workflows/dispatch-phase.js`. Unlike the review-refute-fix block —
//! which is stamped by `scripts/gen-workflow-review.sh` — this second block is NOT
//! run through the generator; instead `scripts/verify-workflow-dispatch.sh` gates
//! the two copies for byte-equality, which achieves the same drift protection
//! without teaching the generator a second block.
//!
//! Everything the block needs is self-contained (no imports, pure array/string
//! ops, no Date.now / Math.random). The `export { … }` at the bottom lives
//! OUTSIDE the markers so it is never copied into the workflow script (whose only
//! permitted export is `meta`). The verify harness imports this module and unit-
//! tests the pure logic with fabricated ranked finding arrays — zero LLM calls.

// >>> dispatch-outcome:begin <<<
// Pure, deterministic decision logic for the dispatch-phase pipeline.
//
// This block is the single source of truth in
// .claude/workflows/lib/dispatch-phase.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/dispatch-phase.js (the Workflow runtime cannot load modules
// at run time). scripts/verify-workflow-dispatch.sh gates the two copies for
// drift. No Date.now / Math.random — pure array/string ops only.

// hasBlocking(findings, tier) — is there a blocking finding, tier-scaled?
// For the `large` tier a surviving `concern` is treated as blocking too (a
// one-directional tightening — the gate can only get stricter, never looser).
function hasBlocking(findings, tier) {
  const list = Array.isArray(findings) ? findings : [];
  const blockers = tier === 'large' ? ['blocking', 'concern'] : ['blocking'];
  return list.some((f) => f && blockers.indexOf(f.severity) !== -1);
}

// summarizeFindings(findings) — a deterministic one-line label. The array is
// assumed already ranked (most-severe first), so the top finding is list[0].
function summarizeFindings(findings) {
  const list = Array.isArray(findings) ? findings : [];
  if (list.length === 0) return 'no surviving findings';
  const top = list[0] || {};
  const sev = top.severity || 'finding';
  const what = top.what_fails || top.concern || top.id || 'unspecified';
  return list.length + ' finding(s); top: [' + sev + '] ' + what;
}

// DEFAULT_MAX_PLAN_REVISE / DEFAULT_MAX_CODE_REWORK — the two in-run retry
// budgets. They are counted INDEPENDENTLY: a plan that took two revisions
// consumes no code-rework budget, and vice versa.
//
// A budget of N means N reworks AFTER the original attempt, i.e. N + 1 attempts:
//   plan: plan → review → revise 1 → review → revise 2 → review → escalate
//   code: implement → review → rework 1 → review → rework 2 → review → rework
//
// 0 is legal and MEANINGFUL: no reworks at all — terminate on the first blocking
// review. It must never be conflated with "unset" by a falsy check.
const DEFAULT_MAX_PLAN_REVISE = 2;
const DEFAULT_MAX_CODE_REWORK = 2;

// parseBudget(value, flag, fallback) — validate a per-run budget override.
// Unset (null/undefined/'') falls back to the caller's default. Anything else
// must be a non-negative integer; a non-integer string is REJECTED rather than
// silently coerced (parseInt('2abc') === 2 is exactly the trap to avoid).
function parseBudget(value, flag, fallback) {
  if (value === null || value === undefined || value === '') return fallback;
  let n = NaN;
  if (typeof value === 'number') {
    n = value;
  } else if (typeof value === 'string' && /^[+-]?[0-9]+$/.test(value.trim())) {
    n = parseInt(value.trim(), 10);
  }
  if (!Number.isInteger(n) || n < 0 || Object.is(n, -0)) {
    throw new Error(
      'dispatch-phase: ' +
        flag +
        ' must be a non-negative integer (got "' +
        String(value) +
        '") — 0 means no reworks, terminate on the first blocking review'
    );
  }
  return n;
}

// parseDispatchArgs(args) — coerce and validate the whole args payload.
//
// The Workflow tool contract forbids stringified args, but LLM callers (rdm-do
// --auto and hand-run single phases) invoke dispatch-phase DIRECTLY and have
// delivered a JSON string; coerce once, then derive every field from it. Budget
// validation runs HERE, at parse time — before any agent() call — so an invalid
// budget can never burn tokens.
function parseDispatchArgs(args) {
  let dispatchArgs = args || {};
  if (typeof dispatchArgs === 'string') {
    try {
      dispatchArgs = JSON.parse(dispatchArgs) || {};
    } catch (e) {
      dispatchArgs = {};
    }
  }
  if (!dispatchArgs || typeof dispatchArgs !== 'object') dispatchArgs = {};
  return {
    roadmap: dispatchArgs.roadmap || '',
    phase: dispatchArgs.phase || '',
    // Task mode: `{ task: <slug> }` dispatches a standalone task instead of a
    // phase — no roadmap, no tier, its own `task/<slug>` worktree.
    task: dispatchArgs.task || '',
    planOnly: !!dispatchArgs.planOnly,
    maxPlanRevise: parseBudget(dispatchArgs.maxPlanRevise, 'maxPlanRevise', DEFAULT_MAX_PLAN_REVISE),
    maxCodeRework: parseBudget(dispatchArgs.maxCodeRework, 'maxCodeRework', DEFAULT_MAX_CODE_REWORK),
  };
}

// runPlanGate(config, deps) — the bounded plan stage. Author a plan, review it,
// and revise up to `config.maxRevise` times, breaking early the moment a review
// comes back with no blockers. Returns
// { fetchError, stage, planDoc, findings, reviewCount, reviseCount }.
//
// Every side effect is reached through the injected `deps` (d.plan / d.revise /
// d.review), so this block names NO ambient runtime global and the module
// imports cleanly in Node — the lib/autopilot.mjs precedent, which is what makes
// the budget loop testable at all.
//
// agent() RESOLVES to null on an unknown/unavailable model id rather than
// throwing (spike consequence 3), so BOTH the initial plan and EVERY revise
// result are null-guarded. The revise guard runs before the reassignment and
// before the next review, so a null doc is never reviewed as an empty plan and
// never clobbers the last good one.
async function runPlanGate(config, deps) {
  const c = config || {};
  const d = deps || {};
  const maxRevise = c.maxRevise != null ? c.maxRevise : DEFAULT_MAX_PLAN_REVISE;
  const tier = c.tier;
  let planDoc = await d.plan();
  if (planDoc === null || planDoc === undefined) {
    return { fetchError: true, stage: 'plan', planDoc: null, findings: [], reviewCount: 0, reviseCount: 0 };
  }
  let findings = await d.review(planDoc);
  let reviewCount = 1;
  let reviseCount = 0;
  for (let i = 0; i < maxRevise; i++) {
    if (!hasBlocking(findings, tier)) break;
    const revised = await d.revise(planDoc, findings);
    reviseCount++;
    if (revised === null || revised === undefined) {
      return {
        fetchError: true,
        stage: 'revise',
        planDoc: planDoc,
        findings: findings,
        reviewCount: reviewCount,
        reviseCount: reviseCount,
      };
    }
    planDoc = revised;
    findings = await d.review(planDoc);
    reviewCount++;
  }
  return {
    fetchError: false,
    stage: null,
    planDoc: planDoc,
    findings: findings,
    reviewCount: reviewCount,
    reviseCount: reviseCount,
  };
}

// runCodeGate(config, deps) — the bounded code stage. Implement, review, and
// rework up to `config.maxRework` times, breaking early on a clean review.
// Returns { findings, rounds, reworkCount, reviewCount } where `rounds` is the
// per-round review findings in order (always at least one entry).
//
// No null guard is needed here: `implement` returns no document the pipeline
// consumes, and the review pipeline already converts an all-null finder sweep
// into a blocking finding.
async function runCodeGate(config, deps) {
  const c = config || {};
  const d = deps || {};
  const maxRework = c.maxRework != null ? c.maxRework : DEFAULT_MAX_CODE_REWORK;
  const tier = c.tier;
  await d.implement(null);
  let findings = await d.review();
  const rounds = [findings];
  let reworkCount = 0;
  for (let i = 0; i < maxRework; i++) {
    if (!hasBlocking(findings, tier)) break;
    await d.implement(findings);
    reworkCount++;
    findings = await d.review();
    rounds.push(findings);
  }
  return { findings: findings, rounds: rounds, reworkCount: reworkCount, reviewCount: rounds.length };
}

// codeReviewRounds(input) — the per-round code-review findings, newest last.
//
// The modern caller passes `codeReviews` (runCodeGate's `rounds`), which already
// records exactly the rounds that ran — however many, INCLUDING zero reworks.
// The legacy two-slot shape (`codeFindings` + `codeFindingsAfterRework`) is
// derived: a second round only existed if the rework budget was non-zero AND the
// first pass was blocking. That guard is the fix for the budget-0 hole, where an
// always-empty `codeFindingsAfterRework` used to mark a failing first review
// clean.
function codeReviewRounds(input) {
  const i = input || {};
  if (Array.isArray(i.codeReviews) && i.codeReviews.length) return i.codeReviews;
  const first = i.codeFindings || [];
  const maxRework = i.maxRework != null ? i.maxRework : DEFAULT_MAX_CODE_REWORK;
  if (maxRework > 0 && hasBlocking(first, i.tier)) return [first, i.codeFindingsAfterRework || []];
  return [first];
}

// classifyOutcome — the total, deterministic decision tree. Returns one of
// 'escalated' | 'reviewed' | 'rework'.
//
// The deterministic pipeline cannot classify a code finding's *nature* (the
// FINDING schema carries severity but no fixable/decision flag), so a code
// defect that survives the bounded reworks resolves to 'rework'; genuine
// decisions surface earlier at the plan gate as 'escalated'. That is why the
// code stage yields only reviewed|rework and escalated originates at the plan
// gate.
function classifyOutcome(input) {
  const i = input || {};
  const tier = i.tier;
  const planFindings = i.planFindings || [];
  // 1. Plan gate: a blocking plan finding escalates before any implementation.
  //    An empty/ambiguous plan is surfaced as a blocking coherence finding by
  //    the plan-review stage, so that case lands here too.
  if (hasBlocking(planFindings, tier)) return 'escalated';
  // 2. Plan approved → implement ran → code-review ran (once per round). The
  //    LAST review's findings decide, for any number of rework rounds including
  //    zero: still blocking → rework, otherwise reviewed.
  const rounds = codeReviewRounds(i);
  const last = rounds[rounds.length - 1] || [];
  return hasBlocking(last, tier) ? 'rework' : 'reviewed';
}

// buildOutcome — the OUTCOME contract { roadmap, phase, outcome, summary,
// findings }. fetchError short-circuits to escalated. Never emits a land-time
// completion directive.
function buildOutcome(input) {
  const i = input || {};
  const roadmap = i.roadmap;
  const phase = i.phase;
  const tier = i.tier;
  if (i.fetchError === true) {
    return { roadmap: roadmap, phase: phase, outcome: 'escalated', summary: 'phase fetch failed', findings: [] };
  }
  const planFindings = i.planFindings || [];
  const classifierInput = {
    planFindings: planFindings,
    codeFindings: i.codeFindings,
    codeFindingsAfterRework: i.codeFindingsAfterRework,
    codeReviews: i.codeReviews,
    maxRework: i.maxRework,
    tier: tier,
  };
  const outcome = classifyOutcome(classifierInput);
  // The LAST code-review round is what both the rework and reviewed payloads
  // report — never a stale earlier pass, whatever the rework budget was.
  const rounds = codeReviewRounds(classifierInput);
  const lastRound = rounds[rounds.length - 1] || [];
  let findings;
  let summary;
  if (outcome === 'escalated') {
    findings = planFindings;
    summary = 'plan gate escalated: ' + summarizeFindings(planFindings);
  } else if (outcome === 'rework') {
    findings = lastRound;
    summary = 'code rework unresolved: ' + summarizeFindings(lastRound);
  } else {
    findings = lastRound;
    summary = 'phase reviewed clean: ' + summarizeFindings(lastRound);
  }
  return { roadmap: roadmap, phase: phase, outcome: outcome, summary: summary, findings: findings };
}

// buildTaskOutcome — the task-shaped OUTCOME contract { task, outcome, summary,
// findings }. A task is keyed by slug and belongs to no roadmap, so it emits a
// `task` identifier instead of `roadmap`/`phase`; the decision core
// (classifyOutcome / hasBlocking / summarizeFindings) is shared UNCHANGED with
// the phase path. Tasks always dispatch at the fixed `medium` tier, so the
// `large` gate-tightening in hasBlocking never applies to them. fetchError
// short-circuits to escalated. Never emits a land-time completion directive.
function buildTaskOutcome(input) {
  const i = input || {};
  const task = i.task;
  const tier = i.tier;
  if (i.fetchError === true) {
    return { task: task, outcome: 'escalated', summary: 'task fetch failed', findings: [] };
  }
  const planFindings = i.planFindings || [];
  const classifierInput = {
    planFindings: planFindings,
    codeFindings: i.codeFindings,
    codeFindingsAfterRework: i.codeFindingsAfterRework,
    codeReviews: i.codeReviews,
    maxRework: i.maxRework,
    tier: tier,
  };
  const outcome = classifyOutcome(classifierInput);
  const rounds = codeReviewRounds(classifierInput);
  const lastRound = rounds[rounds.length - 1] || [];
  let findings;
  let summary;
  if (outcome === 'escalated') {
    findings = planFindings;
    summary = 'plan gate escalated: ' + summarizeFindings(planFindings);
  } else if (outcome === 'rework') {
    findings = lastRound;
    summary = 'code rework unresolved: ' + summarizeFindings(lastRound);
  } else {
    findings = lastRound;
    summary = 'task reviewed clean: ' + summarizeFindings(lastRound);
  }
  return { task: task, outcome: outcome, summary: summary, findings: findings };
}
// >>> dispatch-outcome:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  DEFAULT_MAX_PLAN_REVISE,
  DEFAULT_MAX_CODE_REWORK,
  parseBudget,
  parseDispatchArgs,
  runPlanGate,
  runCodeGate,
  codeReviewRounds,
  hasBlocking,
  summarizeFindings,
  classifyOutcome,
  buildOutcome,
  buildTaskOutcome,
};
