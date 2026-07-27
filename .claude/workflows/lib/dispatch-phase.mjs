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
//!
//! The verdict half of the decision core — `classifyOutcome` and its helpers
//! `hasBlocking`, `summarizeFindings`, `codeReviewRounds`, and
//! `DEFAULT_MAX_CODE_REWORK` — was LIFTED into `lib/review.mjs`, the canonical
//! review source, so every surface shares one classifier. In the `.js` consumer
//! those names arrive via the stamped review block (which is positioned BEFORE
//! this block, since `const DEFAULT_MAX_CODE_REWORK` is TDZ-bound and, unlike a
//! function declaration, does not hoist). In Node they arrive via the import
//! below, which lives OUTSIDE the markers and is re-exported for the harness.

import {
  classifyOutcome,
  codeReviewRounds,
  hasBlocking,
  acTableHasGap,
  summarizeFindings,
  statusFor,
  writesCompletion,
  deriveSignals,
  DEFAULT_MAX_CODE_REWORK,
} from './review.mjs';

// >>> dispatch-outcome:begin <<<
// Pure, deterministic decision logic for the dispatch-phase pipeline.
//
// This block is the single source of truth in
// .claude/workflows/lib/dispatch-phase.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/dispatch-phase.js (the Workflow runtime cannot load modules
// at run time). scripts/verify-workflow-dispatch.sh gates the two copies for
// drift. No Date.now / Math.random — pure array/string ops only.
//
// `hasBlocking`, `summarizeFindings`, `codeReviewRounds`, `classifyOutcome`,
// `statusFor`, `writesCompletion`, and `DEFAULT_MAX_CODE_REWORK` are NOT declared
// here: they belong to the canonical review source (lib/review.mjs) and reach
// this block from the stamped review block that precedes it in the workflow
// consumer.

// DEFAULT_MAX_PLAN_REVISE — the in-run plan-revision budget. It is counted
// INDEPENDENTLY of the code-rework budget (DEFAULT_MAX_CODE_REWORK, which lives
// in the review source): a plan that took two revisions consumes no code-rework
// budget, and vice versa.
//
// A budget of N means N reworks AFTER the original attempt, i.e. N + 1 attempts:
//   plan: plan → review → revise 1 → review → revise 2 → review → escalate
//   code: implement → review → rework 1 → review → rework 2 → review → rework
//
// 0 is legal and MEANINGFUL: no reworks at all — terminate on the first blocking
// review. It must never be conflated with "unset" by a falsy check.
const DEFAULT_MAX_PLAN_REVISE = 2;

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
    // --- Optional caller-supplied hoists (see docs/mechanical-agent-inventory.md).
    // A caller that is ALREADY a running agent with the repo in context (the
    // rdm-dispatch-phase / rdm-do --auto shims) can run the mechanical command
    // itself and pass the result here, so the workflow never spawns a dedicated
    // subagent for it. Every one of these is OPTIONAL: absent/malformed simply
    // falls through to the in-workflow agent, which is exactly what a direct
    // `Workflow` invocation (no caller) does.
    phaseMeta: dispatchArgs.phaseMeta && typeof dispatchArgs.phaseMeta === 'object' ? dispatchArgs.phaseMeta : null,
    taskMeta: dispatchArgs.taskMeta && typeof dispatchArgs.taskMeta === 'object' ? dispatchArgs.taskMeta : null,
    // The caller already wrote `--status in-progress` itself and it exited 0, so
    // the workflow's own observability stamp is redundant. NEVER set by a
    // --plan-only invocation (the workflow suppresses the stamp there anyway).
    alreadyInProgress: !!dispatchArgs.alreadyInProgress,
  };
}

// hoistedMetaComplete(meta, isTask) — the ALL-OR-NOTHING guard on a caller-
// supplied phase/task meta payload. A hoisted meta replaces a fetch agent that
// did TWO things: read the item body AND resolve the five per-step model ids.
// Accepting a partial payload would therefore save nothing (the driver would
// still need a model-resolving agent) while actively breaking the run: an
// incomplete `models` map trips the driver's `unresolvedStep` check and
// short-circuits the whole dispatch as a fetchError. So: accept only when the
// body is a non-empty string AND all five model ids are non-empty strings —
// otherwise reject and let the original agent run untouched.
//
// `isTask` selects between the two schemas' requirements. A TASK_META payload
// carries no roadmap/stem/model tier and the driver hard-codes a task's tier to
// `medium`, so body + models are all it needs. A PHASE_META payload is
// different: `meta.model` is the phase's DIFFICULTY TIER, and it is the driver's
// SOLE source for it — unlike `stem`/`roadmap`, which fall back to values the
// top-level args already carry, an absent tier falls back to a hard-coded
// 'medium'. That default is not neutral: `hasBlocking` scales with the tier, and
// a `large` phase silently downgraded to `medium` stops treating a surviving
// `concern` finding as blocking — loosening the gate, the opposite direction
// from the one-directional tightening this gate exists to uphold. So the phase
// case ALSO requires a non-empty string `model`, mirroring PHASE_META_SCHEMA's
// own `required` list; anything short of that falls back to the fetch agent,
// which always supplies it.
function hoistedMetaComplete(meta, isTask) {
  if (!meta || typeof meta !== 'object') return false;
  if (typeof meta.body !== 'string' || String(meta.body).trim() === '') return false;
  // Phase mode only: the difficulty tier has no recoverable fallback.
  if (!isTask && (typeof meta.model !== 'string' || meta.model.trim() === '')) return false;
  const m = meta.models;
  if (!m || typeof m !== 'object') return false;
  const keys = ['plan', 'implement', 'review_find', 'review_verify', 'mechanical'];
  return keys.filter((k) => typeof m[k] !== 'string' || m[k] === '').length === 0;
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
// `d.review(planDoc)` is a `runReview` from the canonical review source and
// therefore resolves `{ survivors, acTable }`, not a bare array — both call
// sites below destructure it. `acTable` is discarded: plan mode never sets it
// (the `ac` dimension does not exist there), and `hasBlocking`'s
// `Array.isArray(findings)` guard would otherwise silently see a non-array and
// report no blocking findings at all, permanently defeating the plan gate's
// escalation path.
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
  let reviewResult = (await d.review(planDoc)) || {};
  let findings = reviewResult.survivors || [];
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
    reviewResult = (await d.review(planDoc)) || {};
    findings = reviewResult.survivors || [];
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
// Returns { findings, rounds, acRounds, reworkCount, reviewCount, actResult }
// where `rounds`/`acRounds` are the per-round review findings / AC tables in
// order (always at least one entry each).
//
// `d.review()` is a `runReview` from the canonical review source and therefore
// resolves `{ survivors, acTable }`, not a bare array — every round destructures
// it. The rework-loop continuation checks BOTH `hasBlocking` and
// `acTableHasGap`: an AC-only gap (no blocking finding at all) must still
// consume the rework budget instead of exiting after round 1 and reporting
// `rework` without ever attempting a fix.
//
// No null guard is needed here: `implement` returns no document the pipeline
// consumes, and the review pipeline already converts an all-null finder sweep
// into a blocking finding.
//
// Act step: once the loop settles on a CLEAN final round (no blocking finding,
// no AC-table gap) with non-empty survivors, the optional `d.act` dep is
// invoked exactly once to incorporate them by size (small → fixed inline,
// large → filed as a task — see buildCodeActPrompt). This never runs on a
// still-blocking/AC-gapped round, whatever caused it (still-blocking findings
// and unresolved AC gaps are handled by the rework/status machinery, not this
// step — "never fix large changes inline" stays intact). A missing `act` dep or
// a thrown Act call is swallowed: concern/suggestion findings are non-gating by
// the module's own severity contract, so a failed fix-attempt must never
// change the outcome.
//
// Rework notes: `d.implement` is called with `null` for the first pass and
// `{ findings, acTable }` on every rework pass — NEVER a bare findings array.
// The AC table is a structured side-channel decoupled from `findings` (a FAIL
// criterion need not also appear as a finding), so without also passing
// `acTable` an AC-only-gap rework (empty `findings`) would hand the
// implementer zero information about what to fix.
async function runCodeGate(config, deps) {
  const c = config || {};
  const d = deps || {};
  const maxRework = c.maxRework != null ? c.maxRework : DEFAULT_MAX_CODE_REWORK;
  const tier = c.tier;
  await d.implement(null);
  let reviewResult = (await d.review()) || {};
  let findings = reviewResult.survivors || [];
  let acTable = reviewResult.acTable != null ? reviewResult.acTable : null;
  const rounds = [findings];
  const acRounds = [acTable];
  let reworkCount = 0;
  for (let i = 0; i < maxRework; i++) {
    if (!hasBlocking(findings, tier) && !acTableHasGap(acTable)) break;
    await d.implement({ findings: findings, acTable: acTable });
    reworkCount++;
    reviewResult = (await d.review()) || {};
    findings = reviewResult.survivors || [];
    acTable = reviewResult.acTable != null ? reviewResult.acTable : null;
    rounds.push(findings);
    acRounds.push(acTable);
  }
  let actResult = null;
  const isClean = !hasBlocking(findings, tier) && !acTableHasGap(acTable);
  if (isClean && findings.length > 0) {
    actResult = d.act ? await d.act(findings).catch(() => null) : null;
  }
  return {
    findings: findings,
    rounds: rounds,
    acRounds: acRounds,
    reworkCount: reworkCount,
    reviewCount: rounds.length,
    actResult: actResult,
  };
}

// JSON Schema the code-lane Act step is forced to satisfy: one disposition per
// surviving finding it was asked to incorporate. Mirrors the STAMP_ACK_SCHEMA
// pattern (a small, verifiable acknowledgement) rather than free text.
const CODE_ACT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['handled'],
  properties: {
    handled: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['id', 'action'],
        properties: {
          id: { type: 'string', minLength: 1 },
          action: { type: 'string', enum: ['fixed-inline', 'filed-as-task'] },
          taskSlug: { type: 'string' },
        },
      },
    },
  },
};

// buildCodeActPrompt(kind, roadmapOrTask, ident, worktreeRef, survivors) — the
// code-lane Act step: an already-verified surviving finding is incorporated by
// SIZE, not severity (severity already decided the outcome — this decides
// whether/how the finding is acted on). Modeled directly on
// lib/plan-review.mjs's buildActPrompt, but code-review findings are fixed
// inline in the worktree (no whole-document authoritative-body rewrite) and
// large ones are filed with `rdm task create`, not a plan-doc note.
function buildCodeActPrompt(kind, roadmapOrTask, ident, worktreeRef, survivors) {
  const target = kind === 'task' ? 'task/' + ident : roadmapOrTask + '/' + ident;
  return [
    'You are acting on ALREADY-VERIFIED code-review findings for ' + target + ' (worktree: ' + worktreeRef + ').',
    'These findings survived refutation and are non-gating (the reviewed outcome is already decided).',
    JSON.stringify(survivors, null, 2),
    'For EACH finding, decide SMALL vs LARGE:',
    '- SMALL — localized, low-risk, no new acceptance criterion (a typo, a missing doc comment, a tightened ' +
      'error message, an extra test). Fix it directly in the worktree at ' + worktreeRef +
      ' and re-run the relevant tests. Do not create a separate landing commit — the fix folds into the ' +
      'eventual land-time commit.',
    '- LARGE — new modules, cross-cutting changes, or anything that would warrant its own acceptance ' +
      'criterion. Do NOT edit code for these: file it with `./target/debug/rdm task create <slug> --title ' +
      '"Code review finding: <desc>" --body "<details>" --tags code-review --no-edit --project rdm`.',
    'Return JSON matching the CODE_ACT schema: a `handled` array with ONE entry per finding you were given — ' +
      'id, action (fixed-inline|filed-as-task), and taskSlug when you filed a task.',
  ].join('\n');
}

// OUTCOME_REASON_PREFIX — which gate a non-clean outcome came out of.
// dispatch-phase's escalations originate at the PLAN gate (classifyOutcome only
// returns 'escalated' from a blocking plan finding, or from a fetch failure
// before any code exists), so they are tagged `[plan]`; an unresolved code
// rework is tagged `[code]`. This deliberately differs from the canonical
// STATUS_MAPPING.reasonPrefix (`[code]`), which describes the INTERACTIVE review
// surface, where an escalation comes out of the code gate. The tag names which
// gate escalated, not which module produced the string.
const OUTCOME_REASON_PREFIX = { escalated: '[plan]', rework: '[code]' };

// outcomePolicy(outcome, kind, summary) — the gate/completion policy owned by the
// canonical review source, projected onto the OUTCOME contract so no consumer
// has to restate the map:
//   status           — the rdm status this outcome maps to for `kind`
//                      ('phase' | 'task'), straight from statusFor().
//   writesCompletion — MAY this outcome's surface write the land-time completion
//                      directive? Expressed ONLY as a boolean, never as the
//                      directive literal: this block is stamped verbatim into
//                      workflow scripts, and the dispatch harness forbids that
//                      literal anywhere in a stamped region. The land-time writer
//                      (`rdm-land`) turns this boolean plus the OUTCOME's
//                      identifiers into the real trailer via `rdm hook done-line`.
//   reason           — a gate-tagged park/escalation note; empty on a clean review.
function outcomePolicy(outcome, kind, summary) {
  const prefix = OUTCOME_REASON_PREFIX[outcome];
  return {
    status: statusFor(outcome, kind),
    writesCompletion: writesCompletion(outcome),
    reason: prefix ? prefix + ' ' + summary : '',
  };
}

// annotateHandled(findings, actResult) — realize the guideline "for each
// finding, state how it was handled (fixed-inline / filed-as-task)" for the
// mechanical code lane: stamp a `handled` field onto each finding from the
// matching `actResult.handled` entry (by `id`), defaulting to `'unhandled'`
// when the Act step wasn't run, failed, or didn't report that specific
// finding. Pure and order-preserving; a no-op (returns `findings` unchanged)
// when `actResult` carries no usable `handled` array.
function annotateHandled(findings, actResult) {
  if (!actResult || !Array.isArray(actResult.handled)) return findings;
  const actionById = {};
  actResult.handled.forEach((h) => {
    if (h && h.id) actionById[h.id] = h.action;
  });
  return findings.map((f) => ({ ...f, handled: (f && f.id && actionById[f.id]) || 'unhandled' }));
}

// buildOutcome — the OUTCOME contract { roadmap, phase, outcome, status,
// writesCompletion, summary, reason, findings }. fetchError short-circuits to
// escalated. Never emits a land-time completion directive — it emits the
// `writesCompletion` boolean instead, and `rdm-land` writes the trailer.
function buildOutcome(input) {
  const i = input || {};
  const roadmap = i.roadmap;
  const phase = i.phase;
  const tier = i.tier;
  if (i.fetchError === true) {
    const failSummary = 'phase fetch failed';
    const failPolicy = outcomePolicy('escalated', 'phase', failSummary);
    return {
      roadmap: roadmap,
      phase: phase,
      outcome: 'escalated',
      status: failPolicy.status,
      writesCompletion: failPolicy.writesCompletion,
      summary: failSummary,
      reason: failPolicy.reason,
      findings: [],
    };
  }
  const planFindings = i.planFindings || [];
  const acRounds = i.acRounds || [];
  const lastAcTable = acRounds.length ? acRounds[acRounds.length - 1] : null;
  const classifierInput = {
    planFindings: planFindings,
    codeFindings: i.codeFindings,
    codeFindingsAfterRework: i.codeFindingsAfterRework,
    codeReviews: i.codeReviews,
    maxRework: i.maxRework,
    tier: tier,
    acTable: lastAcTable,
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
    // An AC-only gap can force `rework` with an EMPTY lastRound findings
    // array (no blocking finding at all) — summarizeFindings([]) would then
    // misleadingly read "no surviving findings". Name the real cause instead.
    summary =
      lastRound.length === 0 && acTableHasGap(lastAcTable)
        ? 'code rework unresolved: unmet acceptance criteria in AC table'
        : 'code rework unresolved: ' + summarizeFindings(lastRound);
  } else {
    findings = annotateHandled(lastRound, i.actResult);
    summary = 'phase reviewed clean: ' + summarizeFindings(lastRound);
  }
  const policy = outcomePolicy(outcome, 'phase', summary);
  return {
    roadmap: roadmap,
    phase: phase,
    outcome: outcome,
    status: policy.status,
    writesCompletion: policy.writesCompletion,
    summary: summary,
    reason: policy.reason,
    findings: findings,
  };
}

// buildTaskOutcome — the task-shaped OUTCOME contract { task, outcome, status,
// writesCompletion, summary, reason, findings }. A task is keyed by slug and
// belongs to no roadmap, so it emits a `task` identifier instead of
// `roadmap`/`phase`; the decision core (classifyOutcome / hasBlocking /
// summarizeFindings / outcomePolicy) is shared UNCHANGED with the phase path.
// Tasks always dispatch at the fixed `medium` tier, so the `large`
// gate-tightening in hasBlocking never applies to them. `escalated` maps to the
// `blocked` TASK status — never downgraded to `in-progress`. fetchError
// short-circuits to escalated. Never emits a land-time completion directive.
function buildTaskOutcome(input) {
  const i = input || {};
  const task = i.task;
  const tier = i.tier;
  if (i.fetchError === true) {
    const failSummary = 'task fetch failed';
    const failPolicy = outcomePolicy('escalated', 'task', failSummary);
    return {
      task: task,
      outcome: 'escalated',
      status: failPolicy.status,
      writesCompletion: failPolicy.writesCompletion,
      summary: failSummary,
      reason: failPolicy.reason,
      findings: [],
    };
  }
  const planFindings = i.planFindings || [];
  const acRounds = i.acRounds || [];
  const lastAcTable = acRounds.length ? acRounds[acRounds.length - 1] : null;
  const classifierInput = {
    planFindings: planFindings,
    codeFindings: i.codeFindings,
    codeFindingsAfterRework: i.codeFindingsAfterRework,
    codeReviews: i.codeReviews,
    maxRework: i.maxRework,
    tier: tier,
    acTable: lastAcTable,
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
    // See buildOutcome's identical AC-only-gap note: an empty lastRound with a
    // gapped AC table must not read as "no surviving findings".
    summary =
      lastRound.length === 0 && acTableHasGap(lastAcTable)
        ? 'code rework unresolved: unmet acceptance criteria in AC table'
        : 'code rework unresolved: ' + summarizeFindings(lastRound);
  } else {
    findings = annotateHandled(lastRound, i.actResult);
    summary = 'task reviewed clean: ' + summarizeFindings(lastRound);
  }
  const policy = outcomePolicy(outcome, 'task', summary);
  return {
    task: task,
    outcome: outcome,
    status: policy.status,
    writesCompletion: policy.writesCompletion,
    summary: summary,
    reason: policy.reason,
    findings: findings,
  };
}
// >>> dispatch-outcome:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  DEFAULT_MAX_PLAN_REVISE,
  DEFAULT_MAX_CODE_REWORK,
  parseBudget,
  parseDispatchArgs,
  hoistedMetaComplete,
  runPlanGate,
  runCodeGate,
  codeReviewRounds,
  hasBlocking,
  acTableHasGap,
  summarizeFindings,
  classifyOutcome,
  statusFor,
  writesCompletion,
  deriveSignals,
  outcomePolicy,
  OUTCOME_REASON_PREFIX,
  CODE_ACT_SCHEMA,
  buildCodeActPrompt,
  annotateHandled,
  buildOutcome,
  buildTaskOutcome,
};
