---
name: rdm-document
description: Generate user documentation from a completed rdm roadmap using phase descriptions and commit SHAs
allowed-tools:
  - Bash
  - Workflow
  - Read
  - Write
---

Generate user-facing documentation from a completed rdm roadmap. `$ARGUMENTS` should be `<roadmap-slug> [--out <path>] [--rdm-bin <path>] [--project <name>]`.

This skill is a thin shim over the `rdm-wf-document` Workflow (`.claude/workflows/rdm-wf-document.js`, provisioned automatically by `rdm agent-config claude --skills`), which does the headless work — validating all-done, gathering each phase's body + commit diff in parallel, and synthesizing the draft — and hands back the draft and its output path (default `docs/<slug>.md`), which this shim writes. The workflow produces an **artifact**, not a completion signal: it performs no approval step and mutates no rdm status. The terminal human approval below is this shim's one job, and it is never delegated back into the workflow.
{principles}
## Steps

1. Parse `$ARGUMENTS` into the roadmap slug and an optional `--out <path>`.
2. Run the roadmap read yourself and pass it along. The workflow reads no rdm document — it dispatches only the per-phase gatherers and the synthesizer, both of which read and judge — so this is REQUIRED, not an optimization.
   - `roadmapMeta` — the parsed object from `<rdmBin> roadmap show <slug> <proj-flag> --format json`, shaped as `{ found: true, slug, title, phases: [{ stem, title, status, commit }, …] }` with the phase records copied **verbatim**, never summarized. Without it the workflow refuses to run and hands back the exact command as `roadmapCommand`.
3. Invoke the `rdm-wf-document` Workflow with `{ roadmap: <slug>, out: <path or omitted>, roadmapMeta, rdmBin, project }`, where `rdmBin` is the value following `--rdm-bin`; when not supplied, `$RDM_BIN` if set, else a plain `rdm` on `PATH` (`<rdmBin>`), and `project` is the supplied `--project` value, else the project name used in `{proj_flag}` (`<proj-flag>` is the matching `--project` flag). `rdmBin` and `project` are what the per-phase gatherers and the synthesizer use to run `phase show` themselves. Pass `args` as a JSON object, never a stringified value.
4. Branch on the result:
   - **`result.aborted === true`**: report why and stop — this is a human decision, not a retry.
     - `result.incompletePhases` non-empty: list each incomplete phase and its status; the roadmap isn't ready to document yet.
     - `result.incompletePhases` empty (a fetch or synthesis failure): relay that the roadmap could not be read or drafted, and suggest checking the slug.
   - **success**: create the parent directory of `result.path` with `mkdir -p` in Bash, then write `result.draft` to `result.path` with the Write tool — never run `result.writeScript`: the draft is model output over untrusted phase bodies and diffs, and a shell heredoc lets a crafted line end the document and run commands. Then present `result.draft` (or the file contents) to the user. Summarize what was generated and note any gaps the draft itself calls out (e.g., phases without commit SHAs, internal-only phases folded into "How it works"). When "How it works" cites an implementation location, it should be a pinned `rdm:src/<path>@<sha>[#Lline]` link (built from the phase's `commit` field) rather than a bare commit SHA or `file:line`, because the web UI and editor integrations resolve it to a permalink — flag any bare SHA to the user. **The task is not done until the user has reviewed and approved the documentation** — this is the workflow's only human touch, and it happens here, never inside the workflow.

## Edge cases

- **Roadmap not found**: the workflow reports `aborted: true` with an empty `incompletePhases` — relay the failure and stop.
- **Phases without commit SHAs**: the workflow's per-phase gather step already fell back to phase body/title alone for those phases; the draft may call this out — pass that along to the user.
