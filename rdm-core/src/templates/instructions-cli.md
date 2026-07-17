# rdm

rdm is a CLI for managing project roadmaps, phases, and tasks. Use these instructions to interact with plan data exclusively through the rdm CLI.

## Setup

The plan repo location is set via `RDM_ROOT` environment variable or `--root` flag. The project is specified with `{proj_flag}` (or set `RDM_PROJECT` env var, or configure `default_project` in `rdm.toml`).

## Discovering work

```bash
rdm roadmap list {proj_flag}              # list all roadmaps with progress
rdm task list {proj_flag}                  # list open/in-progress tasks
rdm task list {proj_flag} --status all     # list all tasks including done
rdm task list {proj_flag} --tag bug        # list open tasks carrying tag "bug"
```

## Reading details

```bash
rdm roadmap show <slug> {proj_flag}          # show roadmap with phases and body
rdm phase list --roadmap <slug> {proj_flag}  # list phases with numbers and statuses
rdm phase show <stem-or-number> --roadmap <slug> {proj_flag}  # show phase details
rdm task show <slug> {proj_flag}             # show task details
```

Add `--no-body` to any `show` command to suppress body content when you only need metadata.
Add `--at <sha>` to any `show` command to read the body as it was at a specific git revision; metadata still reflects the current state, and the SHA is surfaced in the output as a `Revision:` line (text/markdown) or a `revision` field (JSON).

## Searching

`rdm search` is fuzzy (typo-tolerant) and matches against titles and bodies. Tags are a hard pre-filter — combine them to narrow results.

```bash
rdm search auth {proj_flag}                  # find items mentioning "auth"
rdm search index --type task {proj_flag}     # find only tasks matching "index"
rdm search "" --tag bug {proj_flag}          # list every item carrying tag "bug"
rdm search auth --tag bug --tag ui {proj_flag}  # ANDs across tags — must carry every listed tag
```

## Updating status

Always pass `--no-edit` to prevent the CLI from opening an interactive editor.

```bash
rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> {proj_flag}
rdm task update <slug> --status done --no-edit {proj_flag}
```

## Committing changes

Every mutating command (`roadmap`/`phase`/`task`/`review` create, update, delete, and friends) only **stages** its change to disk — it never commits to git on its own. Land a batch of staged changes explicitly:

```bash
rdm status                                   # show what's staged (path + change kind)
rdm commit -m "feat(plan): describe the batch"  # land every staged change as one commit
rdm discard --force                          # discard staged changes (irreversible)
```

`rdm status`, `rdm commit`, and `rdm discard` operate on the whole plan repo's git state, not a single project, so they take no `--project` flag. Prefer batching related mutations (e.g. a roadmap plus all its phases, or a status update plus its follow-on task) into a single `rdm commit` rather than committing after every individual command.

## Document reviews

Reviews are structured feedback on a roadmap, phase, or task document, with inline comments anchored to quoted text. A review targets `roadmap/<slug>`, `phase/<roadmap-slug>/<stem-or-number>`, or `task/<slug>`, and moves `draft` → `submitted` (with a verdict: `approve`, `request-changes`, or `comment`) → `addressed` or `dismissed`.

```bash
rdm review start --on task/<slug> --no-edit {proj_flag}          # start a draft; prints the review id
rdm review comment <review-id> --quote "exact text" --body "Feedback." --no-edit {proj_flag}
rdm review comment <review-id> --body "Whole-document feedback." --no-edit {proj_flag}
rdm review submit <review-id> --verdict request-changes --no-edit {proj_flag}
rdm review requests {proj_flag}                                  # the work queue: submitted reviews requesting changes
rdm review show <review-id> --format json {proj_flag}            # full anchors + resolution states, one call
rdm review update <review-id> --comment 1 --status addressed --applied-commit <sha> --reply "Fixed." {proj_flag}
rdm review update <review-id> --state addressed {proj_flag}      # close once every comment is resolved
rdm review list --state submitted {proj_flag}                    # filter by --on/--state/--verdict/--author
```

Key mechanics:

- `--quote` must match the document text **exactly**; it is located in the document as it was when the review started (`created_commit`), so quoting stays valid even after the document is edited. If the quote appears more than once, the error lists every occurrence — re-run with `--occurrence <n>` (1-based).
- On a roadmap review, `--doc phase/<stem-or-number>` points a comment at one of the roadmap's phases.
- `rdm review show` reports each comment's anchor as `resolved`, `drifted` (the document changed since the review), or `unresolved`. In JSON, drifted ranges index the `created_commit` version of the body — read it with `--at <created_commit>` — never the current one.
- **Acting on a review (the agent loop)**: `rdm review requests` → for each comment, make the change → `rdm review update <id> --comment <n> --status addressed --applied-commit <sha> --reply "..."` (or `--status wont-fix --reply "why"`) → `rdm review update <id> --state addressed`. The `rdm-revise` skill automates this loop end to end, including drifted-anchor clarification replies.
- Searching review text: `rdm search <query> --type review {proj_flag}` matches summaries and comment bodies.

## Creating items

Always pass `--no-edit` to suppress the interactive editor.

```bash
rdm roadmap create <slug> --title "Title" --body "Summary." --tags bug,ui --no-edit {proj_flag}
rdm phase create <slug> --title "Title" --number <n> --body "Details." --tags audit --no-edit --roadmap <slug> {proj_flag}
rdm task create <slug> --title "Title" --body "Description." --tags bug --no-edit {proj_flag}
```

For `phase create`, pass a bare slug like `hook-commit-bug` — rdm prepends `phase-<number>-` automatically. Do **not** include `phase-N-` in the slug; you'll get a doubled prefix like `phase-1-phase-1-hook-commit-bug`.

`--tags` is comma-separated. Pass `--tags ""` (or omit it) for no tags. On `update`, `--tags` replaces the existing list.

## Tagging convention

- Tag work to make it findable across roadmaps, phases, and tasks (e.g. all auth-related items get `auth`).
- Use lowercase kebab-case (`bug`, `auth`, `tech-debt`).
- Prefer existing tags in the project — run `rdm search "" --tag <candidate> {proj_flag}` to check what's already in use before inventing a new one.

## Body content

Use `--body` for short inline content. `--body` is **authoritative**: when you pass it, rdm uses the value verbatim and ignores stdin. This includes backticks, em-dashes, curly quotes, and other Unicode/punctuation — none of it triggers stdin reads or hangs. For multiline content, pipe via stdin instead (do not also pass `--body`):

```bash
rdm task create <slug> --title "Title" --no-edit {proj_flag} <<'EOF'
Multi-line body content goes here.

It supports full Markdown.
EOF
```

To intentionally empty an existing body on `phase update`, `task update`, or `roadmap update`, pass `--clear-body` (mutually exclusive with `--body`). Passing `--body ""` against a non-empty body is rejected to prevent silent clobber from a truncated heredoc or empty command substitution.

## Planning workflow

### Before starting work

Run `rdm roadmap list {proj_flag}` to see all roadmaps and their progress. Check `rdm task list {proj_flag}` for open tasks. Identify what is in-progress and what comes next before writing any code.

### Implementing a roadmap phase

1. Read the phase: `rdm phase show <stem-or-number> --roadmap <slug> {proj_flag}`
2. Plan your approach and get approval before starting
3. Implement the work described in the phase
4. Include a `Done:` line in the git commit message — the post-merge hook will mark the phase done and record the commit SHA.
   **Use the exact roadmap slug and phase stem from the rdm commands above — do NOT invent or paraphrase them:**
   ```
   Done: <roadmap-slug>/<phase-stem>
   ```
5. Check the next phase: `rdm phase list --roadmap <slug> {proj_flag}`

### Completing a task

1. Implement the work described in the task
2. Include a `Done: task/<slug>` line in the git commit message — the post-merge hook will mark the task done and record the commit SHA.
   **Use the exact task slug from the rdm commands above — do NOT invent or paraphrase it.**

### Discovering bugs or side-work

If you encounter a bug or unrelated improvement while working on a phase, do not fix it inline. Create a task instead:

```bash
rdm task create <slug> --title "Description of the issue" --body "Details." --no-edit {proj_flag}
```

This keeps the current phase focused and ensures nothing is forgotten.

### When a task grows too complex

If a task becomes large enough to warrant multiple phases, promote it to a roadmap:

```bash
rdm promote <task-slug> --roadmap-slug <new-roadmap-slug> {proj_flag}
```

If the task instead belongs inside an already-existing thematic roadmap, fold it in as a new trailing phase instead of creating a new roadmap:

```bash
rdm promote <task-slug> --into <existing-roadmap-slug> {proj_flag}
```

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
- `in-progress` → `wont-fix` — decided not to do
- `open` → `wont-fix` — decided not to do before starting
- `done` and `wont-fix` are terminal

{principles}