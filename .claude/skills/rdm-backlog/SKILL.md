---
name: rdm-backlog
description: Run a batched backlog grooming pass over rdm's stale/duplicate/tag-cluster/archivable signals and emit a reviewable, propose-only plan of exact rdm commands — no mutations
allowed-tools:
  - Bash
  - Workflow
---

Run a batched **grooming pass** over the backlog by invoking the **`rdm-wf-backlog` Workflow** (`.claude/workflows/rdm-wf-backlog.js`, provisioned automatically by `rdm agent-config claude --skills`). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and prints the batched plan the workflow returns. All the analysis — the category registry, the per-category grooming rules (retire-vs-consolidate, survivor-pick, existing-roadmap check, archive rationale), and the batch consolidation — lives in the workflow (`.claude/workflows/lib/backlog.mjs`), not in this prose.

**Non-mutation guarantee:** the workflow runs exactly ONE Bash-executing step — `rdm backlog report` (read-only) — and every analyzer agent is explicitly instructed to propose text only, never to execute a mutating command. It never runs `create`, `update`, `merge`, `archive`, `promote`, `rdm commit`, or `rdm discard`. Every action is a `{command, rationale}` pair for a human to run later, never executed here. The `allowed-tools` above deliberately omit `Read`/`Write`/`Edit`: this skill's only output is its final chat message.

## Contract

**Input** (`$ARGUMENTS`): all optional — `[--project <name>]` (standard resolution chain applies when omitted), `[--older-than <days>]`, `[--tag <tag>]`. There is no positional argument naming an item to change, because this skill changes nothing.

## What to do

1. **Parse `$ARGUMENTS`** into `{ project, olderThan, tag }`, omitting any field not supplied.
2. **Gather the two mechanical values yourself and add them to that object.** You are already a running agent with the repo in context; the workflow is not, so each of these otherwise costs it a whole dedicated subagent. Both are **optional** — the workflow falls back to its own in-workflow fetch for anything you omit or get wrong. Neither weakens the propose-only contract: both commands are read-only.
   - `mechanicalModel` — the id printed by `./target/debug/rdm model resolve mechanical`, verbatim.
   - `report` — the parsed object from `./target/debug/rdm backlog report --format json` (adding `--older-than <days>` / `--tag <tag>` when supplied), passed through **verbatim**, never summarized. It must carry all four signal arrays (`stale_tasks`, `duplicate_clusters`, `tag_clusters`, `archivable_roadmaps`) or the workflow rejects it and fetches its own.
3. **Invoke the `rdm-wf-backlog` workflow** via the Workflow tool with that object (`{ project, olderThan, tag, mechanicalModel, report }`). Pass `args` as a JSON object, never a stringified value. The workflow:
   - uses the `report` you supplied, or runs `rdm backlog report --format json` itself once (its only Bash-executing step);
   - fans one READ-ONLY analyzer agent out per populated signal category (`stale_tasks`, `duplicate_clusters`, `tag_clusters`, `archivable_roadmaps`) in parallel;
   - consolidates the results into one ordered batch — a subsection per category that produced a proposal, plus a merged `## Open questions` section for anything it could not confidently resolve;
   - short-circuits to `{ groomed: false, summary: "Nothing to groom — the backlog report returned no signals" }` when all four categories are empty.
4. **Print the returned `summary` field verbatim** as your final message — the whole grooming plan (or the "Nothing to groom" message). Do not paraphrase, re-order, or drop any proposal.

See [`docs/workflow-schemas.md`](../../../docs/workflow-schemas.md) for the full workflow contract.
