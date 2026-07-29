//! review — the single canonical review implementation for every rdm surface.
//!
//! This module is the **one source of truth** for the whole review:
//! **find → refute → filter → verdict → gate/completion policy**. Every surface
//! consumes it, so an improvement to review lands once and is usable everywhere:
//!
//!   * the autonomous workflow lane — `.claude/workflows/review-refute-fix.js`
//!     and `.claude/workflows/dispatch-phase.js` — receives the marked block
//!     below VERBATIM, stamped by `scripts/gen-workflow-review.sh` (the Claude
//!     Code Workflow runtime cannot `import`/`require`; see
//!     docs/workflow-schemas.md § "Import spike");
//!   * the interactive skill lane — `rdm-core/src/templates/skill-review-*.md`
//!     — receives the literate `//|` prose rendered to markdown by
//!     `scripts/gen-skill-review.sh`.
//!
//! Edit HERE — never in a consumer — then re-run BOTH generators.
//! `scripts/verify-workflow-review.sh` fails on drift in either direction.
//!
//! ## Two marker systems
//!
//! NOTE: the marker tokens are deliberately NOT spelled out in full anywhere in
//! this header — `gen-workflow-review.sh` locates the stamped block by a plain
//! substring match, so an incidental prose mention of its begin token would
//! silently truncate extraction. Read the real marker comments below instead.
//!
//! 1. The **stamped block** (`review-refute-fix` markers). Copied verbatim into
//!    every workflow-script consumer. Self-contained: no imports, no ambient
//!    globals named at module scope, no `Date.now` / `Math.random`. It must
//!    NEVER contain a land-time completion directive literal — the dispatch
//!    harness greps the stamped region for exactly that.
//! 2. The **skill-renderable spec**: a `review-spec` region NESTED inside the
//!    stamped block, and a `review-gate-spec` region OUTSIDE and AFTER it.
//!    `gen-skill-review.sh` emits only the `//| ` literate comment lines from
//!    these two regions, in that order. The nested markers are inert to
//!    `gen-workflow-review.sh` (whose awk matches only the outer token), so they
//!    ride along as harmless comments in the stamped copy.
//!
//! ## Mode tags on the literate prose
//!
//! The ONE spec region pair renders TWO skills — the code-review skill
//! (`--mode code`) and the plan-review skill (`--mode plan`). Each `//|` prose
//! line therefore carries an optional per-line mode tag immediately after the
//! prefix:
//!
//!   * `//| …`      — **shared**: rendered into BOTH modes.
//!   * `//|code| …` — rendered into `--mode code` only.
//!   * `//|plan| …` — rendered into `--mode plan` only.
//!
//! The tag is recognized only as the literal `code|` or `plan|` immediately
//! after `//|`, so shared prose must never begin with the text `code|` or
//! `plan|`. This is an optional prefix on the existing marker system — there is
//! no second region, no second generator, and no second consumer list.
//!
//! Everything after the `review-spec` end marker and inside the stamped block is
//! **machinery**: JSON schemas, `survives`/`rankFindings`, dimension selection,
//! the outcome classifier, and the status mapping. Machinery is never rendered
//! into a skill.
//!
//! The stamped block runs in two contexts unchanged:
//!   1. The Workflow runtime, where `agent`/`pipeline`/`parallel`/`log` are
//!      ambient globals — `buildReviewPipeline(mode)` picks them up by default.
//!   2. Node (this file, imported by the verify harness), where those globals
//!      do not exist — the harness injects fakes via the `deps` argument, so the
//!      pure composition and filtering logic is testable with zero LLM calls.
//!
//! The `export { ... }` at the bottom lives OUTSIDE every marker: it exists only
//! for Node's importer and is never copied into a workflow consumer (a bare
//! `export` mid-body would break a workflow script, whose only permitted export
//! is `meta`).

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

// NON_GATING_SEVERITIES — the severities whose refuter verdict cannot change
// anything, so no refuter is dispatched for them.
//
// DERIVATION (auditable, not a guess): `hasBlocking` below is the ONLY place a
// severity becomes an outcome. Its blocker set is ['blocking'], widened to
// ['blocking', 'concern'] at the `large` tier — so `blocking` always gates and
// `concern` gates at some tier. The one other outcome channel, `acTableHasGap`,
// reads the structured AC table and never consults a finding's severity at all.
// Subtracting the WIDEST blocker set from the severity scale therefore leaves
// exactly `suggestion`, and leaves it non-gating at EVERY tier — which is why
// this set is tier-independent and no tier has to be threaded into the pipeline.
const NON_GATING_SEVERITIES = ['suggestion'];

// needsRefutation(finding) — should this finding get a refuter?
//
// FAIL-SAFE BY CONSTRUCTION: refute UNLESS the severity is *explicitly* listed
// as non-gating. A finding with a missing, misspelled, or newly-invented
// severity is therefore still refuted — the skip is opt-in per severity, never
// a default. (Ranking degrades the same way: an unknown severity sorts last via
// SEVERITY_RANK's `!= null` guard rather than gating.)
function needsRefutation(finding) {
  const severity = finding && finding.severity;
  return NON_GATING_SEVERITIES.indexOf(severity) === -1;
}

// UNREFUTED_DISPOSITION — how an act step must treat a finding that reached it
// WITHOUT a refuter verdict (`unrefuted: true`). Single-sourced here, in the
// stamped block, so every act-step consumer (dispatch-phase's code act step,
// plan-review's plan act step) states one identical rule — the same reason
// `hasBlocking` lives here rather than in each consumer.
const UNREFUTED_DISPOSITION = [
  'Findings marked `unrefuted: true` were **reported, not verified** — no refuter graded them, so treat them as',
  'observations, never as confirmed defects. Incorporate the ones that improve readability or clarity where the',
  'change is **not major**; skip the rest and state why. "Major" means anything that would alter the approach,',
  'widen scope, or touch code outside the diff under review — that is follow-up material, not an in-flight edit.',
].join('\n');

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
//| ### Refute — a FRESH agent per GATING finding, in parallel
//|
//| For every finding whose severity can gate the outcome, dispatch a **separate**
//| read-only refuter. The agent that found an issue is never the agent that
//| confirms it. The refuter starts from the stance *"this is NOT a real issue
//| unless the code proves otherwise"*, reads the actual cited location and its
//| surrounding context, and returns `refuted` (boolean), a corrected `confidence`
//| (0-100), and a rationale.
//|
//| **Non-gating pass-through.** A `suggestion` gates nothing at any tier — the
//| verdict consults only `blocking` (and `concern`, at the `large` tier), and the
//| acceptance-criteria channel never reads a finding's severity at all — so a
//| refuter's verdict on one cannot change the outcome either way. No refuter is
//| dispatched for it. It passes straight through, marked `unrefuted: true`, and
//| is still subject to the confidence floor. `suggestion` is the ONLY severity
//| treated this way, and the rule is fail-safe: a finding whose severity is
//| missing or unrecognized is refuted like a gating one.
//|
//| `concern` is deliberately **not** passed through, even though it does not gate
//| at the default tier. Measured over the whole recorded refuter corpus (989
//| refuters; `scripts/measure-refuter-severity.mjs`, recorded in
//| `docs/token-baseline.json` § `nonGatingRefutationSkip`):
//|
//| | severity | graded | refuted | rate |
//| |---|---:|---:|---:|
//| | blocking | 197 | 75 | 38.1 % |
//| | concern | 522 | 263 | 50.4 % |
//| | suggestion | 236 | 175 | 74.2 % |
//|
//| A `concern` is overturned MORE often than a `blocking` one, so its refuter is
//| doing real work — and it gates outright at the `large` tier. Skipping only
//| `suggestion` drops 239 refuters (24.2 % of all refuters, 20.7 % of refuter
//| tokens) with no severity that can gate losing its counter-check.
//|
//| ### Filter & consolidate
//|
//| - **Drop** any finding a refuter refuted, and any whose post-refutation
//|   confidence is below the confidence floor (70).
//| - A refuter that *crashes* is not proof of refutation — keep such a finding as
//|   un-refuted rather than silently dropping it. It is **not** marked
//|   `unrefuted: true`: that marker means "deliberately never graded", not
//|   "grading failed".
//| - A finding passed through un-refuted carries `unrefuted: true` and faces the
//|   **same confidence floor** as everything else: the refuter is skipped, the
//|   floor is not.
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
          ((found && found.findings) || []).map((f, idx) => () => {
            // NON-GATING PASS-THROUGH. A refuter is dispatched only where its
            // verdict could change something. A `suggestion` gates nothing at
            // any tier (see NON_GATING_SEVERITIES), so grading it burns a whole
            // agent to reach the same outcome either way: pass it straight
            // through, marked `unrefuted: true` so every downstream consumer can
            // tell reported-only from refuter-verified. verdict stays null, so
            // survives() applies the confidence floor to it EXACTLY as it does
            // to a refuted-but-not-killed finding — the floor is not bypassed.
            if (!needsRefutation(f)) {
              return Promise.resolve({
                finding: { ...f, concern: f.concern || dim.key, unrefuted: true },
                verdict: null,
                skipped: true,
              });
            }
            return _agent(refutePrompt(mode, dim, f, ctx), {
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
              // NOT marked `unrefuted` — that marker means "deliberately never
              // graded", and an act step must not treat a crashed gating finding
              // as a mere observation.
              .catch(() => ({ finding: { ...f, concern: f.concern || dim.key }, verdict: null }));
          })
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
    // A pass-through ALSO carries verdict === null, so it must be excluded here
    // or a deliberate non-gating skip would be mis-reported as a refuter crash.
    const refuterErrors = graded.filter((g) => g.verdict === null && !g.skipped).length;
    const passedThrough = graded.filter((g) => g.skipped).length;
    const survivors = graded.filter((g) => survives(g.finding, g.verdict)).map((g) => g.finding);
    _log(
      mode +
        ' review: ' +
        survivors.length +
        '/' +
        graded.length +
        ' finding(s) survived refutation' +
        (refuterErrors ? ' (' + refuterErrors + ' kept un-refuted after a refuter error)' : '') +
        (passedThrough ? ' (' + passedThrough + ' non-gating passed through un-refuted)' : '')
    );
    return { survivors: rankFindings(survivors), acTable: acTable };
  };
}
// >>> review-refute-fix:end <<<

// >>> review-gate-spec:begin (skill-only prose — NEVER stamped into a workflow script) <<<
// This region sits OUTSIDE the stamped block on purpose: it is the only place
// the land-time completion trailer may be named, because the dispatch harness
// forbids that literal anywhere inside a stamped region.
//|
//| ### Act — verified findings by size, un-refuted ones by disposition
//|
//| Report first, then act. Findings reach this step with two different
//| provenances, and they are handled differently:
//|
//| - A finding a refuter **graded and failed to refute** is acted on by SIZE —
//|   small or large, below.
//| - A finding marked `unrefuted: true` was **reported, not verified** — no
//|   refuter graded it (it is a non-gating severity; see § Refute), so treat it
//|   as an observation, never as a confirmed defect. Incorporate the ones that
//|   improve readability or clarity where the change is **not major**; skip the
//|   rest and state why. "Major" means anything that would alter the approach,
//|   widen scope, or touch code outside the diff under review — that is
//|   follow-up material, not an in-flight edit.
//|
//| Never fix or file a finding that carries neither provenance.
//|
//|code| - **Small** — localized, low-risk, no new acceptance criteria (a typo, a
//|code|   missing doc comment, a tightened error message, an extra test). Fix it
//|code|   inline, run the relevant tests, then fold it into the implementation commit.
//|plan| - **Small** — a localized wording, typo, or missing-detail fix to the plan
//|plan|   document itself. Apply it directly: the body is whole-document-authoritative,
//|plan|   so read the current body, apply the change, and write the **entire** modified
//|plan|   body back — there is no patch/diff mechanism.
//|code| - **Large** — new modules, cross-cutting changes, or anything that warrants
//|code|   its own acceptance criterion. Do **NOT** fix inline: file it as a task.
//|plan| - **Large** — a structural concern: a missing prerequisite, scope too big for
//|plan|   one phase, or a conflicting design decision. Do **NOT** edit the plan
//|plan|   document for these: file it as a task.
//|
//| For each finding, state how it was handled (fixed-inline / filed-as-task).
//|plan|
//|plan| In `--implementation-plan` mode the *act* half is skipped entirely — there is
//|plan| no persisted rdm item to write to or file against. Findings are still
//|plan| reported; folding them back into the plan text is left to the caller.
//|code|
//|code| ### Gate — status mapping
//|code|
//|code| The review owns the `needs-review` → `reviewed` gate. Persist the status the
//|code| outcome maps to, for the item's kind:
//|code|
//|code| | Outcome | When | Phase status | Task status | Completion trailer |
//|code| |---|---|---|---|---|
//|code| | **reviewed** | clean, or clean after small fixes | `reviewed` | `reviewed` | write it |
//|code| | **rework** | a fixable defect, or an unmet acceptance criterion | `in-progress` | `in-progress` | do **not** write it |
//|code| | **escalated** | a blocker needing a human decision | `blocked` | `blocked` | do **not** write it |
//|code|
//|code| Tasks and phases map identically — `blocked` is a valid task status, so an
//|code| escalated task is *not* downgraded to `in-progress`. On `escalated`, prefix
//|code| the recorded reason with `[code]` so the blocked queue shows which gate
//|code| escalated it.
//|code|
//|code| Never set the item to `done` directly — that flip is owned by the
//|code| merge-to-main hook.
//|code|
//|code| **The completion trailer.** On `reviewed` only, amend the land-time
//|code| completion trailer into the branch commit; this completes the directive
//|code| deliberately deferred by the finalize step, so the merge-to-main hook flips
//|code| the item `reviewed → done` later. Never hand-type the trailer format — ask rdm
//|code| for it, so the format string has exactly one home:
//|code|
//|code| ```bash
//|code| rdm hook done-line --roadmap <slug> --phase <stem>   # prints: Done: <slug>/<stem>
//|code| rdm hook done-line --task <slug>                     # prints: Done: task/<slug>
//|code| ```
//|code|
//|code| On `rework` and `escalated`, write **no** trailer.
//|plan|
//|plan| ### Gate — clear or leave `needs-plan-review`
//|plan|
//|plan| The plan review owns the reserved `needs-plan-review` tag. It **never**
//|plan| persists an rdm status — the item's status is the implementation lane's to
//|plan| own — and it never writes a land-time completion directive.
//|plan|
//|plan| | Outcome | `needs-plan-review` | rdm status written |
//|plan| |---|---|---|
//|plan| | **reviewed** | cleared | none |
//|plan| | **rework** | left in place | none |
//|plan| | **escalated** | left in place | none |
//|plan|
//|plan| On **reviewed**:
//|plan|
//|plan| 1. Read the target's current tags (the `tags` array is present in every JSON
//|plan|    summary).
//|plan| 2. Filter `needs-plan-review` out of that array by **exact string match**.
//|plan|    This is idempotent: a target that already lacks the tag is a safe no-op.
//|plan| 3. Write the **complete remaining list** back. Tags **replace** the whole list
//|plan|    — there is no remove-one-tag operation — so always read-filter-write, or a
//|plan|    sibling tag (e.g. the reserved `depends-unlanded`) is silently dropped.
//|plan|    When `needs-plan-review` was the only tag, write an **empty** list.
//|plan| 4. Land it with a `chore(plan): clear needs-plan-review on <target>` commit.
//|plan|
//|plan| On **rework** and **escalated**: do **not** touch the tags. `needs-plan-review`
//|plan| is left unchanged in place. State explicitly in the report that the tag was
//|plan| left, and enumerate exactly what must change before the next review pass. On
//|plan| `escalated`, say what human decision is required; prefix a recorded reason
//|plan| with `[plan]` so it is attributable to this gate.
//|plan|
//|plan| Scope of the gate by target type:
//|plan|
//|plan| - **`--roadmap <slug>`** — gate each phase **individually**, and the roadmap
//|plan|   body separately. One phase's `rework` must not hold the tag on phases that
//|plan|   reached `reviewed`, and the roadmap body's own outcome is independent of any
//|plan|   phase's.
//|plan| - **`--implementation-plan`** — **no gate at all.** There is no persisted rdm
//|plan|   item, so there is no tag to clear and nothing to mutate; report the outcome
//|plan|   and findings only.
//|
//| ### Guidelines
//|
//| - Be objective — evaluate against the stated acceptance criteria, not personal
//|   preferences.
//| - Provide specific evidence (file:line, test name) for every finding.
//| - **No finding of a GATING severity is surfaced, fixed, or filed until a
//|   separate refuter agent has failed to refute it.** The finder never grades
//|   its own work. Non-gating `suggestion` findings are the one exception: they
//|   pass through un-refuted, marked `unrefuted: true`, and are acted on under
//|   the disposition rule above rather than fixed as verified defects.
//| - Filter hard: drop refuted findings and anything below 70 confidence. One
//|   strong finding beats five weak ones.
//| - The dispatched sub-agents only review and report — they never modify code.
//|   The orchestrator applies small fixes, and only after refutation or under the
//|   un-refuted disposition rule.
//| - Never fix large changes inline — file them as tasks.
//| - If acceptance criteria are missing or vague, report it as a finding rather
//|   than guessing intent.
// >>> review-gate-spec:end <<<

// Node-only exports for the verify harness. NOT part of the generated block —
// the marker END is above this line, so the generator never copies these.
export {
  CONFIDENCE_FLOOR,
  SEVERITY_RANK,
  NON_GATING_SEVERITIES,
  needsRefutation,
  UNREFUTED_DISPOSITION,
  DIMENSIONS,
  SIGNAL_KEYS,
  OUTCOMES,
  STATUS_MAPPING,
  GATE_POLICY,
  gateFor,
  statusFor,
  writesCompletion,
  selectDimensions,
  deriveSignals,
  findPrompt,
  refutePrompt,
  FINDINGS_SCHEMA,
  VERDICT_SCHEMA,
  AC_ENTRY_SCHEMA,
  AC_REVIEW_SCHEMA,
  survives,
  rankFindings,
  hasBlocking,
  acTableHasGap,
  summarizeFindings,
  codeReviewRounds,
  classifyOutcome,
  DEFAULT_MAX_CODE_REWORK,
  buildReviewPipeline,
  stripNonPhaseUnitOfWork,
  filterPlanReviewTag,
  classifyPlanOutcome,
};
