---
name: rdm-document
description: Generate user documentation from a completed rdm roadmap using phase descriptions and commit SHAs
allowed-tools:
  - Workflow
  - Read
---

Generate user-facing documentation from a completed rdm roadmap. `$ARGUMENTS` should be `<roadmap-slug> [--out <path>]`.

This skill is a thin shim over the `rdm-wf-document` Workflow (`.claude/workflows/rdm-wf-document.js`), which does the headless work — validating all-done, gathering each phase's body + commit diff in parallel, and synthesizing the draft — and writes it to disk itself (default `docs/<slug>.md`). The workflow produces an **artifact**, not a completion signal: it performs no approval step and mutates no rdm status. The terminal human approval below is this shim's one job, and it is never delegated back into the workflow.

## Steps

1. Parse `$ARGUMENTS` into the roadmap slug and an optional `--out <path>`.
2. Gather the two mechanical values yourself and pass them along. You are already a running agent with the repo in context; the workflow is not, so each of these otherwise costs it a whole dedicated subagent. Both are **optional** — the workflow falls back to its own in-workflow fetch for anything you omit or get wrong.
   - `mechanicalModel` — the id printed by `./target/debug/rdm model resolve mechanical`, verbatim.
   - `roadmapMeta` — the parsed object from `./target/debug/rdm roadmap show <slug> --project rdm --format json`, shaped as `{ found: true, roadmap, title, phases: [{ stem, title, status, commit, body }, …] }` with the phase records copied **verbatim**, never summarized. The workflow rejects anything without `found === true` and an array `phases` and fetches its own.
3. Invoke the `rdm-wf-document` Workflow with `{ roadmap: <slug>, out: <path or omitted>, mechanicalModel, roadmapMeta }`.
4. Branch on the result:
   - **`result.aborted === true`**: report why and stop — this is a human decision, not a retry.
     - `result.incompletePhases` non-empty: list each incomplete phase and its status; the roadmap isn't ready to document yet.
     - `result.incompletePhases` empty (a fetch or synthesis failure): relay that the roadmap could not be read or drafted, and suggest checking the slug.
   - **success**: Read the file at `result.path` and present `result.draft` (or the file contents) to the user. Summarize what was generated and note any gaps the draft itself calls out (e.g., phases without commit SHAs, internal-only phases folded into "How it works"). **The task is not done until the user has reviewed and approved the documentation** — this is the workflow's only human touch, and it happens here, never inside `rdm-wf-document.js`.

## Edge cases

- **Roadmap not found**: the workflow reports `aborted: true` with an empty `incompletePhases` — relay the failure and stop.
- **Phases without commit SHAs**: the workflow's per-phase gather step already fell back to phase body/title alone for those phases; the draft (and its `Limitations`/body) may call this out — pass that along to the user.
- **Single-phase roadmaps**: handled identically inside the workflow's git-gather step; nothing extra for this shim to do.
