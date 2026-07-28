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

**This definition is live.** The mechanical `agent()` call sites of the four local-only
workflows (`document.js`, `backlog.js`, `estimate.js`, `plan-review.js`) resolve against it —
19 records in all, asserted bidirectionally by `scripts/verify-workflow-review.sh` §2c. It is
**not** used by the three distributed workflows, and not by any judgment agent.

This file *does* resolve through the CLI's session-agent path: `claude --agent rdm-mechanical
-p …` runs it, and a controlled 2×2 measures it at **27190 first-request tokens against the
default agent's 47084 — a 19894-token (−42 %) saving**, replicated with and without the project
`CLAUDE.md`. The prize is real.

**Resolution through `agent({ agentType })` from inside a Workflow run is CONFIRMED**
(2026-07-28, run `wf_40f5594e-208`): case B resolved and reported
`toolNames: ["Bash", "StructuredOutput"]`, so both the registry lookup and the tool
restriction are in force on the path the call sites actually use.

An earlier spike appeared to show the opposite; that result was invalid — it was dispatched from
a session whose project root had no `.claude/agents/` directory at session start, which the
subagent docs name as a restart case, so the definition was never loaded.

**Measured saving on the Workflow path: 8907 tokens per agent (−23 %)** — 38689 for the default
agent against 29782 for this one, reproduced exactly across two independent case pairs. Note
this is roughly **half** the 19894 (−42 %) the `claude -p` 2×2 measures; a Workflow subagent's
default floor is already much leaner than a CLI session's, so the margin is smaller. Quote 8907
for these call sites.

See `docs/workflow-schemas.md` § "agentType / effort options spike" for the evidence tables and
the disposition.

Two deliberate choices:

- **No `model:` key.** Every mechanical `agent()` call site already passes
  `model: models.mechanical` / `_mechanicalModel`, and a per-call `model` overrides the
  definition. `scripts/verify-workflow-review.sh` §5b-mechanical asserts those pins, so a
  definition-level model would fight a gated invariant for no gain.
- **`tools: Bash, StructuredOutput` is now a VERIFIED working set** — though still not a proven
  *minimum*. The mechanical call sites all return through `agent(prompt, { schema })`
  (`STAMP_ACK_SCHEMA`, `DIFF_SIGNALS_SCHEMA`, `ACK_SCHEMA`, `ESTIMATE_SCHEMA`), and the 2026-07-28
  spike confirmed that path survives this list: case B ran `Bash` and returned a valid schema'd
  object under exactly these two tools, as did both threaded agents in the live `backlog`
  dispatch. Do not narrow the list further without re-proving the schema-return path — every
  threaded site depends on it.

**This file is not distributed.** `rdm-core/src/agent_config.rs` exposes `generate_skills` and
`generate_workflows` only; there is no `.claude/agents/` emission surface. An unresolvable
`agentType` *raises* in the Workflow runtime rather than degrading silently, so no distributed
workflow template may reference this agent until
`ship-mechanical-agent-type-downstream` lands — `scripts/verify-workflow-review.sh` §2b
enforces that.
