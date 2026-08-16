//! plan-review — the driver core of the standalone plan-review workflow.
//!
//! This is the **single source of truth** for the plan-review DRIVER: argument
//! parsing (`parsePlanArgs`), the mechanical fetch/act/gate prompt builders, and
//! the dependency-injected orchestration (`runPlanReviewDriver`). Because the
//! Claude Code Workflow runtime cannot `import`/`require` (see
//! docs/workflow-schemas.md § "Import spike"), the marked block below is copied
//! BYTE-IDENTICAL into `.claude/workflows/rdm-wf-plan-review.js`. Unlike the
//! review-refute-fix block — which is stamped by `scripts/gen-workflow-review.sh`
//! — this block is NOT run through the generator (it is unique to the one
//! plan-review consumer); instead `scripts/verify-workflow-review.sh` gates the
//! two copies for byte-equality, exactly as `scripts/verify-workflow-dispatch.sh`
//! gates the sibling `dispatch-outcome` block.
//!
//! Every side effect the driver reaches is injected through `deps` (agent /
//! parallel / log / runPlanReview), so this block names NO ambient runtime global
//! and the module imports cleanly in Node — the lib/dispatch-phase.mjs precedent,
//! which is what makes the driver testable at all. The verify harness imports this
//! module and drives `parsePlanArgs` + `runPlanReviewDriver` against a fake
//! agent/parallel harness with ZERO LLM calls.
//!
//! The review CORE the driver consumes — `buildReviewPipeline`,
//! `stripNonPhaseUnitOfWork`, `filterPlanReviewTag`, `classifyPlanOutcome`,
//! `gateFor`, and `summarizeFindings` — lives in `lib/review.mjs`, the canonical
//! review source. In the `.js` consumer those names arrive via the stamped review
//! block (positioned BEFORE this block). In Node they arrive via the import below,
//! which lives OUTSIDE the markers and is re-exported for the harness.

import {
  UNREFUTED_DISPOSITION,
  buildReviewPipeline,
  stripNonPhaseUnitOfWork,
  filterPlanReviewTag,
  classifyPlanOutcome,
  gateFor,
  summarizeFindings,
  resolveRefutationBudget,
  buildReviewCoverage,
  coverageSummaryClause,
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
// `buildReviewPipeline`, `stripNonPhaseUnitOfWork`, `filterPlanReviewTag`,
// `classifyPlanOutcome`, `gateFor`, `summarizeFindings`, and
// `resolveRefutationBudget` are NOT declared here: they belong to the canonical
// review source (lib/review.mjs) and reach this block from the stamped review
// block that precedes it in the workflow consumer (and from the import above in
// Node).

// parsePlanArgs(rawArgs) — resolve the four target types from a raw $ARGUMENTS
// flag string, a JSON payload, or a structured object. Returns
// { kind, roadmap, phase, task, planText } where kind is one of
// 'task' | 'phase' | 'roadmap' | 'implementation-plan'. Throws an actionable
// error when no target can be resolved.
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

  // --- Optional caller-supplied hoists (see docs/mechanical-agent-inventory.md).
  // Read from STRUCTURED OBJECT KEYS ONLY — deliberately never parsed out of the
  // `$ARGUMENTS` flag string, which would let a raw prose target string
  // masquerade as a fetched payload. Every one is OPTIONAL: absent or malformed
  // falls through to the in-workflow agent, which is what a direct `Workflow`
  // invocation (and, today, every DISTRIBUTED caller of this workflow) does.
  //
  // `fetched` is the priority hoist: the fetch agents it replaces have twice
  // transcribed junk over real plan tags in production (runs wf_e3402021-0af and
  // wf_f4be8027-dbb), and `agent(..., { schema })` provably cannot catch that —
  // both corrupt returns were schema-valid. Passing the parsed
  // `rdm ... show --format json` through `args` removes the transcription step
  // entirely. NOTE: validating the CONTENT of a hoisted payload is deliberately
  // NOT done here — that belongs to task fix-plan-review-gate-tag-clobber.
  const fetched = a.fetched && typeof a.fetched === 'object' ? a.fetched : null
  const wontFixedTexts = Array.isArray(a.wontFixedTexts) ? a.wontFixedTexts : null
  const mechanicalModel =
    typeof a.mechanicalModel === 'string' && a.mechanicalModel.trim() !== '' ? a.mechanicalModel.trim() : null
  // The judgment-site siblings of mechanicalModel above: the resolved
  // `review-find`/`review-verify` model ids, threaded into the finder/refuter
  // agent() calls inside buildReviewPipeline (see docs/refuter-model-tiering.md
  // § "The rdm-wf-plan-review.js model-omission question" — this was an adjudicated
  // oversight, not a policy choice, and is fixed by this hoist).
  const findModel = typeof a.findModel === 'string' && a.findModel.trim() !== '' ? a.findModel.trim() : null
  const verifyModel = typeof a.verifyModel === 'string' && a.verifyModel.trim() !== '' ? a.verifyModel.trim() : null
  // Per-unit REFUTATION budget, threaded into every review context below.
  // Read from a STRUCTURED key only (like every other hoist here) and RESOLVED
  // HERE, at parse time — before any agent() call — by the review core's single
  // validator, so an invalid value throws instead of burning tokens. Unset
  // resolves to the core's documented default; `0` is legal and distinct from
  // unset (grade nothing).
  const maxRefutations = resolveRefutationBudget(a.maxRefutations)

  return {
    kind: kind,
    roadmap: roadmap,
    phase: phase,
    task: task,
    planText: planText,
    fetched: fetched,
    wontFixedTexts: wontFixedTexts,
    mechanicalModel: mechanicalModel,
    findModel: findModel,
    verifyModel: verifyModel,
    maxRefutations: maxRefutations,
  }
}

// hoistedFetchedOk(fetched, kind) — the shape guard on a caller-supplied target
// payload. It stands in for the { body, tags, phases } shape buildReviewUnits
// consumes (the same shape the fetch agents below now ASSEMBLE, driver-side,
// from a raw transcript — see RAW_STDOUT_SCHEMA), so it must be no weaker than
// that shape: a non-empty `body` (buildReviewUnits' own fail-closed condition)
// AND a `tags` array of strings, plus — for the roadmap kind — an array
// `phases` whose every entry carries a non-empty string `stem`, a string
// `body`, and its own `tags` array of strings.
//
// `tags` is required, not optional-with-a-default, because it is WRITTEN BACK:
// on a `reviewed` outcome the gate issues `rdm ... update --tags "<list>"`, and
// `--tags` replaces the whole list. Accepting a payload with no `tags` would let
// buildReviewUnits default it to `[]` and the gate would then clobber every real
// tag the item carried. Anything this guard rejects runs the original
// schema-enforced fetch agent instead — a cost, never a correctness loss.
//
// This is a SHAPE guard only: it cannot tell a real tag list from a transcribed
// one (see parsePlanArgs' note on the two recorded corruptions, both of which
// are schema-valid and are accepted here by design). Content validation is task
// fix-plan-review-gate-tag-clobber's scope.
function stringArrayOk(v) {
  return Array.isArray(v) && v.every((s) => typeof s === 'string')
}
function hoistedFetchedOk(fetched, kind) {
  if (!fetched || typeof fetched !== 'object') return false
  if (typeof fetched.body !== 'string' || String(fetched.body).trim() === '') return false
  if (!stringArrayOk(fetched.tags)) return false
  if (kind === 'roadmap') {
    if (!Array.isArray(fetched.phases)) return false
    const phasesOk = fetched.phases.every(
      (p) =>
        p &&
        typeof p === 'object' &&
        typeof p.stem === 'string' &&
        p.stem.trim() !== '' &&
        typeof p.body === 'string' &&
        stringArrayOk(p.tags)
    )
    if (!phasesOk) return false
  }
  return true
}

const STAMP_ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok'],
  properties: { ok: { type: 'boolean' } },
}

// RAW_STDOUT_SCHEMA — the ONLY schema the mechanical fetch agents below are
// forced to satisfy. One string field, deliberately not a nested object: there
// is nothing here for an agent to interpret, rename, or compose. This replaces
// the former PLAN_TARGET_SCHEMA / ROADMAP_TARGET_SCHEMA, which asked the agent
// to hand back an already-composed { body, tags, phases } object — the exact
// shape that let a fetch agent transcribe junk over real plan data in
// production (see the fetch-prompt comment below). Parsing, field extraction,
// and identity validation now live entirely in this driver (parseJsonStdout /
// parseTranscriptBlocks / extractRoadmapFromJson / extractPhaseFromJson /
// extractTaskFromJson below) — the agent's only job is verbatim transcription.
const RAW_STDOUT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['transcript'],
  properties: {
    transcript: { type: 'string' },
  },
}

// stringArrayOk / stemDup / etc. are shared by hoistedFetchedOk above and the
// extract*FromJson validators below.

// parseTranscriptBlocks(transcript) — pure, never throws. Splits a raw
// transcript into the `===CMD: <command>===`-delimited blocks a fetch agent
// was instructed to emit (see buildRoadmapFetchPrompt below); a transcript
// with no recognizable marker returns []. Each block's `stdout` is the raw
// text between its marker and the next marker (or end of transcript).
const TRANSCRIPT_MARKER_RE = /^===CMD: (.*)===\s*$/
function parseTranscriptBlocks(transcript) {
  const text = typeof transcript === 'string' ? transcript : ''
  const lines = text.split('\n')
  const blocks = []
  let current = null
  for (let i = 0; i < lines.length; i++) {
    const m = TRANSCRIPT_MARKER_RE.exec(lines[i])
    if (m) {
      if (current) blocks.push(current)
      current = { command: m[1], stdoutLines: [] }
      continue
    }
    if (current) current.stdoutLines.push(lines[i])
  }
  if (current) blocks.push(current)
  return blocks.map((b) => ({ command: b.command, stdout: b.stdoutLines.join('\n') }))
}

// parseJsonStdout(stdout) — pure, never throws. JSON.parse()s the given text
// and requires the result to be a plain (non-array) object — anything else
// (a parse error, an array, a primitive, null) reports { ok:false }, which is
// this module's uniform fail-closed signal.
function parseJsonStdout(stdout) {
  try {
    const value = JSON.parse(String(stdout))
    if (!value || typeof value !== 'object' || Array.isArray(value)) return { ok: false }
    return { ok: true, value: value }
  } catch (e) {
    return { ok: false }
  }
}

// extractRoadmapFromJson(json, expectedSlug) — pure identity/collision
// validator for the roadmap-level block of a fetch:roadmap transcript. Rejects
// (ok:false) on anything that does not match `rdm roadmap show <expectedSlug>
// --format json`'s real contract: json.slug must equal expectedSlug, `body`
// must be a non-empty (trimmed) string, `tags` a string array. `phases` — the
// roadmap's own per-phase SUMMARY array (stem + whatever else `rdm roadmap
// show` reports; never a full body) — is optional; when present, every entry
// needs a non-empty string `stem`, AND the summary must clear two collision
// guards that reject the exact wf_e3402021-0af corruption shape: no stem may
// equal the roadmap's own slug (a lone phase entry mislabeled with the
// roadmap slug), and no two stems may be identical (phases collapsed into
// fewer, duplicated entries). A legitimately EMPTY phases array is not a
// collision and is accepted. `phaseSummaries` is returned as the authoritative
// phase-stem list the driver fans out over — never the phase blocks' own
// self-reported existence.
function extractRoadmapFromJson(json, expectedSlug) {
  if (!json || typeof json !== 'object') return { ok: false }
  if (json.slug !== expectedSlug) return { ok: false }
  const body = typeof json.body === 'string' ? json.body : ''
  if (body.trim() === '') return { ok: false }
  if (!stringArrayOk(json.tags)) return { ok: false }
  let phaseSummaries = []
  if (json.phases !== undefined) {
    if (!Array.isArray(json.phases)) return { ok: false }
    const shapeOk = json.phases.every(
      (p) => p && typeof p === 'object' && typeof p.stem === 'string' && p.stem.trim() !== ''
    )
    if (!shapeOk) return { ok: false }
    phaseSummaries = json.phases
    const stems = phaseSummaries.map((p) => p.stem)
    if (stems.indexOf(expectedSlug) !== -1) return { ok: false } // stem === roadmap slug
    if (new Set(stems).size !== stems.length) return { ok: false } // duplicate stems
  }
  return { ok: true, body: body, tags: json.tags, phaseSummaries: phaseSummaries }
}

// extractPhaseFromJson(json, expectedRoadmap, expectedStem) — pure
// identity validator for one phase block (either inside a roadmap transcript
// or the sole block of a fetch:phase transcript). Rejects on a stem mismatch
// or a `roadmap` field that disagrees with the roadmap actually being
// reviewed (cross-roadmap contamination of one block inside a shared
// transcript). `body` follows the SAME precedent buildReviewUnits already
// applied to a phase entry: an empty phase body is accepted here (only the
// roadmap-level body is fail-closed on emptiness) — string-typed, defaulting
// to '' when absent or non-string, never rejected for being blank.
function extractPhaseFromJson(json, expectedRoadmap, expectedStem) {
  if (!json || typeof json !== 'object') return { ok: false }
  if (json.stem !== expectedStem) return { ok: false }
  if (json.roadmap !== expectedRoadmap) return { ok: false }
  if (!stringArrayOk(json.tags)) return { ok: false }
  const body = typeof json.body === 'string' ? json.body : ''
  return { ok: true, body: body, tags: json.tags }
}

// extractTaskFromJson(json, expectedSlug) — pure identity validator for a
// fetch:task transcript. Same shape as extractPhaseFromJson; a task has no
// containing roadmap, so there is no cross-roadmap check.
function extractTaskFromJson(json, expectedSlug) {
  if (!json || typeof json !== 'object') return { ok: false }
  if (json.slug !== expectedSlug) return { ok: false }
  if (!stringArrayOk(json.tags)) return { ok: false }
  const body = typeof json.body === 'string' ? json.body : ''
  return { ok: true, body: body, tags: json.tags }
}

// Fetch prompts — mechanical Bash agents (the runtime cannot shell out
// itself). Their output contract is deliberately reduced to VERBATIM
// TRANSCRIPTION ONLY: run the command(s), print the raw stdout unmodified,
// return it under `transcript`. No field extraction, no renaming, no
// summarizing, no JSON composition — that step, which used to live in the
// agent's own judgment, is where a fetch agent twice fabricated a response
// that was schema-valid but had nothing to do with the real document (runs
// wf_e3402021-0af and wf_f4be8027-dbb, recorded on task
// fix-plan-review-gate-tag-clobber). All parsing, extraction, and identity
// validation now happen deterministically in THIS FILE, after the agent
// returns (see the extract*FromJson / parse* functions above).
function buildPhaseFetchPrompt(roadmap, phase) {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root:',
    '  ./target/debug/rdm phase show ' + phase + ' --roadmap ' + roadmap + ' --project rdm --format json',
    'Return a RAW_STDOUT object: `transcript` — the ENTIRE raw stdout of that command, character for',
    'character, exactly as printed. Do not summarize, reformat, extract fields, rename anything, or',
    'comment on it — copy it verbatim.',
    'If the command fails or prints nothing, return an empty string for `transcript`.',
  ].join('\n')
}
function buildTaskFetchPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root:',
    '  ./target/debug/rdm task show ' + slug + ' --project rdm --format json',
    'Return a RAW_STDOUT object: `transcript` — the ENTIRE raw stdout of that command, character for',
    'character, exactly as printed. Do not summarize, reformat, extract fields, rename anything, or',
    'comment on it — copy it verbatim.',
    'If the command fails or prints nothing, return an empty string for `transcript`.',
  ].join('\n')
}
// buildRoadmapFetchPrompt(slug) — this fetch stays at exactly ONE mechanical
// agent invocation per roadmap target, regardless of phase count. It is
// tempting to fix fetch corruption by splitting this into a cheap `roadmap
// show` fetch plus a driver-side parallel() fan-out of one `phase show` agent
// per stem (reusing buildPhaseFetchPrompt) — do NOT reach for that here. Both
// docs/mechanical-agent-inventory.md (§ "The hoist with a recorded correctness
// failure" / "must not be reintroduced") and task
// fix-plan-review-gate-tag-clobber's body (§ "Deferred option (do NOT reach
// for it first)") record why: for a 7-phase roadmap, 1 fetch:roadmap agent
// becoming 8 would inflate the very docs/token-baseline.json baseline the
// (now done) workflow-token-reduction roadmap phase 3 measures against. If
// phase-BODY corruption is ever separately proven (this incident was body/
// tags/phases-count corruption, not per-phase-body corruption), the fan-out
// remains available only after explicit coordination with that roadmap — not
// as a default reached for here. This function keeps the existing single-turn,
// multi-command shape (one `roadmap show` call, then one `phase show` call per
// phase the agent just read) and changes ONLY the output contract: verbatim,
// delimited transcription instead of composed JSON.
function buildRoadmapFetchPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run this command in the repo root:',
    '  ./target/debug/rdm roadmap show ' + slug + ' --project rdm --format json',
    'Before its output, print a line by itself: ===CMD: roadmap show ' + slug + '===',
    'Then print that command\'s raw stdout, character for character, exactly as printed — do not',
    'summarize, reformat, extract fields, rename anything, or comment on it.',
    'That JSON carries a `phases` array. For EACH entry in it, using the exact `stem` value you just',
    'read (copy it verbatim — do not invent, rename, or reorder it), run:',
    '  ./target/debug/rdm phase show <stem> --roadmap ' + slug + ' --project rdm --format json',
    'Before each of those outputs, print a line by itself: ===CMD: phase show <stem>=== (substituting',
    'the real stem value you read), then print that command\'s raw stdout verbatim, exactly as with the',
    'roadmap command above.',
    'Return a RAW_STDOUT object: `transcript` — the concatenation of every ===CMD: ...=== marker line',
    'and the raw stdout that follows it, one block per command, in the order the commands were run.',
    'If the roadmap command fails or prints nothing, still print its marker line followed by an empty',
    'body, and run no phase commands.',
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
// --implementation-plan mode (guarded at the call site). Large findings are
// filed with `--no-plan-review` so the gate's own output is never re-stamped
// `needs-plan-review` and fed back into itself as new input.
function buildActPrompt(kind, roadmap, ident, survivors) {
  // Once the review passes a non-gating finding through un-refuted the payload
  // is of MIXED provenance, so the leading "already survived refutation — do not
  // re-review" claim would be false for part of it. Both the claim and the
  // do-not-re-review directive are therefore conditional; with no un-refuted
  // survivor the prompt is byte-identical to the pre-pass-through one.
  const list = Array.isArray(survivors) ? survivors : []
  const hasUnrefuted = list.some((f) => f && f.unrefuted)
  const lines = hasUnrefuted
    ? [
        'You are the plan-review orchestrator applying findings of MIXED provenance. A finding WITHOUT',
        '`unrefuted: true` survived independent refutation; a finding WITH it was never graded by a refuter.',
      ]
    : [
        'You are the plan-review orchestrator applying already-verified findings. The findings below already',
        'survived independent refutation — do not re-review; act on them.',
      ]
  lines.push(
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
    '  decision): do NOT edit the plan document — file it as a task, with `--no-plan-review` so this finding',
    '  does not itself get re-stamped `needs-plan-review`:',
    '    ./target/debug/rdm task create <slug> --title "Plan review finding: <desc>" --body "<details>" --tags plan-review --no-plan-review --no-edit --project rdm'
  )
  if (hasUnrefuted) {
    lines.push(UNREFUTED_DISPOSITION)
  }
  lines.push(
    'After applying any changes, run: ./target/debug/rdm commit -m "chore(plan): address plan review findings on ' +
      (kind === 'phase' ? roadmap + '/' + ident : ident) +
      '"',
    'If there is nothing small to fix and nothing large to file, make no changes.',
    'Return a STAMP_ACK object: { ok: true } if you completed without error (including the no-op case), else { ok: false }.'
  )
  return lines.join('\n')
}

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

// buildWontFixFetchPrompt — mechanical fetch agent: list every task already
// resolved `wont-fix` that came out of a plan-review finding, as raw
// title+body text for the client-side overlap heuristic above to match
// against. One search covers every unit in this run.
function buildWontFixFetchPrompt() {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm search "" --tag plan-review --status wont-fix --type task --project rdm --format json',
    'Return a WONTFIX_LIST object: `texts` — one string per result, each the concatenation of that result\'s',
    'title and body separated by a newline.',
    'If the command fails or there are no results, return an empty `texts` array.',
  ].join('\n')
}

// buildRoundNoteWritePrompt — mechanical body-audit-note agent: append the
// round note to the END of the target's current body and commit. Runs on
// every non-`reviewed` outcome (persisted targets only — implementation-plan
// has no item to write to and is never routed here) so the body reflects the
// round before the next invocation reads it.
function buildRoundNoteWritePrompt(kind, roadmap, ident, round, outcome, findings) {
  const label = kind === 'phase' ? roadmap + '/' + ident : ident
  const showCmd =
    kind === 'task'
      ? './target/debug/rdm task show ' + ident + ' --project rdm --format json'
      : kind === 'phase'
      ? './target/debug/rdm phase show ' + ident + ' --roadmap ' + roadmap + ' --project rdm --format json'
      : './target/debug/rdm roadmap show ' + ident + ' --project rdm --format json'
  const updateCmd =
    kind === 'task'
      ? './target/debug/rdm task update ' + ident + ' --no-edit --project rdm'
      : kind === 'phase'
      ? './target/debug/rdm phase update ' + ident + ' --roadmap ' + roadmap + ' --no-edit --project rdm'
      : './target/debug/rdm roadmap update ' + ident + ' --no-edit --project rdm'
  return [
    'You are a mechanical body-audit-note agent. Do not plan, implement, or review anything.',
    '1. Read the current body: ' + showCmd + ' (the `body` field).',
    '2. Append exactly this block to the END of that body, separated from the existing content by a blank line:',
    '',
    formatRoundNote(round, outcome, findings),
    '',
    '3. Write the complete new body back verbatim (the current body, a blank line, then the block above) — `--body`',
    '   is whole-document-authoritative, there is no patch mechanism:',
    '   ' + updateCmd + ' --body "<current body>\\n\\n<block above>"',
    '4. Run: ./target/debug/rdm commit -m "chore(plan): record plan review round ' + round + ' on ' + label + '"',
    'Return a STAMP_ACK object: { ok: true } if all commands exited 0, else { ok: false }.',
  ].join('\n')
}

// assembleRoadmapFetchFromTranscript(transcript, expectedSlug) — pure: turn a
// fetch:roadmap agent's raw transcript into the { body, tags, phases } shape
// buildReviewUnits consumes. ALL-OR-NOTHING (same contract the former
// ROADMAP_TARGET_SCHEMA agent held): the roadmap block must parse and pass
// extractRoadmapFromJson, AND every phase its own phaseSummaries names must
// have a matching, validating phase block in the SAME transcript — one
// mismatch (a missing block, a JSON parse failure, a stem/roadmap disagreement)
// fails the WHOLE roadmap fetch, exactly as an empty roadmap body always has.
// Matching a phase block to a summary stem is done by the stem's presence in
// the block's own recorded `command` (the marker text the agent was told to
// print), never by transcript ORDER. Never throws — returns null on any
// failure, which the caller treats identically to `fetched === null`.
function assembleRoadmapFetchFromTranscript(transcript, expectedSlug) {
  const blocks = parseTranscriptBlocks(transcript)
  const roadmapBlock = blocks.find((b) => b.command.indexOf('roadmap show') === 0)
  if (!roadmapBlock) return null
  const rmParsed = parseJsonStdout(roadmapBlock.stdout)
  if (!rmParsed.ok) return null
  const rm = extractRoadmapFromJson(rmParsed.value, expectedSlug)
  if (!rm.ok) return null
  const phaseBlocks = blocks.filter((b) => b.command.indexOf('phase show') === 0)
  const phases = []
  for (let i = 0; i < rm.phaseSummaries.length; i++) {
    const stem = rm.phaseSummaries[i].stem
    const block = phaseBlocks.find((b) => b.command.indexOf(stem) !== -1)
    if (!block) return null
    const pj = parseJsonStdout(block.stdout)
    if (!pj.ok) return null
    const pext = extractPhaseFromJson(pj.value, expectedSlug, stem)
    if (!pext.ok) return null
    phases.push({ stem: stem, body: pext.body, tags: pext.tags })
  }
  return { body: rm.body, tags: rm.tags, phases: phases }
}

const WONTFIX_LIST_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['texts'],
  properties: { texts: { type: 'array', items: { type: 'string' } } },
}

// buildReviewUnits(parsed, fetched) — pure: turn a parsed target plus the fetched
// artifact JSON into the list of independent review units. A `phase`/`task`
// target is a single unit; a `roadmap` target is the roadmap body plus one unit
// per phase, each gated independently. Returns { units, fetchFailed }. FAIL-CLOSED
// on an empty/unread body: an unread plan must NEVER be silently marked reviewed.
//
// Defense-in-depth: a `fetched.phases` stem-collision/duplication guard runs
// here too, using ONLY the `stem` field the documented hoist contract already
// requires (see hoistedFetchedOk) — so it catches a corrupt payload arriving
// from EITHER path, the now-hardened fetch (extractRoadmapFromJson already
// rejects this shape before it reaches here) or a caller-supplied `fetched`
// hoist (whose content validation is out of this phase's scope — see
// docs/mechanical-agent-inventory.md). A trip returns the SAME fail-closed
// shape as an empty body, so the rest of the driver needs no new branch.
function buildReviewUnits(parsed, fetched) {
  const kind = parsed.kind
  if (kind === 'roadmap') {
    const rm = fetched
    if (!rm || !rm.body || String(rm.body).trim() === '') return { units: [], fetchFailed: true }
    const phaseStems = (Array.isArray(rm.phases) ? rm.phases : [])
      .map((p) => p && p.stem)
      .filter((s) => typeof s === 'string')
    if (phaseStems.indexOf(parsed.roadmap) !== -1 || new Set(phaseStems).size !== phaseStems.length) {
      return { units: [], fetchFailed: true }
    }
    const units = []
    units.push({
      kind: 'roadmap',
      targetType: 'roadmap',
      ident: parsed.roadmap,
      roadmap: parsed.roadmap,
      tags: Array.isArray(rm.tags) ? rm.tags : [],
      body: String(rm.body),
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
        body: String(p.body || ''),
        target: 'phase ' + parsed.roadmap + '/' + p.stem + '\n\n' + String(p.body || ''),
      })
    }
    return { units: units, fetchFailed: false }
  }
  // phase or task — a single unit.
  const meta = fetched
  if (!meta || !meta.body || String(meta.body).trim() === '') return { units: [], fetchFailed: true }
  const ident = kind === 'task' ? parsed.task : parsed.phase
  const label = kind === 'task' ? 'task/' + parsed.task : parsed.roadmap + '/' + parsed.phase
  return {
    units: [
      {
        kind: kind,
        targetType: kind,
        ident: ident,
        roadmap: parsed.roadmap,
        tags: Array.isArray(meta.tags) ? meta.tags : [],
        body: String(meta.body),
        target: kind + ' ' + label + '\n\n' + String(meta.body),
      },
    ],
    fetchFailed: false,
  }
}

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

// runPlanReviewDriver(args, deps) — the full plan-review orchestration. Every
// side effect is reached through the injected `deps`:
//   deps.agent          — the mechanical fetch / act / tag-write agent runner.
//   deps.parallel       — the per-unit fan-out primitive.
//   deps.log            — the log sink (optional; defaults to a no-op).
//   deps.runPlanReview  — an async runReview(context) from buildReviewPipeline
//                         ('plan'); optional — built from the review core when
//                         omitted (the Workflow runtime path, where the ambient
//                         agent/pipeline/parallel globals are probed by
//                         buildReviewPipeline itself).
//
// Returns the structured result the caller reports:
//   - implementation-plan: { kind, outcome, summary, findings } (report-only).
//   - fetch failure:       { kind, outcome:'escalated', fetchError:true, ... }.
//   - persisted targets:   { kind, units:[…] } with a single phase/task target
//                          also flattened onto { outcome, summary, findings }.
async function runPlanReviewDriver(args, deps) {
  const d = deps || {}
  const _agent = d.agent
  const _parallel = d.parallel
  const _log = d.log || function () {}
  // Optional: the resolved `rdm model resolve mechanical` id, threaded into
  // every mechanical fetch/gate call below (fetch:roadmap, fetch:<kind>,
  // gate:clear-tag:*). Left unset (undefined) is inert — see agent()'s
  // documented `model: undefined` behavior — so a caller that does not supply
  // it degrades to the pre-existing unpinned behavior rather than breaking.
  // A caller-supplied `args.mechanicalModel` (see parsePlanArgs) takes
  // precedence over the injected dep, so the local shim can skip the whole
  // model:mechanical bootstrap agent.
  let _mechanicalModel = d.mechanicalModel
  // Same deps-then-parsePlanArgs-override precedence as _mechanicalModel above,
  // for the judgment-site (finder/refuter) model ids — see parsePlanArgs' note
  // on findModel/verifyModel.
  let _findModel = d.findModel
  let _verifyModel = d.verifyModel
  // The plan review IS the canonical pipeline — buildReviewPipeline('plan') from
  // the review core, with NO independent review logic in this driver. Passing NO
  // signals is deliberate (see the header note); phase-only unit-of-work scoping
  // is applied per unit via stripNonPhaseUnitOfWork below.
  const runPlanReview = d.runPlanReview || buildReviewPipeline('plan')

  const parsed = parsePlanArgs(args)
  const kind = parsed.kind
  if (parsed.mechanicalModel) _mechanicalModel = parsed.mechanicalModel
  if (parsed.findModel) _findModel = parsed.findModel
  if (parsed.verifyModel) _verifyModel = parsed.verifyModel
  // Already validated by parsePlanArgs via the review core's single validator.
  const maxRefutations = parsed.maxRefutations

  // reviewUnit — run find → refute → filter for ONE review unit, then strip
  // non-phase unit-of-work survivors, drop anything already resolved
  // wont-fix, read the unit's prior round off its own body, and classify with
  // the round cap. Returns a per-unit result the act + gate steps consume
  // independently. `wontFixedTexts` is the SAME list for every unit in a run
  // (one search covers the whole run, not one per unit).
  async function reviewUnit(unit, wontFixedTexts) {
    // runPlanReview is a `runReview` from the canonical review source and
    // resolves `{ survivors, acTable, budget, coverage }`; `acTable` is always
    // `null` in plan mode (the `ac` dimension does not exist there) and is
    // intentionally discarded here. `budget` is the per-unit refutation-budget
    // accounting and `coverage` the per-unit dimension-participation accounting;
    // both are carried through to the reported result.
    //
    // IMPORTANT: `budget` describes the PIPELINE, not this unit's final reported
    // findings — stripNonPhaseUnitOfWork and suppressWontFixed run AFTER it and
    // may drop a survivor that consumed budget.
    const { survivors: rawSurvivors, budget, coverage } = await runPlanReview({ target: unit.target, maxRefutations: maxRefutations, findModel: _findModel, verifyModel: _verifyModel })
    const strippedSurvivors = stripNonPhaseUnitOfWork(rawSurvivors, unit.targetType)
    const survivors = suppressWontFixed(strippedSurvivors, wontFixedTexts)
    const prior = parseRoundNotes(unit.body)
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
      // A dimension that did not participate is named in the SUMMARY STRING, not
      // only in the machine-readable `coverage` key — the gate below derives
      // `reason` from this same string, so a plan review that ran 2 of 3
      // dimensions can never read like a complete one. The clause is empty on a
      // complete run, so a healthy unit's summary is byte-unchanged.
      summary: summarizeFindings(survivors) + coverageSummaryClause(buildReviewCoverage([coverage], null)),
    }
  }

  // ------------------------------------------------------------------ implementation-plan
  // Report-only: no persisted rdm item, so no act and no gate.
  // stripNonPhaseUnitOfWork drops unit-of-work here too (targetType
  // 'implementation-plan' !== 'phase').
  if (kind === 'implementation-plan') {
    const planText = parsed.planText || '(the implementation plan provided in context)'
    // See reviewUnit's identical notes: acTable is always null in plan mode, and
    // `budget` describes the pipeline, not the post-strip survivor set.
    const { survivors: rawSurvivors, budget, coverage } = await runPlanReview({ target: planText, maxRefutations: maxRefutations, findModel: _findModel, verifyModel: _verifyModel })
    const survivors = stripNonPhaseUnitOfWork(rawSurvivors, 'implementation-plan')
    const outcome = classifyPlanOutcome(survivors)
    // Same summary treatment as reviewUnit: reduced coverage is named in the
    // human-visible string, empty on a complete run.
    const planSummary =
      summarizeFindings(survivors) + coverageSummaryClause(buildReviewCoverage([coverage], null))
    _log(
      'plan-review (implementation-plan): ' +
        outcome +
        ' — ' +
        planSummary +
        formatUnitBudget(budget)
    )
    return {
      kind: 'implementation-plan',
      outcome: outcome,
      summary: planSummary,
      budget: budget || null,
      coverage: coverage || null,
      findings: survivors,
    }
  }

  // ------------------------------------------------------------------ persisted targets
  // Fetch the artifact(s), then build the independent review unit list.
  //
  // HOIST: a caller-supplied `fetched` payload replaces the transcribing agent
  // outright. This is the priority hoist of the whole elimination pass — see
  // parsePlanArgs' note on the two recorded production corruptions that
  // schema validation could not catch. A payload the shape guard rejects falls
  // through to the agent below, which is left byte-unchanged.
  let fetched = null
  if (hoistedFetchedOk(parsed.fetched, kind)) {
    fetched = parsed.fetched
    _log('plan-review: ' + kind + ' payload hoisted from caller args (no fetch agent)')
  } else if (kind === 'roadmap') {
    // ONE agent call regardless of phase count — see the "must not be
    // reintroduced" comment on buildRoadmapFetchPrompt above. The agent
    // transcribes raw stdout only; assembleRoadmapFetchFromTranscript does
    // every bit of parsing, extraction, and identity/collision validation.
    try {
      const raw = await _agent(buildRoadmapFetchPrompt(parsed.roadmap), {
        label: 'fetch:roadmap',
        phase: 'Read',
        agentType: 'rdm-mechanical',
        schema: RAW_STDOUT_SCHEMA,
        model: _mechanicalModel,
      })
      fetched = assembleRoadmapFetchFromTranscript(raw && raw.transcript, parsed.roadmap)
    } catch (e) {
      fetched = null
    }
  } else {
    const fetchPrompt =
      kind === 'task' ? buildTaskFetchPrompt(parsed.task) : buildPhaseFetchPrompt(parsed.roadmap, parsed.phase)
    try {
      const raw = await _agent(fetchPrompt, {
        label: 'fetch:' + kind,
        phase: 'Read',
        agentType: 'rdm-mechanical',
        schema: RAW_STDOUT_SCHEMA,
        model: _mechanicalModel,
      })
      const parsedStdout = parseJsonStdout(raw && raw.transcript)
      const extracted = parsedStdout.ok
        ? kind === 'task'
          ? extractTaskFromJson(parsedStdout.value, parsed.task)
          : extractPhaseFromJson(parsedStdout.value, parsed.roadmap, parsed.phase)
        : { ok: false }
      fetched = extracted.ok ? { body: extracted.body, tags: extracted.tags } : null
    } catch (e) {
      fetched = null
    }
  }

  const built = buildReviewUnits(parsed, fetched)
  const units = built.units

  // FAIL-CLOSED: an unread plan must NOT be silently marked reviewed / have its
  // tag cleared. Report the failure and mutate nothing.
  if (built.fetchFailed) {
    _log('plan-review: artifact fetch failed for ' + kind + ' — leaving needs-plan-review in place (fail-closed)')
    return { kind: kind, outcome: 'escalated', fetchError: true, summary: 'plan-review: artifact fetch failed', units: [] }
  }

  // One wont-fix search covers every unit in this run — a human's explicit
  // override on one finding must never be looked up per unit.
  // HOIST: a caller-supplied `wontFixedTexts` array replaces this search agent.
  let wontFixedTexts = []
  if (Array.isArray(parsed.wontFixedTexts)) {
    wontFixedTexts = parsed.wontFixedTexts
    _log('plan-review: wont-fix texts hoisted from caller args (no fetch agent)')
  } else {
    try {
      const wf = await _agent(buildWontFixFetchPrompt(), {
        label: 'fetch:wontfix',
        phase: 'Read',
        agentType: 'rdm-mechanical',
        schema: WONTFIX_LIST_SCHEMA,
        model: _mechanicalModel,
      })
      wontFixedTexts = wf && Array.isArray(wf.texts) ? wf.texts : []
    } catch (e) {
      wontFixedTexts = []
    }
  }

  // Review each unit independently (parallel per-unit fan-out — a phase's outcome
  // never changes a sibling's). A single phase/task target is a one-element list.
  const results = await _parallel(units.map((u) => () => reviewUnit(u, wontFixedTexts)))

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
        await _agent(buildActPrompt(u.kind, u.roadmap, u.ident, r.survivors), {
          label: 'act:' + u.kind + ':' + u.ident,
          phase: 'Act',
          schema: STAMP_ACK_SCHEMA,
        })
      } catch (e) {
        _log('plan-review: act step failed for ' + u.kind + '/' + u.ident + ' — continuing to gate')
      }
    }

    // --- Round audit note (orchestrator-only; skipped for implementation-plan) ---
    // On any non-`reviewed` outcome, record the round: the FULL deduped
    // remaining findings (not just the newly-reported subset), so nothing open
    // is hidden from a future reader — this runs even when survivors is empty
    // (a round-3+ escalation can have zero findings and still must be capped).
    if (kind !== 'implementation-plan' && r.outcome !== 'reviewed') {
      try {
        await _agent(buildRoundNoteWritePrompt(u.kind, u.roadmap, u.ident, r.round, r.outcome, r.survivors), {
          label: 'act:round-note:' + u.kind + ':' + u.ident,
          phase: 'Act',
          schema: STAMP_ACK_SCHEMA,
        })
      } catch (e) {
        _log('plan-review: round-note write failed for ' + u.kind + '/' + u.ident)
      }
    }

    // --- Gate (skipped for implementation-plan) ---
    // On reviewed: read-filter-write the tags to drop needs-plan-review,
    // preserving siblings. On rework/escalated: leave the tag; GATE_POLICY.plan
    // never persists an rdm status (gate.status is a literal null).
    let tagCleared = false
    if (kind !== 'implementation-plan') {
      if (gate.clearsPlanReviewTag) {
        // u.tags comes from the AC1/AC2-validated fetch (or the caller's
        // shape-guarded hoist), not a fresh re-fetch here — a second gate-time
        // fetch agent was considered and DECLINED: there is now only ONE
        // trustworthy fetch per unit, so a re-fetch would only re-inflate the
        // mechanical-agent count this file's design is held to (see
        // docs/mechanical-agent-inventory.md's agent-count-discipline note on
        // this file) for a live-race scenario nothing in this phase asked for.
        const remaining = filterPlanReviewTag(u.tags)
        try {
          const ack = await _agent(buildTagWritePrompt(u.kind, u.roadmap, u.ident, remaining), {
            label: 'gate:clear-tag:' + u.kind + ':' + u.ident,
            phase: 'Gate',
            agentType: 'rdm-mechanical',
            schema: STAMP_ACK_SCHEMA,
            model: _mechanicalModel,
          })
          tagCleared = !!(ack && ack.ok === true)
        } catch (e) {
          _log('plan-review: tag-clear failed for ' + u.kind + '/' + u.ident)
        }
      }
    }

    const reason = gate.reasonPrefix ? gate.reasonPrefix + ' ' + r.summary : ''
    reported.push({
      kind: u.kind,
      ident: u.ident,
      roadmap: u.roadmap,
      outcome: r.outcome,
      round: r.round,
      newlyReported: r.newlyReported,
      repeats: r.repeats,
      status: gate.status,
      clearsPlanReviewTag: gate.clearsPlanReviewTag,
      tagCleared: tagCleared,
      reason: reason,
      summary: r.summary,
      budget: r.budget || null,
      coverage: r.coverage || null,
      findings: r.survivors,
    })
    _log('plan-review (' + u.kind + '/' + u.ident + '): ' + r.outcome + ' — ' + r.summary + formatUnitBudget(r.budget))
  }

  const result = { kind: kind, units: reported }
  if (kind !== 'roadmap' && reported.length === 1) {
    // Flatten a single phase/task target onto the top-level result for convenience.
    result.outcome = reported[0].outcome
    result.summary = reported[0].summary
    result.budget = reported[0].budget
    result.coverage = reported[0].coverage
    result.findings = reported[0].findings
  }
  _log('plan-review (' + kind + '): ' + reported.length + ' unit(s) gated')
  return result
}
// >>> plan-review-driver:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export {
  parsePlanArgs,
  hoistedFetchedOk,
  buildReviewUnits,
  runPlanReviewDriver,
  buildPhaseFetchPrompt,
  buildTaskFetchPrompt,
  buildRoadmapFetchPrompt,
  buildTagWritePrompt,
  buildActPrompt,
  RAW_STDOUT_SCHEMA,
  STAMP_ACK_SCHEMA,
  parseTranscriptBlocks,
  parseJsonStdout,
  extractRoadmapFromJson,
  extractPhaseFromJson,
  extractTaskFromJson,
  assembleRoadmapFetchFromTranscript,
  parseRoundNotes,
  formatRoundNote,
  findingSignature,
  partitionRepeats,
  suppressWontFixed,
  wontFixOverlapMatches,
  classifyRoundOutcome,
  buildWontFixFetchPrompt,
  buildRoundNoteWritePrompt,
  formatUnitBudget,
  WONTFIX_LIST_SCHEMA,
};
