// dispatch-phase — the keystone unit of autonomous execution for phases and tasks.
//
// A deterministic 4-stage pipeline for a roadmap phase or standalone task:
//   Plan → PlanReview → Implement → CodeReview → OUTCOME.
// It replaces rdm-dispatch-phase's prose orchestration with a mechanical driver.
//
// Invoke with args: { roadmap: '<roadmap-slug>', phase: '<stem-or-number>' } (phase mode) or { task: '<slug>' } (task mode).
// Returns the OUTCOME contract: phase mode { roadmap, phase, outcome, status,
// writesCompletion, summary, reason, findings }; task mode { task, outcome,
// status, writesCompletion, summary, reason, findings }, outcome ∈ { reviewed, rework,
// escalated }. `status` and `writesCompletion` carry the canonical review's
// gate/completion policy so no consumer restates it. It NEVER emits a land-time
// completion directive itself — `writesCompletion: true` tells `rdm-land` to
// synthesize the trailer via `rdm hook done-line` at land time. See
// docs/workflow-schemas.md.
//
// This script embeds TWO copied blocks, because the Workflow runtime cannot load
// helper modules at run time (docs/workflow-schemas.md § "Import spike"):
//   1. the review-refute-fix block — stamped from lib/review.mjs by
//      scripts/gen-workflow-review.sh; its `buildReviewPipeline(mode)` is called
//      inline for BOTH review gates (NOT via a nested sub-workflow call).
//   2. the dispatch-outcome block — copied BYTE-IDENTICAL from
//      lib/dispatch-phase.mjs; scripts/verify-workflow-dispatch.sh gates it.

export const meta = {
  name: 'dispatch-phase',
  description:
    'Deterministic 4-stage pipeline for phases and tasks: plan → plan-review → implement → code-review → OUTCOME (reviewed|rework|escalated)',
  // Must list exactly the distinct `phase:` values the driver + the inlined
  // review-refute-fix block actually emit. Both review gates run their finders
  // under 'Find' and refuters under 'Refute' (from the stamped block), so those
  // appear here; there is no 'CodeReview' phase because no agent() call uses it.
  // 'Review' is the mechanical diff-signals agent that feeds the code gate's
  // dimension selection. 'Act' is the optional code-lane Act step that
  // incorporates surviving non-blocking findings on a clean final round.
  // verify-workflow-dispatch.sh asserts this list matches the emitted phases.
  phases: [
    { title: 'Plan' },
    { title: 'PlanReview' },
    { title: 'Implement' },
    { title: 'Review' },
    { title: 'Find' },
    { title: 'Refute' },
    { title: 'Act' },
  ],
}

// The block below is GENERATED from .claude/workflows/lib/review.mjs by
// scripts/gen-workflow-review.sh — do NOT edit it here. Edit the lib and re-run the
// generator; scripts/verify-workflow-review.sh fails the build on drift.
// >>> review-refute-fix:begin (generated into workflow consumers by scripts/gen-workflow-review.sh — edit the lib, not the copy) <<<
// >>> review-spec:begin (the `//|` lines below are rendered into the shipped review skills by scripts/gen-skill-review.sh) <<<
//| ### Dimensions — the adaptive review fleet
//|
//| Scale the fleet to what the change actually touches. **Always-on** dimensions
//| run for every review; **triggered** dimensions run only when the change hits
//| their surface. This keeps a 10-line change cheap while a cross-cutting change
//| still gets full coverage. Each dimension is reviewed by its own **read-only**
//| agent — it reviews and reports, it never edits. When in doubt about a trigger,
//| include the dimension: a spurious agent that finds nothing is cheaper than a
//| missed defect. State which dimensions you ran, and why, in the report.

// Findings scoring below this confidence are dropped even if not refuted.
//|
//| **Confidence floor.** Drop any finding whose post-refutation confidence is
//| below **70**, even when no refuter knocked it down.
const CONFIDENCE_FLOOR = 70;

// Ranking key: lower sorts first. Anything unknown sorts last.
//|
//| **Severity scale** (drives the verdict):
//|
//| - `blocking` — the work must not advance as-is: a logic error, an unmet
//|   acceptance criterion, or a mandatory process violation (e.g. a missing
//|   required changelog entry).
//| - `concern` — recorded but non-gating; it never by itself holds the work back.
//| - `suggestion` — minor optional improvement (subject to the confidence floor).
//|
//| Rank survivors most-severe first, then by confidence descending, then by id.
const SEVERITY_RANK = { blocking: 0, concern: 1, suggestion: 2 };

// The two dimension sets, selected by `mode`. Each finder agent reviews exactly
// one dimension; a fresh refuter then grades each finding it produced.
//   code — reviews an implementation diff (dispatch-phase's code-review stage).
//   plan — reviews a plan document (dispatch-phase's plan-review stage).
//
// A dimension with no `when` predicate is ALWAYS-ON. A dimension carrying
// `when(signals)` is TRIGGERED: `selectDimensions` evaluates it against both the
// change's shape AND the target's type. See selectDimensions' three-way contract
// below — omitted signals mean "unknown", and run everything.
const DIMENSIONS = {
  code: [
    //|code|
    //|code| **Code review dimensions:**
    //|code|
    //|code| - **ac** — *always.* For each acceptance criterion, rate PASS / FAIL /
    //|code|   PARTIAL with evidence (file:line, test name). Flag any criterion that is
    //|code|   unmet, ambiguous, or untestable. The per-criterion table is the contract
    //|code|   and is reported intact.
    {
      key: 'ac',
      title: 'AC compliance',
      focus:
        'For each acceptance criterion in the target, rate PASS / FAIL / PARTIAL with evidence (file:line, test name). Flag any criterion that is unmet, ambiguous, or untestable.',
    },
    //|code| - **correctness** — *always.* Logic bugs, edge cases, race conditions, and
    //|code|   error paths, judged against the project's error-handling conventions
    //|code|   (CLAUDE.md / AGENTS.md). User-facing errors must be actionable.
    {
      key: 'correctness',
      title: 'Correctness & error handling',
      focus:
        'Logic bugs, edge cases, race conditions, and error paths. In rdm-core, errors must be hand-written matchable enums (no anyhow / type erasure); in rdm-cli / rdm-server, anyhow with .context(). User-facing CLI errors must be actionable.',
    },
    //|code| - **tests** — *trigger: the diff adds or changes non-trivial logic, or adds
    //|code|   no test files.* Do tests exist and cover the key behaviors and edge
    //|code|   cases? Was a test-first discipline followed? Are there untested branches?
    {
      key: 'tests',
      title: 'Tests',
      focus:
        'Do tests exist and cover the key behaviors and edge cases? Was TDD followed? Are there untested branches or newly added logic with no test?',
      when: (s) => !!(s.changesLogic || s.missingTests),
    },
    //|code| - **architecture** — *trigger: the diff touches more than one module/layer,
    //|code|   or moves logic between layers.* Does logic live where the project's
    //|code|   architecture says it should, with thin layers on top? No duplicated logic
    //|code|   across interfaces?
    {
      key: 'architecture',
      title: 'Architecture',
      focus:
        'Does logic live in rdm-core with cli/server as thin layers? No duplicated logic across interfaces? Correct core/cli/server separation and conventional-commit scope discipline.',
      when: (s) => !!s.multiModule,
    },
    //|code| - **api-docs** — *trigger: the diff changes a public `rdm-core` item.* Are
    //|code|   public items documented per the project's conventions
    //|code|   (`#![warn(missing_docs)]`)? Are `# Errors`, `# Panics`, and `# Safety`
    //|code|   sections present where the project requires them?
    {
      key: 'api-docs',
      title: 'Public API docs',
      focus:
        'Public items in rdm-core must carry doc comments (#![warn(missing_docs)]). A function returning Result needs a `# Errors` section; one that can panic needs `# Panics`; an `unsafe fn` needs `# Safety`. Flag any public item added or changed by this diff that is missing a required section.',
      when: (s) => !!s.publicApiChanged,
    },
    //|code| - **changelog** — *trigger: the diff makes a user-facing change (CLI
    //|code|   commands, API endpoints, MCP tools, config options, observable
    //|code|   behavior).* A user-facing change MUST carry a `CHANGELOG.md` entry in the
    //|code|   same commit; a missing entry is **blocking**, per the project's
    //|code|   conventions. The entry must read from a user's perspective, not describe
    //|code|   internals.
    {
      key: 'changelog',
      title: 'Changelog',
      focus:
        "A user-facing change (CLI command, API endpoint, MCP tool, config option, or observable behavior) MUST carry a CHANGELOG.md entry under [Unreleased] in the SAME commit — a missing entry is a `blocking` finding per CLAUDE.md. The entry must describe the change from a user's perspective, not internal implementation details.",
      when: (s) => !!s.userFacing,
    },
    //|code| - **security** — *trigger: the diff touches auth, input parsing or
    //|code|   validation, path/file handling, subprocess or shell invocation, secrets
    //|code|   and credentials, deserialization, network code, or `unsafe` blocks.*
    //|code|   Injection, path traversal, secret leakage, missing authorization, and
    //|code|   unsafe-invariant violations. Every `unsafe` block needs a `// SAFETY:`
    //|code|   comment stating the invariant it upholds; an unjustified or risky
    //|code|   construct is a finding.
    {
      key: 'security',
      title: 'Security',
      focus:
        'Injection (shell/command/SQL), path traversal, secret or credential leakage into logs/errors/commits, missing or incorrect authorization, unsafe deserialization, and untrusted-input validation gaps. Every `unsafe` block must carry a `// SAFETY:` comment that states the invariant the caller upholds — an unjustified or invariant-violating `unsafe` is a finding. Judge subprocess and file-path handling against the project conventions.',
      when: (s) => !!(s.securitySurface || s.hasUnsafe),
    },
  ],
  plan: [
    //|plan|
    //|plan| **Plan review dimensions:**
    //|plan|
    //|plan| - **coherence** — *always.* Internal consistency and completeness: are
    //|plan|   the steps and acceptance criteria concrete and actionable? An empty or
    //|plan|   ambiguous plan is itself a `blocking` finding — never guess intent. A
    //|plan|   plan step citing a file or behavior as existing, where it was actually
    //|plan|   introduced by another in-flight (not-yet-landed) roadmap or task, is
    //|plan|   only `blocking` when the target item does **not** carry the
    //|plan|   `depends-unlanded` tag and does not state the dependency explicitly;
    //|plan|   when already annotated, downgrade it to a `concern` (or omit it). A
    //|plan|   plan may delegate implementation decisions to whoever carries it out —
    //|plan|   an undecided point is a `concern`, not `blocking`, unless the undecided
    //|plan|   branches would lead to different goals or outcomes. Coherence is
    //|plan|   `blocking` only when an implementer following the plan as written would
    //|plan|   build the wrong thing, never merely because they would have to make a
    //|plan|   decision themselves.
    {
      key: 'coherence',
      title: 'Coherence',
      focus:
        'Internal consistency and completeness: are the steps and acceptance criteria concrete and actionable? An empty or ambiguous plan is itself a blocking finding — never guess intent. A plan step citing a file or behavior as existing, where it was actually introduced by another in-flight (not-yet-landed) roadmap or task, is only blocking if the target item does NOT carry the `depends-unlanded` tag and does not state the dependency explicitly; when already annotated, downgrade to a concern (or omit) instead of blocking on it. A plan may delegate implementation decisions to whoever carries it out — an undecided point is a concern, not blocking, unless the undecided branches would lead to different goals or outcomes. Coherence is blocking only when an implementer following the plan as written would build the wrong thing, never merely because they would have to make a decision themselves.',
    },
    //|plan| - **architectural-fit** — *always.* Read the project's principles
    //|plan|   (falling back to `CLAUDE.md` / `AGENTS.md` in the project root when no
    //|plan|   principles note is configured — architectural fit must never go
    //|plan|   silently unchecked). Flag any plan step that would violate a stated
    //|plan|   convention or constraint: a violated constraint is what makes a finding
    //|plan|   `blocking`; stylistic preferences alone are not.
    {
      key: 'architectural-fit',
      title: 'Architectural fit',
      focus:
        "Read the project's principles (CLAUDE.md / AGENTS.md if no principles note is configured). Flag any plan step that would violate a stated convention or constraint — a violated constraint is what makes a finding blocking; stylistic preferences alone are not.",
    },
    //|plan| - **unit-of-work** — *trigger: the target is a phase.* Skipped for
    //|plan|   tasks, standalone roadmap bodies, and `--implementation-plan`; run once
    //|plan|   per phase under `--roadmap <slug>` (this can fan out to many parallel
    //|plan|   agents on a large roadmap — no hard cap is required, but be mindful of
    //|plan|   the cost). Is the phase independently deliverable and testable —
    //|plan|   neither too large to land safely nor too trivial to warrant its own
    //|plan|   phase?
    //|plan|
    //|plan| **Plan target types.** A plan review targets a `roadmap` (its own body,
    //|plan| plus every phase gated individually), a `phase`, a `task`, or an
    //|plan| `implementation-plan` — an `rdm-do` plan document handed over in context
    //|plan| ahead of implementation. `implementation-plan` has **no persisted rdm
    //|plan| item** behind it, so it is report-only: no body edit, no filed task, and
    //|plan| no gate (see § Gate).
    {
      key: 'unit-of-work',
      title: 'Unit of work',
      focus:
        'Is the phase independently deliverable and testable — neither too large to land safely nor too trivial to warrant its own phase?',
      // Target-type trigger (not diff shape): only a PHASE has a unit-of-work
      // contract to judge. Tasks, roadmaps, and bare implementation plans skip it.
      when: (s) => s.targetType === 'phase',
    },
    //|plan| - **restraint** — *always.* The counterweight to unit-of-work: flags a
    //|plan|   plan that has over-specified rather than under-specified. Two shapes
    //|plan|   are both findings — (1) the plan spells out a decision that could
    //|plan|   safely be left to whoever carries it out, and (2) the level of detail
    //|plan|   has grown past the point where adding more of it reduces risk rather
    //|plan|   than adding new surface for its own review. Symmetric with
    //|plan|   unit-of-work's two-sided framing: neither too little specification nor
    //|plan|   too much is the goal.
    {
      key: 'restraint',
      title: 'Restraint',
      focus:
        'The counterweight to unit-of-work: flags a plan that has over-specified rather than under-specified. Two shapes are both findings — (1) the plan spells out an implementation decision that could safely be left to whoever carries it out, and (2) the level of detail has grown past the point where adding more of it reduces risk rather than adding new surface for its own review. Symmetric with unit-of-work: neither too little specification nor too much is the goal.',
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
//|
//| ### Find — one read-only agent per applicable dimension, in parallel
//|
//| Each finder agent is told: you are a READ-ONLY reviewer, do not edit any
//| files; review exactly one dimension; report only findings you can back with
//| concrete evidence — **one strong finding beats five weak ones**; return an
//| empty finding list if the dimension is clean. Do not report pure
//| style/formatting nitpicks unless they violate an explicit project rule.
//|
//| Each finding is reported as:
//|
//| ```
//| - id: <short-slug>
//|code|   concern: <ac|correctness|tests|architecture|api-docs|changelog|security>
//|plan|   concern: <coherence|architectural-fit|unit-of-work>
//|code|   location: <path>:<line>
//|plan|   location: <section/heading or phase stem>
//|   severity: blocking | concern | suggestion
//|   confidence: 0-100
//|   what-fails: <the specific problem>
//|   why: <root cause / which rule or AC it violates>
//|   recommendation: <concrete fix>
//| ```
function findPrompt(mode, dim, context) {
  const target = (context && context.target) || '(the target described in your working directory)';
  const diffHint =
    mode === 'code'
      ? 'Inspect the implementation diff (use git log / git diff in the worktree).'
      : 'Inspect the plan document text.';
  // The `ac` dimension in `code` mode is the ONE dimension that returns
  // structured data (the AC_REVIEW_SCHEMA shape) instead of a bare findings
  // array — classifyOutcome consumes its `ac` table directly, never through a
  // finding. Every other dimension (including `ac` in `plan` mode, which does
  // not exist) is unaffected.
  if (mode === 'code' && dim.key === 'ac') {
    return [
      'You are a READ-ONLY reviewer. Do not edit any files.',
      'Review target: ' + target + '.',
      diffHint,
      'Your single dimension is ' + dim.title + ' (' + dim.key + '). ' + dim.focus,
      'Report only findings you can back with concrete evidence. One strong finding beats five weak ones.',
      'Return JSON matching the AC_REVIEW schema: an `ac` array with ONE entry per acceptance criterion — ' +
        'criterion, status (PASS|FAIL|PARTIAL), and evidence (file:line, test name) — plus an OPTIONAL ' +
        '`findings` array (same shape as the FINDINGS schema) for narrative notes that do not reduce to a ' +
        "single criterion's status.",
      'Only leave `ac` empty if the target states no acceptance criteria at all — report that itself as a `findings` entry.',
    ].join('\n');
  }
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
//|
//| ### Refute — a FRESH agent per finding, in parallel
//|
//| For every finding, dispatch a **separate** read-only refuter. The agent that
//| found an issue is never the agent that confirms it. The refuter starts from
//| the stance *"this is NOT a real issue unless the code proves otherwise"*,
//| reads the actual cited location and its surrounding context, and returns
//| `refuted` (boolean), a corrected `confidence` (0-100), and a rationale.
//|
//| ### Filter & consolidate
//|
//| - **Drop** any finding a refuter refuted, and any whose post-refutation
//|   confidence is below the confidence floor (70).
//| - A refuter that *crashes* is not proof of refutation — keep such a finding as
//|   un-refuted rather than silently dropping it.
//| - **Dedup** findings pointing at the same location / same root cause (the
//|   fleet covers overlapping ground by design).
//| - **Rank** survivors by severity, then confidence, then id.
//|code| - The AC table is returned as **structured data**, separate from the
//|code|   findings list — never folded into a finding. A surviving FAIL/PARTIAL
//|code|   criterion is checked directly against that table, never through finding
//|code|   severity or the refute/confidence-floor path, so the guarantee cannot be
//|code|   silently defeated by a refuter or the 70-point floor. Trade-off: this also
//|code|   means an AC-table FAIL bypasses refutation entirely — a hallucinated FAIL
//|code|   from the single `ac` finder can force a spurious rework with no
//|code|   counter-check. The AC table and any `ac`-dimension `findings` entry about
//|code|   the same criterion are two independent channels, not deduplicated against
//|code|   each other.
//|plan| - There is no acceptance-criteria pass/fail table at plan stage — the quality
//|plan|   of the plan's own acceptance criteria is judged by the **coherence**
//|plan|   dimension and surfaces as an ordinary finding.
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

//|
//| ### Verdict — one outcome vocabulary: `reviewed` | `rework` | `escalated`
//|
//| Determine the outcome in this strict order — the first matching rule wins:
//|
//| 1. **escalated** — a surviving blocker that needs a *human decision* rather
//|    than a code change: the goal, approach, or scope is wrong, the work
//|    violates a stated architectural constraint, or the acceptance criteria
//|    themselves are missing, contradictory, or unimplementable as written.
//|code| 2. **rework** — else if any surviving finding is `blocking`, or the structured
//|code|    AC table (returned by the `ac` dimension alongside its findings — see
//|code|    § Refute above) contains any FAIL or PARTIAL criterion. The AC-table check
//|code|    is direct and mechanical: it never routes through finding severity or
//|code|    refutation, so it cannot be silently defeated by a refuter or the
//|code|    confidence floor. The defect is fixable in place; the work goes back for
//|code|    another round.
//|plan| 2. **rework** — else if any surviving finding is `blocking`. The defect is
//|plan|    fixable in place; the work goes back for another round.
//| 3. **reviewed** — else. Clean, or clean after small fixes. Surviving
//|    `concern` and `suggestion` findings are recorded and do **not** gate.
//|
//| Never downgrade a surviving `blocking` finding to "reviewed with concerns" —
//| a blocker always yields `rework` or `escalated`.
//|plan|
//|plan| **Plan-stage reading of the three outcomes.**
//|plan|
//|plan| - `escalated` — the plan needs a **human product decision**: the goal,
//|plan|   approach, or scope is wrong, or it violates a stated architectural
//|plan|   constraint that cannot simply be rewritten in place.
//|plan| - `rework` — the plan document itself needs a fixable rewrite (an ambiguous
//|plan|   step, a missing prerequisite, an untestable acceptance criterion).
//|plan| - `reviewed` — clean, or clean with recorded concerns/suggestions.
//|plan|
//|plan| `rework` and `escalated` both leave the gate **closed**, so this is exactly
//|plan| the outcome the retired PASS / PASS WITH CONCERNS / REWORK vocabulary
//|plan| produced: PASS and PASS WITH CONCERNS both collapse to `reviewed` (they
//|plan| cleared the tag), and REWORK splits into `rework` and `escalated` (both
//|plan| leave it).
//|plan|
//|plan| **Plan-stage severity calibration.** `blocking` means the goal, approach, or
//|plan| scope is wrong, or the plan violates a stated architectural constraint. A
//|plan| defect in a specific proposed line of code or shell (e.g. an off-by-one in
//|plan| proposed pseudo-code) is a `concern` that rides along as an implementation
//|plan| note for the implementing agent — not a gate. An empty or ambiguous plan is
//|plan| still `blocking`.
// The canonical outcome vocabulary. Every surface — the standalone review
// workflow, dispatch-phase, autopilot, and the interactive rdm-review skill —
// speaks exactly these three words. This retired the skill's older
// PASS / PASS-WITH-CONCERNS / BLOCKED / FAIL quartet: PASS and
// PASS-WITH-CONCERNS both collapse to `reviewed`, FAIL becomes `rework`, and
// BLOCKED becomes `escalated`.
const OUTCOMES = ['reviewed', 'rework', 'escalated'];
// >>> review-spec:end <<<

// GATE_POLICY — the ONE mode-dispatched gate table: mode → outcome → policy row.
// The two review surfaces share a gate SKELETON (decide an outcome, then act on
// it) and differ only in the action, so the action is data here rather than a
// forked code path.
//
//   code — the post-implementation gate: persist an rdm status on the item
//          (per kind) and, on `reviewed` only, permit the land-time completion
//          directive. `clearsPlanReviewTag` is always false — the code gate has
//          nothing to do with the pre-implementation tag.
//   plan — the pre-implementation gate: a plan review NEVER persists an rdm
//          status (`status` is an explicit `null`, never `undefined`, so a
//          caller cannot round-trip it into an empty status), and instead
//          clears the reserved `needs-plan-review` tag on `reviewed` only.
//
// The completion policy is expressed ONLY as the boolean `writesCompletion`,
// never as the literal trailer string: this block is stamped verbatim into
// workflow scripts, and scripts/verify-workflow-dispatch.sh forbids that literal
// anywhere inside a stamped region. The literal lives in the skill-only
// `review-gate-spec` region below the stamped block, and the format string
// itself lives in rdm-core (surfaced as `rdm hook done-line`).
const GATE_POLICY = {
  code: {
    reviewed: { phase: 'reviewed', task: 'reviewed', status: 'reviewed', writesCompletion: true, clearsPlanReviewTag: false },
    rework: {
      phase: 'in-progress',
      task: 'in-progress',
      status: 'in-progress',
      writesCompletion: false,
      clearsPlanReviewTag: false,
    },
    escalated: {
      phase: 'blocked',
      task: 'blocked',
      status: 'blocked',
      writesCompletion: false,
      clearsPlanReviewTag: false,
      reasonPrefix: '[code]',
    },
  },
  plan: {
    reviewed: { status: null, writesCompletion: false, clearsPlanReviewTag: true },
    rework: { status: null, writesCompletion: false, clearsPlanReviewTag: false },
    escalated: { status: null, writesCompletion: false, clearsPlanReviewTag: false, reasonPrefix: '[plan]' },
  },
};

// STATUS_MAPPING — the code gate's rows, kept as a named alias so the existing
// consumers (dispatch-phase, autopilot) and their drift harnesses see exactly
// the table they saw before. One table, not a fork.
const STATUS_MAPPING = GATE_POLICY.code;

// The item kinds a code-gate status may be looked up for.
const ITEM_KINDS = ['phase', 'task'];

// gateFor(mode, outcome) — the policy row for one mode/outcome pair. Throws an
// actionable error on an unknown mode or outcome rather than returning
// `undefined`, so a caller can never silently act on a partial row.
function gateFor(mode, outcome) {
  const table = GATE_POLICY[mode];
  if (!table) {
    throw new Error('review: unknown gate mode "' + mode + '" (expected one of ' + Object.keys(GATE_POLICY).join(', ') + ')');
  }
  const row = table[outcome];
  if (!row) {
    throw new Error(
      'review: unknown outcome "' + outcome + '" for gate mode "' + mode + '" (expected one of ' + OUTCOMES.join(', ') + ')'
    );
  }
  return row;
}

// statusFor(outcome, kind) — the rdm status an outcome maps to for a phase or a
// task. Throws on an unknown outcome or kind rather than returning undefined: a
// silent `undefined` would be persisted as an empty status by a caller that did
// not check.
function statusFor(outcome, kind) {
  const row = gateFor('code', outcome);
  if (ITEM_KINDS.indexOf(kind) === -1) {
    throw new Error('review: unknown item kind "' + kind + '" (expected "phase" or "task")');
  }
  return row[kind];
}

// writesCompletion(outcome) — may this outcome's surface write the land-time
// completion directive? Only a clean review may.
function writesCompletion(outcome) {
  return gateFor('code', outcome).writesCompletion === true;
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

// JSON Schema a single AC-table row must satisfy — one entry per acceptance
// criterion (see docs/workflow-schemas.md § AC_ENTRY).
const AC_ENTRY_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['criterion', 'status', 'evidence'],
  properties: {
    criterion: { type: 'string', minLength: 1 },
    status: { type: 'string', enum: ['PASS', 'FAIL', 'PARTIAL'] },
    evidence: { type: 'string' },
  },
};

// JSON Schema the `ac` dimension's finder is forced to satisfy in `code` mode
// ONLY (see docs/workflow-schemas.md § AC_REVIEW_SCHEMA): the structured
// per-criterion table (`ac`, required) plus an OPTIONAL `findings` array (same
// shape as FINDINGS_SCHEMA's) for narrative notes that don't reduce to a
// single criterion's status. Every other dimension keeps using FINDINGS_SCHEMA.
const AC_REVIEW_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ac'],
  properties: {
    ac: { type: 'array', items: AC_ENTRY_SCHEMA },
    findings: FINDINGS_SCHEMA.properties.findings,
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

// The boolean signal keys deriveSignals always populates explicitly.
// `targetType` (string|null) and `changedFiles` (array) ride alongside them.
const SIGNAL_KEYS = [
  'changesLogic',
  'missingTests',
  'multiModule',
  'publicApiChanged',
  'userFacing',
  'securitySurface',
  'hasUnsafe',
];

// selectDimensions(mode, signals) — the deterministic pre-step that decides
// which dimensions actually run.
//
// THREE-WAY CONTRACT (the fail-open rule is load-bearing):
//   * `signals == null` (omitted / genuinely unknown) → return ALL dimensions
//     for the mode, untouched. A caller that cannot compute a diff knows the
//     LEAST, so it must get the MOST coverage.
//   * an explicit signals object (even `{}`) → run the always-on dimensions plus
//     exactly those whose `when` predicate fires. `{}` therefore means "computed,
//     nothing triggered".
//   * an unknown mode → throw.
//
// Do NOT collapse this into `d.when(signals || {})`. Substituting `{}` for
// omitted signals would make EVERY conditional predicate read falsy and silently
// drop the triggered dimensions — returning a strict subset precisely when the
// caller had no information, which is a silent coverage regression.
function selectDimensions(mode, signals) {
  const dims = DIMENSIONS[mode];
  if (!dims) throw new Error('unknown review mode: ' + mode + ' (expected "code" or "plan")');
  if (signals == null) return dims.slice();
  const sel = dims.filter((d) => !d.when || d.when(signals));
  if (sel.length === 0) {
    throw new Error('review: no dimensions selected for mode "' + mode + '" — the always-on set must never be empty');
  }
  return sel;
}

// Path/keyword rules for deriveSignals. Fixed lists anchored on path-segment
// boundaries, so an incidental substring cannot trip a trigger.
const TEST_PATH_PATTERNS = [/(^|\/)tests?(\/|$)/, /(^|[/_.-])test[_.-]/, /[_.-]test\.[a-z]+$/, /(^|[/_.-])spec[_.-]/];
const CODE_EXTENSIONS = ['.rs', '.js', '.mjs', '.cjs', '.ts', '.tsx', '.py', '.go', '.sh', '.pkl'];
const SECURITY_PATH_PATTERNS = [
  /(^|[/_.-])auth([/_.-]|$)/,
  /(^|[/_.-])credential/,
  /(^|[/_.-])secret/,
  /(^|[/_.-])token/,
  /(^|[/_.-])password/,
  /(^|[/_.-])crypto/,
  /(^|[/_.-])exec([/_.-]|$)/,
  /(^|[/_.-])process([/_.-]|$)/,
  /(^|[/_.-])shell([/_.-]|$)/,
  /(^|[/_.-])subprocess/,
  /(^|[/_.-])net(work)?([/_.-]|$)/,
  /(^|[/_.-])http/,
  /(^|[/_.-])serde/,
  /(^|[/_.-])deserial/,
  /(^|[/_.-])parse[rs]?([/_.-]|$)/,
  /(^|[/_.-])path([/_.-]|$)/,
  /(^|[/_.-])fs([/_.-]|$)/,
  /(^|[/_.-])hook([/_.-]|$)/,
];
const SECURITY_DIFF_PATTERNS = [
  /std::process/,
  /Command::new/,
  /std::fs::/,
  /env::var/,
  /from_utf8_unchecked/,
  /set_permissions/,
];
const USER_FACING_PATH_PATTERNS = [
  /^rdm-cli\//,
  /^rdm-server\//,
  /(^|[/_.-])mcp([/_.-]|$)/,
  /(^|[/_.-])config([/_.-]|$)/,
];

// deriveSignals(input) — map `{ targetType, changedFiles, diffText }` to a
// FULLY-POPULATED signals object. Every boolean key in SIGNAL_KEYS is set
// explicitly, never left undefined: a partially-populated object would make a
// conditional dimension drop out on a MISSING key rather than on a real negative.
//
// Pure and deterministic — fixed path/keyword rules, no Date.now / Math.random,
// no shell. A caller that cannot compute a diff must pass NO signals at all (see
// selectDimensions' fail-open rule) rather than a partial object.
function deriveSignals(input) {
  const i = input || {};
  const targetType = i.targetType || null;
  const files = Array.isArray(i.changedFiles) ? i.changedFiles.filter((f) => typeof f === 'string') : [];
  const diffText = typeof i.diffText === 'string' ? i.diffText : null;
  const lower = files.map((f) => f.toLowerCase());

  const isTest = (p) => TEST_PATH_PATTERNS.some((re) => re.test(p));
  const isCode = (p) => CODE_EXTENSIONS.some((ext) => p.slice(-ext.length) === ext);

  const codeFiles = lower.filter((p) => isCode(p) && !isTest(p));
  const testFiles = lower.filter(isTest);

  const dirs = {};
  for (const p of lower) {
    const idx = p.lastIndexOf('/');
    dirs[idx === -1 ? '.' : p.slice(0, idx)] = true;
  }

  return {
    targetType: targetType,
    changedFiles: files.slice(),
    changesLogic: codeFiles.length > 0,
    missingTests: codeFiles.length > 0 && testFiles.length === 0,
    multiModule: Object.keys(dirs).length > 1,
    publicApiChanged:
      lower.some((p) => p.indexOf('rdm-core/src/') === 0) && (diffText === null || /(^|\n)\+.*\bpub\b/.test(diffText)),
    userFacing: lower.some((p) => USER_FACING_PATH_PATTERNS.some((re) => re.test(p))),
    securitySurface:
      lower.some((p) => SECURITY_PATH_PATTERNS.some((re) => re.test(p))) ||
      (diffText !== null && SECURITY_DIFF_PATTERNS.some((re) => re.test(diffText))),
    hasUnsafe: diffText !== null && /(^|\n)\+.*\bunsafe\b/.test(diffText),
  };
}

// hasBlocking(findings, tier) — is there a blocking finding, tier-scaled?
// For the `large` tier a surviving `concern` is treated as blocking too (a
// one-directional tightening — the gate can only get stricter, never looser).
function hasBlocking(findings, tier) {
  const list = Array.isArray(findings) ? findings : [];
  const blockers = tier === 'large' ? ['blocking', 'concern'] : ['blocking'];
  return list.some((f) => f && blockers.indexOf(f.severity) !== -1);
}

// acTableHasGap(acTable) — does a structured AC table (the `ac` dimension's
// code-mode output, see AC_REVIEW_SCHEMA) contain any FAIL or PARTIAL
// criterion? An empty or absent table is NOT a gap — plan mode never sets one,
// and a code review whose `ac` dimension didn't run or whose finder failed to
// resolve a table must not be treated as if it found a defect.
function acTableHasGap(acTable) {
  const list = Array.isArray(acTable) ? acTable : [];
  return list.some((entry) => entry && (entry.status === 'FAIL' || entry.status === 'PARTIAL'));
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

// --- Plan-standalone consolidation helpers -----------------------------------
// Three pure, post-pipeline consolidation/gate helpers the standalone
// plan-review workflow (.claude/workflows/plan-review.js) consumes. They are
// CONSOLIDATION, not find/refute logic — they operate on the ranked survivors a
// `buildReviewPipeline('plan')` run already produced, and add no new review
// dimension, finder, or refuter. They live inside the stamped block so the
// workflow consumer picks them up verbatim (the runtime cannot import), and are
// exported for the Node verify harness.

// stripNonPhaseUnitOfWork(survivors, targetType) — drop any survivor whose
// `concern` is 'unit-of-work' UNLESS the review unit is a phase. Order-preserving
// and idempotent.
//
// This is the CONSUMER-SIDE phase-scoping that selectDimensions' omitted-signals
// path cannot do. plan-review.js deliberately runs `buildReviewPipeline('plan')`
// with NO signals (honoring the dispatch-phase deferral of signal-threading to
// the sibling unify-plan-review roadmap), so selectDimensions fail-opens and the
// unit-of-work finder runs on EVERY unit — task, roadmap body, and
// implementation-plan included. This post-hoc filter makes "unit-of-work only on
// phase units" actually true without threading a signals object.
function stripNonPhaseUnitOfWork(survivors, targetType) {
  const list = Array.isArray(survivors) ? survivors : [];
  if (targetType === 'phase') return list.slice();
  return list.filter((f) => !(f && f.concern === 'unit-of-work'));
}

// filterPlanReviewTag(tags) — the read-filter-write half of the plan gate: return
// the tag list with the reserved `needs-plan-review` removed by EXACT string
// match, order and every sibling tag (e.g. `depends-unlanded`) preserved.
// Idempotent (a list already lacking it is a safe no-op) and returns [] when
// `needs-plan-review` was the only tag. `--tags` replaces the whole list, so a
// caller must always write back this COMPLETE remaining list, never a blind
// single-tag removal.
function filterPlanReviewTag(tags) {
  const list = Array.isArray(tags) ? tags : [];
  return list.filter((t) => t !== 'needs-plan-review');
}

// classifyPlanOutcome(survivors) — map post-strip plan survivors onto the
// canonical outcome vocabulary, reusing `hasBlocking` (no new severity logic):
//   * no blocking survivor            → 'reviewed'
//   * a blocking `architectural-fit`  → 'escalated' (a stated-constraint
//     survivor                          violation needs a human decision, per the
//                                       plan-stage reading)
//   * any other blocking survivor     → 'rework' (a fixable rewrite — e.g. an
//                                       empty/ambiguous plan surfaces as a
//                                       blocking `coherence` finding)
function classifyPlanOutcome(survivors) {
  const list = Array.isArray(survivors) ? survivors : [];
  if (!hasBlocking(list)) return 'reviewed';
  const blockingArchFit = list.some((f) => f && f.severity === 'blocking' && f.concern === 'architectural-fit');
  return blockingArchFit ? 'escalated' : 'rework';
}

// DEFAULT_MAX_CODE_REWORK — the in-run code-rework budget. A budget of N means N
// reworks AFTER the original attempt, i.e. N + 1 attempts. 0 is legal and
// MEANINGFUL (no reworks at all — terminate on the first blocking review) and
// must never be conflated with "unset" by a falsy check.
const DEFAULT_MAX_CODE_REWORK = 2;

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

// classifyOutcome — the total, deterministic decision tree. Returns one of the
// OUTCOMES: 'escalated' | 'reviewed' | 'rework'.
//
// The deterministic pipeline cannot classify a code finding's *nature* (the
// FINDING schema carries severity but no fixable/decision flag), so a code
// defect that survives the bounded reworks resolves to 'rework'; genuine
// decisions surface earlier at the plan gate as 'escalated'. That is why the
// code stage yields only reviewed|rework and escalated originates at the plan
// gate. An LLM-driven surface (the interactive skill) CAN judge nature, and so
// applies rule 1 of the verdict spec above directly.
//
// `input.acTable` is the structured AC table belonging to the LAST completed
// code round (see AC_REVIEW_SCHEMA / acTableHasGap). It is checked directly,
// independent of finding severity and refutation — this can only ever yield
// 'rework', never 'escalated': a code-stage defect's nature still can't be
// classified deterministically (see above), so an AC-table gap stays in the
// same reviewed|rework lane as every other surviving code finding.
function classifyOutcome(input) {
  const i = input || {};
  const tier = i.tier;
  const planFindings = i.planFindings || [];
  // 1. Plan gate: a blocking plan finding escalates before any implementation.
  //    An empty/ambiguous plan is surfaced as a blocking coherence finding by
  //    the plan-review stage, so that case lands here too.
  if (hasBlocking(planFindings, tier)) return 'escalated';
  // 2. AC-table gate: a surviving FAIL/PARTIAL criterion mechanically forces
  //    rework, independent of finding severity and refutation.
  if (acTableHasGap(i.acTable)) return 'rework';
  // 3. Plan approved → implement ran → code-review ran (once per round). The
  //    LAST review's findings decide, for any number of rework rounds including
  //    zero: still blocking → rework, otherwise reviewed.
  const rounds = codeReviewRounds(i);
  const last = rounds[rounds.length - 1] || [];
  return hasBlocking(last, tier) ? 'rework' : 'reviewed';
}

// Build the review pipeline for `mode` ("code" | "plan").
//
// Returns an async `runReview(context)` that:
//   1. selects the applicable dimensions from `context.signals` (see
//      selectDimensions' three-way fail-open contract),
//   2. runs one finder agent per selected dimension IN PARALLEL (stage 1) — in
//      `code` mode the `ac` dimension's finder returns the AC_REVIEW_SCHEMA
//      shape instead of a bare findings array, and its `ac` table is captured,
//   3. runs a FRESH refuter agent per finding, in parallel (stage 2),
//   4. drops any finding that was refuted or scored below CONFIDENCE_FLOOR,
//   5. returns `{ survivors, acTable }` — survivors ranked most-severe-first,
//      and the captured AC table (`null` in `plan` mode, or if the `ac`
//      dimension didn't run or its finder failed to resolve a table).
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
  if (!DIMENSIONS[mode]) throw new Error('unknown review mode: ' + mode + ' (expected "code" or "plan")');
  if (!_agent || !_pipeline || !_parallel) {
    throw new Error('review-refute-fix: missing agent/pipeline/parallel (pass deps outside the Workflow runtime)');
  }

  return async function runReview(context) {
    const ctx = context || {};
    // Deterministic pre-step: which dimensions actually run. A caller with no
    // diff signals passes none and gets EVERY dimension (fail-open).
    const dims = selectDimensions(mode, ctx.signals);
    // Optional explicit models for the two review steps. Callers that have no
    // tier context (the standalone review-refute-fix consumer) simply omit them
    // and the agents inherit the session model exactly as before. Passing
    // `model: undefined` is INERT — verified by the agent() model spike recorded
    // in docs/workflow-schemas.md § "agent() options" — so always-assigning the
    // key is safe and needs no conditional-assignment helper.
    const findModel = ctx.findModel;
    const verifyModel = ctx.verifyModel;
    // Captured the first (only) time the `ac` dimension's finder resolves a
    // table in `code` mode. Stays `null` in `plan` mode (the `ac` dimension
    // does not exist there) and when the `ac` dimension didn't run or its
    // finder failed to resolve a table. This is the STRUCTURED side-channel
    // classifyOutcome consumes directly — never through finding severity or
    // refutation.
    let acTable = null;
    // Stage 1: parallel dimension finders. Stage 2: a fresh refuter per finding.
    // pipeline() keeps each dimension's find→refute chain independent (no barrier).
    const perDimension = await _pipeline(
      dims,
      (dim) => {
        const isAcDimension = mode === 'code' && dim.key === 'ac';
        return _agent(findPrompt(mode, dim, ctx), {
          label: 'find:' + mode + ':' + dim.key,
          phase: 'Find',
          schema: isAcDimension ? AC_REVIEW_SCHEMA : FINDINGS_SCHEMA,
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
          if (isAcDimension && found && Array.isArray(found.ac)) {
            acTable = found.ac;
          }
          return found;
        });
      },
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
    return { survivors: rankFindings(survivors), acTable: acTable };
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

// --- Schemas (dispatch-specific; see docs/workflow-schemas.md) ----------------

// DIFF_SIGNALS — what the mechanical diff agent returns from the item's worktree
// so the code gate can select review dimensions from the REAL change shape via
// the canonical `deriveSignals`. `diffText` is truncated by the agent; truncation
// only weakens trigger detection toward FAIL-OPEN (a missed trigger costs a
// dimension that would have run anyway when the file paths already imply it), and
// an empty/failed result omits `signals` entirely so every dimension runs.
const DIFF_SIGNALS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['changedFiles', 'diffText'],
  properties: {
    changedFiles: { type: 'array', items: { type: 'string' } },
    diffText: { type: 'string' },
  },
}

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
      required: ['plan', 'implement', 'review_find', 'review_verify', 'mechanical'],
      properties: {
        plan: { type: 'string' },
        implement: { type: 'string' },
        review_find: { type: 'string' },
        review_verify: { type: 'string' },
        mechanical: { type: 'string' },
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
      required: ['plan', 'implement', 'review_find', 'review_verify', 'mechanical'],
      properties: {
        plan: { type: 'string' },
        implement: { type: 'string' },
        review_find: { type: 'string' },
        review_verify: { type: 'string' },
        mechanical: { type: 'string' },
      },
    },
  },
}

// STAMP_ACK — what the mechanical in-progress-stamp agent reports back: did the
// status-update command it ran exit 0? No retry — the stamp is best-effort
// observability, not a gated step (see buildStampInProgressPrompt below).
const STAMP_ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok'],
  properties: { ok: { type: 'boolean' } },
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
    'ALWAYS run these three with NO --tier argument, whatever T is:',
    '  ./target/debug/rdm model resolve review-find',
    '  ./target/debug/rdm model resolve review-verify',
    '  ./target/debug/rdm model resolve mechanical',
    'Return the five resulting model ids verbatim in a `models` object with keys',
    'plan, implement, review_find, review_verify, mechanical. Do not invent ids; if a command fails, return an empty body.',
  ].join('\n')
}

function buildTaskFetchPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm task show ' + slug + ' --project rdm --format json',
    'Return a TASK_META object: task (the slug you were given) and body (the task JSON `body` verbatim).',
    'If the command fails or the body is empty, return an empty body.',
    'Then resolve the models for this dispatch. A task carries NO tier, so run all five',
    'resolver commands with NO --tier argument:',
    '  ./target/debug/rdm model resolve plan',
    '  ./target/debug/rdm model resolve implement',
    '  ./target/debug/rdm model resolve review-find',
    '  ./target/debug/rdm model resolve review-verify',
    '  ./target/debug/rdm model resolve mechanical',
    'Return the five resulting model ids verbatim in a `models` object with keys',
    'plan, implement, review_find, review_verify, mechanical. Do not invent ids; if a command fails, return an empty body.',
  ].join('\n')
}

// Observability stamp: a mechanical agent marks the phase/task in-progress the
// moment real work begins on it. Best-effort — never gated, never retried; see
// the driver call site (right after Stage 0, before the plan gate) for the
// try/catch that keeps a failed stamp from affecting control flow.
function buildStampInProgressPrompt(isTaskFlag, roadmapSlugArg, target) {
  const cmd = isTaskFlag
    ? './target/debug/rdm task update ' + target + ' --status in-progress --no-edit --project rdm'
    : './target/debug/rdm phase update ' +
      target +
      ' --status in-progress --no-edit --roadmap ' +
      roadmapSlugArg +
      ' --project rdm'
  return [
    'You are a mechanical status agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root:',
    '  ' + cmd,
    'Return a STAMP_ACK object: { ok: true } if the command exited 0, otherwise { ok: false }.',
    'Do not retry on failure — report the result of the single attempt.',
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
// approved plan doc (+ optional code-review findings / AC-table gaps on the
// rework pass). It is NEVER given the planner's or plan-reviewer's
// context/transcript. `reworkNotes`, when present, is `{ findings, acTable }`
// from runCodeGate's rework call (`d.implement({ findings, acTable })`) —
// NEVER plan-review findings, and never a bare findings array: the AC table is
// a structured side-channel decoupled from `findings` (a FAIL/PARTIAL
// criterion need not also appear as a finding), so an AC-only-gap rework round
// (empty `findings`) still needs `acTable` rendered or the implementer gets no
// signal about what to fix at all.
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
    const notesFindings = Array.isArray(reworkNotes.findings) ? reworkNotes.findings : []
    const notesAcTable = Array.isArray(reworkNotes.acTable) ? reworkNotes.acTable : []
    const acGaps = notesAcTable.filter((e) => e && (e.status === 'FAIL' || e.status === 'PARTIAL'))
    if (notesFindings.length > 0) {
      lines.push('Code-review found these ranked issues on the prior pass — fix every blocking one:')
      lines.push(JSON.stringify(notesFindings, null, 2))
    }
    if (acGaps.length > 0) {
      lines.push(
        'The acceptance-criteria table found these UNMET criteria on the prior pass — fix every one (this is a separate channel from the findings above, not a duplicate):'
      )
      lines.push(JSON.stringify(acGaps, null, 2))
    }
  }
  return lines.join('\n')
}

// Review pre-step: a mechanical Bash agent reads the branch diff out of the
// item's worktree. Its output feeds `deriveSignals` (from the stamped canonical
// review block), which decides which review dimensions actually run.
//
// Diff base: THREE-DOT (`main...HEAD`) scopes to the branch's own changes rather
// than to everything `main` gained meanwhile. For a phase implemented in the
// SHARED per-roadmap worktree, earlier phases of the same roadmap are already on
// the branch, so a later phase sees the whole branch diff. That is
// over-inclusive (a trigger may fire for an earlier phase's files) but never
// under-inclusive, which is the safe direction for a coverage gate.
function buildDiffSignalsPrompt(worktreeRef) {
  return [
    'You are a mechanical diff agent. Do not review, plan, or implement anything, and edit no files.',
    'Find the worktree for this item and work THERE:',
    '  ./target/debug/rdm worktree add ' + worktreeRef + ' --project rdm',
    '(it prints the existing path if the worktree already exists) then `cd` into that path.',
    'Run exactly these two commands and read their output:',
    '  git diff --name-only main...HEAD',
    '  git diff main...HEAD',
    'Return a DIFF_SIGNALS object: `changedFiles` — the repo-relative paths from the first command,',
    'verbatim, one array element each; and `diffText` — the second command\'s output TRUNCATED to the',
    'first 40000 characters (append nothing; just stop). If either command fails or the branch has no',
    'commits of its own, return an empty `changedFiles` array and an empty `diffText`.',
  ].join('\n')
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
      acRounds: f.acRounds,
      maxRework: f.maxRework,
      tier: f.tier,
      actResult: f.actResult,
    })
  }
  return buildOutcome({
    roadmap: roadmap,
    phase: phaseArg,
    fetchError: f.fetchError,
    planFindings: f.planFindings,
    codeReviews: f.codeReviews,
    acRounds: f.acRounds,
    maxRework: f.maxRework,
    tier: f.tier,
    actResult: f.actResult,
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
const unresolvedStep = ['plan', 'implement', 'review_find', 'review_verify', 'mechanical'].filter(
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

// Observability stamp: mark the item in-progress the moment real work begins
// (right after Stage 0 resolves metadata + models, before planning). This is
// the only entry point that reaches dispatch-phase WITHOUT already having
// stamped in-progress itself — interactive rdm-do, rdm-do --auto, and the
// rdm-dispatch-phase skill all stamp before invoking the workflow; autopilot
// calls this workflow directly and writes no status of its own. Best-effort:
// wrapped in try/catch, and a non-ok ack only logs — it never gates the run,
// never mutates plan/code-gate state, and never appears in the returned
// OUTCOME. Guarded by `if (!planOnly)`, using the already-parsed
// `dispatchArgs.planOnly` local: a --plan-only pass does no implementation, so
// stamping in-progress would misreport it, and skipping (not reverting) avoids
// clobbering a phase legitimately left in-progress by an earlier interrupted
// run.
if (!planOnly) {
  try {
    const target = isTask ? taskSlug : stem
    const stampAck = await agent(buildStampInProgressPrompt(isTask, roadmapSlug, target), {
      label: 'stamp:in-progress',
      phase: 'Implement',
      schema: STAMP_ACK_SCHEMA,
      model: models.mechanical,
    })
    if (!stampAck || stampAck.ok !== true) {
      log('dispatch-phase: in-progress stamp did not confirm for ' + itemLabel + ' — continuing (observability only)')
    }
  } catch (e) {
    log('dispatch-phase: in-progress stamp failed for ' + itemLabel + ' — continuing (observability only)')
  }
}

// Stages A + B: author the plan from ONLY the phase body, review it via the
// stamped shared pipeline, and revise it up to the plan-revise budget. The loop
// itself lives in runPlanGate (copied block) so it is driveable from Node; this
// driver only supplies the side effects.
//
// SIGNALS SITE (plan gate): this gate deliberately passes NO `signals`, so
// selectDimensions fail-opens and every plan dimension runs — including
// `unit-of-work`, whose `when` is a TARGET-TYPE trigger (phases only). Threading
// `signals: { targetType: isTask ? 'task' : 'phase' }` here belongs to the
// sibling `unify-plan-review` roadmap (phase 3, wire-plan-gates-and-hook), not
// to this one. Do not add it here.
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
    // The code gate IS the canonical review — `buildReviewPipeline('code')` from
    // the stamped block, with NO independent code-review logic in this driver.
    // The diff is fetched INSIDE this closure so every rework round re-derives
    // its signals from the post-rework tree: a round-2 fix that newly touches an
    // `rdm-core` public item must turn `api-docs` on for round 2.
    review: async () => {
      let diff = null
      try {
        diff = await agent(buildDiffSignalsPrompt(worktreeRef), {
          label: 'diff:signals',
          phase: 'Review',
          schema: DIFF_SIGNALS_SCHEMA,
          model: models.mechanical,
        })
      } catch (e) {
        diff = null
      }
      const changedFiles = diff && Array.isArray(diff.changedFiles) ? diff.changedFiles.filter(Boolean) : []
      if (changedFiles.length === 0) {
        // FAIL-OPEN: omit the `signals` key ENTIRELY — never pass `{}`.
        // selectDimensions treats an omitted `signals` as "unknown → run every
        // dimension", while `{}` means "computed, nothing triggered" and would
        // silently drop tests / api-docs / changelog / security coverage exactly
        // when the driver knew the least.
        log('dispatch-phase: diff signals unavailable for ' + itemLabel + ' — running every code dimension (fail-open)')
        return runCodeReview({ target: reviewTarget, ...reviewModels })
      }
      const signals = deriveSignals({
        targetType: isTask ? 'task' : 'phase',
        changedFiles: changedFiles,
        diffText: typeof diff.diffText === 'string' ? diff.diffText : null,
      })
      return runCodeReview({ target: reviewTarget, signals: signals, ...reviewModels })
    },
    // Act: only invoked by runCodeGate when the FINAL round is clean with
    // non-empty surviving (non-gating) findings. Incorporates each finding by
    // size — small fixed inline in the worktree, large filed as a task — per
    // buildCodeActPrompt. A missing/failing agent call never affects the
    // outcome (runCodeGate already swallows a throw from this dep).
    act: async (findings) =>
      agent(buildCodeActPrompt(isTask ? 'task' : 'phase', roadmap, isTask ? taskSlug : stem, worktreeRef, findings), {
        model: models.implement,
        label: 'act:code',
        phase: 'Act',
        schema: CODE_ACT_SCHEMA,
      }),
  }
)

const outcome = itemOutcome({
  planFindings: planFindings,
  codeReviews: codeGate.rounds,
  acRounds: codeGate.acRounds,
  maxRework: maxCodeRework,
  tier: tier,
  actResult: codeGate.actResult,
})
log('dispatch-phase (' + itemLabel + '): ' + outcome.outcome + ' — ' + outcome.summary)
return outcome
