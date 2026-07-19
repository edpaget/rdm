# rdm

rdm is a tool for managing project roadmaps, phases, and tasks. Use the rdm MCP tools described below to interact with plan data. All tool calls return structured text results.

## Setup

The rdm MCP server is connected and provides tools for plan repo operations. Most tools require a `project` parameter — use {proj_param} for the current project.

## Discovering work

- `rdm_roadmap_list` with `project: {proj_param}` — list all roadmaps with progress
- `rdm_roadmap_list` with `project: {proj_param}, tag: "bug"` — list roadmaps carrying tag "bug"
- `rdm_task_list` with `project: {proj_param}` — list open/in-progress tasks
- `rdm_task_list` with `project: {proj_param}, status: "all"` — list all tasks including done
- `rdm_task_list` with `project: {proj_param}, tag: "bug"` — list open tasks carrying tag "bug"

`tag` is a single exact, case-sensitive tag on these list tools. When any listed
item carries tags, `rdm_task_list` output gains a trailing `Tags` column and
`rdm_roadmap_list` output gains a ` [tags: a, b]` suffix on each tagged line.

## Reading details

- `rdm_roadmap_show` with `project: {proj_param}, roadmap: "<slug>"` — show roadmap with phases and body
- `rdm_phase_list` with `project: {proj_param}, roadmap: "<slug>"` — list phases with numbers and statuses
- `rdm_phase_list` with `project: {proj_param}, roadmap: "<slug>", tag: "audit"` — list phases carrying tag "audit"
- `rdm_phase_show` with `project: {proj_param}, roadmap: "<slug>", phase: "<stem-or-number>"` — show phase details
- `rdm_task_show` with `project: {proj_param}, task: "<slug>"` — show task details

## Searching

Use `rdm_search` for fuzzy matching against titles and body content. Tags are a hard pre-filter — combine them to narrow results.

- `rdm_search` with `query: "auth", project: {proj_param}` — find items mentioning "auth"
- `rdm_search` with `query: "index", kind: "task", project: {proj_param}` — find only tasks
- `rdm_search` with `query: "auth", status: "in-progress", project: {proj_param}` — filter by status
- `rdm_search` with `query: "", tags: ["bug"], project: {proj_param}` — list every item carrying tag "bug"
- `rdm_search` with `query: "auth", tags: ["bug", "ui"], project: {proj_param}` — ANDs across tags

## Updating status

- `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<stem-or-number>", status: "done"`
- `rdm_task_update` with `project: {proj_param}, task: "<slug>", status: "done"`

## Creating items

- `rdm_roadmap_create` with `project: {proj_param}, slug: "<slug>", title: "Title", body: "Summary.", tags: ["bug", "ui"]`
- `rdm_phase_create` with `project: {proj_param}, roadmap: "<slug>", slug: "<slug>", title: "Title", number: <n>, body: "Details.", tags: ["audit"]`
  - Pass a bare slug like `hook-commit-bug` for `slug:` — rdm builds the final stem as `phase-<number>-<slug>`. Do **not** include `phase-N-` in `slug:` or you'll get a doubled prefix like `phase-1-phase-1-hook-commit-bug`.
- `rdm_task_create` with `project: {proj_param}, slug: "<slug>", title: "Title", body: "Description.", tags: ["bug"]`

The `body` parameter accepts full Markdown including multiline content. The `tags` parameter is optional. On `*_update`, passing `tags: [...]` replaces the existing list, and `clear_tags: true` removes all tags (roadmap and phase only; task uses `tags: []`).

`*_update`, `*_create`, and other mutation tools only **stage** their change to disk — none of them commit to git on their own (see "Committing changes" below). Capture the `Commit: <sha>` line from `rdm_commit`'s response, not from the mutation tool, when the edit resolves a review comment (see "Document reviews" below).

## Committing changes

- `rdm_status` — list staged-but-uncommitted changes, each as `{path, change}` (`added` / `modified` / `deleted`). No `project` parameter — it reports the whole plan repo's git state, not one project.
- `rdm_commit` with `message: "..."` — land every currently staged change as one git commit. Omit `message` to auto-generate one from the changed files. Returns a `Commit: <sha>` line.
- `rdm_discard` with `confirm: true` — discard every staged-but-uncommitted change, reverting the working tree to its last commit. Irreversible; omitting or falsifying `confirm` is rejected.

## Document reviews

Reviews are structured feedback on a roadmap, phase, or task document, with inline comments anchored to quoted text. A review moves `draft` → `submitted` (with a verdict: `approve`, `request-changes`, or `comment`) → `addressed` or `dismissed`. Acting on the change-request queue is the agent loop (automated by the `rdm-revise` skill):

- `rdm_review_requests` with `project: {proj_param}` — the work queue: submitted reviews with verdict `request-changes`, each with its target, summary, and `open_comment_count`. Optional `target_kind`/`target_id` narrow to one plan item.
- `rdm_review_show` with `project: {proj_param}, review_id: "<id>"` — the full review in one call: summary, every comment with its anchor (a tagged union on `anchor_type`) and resolution (`resolved`, `drifted`, or `unresolved`), plus `documents[]` carrying each referenced document's `body_at_created_commit` (what the reviewer saw) and `current_body`. `resolved`/`drifted` ranges with `body: "original"` index `body_at_created_commit`, never the current body. Comments with no anchor or an unrecognized `anchor_type` are whole-document feedback — read `current_body` in full.
- Apply each requested edit via `rdm_phase_update` / `rdm_task_update` / `rdm_roadmap_update`, land it with `rdm_commit`, and capture the `Commit: <sha>` line from **that** response.
- `rdm_review_address_comment` with `project: {proj_param}, review_id: "<id>", comment_id: <n>, status: "addressed", applied_commit: "<sha>", reply: "What changed."` — flips the comment and records provenance. Use `status: "wont-fix"` with reasoning to decline (never records an `applied_commit` default); omit `status` to only record a clarification reply and leave the comment open.
- `rdm_review_complete` with `project: {proj_param}, review_id: "<id>"` — closes the review as `addressed`; refuses while any comment is open, listing the offending ids.

## Tagging convention

- Tag work to make it findable across roadmaps, phases, and tasks (e.g. all auth-related items get `auth`).
- Use lowercase kebab-case (`bug`, `auth`, `tech-debt`).
- Prefer existing tags in the project — call `rdm_search` with `query: "", tags: ["<candidate>"], project: {proj_param}` to check what's already in use before inventing a new one.

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

Transitions are not enforced — any status can move to any other. The flow
below is the intended review lifecycle, offered as guidance.

### Phase statuses

- `not-started` → `in-progress` — work begins
- `in-progress` → `needs-review` — implementation finalized, awaiting review
- `needs-review` → `reviewed` — review passed, awaiting merge to main
- `needs-review` → `in-progress` — review found changes to make
- `reviewed` → `done` — merged to main (the `Done:` merge hook flips this)
- `in-progress` → `done` — work is complete
- `in-progress` → `blocked` — waiting on an external dependency
- `blocked` → `in-progress` — blocker resolved
- `in-progress` → `wont-fix` — decided not to do
- `not-started` → `wont-fix` — decided not to do before starting
- `done` and `wont-fix` are terminal

### Task statuses

- `open` → `in-progress` — work begins
- `in-progress` → `needs-review` — implementation finalized, awaiting review
- `needs-review` → `reviewed` — review passed, awaiting merge to main
- `needs-review` → `in-progress` — review found changes to make
- `reviewed` → `done` — merged to main (the `Done:` merge hook flips this)
- `in-progress` → `done` — work is complete
- `in-progress` → `blocked` — waiting on an external dependency
- `blocked` → `in-progress` — blocker resolved
- `in-progress` → `wont-fix` — decided not to do
- `open` → `wont-fix` — decided not to do before starting
- `done` and `wont-fix` are terminal

{principles}