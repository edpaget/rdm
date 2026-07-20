//! review-refute-fix — the shared review pipeline for the autonomous workflow lane.
//!
//! This is the **single source of truth** for the find → refute → filter → rank
//! pipeline. Because the Claude Code Workflow runtime cannot `import`/`require`
//! (see docs/workflow-schemas.md § "Import spike"), the marked block below is
//! stamped verbatim into every workflow-script consumer by
//! `scripts/gen-workflow-review.sh`. Edit the block HERE — never in a consumer —
//! and re-run the generator; `scripts/verify-workflow-review.sh` fails on drift.
//!
//! The block is written to run in two contexts unchanged:
//!   1. The Workflow runtime, where `agent`/`pipeline`/`parallel`/`log` are
//!      ambient globals — `buildReviewPipeline(mode)` picks them up by default.
//!   2. Node (this file, imported by the verify harness), where those globals
//!      do not exist — the harness injects fakes via the `deps` argument, so the
//!      pure composition and filtering logic is testable with zero LLM calls.
//!
//! Everything the block needs is declared INSIDE the markers (no imports), so the
//! stamped copy is self-contained. The `export { ... }` at the bottom lives
//! OUTSIDE the markers: it exists only for Node's importer and is never copied
//! into a workflow consumer (a bare `export` mid-body would break a workflow
//! script, whose only permitted export is `meta`).

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

// Node-only exports for the verify harness. NOT part of the generated block —
// the marker END is above this line, so the generator never copies these.
export {
  CONFIDENCE_FLOOR,
  SEVERITY_RANK,
  DIMENSIONS,
  findPrompt,
  refutePrompt,
  FINDINGS_SCHEMA,
  VERDICT_SCHEMA,
  survives,
  rankFindings,
  buildReviewPipeline,
};
