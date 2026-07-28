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
  name: 'document',
  description:
    'Headlessly draft user documentation from a completed rdm roadmap (phase bodies + commit diffs) and write it to disk',
  phases: [{ title: 'Fetch' }, { title: 'Gather' }, { title: 'Synthesize' }, { title: 'Write' }],
}

// The block below is copied BYTE-IDENTICAL from
// .claude/workflows/lib/document.mjs — do NOT edit it here. Edit the lib and
// scripts/verify-workflow-document.sh fails the build on drift.
// >>> document-core:begin <<<
// Pure, deterministic decision logic for the document workflow.
//
// This block is the single source of truth in
// .claude/workflows/lib/document.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/document.js (the Workflow runtime cannot load modules at run
// time). scripts/verify-workflow-document.sh gates the two copies for drift.
// No Date.now / Math.random — pure array/string ops only.

// parseDocumentArgs(args) — coerce and default the whole args payload.
//
// The Workflow tool contract forbids stringified args, but LLM callers (the
// rdm-document skill shim, or a hand-run invocation) may still deliver a JSON
// string; coerce once, mirroring parseDispatchArgs in lib/dispatch-phase.mjs.
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
// this is a deliberate, documented choice (see document.js's driver), not an
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

// ROADMAP_META — Stage 0's mechanical fetch of the roadmap's phase list.
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

// WRITE_ACK — Stage 3's mechanical write result.
const WRITE_ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok', 'path'],
  properties: { ok: { type: 'boolean' }, path: { type: 'string' } },
}

// --- Prompt builders ----------------------------------------------------------

// buildMechanicalModelPrompt() — a mechanical Bash agent that resolves the
// mechanical dispatch step to a concrete model id, ONCE per run, before any
// other mechanical agent fires. This is deliberately the one call in the whole
// run left UNSIZED (mirrors dispatch-phase's Stage-0 fetch:phase-meta/
// fetch:task-meta exemption and autopilot's own model:mechanical bootstrap,
// both recorded in their respective verify-workflow-*.sh AC-MODEL bootstrap
// whitelists): it is the call that produces the model id every other
// mechanical agent below runs on, so it cannot know its own model before
// running.
function buildMechanicalModelPrompt() {
  return [
    'You are a mechanical fetch agent. Do not plan or implement anything.',
    'Run exactly this command in the repo root and read its printed output:',
    '  ./target/debug/rdm model resolve mechanical',
    'Return the printed model id verbatim as JSON { "model": "<id>" }.',
    'If the command fails or prints nothing, return { "model": "" }.',
  ].join('\n')
}

// MECHANICAL_MODEL — the resolved `rdm model resolve mechanical` id, from the
// one bootstrap call made before Stage 0.
const MECHANICAL_MODEL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['model'],
  properties: {
    model: { type: 'string' },
  },
}

// Stage 0: a mechanical Bash agent reads the roadmap's phase list (the runtime
// cannot shell out itself).
function buildRoadmapFetchPrompt(slug) {
  return [
    'You are a mechanical fetch agent. Do not plan, implement, or review anything.',
    'Run exactly this command in the repo root and read its JSON output:',
    '  ./target/debug/rdm roadmap show ' + slug + ' --project rdm --format json --no-body',
    'If the command exits non-zero, or reports that the roadmap was not found, return exactly:',
    '  { "found": false, "slug": "' + slug + '", "title": "", "phases": [] }',
    'Otherwise return a ROADMAP_META object: found:true, slug (the roadmap JSON `slug`),',
    'title (the roadmap JSON `title`), and phases — one entry per phase with stem (the phase JSON `stem`),',
    'title (the phase JSON `title`), and status (the phase JSON `status`), each taken verbatim.',
    'A roadmap that genuinely has zero phases still has found:true — only an unresolvable roadmap gets found:false.',
  ].join('\n')
}

// Stage 1: one mechanical Bash agent per phase, run inside parallel(). Fetches
// the phase body + commit, then conditionally gathers git history for that
// commit — falling back to body-only when there is no SHA or the git commands
// fail. `gitCmdTemplate` is buildGitRangeCommands('<SHA>') — a placeholder
// rendering the agent substitutes the real commit value into, so the exact
// command text the agent runs always matches the pure function's format.
function buildPhaseGatherPrompt(roadmap, phase, gitCmdTemplate) {
  return [
    'You are a mechanical gather agent. Do not plan, review, or implement anything, and edit no files.',
    'Run exactly this command in the repo root and read its JSON output:',
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

// Stage 3: a mechanical Bash agent writes the draft to disk — the Workflow
// runtime has no filesystem access of its own. Creates parent directories first
// so an --out path with no existing parent still succeeds.
function buildWritePrompt(outPath, draftText) {
  const marker = 'RDM_DOCUMENT_DRAFT_EOF'
  return [
    'You are a mechanical write agent. Do not edit any other files.',
    'Write the exact text between the two ' + marker + ' lines below to the path "' + outPath + '"',
    'in the repo root, creating any missing parent directories first. Run exactly these commands:',
    '  mkdir -p "$(dirname "' + outPath + '")"',
    '  cat > "' + outPath + '" <<\'' + marker + "'",
    draftText,
    marker,
    'Return a WRITE_ACK object: { ok: true, path: "' +
      outPath +
      '" } if BOTH commands exited 0, otherwise { ok: false, path: "' +
      outPath +
      '" }.',
  ].join('\n')
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

// Resolve the mechanical model ONCE, before any other mechanical agent runs
// (including Stage 0's roadmap fetch). An unresolved result stops the run
// before any mechanical agent fires, rather than silently falling through to
// an unpinned Stage 0/1/3 agent.
// HOIST: the caller already ran `rdm model resolve mechanical`.
let mechanicalModel = ''
let mechanicalErr = ''
if (typeof rawDocumentArgs.mechanicalModel === 'string' && rawDocumentArgs.mechanicalModel.trim() !== '') {
  mechanicalModel = rawDocumentArgs.mechanicalModel.trim()
  log('document: mechanical model hoisted from caller args')
} else {
  try {
    const mechanicalModelResult = await agent(buildMechanicalModelPrompt(), {
      label: 'model:mechanical',
      phase: 'Fetch',
      agentType: 'rdm-mechanical',
      schema: MECHANICAL_MODEL_SCHEMA,
    })
    mechanicalModel = mechanicalModelResult && typeof mechanicalModelResult.model === 'string' ? mechanicalModelResult.model.trim() : ''
  } catch (e) {
    mechanicalModel = ''
    mechanicalErr = String((e && e.message) || e)
  }
}
if (!mechanicalModel) {
  log('document: mechanical model could not be resolved (' + (mechanicalErr || 'rdm model resolve mechanical returned nothing') + ') — stopping before any mechanical agent runs')
  return { roadmap: roadmapSlug, aborted: true, incompletePhases: [], path: null, draft: null, fetchError: true }
}

// Stage 0: fetch the roadmap's phase list via a mechanical Bash agent.
// HOIST: the caller already ran `rdm roadmap show --format json`.
let roadmapMeta = null
if (hoistedRoadmapMetaOk(rawDocumentArgs.roadmapMeta)) {
  roadmapMeta = rawDocumentArgs.roadmapMeta
  log('document: roadmap meta hoisted from caller args')
} else {
  try {
    roadmapMeta = await agent(buildRoadmapFetchPrompt(roadmapSlug), {
      label: 'fetch:roadmap-meta',
      phase: 'Fetch',
      agentType: 'rdm-mechanical',
      schema: ROADMAP_META_SCHEMA,
      model: mechanicalModel,
    })
  } catch (e) {
    roadmapMeta = null
  }
}

// Unresolvable roadmap: mirror dispatch-phase's fetchError short-circuit rather
// than silently proceeding as if the roadmap had zero phases.
if (!roadmapMeta || roadmapMeta.found !== true) {
  log('document: roadmap fetch failed for ' + roadmapSlug)
  return { roadmap: roadmapSlug, aborted: true, incompletePhases: [], path: null, draft: null, fetchError: true }
}

const phases = Array.isArray(roadmapMeta.phases) ? roadmapMeta.phases : []

// All-done validation: abort BEFORE any parallel()/synthesis/write stage runs.
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

// Stage 1: parallel per-phase gather. A zero-phase roadmap (vacuously all-done)
// proceeds with an empty record set rather than short-circuiting — a deliberate
// choice documented alongside computeIncompletePhases above.
const gitCmdTemplate = buildGitRangeCommands('<SHA>')
async function gatherPhase(p) {
  try {
    const record = await agent(buildPhaseGatherPrompt(roadmapSlug, p, gitCmdTemplate), {
      label: 'gather:' + p.stem,
      phase: 'Gather',
      agentType: 'rdm-mechanical',
      schema: PHASE_RECORD_SCHEMA,
      model: mechanicalModel,
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

// Stage 3: mechanical Bash agent writes the draft to disk.
let writeAck = null
try {
  writeAck = await agent(buildWritePrompt(outPath, draftText), {
    label: 'write:draft',
    phase: 'Write',
    agentType: 'rdm-mechanical',
    schema: WRITE_ACK_SCHEMA,
    model: mechanicalModel,
  })
} catch (e) {
  writeAck = null
}
if (!writeAck || writeAck.ok !== true) {
  log('document: write failed for ' + outPath)
  return { roadmap: roadmapSlug, aborted: true, incompletePhases: [], path: null, draft: draftText, fetchError: true }
}

log('document (' + roadmapSlug + '): draft written to ' + (writeAck.path || outPath))
return { roadmap: roadmapSlug, aborted: false, incompletePhases: [], path: writeAck.path || outPath, draft: draftText }
