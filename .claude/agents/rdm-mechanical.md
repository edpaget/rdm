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

**Nothing references this definition, and as of the spike run nothing CAN.** This is now a
measured blocker, not a deliberate hold.

This file *does* resolve through the CLI's session-agent path: `claude --agent rdm-mechanical
-p …` runs it, and a controlled 2×2 measures it at **27190 first-request tokens against the
default agent's 47084 — a 19894-token (−42 %) saving**, replicated with and without the project
`CLAUDE.md`. The prize is real.

**But it does not resolve through `agent({ agentType })` from inside a Workflow run**, which is
the path every call site would use. Spike `wf_2bea58b9-38f` (and retry probe `wf_6cca94eb-de0`)
raised `agent type 'rdm-mechanical' not found. Available agents: claude, claude-code-guide,
Explore, general-purpose, Plan, statusline-setup` — a registry containing no project-local
definition at all. Copying this file into the dispatching session's project root before the run
did not help, and a retry minutes later failed identically: the registry is a session-start
snapshot of the *session's* root, not of the workflow script's directory.

The consequence for anyone tempted to wire this up: **an `agentType` literal raises on first
dispatch from any session whose start-of-session registry lacks this file** — including every
session rooted outside this worktree. That applies to the four local-only workflows as much as
to the three distributed ones, and `scripts/verify-workflow-review.sh` §2b would not catch it,
because it greps only the distributed templates. Making a project-local definition resolvable
from a Workflow run is an unsolved prerequisite, upstream of the distribution work below.

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
