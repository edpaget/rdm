// plan-review — standalone plan-mode review workflow for direct invocation.
//
// Reviews the PLAN of an rdm item before implementation begins (the earlier of
// the two review gates), reusing the ONE canonical review core in
// .claude/workflows/lib/review.mjs — `buildReviewPipeline('plan')` for
// find → refute → filter, and `GATE_POLICY.plan` / `gateFor('plan', …)` for the
// gate. It declares NO review dimension, finder, refuter, or gate table of its
// own; the stamped block below is copied VERBATIM from the lib by
// scripts/gen-workflow-review.sh (the Workflow runtime cannot import a module —
// see docs/workflow-schemas.md § "Import spike"). Edit the lib, not the copy;
// scripts/verify-workflow-review.sh fails the build on drift.
//
// Four target types (mirroring the rdm-plan-review skill's $ARGUMENTS surface):
//
//   1. `--task <slug>`            — review a task's plan.
//   2. `--roadmap <slug>`         — review the whole roadmap: its own body plus
//                                   EVERY phase, each gated INDEPENDENTLY (an
//                                   ambient parallel() fan-out).
//   3. `<slug> [phase]`           — positional: a single phase when a phase arg
//                                   is present, else identical to --roadmap.
//   4. `--implementation-plan`    — review an rdm-do plan document handed over in
//                                   context ahead of implementation. There is NO
//                                   persisted rdm item behind it, so it is
//                                   report-only: no body edit, no filed task, and
//                                   no gate.
//
// Args may arrive as a raw $ARGUMENTS flag string, a JSON payload, or a
// structured object ({ roadmap, phase }, { task }, { implementationPlan,
// planText }). See parsePlanArgs.
//
// Unit-of-work scoping: this workflow deliberately passes NO `signals` into the
// pipeline (honoring the dispatch-phase deferral of signal-threading to the
// sibling unify-plan-review roadmap), so selectDimensions fail-opens and the
// unit-of-work finder runs on every unit. Phase-only scoping is instead applied
// in the CONSUMER via stripNonPhaseUnitOfWork(survivors, targetType).

export const meta = {
  name: 'plan-review',
  description:
    'Standalone plan-mode review: find → refute → filter over a task/phase/roadmap/implementation-plan, with per-phase independent needs-plan-review gating',
  phases: [{ title: 'Read' }, { title: 'Find' }, { title: 'Refute' }, { title: 'Act' }, { title: 'Gate' }],
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
    //|plan|   when already annotated, downgrade it to a `concern` (or omit it).
    {
      key: 'coherence',
      title: 'Coherence',
      focus:
        'Internal consistency and completeness: are the steps and acceptance criteria concrete and actionable? An empty or ambiguous plan is itself a blocking finding — never guess intent. A plan step citing a file or behavior as existing, where it was actually introduced by another in-flight (not-yet-landed) roadmap or task, is only blocking if the target item does NOT carry the `depends-unlanded` tag and does not state the dependency explicitly; when already annotated, downgrade to a concern (or omit) instead of blocking on it.',
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
//|code| - Keep the AC table intact; surviving AC FAIL/PARTIAL items become findings.
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
//|code| 2. **rework** — else if any surviving finding is `blocking`, or the AC table
//|code|    contains any FAIL or PARTIAL criterion. The defect is fixable in place; the
//|code|    work goes back for another round.
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

// Build the review pipeline for `mode` ("code" | "plan").
//
// Returns an async `runReview(context)` that:
//   1. selects the applicable dimensions from `context.signals` (see
//      selectDimensions' three-way fail-open contract),
//   2. runs one finder agent per selected dimension IN PARALLEL (stage 1),
//   3. runs a FRESH refuter agent per finding, in parallel (stage 2),
//   4. drops any finding that was refuted or scored below CONFIDENCE_FLOOR,
//   5. returns the survivors ranked most-severe-first.
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

// --- Driver -------------------------------------------------------------------

// parsePlanArgs(rawArgs) — resolve the four target types from a raw $ARGUMENTS
// flag string, a JSON payload, or a structured object. Returns
// { kind, roadmap, phase, task, planText } where kind is one of
// 'task' | 'phase' | 'roadmap' | 'implementation-plan'.
function parsePlanArgs(rawArgs) {
  let a = rawArgs || {}
  if (typeof a === 'string') {
    const trimmed = a.trim()
    if (trimmed.slice(0, 1) === '{') {
      try {
        a = JSON.parse(trimmed) || {}
      } catch (e) {
        a = { target: a }
      }
    } else {
      a = { target: a }
    }
  }
  if (!a || typeof a !== 'object') a = {}

  // Tokenize a raw $ARGUMENTS-style flag string if one was supplied.
  const rawTarget =
    typeof a.target === 'string'
      ? a.target
      : typeof a.arguments === 'string'
      ? a.arguments
      : typeof a.args === 'string'
      ? a.args
      : ''
  const tokens = rawTarget.trim() ? rawTarget.trim().split(/\s+/) : []

  let roadmap = ''
  let phase = ''
  let task = ''
  let implementationPlan = false
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i]
    if (t === '--task') {
      task = tokens[i + 1] || ''
      i++
    } else if (t === '--roadmap') {
      roadmap = tokens[i + 1] || ''
      i++
    } else if (t === '--implementation-plan') {
      implementationPlan = true
    } else if (t.slice(0, 2) !== '--') {
      // Positional `<slug> [phase]`.
      if (!roadmap) roadmap = t
      else if (!phase) phase = t
    }
  }

  // Structured object keys supplement / override the flag string.
  if (typeof a.task === 'string' && a.task) task = a.task
  if (typeof a.roadmap === 'string' && a.roadmap) roadmap = a.roadmap
  if (typeof a.phase === 'string' && a.phase) phase = a.phase
  if (a.implementationPlan) implementationPlan = true

  const planText = typeof a.planText === 'string' ? a.planText : typeof a.plan === 'string' ? a.plan : ''

  let kind
  if (implementationPlan) kind = 'implementation-plan'
  else if (task) kind = 'task'
  else if (roadmap && phase) kind = 'phase'
  else if (roadmap) kind = 'roadmap'
  else
    throw new Error(
      'plan-review: no target — pass --task <slug>, --roadmap <slug>, <slug> [phase], or --implementation-plan'
    )

  return { kind: kind, roadmap: roadmap, phase: phase, task: task, planText: planText }
}

// Schemas the mechanical Bash fetch agents are forced to satisfy. Plumbing, not
// review logic.
const PLAN_TARGET_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['body', 'tags'],
  properties: {
    body: { type: 'string' },
    tags: { type: 'array', items: { type: 'string' } },
  },
}
const ROADMAP_TARGET_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['body', 'tags', 'phases'],
  properties: {
    body: { type: 'string' },
    tags: { type: 'array', items: { type: 'string' } },
    phases: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['stem', 'body', 'tags'],
        properties: {
          stem: { type: 'string' },
          body: { type: 'string' },
          tags: { type: 'array', items: { type: 'string' } },
        },
      },
    },
  },
}
const STAMP_ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok'],
  properties: { ok: { type: 'boolean' } },
}

// Fetch prompts — mechanical Bash agents (the runtime cannot shell out itself).
function buildPhaseFetchPrompt(roadmap, phase) {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm phase show ' + phase + ' --roadmap ' + roadmap + ' --project rdm --format json',
    'Return a PLAN_TARGET object: `body` (the phase JSON `body` verbatim) and `tags` (the phase JSON',
    '`tags` array verbatim, one element each — an empty array if there are none).',
    'If the command fails or the body is empty, return an empty `body` and an empty `tags` array.',
  ].join('\n')
}
function buildTaskFetchPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm task show ' + slug + ' --project rdm --format json',
    'Return a PLAN_TARGET object: `body` (the task JSON `body` verbatim) and `tags` (the task JSON',
    '`tags` array verbatim, one element each — an empty array if there are none).',
    'If the command fails or the body is empty, return an empty `body` and an empty `tags` array.',
  ].join('\n')
}
function buildRoadmapFetchPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm roadmap show ' + slug + ' --project rdm --format json',
    'That JSON carries the roadmap body, the roadmap tags, and a summary of every phase.',
    'For each phase, ALSO run and read:',
    '  ./target/debug/rdm phase show <stem> --roadmap ' + slug + ' --project rdm --format json',
    'to get that phase\'s full `body` and `tags`.',
    'Return a ROADMAP_TARGET object: `body` (the roadmap JSON `body` verbatim), `tags` (the roadmap JSON',
    '`tags` array verbatim), and `phases` — one entry per phase with `stem` (the phase JSON `stem`), `body`',
    '(the phase JSON `body` verbatim), and `tags` (the phase JSON `tags` array verbatim).',
    'If the command fails or the roadmap body is empty, return an empty `body`, an empty `tags` array, and an empty `phases` array.',
  ].join('\n')
}

// buildTagWritePrompt — the read-filter-write half of the gate, as a mechanical
// agent. The COMPLETE remaining list (already filtered by filterPlanReviewTag) is
// written back, since `--tags` replaces the whole list; an empty list writes
// `--tags ""`. Leaves the change staged for the caller's commit.
function buildTagWritePrompt(kind, roadmap, ident, remainingTags) {
  const tagsFlag = remainingTags.length === 0 ? '--tags ""' : '--tags "' + remainingTags.join(',') + '"'
  let updateCmd
  if (kind === 'task') {
    updateCmd = './target/debug/rdm task update ' + ident + ' ' + tagsFlag + ' --no-edit --project rdm'
  } else if (kind === 'phase') {
    updateCmd = './target/debug/rdm phase update ' + ident + ' --roadmap ' + roadmap + ' ' + tagsFlag + ' --no-edit --project rdm'
  } else {
    updateCmd = './target/debug/rdm roadmap update ' + ident + ' ' + tagsFlag + ' --no-edit --project rdm'
  }
  return [
    'You are a mechanical status agent. Do not plan, implement, or review anything.',
    'Run exactly these two commands in the repo root:',
    '  ' + updateCmd,
    '  ./target/debug/rdm commit -m "chore(plan): clear needs-plan-review on ' + (kind === 'phase' ? roadmap + '/' + ident : ident) + '"',
    'Return a STAMP_ACK object: { ok: true } if BOTH commands exited 0, otherwise { ok: false }.',
    'Do not retry on failure — report the result of the single attempt.',
  ].join('\n')
}

// buildActPrompt — orchestrator-only act step: apply small plan-body fixes by
// writing the WHOLE --body, and file large findings as tasks. Never runs in
// --implementation-plan mode (guarded at the call site).
function buildActPrompt(kind, roadmap, ident, survivors) {
  return [
    'You are the plan-review orchestrator applying already-verified findings. The findings below already',
    'survived independent refutation — do not re-review; act on them.',
    'Findings (ranked, most-severe first):',
    JSON.stringify(survivors, null, 2),
    'For each finding, decide small vs large:',
    '- SMALL (a localized wording/typo/missing-detail fix to the plan document itself): apply it by reading the',
    '  current body and writing the ENTIRE modified body back — `--body` is whole-document-authoritative, there',
    '  is no patch mechanism. Use the matching command:',
    kind === 'task'
      ? '    ./target/debug/rdm task update ' + ident + ' --body "<full updated body>" --no-edit --project rdm'
      : kind === 'phase'
      ? '    ./target/debug/rdm phase update ' + ident + ' --roadmap ' + roadmap + ' --body "<full updated body>" --no-edit --project rdm'
      : '    ./target/debug/rdm roadmap update ' + ident + ' --body "<full updated body>" --no-edit --project rdm',
    '- LARGE (a structural concern: a missing prerequisite, scope too big for one phase, a conflicting design',
    '  decision): do NOT edit the plan document — file it as a task:',
    '    ./target/debug/rdm task create <slug> --title "Plan review finding: <desc>" --body "<details>" --tags plan-review --no-edit --project rdm',
    'After applying any changes, run: ./target/debug/rdm commit -m "chore(plan): address plan review findings on ' +
      (kind === 'phase' ? roadmap + '/' + ident : ident) +
      '"',
    'If there is nothing small to fix and nothing large to file, make no changes.',
    'Return a STAMP_ACK object: { ok: true } if you completed without error (including the no-op case), else { ok: false }.',
  ].join('\n')
}

const parsed = parsePlanArgs(args)
const kind = parsed.kind

// The plan review IS the canonical pipeline — buildReviewPipeline('plan') from
// the stamped block, with NO independent review logic in this driver. Passing NO
// signals is deliberate (see the header note); phase-only unit-of-work scoping is
// applied per unit via stripNonPhaseUnitOfWork below.
const runPlanReview = buildReviewPipeline('plan')

// reviewUnit — run find → refute → filter for ONE review unit, then strip
// non-phase unit-of-work survivors and classify. Returns a per-unit result the
// act + gate steps consume independently.
async function reviewUnit(unit) {
  const rawSurvivors = await runPlanReview({ target: unit.target })
  const survivors = stripNonPhaseUnitOfWork(rawSurvivors, unit.targetType)
  const outcome = classifyPlanOutcome(survivors)
  return { unit: unit, survivors: survivors, outcome: outcome, summary: summarizeFindings(survivors) }
}

// ------------------------------------------------------------------ implementation-plan
// Report-only: no persisted rdm item, so no act and no gate. stripNonPhaseUnitOfWork
// drops unit-of-work here too (targetType 'implementation-plan' !== 'phase').
if (kind === 'implementation-plan') {
  const planText = parsed.planText || '(the implementation plan provided in context)'
  const rawSurvivors = await runPlanReview({ target: planText })
  const survivors = stripNonPhaseUnitOfWork(rawSurvivors, 'implementation-plan')
  const outcome = classifyPlanOutcome(survivors)
  log('plan-review (implementation-plan): ' + outcome + ' — ' + summarizeFindings(survivors))
  return {
    kind: 'implementation-plan',
    outcome: outcome,
    summary: summarizeFindings(survivors),
    findings: survivors,
  }
}

// ------------------------------------------------------------------ persisted targets
// Fetch the artifact(s) and build the review unit list. A `phase`/`task` target
// is a single unit; a `roadmap` target is the roadmap body plus one unit per
// phase, each gated independently.
let units = []
let fetchFailed = false

if (kind === 'roadmap') {
  let rm = null
  try {
    rm = await agent(buildRoadmapFetchPrompt(parsed.roadmap), {
      label: 'fetch:roadmap',
      phase: 'Read',
      schema: ROADMAP_TARGET_SCHEMA,
    })
  } catch (e) {
    rm = null
  }
  if (!rm || !rm.body || String(rm.body).trim() === '') {
    fetchFailed = true
  } else {
    units.push({
      kind: 'roadmap',
      targetType: 'roadmap',
      ident: parsed.roadmap,
      roadmap: parsed.roadmap,
      tags: Array.isArray(rm.tags) ? rm.tags : [],
      target: 'roadmap ' + parsed.roadmap + ' (body)\n\n' + String(rm.body),
    })
    const phases = Array.isArray(rm.phases) ? rm.phases : []
    for (let i = 0; i < phases.length; i++) {
      const p = phases[i]
      units.push({
        kind: 'phase',
        targetType: 'phase',
        ident: p.stem,
        roadmap: parsed.roadmap,
        tags: Array.isArray(p.tags) ? p.tags : [],
        target: 'phase ' + parsed.roadmap + '/' + p.stem + '\n\n' + String(p.body || ''),
      })
    }
  }
} else {
  // phase or task — a single unit.
  const fetchPrompt =
    kind === 'task' ? buildTaskFetchPrompt(parsed.task) : buildPhaseFetchPrompt(parsed.roadmap, parsed.phase)
  const fetchSchema = PLAN_TARGET_SCHEMA
  let meta = null
  try {
    meta = await agent(fetchPrompt, { label: 'fetch:' + kind, phase: 'Read', schema: fetchSchema })
  } catch (e) {
    meta = null
  }
  if (!meta || !meta.body || String(meta.body).trim() === '') {
    fetchFailed = true
  } else {
    const ident = kind === 'task' ? parsed.task : parsed.phase
    const label = kind === 'task' ? 'task/' + parsed.task : parsed.roadmap + '/' + parsed.phase
    units.push({
      kind: kind,
      targetType: kind,
      ident: ident,
      roadmap: parsed.roadmap,
      tags: Array.isArray(meta.tags) ? meta.tags : [],
      target: kind + ' ' + label + '\n\n' + String(meta.body),
    })
  }
}

// FAIL-CLOSED: an unread plan must NOT be silently marked reviewed / have its
// tag cleared. Report the failure and mutate nothing.
if (fetchFailed) {
  log('plan-review: artifact fetch failed for ' + kind + ' — leaving needs-plan-review in place (fail-closed)')
  return { kind: kind, outcome: 'escalated', fetchError: true, summary: 'plan-review: artifact fetch failed', units: [] }
}

// Review each unit independently (parallel per-unit fan-out — a phase's outcome
// never changes a sibling's). A single phase/task target is a one-element list.
const results = await parallel(units.map((u) => () => reviewUnit(u)))

// Act + gate each unit independently. Both halves are skipped in
// --implementation-plan mode (handled by the early return above); the explicit
// `if (kind !== 'implementation-plan')` guards make that carve-out grep-visible
// and keep the code robust if the flow is ever restructured.
const reported = []
for (let i = 0; i < results.length; i++) {
  const r = results[i]
  if (!r) continue
  const u = r.unit
  const gate = gateFor('plan', r.outcome)

  // --- Act (orchestrator-only; skipped for implementation-plan) ---
  if (kind !== 'implementation-plan' && r.survivors.length > 0) {
    try {
      await agent(buildActPrompt(u.kind, u.roadmap, u.ident, r.survivors), {
        label: 'act:' + u.kind + ':' + u.ident,
        phase: 'Act',
        schema: STAMP_ACK_SCHEMA,
      })
    } catch (e) {
      log('plan-review: act step failed for ' + u.kind + '/' + u.ident + ' — continuing to gate')
    }
  }

  // --- Gate (skipped for implementation-plan) ---
  // On reviewed: read-filter-write the tags to drop needs-plan-review, preserving
  // siblings. On rework/escalated: leave the tag; GATE_POLICY.plan never persists
  // an rdm status (gate.status is a literal null).
  let tagCleared = false
  if (kind !== 'implementation-plan') {
    if (gate.clearsPlanReviewTag) {
      const remaining = filterPlanReviewTag(u.tags)
      try {
        const ack = await agent(buildTagWritePrompt(u.kind, u.roadmap, u.ident, remaining), {
          label: 'gate:clear-tag:' + u.kind + ':' + u.ident,
          phase: 'Gate',
          schema: STAMP_ACK_SCHEMA,
        })
        tagCleared = !!(ack && ack.ok === true)
      } catch (e) {
        log('plan-review: tag-clear failed for ' + u.kind + '/' + u.ident)
      }
    }
  }

  const reason = gate.reasonPrefix ? gate.reasonPrefix + ' ' + r.summary : ''
  reported.push({
    kind: u.kind,
    ident: u.ident,
    roadmap: u.roadmap,
    outcome: r.outcome,
    status: gate.status,
    clearsPlanReviewTag: gate.clearsPlanReviewTag,
    tagCleared: tagCleared,
    reason: reason,
    summary: r.summary,
    findings: r.survivors,
  })
  log('plan-review (' + u.kind + '/' + u.ident + '): ' + r.outcome + ' — ' + r.summary)
}

const result = { kind: kind, units: reported }
if (kind !== 'roadmap' && reported.length === 1) {
  // Flatten a single phase/task target onto the top-level result for convenience.
  result.outcome = reported[0].outcome
  result.summary = reported[0].summary
  result.findings = reported[0].findings
}
log('plan-review (' + kind + '): ' + reported.length + ' unit(s) gated')
return result
