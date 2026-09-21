// document — headless documentation-draft generator for a completed rdm roadmap.
//
// Validates that every phase of a roadmap is `done`, fans out a per-phase
// git-gather step in parallel() (falling back to phase-body-only when a phase
// has no commit SHA, or the SHA is unreachable), runs one synthesis agent to
// draft the doc from bodies + diffs, and a mechanical Bash agent to write the
// result to `--out` (default `docs/<slug>.md`). Returns
// { roadmap, aborted, incompletePhases, path, draft }.
//
// IMPORTANT: this workflow produces an artifact, not a completion signal — the
// terminal human approval lives in the rdm-document skill shim, never here. The
// driver below performs NO status mutation (no `rdm roadmap/phase/task update
// --status`) and no plan-mode/confirmation call; it simply returns after Stage 3
// (or after the all-done validation's abort short-circuit). The skill shim reads
// the returned draft/path and presents them for the human's terminal review —
// see `.claude/skills/rdm-document/SKILL.md`.
//
// Invoke with args: { roadmap: '<roadmap-slug>', out: '<optional path>' }.
//
// This script embeds ONE copied block, because the Workflow runtime cannot load
// helper modules at run time (docs/workflow-schemas.md § "Import spike"): the
// document-core block, copied BYTE-IDENTICAL from lib/document.mjs;
// scripts/verify-workflow-document.sh gates it for drift. Unlike dispatch-phase
// and plan-review, this workflow does NOT consume the canonical review block —
// it has no plan/code review gate of its own, so it embeds no review-refute-fix
// copy.

export const meta = {
  name: 'rdm-wf-document',
  description:
    'Headlessly draft user documentation from a completed rdm roadmap (phase bodies + commit diffs) and write it to disk',
  phases: [{ title: 'Gather' }, { title: 'Synthesize' }],
}

// The block below is copied BYTE-IDENTICAL from
// .claude/workflows/lib/document.mjs — do NOT edit it here. Edit the lib and
// scripts/verify-workflow-document.sh fails the build on drift.
// >>> document-core:begin <<<
// Pure, deterministic decision logic for the document workflow.
//
// This block is the single source of truth in
// .claude/workflows/lib/document.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/rdm-wf-document.js (the Workflow runtime cannot load modules at run
// time). scripts/verify-workflow-document.sh gates the two copies for drift.
// No Date.now / Math.random — pure array/string ops only.

// parseDocumentArgs(args) — coerce and default the whole args payload.
//
// The Workflow tool contract forbids stringified args, but LLM callers (the
// rdm-document skill shim, or a hand-run invocation) may still deliver a JSON
// string; coerce once, mirroring parseDispatchArgs in the since-deleted
// lib/dispatch-phase.mjs.
function parseDocumentArgs(args) {
  let documentArgs = args || {};
  if (typeof documentArgs === 'string') {
    try {
      documentArgs = JSON.parse(documentArgs) || {};
    } catch (e) {
      documentArgs = {};
    }
  }
  if (!documentArgs || typeof documentArgs !== 'object') documentArgs = {};
  return {
    roadmap: documentArgs.roadmap || '',
    out: documentArgs.out || '',
  };
}

// defaultOutPath(slug) — the default write location when no --out is given.
function defaultOutPath(slug) {
  return 'docs/' + slug + '.md';
}

// resolveOutPath(args) — an explicit `out` always wins over the default.
function resolveOutPath(args) {
  const a = args || {};
  return a.out || defaultOutPath(a.roadmap);
}

// computeIncompletePhases(phases) — every phase whose status is not `done`.
// A roadmap with zero phases is vacuously all-done (returns []), so an empty
// roadmap proceeds to a (contentless) draft rather than short-circuiting —
// this is a deliberate, documented choice (see rdm-wf-document.js's driver), not an
// oversight.
function computeIncompletePhases(phases) {
  const list = Array.isArray(phases) ? phases : [];
  return list.filter((p) => !p || p.status !== 'done');
}

// buildGitRangeCommands(sha) — the git commands a per-phase gather agent runs
// to collect what actually shipped for one phase's commit SHA. Mirrors the
// single-commit range convention: a phase is completed by exactly one commit,
// so the range is always `<sha>~1..<sha>` (degenerates identically whether the
// roadmap has one phase or many, since this operates per-phase, not
// per-roadmap-range). `hasSha` is false for any non-string or empty SHA, which
// is also the fallback-to-body-only signal the gather prompt keys off.
function buildGitRangeCommands(sha) {
  if (typeof sha !== 'string' || sha === '') {
    return { hasSha: false, log: null, diffStat: null };
  }
  return {
    hasSha: true,
    log: 'git log --oneline ' + sha + '~1..' + sha,
    diffStat: 'git diff --stat ' + sha + '~1..' + sha,
  };
}
// >>> document-core:end <<<

// --- Schemas (document-specific; see scripts/verify-workflow-document.sh) -----

// ROADMAP_META — the shape the CALLER supplies for the roadmap's phase list.
// It is no longer an agent schema — nothing fetches it — but the shape guard
// below still reads it, and the caller is told exactly this shape.
// `found` distinguishes "the roadmap does not exist / the command failed" from
// a roadmap that genuinely has zero phases — both would otherwise present as an
// empty `phases` array. A not-found roadmap is treated the same as an
// unresolvable fetch (mirroring dispatch-phase's fetchError short-circuit)
// rather than silently proceeding as if it had no phases.
const ROADMAP_META_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['found', 'slug', 'title', 'phases'],
  properties: {
    found: { type: 'boolean' },
    slug: { type: 'string' },
    title: { type: 'string' },
    phases: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['stem', 'title', 'status'],
        properties: {
          stem: { type: 'string' },
          title: { type: 'string' },
          status: { type: 'string' },
        },
      },
    },
  },
}

// PHASE_RECORD — Stage 1's per-phase gather result: body + commit metadata,
// plus the has-SHA-vs-body-only outcome. `fallback:true` means no git data was
// gathered (missing or unreachable SHA) and the synthesis agent must lean on
// `title`/`body` alone for this phase.
const PHASE_RECORD_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['stem', 'title', 'body', 'hasSha', 'fallback'],
  properties: {
    stem: { type: 'string' },
    title: { type: 'string' },
    body: { type: 'string' },
    commit: { type: 'string' },
    hasSha: { type: 'boolean' },
    fallback: { type: 'boolean' },
    gitLog: { type: 'string' },
    gitDiffStat: { type: 'string' },
  },
}

// DRAFT — Stage 2's synthesis output: the whole draft as one Markdown string.
const DRAFT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['draft'],
  properties: { draft: { type: 'string' } },
}

// `WRITE_ACK_SCHEMA` is GONE with the `write:draft` agent whose ack it shaped.
// The write is command text the orchestrator runs; a shell exit status needs no
// schema.

// --- Prompt builders ----------------------------------------------------------

// `buildMechanicalModelPrompt`, `MECHANICAL_MODEL_SCHEMA` and
// `buildRoadmapFetchPrompt` are GONE with the `model:mechanical` and
// `fetch:roadmap-meta` agents. There is no mechanical agent left for a
// mechanical model to pin, and the roadmap's phase list is a read the
// ORCHESTRATOR does (`rdm roadmap show --format json`) and passes as
// `roadmapMeta`.

// documentRoadmapCommand(slug) — the read-only command that produces that
// payload, returned as TEXT so a caller that omitted it is told what to run.
function documentRoadmapCommand(slug) {
  return './target/debug/rdm roadmap show ' + slug + ' --project rdm --format json'
}

// Stage 1: one READ-ONLY agent per phase, run inside parallel(). It reads the
// phase document and, when the phase recorded a commit, that commit's history —
// and it JUDGES what the change actually did, which is why it is not a
// mechanical transcriber and carries no `agentType`. `gitCmdTemplate` is
// buildGitRangeCommands('<SHA>') — a placeholder rendering the agent substitutes
// the real commit value into, so the command text always matches the pure
// function's format.
function buildPhaseGatherPrompt(roadmap, phase, gitCmdTemplate) {
  return [
    'You are a READ-ONLY documentation gatherer. Plan nothing, implement nothing, and edit no files.',
    'Read the phase document yourself — run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm phase show ' + phase.stem + ' --roadmap ' + roadmap + ' --project rdm --format json',
    'From it, take stem ("' + phase.stem + '"), title (the phase JSON `title`), body (the phase JSON `body`',
    'verbatim), and commit (the phase JSON `commit` field if present and non-empty, else an empty string).',
    'Then decide the git-gather step:',
    '- If `commit` is a non-empty string, treat it as SHA and run these two commands in the CURRENT working',
    '  directory (the source repo you are already in, NOT the plan repo), substituting the real SHA value for',
    '  the <SHA> placeholder below:',
    '    ' + gitCmdTemplate.log,
    '    ' + gitCmdTemplate.diffStat,
    '  If BOTH commands succeed and produce output, set hasSha:true, fallback:false, gitLog (the first',
    '  command\'s output), and gitDiffStat (the second command\'s output).',
    '- If `commit` is empty/missing, OR either git command errors or returns no output (e.g. the SHA was',
    '  rebased away and is unreachable), fall back to body-only: set hasSha:false, fallback:true, and omit',
    '  gitLog/gitDiffStat entirely — rely on the phase title and body alone.',
    'Return a PHASE_RECORD object with exactly these fields.',
  ].join('\n')
}

// Stage 2: a single synthesis agent drafts the whole document from every
// gathered phase record. Preserves the guidance the old rdm-document SKILL.md
// carried in prose (internal/refactoring phases, minimal bodies, cross-
// referencing diffs against descriptions) as agent instructions instead.
function buildSynthesisPrompt(roadmapMeta, records) {
  return [
    'You are a documentation synthesis agent. Write NO files — return the draft as text only.',
    'Draft user-facing Markdown documentation for the completed roadmap "' +
      (roadmapMeta.title || roadmapMeta.slug) +
      '" (' +
      roadmapMeta.slug +
      ').',
    'You are given, per phase, its title/body (the intent) and — where available — a git log + diff --stat',
    '(what actually shipped). A phase marked fallback:true has no git data; lean on its title/body alone.',
    'Phase records (ordered):',
    JSON.stringify(records, null, 2),
    'Use this structure:',
    '# <Feature Title>',
    '',
    '## Overview',
    'What the feature is — one or two paragraphs.',
    '',
    '## Motivation',
    'Why it was built — the problem it solves.',
    '',
    '## Usage',
    'Concrete examples: CLI commands, config options, API calls, in fenced code blocks. This is the most',
    'important section — include real, working examples drawn from the phase bodies/diffs.',
    '',
    '## How it works',
    '(Include only for complex features.) Architecture, key modules, data flow.',
    '',
    '## Limitations',
    '(Include only if applicable.) Known gaps, unsupported scenarios, planned future work.',
    '',
    'Guidelines:',
    '- Write for users, not developers — focus on what they can do, not internal implementation details.',
    '- Internal/refactoring-only phases (no user-visible change) belong briefly in "How it works", if',
    '  anywhere at all — omit them from Usage.',
    '- When a phase body is minimal or empty, lean on its gitDiffStat/gitLog to fill in what actually shipped.',
    '- Cross-reference phase descriptions against diff stats/file lists so the documentation reflects what was',
    '  actually built, and note any discrepancy you find.',
    'If the roadmap has NO phases at all, return a short draft noting the roadmap has no phases to document —',
    'do not fabricate content.',
    'Return a DRAFT object: { draft: "<the full markdown document as a single string>" }.',
  ].join('\n')
}

// Stage 3 is the ORCHESTRATOR's. `buildWriteCommands(outPath, draftText)` returns
// the exact shell that writes the draft — parent directories first, then a
// QUOTED heredoc so nothing in the draft is interpreted — and the caller runs it.
// The Workflow runtime has no filesystem of its own, and dispatching an agent to
// hold one was the mechanical-write pattern this phase removed.
function buildWriteCommands(outPath, draftText) {
  const marker = 'RDM_DOCUMENT_DRAFT_EOF'
  return [
    '  mkdir -p "$(dirname "' + outPath + '")"',
    'cat > "' + outPath + '" <<\'' + marker + "'\n" + draftText + '\n' + marker,
  ]
}

// --- Driver -------------------------------------------------------------------

const documentArgs = parseDocumentArgs(args)
const roadmapSlug = documentArgs.roadmap
const outPath = resolveOutPath(documentArgs)

// coerceRawArgs(a) — JSON-string-tolerant read of the raw payload, so a
// stringified `args` still surfaces the optional caller-supplied hoists below.
// Never throws: anything unusable yields {} and every hoist read then falls
// through to its agent.
function coerceRawArgs(a) {
  let raw = a || {}
  if (typeof raw === 'string') {
    try {
      raw = JSON.parse(raw) || {}
    } catch (e) {
      raw = {}
    }
  }
  if (!raw || typeof raw !== 'object') raw = {}
  return raw
}
// Optional caller-supplied hoists (see docs/mechanical-agent-inventory.md). The
// rdm-document shim is already a running agent with the repo in context, so it
// runs `rdm model resolve mechanical` / `rdm roadmap show --format json` itself
// and passes the results here. Both are OPTIONAL: absent or malformed falls
// through to the original agent, which is what a direct `Workflow` invocation
// always does.
const rawDocumentArgs = coerceRawArgs(args)
// hoistedRoadmapMetaOk(m) — the shape guard, matching the fetch agent's own
// success condition: `found === true` plus an array of phases. Anything else is
// rejected so the all-done validation below can never run on a partial payload.
function hoistedRoadmapMetaOk(m) {
  return !!(m && typeof m === 'object' && m.found === true && Array.isArray(m.phases))
}

if (!roadmapSlug) {
  log('document: no roadmap slug provided')
  return { roadmap: roadmapSlug, aborted: true, incompletePhases: [], path: null, draft: null, fetchError: true }
}

// THE ONLY AGENTS THIS ENGINE DISPATCHES ARE THE PER-PHASE GATHERERS AND THE
// SYNTHESIZER, both of which read and judge. The roadmap's phase list is a read
// the ORCHESTRATOR does and passes as `roadmapMeta`; the draft is written by the
// orchestrator from the command text this returns.
if (!hoistedRoadmapMetaOk(rawDocumentArgs.roadmapMeta)) {
  const cmd = documentRoadmapCommand(roadmapSlug)
  const msg =
    'document: no `roadmapMeta` supplied — run `' +
    cmd +
    '` yourself and pass `{ found: true, slug, title, phases: [{ stem, title, status, commit }] }`. ' +
    'This engine reads nothing.'
  log(msg)
  return { roadmap: roadmapSlug, aborted: true, incompletePhases: [], path: null, draft: null, fetchError: true, roadmapCommand: cmd }
}
const roadmapMeta = rawDocumentArgs.roadmapMeta

const phases = Array.isArray(roadmapMeta.phases) ? roadmapMeta.phases : []

// All-done validation: abort BEFORE any gather/synthesis stage runs.
const incomplete = computeIncompletePhases(phases)
if (incomplete.length > 0) {
  log(
    'document: roadmap ' +
      roadmapSlug +
      ' has incomplete phase(s): ' +
      incomplete.map((p) => (p && p.stem) + ':' + (p && p.status)).join(', ')
  )
  return { roadmap: roadmapSlug, aborted: true, incompletePhases: incomplete, path: null, draft: null, fetchError: false }
}

// Stage 1: parallel per-phase gather.// Stage 1: parallel per-phase gather. A zero-phase roadmap (vacuously all-done)
// proceeds with an empty record set rather than short-circuiting — a deliberate
// choice documented alongside computeIncompletePhases above.
const gitCmdTemplate = buildGitRangeCommands('<SHA>')
async function gatherPhase(p) {
  try {
    const record = await agent(buildPhaseGatherPrompt(roadmapSlug, p, gitCmdTemplate), {
      label: 'gather:' + p.stem,
      phase: 'Gather',
      schema: PHASE_RECORD_SCHEMA,
    })
    if (record) return record
  } catch (e) {
    // fall through to the body-only placeholder below
  }
  log('document: gather failed for phase ' + p.stem + ' — falling back to body-only placeholder')
  return { stem: p.stem, title: p.title || p.stem, body: '', hasSha: false, fallback: true }
}
const phaseRecords = phases.length > 0 ? await parallel(phases.map((p) => () => gatherPhase(p))) : []
const safeRecords = phaseRecords.map(
  (r, i) => r || { stem: phases[i].stem, title: phases[i].title || phases[i].stem, body: '', hasSha: false, fallback: true }
)

// Stage 2: single synthesis agent drafts the whole document.
let synth = null
try {
  synth = await agent(buildSynthesisPrompt(roadmapMeta, safeRecords), {
    label: 'synthesize:draft',
    phase: 'Synthesize',
    schema: DRAFT_SCHEMA,
  })
} catch (e) {
  synth = null
}
const draftText = synth && typeof synth.draft === 'string' ? synth.draft : ''
if (draftText.trim() === '') {
  log('document: synthesis returned an empty draft for ' + roadmapSlug)
  return { roadmap: roadmapSlug, aborted: true, incompletePhases: [], path: null, draft: null, fetchError: true }
}

// Stage 3: return the write as command text. The ORCHESTRATOR runs it.
const writeCommands = buildWriteCommands(outPath, draftText)

log('document (' + roadmapSlug + '): draft ready for ' + outPath + ' — run the returned writeCommands')
return {
  roadmap: roadmapSlug,
  aborted: false,
  incompletePhases: [],
  path: outPath,
  draft: draftText,
  writeCommands: writeCommands,
  writeScript: writeCommands.join('\n'),
}
