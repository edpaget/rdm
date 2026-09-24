---
name: document
description: Generate user documentation from a completed rdm roadmap using phase descriptions and commit SHAs
allowed-tools:
  - Bash
  - Workflow
  - Read
---

Generate user-facing documentation from a completed rdm roadmap. `$ARGUMENTS` should be `<roadmap-slug> [--out <path>]`.

This skill is a thin shim over the `rdm:rdm-wf-document` Workflow (`rdm:rdm-wf-document`, installed by the `rdm` plugin), which does the headless work — validating all-done, gathering each phase's body + commit diff in parallel, and synthesizing the draft — and hands back the shell that writes it to disk (default `docs/<slug>.md`). The workflow produces an **artifact**, not a completion signal: it performs no approval step and mutates no rdm status. The terminal human approval below is this shim's one job, and it is never delegated back into the workflow.

## Steps

1. Parse `$ARGUMENTS` into the roadmap slug and an optional `--out <path>`.
2. Run the roadmap read yourself and pass it along. The workflow reads no rdm document — it dispatches only the per-phase gatherers and the synthesizer, both of which read and judge — so this is REQUIRED, not an optimization.
   - `roadmapMeta` — the parsed object from `rdm roadmap show <slug> --project <PROJECT> --format json`, shaped as `{ found: true, slug, title, phases: [{ stem, title, status, commit }, …] }` with the phase records copied **verbatim**, never summarized. Without it the workflow refuses to run and hands back the exact command as `roadmapCommand`.
3. Invoke the `rdm:rdm-wf-document` Workflow with `{ roadmap: <slug>, out: <path or omitted>, roadmapMeta, rdmBin: "rdm", project }`, where `project` is the project name used in `--project <PROJECT>`. `rdmBin` and `project` are what the per-phase gatherers and the synthesizer use to run `phase show` themselves. Pass `args` as a JSON object, never a stringified value.
4. Branch on the result:
   - **`result.aborted === true`**: report why and stop — this is a human decision, not a retry.
     - `result.incompletePhases` non-empty: list each incomplete phase and its status; the roadmap isn't ready to document yet.
     - `result.incompletePhases` empty (a fetch or synthesis failure): relay that the roadmap could not be read or drafted, and suggest checking the slug.
   - **success**: run `result.writeScript` in Bash — it creates the parent directory and writes the draft through a quoted heredoc — then Read the file at `result.path` and present `result.draft` (or the file contents) to the user. Summarize what was generated and note any gaps the draft itself calls out (e.g., phases without commit SHAs, internal-only phases folded into "How it works"). When "How it works" cites an implementation location, it should be a pinned `rdm:src/<path>@<sha>[#Lline]` link (built from the phase's `commit` field) rather than a bare commit SHA or `file:line`, because the web UI and editor integrations resolve it to a permalink — flag any bare SHA to the user. **The task is not done until the user has reviewed and approved the documentation** — this is the workflow's only human touch, and it happens here, never inside the workflow.

## Edge cases

- **Roadmap not found**: the workflow reports `aborted: true` with an empty `incompletePhases` — relay the failure and stop.
- **Phases without commit SHAs**: the workflow's per-phase gather step already fell back to phase body/title alone for those phases; the draft may call this out — pass that along to the user.

## Resolving `rdmBin` (plugin install)

This skill was installed from the `rdm` plugin, so there is no repo-local build path to assume. Resolve the `rdmBin` argument in this order and use the first that exists:

1. an explicitly supplied `--rdm-bin <path>`;
2. the `RDM_BIN` environment variable;
3. a plain `rdm` on `PATH`.

If none resolves, stop and report: `rdm binary not found. Install rdm, then set RDM_BIN=/path/to/rdm, put rdm on your PATH, or pass --rdm-bin /path/to/rdm.` Never guess a path, and never invoke a workflow without one.
