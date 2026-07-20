// dispatch-phase — the keystone per-phase unit of autonomous execution.
//
// A deterministic 4-stage pipeline for a single roadmap phase:
//   Plan → PlanReview → Implement → CodeReview → OUTCOME.
// It replaces rdm-dispatch-phase's prose orchestration with a mechanical driver.
//
// Invoke with args: { roadmap: '<roadmap-slug>', phase: '<stem-or-number>' }.
// Returns the OUTCOME contract { roadmap, phase, outcome, summary, findings },
// outcome ∈ { reviewed, rework, escalated }. It NEVER emits a land-time
// completion directive — landing is a separate, later step. See
// docs/workflow-schemas.md.
//
// This script embeds TWO copied blocks, because the Workflow runtime cannot load
// helper modules at run time (docs/workflow-schemas.md § "Import spike"):
//   1. the review-refute-fix block — stamped from lib/review-refute-fix.mjs by
//      scripts/gen-workflow-review.sh; its `buildReviewPipeline(mode)` is called
//      inline for BOTH review gates (NOT via a nested sub-workflow call).
//   2. the dispatch-outcome block — copied BYTE-IDENTICAL from
//      lib/dispatch-phase.mjs; scripts/verify-workflow-dispatch.sh gates it.

export const meta = {
  name: 'dispatch-phase',
  description:
    'Deterministic 4-stage per-phase pipeline: plan → plan-review → implement → code-review → OUTCOME (reviewed|rework|escalated)',
  // Must list exactly the distinct `phase:` values the driver + the inlined
  // review-refute-fix block actually emit. Both review gates run their finders
  // under 'Find' and refuters under 'Refute' (from the stamped block), so those
  // appear here; there is no 'CodeReview' phase because no agent() call uses it.
  // verify-workflow-dispatch.sh asserts this list matches the emitted phases.
  phases: [
    { title: 'Plan' },
    { title: 'PlanReview' },
    { title: 'Implement' },
    { title: 'Find' },
    { title: 'Refute' },
  ],
}

// The block below is GENERATED from .claude/workflows/lib/review-refute-fix.mjs by
// scripts/gen-workflow-review.sh — do NOT edit it here. Edit the lib and re-run the
// generator; scripts/verify-workflow-review.sh fails the build on drift.
// >>> review-refute-fix:begin (generated into workflow consumers by scripts/gen-workflow-review.sh — edit the lib, not the copy) <<<
// Findings scoring below this confidence are dropped even if not refuted.
const CONFIDENCE_FLOOR = 70;

// Ranking key: lower sorts first. Anything unknown sorts last.
const SEVERITY_RANK = { blocking: 0, concern: 1, suggestion: 2 };

// The two dimension sets, selected by `mode`. Each finder agent reviews exactly
// one dimension; a fresh refuter then grades each finding it produced.
//   code — reviews an implementation diff (dispatch-phase's code-review stage).
//   plan — reviews a plan document (dispatch-phase's plan-review stage).
const DIMENSIONS = {
  code: [
    {
      key: 'ac',
      title: 'AC compliance',
      focus:
        'For each acceptance criterion in the target, rate PASS / FAIL / PARTIAL with evidence (file:line, test name). Flag any criterion that is unmet, ambiguous, or untestable.',
    },
    {
      key: 'correctness',
      title: 'Correctness & error handling',
      focus:
        'Logic bugs, edge cases, race conditions, and error paths. In rdm-core, errors must be hand-written matchable enums (no anyhow / type erasure); in rdm-cli / rdm-server, anyhow with .context(). User-facing CLI errors must be actionable.',
    },
    {
      key: 'tests',
      title: 'Tests',
      focus:
        'Do tests exist and cover the key behaviors and edge cases? Was TDD followed? Are there untested branches or newly added logic with no test?',
    },
    {
      key: 'architecture',
      title: 'Architecture',
      focus:
        'Does logic live in rdm-core with cli/server as thin layers? No duplicated logic across interfaces? Correct core/cli/server separation and conventional-commit scope discipline.',
    },
  ],
  plan: [
    {
      key: 'coherence',
      title: 'Coherence',
      focus:
        'Internal consistency and completeness: are the steps and acceptance criteria concrete and actionable? An empty or ambiguous plan is itself a blocking finding — never guess intent. A plan step citing a file or behavior as existing, where it was actually introduced by another in-flight (not-yet-landed) roadmap or task, is only blocking if the target item does NOT carry the `depends-unlanded` tag and does not state the dependency explicitly; when already annotated, downgrade to a concern (or omit) instead of blocking on it.',
    },
    {
      key: 'architectural-fit',
      title: 'Architectural fit',
      focus:
        "Read the project's principles (CLAUDE.md / AGENTS.md if no principles note is configured). Flag any plan step that would violate a stated convention or constraint — a violated constraint is what makes a finding blocking; stylistic preferences alone are not.",
    },
    {
      key: 'unit-of-work',
      title: 'Unit of work',
      focus:
        'Is the phase independently deliverable and testable — neither too large to land safely nor too trivial to warrant its own phase?',
    },
  ],
};

// Plan-stage severity contract: what makes a plan-stage finding `blocking`
// versus a `concern` that rides along as an implementation note. Of the six
// findings that drove an observed three-round plan-review escalation, five
// were implementation-level defects in proposed pseudo-code/shell that should
// have been notes under correct calibration, while the sixth was a genuine
// architectural violation that must still block. This line is injected into
// every plan-mode finder prompt only — code-mode prompts are unaffected.
const PLAN_SEVERITY_CALIBRATION =
  'Plan-stage severity contract: `blocking` means the goal, approach, or scope is wrong, or the plan violates a stated architectural constraint. A defect in a specific proposed line of code or shell (e.g. an off-by-one in proposed pseudo-code) is a `concern` that rides along as an implementation note for the implementing agent — not a gate. An empty or ambiguous plan is still `blocking` (see the coherence dimension).';

// Prompt for a finder agent reviewing a single dimension of `mode`.
function findPrompt(mode, dim, context) {
  const target = (context && context.target) || '(the target described in your working directory)';
  const diffHint =
    mode === 'code'
      ? 'Inspect the implementation diff (use git log / git diff in the worktree).'
      : 'Inspect the plan document text.';
  const lines = [
    'You are a READ-ONLY reviewer. Do not edit any files.',
    'Review target: ' + target + '.',
    diffHint,
    'Your single dimension is ' + dim.title + ' (' + dim.key + '). ' + dim.focus,
  ];
  if (mode === 'plan') {
    lines.push(PLAN_SEVERITY_CALIBRATION);
  }
  lines.push(
    'Report only findings you can back with concrete evidence. One strong finding beats five weak ones.',
    'Return JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.',
    'Return an empty `findings` array if the dimension is clean.'
  );
  return lines.join('\n');
}

// Prompt for a refuter agent grading ONE finding. A fresh refuter per finding —
// the finder never grades its own work. The refuter's default stance is that the
// finding is NOT real unless the code/plan proves it.
function refutePrompt(mode, dim, finding, context) {
  const target = (context && context.target) || '(the target described in your working directory)';
  return [
    'You are a READ-ONLY refuter. Do not edit any files.',
    'A prior reviewer raised this ' + dim.key + ' finding against ' + target + ':',
    JSON.stringify(finding, null, 2),
    'Start from the stance: this is NOT a real issue unless the ' +
      (mode === 'code' ? 'code' : 'plan') +
      ' proves otherwise. Read the actual cited location and its surrounding context before deciding.',
    'Return JSON matching the VERDICT schema: refuted (boolean — true if the finding does not hold up), confidence (0-100 in your verdict), and rationale.',
  ].join('\n');
}

// JSON Schema a finder agent is forced to satisfy (see docs/workflow-schemas.md § FINDING).
const FINDINGS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['findings'],
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['id', 'concern', 'severity', 'confidence', 'what_fails'],
        properties: {
          id: { type: 'string', minLength: 1 },
          concern: { type: 'string' },
          location: { type: 'string' },
          severity: { type: 'string', enum: ['blocking', 'concern', 'suggestion'] },
          confidence: { type: 'integer', minimum: 0, maximum: 100 },
          what_fails: { type: 'string' },
          why: { type: 'string' },
          recommendation: { type: 'string' },
        },
      },
    },
  },
};

// JSON Schema a refuter agent is forced to satisfy (see docs/workflow-schemas.md § VERDICT).
const VERDICT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['refuted', 'confidence'],
  properties: {
    refuted: { type: 'boolean' },
    confidence: { type: 'integer', minimum: 0, maximum: 100 },
    rationale: { type: 'string' },
  },
};

// Pure: does a finding survive its refutation and the confidence floor?
// A finding is dropped if a refuter refuted it OR its confidence is below the floor.
function survives(finding, verdict) {
  if (verdict && verdict.refuted) return false;
  const confidence = finding && finding.confidence != null ? finding.confidence : 0;
  if (confidence < CONFIDENCE_FLOOR) return false;
  return true;
}

// Pure, deterministic ranking (no Date.now / Math.random): most severe first,
// then highest confidence, then id as a stable tiebreaker.
function rankFindings(findings) {
  return findings.slice().sort((a, b) => {
    const sa = SEVERITY_RANK[a.severity] != null ? SEVERITY_RANK[a.severity] : 99;
    const sb = SEVERITY_RANK[b.severity] != null ? SEVERITY_RANK[b.severity] : 99;
    if (sa !== sb) return sa - sb;
    const ca = a.confidence != null ? a.confidence : 0;
    const cb = b.confidence != null ? b.confidence : 0;
    if (ca !== cb) return cb - ca;
    return String(a.id).localeCompare(String(b.id));
  });
}

// Build the review-refute-fix pipeline for `mode` ("code" | "plan").
//
// Returns an async `runReview(context)` that:
//   1. runs one finder agent per dimension IN PARALLEL (pipeline stage 1),
//   2. runs a FRESH refuter agent per finding, in parallel (pipeline stage 2),
//   3. drops any finding that was refuted or scored below CONFIDENCE_FLOOR,
//   4. returns the survivors ranked most-severe-first.
//
// `deps` lets the verify harness inject fakes; in the Workflow runtime it is
// omitted and the ambient `agent` / `pipeline` / `parallel` / `log` globals are
// used. `typeof x !== 'undefined'` is a ReferenceError-safe global probe.
function buildReviewPipeline(mode, deps) {
  deps = deps || {};
  const _agent = deps.agent || (typeof agent !== 'undefined' ? agent : undefined);
  const _pipeline = deps.pipeline || (typeof pipeline !== 'undefined' ? pipeline : undefined);
  const _parallel = deps.parallel || (typeof parallel !== 'undefined' ? parallel : undefined);
  const _log = deps.log || (typeof log !== 'undefined' ? log : function () {});
  const dims = DIMENSIONS[mode];
  if (!dims) throw new Error('unknown review mode: ' + mode + ' (expected "code" or "plan")');
  if (!_agent || !_pipeline || !_parallel) {
    throw new Error('review-refute-fix: missing agent/pipeline/parallel (pass deps outside the Workflow runtime)');
  }

  return async function runReview(context) {
    const ctx = context || {};
    // Optional explicit models for the two review steps. Callers that have no
    // tier context (the standalone review-refute-fix consumer) simply omit them
    // and the agents inherit the session model exactly as before. Passing
    // `model: undefined` is INERT — verified by the agent() model spike recorded
    // in docs/workflow-schemas.md § "agent() options" — so always-assigning the
    // key is safe and needs no conditional-assignment helper.
    const findModel = ctx.findModel;
    const verifyModel = ctx.verifyModel;
    // Stage 1: parallel dimension finders. Stage 2: a fresh refuter per finding.
    // pipeline() keeps each dimension's find→refute chain independent (no barrier).
    const perDimension = await _pipeline(
      dims,
      (dim) =>
        _agent(findPrompt(mode, dim, ctx), {
          label: 'find:' + mode + ':' + dim.key,
          phase: 'Find',
          schema: FINDINGS_SCHEMA,
          model: findModel,
        }).then((found) => {
          // An UNKNOWN model id makes agent() RESOLVE to null rather than throw
          // (spike consequence 3). A resolved null would sail through stage 2 as
          // `(null && …) || []` → [], i.e. a silently clean review. Convert it to
          // a thrown stage here — the only thing the runtime's pipeline turns
          // into a null element — so the all-null check below can actually fire.
          if (findModel && (found === null || found === undefined)) {
            throw new Error(
              'review-refute-fix: finder for dimension "' + dim.key + '" returned null with model "' +
                findModel + '" — an unknown/unavailable model id yields null instead of throwing'
            );
          }
          return found;
        }),
      (found, dim) =>
        _parallel(
          ((found && found.findings) || []).map((f, idx) => () =>
            _agent(refutePrompt(mode, dim, f, ctx), {
              // Unique per finding even if a finder emits an empty/duplicate id,
              // so a colliding label can never misattribute a verdict.
              label: 'refute:' + mode + ':' + (f.id || dim.key + ':' + idx),
              phase: 'Refute',
              schema: VERDICT_SCHEMA,
              model: verifyModel,
            })
              .then((verdict) => ({ finding: { ...f, concern: f.concern || dim.key }, verdict }))
              // A refuter CRASH is not proof of refutation. Keep the finding as
              // un-refuted (verdict=null ⇒ survives() retains it if confidence ≥
              // floor) instead of silently dropping it as if it were refuted.
              .catch(() => ({ finding: { ...f, concern: f.concern || dim.key }, verdict: null }))
          )
        )
    );

    // Loud failure on a wholesale model misconfiguration. One dimension dropping
    // to null is tolerated resilience (a single finder crashed); EVERY dimension
    // dropping to null while an explicit model was in play means no review
    // actually ran — e.g. an `[models]` binding this runtime does not know. That
    // must not be reported as a clean review.
    if (findModel && dims.length > 0 && perDimension.every((d) => d === null || d === undefined)) {
      throw new Error(
        'review-refute-fix: every ' + mode + ' dimension finder failed with model "' + findModel +
          '" — refusing to report a clean review; check the [models] tier bindings'
      );
    }

    // Flatten per-dimension → per-finding. A finder whose whole dimension errored
    // is dropped to null by the runtime's pipeline (a thrown stage → null); those
    // nulls are filtered here. A refuter error instead surfaces as verdict=null
    // (see the .catch above) and is kept, not dropped.
    const graded = perDimension.filter(Boolean).flat().filter(Boolean);
    const refuterErrors = graded.filter((g) => g.verdict === null).length;
    const survivors = graded.filter((g) => survives(g.finding, g.verdict)).map((g) => g.finding);
    _log(
      mode +
        ' review: ' +
        survivors.length +
        '/' +
        graded.length +
        ' finding(s) survived refutation' +
        (refuterErrors ? ' (' + refuterErrors + ' kept un-refuted after a refuter error)' : '')
    );
    return rankFindings(survivors);
  };
}
// >>> review-refute-fix:end <<<

// The block below is copied BYTE-IDENTICAL from
// .claude/workflows/lib/dispatch-phase.mjs — do NOT edit it here. Edit the lib and
// scripts/verify-workflow-dispatch.sh fails the build on drift.
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

// --- Schemas (dispatch-specific; see docs/workflow-schemas.md) ----------------

// PHASE_META — what the Stage-0 fetch agent returns from `rdm phase show`.
const PHASE_META_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['roadmap', 'phase', 'stem', 'model', 'body', 'models'],
  properties: {
    roadmap: { type: 'string' },
    phase: { type: 'string' },
    stem: { type: 'string' },
    model: { type: 'string' }, // the tier: small | medium | large
    body: { type: 'string' },
    models: {
      type: 'object',
      additionalProperties: false,
      required: ['plan', 'implement', 'review_find', 'review_verify'],
      properties: {
        plan: { type: 'string' },
        implement: { type: 'string' },
        review_find: { type: 'string' },
        review_verify: { type: 'string' },
      },
    },
  },
}

// TASK_META — the task-mode twin of PHASE_META. A task has no roadmap and no
// difficulty/model tier, so neither field appears here.
const TASK_META_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['task', 'body', 'models'],
  properties: {
    task: { type: 'string' },
    body: { type: 'string' },
    models: {
      type: 'object',
      additionalProperties: false,
      required: ['plan', 'implement', 'review_find', 'review_verify'],
      properties: {
        plan: { type: 'string' },
        implement: { type: 'string' },
        review_find: { type: 'string' },
        review_verify: { type: 'string' },
      },
    },
  },
}

// PLAN_DOC — the plan document the planner agent produces from ONLY the phase body.
const PLAN_DOC_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['steps_per_ac', 'file_map', 'tests_per_ac', 'edge_cases', 'cross_phase_deps', 'summary'],
  properties: {
    steps_per_ac: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['ac', 'steps'],
        properties: { ac: { type: 'string' }, steps: { type: 'array', items: { type: 'string' } } },
      },
    },
    file_map: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['path', 'change'],
        properties: { path: { type: 'string' }, change: { type: 'string' } },
      },
    },
    tests_per_ac: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['ac', 'test'],
        properties: { ac: { type: 'string' }, test: { type: 'string' } },
      },
    },
    edge_cases: { type: 'array', items: { type: 'string' } },
    cross_phase_deps: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string' },
  },
}

// --- Prompt builders ----------------------------------------------------------

// Stage 0: a mechanical Bash agent reads the phase JSON (the runtime cannot shell
// out itself). Sized to the small/mechanical tier.
function buildFetchPrompt(roadmap, phase) {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm phase show --roadmap ' + roadmap + ' ' + phase + ' --project rdm --format json',
    'Return a PHASE_META object: roadmap (the roadmap slug), phase (the stem-or-number you were given),',
    'stem (the phase JSON `stem`), model (the phase JSON `model` tier: small|medium|large),',
    'and body (the phase JSON `body` verbatim). If the command fails or the body is empty, return an empty body.',
    'Then resolve the models for this dispatch. Let T be the phase JSON `model` field.',
    'If T is a non-empty string, run these two WITH the tier hint:',
    '  ./target/debug/rdm model resolve plan --tier T',
    '  ./target/debug/rdm model resolve implement --tier T',
    'If T is empty or missing, run the same two with NO --tier argument.',
    'ALWAYS run these two with NO --tier argument, whatever T is:',
    '  ./target/debug/rdm model resolve review-find',
    '  ./target/debug/rdm model resolve review-verify',
    'Return the four resulting model ids verbatim in a `models` object with keys',
    'plan, implement, review_find, review_verify. Do not invent ids; if a command fails, return an empty body.',
  ].join('\n')
}

function buildTaskFetchPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm task show ' + slug + ' --project rdm --format json',
    'Return a TASK_META object: task (the slug you were given) and body (the task JSON `body` verbatim).',
    'If the command fails or the body is empty, return an empty body.',
    'Then resolve the models for this dispatch. A task carries NO tier, so run all four',
    'resolver commands with NO --tier argument:',
    '  ./target/debug/rdm model resolve plan',
    '  ./target/debug/rdm model resolve implement',
    '  ./target/debug/rdm model resolve review-find',
    '  ./target/debug/rdm model resolve review-verify',
    'Return the four resulting model ids verbatim in a `models` object with keys',
    'plan, implement, review_find, review_verify. Do not invent ids; if a command fails, return an empty body.',
  ].join('\n')
}

// Stage A: the planner is seeded with ONLY the phase body — no worktree, no code.
function buildPlanPrompt(phaseBody) {
  return [
    'You are a planning agent. Produce an implementation PLAN only — write NO code and touch NO files.',
    'You are given ONLY the phase body below; plan strictly from it.',
    '--- PHASE BODY ---',
    phaseBody,
    '--- END PHASE BODY ---',
    'Return a PLAN_DOC: steps_per_ac (steps for each acceptance criterion), file_map (path + change per file),',
    'tests_per_ac (a test per acceptance criterion), edge_cases, cross_phase_deps, and a one-paragraph summary.',
    'Be concrete and actionable — a vague or empty plan will be rejected at plan-review.',
  ].join('\n')
}

// Stage B revise: the planner revises its own plan against the ranked plan
// findings. One bounded round only.
function buildPlanRevisePrompt(phaseBody, planDocText, rankedPlanFindings) {
  return [
    'You are a planning agent revising an earlier PLAN. Write NO code and touch NO files.',
    'Phase body (authoritative source):',
    '--- PHASE BODY ---',
    phaseBody,
    '--- END PHASE BODY ---',
    'Your previous plan:',
    planDocText,
    'Plan-review raised these ranked findings — address every blocking one:',
    JSON.stringify(rankedPlanFindings, null, 2),
    'Return a corrected PLAN_DOC in the same schema.',
  ].join('\n')
}

// Stage C / D-rework: a FRESH implementer seeded ONLY with the phase body + the
// approved plan doc (+ optional code-review findings on the rework pass). It is
// NEVER given the planner's or plan-reviewer's context/transcript. `reworkNotes`
// carries CODE-review findings only — never plan-review findings.
function buildImplementPrompt(worktreeRef, phaseBody, planDocText, reworkNotes) {
  const lines = [
    'You are an implementation agent. You are seeded with ONLY the item body and the approved plan below.',
    'First, create/enter the worktree for this item and work THERE:',
    '  ./target/debug/rdm worktree add ' + worktreeRef + ' --project rdm',
    'then `cd` into the path it prints. Do all edits and the commit in that worktree.',
    '--- PHASE BODY ---',
    phaseBody,
    '--- END PHASE BODY ---',
    '--- APPROVED PLAN ---',
    planDocText,
    '--- END APPROVED PLAN ---',
    'Implement the approved plan, run the project checks, then stage and commit with a conventional-commit message.',
    'Do NOT add any land-time completion directive to the commit message — landing happens later, not here.',
    'If you discover side-work and file it as a task (per "Discovering bugs or side-work" in CLAUDE.md): you are working in the ' +
      worktreeRef +
      ' worktree, not main. If the side-task body cites a file or behavior introduced by this worktree\'s not-yet-landed work, tag it `depends-unlanded` and phrase the body as "<file/behavior>, introduced by ' +
      worktreeRef +
      ', not yet on main" — e.g. `./target/debug/rdm task create sweep-x --title "..." --body "rdm-core/src/ops/tag.rs, introduced by ' +
      worktreeRef +
      ', not yet on main. ..." --tags depends-unlanded --no-edit --project rdm`.',
  ]
  if (reworkNotes) {
    lines.push('Code-review found these ranked issues on the prior pass — fix every blocking one:')
    lines.push(JSON.stringify(reworkNotes, null, 2))
  }
  return lines.join('\n')
}

// Render a PLAN_DOC object to deterministic text for review + implementer seeding.
function renderPlanDoc(planDoc) {
  return JSON.stringify(planDoc, null, 2)
}

// --- Driver -------------------------------------------------------------------
// Args are coerced (a stringified payload is JSON.parsed once) and validated by
// parseDispatchArgs, from the copied block above — including both retry budgets,
// so an invalid budget throws before a single agent() call burns tokens.
const dispatchArgs = parseDispatchArgs(args)
const roadmap = dispatchArgs.roadmap
const phaseArg = dispatchArgs.phase
// Task mode: `{ task: <slug> }` dispatches a standalone task instead of a phase.
// A task belongs to no roadmap, carries no difficulty/model tier, and lives in
// its own `task/<slug>` worktree — see the deltas handled below.
const taskSlug = dispatchArgs.task
const isTask = !!taskSlug
const planOnly = dispatchArgs.planOnly
// The two in-run retry budgets, counted INDEPENDENTLY (each feeds exactly one
// gate). Defaults DEFAULT_MAX_PLAN_REVISE / DEFAULT_MAX_CODE_REWORK; overridable
// per run via the maxPlanRevise / maxCodeRework args.
const maxPlanRevise = dispatchArgs.maxPlanRevise
const maxCodeRework = dispatchArgs.maxCodeRework

// itemOutcome — emit the identifier-correct OUTCOME for whichever mode is
// active. Keeps every downstream return site mode-agnostic.
function itemOutcome(fields) {
  const f = fields || {}
  if (isTask) {
    return buildTaskOutcome({
      task: taskSlug,
      fetchError: f.fetchError,
      planFindings: f.planFindings,
      codeReviews: f.codeReviews,
      maxRework: f.maxRework,
      tier: f.tier,
    })
  }
  return buildOutcome({
    roadmap: roadmap,
    phase: phaseArg,
    fetchError: f.fetchError,
    planFindings: f.planFindings,
    codeReviews: f.codeReviews,
    maxRework: f.maxRework,
    tier: f.tier,
  })
}

// A pre-fetch label for logs emitted BEFORE the fetch resolves the stem. Only
// the fetch-failure log can use this; every later log uses the resolved
// `itemLabel` below, which matches the pre-dual-mode behaviour.
const itemLabelRaw = isTask ? 'task/' + taskSlug : roadmap + '/' + phaseArg

// Stage 0: fetch the phase/task metadata + body via a mechanical Bash agent.
// NOTE: this local is `phaseMeta`, NOT `meta` — the top-level `export const meta`
// (the workflow contract) already owns that identifier in this module scope.
let phaseMeta = null
try {
  phaseMeta = isTask
    ? await agent(buildTaskFetchPrompt(taskSlug), {
        label: 'fetch:task-meta',
        phase: 'Plan',
        schema: TASK_META_SCHEMA,
      })
    : await agent(buildFetchPrompt(roadmap, phaseArg), {
        label: 'fetch:phase-meta',
        phase: 'Plan',
        schema: PHASE_META_SCHEMA,
      })
} catch (e) {
  phaseMeta = null
}

if (!phaseMeta || !phaseMeta.body || String(phaseMeta.body).trim() === '') {
  log('dispatch-phase: ' + (isTask ? 'task' : 'phase') + ' fetch failed for ' + itemLabelRaw)
  return itemOutcome({ fetchError: true })
}

const phaseBody = String(phaseMeta.body)
const stem = isTask ? taskSlug : phaseMeta.stem || phaseArg
const roadmapSlug = phaseMeta.roadmap || roadmap
// Tasks carry no difficulty/model, so they always dispatch at the fixed
// `medium` tier — the `large` gate-tightening never applies to a task.
const tier = isTask ? 'medium' : phaseMeta.model || 'medium'
// Stage C works in the per-task worktree for tasks, the shared per-roadmap
// worktree for phases.
const worktreeRef = isTask ? 'task/' + taskSlug : roadmapSlug
// Explicitly resolved models for this dispatch, from the single Stage-0 batch.
// An incomplete map means the resolver did not run: fail loudly rather than
// dispatching every agent on the inherited session model, which is the silent
// no-op this whole change exists to remove.
const models = phaseMeta.models || {}
// Expressed with .filter() rather than a `for`/`while` on purpose: the driver
// region carries NO `while` at all and only allowlisted `for` headers (gated by
// verify-workflow-dispatch.sh). The two budget-bounded retry loops live in the
// copied dispatch-outcome block, where the Node harness can drive them.
const unresolvedStep = ['plan', 'implement', 'review_find', 'review_verify'].filter(
  (k) => typeof models[k] !== 'string' || models[k] === ''
)[0]
if (unresolvedStep) {
  log('dispatch-phase: unresolved model for step "' + unresolvedStep + '" on ' + itemLabelRaw)
  return itemOutcome({ fetchError: true })
}
const reviewModels = { findModel: models.review_find, verifyModel: models.review_verify }
// Resolved log label: phase mode logs the resolved `stem`, not the raw
// stem-or-number the caller passed, matching the pre-dual-mode behaviour.
const itemLabel = isTask ? 'task/' + taskSlug : roadmap + '/' + stem

// Stages A + B: author the plan from ONLY the phase body, review it via the
// stamped shared pipeline, and revise it up to the plan-revise budget. The loop
// itself lives in runPlanGate (copied block) so it is driveable from Node; this
// driver only supplies the side effects.
const runPlanReview = buildReviewPipeline('plan')
const planGate = await runPlanGate(
  { maxRevise: maxPlanRevise, tier: tier },
  {
    plan: async () =>
      agent(buildPlanPrompt(phaseBody), {
        label: 'plan:author',
        phase: 'Plan',
        schema: PLAN_DOC_SCHEMA,
        model: models.plan,
      }),
    revise: async (doc, findings) =>
      agent(buildPlanRevisePrompt(phaseBody, renderPlanDoc(doc), findings), {
        label: 'plan:revise',
        phase: 'PlanReview',
        schema: PLAN_DOC_SCHEMA,
        model: models.plan,
      }),
    review: async (doc) => runPlanReview({ target: renderPlanDoc(doc), ...reviewModels }),
  }
)

// agent() RESOLVES to null on an unknown/unavailable model id rather than
// throwing (spike consequence 3). runPlanGate guards BOTH the initial plan and
// every revise result and reports which stage produced the null, so the failure
// is diagnosable instead of silently escalating.
if (planGate.fetchError === true) {
  const nullStage = planGate.stage === 'revise' ? 'plan revise' : 'plan'
  log('dispatch-phase: ' + nullStage + ' agent returned null on ' + itemLabelRaw + ' (model: ' + models.plan + ')')
  return itemOutcome({ fetchError: true })
}

const planDoc = planGate.planDoc
const planFindings = planGate.findings

// Plan gate: never implement on a blocking plan.
if (hasBlocking(planFindings, tier)) {
  log('dispatch-phase: plan gate escalated for ' + itemLabel)
  return itemOutcome({ planFindings: planFindings, tier: tier })
}

// --plan-only: the plan gate passed — stop before implementing and report the
// vetted plan as `reviewed` (autopilot's estimate/plan-vet pass). This early
// return is NOT part of the copied dispatch-outcome block, so it must carry the
// identifier for the active mode itself (task-keyed vs roadmap/phase-keyed).
if (planOnly) {
  const o = isTask
    ? { task: taskSlug, outcome: 'reviewed', summary: 'plan-only: plan gate passed', findings: planFindings }
    : { roadmap: roadmap, phase: phaseArg, outcome: 'reviewed', summary: 'plan-only: plan gate passed', findings: planFindings }
  log('dispatch-phase (' + itemLabel + '): plan-only — plan approved')
  return o
}

// Stages C + D: implement in the shared per-roadmap worktree (a FRESH
// implementer seeded with ONLY the phase body + approved plan doc — not the
// planner context), code-review via the same stamped pipeline, and rework up to
// the code-rework budget. As with the plan gate, the loop lives in runCodeGate.
const approvedPlanText = renderPlanDoc(planDoc)
const runCodeReview = buildReviewPipeline('code')
const reviewTarget = isTask ? 'task/' + taskSlug : roadmapSlug + '/' + stem
const codeGate = await runCodeGate(
  { maxRework: maxCodeRework, tier: tier },
  {
    implement: async (notes) =>
      notes == null
        ? agent(buildImplementPrompt(worktreeRef, phaseBody, approvedPlanText), {
            model: models.implement,
            label: 'implement:worktree',
            phase: 'Implement',
          })
        : agent(buildImplementPrompt(worktreeRef, phaseBody, approvedPlanText, notes), {
            model: models.implement,
            label: 'implement:rework',
            phase: 'Implement',
          }),
    review: async () => runCodeReview({ target: reviewTarget, ...reviewModels }),
  }
)

const outcome = itemOutcome({
  planFindings: planFindings,
  codeReviews: codeGate.rounds,
  maxRework: maxCodeRework,
  tier: tier,
})
log('dispatch-phase (' + itemLabel + '): ' + outcome.outcome + ' — ' + outcome.summary)
return outcome
