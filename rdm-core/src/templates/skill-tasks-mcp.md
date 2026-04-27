---
name: rdm-tasks
description: Work on rdm tasks
allowed-tools:
  - Read
  - Glob
  - Grep
  - Write
  - Edit
  - EnterPlanMode
  - ExitPlanMode
  - {t_task_list}
  - {t_task_show}
  - {t_task_update}
---

Work on rdm tasks. `$ARGUMENTS` is an optional task slug.
{principles}
## Steps

1. **List tasks**: use `rdm_task_list` with `project: {proj_param}` to see open and in-progress tasks. Add `tag: "<name>"` to narrow by tag (e.g. `tag: "bug"`).
2. **Show details**: if a task slug was provided in `$ARGUMENTS`, use `rdm_task_show` with `project: {proj_param}, task: "<slug>"`. Otherwise, present the task list and ask the user which task to work on.
3. **Mark in-progress**: use `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "in-progress"`
4. **Enter plan mode**: use the `EnterPlanMode` tool to switch into planning mode.
5. **Create an implementation plan** using the planning tool. The plan should:
   - Break the task into concrete implementation steps based on the task description
   - Include a final step: "Review changes with user and commit"
6. **Wait for user approval**: the user will review the plan and either accept or request changes. Do not proceed until the plan is accepted.
7. **Exit plan mode**: use the `ExitPlanMode` tool to switch back to execution mode.
8. **Execute the plan**: implement each step, following the plan.
9. **Review with user**: present a summary of the changes and ask the user to confirm they are ready to finalize.
10. **Finalize**: on user acceptance, commit the implementation changes with a `Done: task/<slug>` line in the commit message — the post-merge hook will mark the task done and record the commit SHA.
    **Use the exact task slug from the rdm tools you used earlier — do NOT invent or paraphrase it.**
    If the task is also part of a roadmap phase, include a `Done: <roadmap-slug>/<phase-stem>` line as well (using exact slugs/stems from rdm).

When creating a side-work task during step 5 or 9, attach tags so the task is findable later:

Use `rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Description", body: "Details.", tags: ["<tag1>", "<tag2>"]`.

Use lowercase kebab-case tags and prefer ones already present in the project (check with `rdm_search` `query: "", tags: ["<candidate>"], project: {proj_param}`).
