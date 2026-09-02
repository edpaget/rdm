# Golden JSON contract snapshots

This directory freezes rdm's machine-facing `--format json` contract as
committed golden files — one JSON file per command, captured against a
deterministic fixture plan repo and redacted so it stays reproducible across
machines and days. `scripts/verify-golden-json.sh` re-captures and diffs
against these files on every CI run (it matches the `scripts/verify-*.sh`
glob CI already runs), so a change to rdm's JSON shape turns into a red test
at the source instead of a silent contract break for anything consuming this
CLI's `--format json` output (an editor plugin, an MCP server, a script).

## The 20 captured commands

Each command below is captured with `--format json` (or the bare form,
noted) against a hermetic fixture plan repo built by
`scripts/lib/rdm-plan-fixture.sh`, with one submitted `request-changes`
review (authored by the fixed `fixture-bot` identity, matching the
fixture's own `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL` convention, so the
`author` field is reproducible across machines without needing redaction)
and one worktree seeded on top of the standard fixture seed. See
`scripts/lib/golden-capture.sh` for the exact invocation of each.

| Golden file | Command |
| --- | --- |
| `info.json` | `rdm info --format json --project <proj>` |
| `roadmap-list.json` | `rdm roadmap list --format json --project <proj>` |
| `roadmap-show.json` | `rdm roadmap show sample-roadmap --format json --project <proj>` |
| `phase-list.json` | `rdm phase list --roadmap sample-roadmap --format json --project <proj>` |
| `phase-show.json` | `rdm phase show 1 --roadmap sample-roadmap --format json --project <proj>` |
| `task-list.json` | `rdm task list --format json --project <proj>` |
| `task-show.json` | `rdm task show fixture-task-open --format json --project <proj>` |
| `list.json` | `rdm list --format json --project <proj>` |
| `search.json` | `rdm search seed --format json --project <proj>` |
| `next.json` | `rdm next --roadmap sample-roadmap --format json --project <proj>` |
| `tree.json` | `rdm tree --format json --project <proj>` |
| `describe.json` | `rdm describe --format json` (project-independent) |
| `tag-list.json` | `rdm tag list --format json --project <proj>` |
| `backlog-report.json` | `rdm backlog report --format json --project <proj>` |
| `model-show.json` | `rdm model show --format json` (project-independent) |
| `review-list.json` | `rdm review list --format json --project <proj>` |
| `review-show.json` | `rdm review show <id> --format json --project <proj>` |
| `review-requests.json` | `rdm review requests --format json --project <proj>` |
| `worktree-list.json` | `rdm worktree list --format json` (cwd-derived, no `--project`) |
| `worktree-current.json` | `rdm worktree current --format json` (cwd-derived, run from inside the seeded worktree — from the bare code repo root it returns `null`) |

## Commands dropped instead of captured

Three commands accept `--format json` without a parse error but silently
ignore it and always print plain human text. Per this phase's own escape
hatch, each is dropped from the golden set and filed as an ordinary rdm
follow-up task instead of captured as a fake-JSON golden:

| Command | Why it's excluded | Follow-up task |
| --- | --- | --- |
| `rdm status` | `status::run(root, fetch)` in `rdm-cli/src/commands/status.rs` takes no `format` parameter at all | `golden-json-status-format` |
| `rdm hook done-line` | The `DoneLine` arm in `rdm-cli/src/commands/hook.rs` always `println!("{line}")` | `golden-json-hook-done-line-format` |
| `rdm model resolve` | `run_resolve` in `rdm-cli/src/commands/model.rs` always `println!("{}", policy.resolve(...))` | `golden-json-model-resolve-format` |

## Redaction

`golden_redact` (in `scripts/lib/golden-capture.sh`) applies exactly these
five rules to every captured file, so two same-day captures — on the same
machine or different ones — are byte-identical:

1. **Absolute temp paths** — the fixture's temp root, in both its raw
   `mktemp` form and its OS-canonicalized form (macOS resolves
   `/var/folders/...` to `/private/var/folders/...` in some of rdm's own
   printed output — `rdm info`'s `root` field prints the raw form while
   `rdm worktree add/list/current` print the canonicalized form of the
   *same* directory — so both forms are redacted, longest-first) →
   `<TMPDIR>`.
2. **`created`/`completed` NaiveDate fields** (roadmap/phase/task
   frontmatter, `YYYY-MM-DD`) → `<DATE>`.
3. **Commit-SHA-shaped fields** (`commit`, `applied_commit`,
   `created_commit`, `review_sha`) → `<SHA>`.
4. **Review `created`/`submitted` RFC3339 datetimes** — distinct from rule 2's
   NaiveDate fields; these carry a full timestamp (`Utc::now()`) → `<DATETIME>`.
5. **The review `id` field itself** (`YYYY-MM-DD-HHMM-hex`, volatile by both
   day and run) → `<REVIEW-ID>`. Scoped to the review-id shape so the small,
   stable per-comment integer `id` field is never touched.

This extends the volatile-field set `scripts/lib/rdm-plan-fixture.sh`
documents (temp paths, dates, commit SHAs) with two categories this phase
found load-bearing but undocumented there: review IDs and review RFC3339
datetimes. If `scripts/lib/golden-capture.sh`'s header comment and this
README ever appear to disagree, trust the header comment — it is
authoritative.

## Re-blessing after an intentional shape change

Run:

```
scripts/capture-golden.sh
git diff tests/golden/
```

Review the diff to confirm every changed field is intentional and, if
volatile, correctly redacted — then commit the updated goldens.

**An *additive* field change still fails the drift check.** Appending a new
key to an existing JSON object is a shape change like any other:
`scripts/verify-golden-json.sh` diffs byte-for-byte, so a new field must be
re-blessed deliberately with the workflow above — it is never treated as
automatically safe just because nothing existing was removed or renamed.

`scripts/verify-golden-json.sh` is the CI-enforced gate (it matches the
`scripts/verify-*.sh` glob CI already runs on every PR) that fails when a
fresh capture no longer matches the files in this directory.
