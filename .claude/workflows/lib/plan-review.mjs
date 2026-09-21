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
// `classifyPlanOutcome`, `gateFor`, `summarizeFindings`,
// `resolveRefutationBudget` and `resolveReviewers` are NOT declared here: they belong to the canonical
// review source (lib/review.mjs) and reach this block from the stamped review
// block that precedes it in the workflow consumer (and from the import above in
// Node).

// `hoistedModelsComplete` / `computeMissingModels` are GONE with the
// `model:mechanical` bootstrap agent they guarded. There is no mechanical model
// left to resolve: the ORCHESTRATOR resolves `review-find`/`review-verify` in
// Bash and passes the ids, and an absent id is inert (the agents inherit the
// session model).

// parsePlanArgs(rawArgs) — resolve the four target types from a raw $ARGUMENTS
// flag string, a JSON payload, or a structured object. Returns
// { kind, roadmap, phase, task, planSlug, ... } where kind is one of
// 'task' | 'phase' | 'roadmap' | 'implementation-plan'.
//
// EVERY VALUE HERE IS AN IDENTIFIER OR A SHORT LIST. `planSlug` names the
// `plan/<slug>` document under review; `phases` names which phase stems a
// roadmap sweep covers; `tags` carries the item's current tag list so the gate
// can write back a filtered one. No document body is an argument any more —
// `planText` is gone with the transport negotiation it belonged to, because a
// judgment agent fetches the plan itself from the command its prompt names.
//
// Throws an actionable error when no target can be resolved, when `planSlug` is
// given on a non-implementation-plan target, or when an explicit `persist.on`
// disagrees with `plan/<planSlug>`. Those are argument-SHAPE throws: each reads
// only the arguments it was handed.
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

  // The SLUG of the persisted plan document under review — the ONE identifier
  // key that survives the retired planText/planSlug transport negotiation. Read
  // from a STRUCTURED OBJECT KEY ONLY, deliberately never parsed out of the
  // `$ARGUMENTS` flag string, exactly like `persist` below: a positional target
  // slug must never be able to turn a plan-repo write on. Empty string when
  // absent, which is the free-form review-what-is-in-context case.
  const planSlug = typeof a.planSlug === 'string' && a.planSlug.trim() !== '' ? a.planSlug.trim() : ''

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
  if (planSlug) {
    if (kind !== 'implementation-plan') {
      throw new Error(
        'plan-review: planSlug ' +
          planSlug +
          ' requires --implementation-plan, but the target resolved to kind ' +
          kind +
          ' — a slug on a roadmap/phase/task target would be silently ignored'
      )
    }
    // The dual-supply throw that used to live here is gone with `planText`.
    // There is no second way to supply the plan, so there is no disagreement
    // left to catch: `planSlug` names the document, the reviewer reads it, and
    // the verdict is recorded on that same ref.
    if (persist && typeof persist.on === 'string' && persist.on !== 'plan/' + planSlug) {
      throw new Error(
        'plan-review: persist.on ' + persist.on + ' disagrees with planSlug ' + planSlug + " (expected 'plan/" + planSlug + "')"
      )
    }
  }
  // A free-form `--implementation-plan` with no `planSlug` genuinely has nothing
  // to hang a review off, so persist is forced off. `planSlug` is what tells the
  // two apart: a plan named by slug IS a first-class persisted rdm document
  // (`plan/<slug>`), and its verdict is recorded there like any other target's.
  // Surfaced as a flag so the driver can log it rather than silently dropping a
  // caller's request.
  const persistIgnored = !!(persist && kind === 'implementation-plan' && !planSlug)
  if (persistIgnored) persist = null

  return {
    kind: kind,
    roadmap: roadmap,
    phase: phase,
    task: task,
    planSlug: planSlug,
    phases: phases,
    tags: tags,
    priorReviews: priorReviews,
    wontFixedTexts: wontFixedTexts,
    reviewers: reviewers,
    findModel: findModel,
    verifyModel: verifyModel,
    maxRefutations: maxRefutations,
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

// planGateCommands(kind, roadmap, ident, remainingTags) — the ONE place the
// gate's two commands are built. Both buildTagWritePrompt (what the mechanical
// agent is told to run) and buildGateAction (what the caller gets back to run
// itself) consume it, so a prompt and a returned action can never print
// divergent commands. Pure string assembly; no side effects.
//
// The COMPLETE remaining list (already filtered by filterPlanReviewTag) is
// written back, since `--tags` replaces the whole list; an empty list writes
// `--tags ""`.
function planGateCommands(kind, roadmap, ident, remainingTags) {
  const tags = Array.isArray(remainingTags) ? remainingTags : []
  const tagsFlag = tags.length === 0 ? '--tags ""' : '--tags "' + tags.join(',') + '"'
  const label = kind === 'phase' ? roadmap + '/' + ident : ident
  let updateCmd
  if (kind === 'task') {
    updateCmd = './target/debug/rdm task update ' + ident + ' ' + tagsFlag + ' --no-edit --project rdm'
  } else if (kind === 'phase') {
    updateCmd = './target/debug/rdm phase update ' + ident + ' --roadmap ' + roadmap + ' ' + tagsFlag + ' --no-edit --project rdm'
  } else {
    updateCmd = './target/debug/rdm roadmap update ' + ident + ' ' + tagsFlag + ' --no-edit --project rdm'
  }
  const commitCmd = './target/debug/rdm commit -m "chore(plan): clear needs-plan-review on ' + label + '"'
  return { updateCmd: updateCmd, commitCmd: commitCmd, tagsFlag: tagsFlag, label: label }
}

// The gate's EVIDENCE PROSE is gone — `buildGateEvidence`, `renderGateEvidence`,
// `gateTwoPartyClause`, `groupUngradedSurvivors`, `UNGRADED_SEVERITIES`,
// `UNGRADED_REASONS` and `buildTagWritePrompt`. Every one of them existed to
// persuade a safety classifier that a MECHANICAL AGENT's tag write was
// authorized. No agent writes the tag any more: the gate returns the two
// commands and the orchestrator runs them, under its own authority, in the
// session the operator invoked. There is nobody left to persuade.

// buildGateAction(unit, gate) — the DECLARATIVE gate action returned on EVERY
// unit, so a caller can iterate `units[].gateAction` uniformly.
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
function buildGateAction(unit, gate) {
  const u = unit || {}
  const g = gate || {}
  const cached = Array.isArray(u.tags) ? u.tags : null
  const clears = g.clearsPlanReviewTag === true
  const remaining = cached === null ? [] : filterPlanReviewTag(cached)
  const emit = clears && cached !== null
  const cmds = planGateCommands(u.kind, u.roadmap, u.ident, remaining)
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
// A ROUND AUDIT NOTE is appended to a non-`reviewed` unit's body after each
// pass, following the shipped `## Estimate <difficulty> — <justification>`
// body-note convention: a `## Plan Review Round <N> — <outcome>` header
// followed by one bullet per surviving finding. Reading it back on the next
// pass tells the driver which round it is on and what was already reported,
// with no external state.
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
// The persisted-review replacement for parseRoundNotes. Same `{ round, findings }`
// shape, so everything downstream — `round = prior.round + 1`,
// `classifyRoundOutcome(round, survivors)`, `partitionRepeats(survivors,
// prior.findings)` — is literally unchanged and the round-3 cap and the
// reporting-only repeat rule are preserved BY CONSTRUCTION rather than by a
// second implementation.

// `extractPriorReviewsFromTranscript` is gone with the transcript it read. The
// prior reviews now arrive as CALLER DATA (`priorReviews` per unit): the
// orchestrator runs `rdm review list --on <target> --format json` in Bash and
// passes the parsed array. The two functions below read that array exactly as
// they read the extracted one, so the round channel is unchanged.

// priorRoundFromReviews(reviews) — how many rounds this target has already
// been through: every non-draft review recorded against it. A null/unparseable
// list fails TOWARD 0 (the cap engages later, never never) — the same stance
// parseRoundNotes takes on a body with no well-formed header.
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
// TWO CHANNELS, ONE CLASSIFIER. The prior `{ round, findings }` state now comes
// from EITHER the `## Plan Review Round` body note (persist off) or the reviews
// persisted on the target (persist on) — and that is the ONLY difference between
// them. The channel supplies `{ round, findings }` and nothing else; this
// function, its input (the repeat-UNFILTERED survivor list), and the
// reporting-only rule on `partitionRepeats` are identical on both. A repeat can
// therefore never age a still-present blocking finding out into a pass on either
// channel.
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
  const RDM = './target/debug/rdm'
  const PROJ = ' --project rdm'
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
      units.push({
        kind: 'phase',
        ident: p.stem,
        roadmap: parsed.roadmap,
        tags: Array.isArray(p.tags) ? p.tags : null,
        priorReviews: Array.isArray(p.priorReviews) ? p.priorReviews : null,
        target: 'phase/' + parsed.roadmap + '/' + p.stem,
        itemCommand: RDM + ' phase show ' + p.stem + ' --roadmap ' + parsed.roadmap + PROJ + ' --format json',
        roadmapCommand: roadmapCommand,
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
  return {
    skippedPhases: [],
    units: [
      {
        kind: parsed.kind,
        ident: ident,
        roadmap: parsed.roadmap,
        tags: Array.isArray(parsed.tags) ? parsed.tags : null,
        priorReviews: parsed.priorReviews,
        target: isTask ? 'task/' + ident : 'phase/' + parsed.roadmap + '/' + ident,
        itemCommand: isTask
          ? RDM + ' task show ' + ident + PROJ + ' --format json'
          : RDM + ' phase show ' + ident + ' --roadmap ' + parsed.roadmap + PROJ + ' --format json',
        // A task has no parent roadmap, so no intent to inherit.
        roadmapCommand: isTask ? null : roadmapCommand,
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
        { rdmBin: './target/debug/rdm', project: 'rdm' }
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
      reviewers: reviewers,
      maxRefutations: maxRefutations,
      findModel: _findModel,
      verifyModel: _verifyModel,
    })
    const survivors = suppressWontFixed(rawSurvivors, wontFixedTexts)
    const prior = { round: priorRoundFromReviews(unit.priorReviews), findings: priorFindingsFromReviews(unit.priorReviews) }
    const round = prior.round + 1
    const outcome = classifyRoundOutcome(round, survivors)
    const partition = partitionRepeats(survivors, prior.findings)
    return {
      unit: unit,
      survivors: survivors,
      outcome: outcome,
      round: round,
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
  // `plan show <planSlug>` command their prompt names. No ITEM document is
  // reachable from this branch at all, so a finding about a phase body — one the
  // plan does not inherit — is impossible by construction rather than by
  // instruction. The persist ref is DERIVED from the same `planSlug`, so the
  // graded document and the recorded verdict cannot name different documents.
  //
  // No act step and no gate: a plan document carries no tags, so there is no
  // `needs-plan-review` to clear.
  if (kind === 'implementation-plan') {
    const slug = parsed.planSlug
    const planTarget = slug ? 'plan/' + slug : '(the implementation plan provided in context)'
    const { survivors: rawSurvivors, budget, coverage } = await runPlanReview({
      target: planTarget,
      itemCommand: slug ? './target/debug/rdm plan show ' + slug + ' --project rdm --format json' : null,
      roadmapCommand: parsed.roadmap
        ? './target/debug/rdm roadmap show ' + parsed.roadmap + ' --project rdm --format json'
        : null,
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
    // PRESENT ONLY WHEN THEY EXIST, so a free-form (no-slug) run's returned
    // shape stays minimal and the documented "no gateAction / gate keys at all"
    // contract for this kind still holds.
    if (slug) planResult.planSlug = slug
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
    const gateAction = buildGateAction(u, gate)

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
    // r.summary) → gate clause.
    reportedUnit.summary = r.summary + gatePendingClause(reportedUnit)
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
  parsePlanArgs,
  buildReviewUnits,
  runPlanReviewDriver,
  planGateCommands,
  buildGateAction,
  gatePendingClause,
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
