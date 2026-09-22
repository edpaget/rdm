//! plan-review — the driver core of the standalone plan-review workflow.
//!
//! This is the **single source of truth** for the plan-review DRIVER: argument
//! parsing (`parsePlanArgs`), the pure unit/gate/round builders, and the
//! dependency-injected orchestration (`runPlanReviewDriver`). It dispatches
//! FINDER AND REFUTER AGENTS AND NOTHING ELSE — there are no fetch, act or gate
//! agents here any more. A reviewer that needs a document runs the
//! `rdm ... show --format json` command its prompt names; every write the driver
//! used to make is returned as ready-to-run Bash for the orchestrator to run. Because the
//! Claude Code Workflow runtime cannot `import`/`require` (see
//! docs/workflow-schemas.md § "Import spike"), the marked block below is copied
//! BYTE-IDENTICAL into `.claude/workflows/rdm-wf-plan-review.js`. Unlike the
//! review-refute-fix block — which is stamped by `scripts/gen-workflow-review.sh`
//! — this block is NOT run through the generator (it is unique to the one
//! plan-review consumer); instead `scripts/verify-workflow-review.sh` § 5b-drift
//! gates the two copies for byte-equality. (The pattern was established by the
//! `dispatch-outcome` block and its own harness, both deleted by
//! `agent-orchestrated-dispatch` phase 7; the byte-equality gate above is the
//! surviving instance.)
//!
//! Every side effect the driver reaches is injected through `deps` (agent /
//! parallel / log / runPlanReview), so this block names NO ambient runtime global
//! and the module imports cleanly in Node — a pattern first set by the since-deleted
//! `lib/dispatch-phase.mjs`, and what makes the driver testable at all. The verify harness imports this
//! module and drives `parsePlanArgs` + `runPlanReviewDriver` against a fake
//! agent/parallel harness with ZERO LLM calls.
//!
//! The review CORE the driver consumes — `buildReviewPipeline`,
//! `filterPlanReviewTag`, `classifyPlanOutcome`,
//! `gateFor`, and `summarizeFindings` — lives in `lib/review.mjs`, the canonical
//! review source. In the `.js` consumer those names arrive via the stamped review
//! block (positioned BEFORE this block). In Node they arrive via the import below,
//! which lives OUTSIDE the markers and is re-exported for the harness.

import {
  buildReviewPipeline,
  filterPlanReviewTag,
  classifyPlanOutcome,
  gateFor,
  summarizeFindings,
  resolveRefutationBudget,
  resolveReviewers,
  buildReviewCoverage,
  coverageSummaryClause,
  persistReviewCommands,
  parseCommentHeader,
  shellQuote,
} from './review.mjs';

// >>> plan-review-driver:begin <<<
// Pure + dependency-injected driver logic for the standalone plan-review
// workflow.
//
// This block is the single source of truth in
// .claude/workflows/lib/plan-review.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/rdm-wf-plan-review.js (the Workflow runtime cannot load modules at
// run time). scripts/verify-workflow-review.sh gates the two copies for drift.
// No Date.now / Math.random — pure array/string ops plus injected async deps.
//
// `buildReviewPipeline`, `filterPlanReviewTag`,
// `classifyPlanOutcome`, `gateFor`, `summarizeFindings`, `shellQuote`,
// `resolveRefutationBudget` and `resolveReviewers` are NOT declared here: they belong to the canonical
// review source (lib/review.mjs) and reach this block from the stamped review
// block that precedes it in the workflow consumer (and from the import above in
// Node).

// `hoistedModelsComplete` / `computeMissingModels` are GONE with the
// `model:mechanical` bootstrap agent they guarded. There is no mechanical model
// left to resolve: the ORCHESTRATOR resolves `review-find`/`review-verify` in
// Bash and passes the ids, and an absent id is inert (the agents inherit the
// session model).

// --- Environment args: `rdmBin` and `project` -------------------------------
//
// The CANONICAL contract every engine in this lane implements, adopted verbatim
// rather than re-invented — only the error-message prefix differs from
// lib/estimate.mjs's copy. Canonical write-up: docs/workflow-schemas.md §
// "Environment args: `rdmBin` and `project`".
//
// `persistRdmBin` / `persistProjectFlag` in the stamped review block ABOVE this
// one are the same contract under distinct names (that block is stamped
// verbatim into files that already declare these). These three are this block's
// own, and the two sets never collide.

// resolveRdmBin(value) — resolve the rdm executable to invoke. An ABSENT value
// DEFAULTS to a plain `rdm` on PATH, because a plugin-installed consumer has no
// repo-local build path to pass. A present-but-wrong-TYPE value throws rather
// than silently degrading to PATH. No existence preflight — a plain fallback
// only.
function resolveRdmBin(value) {
  if (typeof value === 'string' && value.trim() !== '') return value
  if (value === undefined || value === null || typeof value === 'string') return 'rdm'
  throw new Error(
    'plan-review: rdmBin must be a string path to the rdm executable (omit it to default to `rdm` on PATH)'
  )
}

// parseProjectArg(value) — validate the OPTIONAL project name. Any falsy value
// means "emit no project flag at all", so rdm's own resolution chain applies
// downstream. The value is interpolated into agent prompts and into returned
// Bash, so whitespace and shell metacharacters are rejected rather than escaped.
function parseProjectArg(value) {
  if (!value) return ''
  if (typeof value !== 'string' || !/^[A-Za-z0-9._-]+$/.test(value)) {
    throw new Error(
      'plan-review: project must be a plain project name matching /^[A-Za-z0-9._-]+$/ (got "' + String(value) + '")'
    )
  }
  return value
}

// projectFlag(cfg) — the ` --project <name>` suffix for a PROJECT-SCOPED
// command, or '' when no project was configured. `rdm commit` is NOT
// project-scoped and deliberately carries none.
function projectFlag(cfg) {
  return cfg && cfg.project ? ' --project ' + cfg.project : ''
}

// planSourceCommand(item, pin, rdmBin, projFlag) — the pinned `rdm review
// source` command named in a plan-mode finder/refuter prompt (see
// `reviewTargetBlock`'s `mode === 'plan'` branch in the review core). Mirrors
// the code-review engine's own `sourceCommand` builder
// (`rdm-wf-review-refute-fix.js`'s driver region) byte-for-byte in shape:
// same flag order, same `shellQuote` on every interpolated value. `--no-code`
// is ALWAYS passed, unconditionally — plan review runs before implementation,
// so an empty committed diff between `base` and `head` is the expected,
// legitimate case here, never a caller mistake the way it is in code mode.
// `rdmBin` and `projFlag` are taken ALREADY RESOLVED (as `resolveRdmBin`/
// `projectFlag` return them), matching the calling convention `buildReviewUnits`
// already uses for its own `RDM`/`PROJ` locals.
function planSourceCommand(item, pin, rdmBin, projFlag) {
  return (
    rdmBin +
    ' review source --on ' +
    shellQuote(item) +
    ' --source ' +
    shellQuote(pin.path) +
    ' --base ' +
    shellQuote(pin.base) +
    ' --expected-head ' +
    shellQuote(pin.head) +
    ' --expected-branch ' +
    shellQuote(pin.branch) +
    ' --no-code' +
    (projFlag || '') +
    ' --format json'
  )
}

// parsePlanArgs(rawArgs) — resolve the four target types from a raw $ARGUMENTS
// flag string, a JSON payload, or a structured object. Returns
// { kind, roadmap, phase, task, planSlug, ... } where kind is one of
// 'task' | 'phase' | 'roadmap' | 'implementation-plan'.
//
// EVERY VALUE HERE IS AN IDENTIFIER OR A SHORT LIST. `planSlug` names the
// `plan/<slug>` document under review; `planFile` names a FREE-FORM plan by
// absolute path; `phases` names which phase stems a roadmap sweep covers; `tags`
// carries the item's current tag list so the gate can write back a filtered one.
// No document body is an argument any more — `planText` is gone with the
// transport negotiation it belonged to, because a judgment agent fetches the
// plan itself from the command its prompt names.
//
// Throws an actionable error when no target can be resolved, when an
// implementation-plan target names NO document at all, when `planSlug` /
// `planFile` are given on a non-implementation-plan target or together, or when
// an explicit `persist.on` disagrees with `plan/<planSlug>`. Those are
// argument-SHAPE throws: each reads only the arguments it was handed.
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

  // THE TWO WAYS TO NAME AN IMPLEMENTATION PLAN, both IDENTIFIERS, never a body.
  //
  // `planSlug` names a persisted `plan/<slug>` rdm document. `planFile` names a
  // FREE-FORM plan — a loose file on disk with no rdm document behind it, which
  // is the case `--implementation-plan` was originally built for and which
  // `scripts/lib/codex-runtime.mjs` is the live consumer of. A path is an
  // identifier exactly like a slug is: the reviewer runs the `cat` its prompt
  // names and reads the plan in its own context. That is the phase's rule, not
  // an exception to it — what the rule bans is a DOCUMENT crossing the argument
  // boundary, and `planText` did exactly that, which is why it is gone.
  //
  // Both are read from a STRUCTURED OBJECT KEY ONLY, deliberately never parsed
  // out of the `$ARGUMENTS` flag string, exactly like `persist` below: a
  // positional target slug must never be able to turn a plan-repo write on, nor
  // name a file to read.
  const planSlug = typeof a.planSlug === 'string' && a.planSlug.trim() !== '' ? a.planSlug.trim() : ''
  const planFile = typeof a.planFile === 'string' && a.planFile.trim() !== '' ? a.planFile.trim() : ''

  // Precedence is fixed and total: implementation-plan wins over everything
  // (it is report-only and has no persisted item), then an explicit task, then
  // a roadmap+phase pair (a single phase), then a bare roadmap (the whole
  // roadmap). A positional `<slug>` with no phase therefore behaves exactly
  // like `--roadmap <slug>`.
  let kind
  if (implementationPlan) kind = 'implementation-plan'
  else if (task) kind = 'task'
  else if (roadmap && phase) kind = 'phase'
  else if (roadmap) kind = 'roadmap'
  else
    throw new Error(
      'plan-review: no target — pass --task <slug>, --roadmap <slug>, <slug> [phase], or --implementation-plan'
    )

  // --- Source pin: the SAME flat arg names the code-review engine takes ------
  // (`source`, `base`, `expectedHead`, `expectedBranch` — see
  // rdm-wf-review-refute-fix.js's driver region), reused rather than a nested
  // shape invented for this engine. Read from STRUCTURED OBJECT KEYS ONLY, like
  // `planSlug`/`planFile`/`reviewers`/`tags` above: a positional target slug
  // must never be able to pin a checkout. NONE-OR-ALL: pinning only some of the
  // four would silently review against an unverified path, so a partial pin
  // throws here, at parse time, before any agent runs — the same stance
  // `resolveRefutationBudget` and the other parse-time validators on this
  // function take.
  const rawSource = typeof a.source === 'string' ? a.source.trim() : ''
  const rawBase = typeof a.base === 'string' ? a.base.trim() : ''
  const rawExpectedHead = typeof a.expectedHead === 'string' ? a.expectedHead.trim() : ''
  const rawExpectedBranch = typeof a.expectedBranch === 'string' ? a.expectedBranch.trim() : ''
  const sourceFields = { source: rawSource, base: rawBase, expectedHead: rawExpectedHead, expectedBranch: rawExpectedBranch }
  const sourceKeyNames = Object.keys(sourceFields)
  const sourceKeysGiven = sourceKeyNames.filter((k) => sourceFields[k] !== '')
  let sourcePin = null
  if (sourceKeysGiven.length > 0) {
    const missing = sourceKeyNames.filter((k) => sourceKeysGiven.indexOf(k) === -1)
    if (missing.length > 0) {
      throw new Error(
        'plan-review: a source pin needs all four of source/base/expectedHead/expectedBranch — missing ' +
          missing.join(', ')
      )
    }
    // The same full-hex-SHA shape code review's `requireSha` enforces.
    if (!/^[0-9a-f]{40,64}$/.test(rawExpectedHead)) {
      throw new Error('plan-review: expectedHead must be a full hex commit id (got "' + rawExpectedHead + '")')
    }
    sourcePin = { path: rawSource, base: rawBase, head: rawExpectedHead, branch: rawExpectedBranch }
  }
  // The `--on <item>` value a pinned `rdm review source` call binds to, derived
  // from the SAME identifiers code review derives it from — `task`, or
  // `roadmap`+`phase` — never a new hoisted key. `null` when neither resolves
  // (a bare roadmap sweep with no single phase, or a task-less implementation
  // plan) — the `buildReviewUnits` path derives each unit's own `--on` from its
  // own `target` instead of reading this field; only the `implementation-plan`
  // branch (which has no unit list) actually consumes it.
  const sourceItem = task ? 'task/' + task : roadmap && phase ? 'phase/' + roadmap + '/' + phase : null
  if (sourcePin && kind === 'implementation-plan' && !sourceItem) {
    throw new Error(
      'plan-review: source is pinned but no item to bind it to — pass task "<slug>" or roadmap "<slug>" + phase ' +
        '"<stem>" alongside implementationPlan'
    )
  }

  // --- Caller-supplied values. Read from STRUCTURED OBJECT KEYS ONLY, never
  // parsed out of the `$ARGUMENTS` flag string, which would let a raw prose
  // target string masquerade as one of them.
  //
  // `fetched` is GONE. It was the hoist that let a caller supply the document
  // body the fetch agent would otherwise transcribe. There is no fetch agent and
  // no transcription: a reviewer reads the document itself. What a caller passes
  // now is what it knows and the engine cannot — which PHASES to sweep, what
  // TAGS the item currently carries, which reviews it has already had.
  const wontFixedTexts = Array.isArray(a.wontFixedTexts) ? a.wontFixedTexts : null
  // The CALLER-SELECTED REVIEWER SET, applied to every review unit in this run.
  // Read from a STRUCTURED key only, exactly like `fetched` below — a positional
  // target slug must never be able to change which reviewers run. Absent (null)
  // means run every plan reviewer; see the review core's `resolveReviewers`.
  // An unrecognised name is dropped silently there and shows in coverage.
  //
  // ONE list for the whole run, deliberately. A roadmap sweep applies it to the
  // roadmap-body unit and to every phase unit alike: a caller that wants
  // `unit-of-work` graded on the phases accepts it running against the body too,
  // and that choice is visible in each unit's `coverage.selected`. A second
  // per-kind key would be a second mechanism for the caller to keep in sync.
  const reviewers = Array.isArray(a.reviewers) ? a.reviewers.filter((r) => typeof r === 'string') : null
  // RESOLVED HERE, at parse time, for the same reason `resolveRefutationBudget`
  // is: the ONE refusal the reviewer-selection design keeps — a set that
  // resolves to NO reviewer at all — has to escape the DRIVER, and inside
  // `reviewUnit` it cannot. `reviewUnit` runs inside a `_parallel` thunk, and
  // `parallel()`'s documented thrown-thunk → null degradation turns that throw
  // into a dropped unit: a mistyped reviewer name reviewed nothing and reported
  // `0 unit(s) reviewed`, indistinguishable from a clean sweep. Resolving before
  // any thunk exists puts the refusal back where it was designed to be — the
  // result is discarded, since each unit resolves its own set from the same
  // list, and every OTHER part of the contract (an unrecognised name dropped
  // silently, no floor on the set's size) is untouched by calling it early.
  resolveReviewers('plan', reviewers)
  // The PHASE STEMS a roadmap sweep covers. The engine does NOT read a roadmap
  // to discover them: the orchestrator ran `rdm roadmap show` itself and says
  // which phases to review. Each entry is `{ stem, tags?, status?, priorReviews? }`
  // — `stem` is required, the rest are what the caller happens to know. With no
  // list, a roadmap target reviews the roadmap document ALONE.
  const phases = Array.isArray(a.phases)
    ? a.phases
        .map((p) => (typeof p === 'string' ? { stem: p } : p))
        .filter((p) => p && typeof p === 'object' && typeof p.stem === 'string' && p.stem.trim() !== '')
    : null
  // The item's CURRENT tag list, for a single-unit target. `--tags` replaces the
  // whole list, so the gate can only write back a filtered copy of a list it was
  // actually given: absent means no gate commands are emitted at all, rather
  // than a `--tags ""` that would silently drop a sibling tag.
  const tags = Array.isArray(a.tags) ? a.tags.filter((t) => typeof t === 'string') : null
  // The reviews already recorded on this target, as `rdm review list --on
  // <ref> --format json` reports them. Feeds the round channel (see
  // priorRoundFromReviews / priorFindingsFromReviews). Absent fails toward
  // round 0, the same stance parseRoundNotes takes on a body with no header.
  const priorReviews = Array.isArray(a.priorReviews) ? a.priorReviews : null
  // The resolved `review-find` / `review-verify` model ids for the judgment
  // sites, threaded into the finder/refuter agent() calls inside
  // buildReviewPipeline. The ORCHESTRATOR resolves them in Bash; an absent id is
  // inert and the agent inherits the session model.
  const findModel = typeof a.findModel === 'string' && a.findModel.trim() !== '' ? a.findModel.trim() : null
  const verifyModel = typeof a.verifyModel === 'string' && a.verifyModel.trim() !== '' ? a.verifyModel.trim() : null
  // Per-unit REFUTATION budget, threaded into every review context below.
  // Read from a STRUCTURED key only (like every other hoist here) and RESOLVED
  // HERE, at parse time — before any agent() call — by the review core's single
  // validator, so an invalid value throws instead of burning tokens. Unset
  // resolves to the core's documented default; `0` is legal and distinct from
  // unset (grade nothing).
  const maxRefutations = resolveRefutationBudget(a.maxRefutations)
  // The two ENVIRONMENT axes: which rdm executable every command this engine
  // builds or names in a prompt invokes, and which project the project-scoped
  // ones are scoped to. Read from STRUCTURED OBJECT KEYS ONLY, never parsed out
  // of the `$ARGUMENTS` flag string — the same rule `persist`, `reviewers` and
  // `planFile` follow, and it matters most here: a positional target slug must
  // never be able to choose which binary runs. RESOLVED HERE, at parse time,
  // like `maxRefutations` above, so an invalid value throws before any agent
  // burns a token. An absent `rdmBin` yields a plain `rdm` on PATH; an absent
  // `project` yields '' and therefore no flag at all, never a default.
  const rdmBin = resolveRdmBin(a.rdmBin)
  const project = parseProjectArg(a.project)
  // The PERSIST switch — record this review as a REAL rdm review (see
  // docs/workflow-schemas.md § "Persisting a review"). Read from a STRUCTURED
  // key only: a positional target slug must never be able to turn writing into
  // the plan repo on. Default OFF. It no longer WRITES anything — it makes the
  // engine return the ladder that would.
  let persist = resolvePersistArg(a.persist)
  // `planSlug` names a REAL persisted rdm document, so every way of using it
  // wrongly is caught HERE, at parse time, before any agent() call — the
  // resolveRefutationBudget precedent. Each throw names both halves of the
  // disagreement so the caller can see which one to change.
  if (planSlug && kind !== 'implementation-plan') {
    throw new Error(
      'plan-review: planSlug ' +
        planSlug +
        ' requires --implementation-plan, but the target resolved to kind ' +
        kind +
        ' — a slug on a roadmap/phase/task target would be silently ignored'
    )
  }
  if (planFile && kind !== 'implementation-plan') {
    throw new Error(
      'plan-review: planFile ' +
        planFile +
        ' requires --implementation-plan, but the target resolved to kind ' +
        kind +
        ' — a file on a roadmap/phase/task target would be silently ignored'
    )
  }
  if (planSlug && planFile) {
    throw new Error(
      'plan-review: planSlug ' +
        planSlug +
        ' and planFile ' +
        planFile +
        ' both name the plan under review — pass exactly one, so the graded document is never in doubt'
    )
  }
  // A REVIEW THAT CAN GRADE NOTHING NEVER RUNS. An implementation-plan target
  // naming neither a slug nor a file has no document any reviewer can reach, and
  // before this throw existed such a run dispatched every reviewer against the
  // literal string "(the implementation plan provided in context)" with nothing
  // in context, then returned `outcome: reviewed`, `coverage.complete: true` — a
  // clean, complete-looking approval of an empty document. Coverage cannot see
  // this (every reviewer DID run), so the refusal belongs here, at parse time,
  // before any agent burns a token. It is the no-target throw above applied to
  // the one target kind that could resolve without naming anything.
  if (kind === 'implementation-plan' && !planSlug && !planFile) {
    throw new Error(
      'plan-review: --implementation-plan names no document — pass planSlug ' +
        '"<slug>" for a persisted plan/<slug>, or planFile "/abs/path/to/plan.md" for a free-form one. ' +
        'Reviewing with neither would grade an empty document and report it clean.'
    )
  }
  if (planSlug) {
    // The dual-supply throw `planText` used to need is above, now between the
    // two IDENTIFIERS rather than between an identifier and a body: whichever
    // one is given, the reviewer reads the document itself, so there is no
    // transcription to disagree with — only which document to read.
    if (persist && typeof persist.on === 'string' && persist.on !== 'plan/' + planSlug) {
      throw new Error(
        'plan-review: persist.on ' + persist.on + ' disagrees with planSlug ' + planSlug + " (expected 'plan/" + planSlug + "')"
      )
    }
  }
  // A free-form `--implementation-plan` (a `planFile`, no `planSlug`) genuinely
  // has nothing to hang a review off — a loose file is not an rdm document and
  // no `review --on` ref names it — so persist is forced off. `planSlug` is what
  // tells the two apart: a plan named by slug IS a first-class persisted rdm
  // document (`plan/<slug>`), and its verdict is recorded there like any other
  // target's. Surfaced as a flag so the driver can log it rather than silently
  // dropping a caller's request.
  const persistIgnored = !!(persist && kind === 'implementation-plan' && !planSlug)
  if (persistIgnored) persist = null

  return {
    kind: kind,
    roadmap: roadmap,
    phase: phase,
    task: task,
    planSlug: planSlug,
    planFile: planFile,
    sourcePin: sourcePin,
    sourceItem: sourceItem,
    phases: phases,
    tags: tags,
    priorReviews: priorReviews,
    wontFixedTexts: wontFixedTexts,
    reviewers: reviewers,
    findModel: findModel,
    verifyModel: verifyModel,
    maxRefutations: maxRefutations,
    rdmBin: rdmBin,
    project: project,
    persist: persist,
    persistIgnored: persistIgnored,
  }
}

// resolvePersistArg(value) — the four legal shapes of the `persist` arg, and a
// THROW on anything else:
//
//   absent / null / false        -> null            (off, the default)
//   true / {}                    -> { on: null }    (on; derive the ref per unit)
//   { on: '<rdm review ref>' }   -> { on: '<ref>' } (on; explicit single target)
//
// Throwing on an unrecognized shape is deliberate: a silent "off" would turn a
// typo into a review that was never recorded, with nothing in the transcript to
// say so.
function resolvePersistArg(value) {
  if (value === undefined || value === null || value === false) return null
  if (value === true) return { on: null }
  if (typeof value === 'object' && !Array.isArray(value)) {
    const on = value.on
    if (on === undefined || on === null) return { on: null }
    if (typeof on === 'string' && on.trim() !== '') return { on: on.trim() }
  }
  throw new Error(
    'plan-review: persist must be omitted, `true`, `{}`, or `{ on: "<rdm review ref>" }` — got ' + JSON.stringify(value)
  )
}

// persistTargetFor(unit, persist, unitCount) — the `rdm review --on` ref for ONE
// review unit. Pure.
//
// An explicit `persist.on` is honored ONLY on a single-unit run. A roadmap
// fan-out reviews N units independently, so collapsing them onto one caller-
// supplied target would merge N independent reviews into one — the ref is
// derived per unit instead.
//
// This is the ONE place a plan-review persist ref is built. The writer treats
// `target` as opaque and never prefixes anything (see review.mjs's
// persistReviewCommands), so a mistake here cannot be repaired downstream.
function persistTargetFor(unit, persist, unitCount) {
  const u = unit || {}
  const count = typeof unitCount === 'number' ? unitCount : 1
  if (persist && typeof persist.on === 'string' && persist.on !== '' && count === 1) return persist.on
  if (u.kind === 'phase') return 'phase/' + u.roadmap + '/' + u.ident
  if (u.kind === 'task') return 'task/' + u.ident
  return 'roadmap/' + u.ident
}

// `gateMode` is GONE, and with it `PLAN_GATE_MODES` / `resolvePlanGateMode`.
// It chose between "the gate agent writes the tag in-run" and "compute the
// action and hand it back". Only the second exists now — the engine has no
// agent that can write — so the choice collapses: EVERY run returns the action,
// and the orchestrator applies it. What used to be the named escalation path
// for a caller too close to the plan is simply how the gate works.

// planGateCommands(kind, roadmap, ident, remainingTags, cfg) — the ONE place the
// gate's two commands are built, consumed by buildGateAction (what the caller
// gets back to run itself). Pure string assembly; no side effects.
//
// `cfg` carries the ENVIRONMENT axes (`{ rdmBin, project }`, as parsePlanArgs
// resolved them). Omitting it yields a plain `rdm` and no project flag, which is
// exactly the contract — never a repo-local build path baked into the emitted
// bytes.
//
// The COMPLETE remaining list (already filtered by filterPlanReviewTag) is
// written back, since `--tags` replaces the whole list; an empty list writes
// `--tags ""`.
//
// BOTH LINES CARRY `|| exit 1`, the emitted-ladder rule this lane applies
// everywhere (see review.mjs's persistReviewCommands and the code engine's
// gateCommands). A caller pastes these into a plain shell with no `set -e`, and
// a refused `update` followed by a successful `commit` would otherwise exit 0
// and report a tag cleared that is still set.
function planGateCommands(kind, roadmap, ident, remainingTags, cfg) {
  const tags = Array.isArray(remainingTags) ? remainingTags : []
  const tagsFlag = tags.length === 0 ? '--tags ""' : '--tags "' + tags.join(',') + '"'
  const label = kind === 'phase' ? roadmap + '/' + ident : ident
  const bin = resolveRdmBin(cfg && cfg.rdmBin)
  const proj = projectFlag(cfg)
  // Defense-in-depth: every `rdm` line an engine emits into a persist/gate
  // ladder carries `< /dev/null`, mirroring review.mjs's
  // `persistReviewCommands` — see its comment. `update`/`commit` don't
  // currently block on stdin (task/phase/roadmap `update` never reads it),
  // but the redirect keeps this ladder safe against the whole class of bug
  // regardless of which `rdm` surface later grows a stdin read.
  let updateCmd
  if (kind === 'task') {
    updateCmd = bin + ' task update ' + ident + ' ' + tagsFlag + ' --no-edit' + proj + ' < /dev/null || exit 1'
  } else if (kind === 'phase') {
    updateCmd = bin + ' phase update ' + ident + ' --roadmap ' + roadmap + ' ' + tagsFlag + ' --no-edit' + proj + ' < /dev/null || exit 1'
  } else {
    updateCmd = bin + ' roadmap update ' + ident + ' ' + tagsFlag + ' --no-edit' + proj + ' < /dev/null || exit 1'
  }
  // `rdm commit` REJECTS a project flag, so it carries none — the allow-list
  // rule lib/estimate.mjs states for the same pair.
  const commitCmd = bin + ' commit -m "chore(plan): clear needs-plan-review on ' + label + '" < /dev/null || exit 1'
  return { updateCmd: updateCmd, commitCmd: commitCmd, tagsFlag: tagsFlag, label: label }
}

// The gate's EVIDENCE PROSE is gone — `buildGateEvidence`, `renderGateEvidence`,
// `gateTwoPartyClause`, `groupUngradedSurvivors`, `UNGRADED_SEVERITIES`,
// `UNGRADED_REASONS` and `buildTagWritePrompt`. Every one of them existed to
// persuade a safety classifier that a MECHANICAL AGENT's tag write was
// authorized. No agent writes the tag any more: the gate returns the two
// commands and the orchestrator runs them, under its own authority, in the
// session the operator invoked. There is nobody left to persuade.

// buildGateAction(unit, gate, cfg) — the DECLARATIVE gate action returned on
// EVERY unit, so a caller can iterate `units[].gateAction` uniformly. `cfg` is
// the `{ rdmBin, project }` pair parsePlanArgs resolved, threaded straight
// through to planGateCommands.
//
// The engine never applies it. `commands` comes from `planGateCommands`, and the
// orchestrator runs those two lines in Bash under its own authority. Three cases:
//
//   * the outcome does not clear the tag (`rework`/`escalated`) →
//     `clearsPlanReviewTag: false` and an EMPTY `commands` array.
//   * the outcome clears it and the caller supplied the unit's CURRENT tags →
//     the read-filter-write pair, carrying the complete remaining list.
//   * the outcome clears it and the caller supplied NO tags → still no commands,
//     and `tagsUnknown: true` says why. `--tags` replaces the whole list, so
//     writing one this engine was never shown would silently drop a sibling tag
//     such as `depends-unlanded`. Refusing to guess is the only safe branch, and
//     it is visible rather than silent.
function buildGateAction(unit, gate, cfg) {
  const u = unit || {}
  const g = gate || {}
  const cached = Array.isArray(u.tags) ? u.tags : null
  const clears = g.clearsPlanReviewTag === true
  const remaining = cached === null ? [] : filterPlanReviewTag(cached)
  const emit = clears && cached !== null
  const cmds = planGateCommands(u.kind, u.roadmap, u.ident, remaining, cfg)
  return {
    kind: u.kind,
    ident: u.ident,
    roadmap: u.roadmap,
    clearsPlanReviewTag: clears,
    tagsUnknown: clears && cached === null,
    remainingTags: remaining,
    removedTags: cached === null ? [] : cached.filter((t) => remaining.indexOf(t) === -1),
    commands: emit ? [cmds.updateCmd, cmds.commitCmd] : [],
  }
}

// `gateFailureClause` is gone. It marked a gate that ATTEMPTED the tag write and
// did not succeed. Nothing attempts it here any more, so a blocked gate is not a
// state this engine can be in; an orchestrator whose own `rdm ... update` exits
// nonzero reports that itself, with the shell's message.

// gatePendingClause(reportedUnit) — the marker that makes a unit whose tag the
// CALLER must still clear self-describing in its summary line, without being
// reported as a failure. Empty on every other unit.
//
// It carries the literal commands rather than only pointing at
// `gateAction.commands`, because a caller that only ever reads `summary` (a log
// line, a chat message) would otherwise have to go find the JSON to act.
//
// QUOTING HAZARD — read before reusing this clause anywhere. Unlike
// `coverageSummaryClause`, which is documented as quote-free precisely BECAUSE
// it is interpolated into Bash prompts, this clause embeds an exact rdm command
// containing double quotes (`--tags "a,b"`). It must therefore NEVER be
// interpolated into a prompt: in plan mode `summary`/`reason` are RETURNED DATA,
// and no prompt builder in this file reads either.
function gatePendingClause(reportedUnit) {
  const u = reportedUnit || {}
  const action = u.gateAction || {}
  if (action.clearsPlanReviewTag !== true) return ''
  if (action.tagsUnknown === true) {
    return (
      ' [gate pending: this unit is reviewed, but its current tag list was not supplied, so no ' +
      'needs-plan-review clear could be written out — re-run with the tags, or clear it by hand]'
    )
  }
  const cmds = Array.isArray(action.commands) ? action.commands : []
  return ' [gate pending: needs-plan-review is cleared by running — ' + cmds.join(' && ') + ']'
}

// `buildActPrompt` is gone with the `act:*` agent. Applying a small plan fix and
// filing a large finding as a task are judgment PLUS a write, which is exactly
// what the orchestrator is: it reads the returned findings and acts. The
// disposition rule for an un-refuted finding still travels — it is
// `UNREFUTED_DISPOSITION` in the review core, rendered into every review skill.

// --- Round-capping helpers (bounds repeated plan-review passes on one item) --
// A ROUND AUDIT NOTE is rendered for a non-`reviewed` unit after each pass,
// following the shipped `## Estimate <difficulty> — <justification>` body-note
// convention: a `## Plan Review Round <N> — <outcome>` header followed by one
// bullet per surviving finding. `formatRoundNote` below only RENDERS it — the
// engine returns it to the caller as `roundNote` for a human-readable log, and
// its inverse `parseRoundNotes` is never called from `reviewUnit`. Neither
// function feeds the round channel; the driver's own round state comes solely
// from `unit.priorReviews`, described below.
//
// IMPORTANT: repeat-filtering below is REPORTING-ONLY. It thins what gets
// written to the audit note / shown to a human so an unresolved complaint
// is not re-litigated verbatim every round — it must NEVER be used to decide
// the round's outcome. Rounds 1 and 2 both classify from the FULL (wont-fix-
// suppressed, repeat-UNfiltered) survivor set, so a finding that is still
// genuinely present and blocking keeps the plan in rework/escalated on round
// 2 exactly as it would on round 1 — it cannot silently "age out" into a pass
// purely by being repeated. Only an actual fix (the finder stops reporting
// it) or an explicit human `wont-fix` removes a finding from the outcome.

// --- The review-derived round channel ----------------------------------------
// The ONLY input to the round cap: `unit.priorReviews`, shaped into
// `{ round, findings }` by `priorRoundFromReviews`/`priorFindingsFromReviews`
// below. This channel is read UNCONDITIONALLY by `reviewUnit` — regardless of
// whether THIS pass sets `persist` — so `round = prior.round + 1`,
// `classifyRoundOutcome(round, survivors)`, and `partitionRepeats(survivors,
// prior.findings)` engage as soon as at least one PRIOR pass persisted a
// review for the target. No prior persisted review ⇒ `priorReviews` is a
// caller-supplied EMPTY ARRAY ⇒ round 1, the same "fails toward round 0"
// stance `priorRoundFromReviews` documents below — there is no second,
// body-note-derived channel.
//
// An empty array is not the only way `unit.priorReviews` can be missing,
// though, and the two are NOT the same claim: `[]` says "the caller ran `rdm
// review list` and there were none" (genuinely round 1); a bare absence
// (`null` — the caller never supplied the key, e.g. skipped `rdm review
// list` entirely) says "the caller does not know" and must not be reported as
// the same thing. `reviewUnit` tells them apart via `roundUnknown`
// (`!Array.isArray(unit.priorReviews)`) and reports it visibly — see
// `roundUnknownClause` below, which mirrors `gatePendingClause`'s
// `tagsUnknown` treatment of the same absent-vs-empty distinction on the tag
// channel. The round STILL reports 1 either way (no arithmetic changes), but
// only the genuinely-empty case reports it silently.

// `extractPriorReviewsFromTranscript` is gone with the transcript it read. The
// prior reviews now arrive as CALLER DATA (`priorReviews` per unit): the
// orchestrator runs `rdm review list --on <target> --format json` in Bash and
// passes the parsed array. The two functions below read that array exactly as
// they read the extracted one, so the round channel is unchanged.

// priorRoundFromReviews(reviews) — how many rounds this target has already
// been through: every non-draft review recorded against it. A null/unparseable
// list fails TOWARD 0 (the cap engages later, never never) — the same stance
// parseRoundNotes takes on a body with no well-formed header. This function
// itself stays silent about WHY it returned 0 — a genuinely empty `[]` and a
// missing/non-array `reviews` both land here — because the caller
// (`reviewUnit`) is the one that computes `roundUnknown` separately and
// surfaces the absent case; this helper's job is only the count.
//
// A HUMAN's review on the same target counts. That is deliberate: a round is a
// pass over the plan, whoever made it, and filtering by author would let an
// agent loop forever alongside a human who keeps requesting changes.
function priorRoundFromReviews(reviews) {
  if (!Array.isArray(reviews)) return 0
  return reviews.filter((r) => r && r.state !== 'draft').length
}

// latestPriorReview(reviews) — the most recently created non-draft review, by
// `created` then `id` (both are timestamp-ordered), never by array position.
function latestPriorReview(reviews) {
  if (!Array.isArray(reviews)) return null
  const considered = reviews.filter((r) => r && r.state !== 'draft')
  if (considered.length === 0) return null
  let best = considered[0]
  for (let i = 1; i < considered.length; i++) {
    const key = (r) => String((r && (r.created || r.id)) || '')
    if (key(considered[i]) > key(best)) best = considered[i]
  }
  return best
}

// priorFindingsFromReviews(reviews) — the LATEST prior review's comments, mapped
// back into the `{ severity, concern, what_fails }` shape `partitionRepeats`
// consumes, through the writer's own inverse parser (`parseCommentHeader`) so
// the two cannot drift. A comment whose body does not carry the header — a
// human-written one — is SKIPPED, never crashed on and never signature-matched
// as a repeat.
function priorFindingsFromReviews(reviews) {
  const latest = latestPriorReview(reviews)
  if (!latest) return []
  const comments = Array.isArray(latest.comments) ? latest.comments : []
  const out = []
  for (let i = 0; i < comments.length; i++) {
    const c = comments[i]
    const h = parseCommentHeader(c && c.body)
    if (!h) continue
    out.push({ severity: h.severity, concern: h.dimension, what_fails: h.whatFails })
  }
  return out
}

// roundUnknownClause(reportedUnit) — the marker that makes a unit whose round
// could not be determined self-describing in its summary line, mirroring
// `gatePendingClause`'s `tagsUnknown` branch above: a caller-absent input
// (here `priorReviews`, there the current tag list) is reported visibly
// rather than silently taking the same fallback value a genuinely-empty input
// would. Empty whenever `roundUnknown` is not `true` (including a healthy
// `priorReviews: []` — a caller that looked and found no prior reviews really
// is on round 1), so a run that supplied its prior reviews renders
// byte-unchanged. Present regardless of outcome — unlike `gatePendingClause`,
// which only ever fires on a `reviewed` unit, a round that could not be
// determined is worth flagging on every outcome, since `round` (still
// reported as 1, per the "fails toward round 0" stance) feeds the round-cap
// arithmetic for every later pass too.
function roundUnknownClause(reportedUnit) {
  const u = reportedUnit || {}
  if (u.roundUnknown !== true) return ''
  return (
    ' [round unknown: no priorReviews were supplied for this target, so round ' +
    (typeof u.round === 'number' ? u.round : 1) +
    ' could not be verified against the target\'s actual review history — pass `rdm review list --on <target> ' +
    '--format json` as priorReviews, or treat this pass as round 1 deliberately]'
  )
}

const ROUND_HEADER_RE = /^## Plan Review Round (\d+) — (\S+)\s*$/

// parseRoundNotes(body) — read every well-formed `## Plan Review Round N —
// outcome` block already present in a fetched body and return the LAST
// (highest-numbered) one as { round, outcome, findings }, where findings is
// the parsed bullet list of { severity, concern, what_fails } for that round.
// Returns { round: 0, outcome: null, findings: [] } when no well-formed
// header is found — this fails TOWARD round 1 (the cap engages later, not
// never), never toward silently skipping the cap on a body that happens to
// contain unrelated text resembling the header.
function parseRoundNotes(body) {
  const text = typeof body === 'string' ? body : ''
  const lines = text.split('\n')
  let best = { round: 0, outcome: null, findings: [] }
  let current = null
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const m = ROUND_HEADER_RE.exec(line)
    if (m) {
      const round = parseInt(m[1], 10)
      if (Number.isFinite(round) && round > 0 && round > best.round) {
        current = { round: round, outcome: m[2], findings: [] }
        best = current
      } else {
        current = null // malformed, duplicate, or lower-numbered — ignore its body
      }
      continue
    }
    if (!current) continue
    // Every severity the WRITER can emit must round-trip through the READER.
    // formatRoundNote renders whatever severity a survivor carries, so a reader
    // that whitelists only two of the three silently truncates a note's bullet
    // list at its first `suggestion` — and non-gating pass-through makes a
    // surviving `suggestion` the common case rather than a rarity.
    const bm = /^- \[(blocking|concern|suggestion)\] ([^:]+): (.*)$/.exec(line)
    if (bm) {
      current.findings.push({ severity: bm[1], concern: bm[2], what_fails: bm[3] })
    } else if (line.trim() !== '' && line.slice(0, 3) !== '## ') {
      // Any other non-bullet, non-blank, non-heading content ends this round's
      // bullet capture (conservative: do not keep scanning past unrelated prose).
      current = null
    }
  }
  return best
}

// formatRoundNote(round, outcome, findings) — pure: render the audit-note
// block text (no surrounding blank lines — the caller joins with '\n\n').
function formatRoundNote(round, outcome, findings) {
  const list = Array.isArray(findings) ? findings : []
  const lines = ['## Plan Review Round ' + round + ' — ' + outcome]
  if (list.length === 0) {
    lines.push('- (no surviving findings)')
  } else {
    for (let i = 0; i < list.length; i++) {
      const f = list[i] || {}
      lines.push('- [' + (f.severity || 'concern') + '] ' + (f.concern || 'general') + ': ' + (f.what_fails || f.id || ''))
    }
  }
  return lines.join('\n')
}

// normalizeWords(text) — lowercase, strip punctuation, split into significant
// (length > 3) words. A deterministic string op, not a real fuzzy-matching
// library — used only by the two heuristics below.
function normalizeWords(text) {
  const s = String(text || '')
    .toLowerCase()
    .replace(/[^a-z0-9\s]/g, ' ')
  return s.split(/\s+/).filter((w) => w.length > 3)
}

// findingSignature(finding) — the deterministic text used for repeat
// detection: concern plus normalized what_fails words.
function findingSignature(finding) {
  const concern = (finding && finding.concern) || ''
  const what = (finding && (finding.what_fails || finding.id)) || ''
  return concern + '::' + normalizeWords(what).join(' ')
}

// partitionRepeats(survivors, priorFindings) — REPORTING-ONLY split into
// { repeats, fresh } by exact signature match against the prior round's
// recorded findings. Never used to decide the outcome (see header note): a
// false negative here (a repeat wrongly treated as fresh) only re-lists
// something in the note, it never changes pass/fail.
function partitionRepeats(survivors, priorFindings) {
  const list = Array.isArray(survivors) ? survivors : []
  const prior = Array.isArray(priorFindings) ? priorFindings : []
  const priorSigs = prior.map(findingSignature)
  const repeats = []
  const fresh = []
  for (let i = 0; i < list.length; i++) {
    const f = list[i]
    const isRepeat = priorSigs.indexOf(findingSignature(f)) !== -1
    ;(isRepeat ? repeats : fresh).push(f)
  }
  return { repeats: repeats, fresh: fresh }
}

// wontFixOverlapMatches(finding, wontFixedTexts) — a deterministic, pure,
// conservative token-overlap heuristic. `rdm search` already did the real
// typo-tolerant fuzzy matching on the fetch side to produce the wont-fixed
// candidate list; this is a SECOND, stricter gate applied client-side. A
// finding is only suppressed when a large majority of its significant words
// appear in a candidate wont-fixed task's text AND at least
// WONTFIX_MIN_OVERLAP_WORDS of them do — biased toward under-suppressing,
// since a false suppress removes a live finding from BOTH the report and the
// outcome, while a false miss only re-reports something already dismissed.
const WONTFIX_OVERLAP_RATIO = 0.7
const WONTFIX_MIN_OVERLAP_WORDS = 3
function wontFixOverlapMatches(finding, wontFixedTexts) {
  const findingWords = normalizeWords((finding && (finding.what_fails || finding.id)) || '')
  if (findingWords.length < WONTFIX_MIN_OVERLAP_WORDS) return false
  const findingSet = new Set(findingWords)
  const list = Array.isArray(wontFixedTexts) ? wontFixedTexts : []
  for (let i = 0; i < list.length; i++) {
    const textWords = new Set(normalizeWords(list[i]))
    let overlap = 0
    findingSet.forEach((w) => {
      if (textWords.has(w)) overlap++
    })
    if (overlap >= WONTFIX_MIN_OVERLAP_WORDS && overlap / findingSet.size >= WONTFIX_OVERLAP_RATIO) return true
  }
  return false
}

// suppressWontFixed(survivors, wontFixedTexts) — drop any survivor matching an
// already-wont-fixed task. Removes it from consideration ENTIRELY: both the
// report and the outcome (a human already explicitly overruled it) — unlike
// repeat-filtering above, which is reporting-only.
function suppressWontFixed(survivors, wontFixedTexts) {
  const list = Array.isArray(survivors) ? survivors : []
  if (!Array.isArray(wontFixedTexts) || wontFixedTexts.length === 0) return list.slice()
  return list.filter((f) => !wontFixOverlapMatches(f, wontFixedTexts))
}

// classifyRoundOutcome(round, survivors) — the round-outcome capper. EVERY
// round classifies from the FULL (wont-fix-suppressed but repeat-unfiltered)
// survivor set via classifyPlanOutcome, exactly as an uncapped run would.
//
// ONE CHANNEL, ONE CLASSIFIER. The prior `{ round, findings }` state comes
// solely from `unit.priorReviews` — the reviews already persisted on the
// target, supplied by the caller — regardless of whether THIS pass sets
// `persist`. There is no second, body-note-derived channel: `formatRoundNote`/
// `parseRoundNotes` render/parse a human-readable audit note only and never
// feed back into this function. This function, its input (the repeat-
// UNFILTERED survivor list), and the reporting-only rule on `partitionRepeats`
// are the only path, so a repeat can never age a still-present blocking
// finding out into a pass. `unit.priorReviews` absent (not merely empty) is a
// SEPARATE signal — `roundUnknown`, computed and reported by `reviewUnit`,
// not by this function — that this pass's round number could not actually be
// verified; it still classifies here as round 1, the same as a genuinely
// empty prior set.
// Round 3+ then escalates only when that base outcome is still non-`reviewed`,
// so an item can never loop forever on an unresolved finding — while a plan
// that was genuinely fixed on the third pass still passes. The cap is an
// anti-loop valve, not a penalty for having needed three rounds: escalating a
// clean survivor list would send a human a plan with nothing left to decide.
function classifyRoundOutcome(round, survivors) {
  const base = classifyPlanOutcome(survivors)
  if (round >= 3 && base !== 'reviewed') return 'escalated'
  return base
}

// `buildWontFixFetchPrompt`, `buildRoundNoteWritePrompt`,
// `assembleRoadmapFetchFromTranscript` and `WONTFIX_LIST_SCHEMA` are gone.
//
// The wont-fix list is one `rdm search "" --tag plan-review --status wont-fix
// --type task` the orchestrator runs; it passes the titles as `wontFixedTexts`,
// which was always the caller-suppliable path and is now the only one.
//
// The round note is no longer WRITTEN by this engine either. `formatRoundNote`
// above still renders the block — it is pure — and the driver returns it on the
// unit as `roundNote`; appending it to the item body is a read-modify-write, so
// it belongs to the orchestrator like every other write.

// TERMINAL_PHASE_STATUSES / isTerminalPhaseStatus(status) — the roadmap-wide
// sweep (buildReviewUnits' roadmap branch below) excludes a phase whose
// status is EXACTLY `done` or `wont-fix` from the review/act/gate pipeline
// entirely: there is no implementation left to vet, and clearing
// `needs-plan-review` on a retired phase would assert something untrue about
// it (task plan-review-skips-terminal-phases, "Why it matters beyond cost").
//
// FAIL-OPEN by construction: `isTerminalPhaseStatus` is an exact string match
// against a small, closed list. Anything else — a missing status, an empty
// string, a typo, a future status value this list has not caught up to —
// evaluates false, so the phase STAYS in the fan-out. A silently-narrowed
// sweep is the same failure shape as a silently-skipped review dimension; this
// filter is only ever allowed to add a skip when it is certain, never to
// infer one from absence. No caller needs to special-case a missing
// `p.status` separately — this one predicate covers it.
//
// DECISION (task plan-review-skips-terminal-phases, "Suggested direction" §3):
// this filter applies ONLY to the roadmap-wide sweep. An explicitly-targeted
// single phase (`--roadmap <slug> <phase>` / positional `<slug> <phase>`) or
// task is always reviewed regardless of status — the phase/task branch of
// buildReviewUnits below never reads a status field at all, so it is
// structurally exempt rather than exempted by a conditional here.
const TERMINAL_PHASE_STATUSES = ['done', 'wont-fix']
function isTerminalPhaseStatus(status) {
  return typeof status === 'string' && TERMINAL_PHASE_STATUSES.indexOf(status) !== -1
}

// formatSkippedPhasesClause(skippedPhases) — the human-visible terminal-phase-
// skip clause, appended to the roadmap sweep's aggregate summary and final log
// line by runPlanReviewDriver. Empty string when nothing was skipped, so a
// sweep with no terminal phases renders byte-unchanged — following the same
// discipline as formatUnitBudget/coverageSummaryClause. A skip must never be
// silent: this is the one place the human-visible surfaces name it; the same
// list is also returned machine-readably as `result.skippedPhases`.
function formatSkippedPhasesClause(skippedPhases) {
  const list = Array.isArray(skippedPhases) ? skippedPhases : []
  if (list.length === 0) return ''
  return (
    ' — skipped ' +
    list.length +
    ' terminal phase(s) from the sweep: ' +
    list.map((p) => p.stem + ' (' + p.status + ')').join(', ')
  )
}

// buildReviewUnits(parsed) — pure: turn a parsed target into the list of
// independent review units. A `phase`/`task` target is a single unit; a
// `roadmap` target is the roadmap body plus one unit per CALLER-SUPPLIED,
// non-terminal phase stem, each gated independently. Returns
// `{ units, skippedPhases }`.
//
// IT READS NOTHING AND NEEDS NOTHING READ. Each unit carries IDENTIFIERS only:
// the ref the reviewers review (`target`), the `rdm ... show --format json`
// commands they run themselves (`itemCommand`, and `roadmapCommand` where a
// parent roadmap exists and its `## Intent` is relevant), the tag list the
// caller supplied for the gate, and the prior reviews the caller supplied for
// the round channel. There is no body here to be empty, so the old fail-closed
// "an unread plan must never be silently marked reviewed" branch has nothing
// left to guard: an unreadable document now fails inside the reviewer that
// tried to read it, and shows up as a missing dimension in `coverage`.
//
// WITH NO `phases` LIST a roadmap target reviews the roadmap document ALONE.
// The engine does not read a roadmap to discover its phases — the orchestrator
// says which to sweep, because it is the one that read the roadmap.
function buildReviewUnits(parsed) {
  // The ENVIRONMENT axes, as parsePlanArgs resolved them. Re-resolved here (not
  // read raw) so a caller driving this pure builder directly still gets the
  // documented fallbacks rather than `undefined` spliced into a command.
  const RDM = resolveRdmBin(parsed && parsed.rdmBin)
  const PROJ = projectFlag(parsed)
  const roadmapCommand = parsed.roadmap ? RDM + ' roadmap show ' + parsed.roadmap + PROJ + ' --format json' : null
  if (parsed.kind === 'roadmap') {
    const units = [
      {
        kind: 'roadmap',
        ident: parsed.roadmap,
        roadmap: parsed.roadmap,
        tags: Array.isArray(parsed.tags) ? parsed.tags : null,
        priorReviews: parsed.priorReviews,
        target: 'roadmap/' + parsed.roadmap,
        itemCommand: roadmapCommand,
        roadmapCommand: roadmapCommand,
      },
    ]
    const phases = Array.isArray(parsed.phases) ? parsed.phases : []
    const skippedPhases = []
    for (let i = 0; i < phases.length; i++) {
      const p = phases[i]
      // FAIL-OPEN: only an exact `done`/`wont-fix` skips. A missing or unknown
      // status keeps the phase in the sweep — a silently narrowed sweep is the
      // same failure shape as a silently skipped reviewer.
      if (isTerminalPhaseStatus(p.status)) {
        skippedPhases.push({ stem: p.stem, status: p.status })
        continue
      }
      const phaseTarget = 'phase/' + parsed.roadmap + '/' + p.stem
      units.push({
        kind: 'phase',
        ident: p.stem,
        roadmap: parsed.roadmap,
        tags: Array.isArray(p.tags) ? p.tags : null,
        priorReviews: Array.isArray(p.priorReviews) ? p.priorReviews : null,
        target: phaseTarget,
        itemCommand: RDM + ' phase show ' + p.stem + ' --roadmap ' + parsed.roadmap + PROJ + ' --format json',
        roadmapCommand: roadmapCommand,
        // Each phase unit binds the pin to ITS OWN target — never a shared or
        // wrong stem — so a roadmap sweep's phases each verify their own
        // checkout independently. The bare roadmap-body unit above gets none:
        // `rdm review source` requires a phase or task item and rejects a
        // roadmap (`resolve_review_source` in rdm-core/src/worktree.rs).
        sourceCommand: parsed.sourcePin ? planSourceCommand(phaseTarget, parsed.sourcePin, RDM, PROJ) : null,
      })
    }
    return { units: units, skippedPhases: skippedPhases }
  }
  // phase or task — a single unit. Never reads a status field: an
  // explicitly-targeted single phase or task is reviewed whatever its status,
  // structurally exempt from the terminal-phase filter rather than exempted by a
  // conditional.
  const isTask = parsed.kind === 'task'
  const ident = isTask ? parsed.task : parsed.phase
  const unitTarget = isTask ? 'task/' + ident : 'phase/' + parsed.roadmap + '/' + ident
  return {
    skippedPhases: [],
    units: [
      {
        kind: parsed.kind,
        ident: ident,
        roadmap: parsed.roadmap,
        tags: Array.isArray(parsed.tags) ? parsed.tags : null,
        priorReviews: parsed.priorReviews,
        target: unitTarget,
        itemCommand: isTask
          ? RDM + ' task show ' + ident + PROJ + ' --format json'
          : RDM + ' phase show ' + ident + ' --roadmap ' + parsed.roadmap + PROJ + ' --format json',
        // A task has no parent roadmap, so no intent to inherit.
        roadmapCommand: isTask ? null : roadmapCommand,
        sourceCommand: parsed.sourcePin ? planSourceCommand(unitTarget, parsed.sourcePin, RDM, PROJ) : null,
      },
    ],
  }
}

// `snapshotOriginalTags` is gone. It cached tags off a FETCHED payload so the
// gate write could not observe a value that had taken a detour through the
// review machinery. Nothing fetches tags now: the CALLER supplies them per unit
// (`tags`, and `phases[].tags` on a roadmap sweep), having read them itself, and
// the gate writes back exactly that list filtered. A unit whose tags the caller
// did not supply gets NO gate commands at all rather than a `--tags ""` that
// would silently drop its siblings.

// formatUnitBudget(budget) — the visible per-unit refutation-budget clause,
// appended to a unit's log line ONLY when the bound was actually hit. A unit
// that stayed under budget logs a byte-unchanged line, so a bounded run can
// never be mistaken for complete coverage and an unbounded one reads exactly as
// it did before.
function formatUnitBudget(budget) {
  if (!budget || budget.hit !== true) return ''
  return (
    ' [review budget hit: ' +
    budget.produced +
    ' produced, ' +
    budget.graded +
    ' graded, ' +
    budget.passedThroughBudget +
    ' ungraded]'
  )
}

// runPlanReviewDriver(args, deps) — the full plan-review orchestration.
//
// IT DISPATCHES FINDER AND REFUTER AGENTS AND NOTHING ELSE. It reads no rdm
// document, resolves no model and performs no write: every read it used to make
// through a mechanical agent is now a command NAMED in a reviewer's prompt for
// that reviewer to run itself, and every write it used to make is returned as
// ready-to-run Bash for the orchestrator to run.
//
// Side effects reach it only through the injected `deps`:
//   deps.agent          — passed straight through to the review pipeline.
//   deps.parallel       — the per-unit fan-out primitive.
//   deps.log            — the log sink (optional; defaults to a no-op).
//   deps.runPlanReview  — an async runReview(context) from buildReviewPipeline
//                         ('plan'); optional — built from the review core when
//                         omitted (the Workflow runtime path).
//   deps.findModel / deps.verifyModel — judgment-site model ids; caller args win.
//
// Returns the structured result the caller reports:
//   - implementation-plan: { kind, outcome, summary, findings } plus, with a
//     `planSlug` and `persist` on, the `persistCommands`/`persistScript` ladder.
//     No gate: a plan document carries no tags.
//   - persisted targets: { kind, units:[…] } with a single phase/task target
//     also flattened onto { outcome, summary, findings, gateAction, … }.
async function runPlanReviewDriver(args, deps) {
  const d = deps || {}
  const _parallel = d.parallel
  const _log = d.log || function () {}
  let _findModel = d.findModel
  let _verifyModel = d.verifyModel
  // The plan review IS the canonical pipeline — buildReviewPipeline('plan') from
  // the review core, with NO independent review logic in this driver. Which
  // reviewers run is the CALLER's choice, threaded through as `reviewers`.
  const runPlanReview = d.runPlanReview || buildReviewPipeline('plan')

  const parsed = parsePlanArgs(args)
  const kind = parsed.kind
  if (parsed.findModel) _findModel = parsed.findModel
  if (parsed.verifyModel) _verifyModel = parsed.verifyModel
  // Already validated by parsePlanArgs via the review core's single validator.
  const maxRefutations = parsed.maxRefutations
  const reviewers = parsed.reviewers
  const persist = parsed.persist
  const persistOn = !!persist
  if (parsed.persistIgnored) {
    _log('plan-review: --implementation-plan with no planSlug has no persisted target — persist ignored')
  }
  // The wont-fix list is the ORCHESTRATOR's to supply: it is the one that can
  // run `rdm search "" --tag plan-review --status wont-fix --type task`. Absent
  // means suppress nothing, which is the safe direction — a finding that was
  // already dismissed is re-reported rather than a live one being hidden.
  const wontFixedTexts = Array.isArray(parsed.wontFixedTexts) ? parsed.wontFixedTexts : []

  // planPersistCommands(persistTarget, outcome, survivors) — THE ONE WRITER that
  // turns a plan-mode verdict into the exact command ladder recording it as a
  // REAL rdm review, so an agent's review is the same artifact a human's is. It
  // RUNS NOTHING: the ladder is returned on the unit result and the orchestrator
  // pastes it into Bash. A writer that throws (an unrepresentable target) logs
  // and yields null, leaving the outcome and the gate exactly as they were — a
  // review that could not be DESCRIBED is a lost audit trail, never a changed
  // verdict.
  function planPersistCommands(persistTarget, outcome, survivors) {
    try {
      return persistReviewCommands(
        { mode: 'plan', outcome: outcome, survivors: survivors },
        persistTarget,
        { rdmBin: parsed.rdmBin, project: parsed.project }
      )
    } catch (e) {
      _log('plan-review: could not build the persist ladder for ' + persistTarget + ' (' + String((e && e.message) || e) + ')')
      return null
    }
  }

  // reviewUnit — run find → refute → filter for ONE review unit, drop anything
  // already resolved wont-fix, read the unit's prior round off the reviews the
  // caller supplied, and classify with the round cap.
  async function reviewUnit(unit) {
    // `acTable` is always null in plan mode and is discarded. `budget` describes
    // the PIPELINE, not this unit's final reported findings — suppressWontFixed
    // runs after it and may drop a survivor that consumed budget.
    const { survivors: rawSurvivors, budget, coverage } = await runPlanReview({
      target: unit.target,
      itemCommand: unit.itemCommand,
      roadmapCommand: unit.roadmapCommand,
      sourceCommand: unit.sourceCommand,
      reviewers: reviewers,
      maxRefutations: maxRefutations,
      findModel: _findModel,
      verifyModel: _verifyModel,
    })
    const survivors = suppressWontFixed(rawSurvivors, wontFixedTexts)
    // `roundUnknown` distinguishes a caller who supplied no `priorReviews` at
    // all (`null` — cannot know the round) from one who supplied `[]` (looked,
    // found none — genuinely round 1). Computed here, off the RAW
    // `unit.priorReviews`, rather than inside `priorRoundFromReviews` — that
    // helper's contract stays "how many rounds", the same non-array-fails-
    // toward-0 shape it always had; this is a second, independent read of the
    // same input for visibility only. See roundUnknownClause and the
    // "review-derived round channel" comment block above.
    const roundUnknown = !Array.isArray(unit.priorReviews)
    const prior = { round: priorRoundFromReviews(unit.priorReviews), findings: priorFindingsFromReviews(unit.priorReviews) }
    const round = prior.round + 1
    const outcome = classifyRoundOutcome(round, survivors)
    const partition = partitionRepeats(survivors, prior.findings)
    return {
      unit: unit,
      survivors: survivors,
      outcome: outcome,
      round: round,
      roundUnknown: roundUnknown,
      newlyReported: partition.fresh,
      repeats: partition.repeats,
      budget: budget || null,
      coverage: coverage || null,
      // A reviewer that did not participate is named in the SUMMARY STRING, not
      // only in the machine-readable `coverage` key, so a plan review that ran
      // 2 of 3 reviewers can never read like a complete one. Empty on a complete
      // run, so a healthy unit's summary is byte-unchanged.
      summary: summarizeFindings(survivors) + coverageSummaryClause(buildReviewCoverage([coverage], null)),
    }
  }

  // ------------------------------------------------------------------ implementation-plan
  // THE GRADED DOCUMENT IS THE PLAN ITSELF, read by the reviewers from the
  // command their prompt names — `plan show <planSlug>` for a persisted plan,
  // `cat <planFile>` for a free-form one. Exactly one of the two is set: parse
  // time refuses both and refuses neither, so there is always a document and it
  // is never ambiguous which. No ITEM document is reachable from this branch at
  // all, so a finding about a phase body — one the plan does not inherit — is
  // impossible by construction rather than by instruction. The persist ref is
  // DERIVED from the same `planSlug`, so the graded document and the recorded
  // verdict cannot name different documents.
  //
  // No act step and no gate: a plan document carries no tags, so there is no
  // `needs-plan-review` to clear.
  if (kind === 'implementation-plan') {
    const slug = parsed.planSlug
    const file = parsed.planFile
    const planTarget = slug ? 'plan/' + slug : 'the implementation plan at ' + file
    // parsePlanArgs already refused a pin with no `sourceItem` to bind it to
    // (source is pinned but neither `task` nor `roadmap`+`phase` was given), so
    // by construction `parsed.sourceItem` is non-null whenever `sourcePin` is.
    const sourceCommand = parsed.sourcePin
      ? planSourceCommand(parsed.sourceItem, parsed.sourcePin, parsed.rdmBin, projectFlag(parsed))
      : null
    const { survivors: rawSurvivors, budget, coverage } = await runPlanReview({
      target: planTarget,
      itemCommand: slug
        ? parsed.rdmBin + ' plan show ' + slug + projectFlag(parsed) + ' --format json'
        : 'cat -- ' + shellQuote(file),
      roadmapCommand: parsed.roadmap
        ? parsed.rdmBin + ' roadmap show ' + parsed.roadmap + projectFlag(parsed) + ' --format json'
        : null,
      sourceCommand: sourceCommand,
      reviewers: reviewers,
      maxRefutations: maxRefutations,
      findModel: _findModel,
      verifyModel: _verifyModel,
    })
    const survivors = suppressWontFixed(rawSurvivors, wontFixedTexts)
    // classifyPlanOutcome, NOT classifyRoundOutcome: the round cap stays out of
    // this mode, where the bound is the orchestrator's own revise budget.
    const outcome = classifyPlanOutcome(survivors)
    const planSummary = summarizeFindings(survivors) + coverageSummaryClause(buildReviewCoverage([coverage], null))
    _log('plan-review (implementation-plan): ' + outcome + ' — ' + planSummary + formatUnitBudget(budget))
    const planResult = {
      kind: 'implementation-plan',
      outcome: outcome,
      summary: planSummary,
      budget: budget || null,
      coverage: coverage || null,
      findings: survivors,
    }
    // PRESENT ONLY WHEN THEY EXIST, so a free-form run's returned shape stays
    // minimal and the documented "no gateAction / gate keys at all" contract for
    // this kind still holds. Exactly one of the two is ever set, and it names
    // the document the reviewers were told to read — so a caller reading the
    // result can always say WHAT was graded.
    if (slug) planResult.planSlug = slug
    if (file) planResult.planFile = file
    if (slug && persistOn) {
      const ladder = planPersistCommands('plan/' + slug, outcome, survivors)
      if (ladder) {
        planResult.persistCommands = ladder
        planResult.persistScript = ladder.join('\n')
      }
    }
    return planResult
  }

  // ------------------------------------------------------------------ persisted targets
  const built = buildReviewUnits(parsed)
  const units = built.units
  // The phases the roadmap-wide sweep excluded as terminal (done/wont-fix) —
  // always an array. Reported on `result.skippedPhases`, the aggregate
  // `result.summary` and the final log line; never dropped silently.
  const skippedPhases = built.skippedPhases || []

  // Review each unit independently (parallel per-unit fan-out — a phase's
  // outcome never changes a sibling's). A single phase/task target is a
  // one-element list.
  const results = await _parallel(units.map((u) => () => reviewUnit(u)))

  const reported = []
  // Units whose thunk yielded nothing. `parallel()` degrades a thrown thunk to
  // null, so a unit can vanish here for a reason this driver never sees. It is
  // NOT "nothing to report": a sweep that lost every unit would otherwise read
  // exactly like one that reviewed every unit cleanly. Named on the result and
  // in the summary so the two can never be confused again.
  const failedUnits = []
  for (let i = 0; i < results.length; i++) {
    const r = results[i]
    if (!r) {
      const u = units[i]
      const ident = u ? u.kind + '/' + u.ident : 'unit ' + (i + 1)
      failedUnits.push(ident)
      _log('plan-review (' + ident + '): the review produced no result — this unit was NOT reviewed')
      continue
    }
    const u = r.unit
    const gate = gateFor('plan', r.outcome)
    const gateAction = buildGateAction(u, gate, parsed)

    // The persist ladder for this unit, BUILT and returned, never run.
    const unitPersist = persistOn
      ? planPersistCommands(persistTargetFor(u, persist, units.length), r.outcome, r.survivors)
      : null

    // `reason` derives from the UNDECORATED summary: only `escalated` carries a
    // reasonPrefix in plan mode, and the gate clause only ever attaches to a
    // `reviewed` unit, so the two never collide.
    const reason = gate.reasonPrefix ? gate.reasonPrefix + ' ' + r.summary : ''
    const reportedUnit = {
      kind: u.kind,
      ident: u.ident,
      roadmap: u.roadmap,
      outcome: r.outcome,
      round: r.round,
      roundUnknown: r.roundUnknown,
      newlyReported: r.newlyReported,
      repeats: r.repeats,
      status: gate.status,
      clearsPlanReviewTag: gate.clearsPlanReviewTag,
      gateAction: gateAction,
      reason: reason,
      summary: r.summary,
      budget: r.budget || null,
      coverage: r.coverage || null,
      findings: r.survivors,
    }
    // The ROUND AUDIT NOTE, rendered and returned rather than written: appending
    // it to the item body is a read-modify-write, which belongs to the
    // orchestrator like every other write. Present only on a non-`reviewed`
    // outcome, which is when a round is worth recording — including a round with
    // zero surviving findings, since a round-3 escalation must still be capped.
    if (r.outcome !== 'reviewed') {
      reportedUnit.roundNote = formatRoundNote(r.round, r.outcome, r.survivors)
    }
    if (unitPersist) {
      reportedUnit.persistCommands = unitPersist
      reportedUnit.persistScript = unitPersist.join('\n')
    }
    // Clause order is FIXED: summarizeFindings → coverage clause (inside
    // r.summary) → round-unknown clause → gate clause.
    reportedUnit.summary = r.summary + roundUnknownClause(reportedUnit) + gatePendingClause(reportedUnit)
    reported.push(reportedUnit)
    _log('plan-review (' + u.kind + '/' + u.ident + '): ' + r.outcome + ' — ' + reportedUnit.summary + formatUnitBudget(r.budget))
  }

  // How many units are waiting on the caller to run their gate commands. Always
  // present (0, never undefined) so a caller summing across runs never gets
  // `undefined` at the moment it is deciding whether a tag was left set.
  const gatePendingCount = reported.filter((x) => x.gateAction && x.gateAction.clearsPlanReviewTag === true).length
  const skippedClause = formatSkippedPhasesClause(skippedPhases)
  // Attached to every summary a unit-failure can reach, so "reviewed nothing"
  // can never present as "found nothing". Empty on a healthy run, so a healthy
  // run's summary is byte-unchanged.
  const failedClause = failedUnits.length
    ? ' [' + failedUnits.length + ' unit(s) NOT reviewed: ' + failedUnits.join(', ') + ']'
    : ''
  const result = {
    kind: kind,
    units: reported,
    gatePendingCount: gatePendingCount,
    skippedPhases: skippedPhases,
    // Always present (an array, never undefined) for the same reason
    // `gatePendingCount` always is: a caller deciding whether a sweep covered
    // its phases must not have to tell absent from zero.
    failedUnits: failedUnits,
  }
  if (kind !== 'roadmap' && reported.length === 1) {
    // Flatten a single phase/task target onto the top-level result.
    result.outcome = reported[0].outcome
    result.summary = reported[0].summary + failedClause
    result.budget = reported[0].budget
    result.coverage = reported[0].coverage
    result.findings = reported[0].findings
    result.gateAction = reported[0].gateAction
    if (reported[0].roundNote) result.roundNote = reported[0].roundNote
    if (reported[0].persistCommands) {
      result.persistCommands = reported[0].persistCommands
      result.persistScript = reported[0].persistScript
    }
  }
  if (kind === 'roadmap') {
    result.summary = 'plan-review (roadmap): ' + reported.length + ' unit(s) reviewed' + skippedClause + failedClause
  }
  // A single-unit target that produced NO reported unit has no flattened
  // summary of its own, and would otherwise return with none at all. Give it
  // the failure, so the one shape where the loss is total is also the one that
  // says so loudest.
  if (!result.summary && failedUnits.length) {
    result.summary = 'plan-review (' + kind + '): 0 unit(s) reviewed' + skippedClause + failedClause
  }
  _log(
    'plan-review (' + kind + '): ' + reported.length + ' unit(s) reviewed' +
      (gatePendingCount > 0
        ? ' — ' + gatePendingCount + ' gate(s) pending (run units[].gateAction.commands)'
        : '') +
      skippedClause +
      failedClause
  )
  return result
}

// >>> plan-review-driver:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  resolveRdmBin,
  parseProjectArg,
  projectFlag,
  planSourceCommand,
  parsePlanArgs,
  buildReviewUnits,
  runPlanReviewDriver,
  planGateCommands,
  buildGateAction,
  gatePendingClause,
  roundUnknownClause,
  resolvePersistArg,
  persistTargetFor,
  priorRoundFromReviews,
  priorFindingsFromReviews,
  parseRoundNotes,
  formatRoundNote,
  findingSignature,
  partitionRepeats,
  suppressWontFixed,
  wontFixOverlapMatches,
  classifyRoundOutcome,
  formatUnitBudget,
  TERMINAL_PHASE_STATUSES,
  isTerminalPhaseStatus,
  formatSkippedPhasesClause,
};
