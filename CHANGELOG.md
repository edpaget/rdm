# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- New `rdm-backlog` Claude Code skill: a propose-only backlog grooming pass. It
  reads `rdm backlog report` and emits a single batched, human-reviewable plan
  that pairs each proposed consolidate / merge / retire / archive action with the
  literal `rdm` command that would carry it out and a one-line rationale, framing
  proposed roadmaps to be `/rdm-autopilot`-ready. It performs zero mutations
  (no create/update/merge/archive, no staging, no `rdm commit`), handles the
  empty-backlog case, and surfaces ambiguous or destructive-if-wrong decisions as
  open questions rather than acting on them. `rdm agent-config claude --skills`
  and `rdm agent-config pi --skills` now emit 10 skill files instead of 9; the
  `--mcp` variant is unchanged at 9 pending a dedicated `rdm_backlog_report` MCP
  tool.
- rdm now emits a warning to stderr when the global config (~/.config/rdm/config.toml) or repo config (rdm.toml) contains invalid TOML, instead of silently falling back to defaults.
- New `[models]` config table schema (`small`/`medium`/`large` model ids,
  `review_floor`, and per-step `[models.steps]` overrides) in `rdm.toml` and
  the global config, with repo-over-global merge semantics. Not yet consumed
  by any command — this lays the storage foundation for upcoming model-tier
  resolution.
- `rdm-core::model_policy` module with a `ModelPolicy` resolver that turns a
  dispatch step (`plan`, `implement`, `review-find`, `review-verify`,
  `mechanical`) plus an optional caller tier hint into a concrete model id,
  applying the `[models]` config from `rdm.toml`/global config with built-in
  defaults (`small`→`haiku`, `medium`→`sonnet`, `large`→`opus`, review floor
  `medium`). Review-reasoning steps are clamped up to the review floor;
  `mechanical` is exempt. Not yet wired into any CLI command or skill — this
  is the internal sizing engine upcoming phases will consume.
- `rdm model resolve <step> [--tier <tier>]` and `rdm model show` (`--format
  json` supported) — CLI porcelain over the `[models]` sizing policy,
  resolving a dispatch step (`plan`, `implement`, `review-find`,
  `review-verify`, `mechanical`) to a concrete model id, or inspecting the
  full resolved policy (tier bindings, review floor, per-step models).
- `rdm config set server.quick_filters "Label1:tag1,Label2:tag2"` (plus matching `rdm config get`/`rdm config list` support) configures HTML quick-filter chips without hand-editing `rdm.toml`. An empty value clears all chips; the key is repo-only (`--global` is rejected).
- `rdm backlog report [--older-than <days>] [--tag <tag>] [--project <project>]` — a read-only backlog grooming sensor. Prints stale tasks (`open`/`in-progress` tasks past a staleness threshold, default 60 days), likely-duplicate task clusters (via the existing fuzzy `search` matcher — no new similarity engine), thematic tag clusters among active tasks, and archivable roadmaps (every phase terminal, not yet archived). Supports `--format json` for structured output alongside the default human/markdown rendering. Performs zero writes and requires no plan-repo schema change.
- `rdm bootstrap` gained a `--print-root` flag (prints only the resolved
  plan-repo path to stdout, with all narration moved to stderr) and
  `--format json` support (`{"path", "status", "commits_merged"}`), for
  reliable scripting in session-start hooks instead of parsing the human
  success banner. `--print-root` takes precedence over `--format` when
  both are given. `templates/claude-code-web/.claude/hooks/SessionStart.sh`
  now uses `--print-root` instead of a `sed`-based parse of bootstrap's
  stdout.
- `rdm promote <task-slug> --into <existing-roadmap-slug>` consolidates a task
  into an already-existing roadmap as a new trailing phase (auto-numbered),
  carrying over the task's body (prefixed with a provenance line naming the
  source task) and tags, instead of creating a brand-new 1:1 roadmap. `--body`
  and `--no-edit` are supported to override the phase body content. The
  source task is not deleted — it is closed as `done` with a pointer note
  naming the new roadmap and phase, so it drops out of the active `task
  list` while remaining inspectable. Consolidating into a nonexistent
  roadmap, or a task that is already `done`/`wont-fix`, fails with an
  actionable error. The existing `--roadmap-slug` (create-new-roadmap) form
  is unchanged.
- `rdm task merge <survivor> --from <a> --from <b>` (comma-separated or
  repeated) folds one or more duplicate tasks into a survivor: it unions the
  sources' tags into the survivor, appends each source body under a `## Merged
  from task <slug>` heading (in `--from` order), and closes every source
  `wont-fix` with a `superseded by task/<survivor>` pointer note. Merged
  sources drop out of the active `task list` but stay inspectable with their
  provenance intact. The merge is idempotent — re-running it is a no-op — and
  self-merges, unknown survivors, unknown sources, and empty `--from` all fail
  with actionable errors before any change is written.
- `rdm task update --reason <text>` / `--clear-reason` records (or clears) a
  persisted close reason on a task — e.g. why it was retired as `wont-fix`.
  The reason survives read-back, is shown by `task show` and included in
  `task show --format json` (`close_reason`), and is preserved across later
  status changes until explicitly cleared.

### Changed

- The generated `rdm-review` skill (CLI and MCP variants) and the dogfood
  `.claude/skills/rdm-review/SKILL.md` now size the review fleet via the
  `[models]` policy instead of inheriting the session's model: step 1 derives
  a `small`/`medium`/`large` tier hint from the diff's blast radius, step 2
  resolves `rdm model resolve review-find --tier <hint>` (or `mechanical` for
  scripted checks) for each dispatched finder agent, and step 3 resolves `rdm
  model resolve review-verify` for the refute pass. Every dispatched agent is
  now given an explicit `model`, closing the session-model-inheritance leak.
- `rdm-autopilot` and `rdm-dispatch-phase` skills now state an explicit **synchronous dispatch contract**: every subagent (per-phase dispatch, planner, plan reviewer, implementer) is spawned synchronously and its returned result is the sole channel back — no background-and-poll, no resume-by-message, and no `SendMessage` to a parent (`"claude"` does not resolve); rework always spawns a fresh subagent.
- `rdm-autopilot` and `rdm-dispatch-phase` skills now make subagent dispatch a
  non-skippable **MUST**: a new "Mandatory dispatch — no inline work" section
  in each skill explicitly prohibits doing the planning/implementation/review
  inline, requires a pre-action declaration of which subagent/role is being
  dispatched, and adds a "Self-check before proceeding" checkpoint restated
  at each dispatch point (`rdm-autopilot` step 2; `rdm-dispatch-phase` steps
  4-6), plus a "mandatory, not best-effort" lead-in on `rdm-dispatch-phase`'s
  Context isolation section and a named negative example
  ("inline-collapse") of the failure this closes. The CLI and MCP-variant
  templates under `rdm-core/src/templates/` are brought into sync with both
  this change and phase 1's synchronous-dispatch wording, which they had
  been missing. See `docs/subagent-dispatch-enforcement.md` for the
  evaluated techniques and rationale.
- `rdm-dispatch-phase` and `rdm-autopilot` skills now include explicit guidance on safe operations under `--permission-mode auto`: use `Edit` (surgical) rather than `Write` (whole-file overwrite) when modifying existing tracked files, and never run destructive git operations (`git stash -u`, `git reset --hard`, `git clean -fdx`) that trigger the auto-mode permission classifier and stall unattended runs. The guidance points to the per-roadmap worktree isolation as the alternative (commit a WIP commit instead of stashing; clean up the worktree after the phase is done).
- `rdm worktree add` without `--base` now defaults the new branch to the invoking checkout's current branch instead of always basing off `main`'s `HEAD`, so a worktree created while on a feature branch builds on that branch's work. Detached HEAD (or the invoking branch matching the item's own target branch) still falls back to the prior `HEAD` default; an explicit `--base <ref>` always takes precedence.

### Fixed

- `rdm serve` now resolves `?at=<sha>` historical-revision requests against the plan repo's real git history by default (when the plan root is a git repository), instead of always returning 404 "the store has no history available." Non-git plan roots keep the previous behavior. Note: this applies to the git-featured build shipped via `rdm-cli` (the default); a standalone `rdm-server` build without the new `git` cargo feature keeps FsStore-only (always-404) behavior.
- `rdm bootstrap doctor` now correctly detects rdm on PATH on Windows by checking for `.exe`, `.bat`, and `.cmd` extensions in addition to the unextended name.
- When running a non-init command against an uninitialized plan repo (no
  `rdm.toml` at the resolved root), the error now clearly guides users to
  `rdm init`: "no plan repo found at {path} — run `rdm init` to create one",
  instead of the opaque "failed to open git repository" error. Commands that
  do not require a repo (`rdm init`, `rdm describe`, `rdm agent-config`, `rdm
  model`, and `rdm bootstrap` and `rdm hook` when git is enabled) proceed
  normally without requiring `rdm.toml`.

## [0.16.0] - 2026-07-04

### Security

- Upgraded the MCP server's `rmcp` dependency to 2.0.0, which fixes an OAuth
  resource-spoofing vulnerability, a metadata SSRF, and a streamable-HTTP
  session leak, and aligns tool-response content encoding with the MCP
  2025-11-25 specification.

### Changed

- `rdm task list` with no `--status` now shows all active tasks (open,
  in-progress, needs-review, reviewed) instead of only open/in-progress,
  matching the REST server's default. Done and wont-fix remain hidden by default.
- The `post-merge`/`post-commit` hooks now apply all `Done:` directives from a
  single hook invocation as one plan-repo commit (with a single `INDEX.md`
  regeneration) instead of one commit per directive. The commit message
  enumerates every applied `Done: <target>` directive alongside its source
  commit SHA, so per-directive provenance is preserved without a per-directive
  commit. Note: if the shared index-regeneration/commit step fails, no
  directive in that batch is committed (per-directive log lines plus a
  `batch-commit-error` event still record what happened) — previously an
  unrelated failure could not undo an already-committed directive's commit.
- Clarified in `--help` (for `roadmap`/`phase`/`task` `create`/`update`) and in
  the agent-facing docs (`CLAUDE.md`, `rdm agent-config`'s CLI instructions
  template) that `--body` accepts any text verbatim — backticks, em-dashes,
  and other Unicode/punctuation included — and always takes precedence over
  stdin, which is never read once `--body` is set. No behavior changed;
  `--body` was already authoritative (fixed in a prior release) and does not
  hang on special-character content — this is documentation-only, backed by
  new regression tests.
- **BREAKING:** The opt-in staging mode is gone — every mutating `rdm`
  command (`roadmap`/`phase`/`task`/`review`/`promote`/etc. create, update,
  delete) now stages its change to disk and defers the git commit
  **unconditionally**; there is no longer a way to auto-commit per mutation.
  The `--stage` flag, the `RDM_STAGE` environment variable, and the `stage`
  field in `rdm.toml` (both repo-level and global config) are removed —
  passing `--stage` or setting `RDM_STAGE`/`stage` is now a plain unknown-flag
  / ignored-config-key situation rather than a behavior toggle. **Migration:**
  after a batch of mutating commands, run `rdm commit -m "..."` to land them
  as one git commit; `rdm status` shows what's pending and `rdm discard
  --force` reverts it. `rdm hook post-commit`/`post-merge` and `rdm bootstrap`
  are unaffected — they already committed unconditionally through an internal
  always-commit pathway and continue to do so. See the MCP bullet below for
  the equivalent change on the MCP server, which is now stage-only and
  exposes `rdm_status`/`rdm_commit`/`rdm_discard`.
- MCP mutation tools now stage changes to disk and require an explicit
  `rdm_commit` to land them — no more auto-commit per mutation. `*_update`
  responses no longer carry a `Commit:` trailer and
  `rdm_review_address_comment` no longer auto-defaults `applied_commit`;
  thread the SHA from `rdm_commit`'s response instead.
- The `rdm-autopilot` agent skill (CLI and MCP variants) now dispatches each
  phase — including the `rdm-estimate` step when a difficulty is unset — as a
  single isolated `Agent` subagent instead of invoking `rdm-estimate` and
  `rdm-dispatch-phase` inline via the `Skill` tool. Only the structured
  `{roadmap, phase, outcome, summary, findings}` outcome crosses back into the
  loop, so the loop's context stays flat across a multi-phase run instead of
  accumulating every phase's plan/plan-review/implementation/code-review detail.
  No change to how the loop interprets `reviewed`/`rework`/`escalated`, the
  per-phase rework-retry budget, or blocked-parking. See
  [`docs/autonomous-loop.md`](docs/autonomous-loop.md).
- The `rdm-dispatch-phase` agent skill (CLI and MCP variants) now splits the
  planning agent from the implementing agent. The planning subagent (step 4)
  returns a **self-contained plan document** — steps mapped to each acceptance
  criterion, a file/crate navigation map, and a per-AC test list — and the
  independent plan gate (step 5) approves that exact document. Implementation
  (step 6) is then handed to a **new** implementer subagent seeded only with the
  phase body and the approved plan document, rather than reusing the planner's
  accumulated exploration context. The plan document carries the navigation map
  forward so the implementer inherits it instead of re-discovering it. The skill's
  "Context isolation" section now names the planner→implementer boundary
  explicitly alongside the existing planner→reviewer one.
- The `rdm-dispatch-phase` plan gate (step 5, CLI and MCP variants) now scales
  its rigor to the phase's difficulty tier instead of applying one fixed level
  of scrutiny: trivial/easy gets a holistic single-reviewer judgment, moderate
  requires the reviewer to cite per-finding evidence (the specific AC text,
  plan step, or file/crate each checklist judgment rests on), and hard adds a
  refute pass — a third, fresh subagent whose only job is to refute the
  reviewer's verdict, adapted from (not a faithful mirror of) `rdm-review`'s
  verify step: it is deliberately one-directional and can only tighten the
  gate's verdict, never loosen it. The checklist itself is sharpened for every
  tier: acceptance-criteria→step and acceptance-criteria→test mappings, edge
  cases/error paths per AC, and declared cross-phase/cross-crate dependencies,
  alongside the existing scope and architecture checks. The `revise` round now
  explicitly re-checks the revised plan against the same checklist, and an
  un-converged revise round escalates (stage `plan`) rather than silently
  proceeding with a deficient plan.

### Fixed

- The CLI's "staged — run `rdm commit` to persist" and "N uncommitted
  change(s)" hints now print to stderr instead of stdout. Since staging is
  the only workflow (no more opt-in `--stage`), these hints previously fired
  on every mutating and read command, corrupting machine-readable stdout
  (`--format json` and any piped/captured output).
- Roadmap status now treats `needs-review` and `reviewed` phases as active work: a roadmap whose phases are only in review states (no `in-progress` phase) is reported as `in-progress` instead of `not-started`, in the CLI and the server UI's roadmap status badge.
- REST API 400/422 responses for invalid status values in request filters and
  updates now list the complete status set including `needs-review` and
  `reviewed`, derived from the core `ParseError` instead of hand-maintained
  literals. This affects `/projects/:project/tasks?status=` (GET), `PATCH
  /projects/:project/tasks/:task`, `/projects/:project/roadmaps/:roadmap/phases?status=`
  (GET), and `PATCH /projects/:project/roadmaps/:roadmap/phases/:phase`.

### Added

- New `hook_timeout_secs` config option (repo `rdm.toml` and global config,
  same precedence as `default_branch`) bounds how long `rdm hook post-merge`
  / `rdm hook post-commit` may run before giving up. Defaults to 30 seconds
  when unset (a configured `0` is treated the same as unset — an unbounded
  timeout would defeat the point of the guard). A hook that hits its
  deadline logs a `timeout` event and still exits 0, so it can never block
  the invoking `git commit`/`git merge` indefinitely.
- New `rdm_status`, `rdm_commit`, and `rdm_discard` MCP tools mirroring the
  CLI's `status`/`commit`/`discard`: inspect staged changes (path + change
  kind), land a batch as one commit with an explicit or auto-generated
  message, or discard staged changes (requires `confirm: true`). Gated on
  the `git` feature.
- The MCP server now exposes the LLM revision workflow, so an agent can
  discover and act on document reviews entirely over MCP:
  - `rdm_review_requests` — the change-request queue (submitted reviews
    with verdict `request-changes`), each entry carrying its target,
    author, summary, `created_commit`, and `open_comment_count`, with
    optional `target_kind`/`target_id` filters.
  - `rdm_review_show` — the full review in one call: summary, every
    comment with its tagged-union anchor (`anchor_type`) and computed
    resolution (`resolved`/`drifted`/`unresolved` with byte range and
    which body it indexes), plus a `documents` array inlining each
    referenced document's body at the review's `created_commit` and at
    HEAD. Unknown anchor types round-trip verbatim and resolve as
    `unresolved` (whole-document treatment).
  - `rdm_review_address_comment` — flips a comment to `addressed` or
    `wont-fix`, records the `applied_commit` provenance SHA, and stores
    the agent's reply; omit `status` to leave the comment open with a
    clarification reply. `applied_commit` is passed through verbatim
    (including for `wont-fix`) and left `null` if omitted — thread the
    `Commit:` value the `rdm_commit` tool reports for the batch that
    applied the fix.
  - `rdm_review_complete` — closes a review as `addressed`, refusing
    while any comment is open and listing the offending comment ids.
- New `rdm-revise` Claude Code / Pi skill (generated by
  `rdm agent-config --skills` in both CLI and MCP variants, and invocable
  as `/rdm-revise`): walks an agent through the revision loop — read the
  review summary for intent first, dispatch each comment on its
  `anchor_type` (resolved span, drifted span, or whole-document), apply
  edits through `rdm ... update`, record `applied_commit` + reply per
  comment, ask for clarification (leaving the comment open) when an
  anchor drifted beyond recovery, `wont-fix` with reasoning as the escape
  hatch, and close the review once nothing remains open.
- `rdm agent-config` instructions (CLI and MCP variants) now document the
  document-review agent loop and the new MCP review tools.
- New core helper `rdm_core::ops::reviews::change_requests` — the single
  definition of the change-request queue shared by `rdm review requests`
  and the MCP `rdm_review_requests` tool.
- New hermetic regression harness
  `scripts/verify-review-revision-loop.sh` covering the resolved-anchor,
  whole-document, drifted-anchor-clarification (blocks close), wont-fix,
  and completion paths of the revision loop end to end.
- With JavaScript enabled, `rdm-server` detail pages now support
  GitHub-style select-to-anchor review comments: while your draft review
  is open, highlighting text in a rendered roadmap, phase, or task body
  pops an "Add review comment" affordance that attaches a
  text-quote-anchored comment to the draft. The selection is mapped back
  to the markdown source (formatting spans, inline code, lists, tables,
  and multi-byte text included) and re-validated server-side before
  anything is stored, so the anchor always re-resolves to the exact
  selected span and later renders as an inline highlight; selections that
  cannot be mapped confidently degrade to a general comment carrying the
  selected text as a blockquote plus a visible "no anchor attached" note —
  a wrong anchor is never stored. The draft panel now updates in place
  without a full page reload and shows a quote preview on pending
  anchored comments. The plain-HTML review flow keeps working unchanged
  with JavaScript disabled.
- The roadmap detail page now renders each phase's body inside a
  collapsed, keyboard-accessible disclosure (native `<details>` — the
  phase list replaces the old table). Expanding a phase and selecting
  text in its body attaches the comment to the open roadmap draft scoped
  to that phase, so phase-level feedback can be authored from the roadmap
  page. Phase bodies are omitted when viewing a pinned `?at=` revision,
  and inline review highlights still live on the phase pages (the
  existing cross-links).
- `rdm-server`'s roadmap, phase, and task detail pages now support
  authoring reviews entirely through plain HTML forms — no JavaScript
  required. A "Start review" form begins (or resumes, if one is already
  open on the document by the same author) a draft; a server-rendered
  draft panel lets you add whole-document comments (with an optional
  phase-scoped dropdown on roadmap pages), edit or remove pending
  comments, and submit with a required verdict (Comment / Approve /
  Request changes) plus a summary. Draft comments stay private to the
  draft panel and never appear in the public Reviews section until
  submitted. Submitted reviews gain an inline Dismiss control, and drafts
  a Delete button. Author identity comes from the form and is remembered
  across visits via an `rdm_author` cookie so the panel resumes your own
  open draft, not someone else's. Validation and lifecycle errors
  (missing verdict, blank comment, editing after submit, out-of-scope
  phase scope) redirect back to the page with a readable inline banner
  instead of a raw Problem+JSON body.
- `rdm-server`'s roadmap, phase, and task detail pages now render a Reviews
  section: every non-draft review of the document (submitted, addressed, or
  dismissed) with its state and verdict badges, author, relative timestamp,
  summary, and comments in order — drafts are never shown. Anchored
  comments show a quote preview; on the current body, hovering or
  keyboard-focusing the preview highlights the resolved span inline in the
  rendered body (a small new `/static/review-highlight.js` script; pages
  stay fully readable with JavaScript disabled, degrading to the quote
  preview). Anchors that no longer resolve (or, once a history-aware
  backend lands, have drifted) get an "outdated" badge and show the
  original quote instead of a highlight. Roadmap-review comments scoped to
  a phase (`doc`) link through to that phase — where they render and
  highlight — and link back to the roadmap review from there. The roadmaps
  and tasks list pages gain a Reviews column with open-review/open-comment
  counts per item (phase-targeted reviews rolled up into their roadmap,
  matching `INDEX.md`), linking to the item's Reviews section.
- `rdm-core`: `ops::reviews::count_open_reviews` (and its slice-based
  sibling `count_open_reviews_in`) is the single open-review/open-comment
  counting pass shared by `INDEX.md` generation and the web list pages, so
  the two surfaces can never report different numbers.

- `rdm-server` now exposes a REST API for document reviews under
  `/projects/:project/reviews`: `GET` lists reviews as lightweight metadata
  summaries (filterable by `?on=<kind>/<id>`, `?state=`, `?verdict=`, and
  `?author=`); `POST` starts a draft (`{"target": "<kind>/<id>"}` plus
  optional `author` and initial `summary`); `GET /:id` returns the full
  detail — summary, comments, and each comment's anchor resolution
  (resolved / drifted / unresolved, with the quoted text, byte range, and
  which body the range indexes) so clients can highlight without extra
  calls; `POST /:id/comments` adds a comment (optionally anchored via a
  tagged `anchor` object or scoped to a roadmap phase via `doc`); `PATCH
  /:id/comments/:n` edits a draft comment's `body`/`anchor`/`doc` — omit a
  field to keep it, send `null` to clear it, or send a value to replace it
  — or, once submitted, records `status`/`applied_commit`/`reply`; `POST
  /:id/submit` stamps a `verdict` (optionally replacing the `summary`);
  `PATCH /:id` transitions to `addressed` or `dismissed`; and `DELETE /:id`
  removes drafts (submitted reviews are part of the record and return 409).
  All lifecycle rules are enforced by `rdm-core` and surface as RFC 9457
  Problem+JSON with actionable detail, and an anchor with an unrecognized
  `anchor_type` round-trips through the API untouched.
- `rdm-core::anchor::resolve_comments` runs the per-comment anchor
  resolution pass for a whole review in one call — the shared helper behind
  both the CLI's review rendering and the new server review endpoints.

- The full review-authoring loop is now available on the CLI, joining the
  existing needs-review queue commands under `rdm review` (whose `--help`
  now groups the two families): `start --on <kind>/<id>` creates a draft
  review of a roadmap, phase (`phase/<roadmap>/<stem-or-number>`), or task;
  `comment <id>` appends a comment — with `--quote "<text>"` the quoted
  text is located in the document **as of the review's `created_commit`**
  and a text-quote anchor (with ~32 chars of surrounding context) is
  derived automatically, an ambiguous quote fails with a 1-based occurrence
  list to disambiguate via `--occurrence <n>`, and `--doc
  phase/<stem-or-number>` scopes a roadmap-review comment to one of its
  phases; `submit <id> --verdict approve|request-changes|comment` finalizes
  the draft (optionally replacing the summary with `--body`); `list`
  filters by `--on`/`--state`/`--verdict`/`--author`; `show <id>` renders
  the summary and each comment with its anchor quote and resolution state
  (resolved / drifted / unresolved), with `--no-body` to suppress bodies;
  `update <id>` records comment resolutions (`--comment <n> --status
  addressed|wont-fix [--applied-commit <sha>] [--reply "..."]`) and closes
  the review (`--state addressed|dismissed`), validated by the lifecycle
  state machine; `delete <id>` removes drafts (submitted reviews require
  `--force`); and `requests` is the agent work queue (submitted reviews
  requesting changes). `--format json` on `list`, `show`, and `requests`
  includes each comment's full anchor and its resolution (with the quoted
  text and whether the range indexes the original or current body), so
  agents need no second call. All review mutations respect staging mode and
  the `--project`/`RDM_PROJECT`/`default_project` resolution chain.

- `rdm search` now indexes reviews: summaries and comment bodies are
  searchable, and `--type review` narrows results to them. `rdm describe
  review` documents the review entity, and `rdm agent-config` output
  teaches agents the new review commands.

- Anchored review comments can now be located within a target's body and
  re-located after the body is edited: exact-quote matching with
  prefix/suffix disambiguation for repeated text, and a fuzzy context
  fallback that recovers a drifted span from its surviving surrounding
  context. History-aware resolution finds the span in the body the reviewer
  originally saw (at the review's recorded commit), flags whether it has
  since drifted, degrades to the current body when history is unavailable,
  and reports unresolved (never failing) for unknown revisions, deleted
  targets, and unrecognized anchor types. Library-only groundwork
  (`rdm_core::anchor`); no CLI or API surface yet.

- Review operations in `rdm-core`: `create_review`, `add_comment`,
  `update_comment`, `remove_comment`, `submit_review`, `update_review`,
  `get_review`, `delete_review`, and review filtering (by target, state,
  verdict, and author) enforce the review lifecycle (`draft` → `submitted` →
  `addressed` | `dismissed`). Submitting requires a verdict and a non-empty
  review (at least one comment or a summary); comment structure locks after
  submission (only a comment's status, applied commit, and reply may still
  change); `addressed` requires every comment resolved; and terminal states
  reject further transitions. Creating a review validates the target exists
  and stamps the plan-repo HEAD the reviewer saw (`created_commit`), even
  under staging mode. `INDEX.md` now shows, per roadmap and per task, the
  count of open (submitted) reviews and the open comments within them —
  phase-targeted reviews roll up into their roadmap's row. Library-only
  groundwork: no CLI or API surface yet.

- A foundational Review data model in `rdm-core`. Reviews of a roadmap, phase,
  or task are stored as markdown files under a project's `reviews/` directory
  (`reviews/<id>.md`) with the review summary as the body and all metadata —
  state, verdict, timestamps, and the full list of inline comments — in the
  frontmatter. Comments can be anchored to a quoted span of the target's body
  (text-quote anchors with surrounding context), and anchor types this build
  does not recognize round-trip losslessly, so an older rdm never corrupts
  reviews written by a newer one. This is groundwork only: reviews can be
  written, loaded, and listed through the core library, with no CLI or API
  surface yet.

- `roadmap update`, `phase update`, and `task update` now accept `--title
  <TITLE>` to rename an item in place. Only the frontmatter title changes — the
  slug (for roadmaps/tasks) and the stem/number (for phases) are never touched,
  and `INDEX.md` is regenerated as part of the same mutation. An empty or
  whitespace-only `--title` is rejected with an actionable error (titles are
  required and cannot be cleared); omit `--title` to leave the existing title
  unchanged.

### Fixed

- Fixed a class of hangs that could leave a plan-repo mutation (or `rdm hook
  post-merge`/`post-commit`) stuck indefinitely, requiring a manual
  `SIGTERM`, in `--auto`/agent-driven flows:
  - Every git subprocess rdm spawns is now hardened to be strictly
    non-interactive — `GIT_EDITOR`/`GIT_SEQUENCE_EDITOR` are forced to a
    no-op so a merge that would otherwise open an interactive commit-message
    editor can't block on it, and `GIT_TERMINAL_PROMPT`/`GIT_ASKPASS` are
    forced so a `fetch`/`push` against an authentication-required remote
    fails fast instead of hanging on a credential or host-key prompt. This
    holds regardless of the invoking user's `core.editor`/`GIT_EDITOR`/
    `VISUAL`/credential-helper configuration.
  - `rdm remote pull`'s diverged-history merge now also explicitly passes
    `--no-edit` (defense-in-depth alongside the blanket editor hardening
    above).
  - `rdm hook post-merge`/`post-commit` now detect when they were themselves
    spawned as a git subprocess by rdm (as can happen if a real `git commit`
    made by `rdm resolve` re-triggers the plan repo's own installed hooks)
    and short-circuit immediately instead of re-running the `Done:`-directive
    pipeline.
  - `rdm hook post-merge`/`post-commit` execution is now bounded by the new
    `hook_timeout_secs` deadline (see Added, above) as a last-resort backstop.

## [0.15.0] - 2026-07-01

### Added

- A regression harness for the auto-review Stop hook loop,
  `scripts/verify-auto-review-hook-loop.sh`. It drives the real hook scripts
  end-to-end in hermetic temp dirs and asserts all four contract states against
  the concrete `.claude/hooks/rdm-review-on-finalize.sh` — fires
  `{"decision":"block",...}` when an item is `needs-review` on the current
  branch, stays silent on an unrelated branch, stays silent under the
  `stop_hook_active` loop guard, and stays silent once the item is `reviewed` —
  plus the shipped `rdm-core/src/templates/hook-review-on-finalize.sh`'s
  previously-untested loop-guard and reviewed-cleared cases. Auto-picked-up by
  the existing `scripts/verify-*.sh` CI glob.

- `phase update`/`task update --status needs-review` now warns on stderr when
  HEAD carries no committed changes worth reviewing, surfacing items that would
  otherwise be silently stranded in `needs-review` with nothing to review. For a
  phase, the baseline is the previous finalized phase in the same roadmap (so the
  long-lived `roadmap/<slug>` branch in the one-worktree-per-roadmap model is
  handled correctly): it warns when HEAD has not advanced past that phase's
  committed work. For the first phase of a roadmap and for standalone tasks, the
  baseline is the configured `default_branch`: it warns when HEAD has no commits
  beyond it. The transition is not blocked and the check fails open on any git
  error — this is a non-destructive data-integrity nicety, not a gate.

- `rdm review restamp` refreshes `review_sha`/`review_branch` on every in-scope
  `needs-review` item to the current source-repo HEAD and branch. Run it after
  amending or rebasing a commit while an item is still `needs-review`: the
  original stamp would otherwise point at a now-dangling commit and the item
  could silently drop out of `rdm review pending` scope (via the
  SHA-reachability fallback), suppressing the auto-review reprompt. Scope
  matches `review pending` exactly, and it is idempotent (items already stamped
  at the current HEAD/branch are left untouched). The Claude Stop hook, the Pi
  `agent_end` extension, the generated `hook-review-on-finalize.sh` template,
  and the shipped `.claude/hooks/rdm-review-on-finalize.sh` now call it
  automatically before checking `review pending`, so this self-heals
  transparently in the normal finalize → review loop.

- `rdm worktree prune` removes every worktree whose plan item is already `done`
  in one command. Resolves each rdm-managed worktree's item status (phase, task,
  or whole roadmap) and removes the done ones; dirty worktrees are skipped unless
  `--force`, `--delete-branch` also deletes their merged branches, and
  `--dry-run` reports what would be removed without changing anything. See
  [`docs/landing.md`](docs/landing.md).
- `rdm agent-config --skills` now emits an `rdm-land` skill (CLI and MCP
  variants) that lands a `reviewed` item to `main` with **linear history**
  (rebase onto `main`, then `git merge --ff-only` — never a merge commit),
  re-running the CI-equivalent checks on the rebased branch first. The
  fast-forward flips the item to `done` via the existing post-commit hook, and
  the skill then cleans up the worktree (`rdm worktree remove --delete-branch`,
  or `rdm worktree prune` for batch cleanup). On rebase conflict or failing
  checks it aborts cleanly and escalates per `docs/escalation-protocol.md`
  instead of force-merging. Landing runs only on explicit invocation (or
  autopilot's opt-in `--land`); it never auto-lands. See
  [`docs/landing.md`](docs/landing.md).
- `rdm-autopilot` agent skill (shipped by `rdm agent-config --skills`, in both
  CLI and MCP variants). It drives **one named roadmap** from `not-started` to
  `reviewed` unattended: each iteration asks `rdm next` for the next actionable
  phase, estimates it (`rdm-estimate`) if needed, dispatches it on its model
  tier through `rdm-dispatch-phase` (plan gate, implementation, `rdm-review`),
  interprets the `reviewed`/`rework`/`escalated` outcome, and advances —
  parking a phase `blocked` when its rework budget is exhausted so the loop
  steps past it. Decisions and blockers are **batched, not raised mid-run**
  (review them with `rdm review blocked`), and the run is bounded by a global
  step budget. Opt-in `--land` (default OFF — `main` is never touched without
  it) and bounded `--plan-only` / `--max-phases` dry-run modes. See
  [`docs/autonomous-loop.md`](docs/autonomous-loop.md).

### Fixed

- `rdm worktree prune --delete-branch` now reports partial success when a done
  item's worktree is removed but its branch is retained because the branch is not
  merged into HEAD (and `--force` was not passed). Previously the whole operation
  was reported as `failed` even though the worktree removal succeeded, so the
  `removed`/`failed` counts misled and the orphaned branch could not be
  re-cleaned by a later prune. There is now a distinct `removed-branch-kept`
  action with a per-result `reason`, a top-level `branch_kept` count in
  `--format json`, and a matching `N branch kept` figure and `removed, branch
  kept (…)` note in the text summary.
- `rdm worktree prune` now reports a worktree that became dirty between the
  initial scan and its removal as `skipped-dirty` rather than `failed`.
- `rdm worktree add`/`list`/`remove` now work when the project's canonical repo
  is **bare** (no working tree of its own) — whether invoked from a linked
  worktree of the bare repo or from inside the bare directory itself. Previously
  every subcommand failed with "not inside a git repository". Sibling worktrees
  are placed under `<parent>/<repo-name>__worktrees/`, with any `.git`/`.bare`
  suffix stripped from the anchor name so the layout matches the normal-repo
  case.
- `rdm worktree` commands run against a working directory that does not exist (or
  is not a directory) now report an actionable "directory does not exist" error
  instead of the misleading "git is not installed".

### Changed

- The `rdm-do`, `rdm-review`, and `rdm-dispatch-phase` skills emitted by `rdm
  agent-config` (CLI and MCP variants) now describe the split `Done:` line as a
  single deferred two-stage protocol. The finalize step says the `Done:` line is
  withheld _YET_ because `rdm-review` adds it on a passing review, and the review
  gate's `git commit --amend` step says it is _completing_ that deferred
  directive — so neither stage reads as contradicting the other. This stops
  auto-mode agent permission classifiers from denying the review-time amend as a
  violation of the finalize-time "no `Done:` line" instruction.
- The `rdm-review` skill emitted by `rdm agent-config` now runs the full
  **find → verify → filter → report → act → gate** pipeline: an adaptive review
  fleet (base AC-compliance + correctness agents, plus conditional agents gated
  on what the diff touches), a per-finding adversarial refute/verify pass where
  the agent that finds an issue is never the one that confirms it, and a
  confidence filter that drops refuted or low-confidence findings before
  anything is fixed or filed. This brings the generated skill to parity with the
  in-repo dogfooding `rdm-review` skill (previously it shipped an older
  fixed-two-agent find → report → act flow). Both the CLI and MCP flavors are
  updated.
- The `rdm-review` skill emitted by `rdm agent-config` now escalates a review to
  a **BLOCKED** verdict when any surviving finding is `blocking`, matching the
  in-repo skill. It adds an explicit severity scale, a strict verdict-order
  (BLOCKED → FAIL → PASS WITH CONCERNS → PASS), and a gate that routes a BLOCKED
  phase to `blocked` and a BLOCKED task to `in-progress` (tasks have no `blocked`
  status) instead of silently downgrading blockers to "pass with concerns". Both
  the CLI and MCP flavors are updated.

## [0.14.0] - 2026-06-29

### Changed

- _Development:_ the shared `.githooks/pre-commit` gate is now driven by
  [`hk`](https://hk.jdx.dev/) (declared in `hk.pkl`, provisioned by `mise
  install`) instead of a hand-rolled cargo script. The repo's shell scripts are
  now linted (`shellcheck`) and formatted (`shfmt`, 4-space / indented case via
  `.editorconfig`) the same way Rust is, enforced in both pre-commit and CI. CI
  additionally runs the `scripts/verify-*.sh` integration harnesses (on pushes
  to `main` and on pull requests). `post-commit` / `post-merge` (the rdm `Done:`
  hooks) are unchanged.
- The `rdm-dispatch-phase` agent skill now targets **one worktree per roadmap**,
  reused across phases: step 3 creates (or idempotently reuses) the roadmap's
  shared `roadmap/<slug>` worktree via `rdm worktree add <slug>` instead of a
  per-phase `<slug>/<phase-stem>` worktree, so an autonomous roadmap run no
  longer spins up a fresh worktree for every phase. The auto-review Stop hook
  and Pi extension are unchanged in behavior but now document the one-worktree
  model (they fire from the roadmap worktree on the `roadmap/<slug>` branch, so
  the branch-scoped review filter resolves exactly that roadmap's items), and
  the README worktree docs cover roadmap-scoped worktrees.
- Finalizing a phase or task into `needs-review` now also stamps the branch of
  the checkout that produced it (`review_branch`), and `rdm review pending`
  scopes the queue to the current checkout's branch: it keeps only items whose
  stamped branch matches, so a roadmap's review trigger can never pick up
  another roadmap's items even when it fires from a different checkout. Legacy
  items finalized before this change carry no branch and fall back to the
  previous SHA-reachability behavior (fail open), so nothing pre-stamp is
  dropped. The `rdm review pending --format json` output now includes a
  `branch` field.

### Added

- A cross-host worktree-review regression harness,
  `scripts/verify-worktree-review-loop.sh`. It drives the full
  do → finalize → trigger → review loop for the one-worktree-per-roadmap model
  across two roadmaps in hermetic temp dirs and asserts roadmap isolation — a
  roadmap's review trigger fires that roadmap's review and stays silent about
  the other — across both supported host paths (the Claude Stop hook template
  and the Pi `agent_end` contract), including that a trigger from the `main`
  checkout never misfires for an in-flight roadmap review.
- `rdm worktree add <roadmap-slug>` now accepts a bare roadmap reference and
  creates (or idempotently reuses) a single worktree on a `roadmap/<slug>`
  branch — one worktree per roadmap, shared by all its phases — at
  `<repo>__worktrees/roadmap-<slug>`. `rdm worktree current` reports that
  roadmap context (via marker or by inverting the `roadmap/<slug>` branch name),
  and `rdm worktree remove` accepts the bare roadmap form too. Also exposed
  through the `rdm_worktree_add` / `rdm_worktree_current` MCP tools. This is the
  isolation unit for the one-worktree-per-roadmap rdm-do flow.
- `rdm worktree current` reports the plan item the current checkout corresponds
  to — the rdm worktree marker if present, otherwise the item inferred from the
  branch name (`phase/<roadmap>/<stem>` or `task/<slug>`), so a hand-made
  worktree or the main checkout sitting on an item branch is also recognized. It
  prints `Not in an rdm worktree.` (text) / `null` (JSON) and exits 0 when the
  checkout is on neither (e.g. the main checkout on `main`). Exposed as the
  `rdm_worktree_current` MCP tool as well. This is the detection primitive the
  `rdm-do` skill will use to reuse the current worktree instead of creating a
  redundant nested one.
- `rdm-dispatch-phase` agent skill (shipped by `rdm agent-config --skills`, in
  both CLI and MCP variants). It runs a single roadmap phase end-to-end in an
  isolated worktree on the phase's assigned model tier and returns a structured
  outcome (`reviewed` | `rework` | `escalated`) for an orchestrator to act on. A
  fresh implementer subagent is seeded with only that phase's body and the repo,
  drafts a tactical plan, and — because autopilot has no human to approve the
  plan — a *separate*, lightweight reviewer gates the plan against the phase's
  acceptance criteria, scope, and the core/cli/server separation before any code
  is written, returning approve / revise / escalate. The plan gate is bounded
  (one review pass plus at most one revise round); code review is delegated to
  `rdm-review`, and a genuine AC/architecture ambiguity parks the phase as
  `blocked` rather than guessing.
- A phase can now be parked as `blocked` with a recorded escalation reason.
  `rdm phase update <phase> --status blocked --reason "<why>"` stores the reason
  in the phase's frontmatter (`blocked_reason`); `--clear-reason` removes it. The
  reason is shown by `rdm phase show` (human and `--format json`) and is
  preserved across a later resume — moving a phase back to `in-progress` no
  longer loses why it stalled. The MCP `rdm_phase_update` tool gains matching
  `reason` / `clear_reason` parameters.
- `rdm review blocked` lists every phase parked as `blocked` — the escalation
  queue awaiting a human decision — with its recorded reason, so decisions can be
  answered in a batch instead of interrupting a run mid-flight. `--format json`
  emits an array of `{identifier, project, title, reason}`; `--project` selects
  the project.
- Escalation protocol documentation (`docs/escalation-protocol.md`): the single
  shared definition of when an autonomous run interrupts a human versus parks a
  decision. It distinguishes routine findings (never escalate; handled by
  `rdm-review`) from decisions/blockers (escalate), tags each escalation with its
  stage (`plan` vs `code`), specifies the plan-revise and rework-retry budget
  triggers, and defines the auto-handle / park-as-blocked / raise-to-user
  decision rule.

### Changed

- The `rdm-do` skill now uses a **one-worktree-per-roadmap, work-in-place**
  model. A roadmap gets a single worktree (`roadmap/<slug>` branch) and every
  phase is implemented in place in it: the skill reads `rdm worktree current`,
  compares the current worktree's roadmap to the target, and **works in place**
  on a match (the common case for every phase after the first), creates/enters
  the roadmap worktree once from the main checkout on a miss, or switches on a
  mismatch (interactively asking, or automatically under `--auto`). Because entry
  happens at most once and the session never re-enters or nests, `EnterWorktree`
  is now a one-time convenience rather than a correctness dependency — non-Claude
  hosts (Pi, web, MCP) get a fully correct entry path via plain `cd`/launch. The
  MCP variant additionally renders the `rdm_worktree_current` tool. Tasks keep
  their existing per-task worktree.

### Fixed

- `rdm hook post-commit` / `post-merge` now always commit the `Done:`
  phase/task updates they apply, even when staging mode is enabled via
  `--stage`, `RDM_STAGE`, or `stage = true` in `rdm.toml`. Previously the hook
  inherited the resolved staging preference, so the update could be written to
  disk without a commit and silently lost (leaving the plan repo dirty and the
  `Done:` directive unapplied).

## [0.13.0] - 2026-06-19

### Added

- `rdm review pending` lists the `needs-review` phases and tasks that are in
  scope for the current source-repo branch — those whose source-repo SHA
  (stamped when the item entered `needs-review`) is reachable from the current
  HEAD, plus any unstamped/legacy items (which fail open). It is the single
  shared source of truth for the auto-review Stop hook and the `rdm-review`
  skill, so they never disagree about what to review. `--format json` emits an
  array of `{kind, identifier, project, title}`; `--project` selects the
  project. Available when built with the `git` feature.

- `rdm-estimate` agent skill (shipped by `rdm agent-config --skills`, in both
  CLI and MCP variants). Given a roadmap slug or a single phase, it reads each
  phase body, rates its difficulty (`trivial` | `easy` | `moderate` | `hard`)
  with a one-line justification, records that note in the phase body, and sets
  the difficulty via `rdm phase update` — the model tier is assigned
  automatically from the difficulty. Phases that already have a difficulty are
  skipped, so re-running is idempotent and never overwrites a human-set value;
  clear a phase's difficulty to re-estimate it.
- `rdm next --roadmap <slug>` prints the next actionable phase in a roadmap
  (text and JSON): the lowest-numbered phase that is `not-started` or
  `in-progress`, skipping phases under review, done, blocked, or won't-fix.
  Roadmap dependencies are honored — if a dependency roadmap is not yet
  complete, the command reports a distinct `blocked-on-dependencies` result
  listing the unmet slugs; when nothing is actionable it reports `nothing`. All
  three outcomes exit 0. `--roadmap` is required (scope is one roadmap at a
  time; there is no project-wide scan).
- Phases now carry optional `difficulty` (`trivial` | `easy` | `moderate` |
  `hard`) and `model` tier (`small` | `medium` | `large`) fields. Set them at
  creation with `rdm phase create --difficulty <d> --model <m>` or later with
  `rdm phase update --difficulty <d>` / `--model <m>` (and `--clear-difficulty`
  / `--clear-model` to remove them). Both are surfaced in `rdm phase show` and
  `rdm phase list` (text and JSON) and reported by `rdm describe phase`. These
  fields are foundational metadata for upcoming difficulty-aware model
  selection.
- The rdm MCP server now exposes worktree lifecycle tools — `rdm_worktree_add`,
  `rdm_worktree_list`, and `rdm_worktree_remove` — mirroring the `rdm worktree`
  CLI commands. They run against the project (code) repo discovered from the
  server's working directory (refusing to run inside the plan repo) and are
  available when the server is built with the `git` feature. With them, the MCP
  `rdm-do` skill now creates and works inside an isolated git worktree via the
  MCP tool (no Bash), matching the CLI skill's behavior.

- `rdm agent-config pi --hooks` ships the Pi auto-review extension to end-user
  projects. It writes `.pi/extensions/rdm-review.ts`, which Pi auto-discovers
  (no settings registration). The extension subscribes to Pi's `agent_end`
  lifecycle event and re-prompts the agent to run the `rdm-review` skill while
  any item is in `needs-review`; it calls `rdm` on `PATH` with standard project
  resolution (no hard-coded project). The flag is composable with `--skills` and
  honors `--out <dir>` (project `.pi/`) and `--user` (`~/.pi/agent/`).
- `rdm agent-config claude --hooks` ships the auto-review Stop hook to end-user
  projects. It writes a generalized `.claude/hooks/rdm-review-on-finalize.sh`
  (executable; calls `rdm` on `PATH` and uses standard project resolution
  instead of a hard-coded project) and registers it under `hooks.Stop` in
  `.claude/settings.json`, merging non-destructively into any existing settings
  (other keys preserved; re-running is idempotent). The flag is claude-only and
  composable with `--skills`; it honors `--out <dir>` and `--user` (`~/.claude/`).
- `rdm worktree` command family (`add` / `list` / `remove`) for managing git
  worktrees in your project (code) repo, keyed to plan items. `add <item>`
  creates (or idempotently reuses) a worktree and branch for a phase
  (`<roadmap>/<phase-stem-or-number>`) or task (`task/<slug>`); branches are
  named `phase/<roadmap>/<stem>` or `task/<slug>`, and worktrees live as
  siblings of the repo under `<repo>__worktrees/`. `--base <ref>` chooses the
  branch point (default current HEAD); `--format json` emits the item, branch,
  path, and created flag. `list` shows item/branch/path and a dirty flag.
  `remove <item|path>` deletes a worktree (refusing a dirty tree without
  `--force`), with `--delete-branch` to drop the branch too (refusing unmerged
  commits without `--force`). Commands run against the repo discovered from the
  current directory and refuse to run inside the plan repo. Only rdm-created
  worktrees (tracked via an internal marker) are listed or removable.
- New `rdm-tui` crate: a terminal UI binary (`rdm-tui`) that opens an
  interactive screen listing the projects in your plan repo. It resolves the
  plan repo the same way the CLI does (`RDM_ROOT`, global config `root`, then
  the XDG data dir), shows a hint when no projects exist yet, and quits on `q`,
  `Esc`, or `Ctrl-C` while always restoring the terminal — even on a panic.
  This is the foundation for richer roadmap and task browsing in later
  releases; it is read-only and does not yet open roadmaps or tasks.
- The `rdm-tui` terminal UI now navigates: press `Enter` on a project to open
  its roadmap list (showing each roadmap's status, slug, title, priority, and
  `done/total` phase progress), and `Enter` on a roadmap to open its detail
  view (the roadmap body plus its phases with status badges). `Esc`/`h` go back
  one screen — restoring the previous cursor position — and `Esc`/`h` from the
  project list quits. Statuses are shown as text labels with ASCII symbols so
  they remain distinguishable without color.
- The `rdm-tui` terminal UI now opens a phase to a detail screen: press `Enter`
  on a phase to see a metadata block (status, completion date, short commit SHA,
  tags) above the phase body rendered as terminal markdown — headings, bold/
  italic/strikethrough, lists, code blocks, GFM tables, and block quotes are all
  visually distinct without color. Cycle between phases with `n`/`p` (or the
  arrow keys), and scroll long bodies with `j`/`k`, `PageUp`/`PageDown`, and
  `Ctrl-u`/`Ctrl-d`. `Esc`/`h` returns to the phase list with its cursor intact.
- The `rdm-tui` terminal UI now opens a per-project task list: press `t` from the
  project list to browse a project's tasks in columns (slug, title, status,
  priority, tags), and use `t`/`r` to switch between the task and roadmap lists.
  Quick-filter the list by cycling the status filter with `s`
  (`all`→`open`→`in-progress`→`done`→`wont-fix`) and toggling a tag-filter popup
  with `f` (`space` to toggle tags, `enter` to apply, `esc` to cancel). Press
  `Enter` on a task to open a scrollable detail screen with its metadata (status,
  priority, tags, created/completed dates, short commit SHA) above its markdown
  body; `Esc`/`h` returns to the list with the cursor and filters intact.
- `rdm_core::ops::task::filter_tasks` (and the `TaskFilter`/`task_matches`
  building blocks): a reusable task-filtering op over status, priority, and tags
  (AND), now shared by the CLI's `task list` and the TUI's task browser.
### Changed

- `rdm phase update --difficulty <d>` now auto-derives `--model` from the
  difficulty→tier mapping (`trivial`/`easy` → small, `moderate` → medium,
  `hard` → large) when `--model` is omitted and no model is already set. An
  explicit `--model` / `--clear-model` and any previously set model are
  respected — the derive only fills an empty model.
- Each plan-repo mutation (create/update/delete/promote/archive/split of a
  roadmap, phase, task, or project) now records a **single** git commit that
  bundles the entity change with the regenerated `INDEX.md`, instead of two
  separate commits. `INDEX.md` regeneration is now an inherent part of every
  mutation, so the index can no longer drift out of date. `--no-index` still
  skips regeneration (committing the entity change alone) as before.
- Roadmap aggregate-status computation (the overall `not-started` /
  `in-progress` / `done` derived from a roadmap's phases) now lives in
  `rdm-core` so every interface shares one implementation. No behavior change
  to the server or web UI.
- `rdm-do` (the in-repo Claude Code skill plus the shipped CLI and MCP skill
  templates) now does its work in an isolated git worktree created after marking
  the item in-progress instead of the live checkout. The CLI variants use
  `rdm worktree add <item>`; the MCP variant drives the equivalent
  `rdm_worktree_add` MCP tool (it no longer works in the live checkout). All
  three variants (dogfood, CLI, MCP) also gain two run modes: interactive
  (default — plan, approval gate, review-with-user) and `--auto` non-interactive
  (skips the approval and review gates and finalizes autonomously). For
  unattended Claude Code runs, launch with `--permission-mode auto` (or
  `bypassPermissions` in a sandbox) so file edits and bash/tool calls don't
  block on prompts. The finalize contract is unchanged (commit on the branch,
  set `needs-review`); the branch is left for merge to main.
- The MCP server and REST API no longer emit CLI-specific navigation hints
  (`rdm phase show …`) in `roadmap show` / `phase show` output. These
  `Hint:` / `Prev:` / `Next:` lines now live only in the `rdm` CLI, where the
  output is unchanged. Markdown table separators (`--format markdown` and
  generated `INDEX.md`) now render with fixed-width `---` / `---:` cells; column
  alignment is unchanged.

### Fixed

- The auto-review Stop hook (and the `rdm-review` skill) no longer misfire
  across worktrees: an item finalized to `needs-review` on one branch no longer
  reprompts a session finishing an unrelated branch, where that item's diff
  isn't even checked out. The `needs-review` transition now stamps the
  source-repo HEAD SHA, and `rdm review pending` scopes the prompt to items
  reachable from the current HEAD (unstamped/legacy items still fail open).
- Marking a task `wont-fix` now stamps a `completed` date (and records the
  optional commit SHA), matching the behavior of marking it `done`. Previously
  `wont-fix` tasks were left with no completion date.
- `rdm worktree add` run from inside a linked worktree now creates the new
  worktree as a sibling of the main repo instead of nesting it under the current
  worktree. Discovery resolves the repository's main working tree rather than the
  current working tree's top-level.
- Merge-conflict output now shows roadmap/phase context for conflicted phase
  files. Phase conflicts were previously misclassified as generic ("Other")
  paths because the classifier expected a layout the tool never writes, so the
  conflict listing dropped their roadmap and phase names.

## [0.12.0] - 2026-06-12

### Added

- Dogfood Claude Code Stop hook (`.claude/hooks/rdm-review-on-finalize.sh`)
  that reprompts the agent to run the `rdm-review` skill while any rdm item is
  in `needs-review`. The status is the sentinel — there is no marker file — so
  once review moves the item out of `needs-review` the next stop is allowed,
  and `stop_hook_active` prevents reprompt loops. Wires `.claude/` in this repo
  only; shipping equivalent config from `rdm agent-config` is tracked
  separately.
- `needs-review` and `reviewed` statuses for both phases and tasks, accepted
  everywhere statuses are (CLI `--status` on `create`/`update`/`list`/`search`,
  the REST server status selects and request parsing, the MCP tools, the
  `rdm describe` schema, and the web UI status badges).
  `needs-review` means implementation is finalized and awaiting review;
  `reviewed` means review passed and the item is awaiting merge to main (where
  the existing `Done:` merge hook flips it to `done`). Both are non-terminal,
  so they never stamp a completion date. The intended review lifecycle
  (`in-progress → needs-review → reviewed → done`) is documented in the agent
  instructions; status transitions remain unconstrained.

### Changed

- Removed the unmaintained transitive dependency `proc-macro-error2`
  (RUSTSEC-2026-0173) by building CLI tables with tabled's `Builder` API
  instead of its derive feature (and bumping tabled to 0.21). Table output is
  unchanged.
- `rdm-review` (the in-repo Claude Code skill plus the shipped CLI and MCP
  skill templates) now categorizes findings by size and owns the status
  transition. Small findings (localized, low-risk) are fixed inline and amended
  into the implementation commit; large findings (new modules, cross-cutting)
  are filed as rdm tasks instead of being fixed inline. On a passing review the
  skill sets the item to `reviewed` and writes the `Done:` line (the merge hook
  later flips it to `done`); on substantial rework it returns the item to
  `in-progress` with no `Done:` line.
- `rdm agent-config --skills` now emits a single `rdm-do` skill (CLI and MCP)
  in place of the previous `rdm-implement` and `rdm-tasks` skills, so the
  shipped skill set drops from 5 to 4 (`rdm-roadmap`, `rdm-do`, `rdm-review`,
  `rdm-document`). The merged `rdm-do` skill handles both roadmap phases
  (`<roadmap-slug> [phase-number]`) and tasks (`--task <slug>`), and finalizes
  by transitioning the item to `needs-review` (no `Done:` line); `rdm-review`
  produces the `Done:` line on a passing review.
- The in-repo Claude Code skills `rdm-implement` and `rdm-tasks` are merged
  into a single `rdm-do` skill that handles both roadmap phases
  (`<roadmap-slug> [phase-number]`) and tasks (`--task <slug>`). Finalize no
  longer commits a `Done:` line straight to `done`; instead it commits the
  implementation and transitions the item to `needs-review`, leaving it for
  the `rdm-review` skill to produce the `Done:` line on a passing review.

### Fixed

- `rdm search --status <status>` without `--type` now matches both phases and
  tasks for statuses shared by both kinds (`in-progress`, `needs-review`,
  `reviewed`, `done`, `wont-fix`); previously it silently returned phases only.
  Applies to the CLI, the server's `?status=` filter, and the MCP `search`
  tool's `status` argument.

## [0.11.0] - 2026-06-05

### Added

- `rdm agent-config` now supports the `pi` coding agent platform. Writes to
  `.pi/AGENTS.md` for project-local config or `~/.pi/agent/AGENTS.md` with
  `--user`.
- `rdm agent-config --skills` now supports the `pi` platform, writing skill
  files to `.pi/skills/` (project) or `~/.pi/agent/skills/` (user).
- HTTP server now serves the project logo as an SVG favicon at `/favicon.ico`.
  The favicon is linked in the base template, displayed in browser tabs, and
  sent with a `Cache-Control: public, max-age=86400` header so browsers can
  cache it for a day.
- `--clear-body` flag on `phase update`, `task update`, and `roadmap update`.
  Use it to intentionally empty an existing body (it is mutually exclusive
  with `--body`).
- `clear_body` field on `PATCH /projects/:project/roadmaps/:roadmap`,
  `PATCH /projects/:project/roadmaps/:roadmap/phases/:phase`, and
  `PATCH /projects/:project/tasks/:task` (mirrors the new CLI flag).

### Changed

- `rdm agent-config claude --skills --out <dir>` now writes skill files under
  `<dir>/.claude/skills/rdm-*/SKILL.md` (treating `<dir>` as a project root).
  Previously the files were placed directly under `<dir>/rdm-*/SKILL.md`. The
  `--user` path is unchanged.
- Roadmap detail page now lays out its metadata (status, priority, last
  changed, dependencies, tags) in the same definition-list style as the
  task and phase pages, with badges for status and priority, instead of a
  loose run of inline paragraphs.
- `--body` is now authoritative on `phase`, `task`, and `roadmap`
  create/update: rdm no longer reads stdin when `--body` is provided,
  eliminating a fragile interaction with background and other
  non-interactive runners that could overwrite or mix bodies.
- `rdm agent-config pi --mcp` now exits with an actionable error
  pointing users to `--skills` (or omitting `--mcp` for the AGENTS.md
  integration). Pi has no native MCP support, so the previous behavior
  silently produced unusable output.

### Fixed

- `phase update`, `task update`, and `roadmap update` no longer silently
  overwrite a non-empty body with empty content. Passing an explicit
  `--body ""` against an existing body is now rejected with an actionable
  error; use `--clear-body` to confirm.

## [0.10.3] - 2026-05-15

### Fixed

- npm publish step now runs from inside the extracted package directory
  instead of passing the path as an argument, so npm no longer
  misinterprets the relative path as a GitHub `<owner>/<repo>`
  shorthand and fails with `Permission denied (publickey)`.

## [0.10.2] - 2026-05-15

### Fixed

- npm publish workflow now uses Node 24 instead of Node 22 to avoid the
  broken bundled npm in the 22.22.2 runner toolcache image (missing
  `promise-retry` module), which was causing the `npm install -g
  npm@latest` step to crash and prevent the npm package from being
  published on release.

## [0.10.1] - 2026-05-15

## [0.10.0] - 2026-05-15

### Added

- post-commit and post-merge hooks now write a diagnostic log to
  `<git_dir>/rdm-hook.log` on every invocation, recording entry, branch and
  directive decisions, per-directive apply outcomes, and any errors. The file
  is auto-truncated past 256 KB and lives inside `.git/`, so it is never
  committed. Re-run `rdm hook install --force` to pick up the matching shim
  update that also captures native git/binary errors into the same file.
### Added

- Prebuilt binaries for two additional target platforms —
  `x86_64-apple-darwin` (Intel macOS) and `aarch64-unknown-linux-gnu`
  (ARM64 Linux) — generated by cargo-dist on every release, alongside the
  existing `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu` binaries.
- npm install method: `npx -y @edpaget/rdm mcp` (or `npm install -g
  @edpaget/rdm`) downloads the matching prebuilt rdm binary from the
  GitHub Release on install. Generated via the cargo-dist npm installer
  and published from CI on each release tag via
  [npm trusted publishing (OIDC)](https://docs.npmjs.com/trusted-publishers/),
  so no long-lived `NPM_TOKEN` secret is needed. Covers macOS arm64/x64
  and Linux arm64/x64.
- MCP Registry metadata at `io.github.edpaget/rdm` — `server.json`
  committed at the repo root, and the npm publish workflow now injects
  `mcpName: io.github.edpaget/rdm` into every published `package.json`
  so the MCP Registry can verify ownership. README adds Claude Code
  (`claude mcp add rdm -- npx -y @edpaget/rdm mcp`) and Cursor install
  snippets.

## [0.9.0] - 2026-05-15

### Added

- Revision-scoped reads on the storage layer: `Store::head_sha` and
  `Store::fetch_body_at(path, sha)` let callers read a target's body at a
  specific git revision. Backed by `git show` in the git store and a
  snapshot map (synthetic `mem-N` SHAs) in the memory store. Surfaces
  typed errors for unknown SHAs, paths missing at a SHA, and backends with
  no notion of history.
- `--at <sha>` flag on `rdm roadmap show`, `rdm phase show`, and
  `rdm task show` reads the body as it was at the given git revision while
  keeping current metadata. The same capability is exposed as the `?at=<sha>`
  query parameter on the matching `GET /projects/...` detail routes (HTML,
  HAL+JSON, Problem+JSON 404 on unknown/missing-at-revision SHAs). Text and
  Markdown output prepend a `Revision: <sha>` line; HAL+JSON includes a
  `revision` field; HTML detail pages render an `aria-live` "Viewing
  revision …" badge near the title.
- A small embedded JavaScript client (`/static/edit.js`) wired into every
  rdm-server page that intercepts `<form data-rdm-edit>` submissions, PATCHes
  the resource as JSON, reloads on success, and surfaces server validation
  errors inline.
- Inline status editor on the phase- and task-detail HTML pages: a `<select>`
  next to the status badge submits a `PATCH` (via `/static/edit.js`) and
  reloads the page with the new status. Phases include `wont-fix`; tasks
  include all four statuses.
- Inline body editor on roadmap-, phase-, and task-detail HTML pages: a
  collapsible `<details>` block exposes the raw markdown in a `<textarea>`
  and PATCHes the resource (via `/static/edit.js`) on submit. Clearing the
  textarea and saving is supported and round-trips. The rendered HTML body
  remains the default read view.
- Inline tag editor on roadmap-, phase-, and task-detail HTML pages: a
  collapsible `<details>` block exposes the current tags as a
  comma-separated `<input>` with "Save tags" and "Clear tags" buttons;
  submits PATCH the resource (via `/static/edit.js`) and reload.
  Whitespace, empty entries, and duplicates are normalized client-side.
  The phase detail page now also shows tags in the read view (parity with
  task/roadmap).
- Phase status `wont-fix`, treated like `done` for roadmap completion.
  Agent-facing surface area (`Describe` schema, agent-config instruction
  templates, and the CLI's combined-status error message) now lists
  `wont-fix` as a valid phase status and documents the
  `not-started`/`in-progress` → `wont-fix` transitions.

### Changed

- Quick-filter chips render right-aligned on the breadcrumb row as a
  horizontal group with vertical separators on desktop, and stack below
  the breadcrumb on narrow screens. The same placement is used on the
  roadmap list, roadmap detail, and task list pages.
- HTML pages now load their stylesheet from `/static/styles.css` instead
  of an inline `<style>` block, making each rendered page significantly
  smaller and allowing the browser to cache the stylesheet across
  navigations.

### Fixed

- Roadmap, phase, and task detail pages now render GFM pipe tables (and
  strikethrough, task lists, and GitHub-style `[!NOTE]` callouts) as proper
  HTML instead of passing the source syntax through as literal text.

## [0.8.0] - 2026-04-27

### Added

- Tags on roadmaps and phases. `--tags <csv>` on `roadmap create`, `roadmap
  update`, `phase create`, and `phase update` sets/replaces tags. Tags appear
  in `roadmap show`, `phase show`, and JSON output. Promoting a task
  preserves its tags onto the seed phase.
- `rdm search --tag <name>` filters results to items carrying the given tag.
  The flag is repeatable (`--tag bug --tag ui`) and ANDs together — an item
  must carry every listed tag to match. Items with no tags are excluded by
  any non-empty tag filter. Combine with `--type`, `--status`, etc., or use
  `--tag` with an empty query (`rdm search "" --tag bug`) to list every
  item carrying the tag. JSON results include a `tags` field.
- HTTP server tag filtering for roadmaps and phases:
  `GET /projects/<p>/roadmaps?tag=<t>` and the new
  `GET /projects/<p>/roadmaps/<r>/phases?tag=<t>` endpoint return only items
  with the given tag. The roadmap detail page also honors `?tag=<t>` to
  filter the embedded phases section. JSON responses include a `tags` field
  on roadmap and phase summaries/details. `POST` and `PATCH` bodies for
  roadmaps and phases now accept `tags: [...]` (and `clear_tags: true` on
  PATCH) to set, replace, or clear tags.
- `[server.quick_filters]` in `rdm.toml` defines named tag presets that
  render as clickable chips on the roadmap, phase, and task list HTML
  pages. Each chip links to the same page with `?tag=<value>`; the active
  chip is highlighted and an "All" link clears the filter. Override per-run
  via `RDM_SERVER_QUICK_FILTERS="Bugs:bug,UI:ui"` (env) or
  `rdm serve --quick-filter Bugs:bug --quick-filter UI:ui` (repeatable CLI
  flag). CLI flags > env > toml; higher-precedence sources fully replace
  lower ones rather than merging.
- MCP server tag support. `rdm_roadmap_create` and `rdm_phase_create` now
  accept a `tags` array; `rdm_roadmap_update` and `rdm_phase_update`
  accept `tags` (replace) and `clear_tags: true` (remove all). The
  `rdm_roadmap_list` and `rdm_phase_list` tools accept an optional `tag`
  filter, and `rdm_search` accepts a `tags` array (AND semantics) matching
  the CLI `--tag` flag.

### Changed

- `rdm agent-config` instructions (CLI and MCP variants) now demonstrate
  tagging: `--tags`/`tags: [...]` on create/update, `--tag`/`tag: "..."`
  on list, and the `--tag`/`tags: [...]` filter on search. Includes a
  short tagging convention note (lowercase kebab-case; check existing
  tags before inventing one). The `rdm-tasks` and `rdm-roadmap` skills
  (and their embedded templates) inherit the same examples.

## [0.7.1] - 2026-04-24

### Fixed

- `rdm hook post-merge` / `post-commit` no longer panic when a commit message contains a line starting with a multi-byte UTF-8 character (e.g. an em dash). The `Done:` prefix check now operates on bytes instead of slicing the string at a non-char-boundary.

## [0.7.0] - 2026-04-20

### Added

- Claude Code web sandbox template under `templates/claude-code-web/`: a `SessionStart` hook script, a `.claude/settings.json` snippet, and a `devcontainer.json` fragment that together install rdm and bootstrap a plan repo on session start. Drop them into a source repo with `scripts/install-claude-code-web-template.sh <target>` (idempotent; prompts before overwriting differing files). Full setup in `docs/claude-code-web.md`.
- `rdm bootstrap --token <token>` (also `RDM_PLAN_REPO_TOKEN` env) injects an access token into HTTPS clone URLs for private plan repos. SSH URLs are cloned as-is with a warning; plain `http://` URLs with a token are rejected. The token is never echoed to stdout or stderr, including on clone failures.
- `rdm bootstrap doctor` subcommand diagnoses sandbox readiness: rdm on PATH, configured plan-repo root, plan-repo URL, token presence, and — for GitHub HTTPS URLs — token scopes via `GET /repos/:owner/:repo`. Exits non-zero on critical failures so CI can gate on it.
- `docs/claude-code-web.md` now has a "Credentials" section covering fine-grained PATs (minimum scopes) and SSH deploy keys.
- `scripts/verify-claude-code-web-loop.sh` — hermetic end-to-end regression harness for the Claude Code web sandbox loop. Uses temp dirs and bare clones in place of GitHub; confirms the template, bootstrap, and source-repo `Done:` → plan-repo phase update all work together. Exits non-zero on any regression.

## [0.6.2] - 2026-04-12
### Added

- `rdm bootstrap --plan-repo <url> [--path <dir>] [--branch <name>] [--init]` clones a plan repo into a target directory (defaulting to `$XDG_DATA_HOME/rdm/plan-repo`) and fast-forwards it on subsequent runs. Designed for Claude Code web session-start hooks and other sandbox bootstrap scripts that need an idempotent "get me a plan repo" command.
- `install.sh` at repo root: `curl -fsSL https://github.com/edpaget/rdm/releases/latest/download/install.sh | sh` downloads a prebuilt rdm binary for the current platform. Supports `--version <tag>` to pin a specific release and `--dir <path>` to override the install location. Wraps the cargo-dist shell installer, which handles OS/arch detection and sha256 verification.
- CI workflow `install-test.yml` exercises `install.sh` on `ubuntu-latest` and `macos-latest` whenever `install.sh` changes.
- CI workflow `attach-install-sh.yml` attaches `install.sh` to each GitHub Release so the stable `releases/latest/download/install.sh` URL always resolves to the tagged version of the wrapper.

### Changed

- Upgraded rmcp dependency from 0.16 to 1.4
- `GitStore::clone_remote` now takes an optional `branch: Option<&str>` argument to clone a specific branch via `git clone --branch`
- Releases now publish an `x86_64-unknown-linux-gnu` tarball and a cargo-dist `rdm-cli-installer.sh` alongside the existing `aarch64-apple-darwin` tarball and Homebrew formula.

## [0.6.1] - 2026-03-31

## [0.6.0] - 2026-03-31

### Added

- Roadmap priority support in REST API: list/detail responses include priority, create accepts optional priority, new PATCH endpoint for updating priority, `?sort=priority` and `?priority=<level>` query params on list
- Roadmap priority support in MCP tools: `rdm_roadmap_create` accepts optional priority, `rdm_roadmap_list` supports sort and priority filter, new `rdm_roadmap_update` tool for setting/clearing priority and body
- Roadmap priority badges in HTML views: list page shows a Priority column and detail page displays priority next to status

## [0.5.0] - 2026-03-26

### Added

- `rdm_create_project` MCP tool to create new projects from within MCP clients
- Search results are now capped by relevance score, filtering out low-quality matches
- Optional `priority` field on roadmaps (`low`, `medium`, `high`, `critical`) — reuses the existing priority model from tasks
- `rdm roadmap create --priority <level>` and `rdm roadmap update` command for setting/clearing priority via CLI
- `rdm roadmap list --sort priority` sorts roadmaps by priority descending; `--priority <level>` filters by priority level
- `rdm roadmap show` displays priority when set

### Fixed

- `default_branch` is now recognized as a valid config key for `rdm config get` and `rdm config set`
- MCP server logs errors when store construction silently falls back instead of swallowing them

## [0.4.0] - 2026-03-24
### Added

- `rdm agent-config --user` writes agent config to the user-level config directory (e.g. `~/.claude/`) instead of a project directory, enabling global agent integration

## [0.3.1] - 2026-03-21

### Added

- `rdm agent-config --mcp` now generates MCP-oriented agent instructions referencing MCP tool names instead of CLI commands
- `rdm agent-config --mcp --skills` generates MCP-aware Claude Code skills that use `mcp__rdm__*` tools in `allowed-tools`
- When `--mcp --out` is used, `.mcp.json` is written alongside the instructions or skills
- MCP agent instructions include a Searching section with `rdm_search` tool

### Changed

- `--mcp` flag is no longer mutually exclusive with `--skills`; it is now a modifier that switches output to MCP tool references
- Restructured README to lead with installation and quick start, added "Core Workflow: Plan, Implement, Done" section showcasing the plan-implement-done cycle, and moved reference material (architecture, REST API endpoints) to dedicated docs

## [0.3.0] - 2026-03-21

### Added

- `rdm hook post-commit` subcommand: parses `Done:` directives from HEAD on the default branch, enabling automatic phase/task completion for fast-forward merges
- `rdm hook install` now installs both `post-merge` and `post-commit` hooks
- `rdm hook uninstall` now removes both hooks
- `default_branch` config key in both repo (`rdm.toml`) and global config — sets the branch name used by the post-commit hook (defaults to `main`)
- `current_branch_at()` public function in `rdm-store-git` for querying the current branch name
- `rdm_init` MCP tool to initialize a plan repo from within an MCP client (e.g. Cursor); accepts an optional `default_project` parameter to create a project during init
- `auto_init` global config option — when `true`, the MCP server automatically initializes the plan repo on first tool call if not already set up
- Improved MCP error messages for uninitialized repos: errors now mention the `rdm_init` tool instead of the CLI `rdm init` command

## [0.2.0] - 2026-03-20

### Added

- `rdm init --remote <url>` to clone an existing shared plan repo instead of creating an empty one; sets `remote.default = "origin"` and validates the cloned repo has `rdm.toml`
- `GitStore::clone_remote(url, root)` static constructor for cloning remote git repositories
- `rdm init --default-project <name>` flag to set `default_project` in repo config and create the project directory
- `rdm init --default-format <fmt>` flag to set `default_format` in global config
- `rdm init` with `--stage` persists `stage = true` to repo config
- `rdm init` now creates parent directories recursively, creates the global config file, and prints a summary with paths, settings, and next steps
- `PlanRepo::init_with_config()` in rdm-core for initializing with a custom `Config`

- `rdm config get <key>` command to view a config value with its source (CLI flag, env var, repo config, global config, or default)
- `rdm config set <key> <value> [--global]` command to set config values in repo or global config with validation
- `rdm config list` command to display all known config keys with resolved values and sources
- `default_format` config key in both repo (`rdm.toml`) and global config — sets the default output format (human, json, table, markdown)
- Format resolution chain: `--format` flag > `RDM_FORMAT` env var > `default_format` in config > `human`
- `InvalidConfigValue` error variant in rdm-core with actionable error messages
- `ConfigSource` and `ResolvedValue<T>` types in rdm-core for tracking where config values come from
- Config validation: invalid `default_format` values are rejected at parse time with clear error messages

### Changed

- `--format` flag no longer defaults to `human` at the clap level; the default is now resolved through the config hierarchy, allowing `default_format` in config files to take effect

### Fixed

- `--root` and `RDM_ROOT` now expand `~` to the home directory and resolve `.`/`..` segments, fixing silent failures when paths are set in config files like `.mise.toml` where the shell doesn't perform tilde expansion

### Added

- `Done: task/<slug>` directive support in post-merge hook — tasks can now be marked done via commit messages, just like phases
- `commit` and `completed` fields on the Task model — automatically set when a task transitions to done
- `--commit` flag on `rdm task update` for manually associating a commit SHA with a task

- XDG-compliant default paths: `rdm` now works out of the box without `RDM_ROOT` by resolving a plan repo root from `~/.config/rdm/config.toml` (global config) or `$XDG_DATA_HOME/rdm` (default data dir)
- `GlobalConfig` struct in rdm-core for parsing global config files with `root`, `default_project`, `stage`, and `remote` fields
- Config merging: CLI flags > env vars > repo config (`rdm.toml`) > global config (`~/.config/rdm/config.toml`) for project, staging, and remote resolution
- `rdm-review` skill for independent post-implementation review with parallel AC compliance and code quality agents
- `skill_review()` generator function in `rdm-core::agent_config` for generating the review skill via `rdm agent-config --skills`
- `rdm-document` Claude Code skill for generating user documentation from completed roadmaps using phase descriptions and commit SHAs
- `rdm agent-config --skills` now generates the `rdm-document` skill alongside the existing three
- `Done:` commit message convention documented in generated agent configs, `rdm-implement` and `rdm-tasks` skills
- `rdm hook install` / `rdm hook uninstall` to manage the post-merge git hook in the plan repo
- `rdm hook post-merge` subcommand: parses `Done: roadmap/phase` directives from the HEAD commit and marks matching phases done with the commit SHA
- `update_phase` is now idempotent for Done→Done transitions: re-marking a done phase with a new commit SHA updates the SHA while preserving the completed date; omitting `--commit` is a safe no-op
- `HeadCommitInfo`, `head_commit_info()`, `git_dir()`, and `default_branch_name()` on `GitStore`
- `rdm_core::hook` module with `DoneDirective` and `parse_done_directives()` for parsing `Done:` directives from commit messages

### Removed

- `.githooks/post-merge` bash script (replaced by `rdm hook` subcommands)
- `--commit <sha>` flag on `rdm phase update` to associate a git commit SHA with phase completion (requires `--status done`)
- `commit` field in phase frontmatter, phase detail display, and JSON output

### Added

- Merge conflict detection during `rdm remote pull` with rdm-aware item context (roadmap, phase, task classification)
- `rdm conflicts` command to list unresolved merge conflicts with item context
- `rdm resolve <file>` command to mark conflicts resolved and auto-complete merge with INDEX.md regeneration
- `rdm discard --force` now aborts an in-progress merge before discarding changes
- `rdm status` shows merge-in-progress state with conflict count
- `MergeConflictResult`, `PullOutcome`, `ResolveResult` structs and `git_list_unmerged`, `git_is_merge_in_progress`, `git_merge_abort`, `git_resolve_conflict` methods on `GitStore`
- `MergeConflict`, `NoMergeInProgress`, `NotConflicted` error variants in rdm-core
- `classify_path` function and `ConflictItem`/`ConflictItemKind` types in new `rdm-core::conflict` module

### Changed

- `rdm remote pull` now attempts a real merge when branches have diverged instead of rejecting with `BranchesDiverged`; non-conflicting concurrent edits merge cleanly

- Top-level `INDEX.md` now shows a lightweight summary table linking to each project's `INDEX.md` instead of inlining all project details

### Added

- `rdm remote push [name]` command to push local commits to a remote (supports `--force`)
- `rdm remote pull [name]` command to fetch and fast-forward merge from a remote, with automatic INDEX.md regeneration
- `PushResult`, `PullResult` structs and `git_push`/`git_pull` methods on `GitStore`
- `PushRejected` and `BranchesDiverged` error variants with actionable messages
- `rdm remote add <name> <url>` command to register a git remote on the plan repo
- `rdm remote remove <name>` command to remove a git remote
- `rdm remote list` command to display all configured remotes with their URLs
- `rdm remote fetch [name]` command to fetch from a git remote (defaults to `remote.default` in `rdm.toml`)
- `rdm status --fetch` flag to fetch from the default remote before showing sync status
- Sync status display on `rdm status` showing ahead/behind commit counts relative to the default remote's tracking branch
- `SyncStatus` struct and `git_fetch`/`git_sync_status` methods on `GitStore` for programmatic fetch and ahead/behind detection
- `RemoteInfo` struct and `git_remote_add/remove/list` methods on `GitStore` for programmatic remote management
- `RemoteConfig` struct in `rdm-core::config` with `[remote]` section support in `rdm.toml`
- `RemoteNotFound` and `DuplicateRemote` error variants in rdm-core
- `format_top_level_index` function in `rdm-core::display` for the new summary-style root index
- Per-project `INDEX.md` files at `projects/<name>/INDEX.md` with relative links, generated alongside the root index
- `format_project_index` function in `rdm-core::display` for standalone per-project index rendering
- `PlanRepo::generate_project_index` method and `project_index_path` path builder in `rdm-core`
- Web UI hides completed roadmaps by default; toggle link (`?show_completed=true`) reveals them
- `rdm tree` command — hierarchical overview of a project's roadmaps, phases, and tasks with statuses (human, JSON, and Markdown formats)
- `rdm-core::tree` module with `TreeNode` types, `build_tree()`, and formatting functions
- Navigation hints in `roadmap show` output — shows how to drill into individual phases
- Prev/next phase navigation in `phase show` output — human and Markdown formats show commands for adjacent phases; JSON includes `prev_phase`/`next_phase` fields
- `rdm describe` command for model introspection — lists entity types or shows fields for a specific entity (project, roadmap, phase, task)
- `rdm-core::describe` module with `Describe` trait, `EntityInfo`/`FieldInfo` types, and formatting functions
- End-to-end agent workflow integration tests validating the full project → roadmap → phase → body discovery path, JSON parity, schema coverage, and programmatic navigation
- Drift tests that compare serde keys against `Describe` field names to catch struct/describe mismatches at compile time
- `project show` command with `--format human/json/markdown` support
- `--format json` support on all read commands: `roadmap list/show`, `phase list/show`, `task list/show`, `project list/show`, `search`, and top-level `list`
- `rdm-core::json` module with serializable JSON output structs (`RoadmapJson`, `PhaseJson`, `TaskJson`, `ProjectJson`, `SearchResultJson`, and summary variants) for stable machine-readable output

### Changed

- `roadmap show --format json` now nests phase summaries (without body) instead of full phase objects; use `phase show --format json` for full phase content
- `search --format json` now outputs via `SearchResultJson` types from the `json` module for a consistent contract
- `--mcp` flag on `rdm agent-config` to generate `.mcp.json` configuration for MCP-aware clients
- `generate_mcp_config` function in `rdm-core::agent_config` for programmatic MCP config generation
- End-to-end MCP workflow integration test covering the full agent lifecycle
- MCP Server section in README with tool table, config generation, and usage instructions
- `--format markdown` option for clean Markdown output on list, show, and search commands
- `--format table` option for pretty terminal tables on list and search commands (powered by `tabled` crate)
- Global `--format` flag on all read commands (defaults to `human`; `text` accepted as alias for backward compatibility)
- 6 mutation MCP tools: `rdm_roadmap_create`, `rdm_phase_create`, `rdm_phase_update`, `rdm_task_create`, `rdm_task_update`, `rdm_task_promote`
- 8 read-only MCP tools: `rdm_project_list`, `rdm_roadmap_list`, `rdm_roadmap_show`, `rdm_phase_list`, `rdm_phase_show`, `rdm_task_list`, `rdm_task_show`, `rdm_search`
- `rdm roadmap archive <slug>` command with `--force` flag to archive completed roadmaps
- `rdm roadmap list --archived` flag to show archived roadmaps
- `rdm roadmap unarchive <slug>` command to restore archived roadmaps to active status
- `RoadmapHasIncompletePhases` error variant in rdm-core for archive validation
- `rdm roadmap split <slug> --phases <stems-or-numbers>... --into <new-slug> --title "Title"` command to extract selected phases from an existing roadmap into a new one, with automatic renumbering and optional `--depends-on` flag
- `PlanRepo::split_roadmap` method in rdm-core for programmatic roadmap splitting
- `InvalidPhaseSelection` error variant in rdm-core for phase selection validation
- Dark mode support for the web UI with toggle button and system-preference detection
- Theme preference persists to `localStorage` across sessions
- Computed overall status badge (done / in-progress / not-started) on roadmap list and detail pages
- Last-changed timestamp on roadmap list and detail pages, derived from file modification times
- `--stage` global flag and `RDM_STAGE` env var for deferred git commits — files are written to disk but the git commit is skipped until explicitly requested
- `rdm status` command to show uncommitted changes in the plan repo
- `rdm commit -m "message"` command for explicit git commits (auto-generates message if `-m` is omitted)
- `rdm discard --force` command to reset working directory to HEAD state
- `stage` option in `rdm.toml` for persistent staging mode
- `staging_mode` on `GitStore` with `git_commit()`, `git_status()`, and `git_discard()` public methods
- `FileChange` enum and `FileStatus` struct in `rdm-store-git` for working directory status reporting
- Uncommitted changes hint on read-only commands (list, show, search) when staging mode is active
- `rdm-store-git` crate — git-backed Store with automatic commits via gitoxide; every `commit()` builds a tree from the working directory and creates a git commit with an auto-generated message
- `git` feature flag on `rdm-cli` (default-on) — enables `GitStore` for automatic git commits on all plan repo mutations
- `Error::Git(String)` variant in rdm-core for git-specific errors
- `rdm-store-fs` crate: filesystem-backed `Store` with in-memory staging — writes buffer in memory, `commit()` flushes to disk using write-to-temp + rename for best-effort atomicity, `discard()` drops the buffer
- `PlanRepo` mutation methods now auto-commit staged changes, so callers don't need explicit `commit()` calls
- `rdm mcp` subcommand: stdio MCP server (scaffold, no tools yet)
- `mcp` feature flag in rdm-cli (default-enabled)

### Changed

- Refactored all inline CSS colors in `base.html` to use CSS custom properties
- Bump `headers-accept` from 0.1 to 0.3
- Bump `mediatype` from 0.19 to 0.21

## [0.1.1] - 2026-03-18

### Added

- Homebrew tap (`edpaget/homebrew-rdm`) with auto-updated formula on release via cargo-dist
- `sign-release.yml` workflow: Sigstore cosign keyless signing of release artifacts with verification instructions appended to GitHub Release notes
- `prepare-release.yml` workflow: one-click version bump, changelog update, commit, tag, and push via `workflow_dispatch`
- cargo-dist configuration for automated binary releases (`rdm` binary for `aarch64-apple-darwin`)
- GitHub Actions release workflow (`.github/workflows/release.yml`) triggered by version tags
- `[profile.dist]` with thin LTO for optimized release builds

### Changed

- Workspace version centralized in root `Cargo.toml`; all crates now use `version.workspace = true`
- Rust version bumped from 1.87 to 1.94
- `repository` field added to workspace package metadata

### Changed

- `FsStore` moved from `rdm-core::store::FsStore` to `rdm_store_fs::FsStore`; import path updated in `rdm-cli` and `rdm-server`

- `rdm phase update` no longer requires `--status`; omitting it preserves the existing status, enabling content-only updates
- `PlanRepo::update_phase` now accepts `Option<PhaseStatus>` instead of `PhaseStatus`
- Server `PATCH /phases/:phase` endpoint accepts optional `status` field in request body

### Added

- `rdm roadmap delete <slug> --force` command to delete a roadmap and all its phases, with automatic cleanup of dependency references from other roadmaps
- `PlanRepo::delete_roadmap` method in rdm-core for programmatic roadmap deletion
- `rdm-implement` and `rdm-tasks` skills now use plan mode (`EnterPlanMode`/`ExitPlanMode`) for a deliberate plan-then-execute workflow with explicit user approval before finalizing
- Generated skills from `rdm agent-config --skills` include the same plan mode workflow
- `rdm roadmap depend <slug> --on <other>` to add a dependency between roadmaps
- `rdm roadmap undepend <slug> --on <other>` to remove a dependency
- `rdm roadmap deps` to display the dependency graph for all roadmaps in a project
- Circular dependency detection rejects cycles with a clear error message
- `CyclicDependency` error variant in rdm-core for dependency cycle detection
- `add_dependency`, `remove_dependency`, and `dependency_graph` methods on `PlanRepo`
- `format_dependency_graph` display function in rdm-core
- `--skills` flag on `rdm agent-config claude` to generate Claude Code skill files (`rdm-roadmap`, `rdm-implement`, `rdm-tasks`) as reusable slash commands
- `rdm-core::agent_config::SkillFile`, `SkillOptions`, and `generate_skills` public API for skill generation
- `rdm agent-config` command to generate AI agent instruction files for Claude Code, Cursor, GitHub Copilot, and AGENTS.md
- Supports `--project` to embed project name in examples and `--out` to write to platform-conventional file paths
- `rdm-core::agent_config` module with `Platform` enum, `AgentConfigOptions`, and `generate_agent_config` function
- "Planning workflow" section in agent config output teaching agents when and how to use rdm commands
- "Status transitions" section documenting valid phase and task status transitions
- `--principles-file` flag on `rdm agent-config` to reference a project principles file in generated instructions
- CLAUDE.md "Searching the plan" subsection documenting `rdm search` usage for AI agents
- `rdm search <query>` CLI command with fuzzy matching across roadmaps, phases, and tasks
- Search flags: `--type` (roadmap|phase|task), `--status`, `--project`, `--limit`, `--format` (text|json)
- Text output displays ranked table with type, title, identifier, and snippet columns
- JSON output (`--format json`) for agent/programmatic consumption
- `format_search_results()` display function in rdm-core for text table formatting
- `Serialize` derives on `SearchResult` and `ItemKind` for JSON serialization
- `search` module in rdm-core: fuzzy search across roadmaps, phases, and tasks by title and body content using `nucleo-matcher`
- `SearchFilter` for narrowing results by item kind, project, or status
- `SearchResult` with kind, identifier, project, title, snippet, and score
- `rdm serve` command with `--port`, `--bind`, and `--root` options
- Graceful shutdown on SIGINT/SIGTERM for `rdm serve` and `rdm-server` binary
- `server` feature flag on `rdm-cli` (enabled by default; disable with `--no-default-features`)
- Integration tests for all server endpoints using reqwest against real TCP server
- Accessibility smoke tests verifying WCAG landmark structure, heading hierarchy, and ARIA attributes
- POST endpoints for creating projects, roadmaps, phases, and tasks (201 Created + Location header)
- PATCH endpoints for updating phase status and task fields (status, priority, tags, body)
- POST endpoint for promoting tasks to roadmaps (`/projects/{project}/tasks/{task}/promote`)
- Automatic index regeneration after all write operations
- Content negotiation for write responses: HAL+JSON returns resource, HTML returns 303 See Other redirect
- 422 Unprocessable Content for invalid request bodies (RFC 9457 Problem Details format)
- `hal_created_response` and `see_other_response` helpers in `rdm-server::extract`
- `validation_error` and `json_rejection_response` helpers in `rdm-server::error`
- HTML rendering for all endpoints with content negotiation: browsers get accessible HTML pages, API clients get HAL+JSON
- WCAG 2.1 AA accessibility: skip-to-content link, breadcrumb navigation with `aria-label` and `aria-current`, proper `<th scope>`, status conveyed by text (not color alone), focus outlines, sufficient color contrast
- Markdown-to-HTML rendering for phase and task body content using pulldown-cmark (raw HTML stripped)
- Format-aware error pages: HTML requests get styled error pages, HAL+JSON requests get RFC 9457 Problem Details
- Askama compile-time templates for all pages: index, roadmaps, roadmap detail, phase detail, task list, task detail, error
- Read-only HAL+JSON endpoints: `GET /` (root with project links), `GET /projects`, `GET /projects/:project/roadmaps`, `GET /projects/:project/roadmaps/:roadmap` (with embedded phases), `GET /projects/:project/roadmaps/:roadmap/phases/:phase` (with prev/next sibling links), `GET /projects/:project/tasks` (with `?status=`, `?priority=`, `?tag=` filters), `GET /projects/:project/tasks/:task`
- `load_project()` method on `PlanRepo` for loading project documents
- HAL+JSON response helpers (`require_hal_json`, `hal_response`) in `rdm-server::extract`
- Server foundation: `rdm-server` binary with axum, health check endpoint (`GET /healthz`), and shared `AppState`
- HAL (Hypertext Application Language) response types in `rdm-core`: `HalLink` and `HalResource<T>` with builder API
- RFC 9457 Problem Details type in `rdm-core` with mappings from all `rdm-core::Error` variants
- Content negotiation extractor parsing `Accept` header for `application/hal+json` and `text/html` (defaults to HTML)
- `AppError` wrapper in `rdm-server` converting core errors to Problem Details HTTP responses
- `phase remove` command to delete a phase from a roadmap (accepts stem or number)
- Interactive `$EDITOR` fallback when no `--body` or stdin is provided (checks `$VISUAL`, then `$EDITOR`, then `vi`)
- `--no-edit` flag on all `create` and `update` commands to suppress interactive editor
- `--body` flag on all `create` and `update` commands for roadmaps, phases, and tasks
- Piped stdin support: body content can be provided via stdin (e.g., `cat notes.md | rdm task create ...`)
- `rdm roadmap show` now displays document body content after the phase table
- `--no-body` flag on `roadmap show`, `phase show`, and `task show` to suppress body output
- `RDM_PROJECT` environment variable for session-level default project (resolution order: `--project` flag > `RDM_PROJECT` env var > `default_project` in `rdm.toml`)
- Body parameter on core create and update functions for roadmaps, phases, and tasks
- `rdm roadmap list --project P` command to list all roadmaps with phase progress
- `rdm index` command to generate `INDEX.md` from current repo state
- `PlanRepo::generate_index` in rdm-core for full index generation (projects, roadmaps with progress, tasks sorted by priority)
- `format_index` display function with `ProjectIndex` and `RoadmapIndexEntry` structs
- `--no-index` global flag to suppress automatic INDEX.md regeneration after mutations
- Auto-regenerate INDEX.md after all mutation commands (project/roadmap/phase/task create, phase/task update, promote)
- `Ord`/`PartialOrd` derive on `Priority` enum (Low < Medium < High < Critical)
- Integration tests for index generation, idempotency, sorting, dependency graphs, auto-index, and `--no-index`

### Removed

- `require_hal_json()` guard — all endpoints now support both HTML and HAL+JSON via content negotiation

### Changed

- `load_roadmap` and `load_task` now return `RoadmapNotFound`/`TaskNotFound` (404) instead of `Io` error (500) when the resource file does not exist
- `task list --status` now uses `TaskStatusFilter` enum for proper clap validation instead of raw string
- `promote` preserves task metadata (priority, created date, tags) in the roadmap body
- `list_tasks` returns `ProjectNotFound` for nonexistent projects instead of an empty list

### Added

- `TaskStatusFilter` type with `Display`/`FromStr` for type-safe status filtering (accepts `all` or any `TaskStatus`)
- `rdm task create`, `rdm task show`, `rdm task update`, and `rdm task list` CLI commands
- `rdm promote` command to convert a task into a roadmap with an initial phase
- `task list` defaults to showing `open` + `in-progress` tasks; `--status all` shows everything
- `task list` supports `--status`, `--priority`, and `--tag` filters
- `PlanRepo::create_task`, `list_tasks`, `update_task`, `promote_task` in rdm-core
- `Display` and `FromStr` impls for `TaskStatus` and `Priority` (enables CLI arg parsing via clap)
- `format_task_detail` and `format_task_list` display functions in rdm-core
- `TaskNotFound` error variant in rdm-core
- Integration tests for all task CLI commands and promote
- `rdm phase list` command to show phases in a roadmap with number, title, status, and stem
- Phase commands (`phase show`, `phase update`) accept phase number as alternative to stem
- `rdm project create` and `rdm project list` CLI commands
- `rdm roadmap create` and `rdm roadmap show` CLI commands
- `rdm phase create`, `rdm phase show`, and `rdm phase update` CLI commands
- `rdm list` command with `--project` and `--all` flags for roadmap progress summaries
- Project resolution: `--project` flag > `default_project` in `rdm.toml` > actionable error
- `PlanRepo::create_project`, `list_projects` for project management
- `PlanRepo::create_roadmap`, `list_roadmaps` for roadmap management
- `PlanRepo::create_phase`, `list_phases`, `update_phase` for phase management
- Auto-numbering for phases (next available number) with explicit `--number` override
- Auto-set `completed` date when phase status transitions to `Done`; auto-clear on non-`Done`
- `Display` and `FromStr` impls for `PhaseStatus` (enables `--status` CLI arg via clap)
- `rdm-core::display` module with `format_roadmap_summary`, `format_phase_detail`, `format_roadmap_list`
- Error variants: `RoadmapNotFound`, `PhaseNotFound`, `DuplicateSlug`, `ProjectNotSpecified`
- Integration tests for all new CLI commands (`cli_project`, `cli_roadmap`, `cli_phase`, `cli_list`, `cli_project_resolution`)
- Cargo workspace with `rdm-core`, `rdm-cli`, and `rdm-server` crates
- Data model types: `PhaseStatus`, `TaskStatus`, `Priority`, `Phase`, `Task`, `Roadmap`, `Project`
- Markdown frontmatter parsing and rendering (`split_frontmatter`, `join_frontmatter`)
- Generic `Document<T>` wrapper with `parse()` and `render()` methods
- Plan repo configuration (`Config` struct, `rdm.toml` parsing)
- `PlanRepo` with path builders, load/write operations for roadmaps, phases, and tasks
- `PlanRepo::load_config` to read and parse `rdm.toml` from an opened repo
- `PlanRepo::init` to initialize a new plan repo with `rdm.toml`, `projects/`, and `INDEX.md`
- `rdm init` CLI command with `--root` flag and `RDM_ROOT` env var support
- Hand-written error types in `rdm-core` with `Display`/`Error` impls
- `rdm-server` stub binary

### Changed

- `create_project` now returns `Document<Project>` for consistency with other create methods
- `Config::to_toml` now returns `crate::error::Result` instead of leaking `toml::ser::Error`

### Fixed

- `Config::from_toml` now returns `crate::error::Error` instead of leaking `toml::de::Error`
- `rdm list` now propagates phase-loading errors instead of silently swallowing them
- CLI integration tests use `tempfile::TempDir` instead of `.tmp/` in project root
