# rdm

rdm is a tool for managing project roadmaps, phases, and tasks. Use the rdm MCP tools described below to interact with plan data. All tool calls return structured text results.

## Setup

The rdm MCP server is connected and provides tools for plan repo operations. Most tools require a `project` parameter — use {proj_param} for the current project.

## Discovering work

- `rdm_roadmap_list` with `project: {proj_param}` — list all roadmaps with progress
- `rdm_task_list` with `project: {proj_param}` — list open/in-progress tasks
- `rdm_task_list` with `project: {proj_param}, status: "all"` — list all tasks including done

## Reading details

- `rdm_roadmap_show` with `project: {proj_param}, roadmap: "<slug>"` — show roadmap with phases and body
- `rdm_phase_list` with `project: {proj_param}, roadmap: "<slug>"` — list phases with numbers and statuses
- `rdm_phase_show` with `project: {proj_param}, roadmap: "<slug>", phase: "<stem-or-number>"` — show phase details
- `rdm_task_show` with `project: {proj_param}, task: "<slug>"` — show task details

## Searching

Use `rdm_search` for fuzzy matching against titles and body content:

- `rdm_search` with `query: "auth", project: {proj_param}` — find items mentioning "auth"
- `rdm_search` with `query: "index", kind: "task", project: {proj_param}` — find only tasks
- `rdm_search` with `query: "auth", status: "in-progress", project: {proj_param}` — filter by status

## Updating status

- `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<stem-or-number>", status: "done"`
- `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "done"`

## Creating items

- `rdm_roadmap_create` with `project: {proj_param}, slug: "<slug>", title: "Title", body: "Summary."`
- `rdm_phase_create` with `project: {proj_param}, roadmap: "<slug>", slug: "<slug>", title: "Title", number: <n>, body: "Details."`
- `rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Title", body: "Description."`

The `body` parameter accepts full Markdown including multiline content.

## Planning workflow

### Before starting work

Use `rdm_roadmap_list` with `project: {proj_param}` to see all roadmaps and their progress. Check `rdm_task_list` with `project: {proj_param}` for open tasks. Identify what is in-progress and what comes next before writing any code.

### Implementing a roadmap phase

1. Read the phase: `rdm_phase_show` with `project: {proj_param}, roadmap: "<slug>", phase: "<stem-or-number>"`
2. Plan your approach and get approval before starting
3. Implement the work described in the phase
4. Include a `Done:` line in the git commit message — the post-merge hook will mark the phase done and record the commit SHA.
   **Use the exact roadmap slug and phase stem from the rdm tools above — do NOT invent or paraphrase them:**
   ```
   Done: <roadmap-slug>/<phase-stem>
   ```
5. Check the next phase: `rdm_phase_list` with `project: {proj_param}, roadmap: "<slug>"`

### Completing a task

1. Implement the work described in the task
2. Include a `Done: task/<slug>` line in the git commit message — the post-merge hook will mark the task done and record the commit SHA.
   **Use the exact task slug from the rdm tools above — do NOT invent or paraphrase it.**

### Discovering bugs or side-work

If you encounter a bug or unrelated improvement while working on a phase, do not fix it inline. Create a task instead:

`rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Description of the issue", body: "Details."`

This keeps the current phase focused and ensures nothing is forgotten.

### When a task grows too complex

If a task becomes large enough to warrant multiple phases, promote it to a roadmap:

`rdm_task_promote` with `project: {proj_param}, task: "<task-slug>", roadmap_slug: "<new-roadmap-slug>"`

## Status transitions

### Phase statuses

- `not-started` → `in-progress` — work begins
- `in-progress` → `done` — work is complete
- `in-progress` → `blocked` — waiting on an external dependency
- `blocked` → `in-progress` — blocker resolved
- `done` is terminal (can be manually reverted if needed)

### Task statuses

- `open` → `in-progress` — work begins
- `in-progress` → `done` — work is complete
- `in-progress` → `wont-fix` — decided not to do
- `open` → `wont-fix` — decided not to do before starting
- `done` and `wont-fix` are terminal

{principles}