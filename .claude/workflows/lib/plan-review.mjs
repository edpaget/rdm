//! plan-review — the driver core of the standalone plan-review workflow.
//!
//! This is the **single source of truth** for the plan-review DRIVER: argument
//! parsing (`parsePlanArgs`), the mechanical fetch/act/gate prompt builders, and
//! the dependency-injected orchestration (`runPlanReviewDriver`). Because the
//! Claude Code Workflow runtime cannot `import`/`require` (see
//! docs/workflow-schemas.md § "Import spike"), the marked block below is copied
//! BYTE-IDENTICAL into `.claude/workflows/plan-review.js`. Unlike the
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
  buildReviewPipeline,
  stripNonPhaseUnitOfWork,
  filterPlanReviewTag,
  classifyPlanOutcome,
  gateFor,
  summarizeFindings,
} from './review.mjs';

// >>> plan-review-driver:begin <<<
// Pure + dependency-injected driver logic for the standalone plan-review
// workflow.
//
// This block is the single source of truth in
// .claude/workflows/lib/plan-review.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/plan-review.js (the Workflow runtime cannot load modules at
// run time). scripts/verify-workflow-review.sh gates the two copies for drift.
// No Date.now / Math.random — pure array/string ops plus injected async deps.
//
// `buildReviewPipeline`, `stripNonPhaseUnitOfWork`, `filterPlanReviewTag`,
// `classifyPlanOutcome`, `gateFor`, and `summarizeFindings` are NOT declared
// here: they belong to the canonical review source (lib/review.mjs) and reach
// this block from the stamped review block that precedes it in the workflow
// consumer (and from the import above in Node).

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

// buildReviewUnits(parsed, fetched) — pure: turn a parsed target plus the fetched
// artifact JSON into the list of independent review units. A `phase`/`task`
// target is a single unit; a `roadmap` target is the roadmap body plus one unit
// per phase, each gated independently. Returns { units, fetchFailed }. FAIL-CLOSED
// on an empty/unread body: an unread plan must NEVER be silently marked reviewed.
function buildReviewUnits(parsed, fetched) {
  const kind = parsed.kind
  if (kind === 'roadmap') {
    const rm = fetched
    if (!rm || !rm.body || String(rm.body).trim() === '') return { units: [], fetchFailed: true }
    const units = []
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
        target: kind + ' ' + label + '\n\n' + String(meta.body),
      },
    ],
    fetchFailed: false,
  }
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
  // The plan review IS the canonical pipeline — buildReviewPipeline('plan') from
  // the review core, with NO independent review logic in this driver. Passing NO
  // signals is deliberate (see the header note); phase-only unit-of-work scoping
  // is applied per unit via stripNonPhaseUnitOfWork below.
  const runPlanReview = d.runPlanReview || buildReviewPipeline('plan')

  const parsed = parsePlanArgs(args)
  const kind = parsed.kind

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
  // Report-only: no persisted rdm item, so no act and no gate.
  // stripNonPhaseUnitOfWork drops unit-of-work here too (targetType
  // 'implementation-plan' !== 'phase').
  if (kind === 'implementation-plan') {
    const planText = parsed.planText || '(the implementation plan provided in context)'
    const rawSurvivors = await runPlanReview({ target: planText })
    const survivors = stripNonPhaseUnitOfWork(rawSurvivors, 'implementation-plan')
    const outcome = classifyPlanOutcome(survivors)
    _log('plan-review (implementation-plan): ' + outcome + ' — ' + summarizeFindings(survivors))
    return {
      kind: 'implementation-plan',
      outcome: outcome,
      summary: summarizeFindings(survivors),
      findings: survivors,
    }
  }

  // ------------------------------------------------------------------ persisted targets
  // Fetch the artifact(s), then build the independent review unit list.
  let fetched = null
  if (kind === 'roadmap') {
    try {
      fetched = await _agent(buildRoadmapFetchPrompt(parsed.roadmap), {
        label: 'fetch:roadmap',
        phase: 'Read',
        schema: ROADMAP_TARGET_SCHEMA,
      })
    } catch (e) {
      fetched = null
    }
  } else {
    const fetchPrompt =
      kind === 'task' ? buildTaskFetchPrompt(parsed.task) : buildPhaseFetchPrompt(parsed.roadmap, parsed.phase)
    try {
      fetched = await _agent(fetchPrompt, { label: 'fetch:' + kind, phase: 'Read', schema: PLAN_TARGET_SCHEMA })
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

  // Review each unit independently (parallel per-unit fan-out — a phase's outcome
  // never changes a sibling's). A single phase/task target is a one-element list.
  const results = await _parallel(units.map((u) => () => reviewUnit(u)))

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

    // --- Gate (skipped for implementation-plan) ---
    // On reviewed: read-filter-write the tags to drop needs-plan-review,
    // preserving siblings. On rework/escalated: leave the tag; GATE_POLICY.plan
    // never persists an rdm status (gate.status is a literal null).
    let tagCleared = false
    if (kind !== 'implementation-plan') {
      if (gate.clearsPlanReviewTag) {
        const remaining = filterPlanReviewTag(u.tags)
        try {
          const ack = await _agent(buildTagWritePrompt(u.kind, u.roadmap, u.ident, remaining), {
            label: 'gate:clear-tag:' + u.kind + ':' + u.ident,
            phase: 'Gate',
            schema: STAMP_ACK_SCHEMA,
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
      status: gate.status,
      clearsPlanReviewTag: gate.clearsPlanReviewTag,
      tagCleared: tagCleared,
      reason: reason,
      summary: r.summary,
      findings: r.survivors,
    })
    _log('plan-review (' + u.kind + '/' + u.ident + '): ' + r.outcome + ' — ' + r.summary)
  }

  const result = { kind: kind, units: reported }
  if (kind !== 'roadmap' && reported.length === 1) {
    // Flatten a single phase/task target onto the top-level result for convenience.
    result.outcome = reported[0].outcome
    result.summary = reported[0].summary
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
  buildReviewUnits,
  runPlanReviewDriver,
  buildPhaseFetchPrompt,
  buildTaskFetchPrompt,
  buildRoadmapFetchPrompt,
  buildTagWritePrompt,
  buildActPrompt,
  PLAN_TARGET_SCHEMA,
  ROADMAP_TARGET_SCHEMA,
  STAMP_ACK_SCHEMA,
};
