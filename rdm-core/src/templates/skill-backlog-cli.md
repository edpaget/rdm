---
name: rdm-backlog
description: Run a batched backlog grooming pass over rdm's stale/duplicate/tag-cluster/archivable signals and emit a reviewable, propose-only plan of exact rdm commands — no mutations
allowed-tools:
  - Bash
  - Workflow
---

Run a batched **grooming pass** over the backlog by invoking the **`rdm-wf-backlog` Workflow** (`.claude/workflows/rdm-wf-backlog.js`, provisioned automatically by `rdm agent-config claude --skills`). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and prints the batched plan the workflow returns. All the analysis — the category registry, the per-category grooming rules (retire-vs-consolidate, survivor-pick, existing-roadmap check, archive rationale), and the batch consolidation — lives in the workflow, not in this prose.

**Non-mutation guarantee:** the workflow runs NO Bash at all — it dispatches read-only analyzer agents and nothing else — and every analyzer agent is explicitly instructed to propose text only, never to execute a mutating command. It never runs `create`, `update`, `merge`, `archive`, `promote`, `rdm commit`, or `rdm discard`. Every action is a `{command, rationale}` pair for a human to run later, never executed here. The `allowed-tools` above deliberately omit `Read`/`Write`/`Edit`: this skill's only output is its final chat message.
{principles}
## Contract

**Input** (`$ARGUMENTS`): all optional — `[--rdm-bin <path>]`, `[--project <name>]`, `[--older-than <days>]`, `[--tag <tag>]`. There is no positional argument naming an item to change, because this skill changes nothing.

## What to do

1. **Parse `$ARGUMENTS`** into a config object, omitting any field not supplied:
   - `project` — the supplied `--project` value, else the project name used in `{proj_flag}`.
   - `olderThan`, `tag` — the supplied `--older-than` / `--tag` values.
   - `rdmBin` — the value following `--rdm-bin`; when not supplied, `$RDM_BIN` if set, else a plain `rdm` on `PATH`; the executable every command below invokes (`<rdmBin>`). The workflow names it in the report command and in every proposal it hands back.
2. **Run the report yourself and add it to that object.** The workflow reads nothing — it has no agent that can run a command — so this is REQUIRED, not a hoist. It does not weaken the propose-only contract: the command is read-only whoever runs it.
   - `report` — the parsed object from `<rdmBin> backlog report --format json {proj_flag}` (substituting a supplied `--project`, and adding `--older-than <days>` / `--tag <tag>` when supplied), passed through **verbatim**, never summarized. It must carry all four signal arrays (`stale_tasks`, `duplicate_clusters`, `tag_clusters`, `archivable_roadmaps`); without it the workflow refuses to run and returns the exact command to use as `reportCommand`.
3. **Invoke the `rdm-wf-backlog` workflow** via the Workflow tool with that object (`{ project, olderThan, tag, rdmBin, report }`). Pass `args` as a JSON object, never a stringified value. The workflow:
   - fans one READ-ONLY analyzer agent out per populated signal category (`stale_tasks`, `duplicate_clusters`, `tag_clusters`, `archivable_roadmaps`) in parallel;
   - consolidates the results into one ordered batch — a subsection per category that produced a proposal, plus a merged `## Open questions` section for anything it could not confidently resolve;
   - short-circuits to `{ groomed: false, summary: "Nothing to groom — the backlog report returned no signals" }` when all four categories are empty.
4. **Print the returned `summary` field verbatim** as your final message — the whole grooming plan (or the "Nothing to groom" message). Do not paraphrase, re-order, or drop any proposal.
