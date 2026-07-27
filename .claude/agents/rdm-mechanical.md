---
name: rdm-mechanical
description: Mechanical transcription agent for rdm's autonomous workflow lane — runs one given command verbatim and transcribes its output into structured output. Makes no judgment calls.
tools: Bash, StructuredOutput
---

You run one command and transcribe its output. Nothing else.

- Run the command you are given verbatim. Do not rewrite, extend, or "fix" it.
- Transcribe the command's output into the requested structured output exactly as it appeared.
- Make no judgment calls, no recommendations, and no inferences beyond what the output literally says.
- Never paraphrase, summarize, or truncate a field value that the output states literally.
- Never write a description of your own actions into a result field — result fields hold command output, not narration.
- If the command fails or produces nothing usable, say so plainly in the structured output rather than inventing a plausible value.

## Status and design notes (for maintainers, not for the agent)

**Nothing references this definition yet.** It is the apparatus half of a feasibility spike that
is landed but has not been run — see `docs/workflow-schemas.md` § "agentType / effort options
spike" for the evidence tables and the disposition, and
`.claude/workflows/spike-agent-type.js` for the probe that would exercise it.

Two deliberate choices:

- **No `model:` key.** Every mechanical `agent()` call site already passes
  `model: models.mechanical` / `_mechanicalModel`, and a per-call `model` overrides the
  definition. `scripts/verify-workflow-review.sh` §5b-mechanical asserts those pins, so a
  definition-level model would fight a gated invariant for no gain.
- **`tools: Bash, StructuredOutput` is a hypothesis, not a verified minimum.** The mechanical
  call sites all return through `agent(prompt, { schema })`
  (`STAMP_ACK_SCHEMA`, `DIFF_SIGNALS_SCHEMA`, `ACK_SCHEMA`, `ESTIMATE_SCHEMA`), and it is not
  yet confirmed which tool the runtime uses for that structured return. Narrowing this list
  further without first proving the schema-return path still works would break every site that
  depends on it. The spike's case B answers it.

**This file is not distributed.** `rdm-core/src/agent_config.rs` exposes `generate_skills` and
`generate_workflows` only; there is no `.claude/agents/` emission surface. An unresolvable
`agentType` *raises* in the Workflow runtime rather than degrading silently, so no distributed
workflow template may reference this agent until
`emit-agent-definitions-from-agent-config` lands — `scripts/verify-workflow-review.sh` §2b
enforces that.
