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
// --implementation-plan mode (guarded at the call site). Large findings are
// filed with `--no-plan-review` so the gate's own output is never re-stamped
// `needs-plan-review` and fed back into itself as new input.
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
    '  decision): do NOT edit the plan document — file it as a task, with `--no-plan-review` so this finding',
    '  does not itself get re-stamped `needs-plan-review`:',
    '    ./target/debug/rdm task create <slug> --title "Plan review finding: <desc>" --body "<details>" --tags plan-review --no-plan-review --no-edit --project rdm',
    'After applying any changes, run: ./target/debug/rdm commit -m "chore(plan): address plan review findings on ' +
      (kind === 'phase' ? roadmap + '/' + ident : ident) +
      '"',
    'If there is nothing small to fix and nothing large to file, make no changes.',
    'Return a STAMP_ACK object: { ok: true } if you completed without error (including the no-op case), else { ok: false }.',
  ].join('\n')
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
    const bm = /^- \[(blocking|concern)\] ([^:]+): (.*)$/.exec(line)
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

// classifyRoundOutcome(round, survivors) — the round-outcome capper. Rounds 1
// and 2 classify from the FULL (wont-fix-suppressed but repeat-unfiltered)
// survivor set via classifyPlanOutcome, exactly as an uncapped run would;
// round 3+ returns 'escalated' UNCONDITIONALLY, regardless of findings
// content, so an item can never loop forever on the same unresolved finding.
function classifyRoundOutcome(round, survivors) {
  if (round >= 3) return 'escalated'
  return classifyPlanOutcome(survivors)
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
  const _mechanicalModel = d.mechanicalModel
  // The plan review IS the canonical pipeline — buildReviewPipeline('plan') from
  // the review core, with NO independent review logic in this driver. Passing NO
  // signals is deliberate (see the header note); phase-only unit-of-work scoping
  // is applied per unit via stripNonPhaseUnitOfWork below.
  const runPlanReview = d.runPlanReview || buildReviewPipeline('plan')

  const parsed = parsePlanArgs(args)
  const kind = parsed.kind

  // reviewUnit — run find → refute → filter for ONE review unit, then strip
  // non-phase unit-of-work survivors, drop anything already resolved
  // wont-fix, read the unit's prior round off its own body, and classify with
  // the round cap. Returns a per-unit result the act + gate steps consume
  // independently. `wontFixedTexts` is the SAME list for every unit in a run
  // (one search covers the whole run, not one per unit).
  async function reviewUnit(unit, wontFixedTexts) {
    const rawSurvivors = await runPlanReview({ target: unit.target })
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
      summary: summarizeFindings(survivors),
    }
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
        model: _mechanicalModel,
      })
    } catch (e) {
      fetched = null
    }
  } else {
    const fetchPrompt =
      kind === 'task' ? buildTaskFetchPrompt(parsed.task) : buildPhaseFetchPrompt(parsed.roadmap, parsed.phase)
    try {
      fetched = await _agent(fetchPrompt, {
        label: 'fetch:' + kind,
        phase: 'Read',
        schema: PLAN_TARGET_SCHEMA,
        model: _mechanicalModel,
      })
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
  let wontFixedTexts = []
  try {
    const wf = await _agent(buildWontFixFetchPrompt(), {
      label: 'fetch:wontfix',
      phase: 'Read',
      schema: WONTFIX_LIST_SCHEMA,
      model: _mechanicalModel,
    })
    wontFixedTexts = wf && Array.isArray(wf.texts) ? wf.texts : []
  } catch (e) {
    wontFixedTexts = []
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
        const remaining = filterPlanReviewTag(u.tags)
        try {
          const ack = await _agent(buildTagWritePrompt(u.kind, u.roadmap, u.ident, remaining), {
            label: 'gate:clear-tag:' + u.kind + ':' + u.ident,
            phase: 'Gate',
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
  parseRoundNotes,
  formatRoundNote,
  findingSignature,
  partitionRepeats,
  suppressWontFixed,
  wontFixOverlapMatches,
  classifyRoundOutcome,
  buildWontFixFetchPrompt,
  buildRoundNoteWritePrompt,
  WONTFIX_LIST_SCHEMA,
};
