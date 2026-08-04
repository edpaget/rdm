---
name: rdm-estimate
description: Rate each phase's difficulty and assign a model tier from its body
allowed-tools:
  - Bash
  - Workflow
---

Rate the difficulty of an rdm roadmap's phases by invoking the **`rdm-wf-estimate` Workflow** (`.claude/workflows/rdm-wf-estimate.js`, provisioned automatically by `rdm agent-config claude --skills`). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and prints the summary the workflow returns. All the loop logic — listing the phases, filtering to the unestimated ones, the parallel rating fan-out, the difficulty writeback with its `## Estimate` audit note, and reading the core-derived tier back — lives in the workflow, not in this prose.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`. If you modify any rdm source, `cargo build` again before running it.**

## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by a **phase number** to narrow the run to a single phase. If no slug is given, stop and say so.

The workflow rates only phases whose **difficulty is unset** and **skips** any phase that already has a difficulty (idempotent — re-running never re-rates or overwrites a human-set or previously-estimated value). The model **tier derives in rdm-core** from the difficulty (`Difficulty::model_tier`); the workflow sets `--difficulty` only, never `--model`, and reads the resulting tier back from `rdm phase show`.

This skill is **non-interactive**. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so the workflow's dispatched agents and bash commands don't block on permission prompts.

## What to do

1. **Parse `$ARGUMENTS`** into a config object:
   - `roadmap` — the required slug (the first positional argument).
   - `phase` — the positive integer phase number, when a second positional argument is present (omit otherwise, meaning "every unestimated phase").
   - `rdmBin` — `"./target/debug/rdm"`, this repo's development build. The workflow names no rdm executable of its own.
   - `project` — `"rdm"`, this repo's plan project.
2. **Gather the two mechanical values yourself and hand them to the workflow.** You are already a running agent with the repo in context; the workflow is not, so each of these otherwise costs it a whole dedicated subagent. Both are **optional** — the workflow falls back to its own in-workflow fetch for anything you omit or get wrong.
   - `mechanicalModel` — the id printed by `./target/debug/rdm model resolve mechanical`, verbatim.
   - `phaseList` — the parsed array from `./target/debug/rdm phase list --roadmap <slug> --project rdm --format json`, passed through **verbatim**, never summarized. It feeds the unestimated filter, so a summarized list would silently skip or re-rate phases.
3. **Invoke the `rdm-wf-estimate` workflow** via the Workflow tool with `{ roadmap, phase, mechanicalModel, phaseList, rdmBin: "./target/debug/rdm", project: "rdm" }` (omit `phase` when not supplied). Pass `args` as a JSON object, never a stringified value.

   `rdmBin` is optional and defaults to a plain `rdm` on `PATH` when omitted — resolve an explicit `--rdm-bin <path>` first, then `$RDM_BIN` if set (this repo's `.mise.toml` sets it to `./target/debug/rdm`), then the default; an explicitly passed value always wins verbatim. Pass the development build here rather than relying on the default, per the development-build rule. `project` is optional and applies only to project-scoped subcommands; `rdm model resolve` never carries it.
4. **Print the returned summary verbatim.** It lists each phase estimated this run with its assigned difficulty, the core-derived model tier, and the one-line justification, plus the phases skipped because they were already estimated.
