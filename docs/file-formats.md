# File Formats

This document describes every file that `rdm` reads and writes in a plan repo. Use it to understand the structure, troubleshoot parsing issues, or hand-edit files when needed.

## Directory Layout

```
my-plans/
├── rdm.toml                          # repo-level configuration
├── INDEX.md                           # auto-generated — do not edit
└── projects/
    └── <project-slug>/
        ├── project.md                 # project metadata
        ├── INDEX.md                   # auto-generated — do not edit
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
---

Optional project-level notes in the markdown body.
```

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `name` | yes | string | Project slug (matches the directory name) |
| `title` | yes | string | Human-readable project title |

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
| `status` | yes | string | `not-started` \| `in-progress` \| `done` \| `blocked` |
| `completed` | no | date | Completion date (YYYY-MM-DD). Set automatically when status becomes `done` |
| `commit` | no | string | Git commit SHA. Recorded by the post-merge hook or `--commit` flag |

### Status transitions

`not-started` &rarr; `in-progress` &rarr; `done`

A phase can also be `blocked` from any non-terminal state. `done` is terminal and cannot be changed.

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
| `status` | yes | string | `open` | `open` \| `in-progress` \| `done` \| `wont-fix` |
| `priority` | yes | string | `medium` | `low` \| `medium` \| `high` \| `critical` |
| `created` | yes | date | *(today)* | Creation date (YYYY-MM-DD). Set automatically |
| `tags` | no | list of strings | | Free-form labels for filtering |
| `completed` | no | date | | Completion date (YYYY-MM-DD). Set automatically when status becomes `done` |
| `commit` | no | string | | Git commit SHA |

### Status transitions

`open` &rarr; `in-progress` &rarr; `done`

A task can also be marked `wont-fix` from any non-terminal state. Both `done` and `wont-fix` are terminal.

### Priority ordering

`low` &lt; `medium` &lt; `high` &lt; `critical`

## `INDEX.md`

Index files are auto-generated by `rdm` — never edit them by hand. There are two levels:

- **Root `INDEX.md`** — lists all projects with roadmap/task counts and overall progress.
- **Per-project `INDEX.md`** — lists that project's roadmaps (with phase counts, progress bars, and dependency info) and tasks (with priority and status).

Index files are regenerated automatically after every mutation (create, update, delete). You can also regenerate manually:

```bash
rdm index
```

### Merge driver

Because `INDEX.md` is generated, it can cause merge conflicts when multiple branches modify plan data. Install the merge driver to auto-regenerate on merge:

```bash
rdm install-merge-driver
```

This adds an entry to `.gitattributes` and `.git/config` so that git runs `rdm index` instead of attempting a three-way merge on `INDEX.md`.

## Dates

All dates use ISO 8601 format: `YYYY-MM-DD` (e.g., `2026-03-14`). Dates are stored without timezone information.

## General Notes

- **Slugs** are used as directory and file names. They should be lowercase, hyphen-separated identifiers (e.g., `fix-barrel-nulls`, `two-way-players`).
- **`task`** is a reserved prefix and cannot be used as a roadmap slug.
- The markdown body in any file is free-form — `rdm` preserves it exactly as written. Use whatever markdown structure works for your team.
