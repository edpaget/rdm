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

export const meta = {
  name: 'spike-agent-type',
  description: 'Feasibility spike: does agent() honor opts.agentType and opts.effort, and does CLAUDE.md load into a custom agent type?',
  phases: [{ title: 'Spike' }],
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
