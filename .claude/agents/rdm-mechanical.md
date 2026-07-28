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

**Nothing references this definition yet** — a deliberate hold pending one verification step.

This file *does* resolve through the CLI's session-agent path: `claude --agent rdm-mechanical
-p …` runs it, and a controlled 2×2 measures it at **27190 first-request tokens against the
default agent's 47084 — a 19894-token (−42 %) saving**, replicated with and without the project
`CLAUDE.md`. The prize is real.

**What is not yet confirmed is resolution through `agent({ agentType })` from inside a Workflow
run**, the path every call site would use. Spike `wf_2bea58b9-38f` attempted it and raised
`agent type 'rdm-mechanical' not found`, but **that result is invalid and has been retracted**:
the run was dispatched from a session whose project root had no `.claude/agents/` directory at
session start, with this file copied in mid-session. Per Claude Code's subagent docs the
watcher "covers only directories that existed when the session started, so after creating a
scope's first agent file in a new `agents` directory, restart to load it" — so the definition
was never loaded, and the spike measured that, not the runtime.

To close it: dispatch `.claude/workflows/spike-agent-type.js` from a session whose project root
contains this file **at session start**. Once this branch lands, the definition sits at the repo
root, so any ordinary session in this repo qualifies. Read case B's `toolNames` — a trimmed list
is the positive evidence.

See `docs/workflow-schemas.md` § "agentType / effort options spike" for the evidence tables and
the disposition.

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
  depends on it. The spike's case B was meant to answer it and **could not** — case B threw on
  registry lookup, so nothing ever ran under this tool list. It stays a hypothesis.

**This file is not distributed.** `rdm-core/src/agent_config.rs` exposes `generate_skills` and
`generate_workflows` only; there is no `.claude/agents/` emission surface. An unresolvable
`agentType` *raises* in the Workflow runtime rather than degrading silently, so no distributed
workflow template may reference this agent until
`ship-mechanical-agent-type-downstream` lands — `scripts/verify-workflow-review.sh` §2b
enforces that.
