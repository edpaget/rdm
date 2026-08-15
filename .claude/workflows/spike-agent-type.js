// spike-agent-type — feasibility spike for the `workflow-token-reduction`
// roadmap's mechanical-context-trim phase. It answers, from run evidence rather
// than from self-report, the three questions that phase is gated on:
//
//   Q1  Is the `agentType` registry reachable from the Workflow runtime, and
//       does `.claude/agents/rdm-mechanical.md` actually resolve there?
//   Q2  Is `effort: 'low'` HONORED, or merely accepted?
//   Q3  Does the project `CLAUDE.md` load into a custom `agentType` subagent,
//       and by how much does that move the per-agent context floor?
//
// HOW THE ANSWERS ARE READ — none of them come from what the agents say:
//
//   Q1  Compare the `toolNames` the `agentType: 'rdm-mechanical'` case reports
//       against the control's. The definition restricts tools, so a trimmed
//       list is positive evidence the definition resolved. Case C (an unknown
//       agent type) is the control for the failure mode: the runtime carries a
//       dedicated `agent({agentType}): agent type '...' not found. Available
//       agents: ...` error, so C is expected to THROW rather than resolve to
//       null the way an unknown `model` id does. Record which it actually does
//       — the distributed lane's blast radius depends on it.
//
//   Q2  Do NOT read `effort` off anything this script returns. Read it out of
//       the run's transcripts: every `assistant` record in
//       `~/.claude/projects/**/subagents/workflows/<runId>/agent-*.jsonl`
//       carries a TOP-LEVEL `effort` field, a sibling of `message.model` — the
//       same channel the earlier `model` spike used. Across the whole corpus
//       that field has only ever held `"high"` or been absent, never `"low"`,
//       so a single `"low"` record is conclusive. See
//       `docs/workflow-schemas.md` § "agentType / effort options spike".
//
//   Q3  Do NOT trust `claudeMdFact`; a model can produce a plausible value
//       without having been given the file. It is a cheap corroborating signal
//       only. The real instrument is `firstRequestTokens` from
//       `scripts/lib/token-report.mjs` (it prices the system prompt, where
//       CLAUDE.md is injected, and the transcript does not record that prompt
//       directly). The control-minus-mechanical delta IS the measured floor
//       movement, and is comparable to `docs/token-baseline.json`'s recorded
//       12052 (project CLAUDE.md) / 15312 (project + user-global).
//
// The cases are dispatched SEQUENTIALLY on purpose: a `parallel()` fan-out
// interleaves transcript writes and makes per-case first-request attribution
// harder. Every case sends one IDENTICAL prompt, so any first-request token
// difference is attributable to the options and not to prompt length.

// ---------------------------------------------------------------------------
// SECOND MODE — `mode: 'fidelity'` (added by `regularize-mechanical-agents`).
//
// The 8-case matrix above settled that `effort: 'low'` is HONORED at the call
// site. It did NOT settle the question that actually gates threading it at the
// mechanical call sites: does low effort DEGRADE mechanical transcription? A
// mechanical agent's whole job is to run one command and transcribe its output
// into a fixed schema, so a degradation there is silent data corruption — a
// wrong `ok: true`, a mis-read difficulty, an inverted signal boolean — not a
// visible failure. Nothing in the 8-case matrix can see that: every case runs
// the same introspection prompt whose answer no consumer reads.
//
// This mode is that missing instrument. It is a PAIRED A/B: the same mechanical
// prompt dispatched twice, once with `effort` absent (control — production's
// current shape) and once with `effort: 'low'` (treatment), identical in every
// other respect including `agentType` and the model pin. Pairs are dispatched
// SEQUENTIALLY for the same reason the matrix above is: clean per-case
// transcript attribution.
//
// It deliberately returns the RAW pairs and adjudicates nothing. The pass bar
// (`docs/token-baseline.json` § mechanicalContextTrim.effortFidelity.method) is
// applied by the human/agent reading the result, against the CONSUMED fields
// only — the fields real consumers branch on — because free-text fields like
// `justification` and `detail` are allowed to differ and comparing them would
// manufacture a failure.
//
// SAFETY: every prompt is templated against a caller-supplied `--root` pointing
// at a THROWAWAY plan repo. Nothing here may run against the real $RDM_ROOT:
// two of the five schema shapes are exercised by prompts that WRITE (a tag
// rewrite and a difficulty writeback). `runRoot` is required in this mode and
// the run throws without it rather than silently defaulting.
//
// RESULT (this mode HAS been run; do not re-run it expecting a different
// answer without first reading the record). Run `wf_0e8e31e2-415`, 15 pairs /
// 30 dispatches: the transcription half PASSED 15/15 — every `low` arm was
// schema-valid and equal to its `high` pair, and to the seeded known-correct
// answer, on the consumed fields. Threading was nonetheless REFUSED, for two
// reasons the study itself surfaced: the same pairs show no output-token drop
// (11831 low vs 9819 control, 8 pairs up / 7 down), and the mechanical tier
// resolves to haiku, which emits no top-level `effort` field at all in the
// whole corpus — so the treatment is unfalsifiable exactly where it would be
// threaded. `docs/token-baseline.json` § mechanicalContextTrim.effortFidelity
// is canonical, including the VOID first run `wf_8da984c5-f57` (bad `--root`
// placement) that must not be mistaken for evidence.
// ---------------------------------------------------------------------------

export const meta = {
  name: 'spike-agent-type',
  description: 'Feasibility spike: does agent() honor opts.agentType and opts.effort, and does CLAUDE.md load into a custom agent type?',
  phases: [{ title: 'Spike' }, { title: 'Fidelity' }],
}

const SPIKE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['version', 'toolNames', 'claudeMdFact'],
  properties: {
    version: { type: 'string' },
    toolNames: { type: 'array', items: { type: 'string' } },
    claudeMdFact: { type: 'string' },
  },
}

const PROMPT = [
  'Do exactly three things and return them in the structured output. Do not do anything else.',
  '',
  '1. Run this command with Bash, from the repository root, and put its exact stdout in `version`:',
  '',
  '   ./target/debug/rdm --version',
  '',
  '2. Put the names of every tool you currently have available into `toolNames`, one name per array',
  '   element, exactly as they are named in your own tool list. Do not guess and do not include a tool',
  '   you cannot actually call.',
  '',
  '3. WITHOUT running any command and WITHOUT reading any file, answer from memory of your own',
  '   instructions: what is the documented DEFAULT value of the `hook_timeout_secs` setting in this',
  "   project's CLAUDE.md? Put just the value in `claudeMdFact` (for example `30s`). If you have no",
  '   instructions describing that setting, put the literal string `UNKNOWN` in `claudeMdFact`.',
  '   Do NOT look it up. An honest `UNKNOWN` is the useful answer here.',
].join('\n')

// --- fidelity mode ---------------------------------------------------------
// The five schema shapes are copied VERBATIM from their production call sites
// rather than re-typed, so a schema change there cannot silently diverge from
// what this study certified:
//   STAMP_ACK_SCHEMA    .claude/workflows/lib/plan-review.mjs      (gate:clear-tag)
//   ACK_SCHEMA          .claude/workflows/rdm-wf-estimate.js       (estimate:write)
//   TIER_SCHEMA         .claude/workflows/rdm-wf-estimate.js       (estimate:tier)
//   ESTIMATE_SCHEMA     .claude/workflows/rdm-wf-estimate.js       (estimate:rate)
//   DIFF_SIGNALS_SCHEMA .claude/workflows/rdm-wf-dispatch-phase.js (diff:signals)
const STAMP_ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok'],
  properties: { ok: { type: 'boolean' } },
}

const ACK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['ok'],
  properties: {
    ok: { type: 'boolean' },
    detail: { type: 'string' },
  },
}

const TIER_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['model'],
  properties: {
    model: { type: 'string' },
  },
}

const ESTIMATE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['stem', 'difficulty', 'justification'],
  properties: {
    stem: { type: 'string' },
    difficulty: { type: 'string', enum: ['trivial', 'easy', 'moderate', 'hard'] },
    justification: { type: 'string' },
  },
}

const DIFF_SIGNALS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['changedFiles', 'diffText'],
  properties: {
    changedFiles: { type: 'array', items: { type: 'string' } },
    diffText: { type: 'string' },
  },
}

// The fields a real consumer BRANCHES ON, per schema. Adjudication compares
// only these; `justification`/`detail`/`diffText` are free prose whose wording
// legitimately differs between two dispatches of the same prompt, and diffing
// them would manufacture a failure the pass bar never intended.
const CONSUMED_FIELDS = {
  STAMP_ACK: ['ok'],
  ACK: ['ok'],
  TIER: ['model'],
  ESTIMATE: ['stem', 'difficulty'],
  DIFF_SIGNALS: ['changedFiles'],
}

// coerceArgs — the Workflow tool may hand `args` through as a JSON string.
// Mirrors rdm-wf-backlog.js's coerceRawArgs rather than inventing a new shape.
function coerceArgs(a) {
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

// buildFidelityCases(cfg) — the 5 x 3 prompt matrix, pure so it can be reasoned
// about (and diffed) without dispatching anything.
//
// FALSE-PASS DEFENCE: a prompt whose correct answer is constant cannot detect
// degradation — an agent that ignored the command entirely and guessed
// `{ ok: true }` would score identically to one that ran it. So every write
// shape includes an instance whose CORRECT answer is the negative one
// (`ok: false`, via a deliberately nonexistent stem), and every read shape
// asks for a value seeded into the throwaway repo and therefore known
// independently of what the agent says.
function buildFidelityCases(cfg) {
  // `--root` is a GLOBAL rdm flag: it must sit between the binary and the
  // subcommand. Appending it after the subcommand's own arguments — which the
  // first build of this instrument did — is rejected outright with
  // `error: unexpected argument '--root' found`, which collapses every write
  // shape to a constant `ok: false` and silently destroys the discrimination
  // this study depends on. Run wf_8da984c5-f57 is the recorded instance.
  // §2b-fid check (7) gates the placement so it cannot regress.
  const rdm = cfg.rdmBin + ' --root ' + cfg.runRoot
  const projFlag = cfg.project ? ' --project ' + cfg.project : ''
  const slug = cfg.roadmap
  const good = cfg.phaseStems
  const bogus = 'phase-99-no-such-phase-xyz'

  const stampPrompt = (stem) =>
    [
      'You are a mechanical status agent. Do not plan, implement, or review anything.',
      'Run exactly these two commands, verbatim and unmodified, and read their exit codes:',
      '  ' + rdm + ' phase update ' + stem + ' --roadmap ' + slug + ' --tags "" --no-edit' + projFlag,
      '  ' + rdm + ' commit -m "chore(plan): fidelity probe ' + stem + '"',
      'Return { "ok": true } if BOTH commands exited 0, otherwise { "ok": false }.',
      'Do NOT repair, reorder, or re-run a failing command with different arguments.',
      'Report what actually happened. A nonzero exit is a legitimate and expected answer here.',
    ].join('\n')

  const ackPrompt = (stem, difficulty) =>
    [
      'You are a mechanical write agent. Do not plan, implement, or review anything.',
      'Run exactly this command, verbatim and unmodified, and read its exit code:',
      '  ' +
        rdm +
        ' phase update ' +
        stem +
        ' --roadmap ' +
        slug +
        ' --difficulty ' +
        difficulty +
        ' --no-edit' +
        projFlag,
      'Return { "ok": true, "detail": "<the command\'s own message, verbatim>" } if it exited 0,',
      'otherwise { "ok": false, "detail": "<the error it printed, verbatim>" }.',
      'Do NOT repair, reorder, or re-run a failing command with different arguments.',
      'Report what actually happened. A nonzero exit is a legitimate and expected answer here.',
    ].join('\n')

  const tierPrompt = (stem) =>
    [
      'You are a mechanical read agent. Do not plan, implement, review, or compute anything.',
      'Run exactly this command, verbatim and unmodified:',
      '  ' + rdm + ' phase show ' + stem + ' --roadmap ' + slug + ' --format json' + projFlag,
      'Return { "model": "<the value of the returned JSON object\'s `model` field, verbatim>" }.',
      'Do NOT derive, map, or infer the model from the difficulty — copy the field.',
      'Do NOT repair, reorder, or re-run a failing command with different arguments.',
      'If the command fails or the field is absent, return { "model": "" }.',
    ].join('\n')

  const estimatePrompt = (stem) =>
    [
      'You are a mechanical transcription agent. Do not rate, judge, or compute anything yourself.',
      'Run exactly this command, verbatim and unmodified:',
      '  ' + rdm + ' phase show ' + stem + ' --roadmap ' + slug + ' --format json' + projFlag,
      'The returned `body` contains a line of the form:',
      '  ## Estimate <difficulty> — <justification>',
      'Transcribe it. Return { "stem": "' + stem + '", "difficulty": "<the difficulty word on that line>",',
      '"justification": "<the text after the em-dash>" }.',
      'The difficulty MUST be copied from that line, not decided by you.',
      'Do NOT repair, reorder, or re-run a failing command with different arguments.',
    ].join('\n')

  const diffPrompt = (base) =>
    [
      'You are a mechanical diff agent. Do not plan, implement, or review anything.',
      'Run exactly these two commands, verbatim and unmodified, in ' + cfg.sourceRoot + ':',
      '  git diff --name-only ' + base + '...HEAD',
      '  git diff ' + base + '...HEAD',
      'Do NOT repair, reorder, or re-run a failing command with different arguments.',
      'Return { "changedFiles": [<one array element per line of the FIRST command\'s output, verbatim>],',
      '"diffText": "<the SECOND command\'s output, truncated to the first 40000 characters>" }.',
      'Do not add, reorder, normalize, or drop any path. If the first command prints nothing,',
      'return an empty array.',
    ].join('\n')

  const cases = []
  const push = (schemaName, schema, i, prompt) => cases.push({ schemaName, schema, i, prompt })

  // STAMP_ACK — two true-answer instances, one whose correct answer is false.
  push('STAMP_ACK', STAMP_ACK_SCHEMA, 1, stampPrompt(good[0]))
  push('STAMP_ACK', STAMP_ACK_SCHEMA, 2, stampPrompt(good[1]))
  push('STAMP_ACK', STAMP_ACK_SCHEMA, 3, stampPrompt(bogus))
  // ACK — same shape, plus a distinct difficulty per instance so a constant
  // answer is detectable downstream by the TIER read.
  push('ACK', ACK_SCHEMA, 1, ackPrompt(good[0], 'trivial'))
  push('ACK', ACK_SCHEMA, 2, ackPrompt(good[1], 'hard'))
  push('ACK', ACK_SCHEMA, 3, ackPrompt(bogus, 'easy'))
  // TIER — reads back what ACK just wrote; the correct answers differ per stem.
  push('TIER', TIER_SCHEMA, 1, tierPrompt(good[0]))
  push('TIER', TIER_SCHEMA, 2, tierPrompt(good[1]))
  push('TIER', TIER_SCHEMA, 3, tierPrompt(good[2]))
  // ESTIMATE — transcription, NOT rating. The production estimate:rate site is
  // a JUDGMENT site and is never threaded; what this certifies is the schema
  // SHAPE under a mechanical prompt, which is what the trimmed agent would face.
  push('ESTIMATE', ESTIMATE_SCHEMA, 1, estimatePrompt(good[0]))
  push('ESTIMATE', ESTIMATE_SCHEMA, 2, estimatePrompt(good[1]))
  push('ESTIMATE', ESTIMATE_SCHEMA, 3, estimatePrompt(good[2]))
  // DIFF_SIGNALS — three different bases over the seeded source repo, so the
  // correct changedFiles set differs per instance.
  push('DIFF_SIGNALS', DIFF_SIGNALS_SCHEMA, 1, diffPrompt(cfg.diffBases[0]))
  push('DIFF_SIGNALS', DIFF_SIGNALS_SCHEMA, 2, diffPrompt(cfg.diffBases[1]))
  push('DIFF_SIGNALS', DIFF_SIGNALS_SCHEMA, 3, diffPrompt(cfg.diffBases[2]))
  return cases
}

const spikeArgs = coerceArgs(args)

if (spikeArgs.mode === 'fidelity') {
  // Required, and deliberately un-defaulted: two of the five shapes WRITE.
  // Defaulting `runRoot` to the ambient repo would point a tag rewrite and a
  // difficulty writeback at the real plan repo.
  if (!spikeArgs.runRoot || !spikeArgs.sourceRoot) {
    throw new Error(
      'spike-agent-type fidelity mode requires { runRoot, sourceRoot } pointing at THROWAWAY repos — ' +
        'two of the five schema shapes write to the plan repo, so there is no safe default'
    )
  }
  const cfg = {
    rdmBin: spikeArgs.rdmBin || './target/debug/rdm',
    runRoot: spikeArgs.runRoot,
    sourceRoot: spikeArgs.sourceRoot,
    project: spikeArgs.project || null,
    roadmap: spikeArgs.roadmap || 'fidelity-probe',
    phaseStems: spikeArgs.phaseStems || ['phase-1-alpha', 'phase-2-beta', 'phase-3-gamma'],
    diffBases: spikeArgs.diffBases || ['HEAD~3', 'HEAD~2', 'HEAD~1'],
    model: spikeArgs.model || 'haiku',
  }
  const fidelityCases = buildFidelityCases(cfg)

  // Each pair: control (effort ABSENT — production's current shape) then
  // treatment (effort: 'low'), same prompt string, same everything else.
  // `agentType` is on BOTH arms so the study measures the effort axis alone
  // against the already-shipped trimmed definition, not the two changes at once.
  const pairs = []
  for (const c of fidelityCases) {
    const baseOpts = {
      label: 'fidelity:' + c.schemaName + ':' + c.i,
      phase: 'Fidelity',
      schema: c.schema,
      agentType: 'rdm-mechanical',
      model: cfg.model,
    }
    const arm = async (suffix, extra) => {
      try {
        const v = await agent(c.prompt, Object.assign({}, baseOpts, { label: baseOpts.label + ':' + suffix }, extra))
        return { value: v, error: '' }
      } catch (err) {
        return { value: null, error: String((err && err.message) || err) }
      }
    }
    const high = await arm('control', {})
    const low = await arm('low', { effort: 'low' })
    log(
      'fidelity ' +
        c.schemaName +
        '#' +
        c.i +
        ' control=' +
        (high.error || JSON.stringify(high.value)) +
        ' low=' +
        (low.error || JSON.stringify(low.value))
    )
    pairs.push({
      schema: c.schemaName,
      i: c.i,
      consumedFields: CONSUMED_FIELDS[c.schemaName],
      prompt: c.prompt,
      high: high.value,
      highErr: high.error,
      low: low.value,
      lowErr: low.error,
    })
  }
  // Raw pairs only — adjudication is the reader's, against the recorded pass
  // bar. Returning a verdict here would let the instrument grade itself.
  return { mode: 'fidelity', cfg, consumedFields: CONSUMED_FIELDS, pairs }
}

// The agentType axis crossed with the effort axis. A and B are the Q3 pair whose
// firstRequestTokens delta is the measured floor movement — they must differ in
// NOTHING but `agentType`. E/F pairs the effort value with and without the
// custom type so the two options' interaction is observed, not assumed.
const cases = [
  { name: 'A-control', opts: {} },
  { name: 'B-agentType-valid', opts: { agentType: 'rdm-mechanical' } },
  { name: 'C-agentType-unknown', opts: { agentType: 'no-such-agent-xyz' } },
  { name: 'D-agentType-undefined', opts: { agentType: undefined } },
  { name: 'E-effort-low', opts: { effort: 'low' } },
  { name: 'F-effort-low-plus-agentType', opts: { effort: 'low', agentType: 'rdm-mechanical' } },
  { name: 'G-effort-undefined', opts: { effort: undefined } },
  { name: 'H-effort-invalid', opts: { effort: 'not-an-effort-xyz' } },
]

const results = []
for (const c of cases) {
  let value = null
  let error = ''
  try {
    value = await agent(PROMPT, Object.assign({ label: 'spike:' + c.name, phase: 'Spike', schema: SPIKE_SCHEMA }, c.opts))
  } catch (err) {
    error = String((err && err.message) || err)
  }
  // A `null` return WITHOUT a throw is the model-spike failure shape (silent
  // resolution failure). Distinguish it from a throw in the log so case C's
  // answer is unambiguous when the transcripts are read back.
  const shape = error ? 'THREW' : value === null ? 'NULL (no throw)' : 'OK'
  log('spike-agent-type ' + c.name + ': ' + shape + ' ' + (error || JSON.stringify(value)))
  results.push({ case: c.name, opts: c.opts, shape, error, result: value })
}

return { results }
