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

// classifyOutcome — the total, deterministic decision tree. Returns one of
// 'escalated' | 'reviewed' | 'rework'.
//
// The deterministic pipeline cannot classify a code finding's *nature* (the
// FINDING schema carries severity but no fixable/decision flag), so a code
// defect that survives the one bounded rework resolves to 'rework'; genuine
// decisions surface earlier at the plan gate as 'escalated'. That is why the
// code stage yields only reviewed|rework and escalated originates at the plan
// gate.
function classifyOutcome(input) {
  const i = input || {};
  const tier = i.tier;
  const planFindings = i.planFindings || [];
  const codeFindings = i.codeFindings || [];
  const codeFindingsAfterRework = i.codeFindingsAfterRework || [];
  // 1. Plan gate: a blocking plan finding escalates before any implementation.
  //    An empty/ambiguous plan is surfaced as a blocking coherence finding by
  //    the plan-review stage, so that case lands here too.
  if (hasBlocking(planFindings, tier)) return 'escalated';
  // 2. Plan approved → implement ran → code-review ran.
  //    Clean first pass → reviewed.
  if (!hasBlocking(codeFindings, tier)) return 'reviewed';
  //    Otherwise the one bounded rework ran; judge its result.
  if (!hasBlocking(codeFindingsAfterRework, tier)) return 'reviewed';
  //    Budget exhausted with a surviving defect → rework (phase back to work).
  return 'rework';
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
  const codeFindings = i.codeFindings || [];
  const codeFindingsAfterRework = i.codeFindingsAfterRework || [];
  const outcome = classifyOutcome({
    planFindings: planFindings,
    codeFindings: codeFindings,
    codeFindingsAfterRework: codeFindingsAfterRework,
    tier: tier,
  });
  let findings;
  let summary;
  if (outcome === 'escalated') {
    findings = planFindings;
    summary = 'plan gate escalated: ' + summarizeFindings(planFindings);
  } else if (outcome === 'rework') {
    findings = codeFindingsAfterRework;
    summary = 'code rework unresolved: ' + summarizeFindings(codeFindingsAfterRework);
  } else {
    // reviewed — surface whichever pass came back clean of blockers.
    findings = hasBlocking(codeFindings, tier) ? codeFindingsAfterRework : codeFindings;
    summary = 'phase reviewed clean: ' + summarizeFindings(findings);
  }
  return { roadmap: roadmap, phase: phase, outcome: outcome, summary: summary, findings: findings };
}
// >>> dispatch-outcome:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export { hasBlocking, summarizeFindings, classifyOutcome, buildOutcome };
