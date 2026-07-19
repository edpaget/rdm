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
        'Internal consistency and completeness: are the steps and acceptance criteria concrete and actionable? An empty or ambiguous plan is itself a blocking finding — never guess intent.',
    },
    {
      key: 'architectural-fit',
      title: 'Architectural fit',
      focus:
        "Read the project's principles (CLAUDE.md / AGENTS.md if no principles note is configured). Flag any plan step that would violate a stated convention or constraint.",
    },
    {
      key: 'unit-of-work',
      title: 'Unit of work',
      focus:
        'Is the phase independently deliverable and testable — neither too large to land safely nor too trivial to warrant its own phase?',
    },
  ],
};

// Prompt for a finder agent reviewing a single dimension of `mode`.
function findPrompt(mode, dim, context) {
  const target = (context && context.target) || '(the target described in your working directory)';
  const diffHint =
    mode === 'code'
      ? 'Inspect the implementation diff (use git log / git diff in the worktree).'
      : 'Inspect the plan document text.';
  return [
    'You are a READ-ONLY reviewer. Do not edit any files.',
    'Review target: ' + target + '.',
    diffHint,
    'Your single dimension is ' + dim.title + ' (' + dim.key + '). ' + dim.focus,
    'Report only findings you can back with concrete evidence. One strong finding beats five weak ones.',
    'Return JSON matching the FINDINGS schema: a `findings` array, each with id, concern, location, severity (blocking|concern|suggestion), confidence (0-100), what_fails, why, recommendation.',
    'Return an empty `findings` array if the dimension is clean.',
  ].join('\n');
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
    // Stage 1: parallel dimension finders. Stage 2: a fresh refuter per finding.
    // pipeline() keeps each dimension's find→refute chain independent (no barrier).
    const perDimension = await _pipeline(
      dims,
      (dim) =>
        _agent(findPrompt(mode, dim, ctx), {
          label: 'find:' + mode + ':' + dim.key,
          phase: 'Find',
          schema: FINDINGS_SCHEMA,
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
            })
              .then((verdict) => ({ finding: { ...f, concern: f.concern || dim.key }, verdict }))
              // A refuter CRASH is not proof of refutation. Keep the finding as
              // un-refuted (verdict=null ⇒ survives() retains it if confidence ≥
              // floor) instead of silently dropping it as if it were refuted.
              .catch(() => ({ finding: { ...f, concern: f.concern || dim.key }, verdict: null }))
          )
        )
    );

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

// --- Schemas (dispatch-specific; see docs/workflow-schemas.md) ----------------

// PHASE_META — what the Stage-0 fetch agent returns from `rdm phase show`.
const PHASE_META_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['roadmap', 'phase', 'stem', 'model', 'body'],
  properties: {
    roadmap: { type: 'string' },
    phase: { type: 'string' },
    stem: { type: 'string' },
    model: { type: 'string' }, // the tier: small | medium | large
    body: { type: 'string' },
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
function buildImplementPrompt(roadmapSlug, phaseBody, planDocText, reworkNotes) {
  const lines = [
    'You are an implementation agent. You are seeded with ONLY the phase body and the approved plan below.',
    'First, create/enter the shared per-roadmap worktree and work THERE:',
    '  ./target/debug/rdm worktree add ' + roadmapSlug + ' --project rdm',
    'then `cd` into the path it prints. Do all edits and the commit in that worktree.',
    '--- PHASE BODY ---',
    phaseBody,
    '--- END PHASE BODY ---',
    '--- APPROVED PLAN ---',
    planDocText,
    '--- END APPROVED PLAN ---',
    'Implement the approved plan, run the project checks, then stage and commit with a conventional-commit message.',
    'Do NOT add any land-time completion directive to the commit message — landing happens later, not here.',
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
// The Workflow tool contract forbids stringified args, but LLM callers (rdm-do
// --auto and hand-run single phases) invoke dispatch-phase DIRECTLY and have
// delivered a JSON string; coerce once, then derive every field from it.
let dispatchArgs = args || {}
if (typeof dispatchArgs === 'string') {
  try {
    dispatchArgs = JSON.parse(dispatchArgs) || {}
  } catch (e) {
    dispatchArgs = {}
  }
}
if (!dispatchArgs || typeof dispatchArgs !== 'object') dispatchArgs = {}
const roadmap = dispatchArgs.roadmap || ''
const phaseArg = dispatchArgs.phase || ''
const planOnly = !!dispatchArgs.planOnly

// Stage 0: fetch the phase metadata + body via a mechanical Bash agent.
// NOTE: this local is `phaseMeta`, NOT `meta` — the top-level `export const meta`
// (the workflow contract) already owns that identifier in this module scope.
let phaseMeta = null
try {
  phaseMeta = await agent(buildFetchPrompt(roadmap, phaseArg), {
    label: 'fetch:phase-meta',
    phase: 'Plan',
    schema: PHASE_META_SCHEMA,
  })
} catch (e) {
  phaseMeta = null
}

if (!phaseMeta || !phaseMeta.body || String(phaseMeta.body).trim() === '') {
  log('dispatch-phase: phase fetch failed for ' + roadmap + '/' + phaseArg)
  return buildOutcome({ roadmap: roadmap, phase: phaseArg, fetchError: true })
}

const phaseBody = String(phaseMeta.body)
const stem = phaseMeta.stem || phaseArg
const roadmapSlug = phaseMeta.roadmap || roadmap
const tier = phaseMeta.model || 'medium'

// Stage A: author the plan from ONLY the phase body.
let planDoc = await agent(buildPlanPrompt(phaseBody), {
  label: 'plan:author',
  phase: 'Plan',
  schema: PLAN_DOC_SCHEMA,
})

// Stage B: plan-review via the stamped shared pipeline, called inline.
const runPlanReview = buildReviewPipeline('plan')
let planFindings = await runPlanReview({ target: renderPlanDoc(planDoc) })

// Bounded to ONE revise round: revise once, then re-review once.
if (hasBlocking(planFindings, tier)) {
  planDoc = await agent(buildPlanRevisePrompt(phaseBody, renderPlanDoc(planDoc), planFindings), {
    label: 'plan:revise',
    phase: 'PlanReview',
    schema: PLAN_DOC_SCHEMA,
  })
  planFindings = await runPlanReview({ target: renderPlanDoc(planDoc) })
}

// Plan gate: never implement on a blocking plan.
if (hasBlocking(planFindings, tier)) {
  log('dispatch-phase: plan gate escalated for ' + roadmap + '/' + stem)
  return buildOutcome({ roadmap: roadmap, phase: phaseArg, planFindings: planFindings, tier: tier })
}

// --plan-only: the plan gate passed — stop before implementing and report the
// vetted plan as `reviewed` (autopilot's estimate/plan-vet pass). This early
// return is NOT part of the copied dispatch-outcome block.
if (planOnly) {
  const o = { roadmap: roadmap, phase: phaseArg, outcome: 'reviewed', summary: 'plan-only: plan gate passed', findings: planFindings }
  log('dispatch-phase (' + roadmap + '/' + stem + '): plan-only — plan approved')
  return o
}

// Stage C: implement in the shared per-roadmap worktree. A FRESH implementer
// seeded with ONLY the phase body + approved plan doc — not the planner context.
const approvedPlanText = renderPlanDoc(planDoc)
await agent(buildImplementPrompt(roadmapSlug, phaseBody, approvedPlanText), {
  label: 'implement:worktree',
  phase: 'Implement',
})

// Stage D: code-review via the same stamped pipeline, called inline.
const runCodeReview = buildReviewPipeline('code')
const reviewTarget = roadmapSlug + '/' + stem
const codeFindings = await runCodeReview({ target: reviewTarget })

// Bounded to exactly ONE rework pass.
let codeFindingsAfterRework = []
if (hasBlocking(codeFindings, tier)) {
  await agent(buildImplementPrompt(roadmapSlug, phaseBody, approvedPlanText, codeFindings), {
    label: 'implement:rework',
    phase: 'Implement',
  })
  codeFindingsAfterRework = await runCodeReview({ target: reviewTarget })
}

const outcome = buildOutcome({
  roadmap: roadmap,
  phase: phaseArg,
  planFindings: planFindings,
  codeFindings: codeFindings,
  codeFindingsAfterRework: codeFindingsAfterRework,
  tier: tier,
})
log('dispatch-phase (' + roadmap + '/' + stem + '): ' + outcome.outcome + ' — ' + outcome.summary)
return outcome
