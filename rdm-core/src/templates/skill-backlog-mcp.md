---
name: rdm-backlog
description: Run a batched backlog grooming pass over rdm's stale/duplicate/tag-cluster/archivable signals and emit a reviewable, propose-only plan of exact rdm commands — no mutations
allowed-tools:
  - Read
  - Glob
  - Grep
  - {t_backlog_report}
  - {t_roadmap_list}
  - {t_search}
---

Run a batched **grooming pass** over the backlog and emit a reviewable plan. This skill **reads** `{t_backlog_report}` and **proposes** a set of consolidate / merge / retire / archive actions — each written as the exact `rdm` command a human would run — plus the open questions it could not answer confidently. It **never mutates the plan repo**: no create, update, merge, archive, promote, staging, or commit. Deciding what to actually change stays a human call; this skill does the analysis and hands over a ready-to-run batch.

This mirrors how `rdm-autopilot` batches blockers rather than guessing: surface everything at once, then stop.
{principles}
## Contract

`$ARGUMENTS` is **all optional**:

- `[--project <name>]` — plan repo to groom. If omitted, use `{proj_param}`.
- `[--older-than <days>]` — passed straight through to `{t_backlog_report}` to tune the staleness threshold.
- `[--tag <tag>]` — passed straight through to `{t_backlog_report}` to scope the scan.

There is **no positional argument** naming an item to change, because this skill changes nothing. Thread whatever `project`/`older_than`/`tag` you were given into the tool call below; do not hardcode a project.

## Non-mutation guarantee

This skill makes exactly **one** MCP tool call that touches plan data — `{t_backlog_report}` (read-only) — plus optional read-only `{t_roadmap_list}` / `{t_search}` lookups during analysis. It **never calls** any create, update, merge, archive, promote, or commit tool. Every action it proposes is text for a human to run later, never executed here — those proposed commands are written as literal `rdm` CLI commands (see "Grooming analysis" below), not MCP tool calls this skill makes: there is no MCP tool for merge/archive/promote today, and even where one exists (e.g. task update), this skill still only proposes the CLI form for a human to run. The `allowed-tools` above deliberately omit any write-capable tool.

## Steps

1. Parse `$ARGUMENTS` into optional `project`/`older_than`/`tag` values.
2. Fetch the candidates (read-only): call `{t_backlog_report}` with `project: {proj_param}[, older_than: <days>][, tag: "<tag>"]`.
   The response has four arrays: `stale_tasks` (`slug`, `title`, `status`, `created`, `age_days`), `duplicate_clusters` (`members`: `slug`/`title`), `tag_clusters` (`tag`, `tasks`: `slug`/`title`), and `archivable_roadmaps` (`roadmap`, `title`, `phase_count`).
3. **Empty case:** if all four arrays are empty, say plainly **"Nothing to groom — the backlog report returned no signals"** and **stop**. Do not fabricate a plan or emit an empty `## Grooming plan` / `## Open questions` skeleton.
4. Otherwise, run the grooming analysis below and emit the plan. If only *some* arrays are empty, **omit that category's subsection entirely** rather than printing an empty header.

## Grooming analysis

Turn each populated category into proposed actions. Emit the whole thing as **one reviewable batch** — a flat, ordered list of `{command, rationale}` pairs grouped by category under clear subheadings, so a human can copy/paste-run them top to bottom (or not). Every proposed mutating command must carry `--no-edit` where the verb supports it, because the human is expected to run them unattended later.

**These proposed commands are text for a human to run later, not MCP tool calls this skill makes.** Write them as literal `rdm` CLI invocations exactly as shown below, even though this skill itself only ever calls read-only MCP tools.

**Autopilot-oriented framing (read first):** whenever you propose creating or extending a thematic roadmap, the phase body you propose (the `--body`/stdin content for `promote`) must be structured with `## Context` / `## Steps` / `## Acceptance Criteria` headings — the same shape every existing phase body uses. That way, if a human executes the batch, a later `rdm-estimate` (which needs a body to rate difficulty from) and `rdm-autopilot` (which needs actionable phases) can pick the roadmap up immediately. The goal of every consolidation you propose is a roadmap that is ready for `/rdm-autopilot`.

- **`stale_tasks`** — for each task, decide:
  - *Retire* if it reads as superseded or no longer relevant:
    `rdm task update <slug> --status wont-fix --reason "<why it is stale / superseded>" --no-edit {proj_flag}`
  - *Consolidate* if it is still valid work that fits a theme, into an existing roadmap (`rdm promote <slug> --into <roadmap> --no-edit {proj_flag}`) or a new one (`rdm promote <slug> --roadmap-slug <new-slug> {proj_flag}`). Note: `--no-edit`/`--body` apply only to `--into`; omit them with `--roadmap-slug`.
  - Otherwise → **open question** (below), not a blind guess.
- **`duplicate_clusters`** — for each cluster, pick a survivor (state the rule you used: most complete body, or earliest `created`) and fold the rest in:
  `rdm task merge <survivor> --from <other1> --from <other2> --no-edit {proj_flag}`
  If no survivor is clearly best, → **open question**, do not guess.
- **`tag_clusters`** — a cluster of related tasks under one tag is a consolidation candidate. First check whether a thematic roadmap already covers it (call `{t_roadmap_list}` with `project: {proj_param}` / `{t_search}` with `project: {proj_param}, query: "<tag>", kind: "roadmap"`). If one exists, propose `rdm promote <slug> --into <existing-roadmap> --no-edit {proj_flag}` per task. If none exists, propose one `rdm promote <first-slug> --roadmap-slug <new-thematic-slug> {proj_flag}` (no `--no-edit`/`--body` — those apply only to `--into`) and then `rdm promote <slug> --into <that-new-slug> --no-edit {proj_flag}` for the rest, in that create-then-fold order. Never propose both `--into` and `--roadmap-slug` for the same task — they are mutually exclusive.
- **`archivable_roadmaps`** — each is already all-terminal, so propose:
  `rdm roadmap archive <roadmap> {proj_flag}`
  Rationale: "all phases terminal, not yet archived." **Never** add `--force` — these candidates never need it, and `--force` exists only to override *incomplete* roadmaps, which is not this skill's job.

## Open questions

Always end the plan with an `## Open questions` section (include it even when short) listing every candidate the analysis could not confidently resolve — an ambiguous retire-vs-keep, a duplicate cluster with two equally plausible survivors, a tag cluster too thin or mixed-theme to name a roadmap for. State the item(s) and *why* it is ambiguous, and attach **no command**. The rule: **never propose a merge / retire / archive / consolidate action you are not confident about — file it as an open question instead.** Hold destructive-if-wrong actions (retire, merge, archive) to a stricter confidence bar than purely additive ones (proposing a new roadmap), because undoing a wrong `wont-fix`/merge/archive costs a human more than an over-eager but harmless roadmap proposal.
