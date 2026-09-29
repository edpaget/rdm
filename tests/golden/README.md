# Golden JSON contract snapshots

This directory freezes rdm's machine-facing `--format json` contract as
committed golden files — one JSON file per command, captured against a
deterministic fixture plan repo and redacted so it stays reproducible across
machines and days. The `golden_json` nextest binary
(`rdm-cli/tests/golden_json/`) re-captures and compares against these files
byte for byte on every `cargo nextest run`, so a change to rdm's JSON shape turns into a red test
at the source instead of a silent contract break for anything consuming this
CLI's `--format json` output (an editor plugin, a script, a REST client).

## The 31 captured commands

Each command below is captured with `--format json` (or the bare form,
noted) against a hermetic fixture plan repo built by
`rdm-cli/tests/common/seeded_plan.rs` (in a sandboxed temp `HOME`/XDG), with one submitted `request-changes`
review (authored by the fixed `fixture-bot` identity, matching the
fixture's own `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL` convention, so the
`author` field is reproducible across machines without needing redaction),
one worktree, and one implementation plan seeded on top of the standard
fixture seed. The review, worktree and plan are seeded by the golden capture
itself rather than by the shared seeded fixture, so only the golden lane
changes and `cli_loops::plugin_loop` (which drives the same fixture) is
unaffected. See `rdm-cli/tests/golden_json/capture.rs` for the exact
invocation of each.

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
| `hook-done-line.json` | `rdm hook done-line --roadmap sample-roadmap --phase phase-1-seed-one --format json` (no `--project`; JSON is emitted only on an explicit `--format json` — `RDM_FORMAT` and `default_format` never change the bare trailer it prints otherwise) |
| `plan-create.json` | `rdm plan create fixture-plan --implements task/fixture-task-open --format json --project <proj>` (the seeding call itself; `plan create --format json` prints the created plan) |
| `roadmap-create.json` | `rdm roadmap create fixture-created-roadmap --title "Fixture Created Roadmap" --body … --no-edit --format json --project <proj>` (captured after every read command, so no read golden sees it; prints the created roadmap in the `roadmap show` shape, with `phases: []`) |
| `phase-create.json` | `rdm phase create created-phase --number 1 --roadmap fixture-created-roadmap --title "Created Phase" --body … --no-edit --format json --project <proj>` (prints the created phase in the `phase show` shape, with its computed `stem` and `phase` number; prev/next, plans and `estimate_snapshot` are omitted) |
| `task-create.json` | `rdm task create fixture-created-task --title "Fixture Created Task" --body … --no-edit --format json --project <proj>` (prints the created task in the `task show` shape; plans omitted) |
| `plan-show.json` | `rdm plan show fixture-plan --format json --project <proj>` |
| `plan-list.json` | `rdm plan list --format json --project <proj>` |
| `verify-resolve.json` | `rdm verify resolve --format json --project <proj>` |
| `status.json` | `rdm status --format json` (repo-wide, no `--project`; captured last, against a landed tree: an uncaptured `rdm commit` first lands everything seeded above, then one `task update fixture-task-open --status in-progress` is staged, so the report names only that path; `merge` and `sync` are `null`) |
| `commit.json` | `rdm commit -m "golden: capture commit" --format json` (repo-wide; lands that staged update under the fixture's fixed `RDM_SESSION`, so `changeset` is stable; `commit` is redacted to `<SHA>`) |
| `discard.json` | `rdm discard --force --format json` (repo-wide; discards one staged `task update fixture-task-active --status done`, listing it in `restored`) |

## Commands dropped instead of captured

One command accepts `--format json` without a parse error but silently
ignores it and always prints plain human text. Per this phase's own escape
hatch, it is dropped from the golden set and filed as an ordinary rdm
follow-up task instead of captured as a fake-JSON golden:

| Command | Why it's excluded | Follow-up task |
| --- | --- | --- |
| `rdm model resolve` | `run_resolve` in `rdm-cli/src/commands/model.rs` always `println!("{}", policy.resolve(...))` | `golden-json-model-resolve-format` |

## Redaction

`Redactor` (in `rdm-cli/tests/golden_json/redact.rs`) applies exactly these
seven rules, as regular expressions over the raw text so every other byte is preserved, to every captured file, so two captures — on the same machine or
different ones, on the same day or months apart — are byte-identical:

1. **Absolute temp paths** — the fixture's temp root, in both its raw
   `mktemp` form and its OS-canonicalized form (macOS resolves
   `/var/folders/...` to `/private/var/folders/...` in some of rdm's own
   printed output — `rdm info`'s `root` field prints the raw form while
   `rdm worktree add/list/current` print the canonicalized form of the
   *same* directory — so both forms are redacted, longest-first) →
   `<TMPDIR>`.
2. **`created`/`completed`/`updated` NaiveDate fields** (roadmap/phase/task/
   plan frontmatter, `YYYY-MM-DD`) → `<DATE>`.
3. **Commit-SHA-shaped fields** (`commit`, `applied_commit`,
   `created_commit`, `review_sha`) → `<SHA>`.
4. **Review `created`/`submitted` RFC3339 datetimes** — distinct from rule 2's
   NaiveDate fields; these carry a full timestamp (`Utc::now()`) → `<DATETIME>`.
5. **The review `id` field itself** (`YYYY-MM-DD-HHMM-hex`, volatile by both
   day and run) → `<REVIEW-ID>`. Scoped to the review-id shape so the small,
   stable per-comment integer `id` field is never touched.
6. **The `estimate_snapshot` opaque token** (64 lowercase hex) →
   `<SNAPSHOT>`. It is `content_digest(doc.render())` — a sha256 over the
   phase's *entire* frontmatter and body, including the `created:` date that
   rule 2 redacts at the surface. Redacting the date but not the digest taken
   over it made the golden reproducible within a single day and guaranteed to
   drift the next, which is how it actually broke: `phase-show.json`'s
   `estimate_snapshot` changed with no shape change and no behavior change.
   The rule is scoped to the field name, and
   `golden_json::digest_fields_are_redacted_not_frozen` still asserts the field is
   present and was 64-hex before redaction — only the day-volatile *value* is
   dropped, not the contract that the field exists.
7. **`rdm info`'s running-binary identity** — `version` (the crate semver,
   which moves with every release) → `<VERSION>`, and `binary` (the path of
   the executable under test, which depends on the build directory) →
   `<BINARY>`. Only the values are dropped; the keys stay in the golden.

These seven rules cover the volatile fields of the seeded fixture (temp paths,
dates, commit SHAs) plus four categories found load-bearing: review IDs,
review RFC3339 datetimes, digests taken *over* already-redacted volatile
content, and the running binary's version and path. If `rdm-cli/tests/golden_json/redact.rs`'s module comment and this
README ever appear to disagree, trust the code — it is authoritative.

## Re-blessing after an intentional shape change

Run:

```
cargo nextest run -p rdm-cli --test golden_json --run-ignored only -E 'test(=bless)'
git diff tests/golden/
```

`bless` is an ignored test: the normal run never writes into the checkout.
Review the diff to confirm every changed field is intentional and, if
volatile, correctly redacted — then commit the updated goldens.

**An *additive* field change still fails the drift check.** Appending a new
key to an existing JSON object is a shape change like any other:
`golden_json::capture_matches_committed_goldens` compares byte for byte, so a new field must be
re-blessed deliberately with the workflow above — it is never treated as
automatically safe just because nothing existing was removed or renamed.

`golden_json::capture_matches_committed_goldens` is the CI-enforced gate (it
runs under `cargo nextest run` on every PR) that fails when a fresh capture no
longer matches the files in this directory, naming every drifted or missing
file and the bless command.
