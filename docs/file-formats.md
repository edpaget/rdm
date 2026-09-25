# File Formats

This document describes every file that `rdm` reads and writes in a plan repo. Use it to understand the structure, troubleshoot parsing issues, or hand-edit files when needed.

## Directory Layout

```
my-plans/
├── rdm.toml                          # repo-level configuration
└── projects/
    └── <project-slug>/
        ├── project.md                 # project metadata
        ├── roadmaps/
        │   └── <roadmap-slug>/
        │       ├── roadmap.md         # roadmap metadata and phase ordering
        │       ├── phase-1-<slug>.md
        │       ├── phase-2-<slug>.md
        │       └── ...
        ├── tasks/
        │   ├── <task-slug>.md
        │   └── ...
        ├── plans/
        │   ├── <plan-slug>.md          # implementation plans
        │   └── ...
        ├── reviews/
        │   ├── <review-id>.md          # document and change reviews
        │   └── ...
        ├── runs/
        │   ├── <run-id>.md             # autonomous-lane run records
        │   └── ...
        └── archive/
            └── roadmaps/
                └── <roadmap-slug>/    # archived roadmaps (same structure)
```

Every markdown file follows the same pattern: YAML frontmatter between `---` delimiters, a blank line, then a free-form markdown body.

```
---
key: value
---

Markdown body starts here.
```

## `rdm.toml`

Repo-level configuration. Lives at the plan repo root. All fields are optional.

```toml
default_project = "fbm"       # used when --project is omitted
default_format = "human"       # human | json | table | markdown
stage = false                  # true = defer git commits until `rdm commit`

[remote]
default = "origin"             # default git remote for push/pull

[gates]
reviewed = true                # enforce the `reviewed` transition gate

[projects.web.dispatch]
verify = "npm test"            # per-project override of dispatch.verify
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_project` | string | *(none)* | Project slug to use when `--project` is not passed |
| `default_format` | string | `"human"` | Output format. Valid values: `human`, `json`, `table`, `markdown` |
| `stage` | bool | `false` | When `true`, mutations write files but skip the git commit until you run `rdm commit` |
| `remote.default` | string | *(none)* | Default git remote name |
| `gates.reviewed` | bool | `false` | When `true`, `phase update --status reviewed` / `task update --status reviewed` refuse unless an approved plan, an approving `change/` review naming it, and a clean worktree all exist. Repo-only. See [`core-enforced-gates.md`](core-enforced-gates.md) |
| `projects.<name>` | table | *(none)* | Per-project overrides of the project-scopable keys (`dispatch.verify`, `gates.reviewed`, `plan_review`, `default_branch`), in the same shape as the top-level fields (e.g. `[projects.web.dispatch] verify = "..."`). Repo-only. Set with `rdm config set <key> <value> --project <name>` |

A project-scopable key resolves for a project in this order: its environment override (`RDM_<KEY>`, e.g. `RDM_DISPATCH_VERIFY`, and for `gates.reviewed` also `RDM_REVIEWED_GATE`), then `[projects.<name>]`, then the plan-repo-wide value, then the global config (only for `plan_review` and `default_branch`, which are not repo-only), then the default. `rdm config get <key> --project <name>` and `rdm config list --project <name>` report the resolved value and its source; `--project` on `config` is always explicit and is never taken from `RDM_PROJECT` or `default_project`.

A global config file at `~/.config/rdm/config.toml` supports the same fields plus `root` (path to the plan repo). Repo-level settings in `rdm.toml` override global settings. The `--project` flag, `RDM_PROJECT` env var, and `default_project` config form a resolution chain (flag wins).

## `project.md`

Located at `projects/<slug>/project.md`. Created by `rdm project create`.

```yaml
---
name: fbm
title: Fantasy Baseball Manager
source:
  repo: https://github.com/acme/fbm
  default_branch: main
---

Optional project-level notes in the markdown body.
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `name` | yes | string | Project slug (matches the directory name) |
| `title` | yes | string | Human-readable project title |
| `source` | no | object | The code repository this project's `rdm:src/` links resolve against — `{ repo: String, default_branch: Option<String> }` |

## `roadmap.md`

Located at `projects/<project>/roadmaps/<roadmap-slug>/roadmap.md`. Created by `rdm roadmap create`.

```yaml
---
project: fbm
roadmap: two-way-players
title: Two-Way Player Identity
phases:
  - phase-1-core-valuation
  - phase-2-keeper-service
  - phase-3-draft-engine
dependencies:
  - keeper-surplus-value
---

This roadmap establishes a unified identity model for two-way players
so that pitching and hitting value are combined into a single ranking.
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `project` | yes | string | Project slug this roadmap belongs to |
| `roadmap` | yes | string | Roadmap slug (matches the directory name) |
| `title` | yes | string | Human-readable title |
| `phases` | yes | list of strings | Ordered list of phase file stems (without `.md`) |
| `dependencies` | no | list of strings | Roadmap slugs that must complete before this one |

The `phases` list controls display order. When you create a phase with `rdm phase create`, the stem is appended automatically.

## Phase Files

Located at `projects/<project>/roadmaps/<roadmap>/phase-<N>-<slug>.md`. Created by `rdm phase create`.

The filename encodes the phase number and slug: `phase-1-core-valuation.md`.

For the complete specification of phase body grammar and the acceptance-criteria rubric, see [`docs/authoring-grammar.md`](authoring-grammar.md).

```yaml
---
phase: 1
title: Core valuation layer
status: done
completed: 2026-03-13
commit: a1b2c3d
---

## Context

The current valuation engine treats pitchers and hitters as separate entities...

## Approach

Add a `PlayerType::TwoWay` variant and merge pitching and hitting projections in the
valuation service. This consolidates dual-position players into a single row while
preserving the accuracy of their combined positional value.

## Acceptance Criteria

- [ ] Two-way players appear as a single row in rankings
- [ ] Combined WAR accounts for both pitching and hitting value
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `phase` | yes | integer | 1-based phase number |
| `title` | yes | string | Human-readable title |
| `status` | yes | string | `not-started` \| `in-progress` \| `needs-review` \| `reviewed` \| `done` \| `blocked` \| `wont-fix` |
| `completed` | no | date | Completion date (YYYY-MM-DD). Set automatically when status becomes `done` or `wont-fix` |
| `commit` | no | string | Git commit SHA. Recorded by the post-merge hook or `--commit` flag |
| `gate_override` | no | object | An operator's recorded bypass of the `reviewed` transition gate. See below |

### The `gate_override` block

Both phase and task files may carry a `gate_override` block, written by
`--override-gate "<reason>"` when an operator bypasses the core `reviewed`
transition gate's record preconditions:

```yaml
gate_override:
  reason: "operator: hotfix, plan filed retroactively"
  actor: alice
  at: 2026-09-13
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `reason` | yes | string | Why the gate was bypassed, verbatim as the operator typed it. Never empty |
| `actor` | yes | string | Who bypassed it, resolved like a review's author (`RDM_REVIEW_AUTHOR`, then `$USER`) |
| `at` | yes | date | The date the bypass was recorded (YYYY-MM-DD) |

The block is **absent entirely** unless an override is in force — a file that
was never overridden serializes exactly as it did before the gate existed — and
is **removed** whenever the item leaves `reviewed`, so a stale override can
never authorize a later `reviewed` write. See
[`core-enforced-gates.md`](core-enforced-gates.md).

### Status transitions

`not-started` &rarr; `in-progress` &rarr; `needs-review` &rarr; `reviewed` &rarr; `done`

A phase can also be `blocked` from any non-terminal state. A phase can also be marked `wont-fix` from any non-terminal state. Both `done` and `wont-fix` are terminal.

## Task Files

Located at `projects/<project>/tasks/<slug>.md`. Created by `rdm task create`.

```yaml
---
project: fbm
title: Fix barrel column NULL for 2024 statcast data
status: open
priority: high
created: 2026-03-14
tags:
  - data
  - statcast
---

The `barrel` column in the 2024 statcast import returns NULL for all rows.
This appears to be a schema change in the upstream CSV — the column was
renamed to `barrel_pct`.
```

| Field | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `project` | yes | string | | Project slug |
| `title` | yes | string | | Human-readable title |
| `status` | yes | string | `open` | `open` \| `in-progress` \| `needs-review` \| `reviewed` \| `done` \| `wont-fix` |
| `priority` | yes | string | `medium` | `low` \| `medium` \| `high` \| `critical` |
| `created` | yes | date | *(today)* | Creation date (YYYY-MM-DD). Set automatically |
| `tags` | no | list of strings | | Free-form labels for filtering |
| `completed` | no | date | | Completion date (YYYY-MM-DD). Set automatically when status becomes `done` or `wont-fix` |
| `commit` | no | string | | Git commit SHA |
| `gate_override` | no | object | | An operator's recorded bypass of the `reviewed` transition gate — same shape and lifecycle as a phase's, documented above |

### Status transitions

`open` &rarr; `in-progress` &rarr; `needs-review` &rarr; `reviewed` &rarr; `done`

A task can also be marked `wont-fix` from any non-terminal state. Both `done` and `wont-fix` are terminal.

### Priority ordering

`low` &lt; `medium` &lt; `high` &lt; `critical`

## Plan Files

Located at `projects/<project>/plans/<slug>.md`. Created by `rdm plan create`.

An implementation plan describes *how* one phase or task will be implemented. It is a
document in its own right — separate from the phase body — so that it can be reviewed
with anchored comments, revised without drifting the anchors of reviews on the phase,
and superseded by a later attempt while the earlier one stays readable.

```yaml
---
project: rdm
plan: impl-auth-v2
title: Auth implementation, second attempt
implements: rdm:phase/auth/phase-1-design
supersedes: rdm:plan/impl-auth-v1
status: changes-requested
created: 2026-03-14
updated: 2026-03-15
---

## Approach

Extract the token verifier behind a trait, then swap the backing store.
```

| Field | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `project` | yes | string | | Project slug |
| `plan` | yes | string | | Plan slug (matches the file stem) |
| `title` | yes | string | | Human-readable title |
| `implements` | yes | item ref | | Exactly one `rdm:phase/<roadmap>/<stem>` or `rdm:task/<slug>` — the item this plan implements |
| `supersedes` | no | item ref | | An `rdm:plan/<slug>` naming the earlier plan this one replaces |
| `status` | yes | string | `draft` | `draft` \| `approved` \| `changes-requested` \| `superseded` |
| `created` | yes | date | *(today)* | Creation date (YYYY-MM-DD). Set automatically |
| `updated` | yes | date | *(today)* | Last-modified date (YYYY-MM-DD). Set automatically |

`implements` and `supersedes` use the same `rdm:` item-reference grammar as `Done:`
lines, `rdm review --on`, and `rdm:` body links. A bare `phase/auth/phase-1-design`
(no `rdm:` prefix) is accepted when parsing a hand-edited file; rdm always writes the
`rdm:` form back. Neither reference is validated at parse time — a plan whose
implemented phase or task was renamed or deleted still loads, so renames never corrupt
the plan store. Existence is checked when a plan is *created*.

Note that `implements` is frontmatter, not a body link, so `rdm link check` does not
validate it — only `rdm:` links inside a plan's markdown body are checked.

### Status transitions

A plan's status is **derived from reviews**, never set with a status flag:

- `draft` &rarr; `approved` — a review on `plan/<slug>` was submitted with verdict `approve`
- `draft` &rarr; `changes-requested` — ...submitted with verdict `request-changes`
- a verdict of `comment` leaves the status unchanged
- any state &rarr; `superseded` — a later plan named this one in its `supersedes` field

`superseded` is terminal: a review submitted against an already-superseded plan never
downgrades it back to `approved` or `changes-requested`.

## Review Files

Located at `projects/<project>/reviews/<id>.md`. Created by `rdm review start`.

Everything but the overall summary lives in the frontmatter, including the full
comment list; the body below it is the summary.

### Review targets

A review's `target` is a tagged mapping keyed on `kind`:

| `kind` | Fields | Reference syntax |
|--------|--------|------------------|
| `roadmap` | `roadmap` | `roadmap/<slug>` |
| `phase` | `roadmap`, `stem` | `phase/<roadmap-slug>/<stem-or-number>` |
| `task` | `slug` | `task/<slug>` |
| `plan` | `slug` | `plan/<slug>` |
| `change` | `head`, `base` | `change/<sha-or-rev>` |

A `change` target reviews **code in the project's source repository**, not a
plan-repo document:

```yaml
---
id: 2026-07-01-1430-a1b2
author: ed
target:
  kind: change
  head: 1f0c9a4e1f0c9a4e1f0c9a4e1f0c9a4e1f0c9a4e   # always a full 40-char SHA
  base: 9b2d1c0a9b2d1c0a9b2d1c0a9b2d1c0a9b2d1c0a   # the merge-base, or --base
change_branch: roadmap/auth
implements: rdm:plan/impl-auth-v2
state: submitted
verdict: request-changes
created: 2026-07-01T14:30:00Z
comments:
  - id: 1
    status: open
    anchor:
      anchor_type: file-quote
      path: rdm-core/src/link.rs
      quote: "fn parse(uri: &str)"
      occurrence: 1
      start_line: 142
      end_line: 142
    body: The error path loses the original URI.
---
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `head` | yes | string | Full 40-character SHA of the reviewed tip. Whatever revision the operator names (`HEAD`, a branch, an abbreviated sha) is rev-parsed before it is stored |
| `base` | no | string | The revision the change is diffed against — `--base`, else the merge-base with the project's default branch. **Provenance, not identity**: the reference grammar is `change/<head>` only |
| `change_branch` | no | string | Source-repo branch the change was on, used to pick the tip anchor drift is measured against. Absent on a detached HEAD |
| `implements` | no | item ref | The `rdm:plan/<slug>` this change implements. Only meaningful on a `change` target; absent on every other kind, so every pre-existing review file still loads |

`rdm:change/<sha>` is **not** a valid link — `change` names a review target, not
a linkable document. Link code with `rdm:src/<path>@<sha>` instead.

### Anchors

A comment's `anchor` is a tagged mapping keyed on `anchor_type`. An
unrecognized type round-trips verbatim, so an older binary never corrupts a
review it does not fully understand.

| `anchor_type` | Fields | Used by |
|---------------|--------|---------|
| `text-quote` | `quote`, `prefix`, `suffix` | Every plan-repo document target |
| `file-quote` | `path`, `quote`, `occurrence`, `start_line`, `end_line` | A `change` target |

`file-quote` deliberately stores **no** surrounding context: the review file
lives in the plan repo, and nothing from the source repository beyond the quote
the reviewer chose is ever embedded in it. Duplicate occurrences are
disambiguated with `occurrence` (1-based) plus the recorded line range, which
is also what lets `rdm review show` emit a head-pinned
`rdm:src/<path>@<head>#L<start>-L<end>` permalink with no source checkout
present.

The full model — hunk-restricted anchoring, resolved/drifted/unresolved
detection, the `implements` inference rules, and the never-write-the-source-repo
invariant — is recorded in [`change-reviews.md`](change-reviews.md).

## Run Files

Located at `projects/<project>/runs/<id>.md`. Created by `rdm run record`, and
updated by `rdm run unit-start`, `rdm run unit-end` and `rdm run close`. A run
records **what ran, where, when**: which lane driver drove which roadmap or
task, in which Claude Code session, and — per dispatched unit — over which time
window with what outcome. Everything lives in the frontmatter; the body is
always empty.

```yaml
---
id: 2026-09-24-1530-a1b2
project: rdm
driver: autopilot
roadmap: autopilot-run-accounting
session_uuid: 0f3c9a1e-1111-4222-8333-444455556666
args: autopilot-run-accounting --max 3
status: closed
started: 2026-09-24T15:30:12.345Z
ended: 2026-09-24T16:40:00.001Z
stop_reason: all phases reviewed
units:
- unit: phase-3-run-artifact
  attempt: 1
  started: 2026-09-24T15:31:00Z
  ended: 2026-09-24T15:58:10Z
  outcome: rework
- unit: phase-3-run-artifact
  attempt: 2
  started: 2026-09-24T15:59:02Z
  ended: 2026-09-24T16:20:44Z
  outcome: reviewed
---
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `id` | yes | string | `YYYY-MM-DD-HHMM-xxxx`, minted from the record time with a collision-probed suffix (the review-id contract); also the file stem |
| `project` | yes | string | Project the run belongs to |
| `driver` | yes | string | `autopilot` or `dispatch-phase` |
| `roadmap` / `task` | exactly one | string | Slug of the roadmap or task the run drives |
| `session_uuid` | no | string | Raw Claude Code session uuid (see below). Absent when none was captured |
| `args` | no | string | Free-form invocation arguments of the driver |
| `status` | yes | string | `open`, `closed` or `abandoned` |
| `started` | yes | datetime | When the run was recorded (RFC 3339, sub-second precision kept) |
| `ended` | no | datetime | When the run was closed or abandoned |
| `stop_reason` | no | string | Why the run stopped, set by `rdm run close --stop-reason` |
| `units` | yes | list | Unit entries in start order: `unit` (phase stem, or the task slug on a task run), `attempt`, `started`, and once ended `ended` and a free-form non-empty `outcome` |

### Status and completeness

`status` starts `open` and moves once, via `rdm run close`, to `closed`
(the default) or `abandoned` (`--status abandoned`, for an interrupted run).
Both are terminal: a closed or abandoned run refuses further unit and close
writes.

- A run still `open` is **incomplete** — in flight, or left behind by a
  session that died before closing it. It still loads, and `rdm run show` /
  `rdm run list` label it incomplete.
- A unit entry with no `ended` is **incomplete**. At most one unit is open at a
  time: `unit-start` is refused while another unit is open, and `unit-end`
  always ends that one. Closing a run with a unit still open is allowed; that
  unit simply stays incomplete.

### Attempt ordinals

On a roadmap run, `--unit` accepts a phase stem or number and is stored as the
resolved stem; on a task run it must be the run's task slug. A unit's `attempt`
is one more than the number of entries this run already has for that unit, so a
rework re-dispatch appends a second entry (`attempt: 2`) rather than changing
the first.

### Session capture

`rdm run record` stores `--session-uuid` when given, else the raw value of
`CLAUDE_CODE_SESSION_ID` when it is set and non-empty, else nothing. It never
stores the hashed changeset session id and consults no other harness variable,
because only the raw Claude Code uuid names a transcript directory. A run with
no `session_uuid` cannot be joined to spend, and `rdm run record` says so on
stderr. Every timestamp is the CLI's own clock at the moment of the call.

### Dangling targets

The `roadmap` / `task` target is checked only when the run is recorded. A run
whose roadmap or task was later deleted or archived still loads, and
`rdm run list --roadmap <slug>` still finds it by the recorded slug.

### Not searchable

Runs are deliberately left out of `rdm search` and its `--type` filter: they
have no prose body, and their fields are ids and timestamps, which fuzzy text
search has nothing useful to match. Find them with
`rdm run list --roadmap <slug>` (or `--task <slug>`) instead.

## `INDEX.md`

rdm no longer generates `INDEX.md` files. The `rdm index` command, and the
per-mutation regeneration that used to run alongside it, have both been
removed — see [`docs/index-removal.md`](index-removal.md) for the decision
and its rationale. For a browsable snapshot of the plan repo, run:

```bash
rdm list --format markdown
```

An `INDEX.md` left over from before this removal (or one you write and track
by hand) is an ordinary user file to rdm: `rdm status`, `rdm commit`, and
`rdm discard` treat it exactly like any other tracked path, and a conflict on
it merges and resolves the same way any other markdown file's conflict does.

### Legacy merge-driver cleanup

Earlier versions of rdm configured a custom `rdm-index` git merge driver, in
two halves: `merge=rdm-index` lines in the worktree's `.gitattributes`, and a
matching `[merge "rdm-index"]` section in the repo-local `.git/config`. Both
are retired (independently of, and before, the `INDEX.md` generation removal
above). What happens to an existing repo that still carries them:

- The tracked `.gitattributes` lines are **left alone**. That file is yours —
  you may have added your own rules to it — and a `merge=rdm-index` attribute
  naming a driver that is not configured is inert: git falls back to its
  built-in three-way merge, with proper conflict markers and nothing on stderr.
  Delete the lines if you like; you do not have to.
- The `[merge "rdm-index"]` section in `.git/config` is **yours to remove**.
  rdm does not touch `.git/config` at all. Remove it once, by hand:

  ```bash
  git config --remove-section merge.rdm-index
  ```

  Do it before your next merge if your `.gitattributes` still maps `INDEX.md`
  to the driver. The driver command no longer exists, and git treats a failing
  merge driver as a conflict resolved to your own side, unmodified and
  unmarked — so a mapped path can lose the incoming rows silently.

## Dates

All dates use ISO 8601 format: `YYYY-MM-DD` (e.g., `2026-03-14`). Dates are stored without timezone information.

## General Notes

- **Slugs** are used as directory and file names. They should be lowercase, hyphen-separated identifiers (e.g., `fix-barrel-nulls`, `two-way-players`).
- **`task`** and **`src`** are reserved prefixes and cannot be used as roadmap slugs.
- The markdown body in any file is free-form — `rdm` preserves it exactly as written. Use whatever markdown structure works for your team.
