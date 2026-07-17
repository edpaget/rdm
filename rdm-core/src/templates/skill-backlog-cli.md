---
name: rdm-backlog
description: Run a batched backlog grooming pass over rdm's stale/duplicate/tag-cluster/archivable signals and emit a reviewable, propose-only plan of exact rdm commands — no mutations
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
---

Run a batched **grooming pass** over the backlog and emit a reviewable plan. This skill **reads** `rdm backlog report` and **proposes** a set of consolidate / merge / retire / archive actions — each written as the exact `rdm` command a human would run — plus the open questions it could not answer confidently. It **never mutates the plan repo**: no create, update, merge, archive, promote, staging, or `rdm commit`. Deciding what to actually change stays a human call; this skill does the analysis and hands over a ready-to-run batch.

This mirrors how `rdm-autopilot` batches blockers rather than guessing: surface everything at once, then stop.
{principles}
## Contract

`$ARGUMENTS` is **all optional**:

- `[--project <name>]` — plan repo to groom. If omitted, the standard resolution chain applies (`--project` flag > `RDM_PROJECT` env var > `default_project` in `rdm.toml`).
- `[--older-than <days>]` — passed straight through to `rdm backlog report` to tune the staleness threshold.
- `[--tag <tag>]` — passed straight through to `rdm backlog report` to scope the scan.

There is **no positional argument** naming an item to change, because this skill changes nothing. Thread whatever `--project`/`--older-than`/`--tag` you were given through to every read command below; do not hardcode a project.

## Non-mutation guarantee

This skill runs exactly **one** `rdm` command that touches plan data — `rdm backlog report` (read-only) — plus optional read-only `rdm roadmap list` / `rdm search` lookups during analysis. It **never runs** `create`, `update`, `merge`, `archive`, `promote`, `rdm commit`, or `rdm discard`. Every action it proposes is text for a human to run later, never executed here. The `allowed-tools` above deliberately omit `Write`/`Edit`: the emitted grooming plan is this skill's final chat message, not a file it writes.

## Steps

1. Build first if you are in the rdm source repo, then confirm the binary works.
2. Parse `$ARGUMENTS` into optional `--project`/`--older-than`/`--tag` values.
3. Fetch the candidates (read-only):
   ```bash
   rdm backlog report --format json [--older-than <days>] [--tag <tag>] {proj_flag}
   ```
   The JSON has four arrays: `stale_tasks` (`slug`, `title`, `status`, `created`, `age_days`), `duplicate_clusters` (`members`: `slug`/`title`), `tag_clusters` (`tag`, `tasks`: `slug`/`title`), and `archivable_roadmaps` (`roadmap`, `title`, `phase_count`).
4. **Empty case:** if all four arrays are empty, say plainly **"Nothing to groom — the backlog report returned no signals"** and **stop**. Do not fabricate a plan or emit an empty `## Grooming plan` / `## Open questions` skeleton.
5. Otherwise, run the grooming analysis below and emit the plan. If only *some* arrays are empty, **omit that category's subsection entirely** rather than printing an empty header.

## Grooming analysis

Turn each populated category into proposed actions. Emit the whole thing as **one reviewable batch** — a flat, ordered list of `{command, rationale}` pairs grouped by category under clear subheadings, so a human can copy/paste-run them top to bottom (or not). Every proposed mutating command must carry `--no-edit` where the verb supports it, because the human is expected to run them unattended later.

**Autopilot-oriented framing (read first):** whenever you propose creating or extending a thematic roadmap, the phase body you propose (the `--body`/stdin content for `promote`) must be structured with `## Context` / `## Steps` / `## Acceptance Criteria` headings — the same shape every existing phase body uses. That way, if a human executes the batch, a later `rdm-estimate` (which needs a body to rate difficulty from) and `rdm-autopilot` (which needs actionable phases) can pick the roadmap up immediately. The goal of every consolidation you propose is a roadmap that is ready for `/rdm-autopilot`.

- **`stale_tasks`** — for each task, decide:
  - *Retire* if it reads as superseded or no longer relevant:
    `rdm task update <slug> --status wont-fix --reason "<why it is stale / superseded>" --no-edit {proj_flag}`
  - *Consolidate* if it is still valid work that fits a theme, into an existing roadmap (`rdm promote <slug> --into <roadmap> --no-edit {proj_flag}`) or a new one (`rdm promote <slug> --roadmap-slug <new-slug> {proj_flag}`). Note: `--no-edit`/`--body` apply only to `--into`; omit them with `--roadmap-slug`.
  - Otherwise → **open question** (below), not a blind guess.
- **`duplicate_clusters`** — for each cluster, pick a survivor (state the rule you used: most complete body, or earliest `created`) and fold the rest in:
  `rdm task merge <survivor> --from <other1> --from <other2> --no-edit {proj_flag}`
  If no survivor is clearly best, → **open question**, do not guess.
- **`tag_clusters`** — a cluster of related tasks under one tag is a consolidation candidate. First check whether a thematic roadmap already covers it (`rdm roadmap list {proj_flag}` / `rdm search <tag> --type roadmap {proj_flag}`). If one exists, propose `rdm promote <slug> --into <existing-roadmap> --no-edit {proj_flag}` per task. If none exists, propose one `rdm promote <first-slug> --roadmap-slug <new-thematic-slug> {proj_flag}` (no `--no-edit`/`--body` — those apply only to `--into`) and then `rdm promote <slug> --into <that-new-slug> --no-edit {proj_flag}` for the rest, in that create-then-fold order. Never propose both `--into` and `--roadmap-slug` for the same task — they are mutually exclusive.
- **`archivable_roadmaps`** — each is already all-terminal, so propose:
  `rdm roadmap archive <roadmap> {proj_flag}`
  Rationale: "all phases terminal, not yet archived." **Never** add `--force` — these candidates never need it, and `--force` exists only to override *incomplete* roadmaps, which is not this skill's job.

## Open questions

Always end the plan with an `## Open questions` section (include it even when short) listing every candidate the analysis could not confidently resolve — an ambiguous retire-vs-keep, a duplicate cluster with two equally plausible survivors, a tag cluster too thin or mixed-theme to name a roadmap for. State the item(s) and *why* it is ambiguous, and attach **no command**. The rule: **never propose a merge / retire / archive / consolidate action you are not confident about — file it as an open question instead.** Hold destructive-if-wrong actions (retire, merge, archive) to a stricter confidence bar than purely additive ones (proposing a new roadmap), because undoing a wrong `wont-fix`/merge/archive costs a human more than an over-eager but harmless roadmap proposal.
