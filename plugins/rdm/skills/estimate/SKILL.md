---
name: estimate
description: Rate each phase's difficulty and assign a model tier from its body
allowed-tools:
  - Bash
  - Workflow
---

Rate the difficulty of an rdm roadmap's phases by invoking the **`rdm:rdm-wf-estimate` Workflow** (`rdm:rdm-wf-estimate`, installed by the `rdm` plugin). This skill is a **thin shim**: it parses the invocation, hands off to the workflow, and prints the summary the workflow returns. All the loop logic — filtering to the unestimated phases, the parallel rating fan-out, and building the difficulty writeback — lives in the workflow, not in this prose.

## Contract

**Input** (`$ARGUMENTS`): a **required roadmap slug**, optionally followed by a **phase number** to narrow the run to a single phase, and optionally `--rdm-bin <path>` / `--project <name>`. If no slug is given, stop and say so.

The workflow rates only phases whose **difficulty is unset** and **skips** any phase that already has a difficulty (idempotent — re-running never re-rates or overwrites a human-set or previously-estimated value). The model **tier derives in rdm-core** from the difficulty (`trivial`/`easy` → small, `moderate` → medium, `hard` → large); the returned writeback sets `--difficulty` only, never `--model`. Estimation writes **nothing to the phase body** — there is no audit note, and no body is read, carried or rewritten anywhere in this lane. To force a re-estimate, a human first clears the difficulty (`rdm phase update <phase> --clear-difficulty --no-edit --roadmap <slug> --project <PROJECT>`).

This skill is **non-interactive**. Launch unattended runs with `--permission-mode auto` (or `bypassPermissions` in a sandbox) so the workflow's dispatched agents and bash commands don't block on permission prompts.

## What to do

1. **Parse `$ARGUMENTS`** into a config object:
   - `roadmap` — the required slug (the first positional argument).
   - `phase` — the positive integer phase number, when a second positional argument is present (omit otherwise, meaning "every unestimated phase").
   - `rdmBin` — the value following `--rdm-bin`; when not supplied, `$RDM_BIN` if set, else a plain `rdm` on `PATH`. The workflow names no rdm executable of its own. Call the resolved executable `<rdmBin>`.
   - `project` — the value following `--project` when given, otherwise the name in `--project <PROJECT>`; omit it only when no project name applies, leaving rdm's own `RDM_PROJECT`/`default_project` chain to resolve it. `<proj-flag>` below is ` --project <project>`, or nothing when omitted.
2. **Run the phase list yourself and hand it to the workflow.** The workflow reads nothing — the rater is the only agent it dispatches — so this is REQUIRED, not an optimization.
   - `phaseList` — the parsed array from `<rdmBin> phase list --roadmap <slug><proj-flag> --format json`, passed through **verbatim**, never summarized. It feeds the unestimated filter, so a summarized list would silently skip or re-rate phases. Without it the workflow refuses to run and hands back the exact command as `listCommand`.
3. **Invoke the `rdm:rdm-wf-estimate` workflow** via the Workflow tool with `{ roadmap, phase, phaseList, rdmBin, project }` (omit `phase` when not supplied, and `project` when no project name applies). Pass `args` as a JSON object, never a stringified value.

   `rdmBin` is optional and the workflow defaults it to a plain `rdm` on `PATH` when omitted; an explicitly passed value always wins verbatim. See `docs/workflow-schemas.md` § "Environment args: `rdmBin` and `project`" for the canonical resolution order. `project` is optional and applies only to project-scoped subcommands; `rdm model resolve` never carries it.
4. **Run each returned writeback, then print the summary.** The workflow persists nothing: every rated phase comes back with `writebackCommands` / `writebackScript` — ONE `phase update --difficulty` command, which touches nothing else. Run each one in Bash, in order, **exactly as returned**: there is nothing to substitute, and no phase body is read, carried, or rewritten by you or by it. Report the exit status. Then print the returned summary verbatim — it lists each phase rated this run with its difficulty and one-line justification, plus the phases skipped because they were already estimated.

## Resolving `rdmBin` (plugin install)

This skill was installed from the `rdm` plugin, so there is no repo-local build path to assume. Resolve the `rdmBin` argument in this order and use the first that exists:

1. an explicitly supplied `--rdm-bin <path>`;
2. the `RDM_BIN` environment variable;
3. a plain `rdm` on `PATH`.

If none resolves, stop and report: `rdm binary not found. Install rdm, then set RDM_BIN=/path/to/rdm, put rdm on your PATH, or pass --rdm-bin /path/to/rdm.` Never guess a path, and never invoke a workflow without one.
