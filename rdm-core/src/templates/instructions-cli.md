# rdm

rdm is a CLI for managing project roadmaps, phases, and tasks. Use these instructions to interact with plan data exclusively through the rdm CLI.

## Setup

The plan repo location is set via `RDM_ROOT` environment variable or `--root` flag. The project is specified with `{proj_flag}` (or set `RDM_PROJECT` env var, or configure `default_project` in `rdm.toml`).

## Discovering work

```bash
rdm tag list {proj_flag}                   # list every tag in use, with counts
rdm roadmap list {proj_flag}              # list all roadmaps with progress
rdm roadmap list {proj_flag} --tag auth    # list roadmaps carrying tag "auth"
rdm task list {proj_flag}                  # list open/in-progress tasks
rdm task list {proj_flag} --status all     # list all tasks including done
rdm task list {proj_flag} --tag bug        # list open tasks carrying tag "bug"
rdm task list {proj_flag} --tag bug --tag ui  # ANDs across tags — must carry every listed tag
```

`--tag` is repeatable on `roadmap list`, `task list`, the top-level `rdm list`,
and `search` (each extra flag narrows further; matching is exact and
case-sensitive). Passing no `--tag` at all imposes no tag constraint. When any
listed item carries tags, list output gains a trailing `Tags` column — or, for
the default `roadmap list` view, a ` [tags: a, b]` suffix on each tagged line.

## Reading details

```bash
rdm roadmap show <slug> {proj_flag}          # show roadmap with phases and body
rdm phase list --roadmap <slug> {proj_flag}  # list phases with numbers and statuses
rdm phase show <stem-or-number> --roadmap <slug> {proj_flag}  # show phase details
rdm task show <slug> {proj_flag}             # show task details
```

Add `--no-body` to any `show` command to suppress body content when you only need metadata.
Add `--at <sha>` to any `show` command to read the body as it was at a specific git revision; metadata still reflects the current state, and the SHA is surfaced in the output as a `Revision:` line (text/markdown) or a `revision` field (JSON).

`rdm info --format json` reports what rdm actually resolved for the current environment in one call — `{root, project, default_branch, default_format}`, each following the CLI's real precedence chain — useful when discovering the plan repo location without scraping `rdm config list` or reading `rdm.toml` directly.

## Searching

When looking for specific items by keyword, **prefer `rdm search` over listing and manually scanning results**. `rdm search` is fuzzy (typo-tolerant) and matches against titles and bodies. Tags are a hard pre-filter — combine them to narrow results.

```bash
rdm search auth {proj_flag}                  # find items mentioning "auth"
rdm search index --type task {proj_flag}     # find only tasks matching "index"
rdm search search --status in-progress {proj_flag}  # find in-progress items
rdm search auth --format json {proj_flag}    # structured output for chaining
rdm search "" --tag bug {proj_flag}          # list every item carrying tag "bug"
rdm search auth --tag bug --tag ui {proj_flag}  # ANDs across tags — must carry every listed tag
```

Available filters: `--type` (roadmap|phase|task|review), `--status` (e.g., done, in-progress, open), `--tag <name>` (repeatable, AND), `--limit` (default 20), `--format` (text|json).

## Updating status

Always pass `--no-edit` to prevent the CLI from opening an interactive editor, which will hang in a non-interactive agent context.

```bash
rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> {proj_flag}
rdm task update <slug> --status done --no-edit {proj_flag}
```

`--status reviewed` can optionally be gated on real plan and change-review records plus a clean worktree — a repo-only `gates.reviewed` config flag, default off. When enabled, moving an item to `reviewed` requires a passing review; an audited `--override-gate "<reason>"` is available as an escape hatch, intended for a human operator or a documented automated exception, never a routine bypass.

A project can configure a verification command to run against implementation (the repo-only `dispatch.verify` config key). `rdm verify resolve {proj_flag}` prints the configured command, or `unresolved`; `rdm verify run --item <item> --format json {proj_flag}` runs it in that item's worktree.

## Committing changes

Every mutating command (`roadmap`/`phase`/`task`/`review` create, update, delete, and friends) only **stages** its change to disk — it never commits to git on its own. Land your staged changes explicitly:

```bash
rdm status                                   # show what YOU changed (path + change kind)
rdm commit -m "feat(plan): describe the batch"  # land this session's changeset as one commit
rdm discard --force                          # discard this session's changes (irreversible)
```

**These three commands are scoped to your own session's changeset, not to the whole plan repo.** A changeset is the set of paths *this* session wrote. `rdm commit` lands exactly those paths — another session working in the same plan repo at the same time never has its uncommitted work swept into your commit, and `rdm discard --force` normally cannot destroy it either. The one exception is a path you *both* touched: if the content there no longer matches what you last wrote (or a path you deleted has been recreated), `rdm discard` leaves that one path exactly as it is and reports it on a `skipped:` line rather than clobbering it — everything else in your changeset still discards normally. `rdm status` lists your own edits and reports any paths belonging to another changeset on a separate trailing line so they are visible but clearly not yours to land.

Batching still applies, and is now safer than before: prefer batching related mutations (e.g. a roadmap plus all its phases, or a status update plus its follow-on task) into a single `rdm commit` rather than committing after every individual command. `rdm status`, `rdm commit`, and `rdm discard` take no `--project` flag — a changeset can span projects.

Your session identity is resolved automatically and survives across separate `rdm` invocations, so an ordinary batch of commands shares one changeset with no setup. Set `RDM_SESSION=<id>` to pin it explicitly. If you are running under an agent harness that starts a fresh shell for every tool call and publishes no session id of its own, that automatic resolution has nothing to key on and each command becomes its own changeset — `rdm commit` says so and names the fix: export `RDM_HARNESS_SESSION_ID=<stable per-session id>` once, before any rdm command. Inspect and recover with:

```bash
rdm session id                               # this session's changeset id
rdm session list                             # every changeset on disk, orphans flagged
rdm session journal                          # the exact paths this changeset has journaled
rdm commit --changeset <id>                  # land an orphaned changeset (recovery path)
rdm commit --all                             # land the whole working tree, every session's work
rdm status --all                             # view the whole working tree, not just yours
```

If `rdm commit` reports `Nothing in this session's changeset to commit.` while the tree is dirty, the listed paths fall into two distinct groups, printed separately: paths that belong to a different changeset (use `rdm session list` to find its id, then `rdm commit --changeset <id>`), and paths that are not attributed to any changeset at all — a write outside rdm, or dirt from before session-scoped commits — for which the only recovery is `rdm commit --all` (there is no owning changeset id to target). Reads are never scoped — `rdm task show`, `rdm search`, and every other read see the whole working tree, including other sessions' uncommitted items.

## Document reviews

Reviews are structured feedback on a roadmap, phase, task, or implementation-plan document, or on a source-repo change. A review targets `roadmap/<slug>`, `phase/<roadmap-slug>/<stem-or-number>`, `task/<slug>`, `plan/<slug>`, or `change/<head-sha>`, and moves `draft` → `submitted` (with a verdict: `approve`, `request-changes`, or `comment`) → `addressed` or `dismissed`.

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
- A `change/<sha>` review (or `change/HEAD`, run from inside a source checkout, which resolves to the current HEAD commit) reviews code rather than a document: comments anchor into the source repo with `--path <repo-relative path> --quote "exact text"`, restricted to the hunks the change touches.
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

**Always pass `--tags` when you create** a roadmap, phase, or task — untagged items are invisible to tag-filtered queries. Because `--tags` on `update` replaces the existing list rather than adding to it, pass the full set (old tags plus the new one) whenever you retro-fit a tag.

## Tagging convention

- Tag at create time, not later: every `roadmap create` / `phase create` / `task create` should carry at least one `--tags` value, so the item is discoverable from the moment it exists.
- Tag work to make it findable across roadmaps, phases, and tasks (e.g. all auth-related items get `auth`).
- Use lowercase kebab-case (`bug`, `auth`, `tech-debt`).
- Suggested starting vocabulary (suggestions, not a closed set — rdm does not validate tags):
  - `bug` (defect in existing behavior)
  - `enhancement` (new capability or improvement)
  - `cli` (rdm-cli surface)
  - `core` (rdm-core library)
  - `server` (HTTP server surface)
  - `web-ui` (browser UI)
  - `docs` (documentation)

  Combine freely (e.g. `--tags bug,cli`); prefer a project-specific tag that already exists over a new one.
- Before inventing a new tag, run `rdm tag list {proj_flag}` to see every tag already in use with its item count, and reuse a match. Tags are compared verbatim — `CLI` and `cli` are different tags.

## Body content

Use `--body` for inline content. `--body` is **authoritative**: when you pass it, rdm uses the value verbatim and ignores stdin. This includes backticks, em-dashes, curly quotes, and other Unicode/punctuation — none of it triggers stdin reads or hangs.

On **`create`**, multiline content may instead be piped via stdin (do not also pass `--body` — stdin is ignored when `--body` is present):

```bash
rdm task create <slug> --title "Title" --no-edit {proj_flag} <<'EOF'
Multi-line body content goes here.

It supports full Markdown.
EOF
```

On **`update`** (`roadmap`/`phase`/`task`), the body is set **only** via `--body` — stdin is never read, so a tags-only or status-only update can't hang on an open pipe. `update` does **not** accept a piped-stdin body (a heredoc is silently ignored). To pass multiline body content on an update, capture it into a shell variable with a quoted heredoc — which keeps backticks, `$`, and punctuation literal — then pass `--body`:

```bash
body=$(cat <<'EOF'
Multi-line body content goes here.

It supports full Markdown.
EOF
)
rdm task update <slug> --body "$body" --no-edit {proj_flag}
```

To intentionally empty an existing body on `phase update`, `task update`, or `roadmap update`, pass `--clear-body` (mutually exclusive with `--body`). Passing `--body ""` against a non-empty body is rejected to prevent silent clobber from a truncated heredoc or empty command substitution.

## Linking

Bodies can carry `rdm:` links — write them as ordinary Markdown links, e.g. `[the auth roadmap](rdm:roadmap/auth)`. There are four item-link forms, using the same identifiers as `Done:` lines and `review --on` (a review target additionally accepts `change/<head-sha>`, but a change is a source-repo diff, not a plan-repo document, so it is not a linkable item form — link code with `rdm:src/<path>@<sha>` instead):

- `rdm:roadmap/<slug>`
- `rdm:phase/<roadmap-slug>/<stem-or-number>`
- `rdm:task/<slug>`
- `rdm:plan/<slug>`

And one pinned code-link form, for pointing at a specific file (and optionally a revision and line range) in the project's source repository:

- `rdm:src/<path>[@<rev>][#Lstart[-Lend]]` — e.g. `rdm:src/rdm-core/src/link.rs@a1b2c3d#L42-L58`

```bash
rdm link check {proj_flag}                       # validate every rdm: link in the project (CI-friendly, nonzero on anything broken)
rdm link check --on task/<slug> {proj_flag}      # scope the check to one document
rdm link list --on phase/<slug>/<stem> {proj_flag}  # a document's outgoing links, resolved
rdm link resolve rdm:task/fix-login {proj_flag}  # resolve a single URI given on the command line
rdm backlinks task/fix-login {proj_flag}         # documents that reference this item
```

Rules:

- Use exact slugs and stems as printed by rdm — never invent or paraphrase one. Run `rdm search <topic> {proj_flag}` first if you're not sure an item exists yet.
- Pin a code link with the revision from `git rev-parse HEAD` in the source repo, or omit `@rev` inside a phase or task body to let it fall back to that item's own stamped `commit` field once one is recorded.
- Before finalizing a body edit, run `rdm link check --on <ref> {proj_flag}` (e.g. `--on task/<slug>` or `--on phase/<roadmap-slug>/<stem>`) and treat a nonzero exit as a blocking issue — a dangling item link or a code link whose path doesn't exist at its pinned revision — to fix before proceeding.

## Token spend

`rdm cost` reports the tokens a Claude Code session spent, per model and per token class, with a per-source breakdown (main session, `Agent` subagents, Workflow agents). It reads Claude Code's local transcripts under `$CLAUDE_CONFIG_DIR/projects` (default `~/.claude/projects`), is read-only, needs no plan repo or `--project`, and reports tokens only.

```bash
rdm cost --session <uuid>                 # one session
rdm cost --workflow-run <wf_id>           # only that Workflow run's agents
rdm cost                                  # the current session (CLAUDE_CODE_SESSION_ID)
rdm cost --session <uuid> --format json   # the full report; warnings are inside it
rdm cost report --roadmap <slug> {proj_flag}   # a roadmap's runs joined to spend: per phase, overhead, unattributed
```

Unlike bare `rdm cost`, `rdm cost report` reads the plan repo: it joins the roadmap's run records (below) to their sessions by time window and reports totals, tokens by class, attempts and wall clock per phase, run overhead, itemised unattributed spend, and missing or unjoinable runs.

## Run records

A run record (`projects/<project>/runs/<id>.md`) notes what an autonomous-lane run drove, in which Claude Code session, and each dispatched unit's time window and outcome. `rdm run record` prints only the new id, so capture it; it stores the raw `CLAUDE_CODE_SESSION_ID` (or `--session-uuid`) and the CLI's own timestamps.

```bash
id=$(rdm run record --driver autopilot --roadmap <slug> {proj_flag})   # or --driver dispatch-phase, --task <slug>
rdm run unit-start "$id" --unit <stem-or-number> {proj_flag}           # before dispatching a unit (task runs: --unit <task-slug>)
rdm run unit-end "$id" --outcome reviewed {proj_flag}                 # after it returns; pairs with the open unit-start
rdm run close "$id" --stop-reason "all phases reviewed" {proj_flag}   # --status abandoned marks an interrupted run
rdm run list --roadmap <slug> {proj_flag}                             # a roadmap's runs; open ones show as incomplete
rdm run show "$id" --format json {proj_flag}                          # units, attempts, windows, outcomes
```

Only one unit is open at a time; re-dispatching a unit records its next attempt. Runs are not searchable with `rdm search` — use `rdm run list`.

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

**Worktree-vs-main hazard:** when you are working inside a shared worktree for a roadmap or task (not `main`), any file or symbol you cite in a new task's body may exist only on that unlanded branch. Describing it as "existing" without qualification is a false premise once checked against `main`. Before filing a side-task from a worktree, check whether the paths/behavior you're citing are on `main` (e.g. if you introduced or modified them in this same roadmap's or task's phases, they are not yet on `main`). If they aren't, tag the new task with the reserved tag `depends-unlanded` and phrase the body as "`<file/behavior>`, introduced by `<roadmap-or-task-worktree-ref>`, not yet on main":

```bash
rdm task create sweep-x --title "..." --body "path/to/file.rs, introduced by roadmap tagging-support, not yet on main. ..." --tags depends-unlanded --no-edit {proj_flag}
```

Remember `--tags` replaces the whole list on `update` — if you later update a `depends-unlanded` task, read-modify-write the tag list so you don't silently drop the annotation.

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
- `in-progress` → `blocked` — waiting on an external dependency
- `blocked` → `in-progress` — blocker resolved
- `in-progress` → `wont-fix` — decided not to do
- `open` → `wont-fix` — decided not to do before starting
- `done` and `wont-fix` are terminal

{principles}