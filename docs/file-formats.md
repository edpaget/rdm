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
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_project` | string | *(none)* | Project slug to use when `--project` is not passed |
| `default_format` | string | `"human"` | Output format. Valid values: `human`, `json`, `table`, `markdown` |
| `stage` | bool | `false` | When `true`, mutations write files but skip the git commit until you run `rdm commit` |
| `remote.default` | string | *(none)* | Default git remote name |

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

## Steps

1. Add a `PlayerType::TwoWay` variant
2. Merge pitching and hitting projections in the valuation service

## Acceptance Criteria

- Two-way players appear as a single row in rankings
- Combined WAR accounts for both pitching and hitting value
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `phase` | yes | integer | 1-based phase number |
| `title` | yes | string | Human-readable title |
| `status` | yes | string | `not-started` \| `in-progress` \| `needs-review` \| `reviewed` \| `done` \| `blocked` \| `wont-fix` |
| `completed` | no | date | Completion date (YYYY-MM-DD). Set automatically when status becomes `done` or `wont-fix` |
| `commit` | no | string | Git commit SHA. Recorded by the post-merge hook or `--commit` flag |

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

### Status transitions

`open` &rarr; `in-progress` &rarr; `needs-review` &rarr; `reviewed` &rarr; `done`

A task can also be marked `wont-fix` from any non-terminal state. Both `done` and `wont-fix` are terminal.

### Priority ordering

`low` &lt; `medium` &lt; `high` &lt; `critical`

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
