# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Security

- **A configured external diff driver or `textconv` filter can no longer be executed by rdm, nor make a modified file read as untouched.** `diff.external` (or a `.gitattributes` `diff=<driver>` filter's `textconv` program) replaces git's own diff machinery, so the change-review diff read used to *run* the operator's script during an operation rdm documents as read-only — and, because such a script rarely prints git's unified format, the hunk set came back empty and `rdm review comment --path` refused a genuinely modified file as "not touched by `<base>..<head>`". The diff read now passes `--no-ext-diff --no-textconv`, and rdm's git subprocesses additionally scrub `GIT_EXTERNAL_DIFF` and `GIT_CONFIG_PARAMETERS` from the child environment — the two configuration layers a command-line flag cannot reach.

- **A `--path` whose filename contains a glob metacharacter no longer anchors a comment into a different file's hunks.** A diff pathspec is a glob by default, so `--path 'a?b.txt'` matched a modified sibling `axb.txt` and returned *its* hunks: the comment was accepted against a file the reviewed range never touched, and the stored anchor's `start_line`/`end_line` and `rdm:src/` permalink described that foreign file. The pathspec is now literal, so such a path is correctly refused as untouched, and the content read and the diff read can no longer disagree about which file is under discussion.

- **The review-persist command ladder now creates its scratch JSON with `mktemp` and deletes it.** It previously redirected `rdm review start --format json` into a fixed basename suffixed with the shell's PID under `$TMPDIR` — a predictable path in a world-writable directory, written with `>`, which follows a symlink, so a symlink planted there beforehand turned the redirect into an arbitrary-file overwrite running as the agent's user. `mktemp` creates the file itself with `O_EXCL` and mode 600, and the ladder removes it once the review id has been read.

- Reject malformed stored change-review head/base identities before source access and protect Git revision argument boundaries, preventing option injection and unintended file writes during review reads. Document the public revision-resolution API's matchable error for unsafe revision inputs.

- The review-persist workflow no longer lets a review target containing shell metacharacters (`$(...)`, backticks) execute arbitrary commands when the agent runs the emitted `rdm review start`/`rdm commit` lines — the target is now shell-quoted like every other untrusted value the writer emits.

- Removed the unused, unguarded, unquoted `worktreeRef`-based checkout entry from the review-persist writer (`persistReviewCommands`); no caller ever supplied it, and it was missing the failure guard and shell-quoting the sibling `source` entry has. The only supported checkout-entry path for a persisted review is the existing guarded, quoted `opts.source` branch.

### Added

- **New reference document `docs/authoring-grammar.md` specifies the canonical grammar for roadmap and phase body documents, the inventory of sections appended by skills and workflows, and the acceptance-criteria rubric that makes ACs actionable for autonomous implementation.**

- **A phase or task's `reviewed` review is now scoped to its own commits, not every earlier phase's in a shared roadmap worktree.** `rdm phase update`/`task update` gain an explicit, write-once `--start-commit <sha>` flag that records the item's starting commit (`started_head`), independent of `--status` — it may be passed alone or combined with any status transition in the same call. `--start-commit` must be a full 40-lowercase-hex-character SHA that resolves to a commit in the item's registered worktree (or an explicit `--source <path>`, when given); a malformed or unresolvable value is refused before anything is written, naming the rejected value and, for the existence check, the repository it was checked against. A second `--start-commit` against an item that already has a recorded value is refused, naming the existing value; the original is left untouched. `rdm review source` and the gated `reviewed` write default their review `base` to the recorded `started_head` instead of the merge-base with the default branch. With no `started_head` recorded — or with one that is no longer an ancestor of HEAD, e.g. after the roadmap or task branch was rebased or an earlier commit amended — `base` falls back to the merge-base, and the response's `baseNote` field names why. An explicit `--base` still overrides. The `rdm-dispatch-phase` skill records the start commit from its already-pinned head immediately before an item's first implementer dispatch, skipping when one is already recorded.
- Codex skill installation now includes a self-contained explicit runtime at `.agents/rdm-runtime` (also supported by `--user`), with the canonical review, plan-review and estimate dependencies. Automated skills remain disabled. Rust/nextest now owns the migrated review, model, estimate and spike regressions, plus process/session and foreign-installation coverage; no `verify-*.sh` runner is required for these checks.

- **`rdm agent-config claude --skills` and `--plugin` now ship all five Workflow engines, not one.** `rdm-wf-plan-review`, `rdm-wf-estimate`, `rdm-wf-backlog` and `rdm-wf-document` were previously local to rdm's own repo because every rdm command they build or name in an agent's prompt hardcoded rdm's own development build path (`./target/debug/rdm`) and rdm's own project name — commands that would name a binary absent from a downstream tree. All four now take the rdm executable and the project as runtime arguments (`rdmBin`, defaulting to a plain `rdm` on `PATH`; `project`, optional, emitting no `--project` flag at all when omitted), the same contract `rdm-wf-review-refute-fix` and `rdm-wf-estimate` already used, with an invalid value refused before any agent runs. A downstream consumer can now invoke the plan-review, estimate, backlog and documentation passes from the installed plugin or the emitted skills tree. `--skills` emits 17 files (was 13) and `--plugin` 17 (was 13).

- **A code review's refuter now grades whether a surviving finding is within the approved plan's scope, and an out-of-scope finding no longer blocks the review.** When a code-mode review is associated with an approved implementation plan, the refuter that already fetches that plan for its grading is now also asked whether the defect is in behaviour the plan actually changed, and whether an adequate fix stays within what the plan changed. A finding graded `inScope: false` is never dropped — it still survives as a confirmed, real finding and is always recorded — but it can no longer make the outcome `rework`/`escalated`; it must be filed as future work instead of gating this dispatch. Findings with no associated plan, plan-mode reviews, and anything not graded for scope are unaffected. The persisted review comment now carries a seventh header line, `inScope`, alongside the existing six.

- `rdm-dispatch-phase` now resolves the planner and implementer model from the phase or task's own `model` tier (`rdm model resolve plan --tier <T>` / `implement --tier <T>`, or with no `--tier` for an untiered item or a task) instead of silently inheriting the driving session's model — so a `large`-tier phase's planner and implementer both actually run at the cost/capability the tier implies.

- **A single prose procedure now drives one phase or task end to end, and leaves the trail that explains the result.** `rdm-dispatch-phase` is no longer a shim over a fixed pipeline: it plans, has the plan reviewed, implements, verifies, code-reviews, triages every comment with a reasoned reply, and writes the terminal `reviewed` status through rdm's own gate — recording an approved `plan/<slug>`, the review that approved it, a `change/<sha>` review, and one `addressed` (with the fixing commit) or `wont-fix` resolution per comment. A human's review of an agent's change and the review tool's own are read and resolved identically: neither the plan-approval wait nor the triage loop ever asks who wrote the review. Both the resume check and the triage loop read only the reviews that belong to the item being driven — its own document, its plans, and the changes recorded against them — so a pending review on some other roadmap is never picked up, never "fixed" in the wrong worktree, and never blocks this item's terminal write. `rdm-do` (both modes) and `rdm-autopilot` now enter this one procedure instead of carrying their own dispatch wiring.

- **`rdm project update` configures a project's source code repository.** `--source-repo <locator>` records the repository that `rdm:src/` links and `change/<ref>` reviews resolve against, `--source-branch <branch>` sets the branch a link with no `@<rev>` falls back to, and `--clear-source` removes both. `rdm project show --format json` now reports the configured source. Until now nothing in the CLI could write it at all, so a project's permalink trail could only be made resolvable by hand-editing the plan repo.

- An explicit Codex runtime reviews real implementation plans and pinned code ranges through canonical review logic, and previews or applies roadmap estimates using isolated session-scoped plan commits. Host model bindings respect core tiers/review floors and a single guarded review-target snapshot; private durable evidence and conservative recovery stop on incomplete reviews or uncertain writes. `rdm model resolve --format json` exposes the resolved step, tier and model. Estimate completion verifies committed content and a settled session journal rather than trusting commit exit status alone. Conditional estimate updates reject concurrent document edits; cancellation stops direct subprocesses and prevents later writes or successful finalization. Reviewers may recover from ordinary nonzero shell lookups without masking interrupted commands or failed Codex turns. Automated skills and review-gate writes remain outside this runtime.

- An opt-in Codex orchestration spike exercises canonical review and estimation with read-only Codex sessions and isolated plan fixtures, with credential-free process-contract tests. Production Codex automation remains outside the supported manual lane.

- Codex handoffs verify rdm's automatically captured review SHA and branch against the implementation worktree; the documentation distinguishes the manual pending-review lane from the retired automatic stop-hook safety net.

- Codex agent configuration with `AGENTS.md` and four discoverable manual skills: roadmap authoring, implementation with independent-review handoff, document revision, and explicit landing. Repository dogfooding now includes a rebuild-before-use entrypoint and setup guidance; unported workflow skills are reported rather than emitted as working support.
- **Reviews can now target code, not just documents.** `rdm review start --on change/<sha>` (or `--on change/HEAD` from inside a source checkout) opens a review of a *change* in the project's source repository: the revision is rev-parsed to a full 40-character SHA and pinned, the base it is diffed against is recorded (`--base <rev>`, else the merge-base with the project's default branch — a missing merge-base is an error naming `--base`, never a silent fallback), and the branch the change was on is stamped so later drift is measured against it. Comments anchor into the source repo with `rdm review comment <id> --path src/lib.rs --quote "…"`: the quote is located in that file's content *at the reviewed head* (read from git's object database, so a dirty or differently line-ending-normalized checkout can't flip a match) and must overlap a hunk the change touches — a quote outside every touched hunk is refused with the nearest one named, and a path the change never touches gets its own distinct message. `rdm review show` then reports each anchor `resolved` (the tip still has every occurrence the quote had at the reviewed head, so code that merely moved still reads as resolved), `drifted` (the tip holds fewer of them — including the case where the anchored occurrence was edited away while an unrelated duplicate elsewhere in the file survived), or `unresolved` (the path is gone), and emits a head-pinned `rdm:src/<path>@<head>#L<start>-L<end>` permalink per anchored comment that `rdm link resolve` accepts — derived from the stored anchor alone, so it renders with no checkout present. A change review records the implementation plan it implements, either explicitly via `--implements rdm:plan/<slug>` or inferred from the worktree's item when it has exactly one `approved` plan (zero or several is an actionable error naming the candidates); `rdm plan show` lists those reviews under a `change_reviews` list kept distinct from reviews *on* the plan, and `rdm backlinks phase/<roadmap>/<stem>` reaches them through the plan. `rdm review list --on change/<sha>`, `rdm review requests`, and `rdm search --type review` all treat the new kind like any other. Two deliberate boundaries: the plan repo's review files embed **nothing** from the source repository beyond the quote the reviewer chose (a `file-quote` anchor stores no surrounding context — duplicate quotes are disambiguated with `--occurrence` and the recorded line range), and the source repository is **never written to** — every interaction goes through a read-only port that spawns only `git rev-parse`/`merge-base`/`show`/`diff`. When no source checkout is reachable the read path degrades: `rdm review show` still prints the review, reports its change anchors unresolved, and says why in a `source_verification_skipped` note — while `rdm review comment --path` fails loudly rather than storing an anchor nothing checked. One consequence outside the review surface: `rdm roadmap create change` is now rejected as a reserved slug (alongside the existing `task`, `plan`, and `src` reservations) with an actionable error, since `change/<sha>` now names a review target. Design record in `docs/change-reviews.md`.
- **rdm can now refuse to mark work `reviewed` unless the review records actually exist.** Turn it on per plan repo with `rdm config set gates.reviewed true` (repo-only; `RDM_REVIEWED_GATE` overrides it, and it ships **off**, so nothing changes until you opt in). With it on, `rdm phase update --status reviewed` and `rdm task update --status reviewed` refuse unless all three of these hold: an `approved` implementation plan implements the item; a `change/` review with verdict `approve` records `implements` pointing at that same plan; and the item's worktree, if `rdm worktree` knows one, is clean. The three are checked in that fixed order so the first failure you see is the most specific one you can act on, and each refusal names both the record that is missing and the exact command that creates it — a superseded plan, for instance, tells you to write a new plan rather than sending you chasing a change review on a dead one. Worktree checking is fail-closed: a worktree rdm cannot inspect is reported as unobservable, never assumed clean, and because worktrees are keyed per roadmap a sibling phase's uncommitted edit will block your phase — the refusal names the dirty paths so you can tell instantly that the dirt was not yours. The `gates.reviewed` / `RDM_REVIEWED_GATE` answer is resolved by one shared rule, so the CLI and the HTTP server can never disagree about whether the gate is on — an HTTP `PATCH` setting `status: reviewed` against an ungated item comes back `409 Conflict` carrying the same actionable refusal the CLI prints. Only surfaces that accept a caller-supplied status are gated (the CLI and the HTTP server); the `rdm review restamp` path, the `Done:` post-merge/post-commit hook path, and rdm's own task consolidate/merge writes are deliberately left ungated and can never write `reviewed`. In a build with rdm-cli's optional `git` feature disabled there is no worktree to resolve, so the worktree precondition is simply never applicable — preconditions (a) and (b) still enforce, and the `phase update` / `task update` surfaces are identical in both builds. Design record, including the ungated allowlist, in `docs/core-enforced-gates.md`.

- `--override-gate "<reason>"` on `rdm phase update` and `rdm task update`: an operator's audited bypass of the `reviewed` gate. It waives the plan and change-review checks **only** — a dirty worktree still refuses, and the refusal says so — and it requires `--status reviewed`, so passing it on any other transition is rejected rather than silently ignored. It equally requires the gate to actually be enforcing: passing it while `gates.reviewed` is off is refused too, naming both remedies (drop the flag, or turn the gate on), because a bypass rdm cannot record is not a bypass it will pretend to honor. An empty reason is refused. The reason, who did it (resolved the same way a review's author is) and the date are recorded in a `gate_override` block on the item, shown by `rdm phase show` / `rdm task show` in both text and JSON, and cleared automatically the moment the item leaves `reviewed` — so a stale override can never authorize a second `reviewed` write. It is operator-only for a human, with one narrow exception: the `rdm-dispatch-phase` orchestrator may also pass it, but only to override a stale-change-review refusal on a delta that changed no executable behavior since an already-approved review, naming the review id and both HEADs in the reason. The autopilot loop may still never emit it, which `scripts/verify-skill-autopilot.sh` asserts mechanically.

- New `rdm verify` command family over the existing `dispatch.verify` key. `rdm verify resolve` reports the configured verification command or the literal `unresolved` (always exit 0 — it is a query). `rdm verify run` executes it and reports `{resolved, command, exit, tail}`, with the last 4000 characters of merged stdout+stderr, running inside a plan item's worktree when you pass `--item <roadmap>/<phase>` / `--item task/<slug>` / `--item <roadmap>`. Exit codes: `2` when nothing is configured, the command's own exit code once it runs (so `rdm verify run && …` composes), and `1` when the command was killed by a signal — fail-closed, because an unrunnable verification is never a pass. A multi-line `dispatch.verify` value is refused with a pointer at putting the steps in a script. Both subcommands read the key through the same accessor `rdm config get dispatch.verify --raw` uses, so the two can never disagree. Discovering a command from CI config or `CLAUDE.md` is deliberately *not* part of this: it stays an agent step whose result is written into the plan document, never into config.

- `rdm phase show` / `rdm task show` (and `rdm describe`) surface the new `gate_override` field. An item that was never overridden serializes exactly as before — the key is omitted entirely — so existing tooling reading these payloads is unaffected.

- `rdm-wf-review-refute-fix`'s opt-in `persist` step now records a code review onto `change/HEAD` — the code itself — instead of onto the phase or task document. It enters the item's worktree first (so `change/HEAD` means the right thing), anchors each finding whose reported location names a real source file with `--path`, and keeps the prefixed item ref (`phase/<roadmap>/<stem>` / `task/<slug>`) as a named fallback that `review start` retries with if the change target is rejected, so a persist never aborts and loses the audit trail. An explicit `persist: { on: '<ref>' }` still overrides both.

- Both review workflows can now **record their findings as a real rdm review** instead of leaving them in an ephemeral run result. Pass `persist: { on: '<ref>' }` (or just `persist: true` to derive the target) to `rdm-wf-plan-review` or `rdm-wf-review-refute-fix` and the run opens a review on the item, writes one comment per surviving finding, submits it with a mapped verdict (`reviewed` → approve, `rework`/`escalated` → request-changes), and commits it — so an agent's review is the same artifact, in the same place, as a human's, readable with `rdm review show`. Findings now carry an optional verbatim `quote`, which becomes a real anchor on the comment (`rdm review show` reports it `resolved`); the refuter checks the quote against the reviewed text and clears a stale one, and a finding with no quote simply becomes a whole-document comment. Comment bodies start with a fixed `severity` / `confidence` / `refuted` / `unrefutedReason` / `dimension` / `finding-id` header so the finding metadata survives the round trip. Persisting is **opt-in and off by default**, so every existing caller is unaffected. Plan review's round number, repeat set, and round-3 escalation cap come from the prior reviews the caller supplies (`priorReviews`, sourced from `rdm review list`) whether or not the current pass itself persists — persisting only determines whether THIS pass adds to that history for a later one to read; the `## Plan Review Round N` note plan review renders is a human-readable log only and is never read back. A unit's result now distinguishes a caller who supplied `priorReviews: []` (looked, found none — genuinely round 1) from one who omitted `priorReviews` entirely (cannot know the round): the latter sets `roundUnknown: true` on the unit and adds a `[round unknown: …]` clause to its summary, so a review that cannot verify its own round number no longer silently reports round 1 as though it had.

- New `rdm plan` command family: `create` (with `--implements phase/<roadmap>/<stem-or-number>` or `--implements task/<slug>`, and an optional `--supersedes plan/<slug>`), `show`, `list` (filterable with `--implements` and `--status`), `update`, and `delete --force`. Every subcommand supports `--format json`, and `plan create --format json` prints the created plan so a script never needs a follow-up `show`. The body contract matches `task` exactly: `--body` is authoritative and stdin is read to EOF on `create` only, `update` never reads stdin at all, `--body ""` against a non-empty body is refused, and `--clear-body` is the explicit opt-in. There is no `--status` flag on `plan update` — a plan's status is derived from reviews and from a later plan's `--supersedes`. `rdm phase show` and `rdm task show` now list the plans implementing them (a `plans` array under `--format json`, omitted entirely when there are none, and a `Plans:` line in the human and markdown views), and `rdm plan show` lists the reviews targeting the plan. Deliberately not included: `rdm search --type plan` (plans are not indexed by search), `plan show --at <sha>` (a review on a plan already reads its historical body, so `rdm review show` covers that need), a `plan` entity in `rdm describe`, and any rdm-server plan routes, detail views, or review forms.

- `rdm-core` can now create, list, filter, update and delete plan documents, and derives a plan's status from reviews rather than keeping a second source of truth. `ops::plan` adds `create_plan`/`list_plans`/`filter_plans`/`plans_implementing`/`update_plan`/`set_plan_status`/`delete_plan`. Creation validates that `implements` names an existing phase or task (a roadmap or plan reference is rejected) and normalizes a numeric phase identifier — `phase/auth/2` — to the roadmap's canonical stem, so `plans_implementing` matches either form. Submitting a review on `plan/<slug>` with verdict `approve` marks the plan `approved`, `request-changes` marks it `changes-requested`, and `comment` leaves it alone; creating a plan that names an earlier one in `supersedes` marks that one `superseded`, flipping only the plan actually named. `superseded` is terminal, so a later approve on a stale plan never downgrades it, and a submit whose plan target has since been deleted still succeeds — the review is the durable record and a derived write that cannot land never fails it. Still `rdm-core`-only in this change — no CLI or server surface creates a plan yet.

- `plan/<slug>` is now a fourth reference kind, recognized everywhere the existing three are: `rdm review start --on plan/<slug>`, `rdm:plan/<slug>` links inside any body, and `rdm link resolve` / `rdm link check` / `rdm link list --on` / `rdm backlinks`. Plan documents live at `projects/<project>/plans/<slug>.md` and carry a required `implements` reference to exactly one phase or task, an optional `supersedes` reference to an earlier plan, and a `draft`/`approved`/`changes-requested`/`superseded` status — the on-disk format is documented in `docs/file-formats.md`. Two consequences are visible right away: `rdm roadmap create plan` is now rejected as a reserved slug (alongside the existing `task` and `src` reservations) with an actionable error, and `rdm review start --on plan/<slug>` now reports a missing plan target rather than an unknown link kind. **No plan document can be created yet** — the `rdm plan` command family arrives in a later change — and `rdm-server` gets compile-keeping stub handling only (a plan-targeted review links to the project page; there are no plan routes, detail views, or review forms).

- Foundation for a new `rdm:` link scheme in `rdm-core`: `rdm:roadmap/<slug>`, `rdm:phase/<roadmap-slug>/<stem>`, and `rdm:task/<slug>` reference plan items using the same syntax as `Done:` lines and `rdm review --on`, and `rdm:src/<path>[@<rev>][#Lstart[-Lend]]` references a location in a project's source repository. A new `source: { repo, default_branch }` block on project frontmatter records the repository `rdm:src/` links resolve against. `rdm roadmap create` now also rejects the reserved slug `src` (alongside the existing `task` reservation) with an actionable error. This phase adds only the data model and parsing — no CLI surface or link resolution yet.

- `rdm-core` can now resolve the `rdm:` links the previous phase parses, and find what references a given item. `resolve_link` (and its narrower `resolve_item_link`/`resolve_code_link`) turns a parsed link into `Resolved::Item { exists }` (a dangling reference is never an error — just `exists: false`) or `Resolved::Code { rev, web_url, .. }`, applying the rev precedence explicit `@rev` > the containing phase/task's stamped `commit` > the project's `source.default_branch` > a hardcoded `"main"`, and building GitHub-style `#L5`/`#L5-L12` web URLs. `backlinks` scans every roadmap, phase, task, and review body/comment in a project for references to a target, returning stably-ordered entries. Still core-only — no CLI or server surface exposes this yet.

- `rdm:` links are now reachable from the CLI. `rdm link check [--on <ref>]` validates every link in a project (or one document — a roadmap scope checks only the roadmap's own body, not its phases), reporting dangling item links, malformed `rdm:` destinations, and code links whose path is missing at its pinned revision as three distinct, separately-labeled categories; it exits nonzero when anything is broken, so it's CI/agent-finalize friendly. Path verification only engages when `cwd` is inside a checkout of the *project's own configured* `source.repo` (matched by canonicalized filesystem path, or by the checkout's `origin` remote for a URL-form source) — never against whichever unrelated git repository (e.g. the plan repo itself) happens to contain the invoking `cwd` — and a pinned revision this checkout cannot resolve (e.g. a shallow CI clone) is treated as inconclusive rather than reported as a deleted file; every skip reason (no checkout, a checkout that doesn't match the configured source, no configured source, or built without git support) is stated explicitly rather than silently omitted. `rdm link list --on <ref>` lists one document's outgoing links, resolved. `rdm link resolve <uri>` resolves a single `rdm:` URI given on the command line — `--format json` emits the flat shape (`kind`, `path`, `rev`, `line`, `end_line`, `web_url`, `exists`, as applicable) the neovim-plugin roadmap's editor integration is built against; a dangling item reference resolves successfully with `exists: false`, never as an error. `rdm backlinks <ref>` lists every document referencing a plan item. All four support `--format text|json` (table/markdown fall back to the text rendering).

- `rdm commit --changeset <id>` commits a *named* changeset instead of your own — the recovery route for work left behind by a session that has gone away. `rdm session list` shows which ones exist and flags the orphans. `rdm commit --all`, `rdm status --all`, and `rdm discard --force --all` are the matching whole-tree opt-ins for the three scoped commands below.

- `rdm-server` now surfaces the changeset its mutations belong to. It resolves one changeset at startup (from `--changeset <id>`, else `RDM_SESSION`, else the usual chain), attributes every write it makes to it, prints it at boot alongside the exact command that lands it, and returns it on every response as the `X-Rdm-Changeset` header. Its default remains staging-only — it never commits on your behalf — so its writes are attributable rather than silently stranded, and it says so on **every** mutation rather than only at boot: a warning on the server's log and the same text back to the client in an `X-Rdm-Staged` response header naming `rdm commit --changeset <id>`. Reads are not tagged. Pass `--autocommit` (or set `RDM_SERVER_AUTOCOMMIT=1`) if you want each mutation committed as it happens; an autocommit that fails, or that finds nothing to land, is reported as an error naming the same recovery command rather than failing your request — and when the reason it landed nothing (or landed only part of what it wrote) is that a file has since disappeared off disk, the log names that path too, so "landed nothing" is never mistaken for "there was nothing to land".

- New `rdm session` command group, and a per-session record of what each session changed. rdm now works out a **changeset identity** for whoever is running it — automatically, with nothing to configure — and writes down exactly which files each of your mutations touched. `rdm session id` prints that identity (`--format json` adds which rung of the resolution chain produced it and how long resolving took); `rdm session journal` prints the exact set of paths your session has written; `rdm session list` shows every changeset, flagging ones whose session is gone; `rdm session adopt <id>` lets you pick an abandoned changeset back up after a crash; `rdm session discard <id> --force` throws one away; and `rdm session gc` cleans up records for processes that have exited. Identity resolves from `RDM_SESSION` if you set it, otherwise from a lease inherited from the shell you are working in, otherwise from a harness variable such as `CLAUDE_CODE_SESSION_ID`, and failing all of those from the process itself — so it always resolves and never errors, including inside a git hook on its bounded timeout. **Commit behavior at the time this landed was unchanged** — it is no longer: `rdm commit`, `rdm status`, `rdm discard`, and the `Done:` hooks now all act on your changeset, described under *Changed* below. All of this state lives inside the repository's `.git` directory, so it never appears in `rdm status` and is never swept into a commit. Details in `docs/session-identity.md`; gated end to end by the new `scripts/verify-session-identity.sh`.

- Agents are now taught the `rdm:` linking vocabulary and use it. The generated `-cli` agent instructions (`rdm agent-config claude|agents-md|cursor|copilot|pi --skills`) gain a `## Linking` section covering the three item-link forms, the pinned `rdm:src/<path>[@<rev>][#Lstart[-Lend]]` code-link form, and the rule to run `rdm link check --on <ref>` before finalizing a body edit. The `rdm-do` skill now composes a `## Key code` list of pinned `rdm:src/` links for touched files at finalize and runs `rdm link check` before invoking the canonical review; `rdm-review`/`rdm-revise` cite finding and reply locations with pinned `rdm:src/` links instead of a bare `file:line`; `rdm-document` emits permalink-resolving `rdm:src/` links instead of bare commit SHAs; `rdm-roadmap` searches for and links related existing items in its drafted bodies. This repo's own dogfooding `CLAUDE.md` and hand-maintained `.claude/skills/` mirror the same guidance, and the shipped `plugins/rdm/` tree is regenerated to match.

- `rdm-server` now renders `rdm:` links in roadmap, phase, and task bodies as live browser navigation instead of leaving them as inert text. The rewrite happens at the pulldown-cmark event level (never by touching the markdown source), so review-anchor byte offsets and the select-to-anchor flow are unaffected, and it applies in every render mode — plain, the select-to-anchor annotated view, and the highlighted-review view. An `rdm:roadmap/…`/`rdm:phase/…`/`rdm:task/…` link to an existing item becomes an anchor to its detail page carrying a `rdm-link-item rdm-status-<status>` class (dimmed/struck-through for `done`/`wont-fix` phase and task targets); a dangling item target renders as a `<span class="rdm-link-broken" title="…">` — never a dead `<a>`. An `rdm:src/…` code link becomes a GitHub-style permalink (`target="_blank" rel="noopener"`) when the project has `source` configured, or a non-navigable `path[@rev]` span otherwise. Roadmap, phase, and task detail pages gain a "Referenced by" section listing every other document that links to them (omitted when there are none), and their JSON responses expose the same resolved outgoing links and backlinks as HAL `_links["rdm:link"]`/`_links["rdm:backlink"]` plus richer `_embedded["links"]`/`_embedded["backlinks"]` detail, in the same field vocabulary as `rdm link resolve --format json`. New styles for all of the above are in `rdm-server/assets/styles.css`. The same rewrite now also applies inside a review's own body: an `rdm:` link in a review comment, a comment's reply, or a review's summary text gets the identical item/code/broken treatment instead of rendering as a raw, non-navigable `rdm:`-scheme href.

- **Model tiers now resolve to a per-host model + reasoning-effort profile, with a fourth `frontier` tier.** `rdm model resolve <step> --format json` now also reports `host` and `effort`, and `rdm model resolve`/`rdm model show` take `--host claude|codex` (default `claude`); plain-text `rdm model resolve` still prints a bare model id. Profiles are configured per host and tier in `rdm.toml` as `[models.profiles.<claude|codex>.<small|medium|large|frontier>]` with optional `model` and `effort` (`low`, `medium`, `high`, `xhigh`, `max`; `codex` accepts up to `xhigh`); an unknown or host-unsupported effort is rejected when the config loads, naming the valid values. Built-in defaults: Claude `opus` at `low`/`medium`/`high`/`xhigh` for small/medium/large/frontier; Codex `gpt-6-sol` at `medium`/`high` and `gpt-6-astra` at `medium`/`xhigh`. The `medium` tier's built-in Claude model is now `opus` (was `sonnet`). `frontier` is accepted wherever a tier is (`--tier`, phase `--model`, `[models.steps]`), but no step or difficulty resolves to it unless configured. The `plan` step now resolves to at least the `medium` tier. Existing `[models] small/medium/large = "<id>"` keys keep working: they set the Claude model for that tier at the tier's default effort.

- The review Workflows (`rdm-wf-review-refute-fix`, `rdm-wf-plan-review`) accept `findEffort`/`verifyEffort` and run each finder and refuter at that reasoning effort; an invalid effort is rejected before any agent runs.

- `rdm-dispatch-phase` now resolves full model + effort profiles and runs the planner, implementer, finders and refuters at their resolved effort. The planner and implementer are dispatched through new `rdm-effort-<level>` agent definitions, installed by `rdm agent-config claude --skills` and shipped in the `rdm` plugin.

### Changed

- The shipped `rdm-dispatch-phase` skill now states up front that the orchestrator is authorized to invoke `--override-gate` for the narrow stale-review waiver case, instead of leading with "operator-only" and correcting it afterward — a framing that had caused a Claude Code auto-mode classifier to deny a legitimate, in-procedure override and force an unnecessary manual approval. The four conditions for the waiver are unchanged, and the audited `<reason>` string must now also cite the authorizing skill section (e.g. "per rdm-dispatch-phase § 'The terminal write' stale-review waiver"), not just the review id and both HEADs. Applies to `.claude/skills/rdm-dispatch-phase/SKILL.md`, the shipped CLI template, and the checked-in plugin tree at `plugins/rdm/skills/dispatch-phase/SKILL.md`.

- **`rdm agent-config claude`'s CLI usage guide gained the generic instructions rdm's own `CLAUDE.md` had accumulated only for itself.** `rdm info`, the `rdm link check|list|resolve`/`rdm backlinks` example commands, `change/<sha>` review-target mechanics, the `gates.reviewed` gate, extra `rdm search` examples plus its available-filters list, and the worktree-vs-main hazard / `depends-unlanded` guidance for a side-task filed from a shared worktree are now part of `rdm-core/src/templates/instructions-cli.md`, so every `rdm agent-config claude` (stdout, or `--out <dir>`, which writes `<dir>/CLAUDE.md`) and `--user` consumer gets them, not just this repo. rdm's own `CLAUDE.md` no longer restates the guide — it cites `rdm agent-config claude` instead — and its kept dev-only directives route through the `$RDM_BIN` convention instead of a hardcoded `./target/debug/rdm` path. **Operator action:** if you have already run `rdm agent-config claude --user`, re-run it (`rdm agent-config claude --user`, no `--project`/`--principles-file` needed) to refresh `~/.claude/CLAUDE.md` with this content — that file is hand-edited and outside any automated regeneration, so it will not pick up the change on its own.

- **Code review no longer blocks on a deferral a phase or task declared outside its criteria, in its own body.** The `ac` dimension's severity contract used to flag ANY criterion the target deferred, caveated, or shipped with acknowledged gaps as a blocking finding, with no distinction between a gap the author scoped out up front (per `docs/authoring-grammar.md`'s authoring rubric — e.g. an "Out of scope: ..." line in the Approach section, with the criterion's own wording already excluding the deferred part) and a caveat an implementer invents at the end of a diff to excuse incomplete work. Such a body-declared, criterion-excluded deferral is now honored as a bounded criterion, rated PASS as scoped. A caveat or exception written inside the criterion text itself (e.g. "supports all operators except regex, deferred to phase 2") is still graded unmet, whether or not the same deferral is also named elsewhere in the body — as is a deferral or caveat that first appears in the implementation diff or its commentary, with no antecedent in the body at all. This applies to code-mode review only; plan-mode review is unaffected.

- **The `rdm-roadmap` skill's phase-create example now writes `Context` / `Approach` / `Acceptance Criteria` sections instead of `Context` / `Steps`.** The old example told authors to write phase bodies as a numbered instruction list, the exact shape `docs/authoring-grammar.md`'s AC rubric and the always-on `restraint` plan dimension penalize, since implementation detail is already derived downstream by `rdm-do`'s plan step and the `rdm-dispatch-phase` skill's planner. The example's `## Approach` section is now described as the strategy or design principle guiding the work rather than a step checklist, and the skill points authors at `docs/authoring-grammar.md`'s AC rubric (observable outcome in positive form, a scope bound carried by the criterion itself, and any known deferral declared in `## Approach` rather than as an in-criterion caveat) for writing each criterion. Both the shipped template (`rdm-core/src/templates/skill-roadmap-cli.md`) and the dogfood copy (`.claude/skills/rdm-roadmap/SKILL.md`) carry the change, and the checked-in plugin copy (`plugins/rdm/skills/roadmap/SKILL.md`) was regenerated to match; the interview step and the `## Intent` grammar are unchanged.

- **The review-persist ladder now distinguishes a correct refusal of a finding on an untouched line, or about a file the diff never modifies at all, from a systemic anchor mismatch, and the dispatch skill parks on that distinction instead of on raw anchor-degradation volume.** A "you missed an edit here" finding necessarily quotes a line the diff did not change, and "you missed editing this file entirely" is an equally legitimate finding about a real, in-range file — both are the same benign refusal (`Error::QuoteOutsideChangedHunks`), and the ladder's existing mechanical retry lands either whole-document with nothing lost. But the ladder's `anchorsDegraded=all` line could not tell that lossless case apart from a systemic one (a path absent at the reviewed head, a quote absent from the file entirely, or an ambiguous quote), so the dispatch skill's park rule fired on both alike. The ladder now also prints `anchorsParkRequired=<yes|no>`, computed from two rules: a build-time or run-time anchor loss for a systemic cause always requires a park; a `blocking` finding losing its anchor always requires a park, even for a benign cause. `anchorsDegraded` keeps printing exactly as before and stays useful (the whole-document-fallback volume is still worth noting in a reply), but is no longer, by itself, a park signal — the dispatch skill (`.claude/skills/rdm-dispatch-phase/SKILL.md` and the shipped `rdm-core/src/templates/skill-dispatch-phase-cli.md`) now parks on `anchorsParkRequired=yes` instead. The headless `gate: true` path's guard is renamed to match: `RDM_PERSIST_ANCHORS_DEGRADED`/`= "all"` is now `RDM_PERSIST_ANCHORS_PARK_REQUIRED`/`= "yes"`.

- **The `rdm-dispatch-phase`, `rdm-autopilot`, and `rdm-land` skill templates are smaller, with no loss of guardrail coverage.** Accreted narrative prose in the shipped templates (`rdm-core/src/templates/skill-{dispatch-phase,autopilot,land}-cli.md`, emitted by `rdm agent-config claude --skills`/`--plugin`) and their dogfood copies (`.claude/skills/{rdm-dispatch-phase,rdm-autopilot,rdm-land}/SKILL.md`) — cases stated more than once, and rationale spelled out at paragraph length where a clause would do — was tightened without softening or dropping any rule. A downstream consumer running those commands now receives smaller skill bodies for the same behavior.

- **`rdm-land` now marks each landed item `done` explicitly instead of writing or amending a `Done:` commit trailer.** Landing used to synthesize a `Done:` line and `git commit --amend` it onto the branch tip before rebasing, so the existing post-commit hook would flip the item `reviewed → done` — but amending rewrites the tip commit's SHA, which the persisted review trail (`--applied-commit`, `rdm:src/…@<sha>` permalinks, gate-override reasons) pins by value, and the skill only ever handled one item even though a bare-roadmap land carries several reviewed phases on the shared branch. After the fast-forward, `rdm-land` now runs `rdm phase update`/`task update --status done --commit <sha> --no-edit` directly for every landed item — every phase that was `reviewed` on the branch, for a bare-roadmap land — recording the landed tip's commit for each. `.claude/skills/rdm-land/SKILL.md`, `rdm-core/src/templates/skill-land-cli.md` (and its regenerated `plugins/rdm/` and raw-skills-baseline copies), the `rdm-autopilot`/`rdm-dispatch-phase`/`rdm-do` skill copies that referenced the old mechanism, and `docs/landing.md` are all updated; the `Done:`-trailer hooks and parsers themselves are untouched (tracked separately as `task/retire-done-trailer-machinery`).

- **`rdm-autopilot` now stops the run, rather than continuing to the next phase, the moment any phase is parked `blocked`.** Previously a park (an `escalated` OUTCOME, an exhausted rework retry, a repeatedly failing advance write, or an unrecognized OUTCOME) left the phase parked and moved on to the next one, so on the shared per-roadmap worktree model a later phase could be implemented and reviewed on top of code already known to be defective. Autopilot's drive loop now stops the run immediately after parking, with a new known-good stop reason, `escalated`, naming the parked stem (rendered as `stop reason: escalated (<stem>)` in the run summary). Both the dogfood `.claude/skills/rdm-autopilot/SKILL.md` and the shipped `rdm-core/src/templates/skill-autopilot-cli.md` (and its regenerated `plugins/rdm/` and raw-skills-baseline artifacts) carry the new policy; the `rdm-dispatch-phase` per-item orchestrator's own park behavior is unchanged.
- Reconcile the Codex runtime with current shared contracts: estimate preview/apply consumes caller-supplied phase lists and typed results, preserves bodies/tags, and writes only unset difficulty with session-scoped commit verification. Report-only reviews forward caller-selected reviewers, parent intent, pinned phase source and approved-plan scope; intentionally omitted AC coverage remains distinct from failed selected coverage. Judgment processes receive the runtime's explicit binary/project/plan/session identity. The opt-in orchestration spike uses plan-file identifiers and the repaired estimate adapter.

- **Estimation records a difficulty and writes nothing to the item body.** Rating a phase used to append a `## Estimate <difficulty> — <justification>` audit note to that phase's body in the same update that set the difficulty — so an estimate pass, whose whole job is to set one enum field, also edited the document being estimated. The `rdm-estimate` skill and the `rdm-wf-estimate` engine now emit ONE command per phase, `rdm phase update <stem> --difficulty <d>`, and the justification is reported to the operator rather than persisted. Nothing in the estimate lane reads, carries or rewrites a body any more, and the model tier still derives in rdm-core from the difficulty.

- **There are now no mechanical agents in any rdm workflow — the estimate, document and backlog engines are the last three, and the agentType spike is deleted.** Each of them wrapped its reads and writes in LLM sub-agents, which is the least deterministic way to run a shell command and the boundary that lost a plan body, a structured path field, and an estimate tier in production. `rdm-wf-estimate` now dispatches only the difficulty **rater**: the caller runs `rdm phase list` and passes `phaseList`, and each rated phase comes back with `writebackCommands` / `writebackScript` — ONE `phase update --difficulty` command, runnable exactly as emitted, with nothing to substitute — for the caller to run. Estimation writes nothing to the phase body at all, and rdm-core derives the model tier from the difficulty. `rdm-wf-document` dispatches only the per-phase gatherers and the synthesizer: the caller runs `rdm roadmap show` and passes `roadmapMeta`, and the draft comes back with `writeCommands` / `writeScript` rather than being written by an agent. (The per-phase gatherer stays an agent, but is no longer classed as mechanical — it reads a commit and judges what the change did.) `rdm-wf-backlog` dispatches only the category analyzers: the caller runs `rdm backlog report --format json` and passes `report`, which does not weaken the propose-only contract because that command is read-only whoever runs it. All three refuse to run without their caller-supplied read and hand back the exact command to use. The three `model:mechanical` bootstrap agents are gone with them — there is no mechanical agent left for a mechanical model to pin. `spike-agent-type.js`, whose whole subject was `agentType` and `effort` on a mechanical call site, is deleted; its result stays recorded in `docs/workflow-schemas.md`. `docs/mechanical-agent-inventory.md` is the sweep record: the live grep, every call site it found, and each one's disposition.

- **The plan-review engine reads nothing and writes nothing either, and phase 32's plan-body transport is gone.** `rdm-wf-plan-review` used to run up to seven mechanical sub-agents around the actual review: `model:mechanical`, `fetch:roadmap` (+ a second `fetch:roadmap-body-check`), `fetch:phase`/`fetch:task`, `fetch:plan` (+ a second `fetch:plan-body-check`), `fetch:roadmap-intent`, `fetch:wontfix`, then `act:*`, `act:round-note:*` and `gate:clear-tag:*`. It now dispatches **finder and refuter agents only** — proven by driving the real driver over every target kind and looking at what it dispatched, not by counting literals. Each reviewer is told the `rdm … show --format json` command for the document it needs and runs it itself, so nothing is transcribed across an agent boundary and there is no second read whose job is to confirm the first. What the caller passes instead is what only the caller knows: which phase stems a `--roadmap` sweep covers (**with none, the roadmap document is reviewed alone — the engine never reads a roadmap to discover its phases**), the item's current tag list, its prior reviews, and the already-dismissed wont-fix titles. Every write comes back as data: `persistCommands`/`persistScript` for the review ladder, `gateAction.commands` for the `needs-plan-review` clear, and `roundNote` for the round audit block — the caller runs them and reports the exit status. A unit whose tag list was not supplied gets **no** gate commands and says so (`tagsUnknown`), because `--tags` replaces the whole list and guessing would silently drop a sibling tag. `planText` and the `planText`/`planSlug` precedence rule are gone: a plan is now named by an **identifier**, either `planSlug` for a persisted `plan/<slug>` document or `planFile` for a free-form plan on disk, and the reviewers read it themselves (`rdm plan show` or `cat`). The two are mutually exclusive, and an `--implementation-plan` target naming *neither* is refused before any reviewer runs rather than grading an empty document. So are `gateMode` (every run now returns the gate action, which was previously the opt-in escape hatch), the `fetched` payload hoist and every shape/fabrication guard that validated a transcription, and the three-id model bootstrap — `findModel` and `verifyModel` are independently optional caller arguments and an omitted one simply inherits the session model.

- **The code-review engine no longer reads or writes anything itself — it reviews, and hands the writes back as commands to run.** `rdm-wf-review-refute-fix` used to wrap five mechanical steps in LLM sub-agents: resolving the source checkout, re-resolving it, resolving the implementation plan, transcribing the item's acceptance criteria, and then persisting the review and writing the status. Every one of them moved a document or a command through an agent boundary, which is where two consecutive real reviews were lost to a single trailing newline. The engine now dispatches **finder and refuter agents only**. It takes the pinned identity (checkout path, base and head SHA, branch) as plain arguments, and each reviewer runs `rdm review source` — and, for acceptance criteria, `rdm phase show` / `rdm task show` — itself, reading the document once in its own context where it is never re-emitted. With `persist` or `gate` set it returns `persistCommands` / `gateCommands` (and a joined `persistScript` / `gateScript`): ready-to-run Bash that the caller executes and whose exit status it reports. The persist acknowledgement round-trip is gone with the agent that produced it, and with it the anchor-degradation accounting that existed only to check that self-report — an outcome is now classified once, before any write is described, and is never re-composed afterwards. What to do when a persist command fails is prose in the calling skill (re-run the one refused comment without its anchor, or park), not a gate.

- **The review pipeline's reviewers are now chosen by whoever invokes it, not inferred from the change.** Both review engines (`rdm-wf-review-refute-fix`, `rdm-wf-plan-review`) and the `rdm-review` / `rdm-plan-review` / `rdm-dispatch-phase` skills take a `reviewers` list naming exactly which reviewers to run; omit it and **every** reviewer for the mode runs, which is the safe default. The engines no longer inspect a diff's shape or a target's type to decide — the diff-shape inference, the per-reviewer trigger predicates, and the transcribed `## Intent` channel are all gone, and the `intent-alignment` reviewer now reads the parent roadmap's `## Intent` section itself. An unrecognised reviewer name is dropped silently and shows up as a gap in the run's reported coverage rather than as an error; nothing refuses a thin set, so an under-reviewed change is a visible choice instead of a blocked one. The one refusal kept is a set that resolves to no reviewer at all. Each reviewer's documentation now carries a cue for when to include it, rendered into every review skill from the same source the engines run.

- **The dispatch's plan review no longer transports the plan body through the orchestrator's own output, and a caller can no longer grade one plan document while persisting the verdict to another.** `rdm-wf-plan-review.js`'s implementation-plan mode used to require `planText` verbatim, which made `rdm-dispatch-phase` reproduce the entire plan document inside its own tool-call arguments — a 19.7 kB plan on the first real dispatch that exercised it. `planSlug` now resolves the body itself: one mechanical `plan show <slug> --format json` read (retried once), plus a second, independent mechanical read that reports only the fetched body's length and first line and must agree with what the first read transcribed — the same fabrication-resistant check `fetch:roadmap-body-check` already applies to a fetched roadmap body. A confirmed identity mismatch, an empty body, or a disagreement between the two reads fails closed to an `escalated` outcome (nothing graded, nothing persisted), never a graded verdict against unverified content. `planSlug` and `planText` are now **mutually exclusive** — supplying both throws, rather than grading the supplied `planText` while persisting to the `planSlug`-derived document (a divergence a corrupted or stale `planText` could previously exploit). `planText` remains the free-form path for a caller with no persisted document at all. *(Superseded within this same unreleased cycle: `planText` and BOTH mechanical `plan show` reads are gone — see the plan-engine entry above. A plan is now named by `planSlug` (persisted) or `planFile` (free-form, by absolute path) and read by the reviewers themselves; supplying both throws, and supplying neither is refused outright.)*

- **The dispatch's plan review now reviews the implementation plan instead of re-reviewing the phase document.** `rdm-dispatch-phase` used to invoke the plan-review engine with the roadmap/phase target, which grades the *item body* — so every finding it could produce was about phase prose that had already been plan-reviewed when the phase was created, while the plan that actually drives implementation went ungraded. It now invokes the engine as an implementation-plan review, naming the plan by `planSlug` (see above — the engine reads the body itself rather than being handed it verbatim), so the coherence, architectural-fit, restraint and intent-alignment dimensions grade the plan's own steps and acceptance criteria. An inaccuracy in a phase body that the plan does not inherit can no longer by itself force a revise round. The engine's implementation-plan mode gains two things to make that possible: a `planSlug` argument, which makes the verdict persist to that `plan/<slug>` document (a free-form plan pasted in with no slug still persists nothing, exactly as before), and acceptance of the parent roadmap's body, from which it extracts the recorded `## Intent` so intent-alignment runs — degrading silently to "no recorded intent" when none is supplied or found. Caller-supplied wont-fix titles now suppress already-dismissed findings on this path too. Standalone plan review over a roadmap, phase or task target is unchanged, including its `needs-plan-review` gate. **One consequence to know about:** because the dispatch no longer reviews the item document, it no longer clears the item's `needs-plan-review` tag — that tag asserts the *item* was plan-reviewed, and clearing it stays a job for the `rdm-plan-review` surface or a manual `rdm search "" --tag needs-plan-review` sweep. *(Superseded within this same unreleased cycle: the engine no longer accepts the roadmap body or extracts `## Intent` from it — the `intent-alignment` reviewer reads the parent roadmap itself. See the reviewer-selection entry above.)*

- **The plan-review engine accepts the parent roadmap's body from its caller, and rdm's own dispatch procedure now supplies everything it already holds.** `rdm-wf-plan-review.js` gains an optional `roadmapBody` argument — the parent roadmap's body verbatim — which replaces the subagent read that a standalone phase target used to make in order to inherit the roadmap's recorded `## Intent`. It is independent of the existing `fetched` argument: the two describe different documents and either may be supplied alone. Omitting it, or supplying a blank or malformed value, simply falls back to the subagent read, so the intent-alignment dimension still runs as it always did; a body that is present but records no `## Intent` yields no intent-alignment dimension, which is the same outcome a failed read has always produced. Neither case blocks the review. The in-repo `rdm-dispatch-phase` procedure now passes the roadmap body, the already-resolved wont-fix corpus and the three review-lane model ids it read in its identity-pin step, instead of letting the engine re-read them. (It also briefly passed the item's own body and tags as `fetched`; the implementation-plan change above — unreleased alongside this one — stopped the dispatch reviewing the item document at all, so it no longer supplies that argument. The `fetched` hoist itself is unchanged and remains available to every other caller.) The distributed skills and the plugin tree are unchanged — they ship no plan-review engine. *(Superseded within this same unreleased cycle: `roadmapBody`, the `fetched` hoist and the mechanical model bootstrap are all gone — the `intent-alignment` reviewer reads the parent roadmap itself, every reviewer fetches the document it needs, and `findModel`/`verifyModel` are two independently optional caller ids rather than an all-or-nothing trio of three. See the plan-engine entry above.)*

- **`rdm agent-config claude --skills` and `--plugin` now emit one Workflow engine instead of two.** The review engine (`rdm-wf-review-refute-fix.js`) is the only one distributed; the per-phase driver ships as the prose `rdm-dispatch-phase` skill, which the emitted skills enter directly. Re-emitting with **either** `--skills` or `--plugin` into a directory that already holds an orphaned `rdm-wf-dispatch-phase.js` from an earlier release **removes it**, reporting the removal — and, in both modes, leaves any file you wrote yourself untouched. A raw-skills emission now writes 13 files (11 skills + 1 engine + 1 agent definition) and the plugin tree 13 (manifest + 11 skills + 1 engine).

- **The `reviewed` gate is now enforcing for rdm's own plan data.** Marking one of rdm's phases or tasks `reviewed` requires what the gate has always asked for — an approved implementation plan, an approving change review at the observed HEAD, and a clean worktree — and a refusal is surfaced verbatim and parked rather than worked around. Other plan repos are unaffected: the gate still ships off by default and is enabled per repo.

- **`rdm-do` and `rdm-autopilot` no longer assemble a dispatch payload.** Both used to gather a phase's body, difficulty tier, five resolved model ids and verification command before handing them to a headless pipeline; the procedure they now enter reads what it needs itself, so that hoisting prose — and the class of silent failure where a partial payload was rejected and the work redone — is gone.

- **The worktree probe behind the `reviewed` gate reports its two caller-visible mismatches distinctly from a query failure.** A probe bound to one item and asked about another, and a registered checkout that has been switched off the branch it was registered on, used to surface through the same opaque git-error variant as "the repository cannot be queried", so a caller could not tell them apart. Each now has its own matchable error naming the two labels or branches and the remedy (`git switch <branch>`, or re-register the checkout), and the HTTP API returns them as 409 Conflict with a detail rather than an opaque error. The gate stays fail-closed: any probe failure still refuses the write, now with a cause that names the real condition.

- Which repository `rdm link check` and `rdm review --on change/…` read from, and which revision a change review's drift is measured against, are now decided by one shared rule in rdm-core instead of two hand-written ladders in the CLI. Both commands keep their existing behavior exactly: change review may still fall back to a configured local `source.repo` and still refuses with an actionable message when it cannot name a source, while `rdm link check` still skips path verification — with the same `path_verification_skipped` wording — in every environment where the checkout you are standing in is not provably the project's source. The two deliberately differ in three of the seven possible environments, and that difference is now documented and tested rather than accidental (see `docs/change-reviews.md` § "Source discovery"). Running from a linked git worktree of the configured source keeps working and still pins that worktree's own HEAD and branch, never the repository's main working tree.

- **`INDEX.md` and `projects/<p>/INDEX.md` are now ordinary files, with no special status anywhere in rdm.** rdm no longer has a notion of a "generated" or "derived" path at all. `rdm status` lists a dirty index as a plain change instead of on a separate `(N generated index file(s) will be included in the next commit: …)` line; `rdm commit` reports `Committed N file(s).` with no `(plus N regenerated index file(s))` suffix; `rdm discard --force` likewise counts every path it restored once, with no split. The post-command `(N uncommitted change(s))` hint counts them too, so a plan repo carrying a stale tracked `INDEX.md` left over from before this roadmap now sees it in that count and in `rdm status` — which is the honest answer for a file nothing maintains any more (a later phase adds an explicit prune). `rdm init` no longer seeds an `INDEX.md`, since nothing maintains it.

  A commit is also no longer rebuilt in memory: previously a scoped commit regenerated the indexes from HEAD-plus-your-changeset rather than taking them off disk, which bought a deliberate one-directional `tree ⊇ index` divergence — a document could land in the tree with no index row for it when its project belonged to another uncommitted session. That mechanism is gone, and the divergence **ceased to exist** rather than being maintained: a commit contains exactly the bytes some session wrote, so there is no reconstructed index for the tree to diverge from. A commit under a project another session has not landed yet still lands, as before, but now because nothing at commit time reads a project — not because an orphan-subtree prune defends against it. Nor does an ordinary mutation write the indexes any more, which takes the plan repo's central write-contention object off the write path — two sessions mutating different files no longer both write the same two derived files.

  One consequence you can hit: a regenerated index is no longer exempt from the overwrite guard. If you ran `rdm index` (now removed, see *Removed* below) and another session rewrote the same file before you committed, your `rdm commit` refused with the ordinary "changed by another session" error naming the path, instead of silently reconciling it away. This is the correct lost-update behavior for a file rdm no longer owns. The same applied to `rdm discard --force`: if another session had rewritten an index since you regenerated it, the discard left it alone and reported it as skipped rather than reverting their rows out from under them.

- **Breaking for direct `rdm-core` library consumers:** `rdm_core::paths::is_derived_path`, `index_path`, `project_index_path` and `is_project_manifest` are removed. The first three named a path class that no longer exists; `is_project_manifest` existed solely to serve the commit-time orphan-subtree prune, which is removed with it. `rdm_core::ops::index` built the two index paths inline; it is itself removed later in this same unreleased cycle, see *Removed* below.

- **Breaking for direct `rdm-store-git` library consumers:** `StatusReport` loses its `derived` bucket — three buckets now (`user` / `others` / `unattributed`) — and `is_clean()`, `is_changeset_clean()`, `total()`, `all()` and `changeset()` no longer account for it. `commit_summary()` and `discard_summary()` return a single unconditional count (`Committed N file(s).` / `Discarded N file(s).`), as does `ScopedDiscard::discard_summary()`. `ChangesetScope` loses its `derived` field, and `GitRepo::reconcile_derived` and the orphan-subtree prune behind it are gone (~230 loc), which also removes `rdm-store-git`'s only production dependency on `rdm_core::ops::index`.

- `rdm discard --force` now leaves a working tree that matches `HEAD` exactly: rdm authors no file of its own into the worktree, so a discard restores your paths and puts nothing back afterwards.

- **Breaking for direct `rdm-core` library consumers:** `rdm_core::ops::mutate` and `rdm_core::ops::mutate_batch` no longer take a `project` argument and no longer regenerate the index — the transaction is one entity write plus one flush, and index regeneration is no longer a failure source inside a mutation or a batch's `finalize_result`. *(Superseded within this same unreleased cycle: `rdm_core::ops::index::generate_index`/`generate_index_for_project`, briefly the only remaining way to regenerate an index, are themselves removed below — nothing regenerates one at all any more.)*
- **Breaking for `rdm backlinks --format json` consumers:** a backlink entry no longer always carries `range_start`/`range_end`. Those two keys describe where a `rdm:` link sits inside the referencing document's *body*, and `rdm backlinks` now also reports **structural** backlinks — references that live in a document's frontmatter rather than its prose, such as the plan that `implements` a phase, the plan that `supersedes` another, and the change review that implements a plan. A structural entry omits both range keys and instead carries `field` (the frontmatter field the reference lives in, e.g. `"implements"`) and, when it was reached transitively through an intermediate document, `via` (that document as an `rdm:` URI). Consumers that read `range_start`/`range_end` unconditionally must treat them as optional and branch on `field` instead. The text and markdown views show the same provenance as a trailing `[implements]` or `[implements via rdm:plan/<slug>]` suffix on the affected line; a plain body link still prints exactly as before. Direct `rdm-core` library consumers see the matching shape change: `BacklinkEntry.byte_range` is replaced by a `BacklinkEntry.reference: BacklinkRef` enum (`Body { byte_range }` | `Field { field, via }`) plus a `byte_range()` accessor returning `Option`.

- **Breaking for `rdm session list --format json` consumers:** The per-changeset `orphaned` boolean field has been removed and replaced by a `liveness` string field with values `"current"`, `"live"`, `"unleased"`, or `"orphaned"`. Scripts checking `orphaned` must migrate to checking `liveness !== "current" && liveness !== "live"` (or equivalent for the specific states they care about). `rdm session list` now distinguishes unleased changesets (no lease file, liveness unknown) from orphaned ones (dead lease, process gone or recycled) in both human and JSON output. Previously, both kinds read as "orphaned" from another session's vantage point, inviting a wrong `rdm session discard` on live work. Changesets with no lease file are now labeled "unleased", while the "orphaned" label is reserved for changesets whose lease exists but whose owning process is gone. The caller's own changeset remains unflagged in all cases.

- **Breaking for direct `rdm-store-git` library consumers:** `StatusReport` gains an `unattributed` bucket alongside `user`/`others` (at the time, also alongside a since-removed `derived`) — `is_clean()`, `total()`, and `all()` now account for it, and `pub(crate) GitRepo::git_status_report_scoped` takes a new `all_owned: &BTreeSet<String>` parameter (the union of every live-or-orphaned changeset's claimed paths) so it can tell a real other changeset's dirt from dirt no changeset claims. `ScopedCommit::unattributed_dirt()`'s name is unchanged but now also fires when only `unattributed` (not `others`) is non-empty — see its doc comment for the corrected meaning.

- `rdm commit` now explains itself when the caller's harness cannot give rdm a stable session. Running under an agent harness that starts a fresh shell for every tool call and publishes no session id, each `rdm` command resolves its own changeset — so a mutation made in one call is "attributed to another changeset" when the next call commits, and nothing lands without `--all`. That was previously silent. The command now exits 0 as before and adds a short block naming the cause (the harness publishes none of the session variables rdm looks for, and gave it no process outliving a single command) and the remedy (`export RDM_HARNESS_SESSION_ID=<stable per-session id>` once in the harness, or `RDM_SESSION=<id>` for a script or CI job). It appears only on the branches that are symptoms — an empty changeset over a tree that is not empty — so a successful commit, and a no-op commit against a clean tree, stay quiet, as does any caller that already has a session id. The same condition is also met, harmlessly, by the very first rdm command in a long-lived interactive shell, so the note states the diagnosis conditionally and names that benign reading rather than asserting a broken harness. `rdm session id --format json` gains a `lease_bootstrapped` field alongside `rung` for the same diagnosis; text output is unchanged.

- **`rdm commit`, `rdm status`, and `rdm discard` are now scoped to your own session.** Previously any of the three acted on the whole working tree, so two people (or two agent sessions) sharing one plan repo could not work at the same time without stepping on each other: your `rdm commit` would sweep up whatever the other session happened to have uncommitted, and your `rdm discard --force` would delete their added files outright. Now `rdm commit` lands only the paths *your* session wrote, `rdm status` shows only your changes (with a line counting what belongs to other sessions), and `rdm discard --force` restores only your paths and leaves everyone else's work — and their rows in the shared `INDEX.md` files — intact. The shared indexes are rebuilt for each commit from the last commit plus your own changes, so your commit's `INDEX.md` never names a file that is not in it, and never rewrites an index you did not touch. Pass `--all` to any of the three for the old whole-tree behavior; `rdm discard --all` additionally requires `--force` and names every other session's changeset before destroying it. If your session has nothing to commit but the tree is dirty anyway — files written outside rdm, or work done before this release — `rdm commit` now tells you which paths those are and how to recover them, instead of either saying "Nothing to commit." or quietly committing them. If a file your session wrote has since disappeared off disk — someone else's `rdm discard --force --all`, a manual `rm`, a crash mid-write — `rdm commit` skips it rather than failing, and now says so on **every** outcome, including the one that lands nothing at all: when the vanished file was the only thing your session had, the commit correctly becomes a no-op, and a bare "Nothing to commit." there would have read as "nothing needed doing" rather than "your work is gone". The path stays in your changeset either way, since the journal is trimmed only by a commit that actually lands. Details in `docs/session-identity.md`; gated by the new `scripts/verify-scoped-commit.sh`.

- The `Done:` hooks (`rdm hook post-merge` / `post-commit`) commit scoped too. This is the committer that fires with nobody deciding to commit — on every merge and every commit to the default branch, including `rdm-land`'s fast-forward — so it was the likeliest to sweep up a colleague's in-progress work. It now lands only its own changeset, and records which changeset and how many paths in the hook log.

- **Every surface that tells an agent how to commit now describes the scoped model.** The instructions `rdm agent-config` emits for Claude, Codex/AGENTS.md, Cursor, Copilot and Pi previously stated outright that "`rdm status`, `rdm commit`, and `rdm discard` operate on the whole plan repo's git state" and that a commit "lands every staged change" — both false since scoping shipped, and read by an agent *before* it ever calls a tool. The CLI instructions now describe the changeset your commit lands, what `rdm status`'s three buckets mean, `RDM_SESSION`, the `rdm session id` / `list` / `journal` observation surface, and the `--all` / `--changeset <id>` escape hatches. Batching guidance is corrected rather than dropped — batching a related group of mutations into one `rdm commit` is still right, and is now safe under concurrency. The `rdm-revise` skill's "commit per comment" rule keeps its conclusion with a corrected premise (it lands every change in *your own* changeset). The runtime hint printed after each mutation now reads `(staged in this session's changeset — run \`rdm commit\` to persist)`, so the first place an agent meets the concept no longer implies a whole-tree sweep. The shipped plugin tree at `plugins/rdm/` is regenerated to match.

- REST API documentation now covers the server's staging model: that mutations are staged and not committed by default, the `x-rdm-changeset` and `x-rdm-staged` response headers and when each is set, `--autocommit` / `RDM_SERVER_AUTOCOMMIT`, how to reconcile what a server staged, and the `409 Conflict` responses returned when a concurrent write would otherwise be lost.

- **Breaking for direct `rdm-store-git` library consumers:** `GitStore::commit_now` is replaced by two explicitly-named entry points — `commit_changeset` (the scoped default) and `commit_whole_tree` (the machine-global escape hatch) — and `GitRepo::git_commit` is now `pub(crate)`, so an out-of-crate caller that tries to sweep the tree fails to compile rather than drifting in silently. `StatusReport` gains an `others` bucket alongside `user`, filled by one partition rather than by a filter stacked on another; `is_clean()` still means "nothing at all differs" and a new `is_changeset_clean()` is the scoped gate. `rdm.toml` is written through the store via the new `rdm_core::io::save_config` rather than by a raw filesystem write, so `rdm init --remote` still lands its config commit.

- *(Superseded within this same unreleased cycle by the generated-path removal above, which collapses the split back into one count. Retained because the diagnosis below is why the split existed.)* `rdm status`, `rdm commit`, `rdm discard`, the post-command "uncommitted changes" hint, and the MCP `rdm_status` / `rdm_commit` / `rdm_discard` tools **counted only changes you authored**, and reported the regenerated `INDEX.md` files separately. Previously every one of them lumped derived output in with your own edits, so after any mutation a session could not tell what its next `rdm commit` would actually sweep up — a read-only `rdm list` would report "3 uncommitted change(s)" when you had edited one file. `rdm status` printed `1 file(s) changed` plus a line naming the generated files, and `rdm commit` / `rdm discard` reported a `(plus N … index file(s))` suffix. Nothing was hidden and nothing was dropped: the generated indexes were still written into every commit and still restored by every discard; only the *reporting* narrowed. What survives into the shipped release is the gating rule that came with it — `rdm commit` and `rdm discard` are gated on whether anything at all differs from HEAD, never on `user` alone — and the decision that a hand-customized `.gitattributes` is a real, committable file you may have edited, so it keeps showing up in `rdm status`.

- **Breaking for direct `rdm-store-git` library consumers:** `GitRepo::git_status` is replaced by `GitRepo::git_status_report`, which returns a `StatusReport` carrying the contract "gate on `is_clean()`/`total()`, report on `user`". The raw unfiltered walk is now `pub(crate) git_status_all` — deliberately unreachable from outside the crate, so no future call site can silently reacquire a list that conflates one session's dirt with another's. *(The report also briefly carried a `derived` bucket backed by `rdm_core::paths::is_derived_path`; both were removed later in this same unreleased cycle.)* `StatusReport` also owns the summary wording every interface prints — `commit_summary()` and `discard_summary()` — so `rdm commit` / `rdm discard` render the same report identically by construction rather than by convention.

- The built-in `small` model tier now resolves to `opus` instead of `haiku` when no `[models] small = …` override is configured — Haiku is no longer reachable anywhere in the default dispatch lane (`rdm model resolve plan|implement --tier small` now print `opus`). An explicit `small` override in `[models]` is unaffected.

- The Codex runtime takes both model and reasoning effort from `rdm model resolve --host codex` instead of its own `host.tiers` bindings and a fixed `medium` effort, and accepts the `frontier` tier. `host.tiers`/`host.steps` are now refused; configure `[models.profiles.codex.<tier>]` / `[models.steps]` instead. `host.capabilities` remains an optional guard.

### Deprecated

- `--no-index` is still accepted but has no effect, since mutations no longer regenerate an index. Passing it prints a one-line deprecation warning on stderr (never on stdout, so `--format json` output stays clean) and is otherwise ignored. It will be removed in a future release.

### Removed

- **The `rdm-wf-dispatch-phase` Workflow engine is gone.** The per-phase driver is now the prose `rdm-dispatch-phase` skill (see *Added*, above — the replacement landed in this same unreleased cycle), so the engine had no caller left. Removed: the engine script itself, its shared source module (`lib/dispatch-phase.mjs`), the shipped template, the copy in the checked-in plugin tree, and its registration in the emission table — plus two instruments that existed only to measure it (`scripts/verify-workflow-dispatch.sh` and `scripts/measure-hoist-delta.mjs`). The prose `rdm-dispatch-phase` skill and every other Workflow engine — review, plan-review, estimate, backlog, document — are untouched. Retirement followed the replacement's functional acceptance; no cost, speed or performance-parity comparison was made, and none is claimed. The static-invariant greps the deleted harness carried over prose and templates are an accepted loss; every behavioral protection it asserted (wrong-checkout selection, gate override, review coverage, anchor accounting, the verification gate, and no completion trailer before landing) is still asserted by `rdm-core`/`rdm-cli` tests and the surviving harnesses.

- Two regression harnesses whose subject no longer exists (`scripts/verify-workflow-do-auto.sh` and `scripts/verify-workflow-do-auto-task.sh`): they asserted the presence of particular sentences describing `rdm-do --auto`'s wiring into the fixed dispatch pipeline, and that wiring has been replaced. The checks that exercise real behavior — the emitted-skill and plugin gates, the review-pipeline gate, and the test suite — are unchanged.

- **The `spike-agent-type` Workflow is gone.** Its whole subject was `agentType: 'rdm-mechanical'` and `effort:` on a mechanical call site, and there are no mechanical call sites left; it disappears from the Workflow/skill listing along with the script. Its result is recorded in `docs/workflow-schemas.md` § "agentType / effort options spike". (See the no-mechanical-agents entry under *Changed* for what replaced the sites it measured.)

- **Two caller-supplied plan-review engine arguments are gone: `planText` and `gateMode`.** `planText` handed `rdm-wf-plan-review` a whole plan document as an argument; a plan is now named by `planSlug` and read by the reviewer itself, so `--implementation-plan` requires a `planSlug` to have anything to review. `gateMode` (with `PLAN_GATE_MODES` / `resolvePlanGateMode`) selected whether the gate wrote or merely reported; the engine writes nothing at all now, so every caller gets the commands back and decides for itself. (See the plan-engine entry under *Changed* for the replacement contract.)

- **The `rdm-index` git merge driver is gone, in both halves.** rdm no longer writes `merge=rdm-index` lines into your `.gitattributes`, and no longer installs a `[merge "rdm-index"]` section in `.git/config`. With mutations no longer regenerating `INDEX.md`, the driver had no job left — it existed only to resolve conflicts on a file rdm rewrote from both sides of a merge. `INDEX.md` now merges the way every other file does, with git's built-in three-way merge: a conflict is an ordinary conflict, with ordinary `<<<<<<<` markers, fixable with `rdm resolve <file>`. *(Superseded within this same unreleased cycle by the full removal below: `rdm index` and the automatic post-merge/post-pull regeneration this entry still describes are both gone.)* See `docs/file-formats.md` § "Legacy merge-driver cleanup" for what a repo configured by an older rdm should clean up by hand.

- `rdm index` no longer accepts the internal `--merge-output` / `--merge-path` flags, which existed solely so git could invoke it as that merge driver. *(Superseded within this same unreleased cycle: `rdm index` itself is removed below, so this distinction no longer applies.)*

- **`rdm index`, and generated `INDEX.md` / `projects/<p>/INDEX.md` files, are gone entirely.** Nothing in rdm generates or reads an index any longer: the `rdm index` command, `rdm resolve`'s post-merge regeneration, and `rdm remote pull`'s post-pull regeneration are all removed, along with `rdm-core::ops::index` and `rdm-core::display::index` (~940 loc). An `INDEX.md` a plan repo tracked from before this removal is left completely untouched — it becomes an ordinary tracked file, just like any other markdown file rdm doesn't own — nothing prunes or migrates it automatically. Use `rdm list --format markdown` for a browsable snapshot instead. This completes the collapse from the roadmap's originally-planned "keep `rdm index` as opt-in porcelain" shape to outright deletion, once it was confirmed nobody browses a plan repo without the rdm binary; see [`docs/index-removal.md`](docs/index-removal.md) for the decision record.

- **BREAKING: rdm's MCP server is retired end to end — the CLI flag, the command, the crate, and every distribution artifact.** This lands as one change across what were three separate stages of removal:

  - `rdm agent-config --mcp` no longer exists on any `agent-config` invocation, so `rdm agent-config claude --skills --mcp` (and every other `--mcp` spelling) now fails with the standard `unexpected argument '--mcp'` error instead of emitting anything. With it go three things it used to produce: the `.mcp.json` file written alongside `--out`/`--user` output, the MCP-flavored instruction file, and the eleven MCP-flavored skill variants that referenced `mcp__rdm__*` tool names instead of `rdm` commands. `rdm agent-config pi --mcp`'s bespoke "Pi does not support MCP natively" rejection is likewise gone — with no flag to reject, Pi now reports the same unknown-argument error as every other platform.
  - The `rdm mcp` command and the in-process MCP server are gone. The `rdm-mcp` crate has been deleted along with the CLI's `mcp` feature, so `rdm mcp` now fails with the standard `unrecognized subcommand` error instead of starting a server, and a build with `mcp` in its feature list no longer compiles (it never existed for `--no-default-features`, and is no longer in `default`). Every tool the server used to expose (`rdm_status`, `rdm_commit`, `rdm_review_requests`, and the rest) is gone with it. The `auto_init` global config key, whose only consumer was the MCP server's auto-initialize-on-first-call behavior, is retired along with it: `rdm config set auto_init <bool> --global` now reports it as an unknown key, and a global config file left over from before this change that still sets `auto_init` continues to parse without error — the key is simply ignored.
  - rdm is no longer distributed or documented as an MCP server. `server.json` and the npm package's `mcpName` field are gone, so the release workflow no longer registers rdm with the MCP Registry and the npm-published package carries no MCP metadata at all. The README's `### MCP Server` section — starting the server, registering it with Claude Code or Cursor, and submitting to the MCP Registry — has been removed, along with `docs/remote-mcp-server.md`.

  **The CLI and `rdm-server` (the HTTP REST API, still started with `rdm serve`) are unaffected by all three stages and continue to work exactly as before.**

  **Migration:** there is no in-process MCP replacement — script against the CLI directly, or drive `rdm-server`'s REST API. Use `rdm agent-config claude --skills --out <dir>` (or `--plugin --out <dir>`) for the agent lane, and `rdm agent-config <platform> --out <dir>` for instructions; both emit the CLI-flavored surface, which is unchanged. If you relied on the generated `.mcp.json`, configure the MCP server through your client's own MCP configuration instead.

  **What still works:** `npx -y @edpaget/rdm` / `npm install -g @edpaget/rdm` remain the documented npm install path for the CLI, `@edpaget/rdm` stays the npm package name, and the CLI and `rdm-server` REST API are unaffected.

### Fixed

- Refresh the packaged Codex review runtime from the canonical shared module after integration with main, so downstream installations receive the current review behavior.

- **`review update --applied-commit` now validates the SHA before storing it, instead of accepting anything typed.** The value is checked for existence against the review's actual target repository — the project's configured source repo for a `change/<sha>` review, the plan repo for a roadmap/phase/task/plan review — and refused, naming that repository, when it does not resolve there. An abbreviated SHA, `HEAD`, a branch, or a tag is now accepted and resolved to its canonical full 40-character form, which is what gets stored — not the operator's literal input. `rdm-server`'s equivalent `PATCH .../comments/:id` endpoint enforces the same check, returning `400 Bad Request` for an unresolvable value. The refusal now tells its cause apart: a checked path that isn't a git checkout at all is reported as such, rather than as a nonexistent commit, and a SHA that genuinely doesn't resolve in a real checkout is reported with a hint that an abbreviated SHA may be ambiguous.

- **An `addressed` or `dismissed` review's `applied_commit` and `reply` can now be corrected, and a genuinely-refused resolution-status change names the review's real state instead of the misleading "has not been submitted".** `review update --comment <n>` previously refused any resolution field once a review closed with that same message, even though the review had in fact been submitted. Only a comment's `status` remains fixed once a review closes; `applied_commit` and `reply` stay correctable through `addressed`/`dismissed`, for fixing a mistaken provenance value or note after the fact.

- **The code-review `changelog` reviewer no longer flags a commit whose entry was corrected by a later commit in the same reviewed range as a missing-changelog blocking finding.** A dispatch review covers a whole range — a phase's implementation commit plus any fix-up commits made while triaging that phase's own review — but the reviewer previously graded each commit individually against `CLAUDE.md`'s same-commit rule, so a review-driven fix-up (behavior changed in one commit, its changelog entry corrected in the next) was still graded blocking even though the range's head carried an accurate entry. It now grades the reviewed range's net entry at its head instead: a user-facing change anywhere in the range with no accurate entry at the head is still `blocking`, but an entry added or corrected by a later commit in the same range is no longer a finding.

- **The review-persist and plan-review-persist command ladders (`rdm-wf-review-refute-fix`, `rdm-wf-plan-review`) no longer fail to parse under macOS's system `/bin/bash` (3.2) when a finding's text contains an apostrophe.** Captured text (comment bodies, quotes, paths, the review summary) previously rode through a quoted heredoc nested inside a `$(...)` command substitution — a construct bash 3.2's parser cannot handle when the heredoc body contains a literal apostrophe ("it's", "don't"), so the emitted ladder failed before it ever ran. Captured values are now assigned through a plain single-quoted string instead, which still carries backticks, `$`, double quotes, em-dashes and embedded newlines through literally.

- **Plan review (`rdm-wf-plan-review`, and the `rdm-dispatch-phase` skill that drives it) can now be pinned to a specific source checkout and commit**, so its finder and refuter agents verify the pinned worktree hasn't moved and read cited files from it instead of the invoking session's own checkout — fixing false findings raised against files or symbols that exist only on an unlanded roadmap branch. Pass the same flat identity the code-review engine already takes (`source`, `base`, `expectedHead`, `expectedBranch`), plus `phase` or `task` to name the item the pin binds to; a roadmap sweep binds each phase unit to its own item independently, and the bare roadmap-body unit is never pinned. `base` and `expectedHead` are both validated as full hex commit ids before any agent runs, matching the code-review engine's own pin check. On a verification failure, the reviewer is told the checkout *could not be verified* — drift since the plan was written, or a bad pin/environment — and quotes the command's actual stderr in its blocking finding, rather than always diagnosing drift outright. The distributed `rdm-dispatch-phase` skill template's optional plan-review "second opinion" call now also names passing this pin from the item's pinned `identity`. Omitting the pin keeps today's behavior unchanged — reads come from the invoking checkout, exactly as before.

- **`rdm verify run --item` no longer misreads a malformed kind-prefixed reference (e.g. `phase/<roadmap>` with the stem omitted) as a phase of a roadmap literally named `phase`.** A malformed `plan/`, `change/`, or `task/` reference is refused up front, naming the grammar `--item` accepts. A malformed `phase/`/`roadmap/` reference is handled differently, because neither is a reserved roadmap slug: it still falls through to the ordinary `<roadmap>/<stem>` resolution first, and only once that resolution genuinely fails — and the leading segment does not itself name an existing roadmap in the project — does it get the same grammar-naming message, in place of the garbled nested "unknown item" error `ItemRef::parse` used to produce. A roadmap genuinely named `roadmap` or `phase` is unaffected either way — it still resolves, or reports its own real "unknown phase" error, through the ordinary grammar.

- **A project roadmap literally named `roadmap` (legal — `roadmap` is not a reserved slug) can now address its own phases through `--item roadmap/<stem>`.** That two-segment reference parses cleanly as an ordinary `roadmap/<slug>` reference, so it previously always collapsed to the bare slug `<stem>` and resolved (or failed to) as an unrelated roadmap literally named `<stem>` — there was no working `roadmap/<stem>` spelling for such a roadmap's own phases at all. `--item roadmap/<stem>` now resolves as a phase of the `roadmap`-named roadmap when `<stem>` genuinely names one of its phases (by stem or number); an ordinary `roadmap/<slug>` reference to any other roadmap is unaffected, even in a project that also has one literally named `roadmap`.

- **`rdm verify run --item <ref>` now exits a distinct code (3) when the item cannot be resolved to a worktree**, instead of the generic exit 1 a failing verification command also produces. The JSON payload reports the command as `resolved: true` with a null `exit`, so a caller can no longer mistake "the command never ran because the item was unresolvable" for either a failing run or an invitation to fall back to a command it reads itself.

- **The plan-review caller prose no longer tells the orchestrator to prepare arguments the engine deleted, or to rewrite a whole body to append one note.** `rdm-dispatch-phase`'s pre-review step still asked for a `roadmapBody` hoist, a `model resolve mechanical` id and a `model:mechanical` engine fallback, and still described a `fetch:*` agent that could re-read anything omitted — every one of which this cycle removed. Followed literally it re-emitted a 19.5 kB roadmap body into the review call's own arguments, the exact transport the change was made to eliminate. The step now gathers only what the session alone can supply (the two judgment-site model ids and the wont-fix corpus), and states plainly that no engine-side fallback exists for any of it. Separately, the plan-review round note is written with a read-modify-write performed in **Bash** — the body stays in a shell variable and never crosses the orchestrator's own output, which is the transport this cycle removed; `--body` remains whole-document-authoritative and there is no patch-shaped write.

- **A plan review that can reach no document is refused instead of reported clean.** `--implementation-plan` with neither `planSlug` nor `planFile` used to dispatch every reviewer against the literal string `(the implementation plan provided in context)` — with no plan in context, because a Workflow agent sees only its own prompt — and then return `reviewed` with `coverage.complete: true`. Coverage could not notice: every reviewer *did* run. That shape is now refused at argument-parse time, before a single agent is dispatched, with an error naming both ways to supply the plan. Free-form plan review itself is restored rather than removed: pass `planFile` with an absolute path and each reviewer reads the file itself, the same contract a `planSlug` gets. The in-repo Codex host lane (`scripts/lib/codex-runtime.mjs`), which reviews a loose plan file and had been silently passing the retired `planText`, now passes the path.

- **An emitted command ladder that a caller pastes into a plain shell can no longer exit 0 after a refused write.** The review-persist ladder (`persistCommands`/`persistScript`), the plan gate's tag-clear pair, the estimate writeback and the document write all ended with a read or a `printf`, and carried no per-line failure handling — so in a shell with no `set -e` (the shape the calling skills explicitly invite) a refused `rdm review start`, `phase update` or tag write was followed by a successful trailing command and the session exited 0. The documented reactions to a failure ("stop and escalate", "a nonzero exit anywhere else is a park") could therefore never trigger, and a persist ladder went on to run its whole comment loop against an empty review id. Every line that can fail now carries `|| exit 1`, the persist ladder additionally stops when it cannot read an id back out of `review start`, and the only unguarded line left is the trailing `printf` that reports the id. This is the same treatment the code engine's status-gate ladder already had.

- **The documentation generator no longer carries phase documents through its agents.** `rdm-wf-document`'s per-phase gatherer was required to return each phase's `body` verbatim, and the engine re-emitted every gathered body into the synthesis prompt — so a whole rdm document crossed one agent boundary as a payload and was re-emitted across a second, which is exactly the transport the rest of this lane removed, and a body that lost a line degraded the generated documentation with no signal. The gatherer now returns its own account of what the phase shipped and no body at all, and the synthesis agent is given each phase's `rdm phase show` command and reads the document itself. Two engine header comments that still described deleted mechanical Bash agents (`rdm-wf-backlog`'s report fetch, `rdm-wf-document`'s draft writer) are corrected to match what the engines do.

- **A code review that ran no `ac` reviewer now says so.** A caller-supplied `reviewers` set omitting `ac` leaves no acceptance-criteria table, which the outcome cannot approve against — so the review escalated with the bare message `required review evidence is incomplete`, while both visibility channels reported full coverage (`acTableAbsent` was computed as "the `ac` reviewer FAILED", which an *unselected* one never did). An unselected `ac` reviewer now counts as absent exactly like a failed one, the summary carries the `NO AC TABLE` clause, and the failure names the missing reviewer. Nothing refuses a thin set — coverage stays visible rather than enforced.

- **A refused status write can no longer leave the code engine's gate ladder exiting 0.** `gateScript`'s last line is a read-back, so a session that pasted the ladder reported the read's success even when the `--status` write before it was refused — by the `reviewed` gate, or by the source binding on a checkout that moved. Every line that can fail now carries `|| exit 1`, and the status the final read-back must show is the returned `status` field.

- **The shipped, plugin, and local `rdm-review` skill no longer instructs writing or obtaining a `Done:` trailer at the review gate.** Its Gate step still described the pre-phase-45 mechanism — "that flip is owned by the merge-to-main hook" and `rdm hook done-line` to synthesize a directive — which `rdm-land` marking a landed item `done` directly, with no trailer, made both wrong and actively misleading. The Gate step now says plainly that `rdm-land` marks a reviewed item `done` itself after a clean fast-forward onto `main`, and that this gate never sets `done` or amends the reviewed commit.

- **A mistyped reviewer name no longer makes a plan review silently review nothing.** `rdm-wf-plan-review` keeps one refusal — a caller-supplied `reviewers` set in which no name resolves throws rather than reporting a clean review over an empty fleet — but it was raised inside the per-unit parallel fan-out, where a thrown thunk degrades to a dropped unit. `reviewers: ['coherance']` (or `[]`) therefore returned `0 unit(s) reviewed` with no outcome and no error: indistinguishable from a sweep that genuinely found nothing. The set is now resolved at argument-parse time, before any agent runs, so the refusal reaches the caller. Independently, a unit lost for **any** reason is now named on `failedUnits` and in the summary, so "reviewed nothing" can never again present as "found nothing".

- **A phase whose acceptance criteria are followed by explanatory prose is no longer reviewed as having no criteria at all.** The review engines parse an item's `## Acceptance criteria` section into the checklist the code review grades against. Any unindented, non-bullet line after the list had started — ordinary trailing prose, such as a note recording what deliberately is *not* a criterion — discarded every criterion already parsed, and the review then escalated with `acceptance criteria missing or ambiguous` before a single dimension had run. Such a line now simply ends the list instead of invalidating it, so the real criteria are graded and the trailing prose is ignored, which is what it is there for.

- **`rdm` can commit again in an environment with no configured git identity.** Since commits became changeset-scoped, `rdm commit` (and every path that lands one — `rdm bootstrap --init`, `rdm init --remote`, the `Done:` hook, the MCP `rdm_commit` tool) updated `HEAD` through a reference edit that resolved the reflog committer from git config alone, and failed the whole commit with `failed to update HEAD: The reflog could not be created or updated` when no `user.name`/`user.email` was set. rdm's own fallback identity (`rdm <rdm@localhost>`) already signed the commit object; it now signs the reflog entry too. This never surfaced on a repo created by `rdm init`, which writes that fallback into `.git/config`, only on one obtained by clone — so a CI runner or a fresh container bootstrapping a plan repo hit it every time.
- **`rdm verify run --item` now resolves a phase to its roadmap's shared worktree instead of never matching.** Under one-worktree-per-roadmap, a registered worktree is keyed by its bare roadmap slug, but `--item <roadmap>/<phase>` matched on the per-phase key and always fell into the "no worktree" arm — whose suggested remedy, `rdm worktree add <roadmap>/<phase>`, would have created exactly the per-phase worktree this project abandoned. `--item` now routes through the same roadmap-collapse policy the `reviewed` gate's worktree probe already uses, and additionally accepts the kind-prefixed `phase/<roadmap>/<stem>`, `roadmap/<slug>`, and `task/<slug>` grammar that `--on`/`--implements` use, alongside the existing unprefixed `rdm worktree add` grammar. A miss now suggests `rdm worktree add <roadmap>` (or `task/<slug>`), never a per-phase worktree. `--item plan/<slug>` or `--item change/<sha>` — neither names a worktree — is now refused up front with a message naming the grammar `--item` actually accepts, instead of being silently misread as a phase of a roadmap literally named `plan`/`change` and failing with a garbled, misleading message.

- **Plan review now recognizes review findings it recorded with older versions of rdm.** Comments persisted before newer header fields existed were being misidentified as human input, so repeated findings from earlier review rounds were reported as new each time; they are now correctly recognized, preserving their original gating behavior.

- **A code review no longer aborts because the implementation plan was read twice.** `rdm-wf-review-refute-fix` used to re-read the approved plan partway through a review and byte-compare the whole document against the copy it read at the start, failing the run with `implementation plan changed during review` on any difference. A Workflow script has no shell of its own, so both reads cross a model-mediated transport that cannot reproduce a large document byte for byte — a single normalized trailing newline was enough to trip it. In practice this aborted reviews of exactly the long, thorough plans the lane is meant to encourage, threw away everything already computed (the abort fires after the source and acceptance stages have run), and reported a cause that had not occurred. Both the comparison and the redundant re-read are gone, so a review proceeds against the plan it resolved at the start.

- **The emitted `rdm-dispatch-phase` skill no longer tells downstream consumers that two Workflow engines ship.** Its "Why there is no plan-review Workflow call here" section still read "only the two engines named above ship" after the shipped set dropped to one, so every fresh `--skills`/`--plugin` tree (and the checked-in `plugins/rdm/`) asserted a capability count that no longer existed; it now names the one engine that ships, `rdm-wf-review-refute-fix`. The same correction reaches the shipped `rdm-mechanical` agent definition ("the one distributed workflow"), `README.md`'s raw-emission description (1 workflow engine; still 13 files), and `docs/codex-support.md`'s support matrix — whose distributed `rdm-dispatch-phase` row now records one Workflow call with a human approve review as the plan gate, which is what the shipped template actually instructs. `docs/plugin-distribution.md`'s runtime-argument and path examples name a surviving engine, and the last present-tense references to the retired engine in `docs/mechanical-agent-inventory.md` and `docs/review-evidence-closeout.md` are now explicitly historical.

- **`rdm agent-config claude --plugin --out <dir>` now prunes retired Workflow engines from the plugin tree it re-emits, instead of leaving them behind as live entrypoints.** Only the `--skills` path ran the superseded-engine cleanup; regenerating a plugin tree rewrote its files but never removed an engine rdm had retired, and an installed plugin lists every `workflows/*.js` as a callable `rdm:<name>` engine — so a consumer who upgraded rdm and re-emitted kept an orphaned, unmaintained engine in their plugin, silently. Both emission modes now run the same cleanup over the same core table and report each removal, so re-emitting is enough and no stale `workflows/<engine>.js` has to be deleted by hand. As with `--skills`, a file whose content you modified, and any file rdm does not know about, is left in place.

- **Starting a change review from inside the plan repo is now refused instead of silently reviewing the plan repo itself.** With a project that configures no `source.repo`, `rdm review start --on change/HEAD` run from the plan repo used to pin the *plan repo's* own `HEAD` and record a review of plan data as though it were the project's code. It now refuses with a message naming both remedies — run it from the project's source checkout, or set `source.repo` in its `project.md`. A project that deliberately configures its `source.repo` at the plan repo is unaffected.

- **A reviewed range with no commits in it is now refused instead of being recorded.** Standing on the project's default branch makes the derived merge base equal the reviewed head, so the range holds nothing and no `--path` anchor can ever land in it. The refusal names `--base` as the way to say what the change is diffed against. An explicit `--base <head>` is still accepted, since that is how an intentionally code-free review, and a review of an orphan branch's root commit, are recorded.

- **A source checkout that does not contain the reviewed commit is now reported as a missing commit, not a missing path.** `rdm review show` used to claim every anchored file "no longer exists at `<head>`" when the checkout simply lacked the commit — a confidently false statement about files the commit does contain. It now reports the reviewed commit as absent in `source_verification_skipped`, with a fetch / `source.repo` remedy, and leaves each comment's `unresolved_reason` empty, because an environmental skip is not a verdict about the reviewer's quote. `rdm review comment --path` likewise names the missing commit rather than blaming the path.

- **The REST API and the web UI no longer report every change-review comment `unresolved` with no explanation.** `GET /projects/<project>/reviews/<id>` reported all of them unresolved with both `unresolved_reason` and `source_verification_skipped` absent — indistinguishable from anchors that genuinely no longer resolve, and contradicting what `rdm review show --format json` said about the same review. Both the REST response and the HTML review section now resolve change-review anchors against the project's configured local `source.repo` and otherwise state why verification was skipped. A change review also now appears on the detail page of the phase or task its implementation plan implements, which is where its anchors are worth reading.

- **A source path containing `@` or `#` is now refused when you anchor a change-review comment to it.** Those are the two characters the `rdm:src/<path>@<rev>#L<n>` permalink grammar reserves, and nothing escapes them, so a comment on (say) `web/app/@modal/page.tsx` used to be accepted and then emit a `Source:` link that resolved back to the path `web/app/` at a nonsense revision. `rdm review comment --path` now rejects such a path up front, naming the offending character, and every path it does accept is guaranteed to round-trip through the link grammar. Characters the grammar does not reserve — `?`, `%`, spaces, non-ASCII names — are still accepted, and anchors already stored with a reserved character still render rather than failing.

- **Change-review drift detection no longer misses an edit that is masked by a fresh copy of the same text.** Drift used to be decided purely by how many occurrences of the quoted text the file held, so one commit that edited the anchored occurrence away *and* introduced an identical copy elsewhere left the count unchanged and the comment reported `resolved` although the line it named was gone. Resolution now also checks that some occurrence at the tip still sits between the same neighbouring lines the anchored one sat between. The count test is kept as the first of the two clauses, so this can only ever *add* drift detections — nothing that reported `drifted` before can start reporting `resolved` — and code that merely moved with its neighbours, or gained a trailing comment beside the quote, still resolves.

- **A dismissed approving change review no longer satisfies the `reviewed` gate.** An approval whose review was *dismissed* — closed without being acted on — used to keep counting as gate evidence forever. Only `submitted` and `addressed` approvals count now; `addressed` still counts, because there the approval stood and its comments were worked. An item already marked `reviewed` on the strength of a dismissed approval keeps that status (the gate is write-time only), but the next `reviewed` write can now refuse.

- **A stale approving change review is now named as stale instead of reported as missing.** When an approving `change/` review exists but was recorded at a different HEAD than the checkout being marked reviewed, the refusal used to be "no approving change review … record one with `rdm review start`" — telling you to create a review that already exists. It now names the review id, both abbreviated SHAs, and the re-review command. In the same change, the HEAD-freshness check runs on **every** observed checkout rather than only a clean one, so a dirty worktree carrying a stale approval reports the stale refusal (the documented (a) → (b) → (c) order) instead of a cleanliness complaint that hid the real cause. This moves only which refusal is reported: every observed-but-not-clean checkout already failed the cleanliness precondition, so no write that was accepted before is refused now, and none that was refused is accepted.

- **The `# Errors` contract on every gated status-write entry point now lists every gate refusal.** All three — `update_phase_gated`, `update_phase_with_estimate_gated` and `update_task_gated` — omitted the disabled-gate refusal (`--override-gate` passed while the gate is off), and two of them scoped it wrongly under "only when the gate is enforcing", which is precisely when it cannot fire. The third delegated by intra-doc pointer plus a hard-coded count of how many gate variants there were, which this release makes false. All three now enumerate the variants directly, and `scripts/verify-reviewed-gate.sh` gains a section that keeps them complete — discovering gated wrappers by signature, pinning the count so a fourth cannot be silently skipped, and rejecting any re-introduced hard-coded count.

- `rdm backlinks`'s and `rdm link check`/`rdm link list`'s `--help` text, `rdm describe`'s `review.target` field, and the shipped Claude Code instructions template now list `plan/<slug>` alongside `roadmap/<slug>`, `phase/<roadmap-slug>/<stem-or-number>`, and `task/<slug>` as an accepted/linkable form (and, where the surface documents a review target rather than a link, `change/<head-sha>` too) — these had not been updated when `plan/<slug>` and `change/<head-sha>` were added as reference kinds.

- A mistyped `--on plan/<slug>` or `--on change/<head-sha>` review target now gets a factually complete error: `rdm`'s "invalid review target" message lists `plan/<slug>` and `change/<head-sha>` alongside `roadmap/<slug>`, `phase/<roadmap-slug>/<stem-or-number>`, and `task/<slug>` as accepted forms, instead of naming only the first three.

- `rdm backlinks change/<head-sha>` now fails with an actionable error ("names commits in the source repository, not a document in the plan repo") instead of silently succeeding with an always-empty result. A change names source-repository commits, not a plan-repo document, so nothing can ever reference it via an `rdm:` link or an `implements` field — `rdm link check --on`/`rdm link list --on`, which resolve the same reference grammar, already rejected a change target the same way; `rdm backlinks` now matches them.

- A source-repository lookup that genuinely fails — git missing, or a malformed stored `change_branch` refused by the revision-safety guard — no longer silently re-points a change review's drift at the repository's HEAD and reports anchors as resolved or drifted against a revision nobody asked for. `rdm review show` now reports those anchors `unresolved` and says why in its `source_verification_skipped` note, the same way it already degrades when no source repository is reachable at all. A branch that was merely renamed or deleted still falls back to HEAD as before.

- A persisted review now reports how each finding was actually recorded, so a degraded code review can no longer read as a clean one. Findings that never carried a quote are counted separately from anchors that were attempted and failed; each failure records why (quote not found, ambiguous, occurrence out of range, outside a changed hunk, missing path, path not a file, path not applicable, start fallback). A review whose attempted anchors all failed, whose counters do not add up, whose target fell back, or whose committed range was empty is escalated for adjudication instead of approved, and the run summary names the degradation. A comment that anchors on a retry stays a clean result. When a change review falls back to its plan-repo document, the emitted commands can no longer carry change-only flags (`--path`, `--base`, `--implements`) that such a target refuses. A quote that cannot be anchored against a code review — because the review declared no committed code change, or because the finding names no file — is now written as a whole-document comment and counted as degraded, instead of producing a command the binary refuses and losing the whole review along with every other finding in it.

- A `change/<sha>` review's `--path`/`--quote` comment can no longer anchor to a directory or a submodule. `rdm review comment --path <directory-or-submodule>` now fails with an actionable error naming the path and what it actually is, and writes no anchor; a comment anchor stored before this check existed (or hand-edited) is re-checked on every read and degrades to `unresolved` with an explanation instead of resolving — `rdm review show` (human, Markdown, and JSON) prints the reason and emits no `rdm:src/...` permalink for it. The `SourceRepo` port gained `object_kind_at`, answering via git's own object typing (`git ls-tree`) rather than guessing from content, so a normal text file whose contents happen to resemble a directory listing still anchors correctly.

- Keep non-Git CLI builds working and bind standalone code approvals to the intended implementation plan and every acceptance criterion. Restore code-comment anchor and source-link regression coverage.


- Standalone code reviews now resolve and pin the existing shared roadmap checkout and exact commit range, including explicit task bindings. `rdm review source` refuses wrong, moved, or unexpectedly empty sources without creating worktrees; source-bound status writes validate the same identity, and the reviewed gate requires approval for the observed head. Automatic reviews cannot approve with missing dimensions, invalid acceptance evidence, unresolved refutation overflow, failed persistence, or failed status readback.

- Fixed Codex implementation handoffs failing when setting `needs-review` by removing the terminal-only `--commit` flag; source provenance remains in the handoff body and pinned code links.
- Clarified that user-level instruction paths preserve relative `CODEX_HOME`/`HOME` values and resolve them against the invoking process's working directory, rather than guaranteeing an absolute path.
- Added an isolated Codex coexistence check for selecting repository skills alongside installed user/plugin copies without changing those copies, with an explicit-path selection recipe.
- `rdm review comment --path` on a `change/` review now works from **any** directory inside the source checkout, not only its top level. The two git reads behind an anchor disagreed about what the path was relative to: the file content was read with `git show <rev>:<path>` (resolved from the repository root) while the touched-hunk set came from a `git diff` pathspec (resolved from the current directory). Run from a subdirectory, the file read succeeded but the hunk set came back empty, so a file the change genuinely modifies was refused with `'<path>' is not touched by <base>..<head>` — a confidently false error whose suggested remedy ("comment on a file the change modifies") was the thing you had already done. In the agent lane this was quieter still: the review workflows' anchoring fallback turns an out-of-hunk refusal into a whole-document comment, so every anchored finding silently lost its anchor. The diff pathspec is now anchored at the repository root and the diff is forced non-relative (`--no-relative`), matching what `--path`'s own documentation always promised. Both guards are needed: anchoring the pathspec alone still came back empty for anyone with `diff.relative = true` in their git config, since git filters the output to the current directory after matching the pathspec.

- **If your `.git/config` still has a `[merge "rdm-index"]` section from an older rdm, remove it yourself:** `git config --remove-section merge.rdm-index`. rdm does not write `.git/config` and does not clean it up. This matters if your `.gitattributes` still maps `INDEX.md` to that driver: the driver command no longer exists, and git treats a failing merge driver as a conflict whose result is your own side, unmodified and with no conflict markers — so the incoming rows are dropped silently.
- A tracked `.gitattributes` carrying `merge=rdm-index` is **left alone** — it is a file you may have edited, and it is harmless without a configured driver: git falls back to its built-in three-way merge, with proper conflict markers and nothing on stderr. Delete the lines if you like; you do not have to.

- The refusal `rdm commit` prints when another session's flush has overwritten a path this changeset journaled (`Error::ChangesetPathOverwritten`) now states plainly what re-running the recommended command actually does: it re-reads current disk, so the retry will fold in whatever content is there now — which may be another session's already-landed edit — before committing under this session's message. Previously the message only said to "re-run the command that produced your change, then commit," without saying what that retry carries. See `docs/lost-update-evaluation.md` § "Retry attribution (phase 16)" for the recorded decision (this is accepted as the changeset model's rebase-onto-current-disk semantics, not a bug) and its rejected alternative (refuse until the other session commits).

- `rdm commit` no longer loses paths journaled by a concurrent process under the same session id. Under an agent harness that publishes one session id for a whole session (`CLAUDE_CODE_SESSION_ID` and friends), every parallel subagent resolves the same changeset, so a `rdm commit` and a `rdm task create` running at the same time were two processes writing one journal — and the commit rewrote that journal wholesale, silently destroying anything recorded while it ran. With 40 parallel creates and 6 concurrent commits under one session, all 40 tasks landed but both `INDEX.md` files were left dirty, `rdm session list` reported the changeset as having journaled no paths, and the next `rdm commit` disowned those indexes as belonging to another changeset with no way to recover them short of `--all`. A commit now records what it landed by appending a single line rather than rewriting the file, and that record names the exact content it landed — so a path a concurrent process rewrote in the meantime (the regenerated `INDEX.md` files, in practice) keeps its entry and the next commit lands it normally.

- `rdm commit --all` no longer strands a path another process journaled while it ran. It used to snapshot the working tree first and only then read each changeset's journal to clear it, so a record appended in between named content the snapshot never captured yet was cleared anyway, leaving its file on disk claimed by nobody. Every journal is now read before the snapshot and cleared from that read, so a record appended afterwards keeps its claim and the next scoped commit lands it.

- `rdm session gc` gains a second sweep that removes changeset journals which are fully committed and owned by no live session, and reports `Removed N fully-committed changeset journal(s).`; `rdm session list` no longer reports a changeset that claims no paths, matching what it showed before. The sweep is safe to run at any time, including from an unrelated shell while other processes are actively writing: rdm cannot generally tell whether anyone is still appending to a shared changeset (a session id pinned with `RDM_SESSION`, or published by a harness, records nothing on disk that would say so), so instead of guessing, every journal write and every sweep take one kernel-enforced lock — writes share it, a sweep needs it alone. A sweep that arrives while a write is in flight skips that changeset; a write that arrives while a sweep is mid-rewrite waits for it to finish and then lands in the rewritten file; and because the kernel releases the lock the instant its holder exits, a killed sweep can never wedge a write. A write that finds the lock held for longer than 10 seconds by a process that is alive but not running gives up *without* writing, so the mutation is named as unattributed by the next `rdm commit` rather than recorded into a file about to be thrown away. A sweep killed between writing its temporary file and renaming it over the journal no longer leaves that temporary file behind: the next sweep of the same changeset removes it.

- `rdm session discard <id> --force` no longer destroys a concurrent process's record along with the changeset. It used to delete the journal file outright, so a `rdm task create` running at the same moment under the same session id lost its record and was left on disk as unattributed dirt that no later commit would claim. A discard now retires exactly what it read, using the same appended record a commit writes, and removes the file afterwards only when nothing else is writing to it: a concurrent write keeps its record and commits normally, while an uncontended `rdm session discard --force` still leaves no journal behind.

- An idempotent write — a status update that re-sets a field to its own existing value, or any other write whose bytes end up matching what's already at HEAD — no longer leaves a stale journal entry behind forever. Previously, `rdm commit` only cleared a path from the journal when that specific commit actually changed its blob, so a no-op write (and a fully no-op changeset, which never even reached the truncation step) kept claiming a path indefinitely; once another session later edited that same path and landed its own commit, the *next*, entirely unrelated `rdm commit` from the first session was wrongly refused with "another session overwrote it" — a failure with no applicable remedy, since the change that supposedly needs re-running was never actually lost. `rdm commit` now truncates every path it can prove is correctly reflected at HEAD, whether or not this specific commit's tree changed, so a no-op update clears itself out of the way immediately. The same fix covers a no-op *delete*: deleting a path another session already deleted and landed first no longer leaves a stale delete entry in the journal, which previously could make a later, unrelated commit fail with "recreated" once a third session recreated that same path. `rdm commit --all` gets an independent fix: it now clears every changeset's journal on disk (live or orphaned) once the whole working tree is confirmed to equal the new HEAD — including when the tree was already clean before `--all` ran — so `rdm session list` correctly reports nothing outstanding once everything has landed, rather than continuing to show changesets holding paths that are already committed.

- `rdm discard` no longer silently overwrites another session's uncommitted edit to a path this session also journaled. It now compares each path's on-disk content against the digest this session recorded when it wrote there (and, for a journaled delete, checks the path is still absent) before restoring it to HEAD — mirroring the guard `rdm commit` already applies. A path another session has since overwritten, or recreated after this session deleted it, is left exactly as it is and named on a new `skipped:` line, instead of being silently reverted or removed; every other path this session legitimately owns in the same batch still discards normally and the command still exits 0. A session's own edits, and a path nobody else has touched, are unaffected. `rdm discard` also now retires from the journal exactly the claims it read and restored: a sibling's record appended under the same session id while the discard was running keeps its claim and commits normally, where previously it was retired unrestored and its file left on disk as unattributed dirt.

- An agent harness that runs each tool call in a fresh shell no longer leaves a dead session lease behind for every `rdm` invocation. Lease creation now sweeps stale leases before it mints one, so a harness that resolves a new changeset per call keeps a bounded lease directory (about one file) instead of one file per command, each naming a process that had already exited. The sweep only ever removes a lease whose owning process is gone or whose pid was recycled, it never runs when the process table cannot be read, and it never touches a changeset journal — so no in-flight work becomes unrecoverable.

- A harness-published session id (e.g. `CLAUDE_CODE_SESSION_ID`) now always wins over an inherited ancestor lease, closing a session-merging bug: previously, once one shell in a session's ancestry had run a single bare (harness-less) `rdm` invocation, every Claude Code session launched under that shell afterward — regardless of its own distinct harness session id — silently inherited that shell's lease and shared a changeset with it, so two independent Claude Code sessions launched from one already-leased terminal could sweep each other's uncommitted work into the same commit. Two sessions launched this way now resolve distinct session ids as expected, while a single session's identity remains stable across repeated invocations.

- `rdm session adopt <id>` now refuses up front, with an actionable error naming the offending variable, when the caller's environment carries a harness session variable (e.g. `CLAUDE_CODE_SESSION_ID`) — instead of reporting success while silently doing nothing. Adoption works by re-pointing the caller's inherited-lease state, but a harness variable now always outranks that lease (see above), so the repointed lease was never being consulted by the caller's own subsequent `rdm session id` calls; the command previously printed "Adopted changeset …" even though the session kept resolving its unchanged harness id. Adopt from a shell with no harness variable set, or pin the id explicitly with `RDM_SESSION=<id>` instead.

- `rdm config set` now stages `rdm.toml` through the plan repo's `Store`, the same path `rdm init` already used, instead of a raw filesystem write that belonged to no changeset. Previously, after `rdm config set`, `rdm status` misreported the change as belonging to "another changeset" and a scoped `rdm commit` (no `--all`) silently landed nothing — only `rdm commit --all` could commit it. `rdm config set` followed by a scoped `rdm commit` now lands `rdm.toml` under the caller's own changeset like any other write. As a narrow, deliberate side effect, `rdm config set` (non-`--global`) against a directory that has never been `rdm init`'d now fails with an actionable error instead of silently writing a git-less `rdm.toml`, matching every other mutating rdm command.

- `rdm status`, `rdm commit`, and `rdm discard` now distinguish two previously-conflated kinds of dirty path outside the caller's own changeset: paths a real other live or orphaned changeset claims ("belong to another changeset", recoverable via `rdm session list` / `rdm commit --changeset <id>` / `--all`) versus paths no changeset claims at all ("are not attributed to any changeset" — a write outside rdm, or dirt predating session-scoped commits — recoverable only via `--all`, since there is no changeset id to target). Previously both were reported as belonging to "another changeset", which was actively wrong for the unattributed case and pointed at a recovery route (`rdm session list`) that could never find anything.

- **`rdm commit` no longer aborts when another session created — but has not yet committed — the project your work lives under.** One session running `rdm project create alt` and leaving it staged, while a second created a task under `alt` and committed first, used to fail that second commit outright with `project not found: alt` — actively misleading, since the project *had* just been created. Your work now lands. The one trade-off, made deliberately: the `INDEX.md` that commit carries has no row for your new document yet, because writing one would mean committing a `projects/alt/INDEX.md` for a project whose `project.md` the commit does not contain. Nothing is lost and nothing needs doing — the rows appear automatically as soon as the other session commits its project. The committed index still never names a file the commit does not contain.

- **Two sessions editing the same item no longer silently drop each other's changes.** Previously, if you and a colleague (or two agent sessions) both ran `rdm task update <slug> --tags …` against the same task, both read the same tags and whoever wrote second simply won — the other's tag was gone, both commands exited 0, and nothing anywhere reported it. Since `--tags` replaces the whole list on update, that is how a reserved tag like `needs-plan-review` or `depends-unlanded` could quietly disappear. rdm now refuses the losing write instead: if an item changed on disk after your command read it, the command fails with an error naming the item (`refusing to write task/fix-bug: it changed on disk after this command read it …`), **nothing at all is written** — not even the unrelated files that command was about to touch — and re-running it applies your change on top of the current content. The same protection covers the gap between writing and committing: `rdm commit` now refuses to land a file another session overwrote after your session wrote it, rather than committing their content under your message. Your own back-to-back commands are unaffected, because the check compares file *content* and never who wrote it; the generated `INDEX.md` files are exempt, since every mutation legitimately rebuilds them. On the `Done:` hook path a conflict is logged and the hook still exits 0, so it can never fail the `git commit` or `git merge` that triggered it. Over HTTP both refusals are `409 Conflict`. Deletions are covered too — see the entry below. The evaluation that chose this mechanism, and every gap it deliberately does not cover, is in `docs/lost-update-evaluation.md`; gated by the new `scripts/verify-lost-update.sh`.

- **A staged deletion no longer wipes a file another session recreated at that path.** rdm's workflow deliberately separates staging from committing, so the gap between `rdm promote` (or `rdm roadmap delete`) staging a deletion and your later `rdm commit` landing it can be arbitrarily long. If another session created a new item at that same path in the meantime, your delayed commit used to remove their file from the landed tree with no error and exit 0 on both sides — their work was gone, and the only trace was that the file still sat on disk while being absent from git. `rdm commit` now refuses instead, naming the item: `refusing to commit the deletion of task/fix-bug: it is present on disk again, so another session recreated it after this changeset deleted it …`, states that nothing was committed, and tells you to re-read the item and re-run your delete if it is still right. Your own staged work is unaffected — a path you wrote and then deleted, or created and then deleted, before committing still commits cleanly, as does a deletion another session already landed — because the check asks only whether the path you emptied is filled again at commit time, never who emptied it. The generated `INDEX.md` files are exempt, since every mutation rebuilds them. Over HTTP the refusal is `409 Conflict`.

- **A code-mode review finding now carries a structured, repo-relative `path` distinct from its free-text `location`, so a persisted change review anchors reliably instead of silently downgrading.** A finder's `location` field is conventionally `<path>:<line>`, but it stays free text and may carry extra human-readable detail past that (e.g. `"src/x.rs:12-18 (mirrored at ...)"`) that defeated the anchor writer's end-anchored suffix-stripping heuristic — the finding's genuine `quote` then silently degraded to an untraceable whole-document comment, indistinguishable from a finding that never asked for a file anchor at all. Code-mode finders are now asked for `path` explicitly whenever they supply `quote`, and the writer tries that structured field first, falling back to the old `location`-parsing heuristic only when it is absent. A persisted comment now records, in a new `anchor` header line, whether its anchor landed (`path`/`quote`), was never requested (`wholeDocumentIntended`), or was dropped at build time (`degraded`) — and a review whose anchors degraded now says so, with a count and reason breakdown, in its own persisted summary.

- **A finder-declared `path` carrying a stray `:<line>` suffix no longer aborts an entire persisted review.** The prompt asks for a bare repo-relative `path`, but a finder that pattern-matches the adjacent `location: <path>:<line>` line sometimes echoed the suffix onto `path` too (e.g. `src/foo.rs:12-18`) — the validity check accepted it anyway (it still looked path-shaped), so the writer emitted an unrefusable `--path`, the real binary refused it outright, and the ladder's `|| exit 1` left a draft review behind with nothing recorded. A trailing `:<line>` or `:<start>-<end>` suffix is now stripped from a declared `path` before it is validated (the same heuristic already applied to `location`), so the finding still lands a real anchor instead of aborting the run. The finder prompt and schema documentation now say explicitly that `path` takes no line suffix.

- **A review whose comment anchors ALL degraded at persist time is now distinguishable, by the caller, from an ordinary clean persist — including an anchor the real binary only refuses once the ladder actually runs.** Previously the all-degraded signal was computed purely at build time, so a finding whose `path` passed every build-time check but whose quote sat outside a hunk the change touches (or whose path fell outside the reviewed range) was invisible: the emitted `--path`/`--quote` line simply aborted the whole ladder. The persist ladder now retries such a refusal itself, mechanically: the same comment is re-emitted whole-document, header-marked `anchor: degraded`, and tallied at run time. The ladder's trailing `printf 'anchorsDegraded=%s\n' "$RDM_PERSIST_ANCHORS_DEGRADED"` line now reports the combined build-time-plus-run-time result — `all`, `partial`, or `none` — rather than a build-time-only preview. `rdm-dispatch-phase` now parks `blocked` by reading that printed line directly, rather than the build-time-only `result.persistDegraded.all` field (still returned, now documented as a preview only), and no longer needs its own prose fallback for a refused anchored comment. The engine's own headless `gate: true` path gained the same protection: its emitted `gateScript` refuses to write `reviewed` when the `RDM_PERSIST_ANCHORS_DEGRADED` environment variable (threaded from the persist ladder's own output) reads `all`. A partially-degraded run still proceeds normally. The persisted review itself now carries the real result too: whenever the combined build-time-plus-run-time degraded total is greater than zero, the ladder appends one whole-document note comment, before submitting the review, stating how many of how many requested anchors could not be placed — a review whose anchors all degraded at run time used to persist with a clean-looking summary and no trace beyond a per-comment header and a stdout line nothing ever wrote back to the document. The all-anchors-degraded gate rule is also single-sourced now, as `persistDegradationGateLines()` in the shared review pipeline, rather than a hand-copied shell block in the engine's driver region.

- **`parseCommentHeader` now accepts a legacy seven-key comment header** (persisted before the `anchor` header line was added) instead of returning `null` and having it misread as an unheadered human comment — which silently broke `priorFindingsFromReviews`'s repeat-finding detection for every review persisted before that change. `anchor` comes back `undefined` (unknown), never guessed, for such a comment.

- **`rdm review start`/`comment`/`submit` no longer hang reading stdin under a non-interactive caller** (e.g. an agent holding stdin open as a never-closing pipe) when `--body` is omitted. They previously blocked on a stdin read before `--no-edit` was even consulted, so `--no-edit` could not save you; they now read a body only from `--body`, or interactively from `$EDITOR`/`$VISUAL` on a real terminal without `--no-edit`. This was deadlocking the very first `review submit` in every persist ladder `rdm-wf-plan-review`/`rdm-wf-review-refute-fix` emit. Independently, every `rdm` line those ladders emit (including `lib/plan-review.mjs`'s tag-clear gate) now redirects stdin from `/dev/null` too, for defense in depth.

- `rdm-dispatch-phase`'s code-review stage now runs its finders and refuters on the resolved `review-find`/`review-verify` models instead of silently inheriting the orchestrating session's model — the same models the plan-review stage already used.

- **A difficulty-only `phase update` no longer strands a stale auto-derived model tier.** Raising (or lowering) `--difficulty` without an explicit `--model` now re-derives the model tier when the previously recorded tier matches what the difficulty being replaced would itself have derived; a tier a human explicitly chose (that doesn't match its own difficulty's derive) is still preserved.

- The shipped review skill no longer claims a `medium`→sonnet built-in default; it points at `rdm model show` and passes the resolved effort.

## [0.21.0] - 2026-09-03

### Added

- rdm's machine-facing `--format json` contract is now frozen as committed golden files under `tests/golden/` — one snapshot per JSON-emitting read command (`info`, `roadmap`/`phase`/`task` list/show, `list`, `search`, `next`, `tree`, `describe`, `tag list`, `backlog report`, `model show`, `review list`/`show`/`requests`, `worktree list`/`current`), captured against a deterministic fixture and redacted so it stays reproducible across machines and days. A new `scripts/verify-golden-json.sh` re-captures and diffs on every CI run, so breaking the JSON contract an editor or plugin integration depends on now fails CI with an actionable re-bless hint (`scripts/capture-golden.sh`) instead of shipping silently. See `tests/golden/README.md`.

- A new `rdm info` subcommand reports what rdm actually resolved for the current environment in a single call: `{root, project, default_branch, default_format}`. This closes the gap for an editor or plugin integration that needs to map its working directory to a plan repo without scraping the text-only `rdm config list` (which ignores `--format json`) or reading `rdm.toml`/`config.toml` directly. `--format json` emits the four fields with `project` omitted entirely (not `null`) when it cannot be resolved, so a plugin can detect "no project selected here" by the key's absence rather than by a special value; `rdm info` (human) and `--format markdown` show the same values each paired with the source it came from (CLI flag, environment variable, repo config, global config, or default), and `--format table` is rejected with the same actionable message shape as `rdm next`/`rdm tree`. Every value follows the exact precedence the rest of the CLI already uses — including that `project` is resolved via `RDM_PROJECT`, not the `RDM_DEFAULT_PROJECT` env var `config list`/`config get` check for the unrelated `default_project` config key, and that `default_branch` has no env-var layer at all. `rdm info` works before `rdm init` (like `rdm describe`/`rdm model`), and root resolution failing (e.g. no `$HOME` and no `RDM_ROOT`) is still a hard error, since only project resolution is made optional. `rdm.toml` is read once per invocation — the repo-vs-global source labels reuse the same `resolve_config_value` precedence helper `rdm config get`/`config list` already use, rather than a second raw parse — so a malformed `rdm.toml` reports its parse-error warning exactly once, not twice.

- A new repo-only `dispatch.verify` config key declares the single command that verifies a project. `rdm config set dispatch.verify "<cmd>"` writes it to the repo `rdm.toml` (`rdm config get dispatch.verify` reads it back, `rdm config list` shows it); it is rejected with `--global` with an actionable error, because a verify command is a property of a project rather than of a user, and an empty or whitespace-only value is refused rather than silently disabling verification. The autonomous dispatch lane (`rdm-wf-dispatch-phase`, and therefore `rdm-do --auto`, `rdm-dispatch-phase` and `rdm-autopilot`) now runs that command once after each implementation attempt, in the item's worktree, and reads its exit code: a non-zero exit — or a verification that could not be run at all — sends the item back through the existing rework budget with the failing command and its output tail handed to the rework implementer, instead of reporting the phase reviewed. The resolved command is also shown to the implementer up front as available tooling, so it is run proactively rather than discovered as a failure. rdm runs the one command and nothing more: it never decomposes, reorders, partially runs, retries, or times out the command — ordering, parallelism and per-tool configuration stay in whatever task runner the command invokes. See `docs/verify-gate.md`.

- `rdm config get <key> --raw` prints the resolved value alone — no `(source: …)` annotation, and no output at all when the key is unset — so a script or agent can consume the value verbatim instead of parsing it out of the human-readable line. The dispatch verify gate reads `dispatch.verify` this way; without it, a value read back through the annotated form would be handed on as `<cmd>  (source: repo config)` and fail as a shell syntax error. The default output is unchanged.

- Plan review now checks a plan against the roadmap's recorded `## Intent`. A new `intent-alignment` dimension asks two questions the other plan lenses structurally cannot: could every acceptance criterion pass while the recorded "Done looks like" stays false (divergence), and does any step pursue something recorded as a non-goal (scope creep)? An acceptance criterion can be internally coherent and still leave the stated goal unmet — that is exactly what this catches. A phase inherits its parent roadmap's intent, so reviewing a roadmap checks the roadmap body and every phase against the same recorded goal. When no intent is recorded — no `## Intent` section, a section reading `(not captured)`, a partially-filled one, or a caller with no roadmap in hand — the check does **not** run: no agent is dispatched, nothing is blocked, and the absence is reported back as a non-blocking suggestion naming the missing input rather than failing the plan. Ships in `rdm-plan-review` (CLI and MCP) and in the distributed workflow engines emitted by `rdm agent-config claude --skills` / `--plugin`. It is also live on the autonomous dispatch path: `rdm-wf-dispatch-phase` reads the parent roadmap's body alongside the phase in its existing Stage-0 fetch (no extra agent), and the three shims that pre-fetch that metadata themselves — `rdm-autopilot`, `rdm-do --auto`, and `rdm-dispatch-phase` — now read it too and hand it over as `roadmapBody`, so a phase dispatched by autopilot is checked against its roadmap's recorded intent rather than silently reported as having none. That field is optional: a failed roadmap read, or a caller still on the older payload shape, degrades to the same non-blocking suggestion.

- The `rdm-roadmap` skill now runs a short, bounded interview with the operator before designing phases — at most 3-5 closed-form questions, one at a time, covering the goal as an observable end state, what is explicitly NOT wanted, and one operator-testable "done looks like" signal. Every answer is recorded verbatim into the roadmap body's new `## Intent` section (with unresolved high-impact questions listed under `Open`); the interview terminates early on an operator signal ("done", "that's it", "no more"), and a headless run with no human present writes `(not captured)` instead of guessing. `rdm-plan-review` now offers the same capture, interactively, for a roadmap or task whose intent was never recorded — skipping silently when it already reads `(not captured)`, a deliberate prior opt-out. Ships in both the CLI and MCP variants of the `rdm-roadmap` and `rdm-plan-review` skill templates (`rdm agent-config claude --skills` output). The `## Intent` section now follows a canonical grammar both skills write as a literal, fill-in-the-blanks template — `**Goal.**`, `**Non-goals.**`, `**Done looks like.**`, and `**Interview.** (captured YYYY-MM-DD)` — so every captured section shapes the same way. `Non-goals`, `Interview`, and an optional `Open` list may be absent; `Goal` and `Done looks like` are what make a section count as captured rather than present-but-empty. This is prose only — no parser, no splice operation, no `rdm intent` command, and no MCP intent tool.

### Changed

- A dispatch that reports an item `reviewed` has now committed everything the run produced. Previously, when the code review fixed a small finding inline, the fix was left uncommitted in the item's worktree and the dispatch still returned `reviewed` — and because landing rebases before merging, that remediation never shipped, so the finding it satisfied was satisfied by nothing. Three changes close that: the review's fix-applying step now commits its own change, with a conventional-commit message whose body carries a `Review-Finding: <id>` line naming each finding that commit closes, and reports the resulting short sha back so the finding **and the commit that closed it** are both visible in the dispatch outcome (on a parked outcome too, not just a clean one); a fix that lands after the verification command already ran now re-runs it, so nothing ships unverified; and a dispatch whose worktree is still dirty at the end comes back as `rework` naming the uncommitted paths instead of reporting the item reviewed. The outcome vocabulary is unchanged (`reviewed | rework | escalated`), the rework budget is not re-entered, and `--plan-only` runs are unaffected. Applies to the autonomous dispatch lane everywhere it runs — `rdm-do --auto`, `rdm-dispatch-phase`, `rdm-autopilot`, and a direct `rdm-wf-dispatch-phase` invocation — both in this repo and in the engines emitted by `rdm agent-config claude --skills` / `--plugin`. See `docs/verify-gate.md` § 8.

- A dispatch that cannot determine how to verify itself now escalates instead of silently reporting success. When `dispatch.verify` is unset, the dispatch first tries to discover a verification command from the project's CI configuration (`.github/workflows/`, `.circleci/config.yml`, `.gitlab-ci.yml`), then `docs/principles.md`, then `CLAUDE.md`/`AGENTS.md`; only if all three yield nothing does it stop — before planning or implementing anything — and report the item `blocked` with a summary naming the config key and every source it checked. A `--plan-only` run is exempt, since it does no implementation. The outcome vocabulary is unchanged (`reviewed | rework | escalated`), and repeated verification failures consume the existing code-rework budget rather than a second one.

- The canonical review's refuter no longer accepts "it's documented / known / already accepted as scope" alone as grounds to dismiss a finding that contradicts the target's stated goal or recorded intent — a recorded deferral is now treated as evidence the defect is real, not evidence it isn't. Genuine technical uncertainty is unaffected: the existing default-to-refuted stance for a finding that can't be verified from the actual code or plan is unchanged. Separately, the always-on acceptance-criteria dimension now requires a criterion the target itself defers, caveats, or ships with acknowledged gaps to be reported as a blocking finding rather than marked PASS, so a "ship with caveats" verdict can no longer read as a clean one. Both changes apply to every surface the canonical review powers — the `rdm-review` and `rdm-plan-review` skills (CLI and MCP) and the `rdm-wf-review-refute-fix` / `rdm-wf-dispatch-phase` / `rdm-wf-plan-review` workflow engines, both distributed (`rdm agent-config claude --skills`/`--plugin`) and local to this repo.

## [0.20.1] - 2026-08-26

### Changed

- The `rdm-do` skill now pre-approves the `EnterWorktree` tool in its `allowed-tools` frontmatter, so a Claude Code user invoking `/rdm-do` no longer has to manually approve the one-time worktree entry the skill's "Get into the roadmap's worktree" step performs. The skill body already told Claude to use `EnterWorktree({path})` on first entry from the main checkout, but the tool was absent from the grant list, so every first-phase run stopped on a permission prompt. Applies to `.claude/skills/rdm-do/SKILL.md`, the shipped CLI template (`rdm agent-config claude --skills` output), and the checked-in plugin tree at `plugins/rdm/skills/do/SKILL.md`. The MCP variant is unchanged — it enters the worktree by `cd`ing into the path `rdm worktree add` returns and never calls `EnterWorktree`.

- The `rdm-do` skill now also pre-approves `ExitWorktree` and knows how to use it. Step 4's **Mismatch** branch (switching from one roadmap's worktree to another's) previously prescribed a relaunch/`cd` as its only path, since `EnterWorktree`'s `path` form is rejected from inside another worktree unless the target lives under `.claude/worktrees/`. It now prefers an in-session path instead: `ExitWorktree({action: "keep"})` back to the session's original launch directory, then `rdm worktree add <slug> --project rdm`, then `EnterWorktree({path})` into the target roadmap's worktree — falling back to relaunch/`cd` only on a host without these tools, or when the current worktree was entered by launch/`cd` rather than by `EnterWorktree` (in which case `ExitWorktree` is a documented no-op). The prose also states two hard rules: always pass `action: "keep"`, never `"remove"` (worktree cleanup belongs to `rdm-land`), and that finalize never auto-exits the worktree — the session stays in place so the next phase of the same roadmap and a subsequent `rdm-land` run both continue in it. Applies to `.claude/skills/rdm-do/SKILL.md`, the shipped CLI template, and the checked-in plugin tree at `plugins/rdm/skills/do/SKILL.md`. The MCP variant is unaffected, as it never calls `EnterWorktree`/`ExitWorktree`.

## [0.20.0] - 2026-08-22
### Changed

- `rdm-wf-plan-review`'s `--roadmap <slug>` sweep (and the equivalent bare positional `<slug>`) now excludes any phase whose status is exactly `done` or `wont-fix` from the review/act/gate pipeline entirely, instead of dispatching the full find-refute-gate fleet against a phase with no implementation left to vet and then attempting to clear `needs-plan-review` on it. The exclusion is fail-open — a phase with a missing, blank, or unrecognized status stays in the fan-out — and reported, never silent: the run's result carries a new `skippedPhases: [{ stem, status }, …]` field, and the roadmap's aggregate summary and final log line both name the skip count and every excluded phase's stem and status. Explicitly targeting a single terminal phase (`--roadmap <slug> <phase>` or the positional `<slug> <phase>` form) is unaffected — it is always reviewed regardless of status; only the roadmap-wide sweep filters. This changes the workflow bytes `rdm agent-config claude --skills` and `--plugin` emit (and the checked-in `plugins/rdm/` tree) since it is local-only (`rdm-wf-plan-review.js` is not a distributed engine), and the `rdm-plan-review` skill's fully-interactive prose (local shim and both shipped CLI/MCP templates) documents the same behavior for a human driving the review without the `Workflow` tool.

- `rdm-wf-plan-review` no longer dispatches a `unit-of-work` finder agent for a task, a roadmap's own body, or an `--implementation-plan` target — only a phase unit has a unit-of-work contract to judge. Previously every review unit passed no `signals` into the shared review pipeline, so its selection step fail-opened to every dimension (correct for a caller with no target-type information, but wasteful here since the target type is always known) and a separate consumer-side filter silently discarded the resulting `unit-of-work` findings afterward. Each review unit now threads a minimal `signals: { targetType }` object, so the dimension is scoped out before its agent is ever dispatched — one fewer judgment-tier agent per task review, per `--implementation-plan` review, and per roadmap-body unit of a `--roadmap` sweep. A phase unit is unaffected: it still runs `unit-of-work` exactly as before. The consumer-side filter remains in place as a defense-in-depth backstop for any other caller of the pipeline that legitimately omits signals.

- `rdm-wf-plan-review`'s `needs-plan-review` gate now explains itself, can be handed back to the caller, and reports a failure loudly. Previously the gate delegated a bare "run these two commands" instruction to a sub-agent, carrying none of the review that justified it; safety classifiers read that as an unmotivated state mutation and blocked it three times across two runs, so units that legitimately reached `reviewed` — including one with zero blocking findings after a full review pass — kept their tag anyway, and the run reported `clearsPlanReviewTag: true` alongside `tagCleared: false` with nothing else to show for it. Three changes: (1) the gate instruction now carries an authorization preamble and the actual review evidence — that the operator invoked the plan-review skill and that clearing on `reviewed` is its specified behavior, which dimension finders ran, how many findings they produced, how many an independent refuter graded, that none survived at blocking severity, the exact tag list about to be written, and that the write touches one reversible metadata tag and no rdm status, code, or land-time directive — and that authorization is honest about its own coverage: refutation is deliberately not total (a non-gating `suggestion` is never refuted, a finding past the per-unit refutation budget passes through un-refuted, and a crashed refuter leaves its finding un-refuted, none of which prevents a `reviewed` outcome), so the gate reports the unit's real graded/un-graded split, itemised by severity and reason, instead of claiming blanket per-finding grading its own evidence block would contradict; (2) a new `gateMode: 'return'` argument makes the workflow compute the gate and write **nothing**, returning `gateAction.commands` (plus the sibling-preserved `remainingTags`) for the caller or a human to apply — the supported checkpoint when the session running the review also authored the plan. Because that hand-back *is* the escalation path, the deferral is self-describing without anyone reading the JSON: the unit's `summary` gains a lowercase `[gate deferred: … — apply: <update> && <commit>]` clause carrying the exact commands, and the run reports a `gateDeferredCount` kept strictly separate from the blocked count, so a surface that only ever logs the summary is already reporting what a human needs to run; (3) a gate that should have cleared the tag and did not is now impossible to miss — an uppercase `[GATE BLOCKED: … apply manually: <commands>]` clause on the unit's and the run's `summary`, a `gateBlocked` flag with a `blockedReason` distinguishing a refusal from a crashed agent, a dedicated log line on both failure paths, and a run-level `gateBlockedCount`. A healthy run's summary is byte-unchanged, and the uppercase/lowercase markers keep a genuine failure separable from a deliberate hand-off by a plain grep. The self-review decision this rests on, its boundary, the verbatim classifier blocks with per-claim rebuttals, and the explicit non-goal (no harness can prove a non-deterministic classifier stops blocking) are recorded in `docs/plan-review-gate-policy.md`.
- The `rdm-plan-review` skill (raw emission via `rdm agent-config claude --skills`, the plugin tree at `plugins/rdm/`, and both CLI/MCP templates) now states the gate's self-review policy in its generated spec: clearing `needs-plan-review` on a `reviewed` unit is specified behavior rather than self-approval and should be stated together with the review evidence that justifies it; a tag write that was supposed to land and did not must be reported at the top of the report with the exact command to run; and a session that authored the plan it is reviewing may decline to perform the write at all and hand back the commands instead. It is written in terms of the write itself, so it applies whether the surface runs the two `rdm` commands from its own prose or a driver runs them — the local-only `rdm-wf-plan-review` workflow's `gateMode`/`gateAction`/`gateBlocked`/`gateDeferred` argument and result fields are deliberately kept out of the shared spec, since a skill that has no such driver could not act on them. Regenerate rather than hand-patch.

### Fixed

- `rdm-wf-plan-review`'s roadmap-body review unit no longer silently accepts a fetch-status sentence (e.g. "Successfully fetched roadmap X with all phase details from the rdm project.") in place of the real roadmap body. A second, independent mechanical call re-reads the roadmap and reports a checkable property of its body (character length and first line), compared against what the primary fetch transcribed; a confirmed mismatch discards the whole fetch and fails closed exactly like the existing empty-body precedent, leaving `needs-plan-review` in place instead of silently reviewing nothing and clearing the tag. An unavailable or erroring check degrades to "proceed unverified" rather than "confirmed corruption," so a flaky verification call can never strand a legitimate roadmap. Scoped strictly to the agent-fetch roadmap path — never the caller-hoisted payload, and never phase/task targets. This does NOT collapse into the tag-clobber fix's `fetchTranscriptionOk` check below (the roadmap's own "Known overlap to resolve at phase 3" directive flagged the two as a possible duplicate): `fetchTranscriptionOk` is deliberately body-content-blind — it validates phase-stem convention and reserved tag tokens, never body text — so it cannot detect a fetch whose body is a fabricated status sentence with otherwise-clean stems/tags, which is exactly this defect's evidence; the two checks are independent, not redundant.
- `rdm-wf-plan-review`'s `needs-plan-review` gate now caches an item's real tags immediately once its fetch is accepted — before any further processing — and writes back only from that cache, never from the copy the review pipeline itself carries internally. Previously the tag write read the same in-flight value the review machinery used for its own unrelated purposes, so the write's safety depended entirely on that value never having been touched downstream; now the write path is structurally decoupled from the review pipeline's internals, closing the last literal gap in the tag-clobber hardening below.
- `rdm-wf-plan-review`'s standalone fetch step (used whenever a caller does not supply pre-fetched `rdm` data) now rejects a transcription that doesn't match `rdm`'s real phase-naming shape or that leaks the fetch prompt's own scaffolding words into tags, retrying once before giving up and leaving the item's tags and `needs-plan-review` status untouched — so a misbehaving fetch can no longer silently overwrite a roadmap's or task's real tags. This closes the last gap left after the fetch-stage redesign described below: that redesign already rejected an identity mismatch (wrong slug/stem/roadmap), but still trusted its own transcription unconditionally once the identity checks passed. The new check is body-content-blind (it only inspects phase stems and tags, never the free-text body), and both previously recorded production corruptions are replayed as regression tests to prove it discriminates them.
- `rdm-wf-plan-review`'s fetch stage no longer asks an LLM agent to interpret or compose `rdm`'s JSON output into a summary object — it now only transcribes the raw command output verbatim, with all parsing, field extraction, and identity validation performed deterministically by the workflow itself. Previously the `fetch:roadmap`/`fetch:<kind>` mechanical agents were handed a schema and asked to fill it in from `rdm ... show --format json`, which twice produced a schema-valid but completely fabricated response in production (run `wf_e3402021-0af`: the roadmap's real body/tags/phases were replaced with prompt-echoed junk and five of six phases vanished into a single phase entry stamped with the roadmap's own slug) — the plan-review gate then faithfully wrote that fabricated tag list over the item's real tags. The redesigned fetch agents transcribe raw stdout only; the workflow's own new identity/collision checks reject a payload whose phase stem collides with the roadmap's own slug, whose phase stems duplicate, or whose phase block disagrees on which roadmap it belongs to — failing the whole fetch closed (leaving `needs-plan-review` in place, no tag write) rather than accepting it. The roadmap fetch stays at exactly **one** mechanical agent call regardless of phase count — it does not become a per-phase fan-out. The standalone phase-identity check also accepts the documented `<roadmap-slug> [phase-number]` positional form: a bare phase number is matched against the fetched response's own numeric `phase` field (not string-compared against the CLI-resolved full stem), so a correctly-fetched numeric phase target is no longer rejected as a false identity mismatch.
- The `rdm-wf-review-refute-fix` and `rdm-wf-plan-review` Workflow engines now correctly handle a stringified `args` payload, which some LLM callers deliver despite the Workflow tool's object-args contract. Previously, `rdm-wf-review-refute-fix` silently fell back to its legacy survivors-only shape and discarded the AC table, outcome, and status whenever `args` arrived as a JSON string instead of an object; it now returns the full outcome in that case too. `rdm-wf-plan-review`'s optional mechanical/find/verify model hoist likewise failed to recognize a stringified `args`, running a redundant bootstrap agent on every such invocation instead of skipping it as designed.
- `rdm-land`'s step 4 ("Re-run the CI-equivalent checks") no longer hardcodes rdm's own `cargo fmt` / `cargo clippy` / `cargo nextest` commands as the operative gate. A downstream consumer in any other language had those three commands fail outright on every landing attempt, since failing checks are an abort condition. The step now instructs discovery of the consuming repo's actual checks, in order: its CI config, else `docs/principles.md`, else `CLAUDE.md`/`AGENTS.md`, keeping rdm's cargo triad only as an illustrative example; when none of those sources name any checks, the skill aborts and escalates rather than silently skipping the gate. Applied to `rdm-core/src/templates/skill-land-{cli,mcp}.md` (and therefore `rdm agent-config claude --skills`/`--plugin` output and the checked-in `plugins/rdm/skills/land/SKILL.md`), `.claude/skills/rdm-land/SKILL.md`, and `docs/landing.md`.
- `rdm-wf-plan-review` no longer misclassifies a roadmap, phase, or task with zero tags as a corrupted fetch. `rdm ... show --format json` omits the `tags` key entirely for an untagged item rather than printing `[]` (rdm-core's real, deliberate wire contract), but the validators guarding both the fetch-agent path and the caller-hoisted path required a real array and rejected the omission outright — an untagged item, or a roadmap with even one untagged phase, could never be plan-reviewed, since the roadmap-wide check is all-or-nothing across every phase. A missing `tags` key is now normalized to `[]` at all five validation sites (the caller-hoisted payload, its per-phase entries, and the standalone roadmap/phase/task extractors); a `tags` key that IS present but malformed (not a string array) is still rejected exactly as before. The `needs-plan-review` gate write is unaffected — an untagged item that reaches `reviewed` still gets a real `--tags ""` from the pre-fetch tag cache, never a fabricated list.

## [0.19.0] - 2026-08-10
### Added

- `rdm agent-config claude --skills --out <dir>` (both the plain CLI and `--mcp` variants) now also emits `.claude/agents/rdm-mechanical.md`, the mechanical-transcription custom-agent definition rdm's local-only Workflow engines resolve against via `agentType`. New public API `rdm_core::agent_config::generate_agents()`, a separate emission surface from `generate_skills`/`generate_workflows`. Neither distributed workflow engine references it yet, so this is a forward-looking emission: `scripts/verify-agent-config-distribution.sh` gained a byte-identity check plus a reference-resolution check (§ 3c) that resolves any future `agentType:` reference in an emitted workflow against the emitted agent-definition set, backed by planted-corruption self-tests. `.claude/agents/` is no longer local-only; see `CLAUDE.md`.
- `rdm-core` can now emit rdm's Claude Code lane as an installable **plugin tree** as well as the existing raw skills tree. New public API in `rdm_core::agent_config`: `generate_plugin_manifest()` returns the `.claude-plugin/plugin.json` manifest (name `rdm`, the crate version, a plugin-facing description, and an author block — deliberately no `workflows` key, since that directory is convention-discovered and the key would replace rather than add to the default), `generate_plugin_skills()` / `generate_plugin_workflows()` return the eleven skills under `skills/<name>/SKILL.md` and the two engines under `workflows/` at the plugin root, and `generate_plugin_files()` returns the whole 14-file tree as `PluginFile { relative_path, content }`. In plugin mode skill names drop their `rdm-` prefix (the `rdm:` plugin namespace already disambiguates, so the skill is `rdm:roadmap` rather than `rdm:rdm-roadmap`), skill bodies name engines by the namespaced form `rdm:rdm-wf-dispatch-phase` instead of by a `.claude/workflows/…` path — **every** mention is rewritten, including the operative "invoke the Workflow with `{ … }`" instructions in the `autopilot`, `dispatch-phase` and `do` shims, so a plugin-installed shim always dispatches through the namespace the runtime actually resolves (engines this distribution does not ship, such as `rdm-wf-estimate`, stay un-namespaced, since namespacing them would name a plugin entry that does not exist) — and every shim that needs an rdm binary carries a note on how to resolve one from a plugin install (`--rdm-bin`, then `RDM_BIN`, then `PATH`, with an actionable error if none resolves). Engine names keep their `rdm-wf-` prefix, which is what keeps the emitted skill names and engine names disjoint. Existing `rdm agent-config claude --skills` output is byte-for-byte unchanged.
- `rdm agent-config claude --plugin --out <dir>` now writes that plugin tree to disk: the manifest at `<dir>/.claude-plugin/plugin.json`, the eleven skills at `<dir>/skills/<name>/SKILL.md`, and the two workflow engines at `<dir>/workflows/<name>.js`. `--plugin` is its own emission mode alongside `--skills` (mutually exclusive with it — pass one or the other, not both) and is Claude-only. It requires `--out`; it cannot be combined with `--user`, since a plugin reaches `~/.claude` by installation (`claude plugin marketplace add` / `claude plugin install`), not by rdm writing into the user directory. `--skills --user` is unaffected and keeps working exactly as before. A new hermetic harness, `scripts/verify-plugin-distribution.sh`, checks the emitted plugin tree's self-consistency (layout, naming, and that every skill's workflow reference resolves to a real file in the same emitted tree).
- rdm now ships as an **installable Claude Code plugin**. The generated plugin tree is checked into the repo at `plugins/rdm/` and a marketplace entry at `.claude-plugin/marketplace.json` points at it, so a consumer can run `claude plugin marketplace add edpaget/rdm` followed by `claude plugin install rdm@rdm` and get the eleven `rdm:<name>` skills plus the two `rdm:rdm-wf-<engine>` Workflow engines — offline, with no auth. `plugins/rdm/` is generator output and must never be hand-edited; regenerate it with `env -u RDM_ROOT -u RDM_PROJECT cargo run -q -- agent-config claude --plugin --out plugins/rdm` (the omitted `--project` is load-bearing — it keeps the generic `--project <PROJECT>` placeholder in every skill body instead of baking in rdm's own project name). Two new harnesses keep it honest. `scripts/verify-plugin-install.sh` is hermetic pure POSIX shell with no external-CLI dependency and runs in CI: it gates the checked-in tree against generator output behind a **version-normalized** drift diff — the manifest `version` is normalized on both sides so a release-time crate bump can never red-light `main`, while a separate assertion against *freshly generated* output is what catches a stale manifest — plus marketplace shape and `source` resolution (with a non-empty-entry floor, since `claude plugin validate --strict` false-passes a dangling `source`), workflow byte-identity, and the eleven-skill inventory with frontmatter validity, each behind a planted-corruption self-test. `scripts/observe-plugin-install.sh` is the developer-run half — it needs the `claude` CLI, so it deliberately sits outside CI's `scripts/verify-*.sh` glob — and performs a real offline `validate --strict` → `marketplace add` → `install rdm@rdm` into an isolated `CLAUDE_CONFIG_DIR`, asserting the installed skill inventory and workflow bytes on the filesystem and proving the invoking user's real `~/.claude` is untouched; it exits `2` with a NOTICE (distinct from both pass and fail) when `claude` is absent. Consumer install instructions, the regeneration rules, and a full run transcript live in `docs/plugin-distribution.md`.

- Documented how to recover a crashed autonomous dispatch: the `Workflow` tool's `resumeFromRunId` relaunch mechanism (relaunch with `{ scriptPath, resumeFromRunId }` and unchanged `agent()` calls replay from cache) is now covered across all seven surfaces that touch the lane — `docs/autonomous-loop.md` (new "Recovering a crashed run" section, with the measured `wf_974e3812-817` evidence), the local `rdm-autopilot`/`rdm-dispatch-phase` skill shims, and the shipped `rdm-autopilot`/`rdm-dispatch-phase` CLI and MCP templates — including the same-session-only limit, the stop-the-prior-run-first requirement, the empty-cached-result caveat, and the conservative-prefix caveat. `docs/workflow-schemas.md` also gained a "Determinism" subsection explaining that the `Date.now()`/`Math.random()` ban across `.claude/workflows/*.js` exists for pipeline determinism generally (resume-cache validity is one consequence of it, not the sole reason), and corrected two spots that mischaracterized the ban as runtime-enforced when it is a repo convention. All four shipped CLI/MCP templates now name the exact file being relaunched (`.claude/workflows/rdm-wf-dispatch-phase.js`) inline in their recovery section rather than only the bare engine name — the two `autopilot` templates previously omitted the filename entirely. `docs/plugin-distribution.md` gained a note explaining why these four shipped templates deliberately never spell out `Workflow({ scriptPath: … })` as literal JS or the bare word `scriptPath` anywhere (a real one leaks unrewritten into the plugin-mode skill body and would trip `plugin_skill_bodies_use_namespaced_workflow_refs`'s zero-`scriptPath` assertion) — narrating a `.js` filename in prose is intentional, not an omission, and the two hand-maintained local skill copies (outside the plugin pipeline) are free to use the literal form.

### Changed

- The review pipeline now reports **which review dimensions actually ran**. Every review result carries a new `coverage` field (`total`/`selected`/`ran`/`failed`/`retried`/`complete`/`acDimensionRan`/`acTableAbsent`, all arrays in dimension-selection order), consumers surface it as `reviewCoverage` on the OUTCOME, and when any round lost a dimension the summary gains a ` [review coverage: N/M dimensions ran; failed: a,b]` clause — plus `; NO AC TABLE` when the acceptance-criteria dimension is the one that died. A review in which every dimension ran is completely unchanged: the clause is empty, so healthy summaries are byte-for-byte what they were. Because the recorded reason is derived from the summary, a parked or escalated unit whose review was incomplete now says so in the `rdm review blocked` queue. Non-participation is recorded, never gated on — a transient API failure cannot stall the autonomous lane, it just can no longer pass unnoticed. This changes the skill and workflow bytes `rdm agent-config claude --skills` and `--plugin` emit (and the checked-in `plugins/rdm/` tree), so regenerate rather than hand-patch.
- The implementation-agent prompt `rdm-wf-dispatch-phase.js` sends on a phase/task dispatch no longer claims the agent is seeded with "ONLY" the item body and approved plan — on a rework or stalled-agent retry that was false, and it never told the agent the worktree might already hold prior commits. The prompt now instructs the agent to run `git log main..HEAD` / `git diff main...HEAD` right after entering the worktree and read what's already there, explicitly distinguishing two cases: earlier phases of the same roadmap already on the shared branch (context to build on, not this item's own work) versus a prior or stalled attempt at this same item — deciding which by comparing the commits against the approved plan, never by their mere presence. Applied identically to both `.claude/workflows/rdm-wf-dispatch-phase.js` and the shipped `rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js`, so every downstream `rdm agent-config claude --skills`/`--plugin` emission (including the checked-in `plugins/rdm/` tree) picks it up. The end-of-prompt `IMPLEMENT_RESULT` diff-report instructions are unchanged.
- Autopilot-driven phase dispatch on the **CLI lane** no longer spends a session-model (Opus) subagent fetching phase metadata on the success path of a dispatched phase: the `rdm-autopilot` skill now fetches it directly, itself, via Bash (`rdm phase show ...` plus the five `rdm model resolve <step>` calls, mirroring `rdm-wf-dispatch-phase`'s own Stage-0 fetch prompt) and forwards it as `phaseMeta` to the `rdm-wf-dispatch-phase` Workflow, which already accepted a hoisted `phaseMeta` and skips its own fetch agent when one is supplied complete. This cuts one Opus-tier subagent per dispatched phase in the common case, the only remaining unsized call in the autonomous lane; it is not unconditional — if the skill's own pre-fetch fails partway (any one of the six commands errors or returns nothing), it deliberately forwards no `phaseMeta` rather than a partial one, and that one dispatch falls back to the same unsized Stage-0 fetch as before, preserving the guarantee that a cold direct `Workflow` invocation still resolves models on its own. The **MCP lane is unaffected by design** — there is no MCP model-resolve tool to run the procedure with, matching the existing `rdm-dispatch-phase`/`rdm-do --auto` MCP precedent — and continues using the in-workflow fetch unconditionally. Consumers of the shipped `rdm-autopilot` skill template (raw skill emission via `rdm agent-config claude --skills`, and the plugin tree at `plugins/rdm/`) should regenerate rather than hand-patch.

### Fixed

- A review whose dimension finder died is no longer reported as a clean review. Previously a finder that returned nothing was dropped silently: a review that ran 3 of 7 dimensions was indistinguishable from one that ran all 7 and found little, and a dead acceptance-criteria finder produced no AC table — which read exactly like a table with no failing criteria, so the dimension that owns the acceptance-criteria contract could fail to run and still yield a clean verdict. A finder that returns nothing is now **retried once**; a dimension that still fails is recorded as non-participating and named in the review summary, and an absent AC table is recorded distinctly from a clean one. If every dimension fails, the review throws rather than reporting a clean result.
- The guard that refuses to report a clean review when every dimension finder failed now fires **whether or not an explicit model was passed**. It was previously conditioned on a model being supplied, which made it — and the companion null-to-error conversion — completely inert in plan review, the one caller that passes no models at all. A total review failure there would have been reported as a clean plan review, and the plan-review gate would then have cleared the `needs-plan-review` tag off a review that never ran. The recognisable "check the `[models]` tier bindings" message is preserved for the misconfigured-model case.
- `README.md`'s installation directions now cover plugin-based distribution. The Installation section gained a "Wire it into your coding assistant" step that leads with `claude plugin marketplace add edpaget/rdm` / `claude plugin install rdm@rdm` as the recommended path, so a reader no longer has to reach the "AI Agent Integration" section further down to learn the plugin exists; the previous "ask your assistant to run `rdm --help`" advice is retained for other assistants and for consumers who cannot use the marketplace. The pinned-release example was refreshed from the long-stale `v0.6.2` to `v0.18.2`.
- Corrected three stale claims about the `rdmBin` runtime argument left behind by the 0.18.2 change that made it optional. `README.md` described it as **required** and told the reader they must "supply one of these options to resolve the binary"; `docs/plugin-distribution.md` § "Decision 4: Runtime Arguments Delivery" still declared it `REQUIRED` with "no ambient fallback; an absent key throws before the first `agent()` call", said the engines "continue to fail fast if `rdmBin` is missing", and carried a decision-table row reading "Required". All now state the shipped contract: omitting `rdmBin` defaults to a plain `rdm` on `PATH`, an explicit value is used verbatim, and only a present-but-non-string value is refused. The `--rdm-bin` flag and `RDM_BIN` environment variable are described as overrides for a binary that is not on `PATH` rather than as setup every consumer must perform, and Decision 4 now defers to `docs/workflow-schemas.md` § "Environment args" as the canonical contract instead of restating it.
- `README.md` no longer tells readers that the skill shims emitted by `rdm agent-config claude --skills --out <dir>` "carry rdm's own dogfood values, so adjust those two arguments for the target repo". They do not: the emitted shims bake in the `--project` supplied at emission time and invoke a bare `rdm`, so no post-emission editing is required. The raw-emission fallback paragraph now says what the fallback actually costs a consumer — 13 loose file copies with no collision protection or namespace prefixing, self-managed workflow discovery, and an assumption that rdm is on `PATH`, since unlike the plugin shims the raw shims carry no binary-resolution section.
- `rdm-wf-plan-review`'s finders and refuters no longer silently inherit the invoking session's model. `.claude/workflows/lib/plan-review.mjs` (and the byte-identical `rdm-wf-plan-review.js` copy) now thread the configured `review-find`/`review-verify` model ids into both `runPlanReview({...})` call sites, resolved by the file's existing `model:mechanical` bootstrap agent alongside the mechanical id — closing the last consumer of the shared review pipeline that had this same session-model-inheritance leak (the sibling `rdm-wf-dispatch-phase` already threaded it; see the review-fleet entry above). The refuter *tier* itself (`keep-opus`, `docs/refuter-model-tiering.md`) is unchanged — this only fixes that the binding was previously absent.

## [0.18.2] - 2026-08-03
### Changed

- The `rdmBin` argument accepted by the `rdm-wf-dispatch-phase`, `rdm-wf-review-refute-fix` and `rdm-wf-estimate` Workflow engines is now **optional and defaults to a plain `rdm` on `PATH`**. Previously it was required and fail-closed: a caller that omitted it got an error before the first agent dispatch. That made the plugin-installed lane unusable out of the box, since a consumer who installs the `rdm` plugin has no repo-local build path to supply and `PATH` is the right answer for essentially every consumer. You can now install the plugin and dispatch a phase or task without configuring anything. An explicitly passed path still wins verbatim, the explicit `"rdm"` sentinel still works unchanged, and a present-but-non-string value is still rejected rather than silently falling back to `PATH`. The skills resolve the binary in the order `--rdm-bin` → `$RDM_BIN` → `rdm`, and the `rdm-autopilot` skill no longer refuses to start when `--rdm-bin` is absent.

- Documented rdm's **recommended distribution path** for downstream consumers: plugin marketplace installation (`claude plugin marketplace add edpaget/rdm` then `claude plugin install rdm@rdm`) over raw skills emission (`rdm agent-config claude --skills --out <dir>`). The plugin provides automatic namespace prefixing, collision protection, and workflow discovery; raw emission is retained as a fallback. Updated `README.md` § "AI Agent Integration" and `CLAUDE.md` with distribution guidance, and reconciled stale workflow-count references across documentation (workflow-schemas.md, mechanical-agent-inventory.md, CLAUDE.md) from "three distributed workflows" to the correct count of two (rdm-wf-dispatch-phase and rdm-wf-review-refute-fix). Corrected the documented rdmBin resolution mechanism to reflect the actual implementation: resolve via `--rdm-bin` flag first, then `RDM_BIN` environment variable, then `PATH` lookup (removed non-existent "standard installation locations" fallback). Added `docs/plugin-distribution.md` § "Which copy runs?" to explain the three distribution surfaces: the authoritative templates in `rdm-core/src/templates/`, the emitted plugin tree consumers install, and this repo's local `.claude/` lane, with precise categorization of which `.claude/workflows/` files are generated (with hand-copied trailing driver blocks) versus hand-maintained byte-checked blocks versus pure hand-maintained copies. Documented why this repo never installs its own plugin (development-build requirement and drift-gate root for generated artifacts).

## [0.18.1] - 2026-08-03
### Added

- The autonomous lane `rdm agent-config claude --skills --out <dir>` emits is now **verified to work in an arbitrary consumer repo**, not merely verified to match this repo's own copies byte-for-byte. `scripts/verify-agent-config-distribution.sh` stands up a hermetic non-rdm, **non-Rust** fixture repo — a Python/TypeScript source tree with real feature branches, its own `rdm init`-seeded plan repo, its own project name, and its own rdm executable path — emits the lane into it, extracts importable modules from the **emitted** engine scripts (with an inverse transform proving each copy is byte-identical to the emitted file, and a proof that the untransformed file genuinely does not import), and then executes their pipeline logic there: every review signal fires on the fixture's own diff and all seven code dimensions are selected (against a docs-only control that selects only the always-on pair), every rdm command the engines build names the fixture's binary and honors the project-flag allow-list with zero `./target/debug/rdm` or `--project rdm`, and three of those built commands are really run against the fixture plan repo and must exit 0 with the expected JSON shape. Four planted corruptions in the emitted bytes prove none of it is vacuous. The harness now requires `node` (on `PATH` or via `mise exec node --`).
- `rdm agent-config claude --skills` now cleans up superseded `.claude/workflows/` files left over from an earlier emission of the same lane, once it recognizes a file's exact prior content: it reports `Removed <path>` for a file whose bytes match a known previously-emitted fingerprint, and `Skipped <path> (content modified since emission; left in place)` for a same-named file it does not recognize (user-edited, or from a version of rdm it doesn't know about) — the latter is never deleted. A removal failure (e.g. a permission error) is reported as `Failed to remove <path>: <error>` and never fails the overall emit. This only runs for Claude Code project output (`--out`, not `--user`; Pi has no `.claude/workflows` at all). The shipped superseded-file table now carries the two engines renamed by the `rdm-wf-` prefix change below, plus the retired `autopilot.js` orphan.
- The `plan`-mode review's always-on dimension set is now **correctly documented as three** — `coherence`, `architectural-fit` and `restraint`. `restraint` carries no `when` predicate and has always run on every plan review; `CLAUDE.md`, `docs/workflow-schemas.md`'s dimension table, and the shipped plan-review skill templates' finding template (`concern: <coherence|architectural-fit|restraint|unit-of-work>`) all said otherwise. No behavior changed — only the description of it.
- The shipped **code-review** skill templates now state **why `ac` and `correctness` are not merged into one always-on finder**: `ac` is the only dimension resolving the AC-review schema rather than the findings schema, and its per-criterion table is the structured side-channel the verdict consumes directly — a channel that never reads a finding's severity, is never refuted, and never consumes refutation budget. Folding it into a shared findings stream would route the acceptance-criteria contract through exactly the path it was kept out of. Rendered into the code-mode skills only; the long-form rationale lives in `docs/workflow-schemas.md`.
- A new on-demand **finder-collapse harness** answers, on evidence, whether `plan` mode's three always-on finders can be collapsed into ONE agent holding three lenses. `scripts/mine-plan-finder-corpus.mjs` recovers real plan REVIEW UNITS verbatim from the full-fidelity `subagents/workflows/<runId>/agent-*.jsonl` transcripts — plan-mode prompts interpolate the plan document inline, so both the document and the three-finder output are replayable — keyed on the same `(runId, unitIdent)` boundary `docs/token-baseline.json` § `refuterFanout` documents, and never adjudicates. `scripts/lib/finder-collapse.mjs` + `scripts/run-finder-collapse.mjs` build both arms (arm A through the REAL exported `findPrompt` over the REAL always-on `DIMENSIONS.plan` entries — never a copy; arm B through a collapsed three-lens prompt that lives in the instrument so a no-ship leaves the lane byte-unchanged), dispatch them with replicates via `claude -p`, and score per-lens finding counts, per-lens severity distribution, adjudicated material recall, `concern` attribution validity and per-class token totals — never blending a rate across lenses (enforced by a recursive key assertion). Modes that spend nothing: `--dry-run`, `--dispatch-stub`, `--score`, `--audit`. The miner's accounting identity — every plan-finder record is either recovered as an always-on lens observation or counted in exactly one skip bucket, since an unrecovered finder is an unknown and an unknown must never contribute to a rate — holds under `--limit` too: a unit past the limit is classified into a single `beyond-limit` bucket before any other test, rather than the boundary unit's records and every later unit being abandoned uncounted. `scripts/verify-finder-collapse.sh` gates all of it hermetically with planted-mutation self-tests, including one that reverts that bucket to a bare `break` and must break the under-truncation identity.
- `docs/finder-collapse.md` records the resulting decision — **`no-ship`, and the review pipeline is unchanged** — with its six-criterion rule pre-registered in an earlier commit than the run it judges. Over 8 real review units x 2 replicates x 2 arms (64 paid dispatches, `opus` both arms), the collapsed finder lost adjudicated material findings in **all three lenses independently**: `coherence` -4 (18.2 %), `architectural-fit` -11 (73.3 %), `restraint` -10 (58.8 %), against a tolerance of one finding, and 9 of arm A's 19 material `blocking` findings. Both arms' findings were equally material under hand adjudication (83/83 rows, complete coverage) — the discriminator is **recall, not precision**. Attribution was perfect (57/57 arm-B findings carried a valid lens key) and tokens fell 55.1 %, and neither of those can carry a ship: the rule is an AND over all six criteria and states that the token criterion alone is never a ship. The scorer additionally refuses to let criteria 2-4 pass while adjudication coverage is incomplete, so an empty adjudication yields `no-ship` rather than a vacuous pass. Machine-readable figures land in `docs/token-baseline.json` § `planFinderCollapse`, audited corpus-free by `node scripts/run-finder-collapse.mjs --audit docs/token-baseline.json`, and a decision/pipeline XOR in the harness keeps a half-landed merged dimension from ever coexisting with that figure.

- The refuter-agreement harness now also A/Bs refuter **shape** — one refuter per gating finding versus one per dimension over that review unit's gating findings — alongside the model question it already answered, and `docs/refuter-batching.md` records the outcome: **`no-measurement`, and the review pipeline is unchanged.** A batched dispatch is one *review unit's* findings for one dimension (`buildReviewPipeline` runs once per unit), so the corpus is grouped by `runId | unitIdent | mode | dim.key`; under that key the committed 56-item corpus yields 35 groupable items in 29 groups — 24 singletons, 4 pairs, 1 triple — i.e. exactly **1 qualifying (size >= 3) group / 3 items against pre-registered floors of 6 groups / 18 items**. A batched arm built from that would be byte-for-byte a per-finding arm across most of its items, so the anchoring effect the experiment exists to detect would be unobservable; no paid A/B was run, and the phase reports a no-measurement outcome rather than a pass. (The earlier `(runId, mode, dim.key)` framing reported four size->=3 groups; those figures are void — two are an artifact of the 12 `constructed` items collapsing into one pseudo-run, and the only non-constructed triple spans two different review units. Both histograms are recorded, the naive one under `supersededNaiveKey`.) New zero-spend command: `node scripts/run-refuter-agreement.mjs --batch-power`.
- New refuter-agreement flags supporting the above, all documented in `--help`: `run-refuter-agreement.mjs` gains `--batch-power`, `--min-batch-group <n>`, `--shape per-finding|batched|both`, `--allow-underpowered` (which stamps a `NO MEASUREMENT` banner and suppresses any decision line, so an underpowered arm can never be mistaken for a pass) and `--audit-section refuterModelTiering|refuterBatching`; `mine-refuter-corpus.mjs` gains `--min-group-size <n>` (emit only candidates whose unit-scoped group has at least n members, so a costly hand-adjudication pass buys only power-adding items) and `--exclude-corpus <path>` (skip already-adjudicated ids so a re-mine appends rather than duplicates), and newly mined items carry `provenance.agentIndex` so a REWORK re-review can be split off a first-round batch instead of silently inflating its size. The scorer reports the two shapes as independent arm buckets with `cost.dispatches` counted by unique dispatch id, `cost.meanTokensPerDispatch` and `cost.meanTokensPerGradedFinding`, plus a new `## ANCHORING` block (`allSameVerdictShare` and refutation rate by position within a batch) computed over qualifying groups only — with false negatives and false positives still on their structurally different denominators and still never blended. Machine-readable figures land in `docs/token-baseline.json` § `refuterBatching`, audited corpus-free by `node scripts/run-refuter-agreement.mjs --audit docs/token-baseline.json --audit-section refuterBatching`.
- `docs/workflow-vs-prose-boundary.md` records where the autonomous lane's workflow-vs-prose boundary goes, so a new script can be classified without re-litigating the decision. It states the rule as five criteria a unit must meet to stay a Workflow script (fan-out-shaped, mechanism rather than policy, headless, deterministic/resumable, hermetically gatable) plus the anti-criterion that decides the autopilot drive loop — a low-iteration sequential driver with negligible fan-out buys nothing from the workflow runtime and pays for it on the part that changes most. It records the two runtime constraints that shape every such answer (the runtime cannot `import`/`require`, and `workflow()` nesting is capped at one level), and gives a disposition with a stated reason for all eight `.claude/workflows/*.js` scripts (`autopilot.js` moves to prose; the other seven stay, `spike-agent-type.js` as an explicit exempt spike artifact) on an axis kept separate from distribution — only three of the eight are emitted downstream, and the five local-only ones all reference an `agentType` a downstream tree has no definition for. It notes the known cost — retiring the JS loop loses `scripts/verify-workflow-autopilot.sh`'s hermetic coverage, which is a first-class phase rather than a cleanup afterthought — and records the two arguments that are deliberately NOT part of the decision: this is not `program-driven-orchestration` (deferred, not superseded, because a headless `claude -p` / Agent SDK orchestrator bills against API usage and is incompatible with Claude subscription billing), and it is not justified by "liveness" (refuted by run `wf_52b569c9-9b4`'s transcripts, where the stalled implementers were actively running rather than backgrounded and had wrongly concluded their work was committed). `CLAUDE.md`'s two-surfaces decision rule now cites the criteria and points at the doc, with autopilot appearing only as an in-flight migration note.
- A new on-demand **refuter-agreement harness** decides, on evidence, whether the autonomous lane's refuters must stay on Opus. `scripts/mine-refuter-corpus.mjs` recovers real historical findings **verbatim** from the full-fidelity `subagents/workflows/<runId>/agent-*.jsonl` transcripts (the 401-character truncation applies only to the `wf_*.json` sidecar previews, not the transcripts) and always emits `groundTruth: null`, because scoring Opus against its own past verdicts would be circular. `scripts/run-refuter-agreement.mjs` regenerates every prompt through the REAL `refutePrompt`, sha-verifies it against the mined original, and dispatches replicates per tier via `claude -p` — with `--dry-run`, `--dispatch-stub`, `--score-only` and `--audit` modes that spend nothing. `scripts/lib/refuter-agreement.mjs` holds the corpus schema and a scorer that reports **false-negative and false-positive rates separately**, over structurally different denominators, with no blended-accuracy field anywhere (enforced by a recursive key assertion) and per-tier token classes plus tool-call means on the same report rows, so the already-measured "cheaper model, same tokens" effect stays visible.
- A checked-in, adjudicated finding corpus at `tests/fixtures/refuter-agreement/corpus.jsonl`: 56 items, 78.6 % mined from real production runs and 21.4 % constructed only to top up under-covered classes, each carrying provenance, a ground-truth class from a closed set, an `authoritative`-vs-`judgement-call` authority flag, and the pinned commit it was adjudicated against. The `mechanically-true-not-a-defect` class — where the two tiers actually diverged — is deliberately over-represented at 42.9 %. `scripts/verify-refuter-agreement.sh` gates all of it hermetically, never dispatching a paid agent, with planted-mutation self-tests. That includes the **real paid-dispatch path** that the dry-run and stub modes bypass but the recorded decision was computed from: `parseClaudeResult` is driven as a pure function over synthetic `claude -p --output-format json` bodies (last-StructuredOutput-wins, bare-JSON and prose/fence-wrapped `result` strings, a missing `usage` object, the `num_tool_uses` fallback, and a non-boolean `refuted` that must bucket as `ungraded` instead of coercing to `false` and inflating the false-positive rate), and `claudeDispatch`'s success / non-zero-exit / non-JSON-body / missing-binary branches run against PATH-shadowed fake `claude` binaries. The miner is covered to the same standard, because its skip branches decide how much history reaches the corpus at all: a hermetic sidecar fixture exercises all six degradation paths (`no-transcript`, `no-prompt`, `unparseable-finding`, `unrecoverable-mode`, `unrecoverable-dim`, `no-verdict`) with an accounting identity asserting `recovered + skipped == refuter records` so none can become a silent drop, plus its full CLI surface (`--severity` singly and as a comma-set, `--until` in both directions, `--limit`, `--out`, `--help`, and every argument-validation error, each of which must now be an actionable named message rather than a raw stack trace).
- `docs/refuter-model-tiering.md` records the resulting decision — **keep Opus, change nothing** — with its supporting numbers and its decision rule stated before those numbers. On the recorded run — a stratified 8-item × 2-tier × 2-replicate subset of the corpus, 16 trials per tier, not a corpus-wide estimate — Sonnet's authoritative-only false-negative rate was 2/5 (40.0 %) against Opus's 1/6 (16.7 %) (every Sonnet false negative landing on a still-true `real-defect`), it was worse on the divergence class, and it spent **89.2 % more tokens per trial** with more tool calls — so the cheaper tier is neither safer nor cheaper in volume. No model binding changed, so no `verify-workflow-*.sh` criterion needed updating; `scripts/verify-workflow-review.sh` gains a pointer comment near §5b-mechanical and the new gate enforces that as an XOR. The doc carries per-class and authoritative-only breakdowns, self-consistency flip rates, and an explicit `## Limitations` section. It also answers the long-contested `plan-review.js` model-omission question: the omission is an **oversight**, not policy — `f4e89d7` and `scripts/verify-workflow-review.sh` §5b-mechanical govern only the MECHANICAL pin, while the sibling `dispatch-phase.js`, the `[models]` config policy, and `CHANGELOG.md`'s own "closing the session-model-inheritance leak" entry all point the other way. Machine-readable figures land in `docs/token-baseline.json` § `refuterModelTiering`, audited corpus-free by `node scripts/run-refuter-agreement.mjs --audit docs/token-baseline.json`.
- A new dev tool, `scripts/measure-lane-tokens.mjs` (over `scripts/lib/token-report.mjs`), measures token usage across Claude Code Workflow lane runs. It locates every `wf_*.json` session sidecar under a `--root` (default `~/.claude/projects`, searching every project-slug directory including `--worktrees-`-named ones), joins each run's agents with their `agent-*.jsonl` transcripts (deduping usage by `requestId`), and reports totals broken out by token class (output / uncached input / cache write / cache read) grouped by agent class, full label, model, and workflow — plus an explicit, never-reconciled discrepancy line between the sidecar's own `totalTokens` and the deduped sum. Invoke it with `--since <iso-date>`, `--workflow <name>` (repeatable, OR'd), and `--format text|json`. It is the measurement tool later phases of the `workflow-token-reduction` roadmap use to substantiate their token-saving claims. Stdlib-only Node, no packages; hermetic regression in `scripts/verify-token-report.sh`.
- `scripts/measure-lane-tokens.mjs` now also reports a **per-agent-class first-request floor** (`floorByAgentClass`): the n/min/p10/median/mean of each measured agent's first transcript request only (uncached input + cache write + cache read, before any tool use) — the same quantity `docs/token-baseline.json`'s whole-corpus `agentContextFloor.measuredFloor` is defined over, now broken out per agent class instead of only as a single global median. `cached`/sidecar-only-fallback records (no per-class split recoverable) are excluded from the floor rather than counted as zero, and a class with no eligible records is omitted from the output rather than reported with `n: 0`. Surfaced in both `--format json` (a new `floorByAgentClass` key) and `--format text` (a new "-- Per-agent-class first-request floor --" section). `scripts/verify-token-report.sh` gained fixture-backed assertions and two new planted-mutation self-tests covering it.
- `docs/token-baseline.md` and its machine-readable twin `docs/token-baseline.json` commit a measured "before" snapshot of the six autonomous lanes' (autopilot, dispatch-phase, plan-review, backlog, estimate, document) real token spend, broken down per agent class and per model with cache reads included. It reconciles the corrected per-class figures against the roadmap body's original sidecar-`tokens`-field survey (the review-vs-implementation ratio narrows from ~11:1 to ~1.67:1 once cache reads are counted), reports a directly-measured ~38.8k-token per-agent context floor attributed between `CLAUDE.md` and tool schemas/system prompt, and documents the confounds (roadmap size, phase difficulty, rework rounds) that make raw per-run totals non-comparable across lane runs. Later phases of the `workflow-token-reduction` roadmap diff their savings claims against this baseline.
- `docs/token-baseline.json` gained a `floorByAgentClass` addendum: the same per-agent-class first-request floor now surfaced by `scripts/measure-lane-tokens.mjs`, regenerated over the existing on-disk corpus (44 runs / 2,110 records — no fresh dispatch) with every mechanical agent class (`fetch`, `stamp`, `model`, `diff`, `gate`, `advance`, `park`) plus the mixed mechanical/judgment `act` and `estimate` classes represented, so the roadmap's later mechanical-elimination phase has real, correctly-caveated per-class "before" evidence instead of only a whole-corpus median. The existing `byAgentClass`/`runSet`/`agentContextFloor`/`totalsDiscrepancy`/`warnings` sections are unchanged; `methodology.ac5RegenerabilityStatus`/`regenerateCommandScope` are updated to note `floorByAgentClass` is now CLI-reproducible while the whole-corpus `agentContextFloor` still is not.
- A companion dev tool, `scripts/measure-hoist-delta.mjs`, measures the
  mechanical-subagent reduction directly rather than by applying an elimination
  rule on paper: it executes the real, post-change `dispatch-phase` driver under
  a recording fake `agent` — once with a pre-change caller's arguments and once
  with the arguments the post-change skill shim passes — and counts the
  subagents each run actually spawns, then prices them using
  `docs/token-baseline.json`'s own measured per-class figures. It reports both a
  raw and a fresh (ex-cache-read) token column, since cache reads dominate the
  raw totals and are the cheapest token there is. Its `--check <doc>` mode
  asserts the figures it computes appear verbatim in
  `docs/mechanical-agent-inventory.md`, so that document cannot drift into a
  stale hand-transcription; `scripts/verify-workflow-dispatch.sh` section 8 runs
  that check in CI with planted-mutation self-tests. Stdlib-only Node, no
  packages.
- A new dev tool, `scripts/measure-refuter-severity.mjs` (over the same
  `scripts/lib/token-report.mjs`), breaks refuter token spend out by the
  **severity of the finding each refuter graded** — the dimension the single
  `refute` agent-class bucket cannot see. It recovers each finding from the
  refuter's own transcript (brace-matching the embedded finding JSON, so a
  review target that itself contains braces still parses) and its verdict from
  the forced `StructuredOutput` call, then reports agent count, verdict tally,
  refutation rate, and all four token classes per severity, plus the projected
  drop from skipping non-gating refutation. `--until` pins the measurement
  window so a committed figure cannot be silently re-baselined by a later lane
  run; `--check <doc>` recomputes over the corpus and asserts a document's
  recorded figures match; `--audit <doc>` checks a document's numbers for
  internal consistency without reading any sidecars, so the committed figures
  are gated on any machine. `scripts/verify-token-report.sh` section 6 gates it
  against a hermetic fixture with two planted-mutation self-tests. The measured
  result is recorded in `docs/token-baseline.json` §
  `nonGatingRefutationSkip` and summarized in `docs/token-baseline.md`.
- `scripts/measure-refuter-severity.mjs` gained two more descriptive
  distributions — **review fanout** — over the same corpus: findings-per-finder
  (n/min/p50/p90/max, split by mode and dimension, read from each finder's own
  `StructuredOutput` output rather than inferred from refuter counts, since a
  `suggestion` finding is never dispatched to a refuter at all) and
  refuters-dispatched-per-review-unit (n/min/p50/p90/max plus a recovery
  rate). Both a refuter's dimension and its review-unit identity are parsed
  from the same prompt header line the severity extractor already reads
  (`A prior reviewer raised this <dim> finding against <target>:`) via a new
  `extractRefuterContext()`, never from the refuter's own
  `refute:<mode>:(f.id|dim.key:idx)` label, which a finder-supplied `f.id`
  routinely displaces the dimension out of. The review-unit boundary is
  deliberately **not** `phaseTitle`/`phaseIndex` — those are the workflow's
  own declared pipeline stages and collapse an entire plan-review run's
  distinct review units into one bucket (measured directly: 96 refuters at one
  `phaseIndex` across 9 real units on a reference run) — so the unit key comes
  exclusively from the target embedded in each refuter's own prompt, with a
  target that is itself pretty-printed JSON (the `--implementation-plan`
  shape) rejected rather than captured as a fake identity. A retried
  dispatch's Workflow-runtime-suffixed label (`dim (retry N)`) is normalized
  before grouping so a retry pools into its non-retried siblings' row instead
  of fragmenting into its own zero-`refutersDispatched` row. Recorded in
  `docs/token-baseline.json` § `refuterFanout` (48-run window ending
  2026-07-29, 2,208 agent records, not subtractable against the per-agent-class
  baseline above — the corpus grew between measurements) and summarized in a
  new `docs/token-baseline.md` § "Phase 1: review fanout". Both `--check` and
  `--audit` now validate this section alongside `nonGatingRefutationSkip` in
  one pass. `scripts/verify-token-report.sh` section 6 gained a fixture
  extension (two finders and two refuters, including a dimension-shadow
  refuter whose `f.id` names a different real dimension than its finding's
  own — proving dimension resolution reads the prompt, not the label) and two
  more planted-mutation self-tests.
- `scripts/measure-refuter-severity.mjs` gained a third distribution —
  **determining-finding rank** — which answers the one question a refutation
  cap lives or dies on: where in a ranked candidate list does the finding that
  actually determined the outcome sit? It replays the live pipeline's own rule
  by **importing** `rankFindings` / `survives` / `hasBlocking` from
  `.claude/workflows/lib/review.mjs` (read-only; no lane file is modified and
  no local severity table or confidence floor is kept), so the measurement
  cannot drift from the behavior it predicts. Ranking is over each unit's full
  **candidate** list, not its survivor list: severity sorts first, so
  rank-among-survivors would be a constant 1, whereas a cap truncates the
  candidate list. The review-unit key is phase 1's, unchanged — a new
  `extractFinderContext()` reads the identical prompt-embedded `context.target`
  from the finder side (`Review target: <target>.`), with no `phaseTitle` /
  `phaseIndex` anywhere. Each unit resolves to exactly one of `determining`
  (with a rank), `non-determining` (fully resolved, nothing gated — a distinct
  row) or `unrecoverable` under a closed, strictly **per-unit** reason
  vocabulary; an agent whose unit identity cannot be resolved is attributable
  to no unit, so it invalidates none and is instead counted as an
  `orphanAgents` diagnostic with a stated residual-risk bound. Nothing is
  imputed, unrecoverable units are excluded from every within-top-N numerator
  and denominator, and the recoverable share is restated beside every headline.
  Also reports candidate-set sizes, a `largeTier` sensitivity variant (the tier
  is embedded in neither prompt and is therefore not recoverable), and an
  `acTableGapUnits` diagnostic for the AC-table side channel. The
  supports/kills conclusion is **derived**, not asserted: an exported
  `CAP_VERDICT_RULE` + `deriveCapVerdict()` produce `supports-cap` /
  `kills-cap` / `inconclusive`, and `--audit` re-derives the verdict from the
  doc's own numbers. Over the same 48-run window ending 2026-07-29 (2,208
  agent records) the result is recorded in `docs/token-baseline.json` §
  `determiningFindingRank` and read in prose in a new
  `docs/token-baseline.md` § "Phase 2: rank of the determining finding":
  **the evidence supports a cap at N = 5**. `scripts/verify-token-report.sh`
  gained a new section 7 over a purpose-built fixture tree
  (`tests/fixtures/token-determining-rank`), plus a direct drive of the
  reason-resolution logic covering the whole closed vocabulary — including the
  three reasons the corpus fixture cannot reach (`multi-round-unit`,
  `ambiguous-finding-join`, `unreadable-finder-transcript`), the load-bearing
  precedence order between them and `dimension-coverage-gap`, and the
  retry-supersession rule that keeps a re-dispatched dimension from being read
  as a second review round — with twelve planted-mutation self-tests.
  The walk checks **severity-eligibility before disposition**: a candidate
  outside `hasBlocking`'s blocker set for the tier being walked (a
  `suggestion` at either tier, a `concern` at the default tier) can never be
  the determining finding whatever its verdict turns out to be, so an
  unreadable verdict on it leaves the unit `non-determining` instead of
  poisoning it to `unrecoverable` — the distinction the section exists to
  keep, and one that really bites on this pre-phase-6 window where
  suggestions were still dispatched to refuters. This moves three units out
  of `unrecoverable`, raising the recoverable share from 82.1 % to
  **85.7 %**; the `supports-cap` verdict at N = 5 is unchanged.
  `deriveCapVerdict` is additionally driven over a synthetic branch table so
  the `kills-cap` outcome — which neither the fixture nor the real corpus
  produces, and which the measurement explicitly treats as a legitimate
  terminal finding — is verified rather than merely reachable.

- `rdm task create` gained a `--no-plan-review` flag: it skips the automatic
  `needs-plan-review` stamp even when the `plan_review` config flag is
  enabled. Intended for tasks filed from a plan-review finding itself, so the
  gate's own output is never fed back into itself as new input to review. The
  standalone plan-review workflow and `rdm-do --auto`'s side-work task filing
  both now pass it automatically.
- The plan-review gate gained a new always-on dimension, **restraint** — the
  counterweight to `unit-of-work`: it flags a plan that specifies an
  implementation decision better left to whoever carries it out, or whose
  level of detail has grown past the point where more of it reduces risk.
- A new local-only agent registry, `.claude/agents/`, holding one definition:
  `rdm-mechanical.md`, a trimmed transcribe-one-command agent with a two-tool
  allowlist. The mechanical `agent()` call sites of the four local-only
  workflows (`document.js`, `backlog.js`, `estimate.js`, `plan-review.js`) now
  dispatch under it, so those lanes' command-transcribing steps run with a far
  smaller context — a measured 8,907 tokens per agent (−23%) less, confirmed by a
  live lane dispatch. Judgment steps (finders, refuters, planners, implementers)
  are deliberately unchanged.
  The definition is **not distributed**: `rdm agent-config` emits skills and
  workflows only, so the three distributed workflows do not reference it.
  Full evidence — including the measured 19,320-token per-agent cost of loading
  `CLAUDE.md` into any subagent, 60% above the previously recorded `chars/4`
  estimate — is in `docs/workflow-schemas.md` § "agentType / effort options
  spike" and `docs/token-baseline.json` → `mechanicalContextTrim`.
- New guards in `scripts/verify-workflow-review.sh`, each with a
  planted-mutation self-test: §2b — no workflow script may pass `effort:`, and no
  *distributed* workflow template may reference `agentType` (an unresolvable one
  raises rather than degrading silently, so it would hard-break every downstream
  lane on first dispatch); §2c — a bidirectional assertion that every mechanical
  call site carries `agentType: 'rdm-mechanical'` and no judgment site does, with
  a completeness sweep that fails if a site is added or removed without updating
  the asserted list.
- `scripts/verify-token-report.sh` gained coverage for `percentile()`'s linear
  interpolation branch, which every existing fixture short-circuited by giving
  each agent class only one record.

### Fixed

- `scripts/verify-token-report.sh` resolves symlinks in its scratch directory.
  On macOS `mktemp -d` returns a path under the `/var` → `/private/var`
  symlink, and every instrument it exercises gates its CLI on
  `path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)`; Node
  realpaths `import.meta.url` but not `process.argv[1]`, so a script copied
  into that scratch directory silently did nothing when run. Every
  planted-mutation self-test in sections 5 and 6 was therefore passing
  vacuously — the fixture comparison failed because the mutant emitted no
  report at all, not because the mutation was caught. The new section 7 also
  asserts each mutant produced a non-empty report before requiring its check
  to flip, so vacuity cannot return silently.

- `autopilot`'s `fetch:next` interpretation (`interpretNext`) no longer silently
  reports a malformed or double-wrapped `rdm next` result as a completed run.
  An agent transcribing `rdm next`'s JSON output was observed re-encoding the
  whole payload as a string inside its own `result` field; that shape fell
  through the old catch-all and was misclassified as "nothing to do", so a run
  with actionable phases remaining stopped early and reported a clean finish.
  `interpretNext` now defensively unwraps a string-encoded `result` (bounded,
  applied uniformly to all three `rdm next` shapes), and any genuinely
  malformed/unrecognized/empty return is classified as a distinct `unparseable`
  stop reason instead of `nothing`. The run summary flags any non-well-known
  stop reason with a loud `*** ABNORMAL TERMINATION` marker (via a new
  allowlist, `isAbnormalStop`, so a future unrecognized reason is fail-safe
  flagged too), and an `unparseable` `fetch:next` failure is also recorded in
  the summary's escalations section (tagged `[fetch]`) — though, since no
  phase stem is known at that point, it is summary-only and does not appear in
  `rdm review blocked`; the summary now says so explicitly right next to the
  `rdm review blocked` pointer, so a reader isn't misled into expecting the
  queue to list it. Making it queue-visible would require an rdm-core
  schema/status-model change (`blocked_phases` is phase-only by design), which
  is out of scope here; tracked as follow-up task
  `surface-fetch-next-escalations-in-blocked-queue`.
- The plan-review round-note reader (`parseRoundNotes`) accepted only
  `blocking` and `concern` bullets while the writer emitted every severity, so
  the first `suggestion` bullet in a previous round's note truncated the rest
  of that round's bullet list — making repeat-finding detection re-list them
  verbatim every pass. The reader now accepts every severity the writer can
  produce. (Reporting-only; outcomes were classified from the live survivor
  set, never the parsed one.)

### Changed

- **BREAKING — every Workflow engine under `.claude/workflows/` is now named
  `rdm-wf-<name>.js`.** The engines used to surface in the skill/slash-command
  listing under bare names that read as siblings of their `rdm-*` skill front
  doors, so `dispatch-phase` and `rdm-dispatch-phase` were indistinguishable
  without reading both files. All six moved at once — a prefix applied to a
  subset would be worse than none, because the *absence* of a prefix would stop
  meaning anything:
  `dispatch-phase.js` -> `rdm-wf-dispatch-phase.js`,
  `review-refute-fix.js` -> `rdm-wf-review-refute-fix.js`,
  `backlog.js` -> `rdm-wf-backlog.js`,
  `document.js` -> `rdm-wf-document.js`,
  `estimate.js` -> `rdm-wf-estimate.js`,
  `plan-review.js` -> `rdm-wf-plan-review.js`.
  Each engine's `meta.name` matches its new filename stem, so the listing entry
  moves with the file.
  **No `rdm-*` skill was renamed** — all 11 front doors (`rdm-do`,
  `rdm-dispatch-phase`, `rdm-autopilot`, `rdm-review`, `rdm-plan-review`,
  `rdm-estimate`, `rdm-backlog`, `rdm-document`, `rdm-land`, `rdm-revise`,
  `rdm-roadmap`) keep their names and invocations; only the engines behind them
  moved. `.claude/workflows/lib/*.mjs` filenames are likewise unchanged.
  **What you must do:** if you invoke a Workflow by name from your own
  automation or prose — a `Workflow` tool call, a custom skill, a script — update
  that name to its `rdm-wf-` form. Invoking a bare engine name now resolves
  nothing.
  **What you need not do:** no manual `rm`. Re-running
  `rdm agent-config claude --skills --out <dir>` **removes the superseded**
  `dispatch-phase.js` and `review-refute-fix.js` from a previously-emitted tree
  as part of the same emit, using the fingerprint-gated cleanup mechanism, and
  reports each as `Removed <path>`. The same emit also removes the long-retired
  `autopilot.js` orphan, which had no successor and no other cleanup path. A
  file you have edited yourself is never removed — it is reported as `Skipped`
  and left in place.

- The `review-refute-fix` and `estimate` workflows no longer hardcode this
  repo's `./target/debug/rdm` binary or `--project rdm` either — the same change
  `dispatch-phase` already landed, applied to the other two engines, so all
  three now honor ONE contract rather than each inventing its own. Both take
  `rdmBin` (**required**, fail-closed, with no PATH fallback — pass the explicit
  sentinel `"rdm"` to opt into PATH resolution) and an optional `project`,
  applied only to project-scoped subcommands: `rdm model resolve` and
  `rdm commit` never carry the flag, while `phase list/show/update`,
  `task update` and `worktree add` do. Both args are validated at parse time, so
  a mis-invocation throws before the first agent is dispatched and costs zero
  tokens.
  - `review-refute-fix`'s two legacy survivors-only shapes — `mode: "plan"`, and
    `mode: "code"` with no item identifiers — are the one documented carve-out:
    they emit no rdm invocations at all, so they keep working with no `rdmBin`
    and return their existing `{ mode, survivors, budget }` result unchanged.
  - The `rdm-review` and `rdm-estimate` skills now pass both args, so nothing
    that works today stops working. One bounded, temporary exception: until the
    prose `rdm-autopilot` loop is parameterized, its `estimate` pre-pass throws —
    **non-fatally**, because that skill already logs a warning and continues
    into its drive loop on an estimate error. Phases then dispatch at whatever
    tier `rdm next` reports rather than a freshly-rated one; autopilot's
    `dispatch-phase` payload is unaffected.
- The `dispatch-phase` workflow no longer hardcodes this repo's
  `./target/debug/rdm` binary or `--project rdm`, so a downstream repo can drive
  the autonomous lane with its own executable and its own project. It now takes
  a **required** `rdmBin` argument — the exact rdm executable to invoke; pass
  `"rdm"` to opt into `PATH` resolution explicitly — and an **optional**
  `project` argument, applied only to project-scoped subcommands (`rdm model
  resolve` and `rdm commit` never receive a project flag; omitting `project`
  emits no flag at all, so rdm's own `RDM_PROJECT`/`default_project` chain
  applies). A `Workflow` invocation that omits `rdmBin` now fails fast with an
  actionable error, before spending a single token, instead of silently running
  whichever `rdm` happens to be first on `PATH`; there is deliberately no
  existence preflight, because a stale global `rdm` would pass one. A `project`
  value that is not a plain name is rejected rather than interpolated into an
  agent's shell prompt. The `rdm-dispatch-phase`, `rdm-do --auto` (both the
  phase and task flows) and `rdm-autopilot` skills all pass the new arguments,
  in both the CLI and MCP variants.

- The local `rdm-autopilot` dogfood skill (`.claude/skills/rdm-autopilot/SKILL.md`)
  no longer hardcodes this repo's `./target/debug/rdm` binary or `--project rdm`
  either — closing the fourth (prose) propagation channel the
  `project-agnostic-lane` roadmap identified, the one no generator, byte-identity
  gate, or `*.js` grep could reach. It now accepts a **required** `--rdm-bin
  <path>` (accepts the literal sentinel `rdm` to opt into `PATH` resolution) and
  an **optional** `--project <name>`, and threads both through: `rdm model
  resolve` still carries no project flag, while `rdm next` / `phase update` /
  `phase show` all do, and both its `estimate` and `dispatch-phase` Workflow
  payloads now pass `rdmBin`/`project` as bare keys instead of this repo's
  literal values. The shipped `skill-autopilot-{cli,mcp}.md` templates were
  already project-agnostic (threaded by the `dispatch-phase` payload change
  above) and needed no change — they invoke no `estimate` pre-pass by design.

- The shipped code-review pipeline now decides **which conditional dimensions
  run from the CONTENT of the diff rather than from rdm-specific file paths**, so
  `api-docs`, `changelog` and `security` fire correctly in any repository and any
  language. Previously these were derived from path lists that were either
  hard-coded to this project's crate layout or matched on a spelling
  coincidence — and because a review that skips a dimension still reports clean,
  the failure was silent. Concretely: `api-docs` now fires when an added line
  introduces an exported or public symbol in any language (`export` /
  `export default`, `module.exports`, `pub` / `pub(crate)`, `public`, a
  capitalized Go identifier, `__all__`) instead of only on a `+pub` line under a
  specific crate path; `changelog` fires when an added line registers a CLI
  subcommand/argument/flag, an attached help or usage string, an HTTP/RPC route
  or tool, or user-visible printed output — instead of on any path merely spelled
  `config` or `mcp`, so a change to `vite.config.ts` no longer trips it; and
  `security` fires on sink-shaped content (process execution, filesystem access,
  environment and secret reads, deserialization or `eval`, raw memory — including
  every Rust `unsafe` shape, the inline `unsafe { … }` expression as well as the
  `unsafe fn` / `unsafe impl` / `unsafe trait` / `unsafe extern` declarations)
  instead of on a security-sounding path name, so a `child_process` sink in
  `src/lib/runner.js` is caught while `src/auth/session.js` with no sink content
  is not. Only ADDED diff lines are scanned, so a *removed* line never trips a
  signal.
- Review coverage is never silently dropped when the diff cannot be read: if a
  change touches code files but their content is unavailable, all three
  conditional signals are set to `true` — an explicit value, never an omitted
  key — so their dimensions still run. A docs-only change, or a readable diff
  that matches nothing, remains a genuine negative and runs only the always-on
  dimensions. The signal-derivation input shape is unchanged
  (`{ targetType, changedFiles, diffText }`) and no configuration option was
  added, so no downstream caller needs to change.
- The `security` dimension of the review skills emitted by `rdm agent-config
  claude --skills` now reviews on a **threat-model** basis instead of matching
  language-specific API and keyword patterns. A finding is a claim that an
  attacker can do something they should not be able to do, backed by the code
  that grants it — explicitly not lint, not style, not "consider using a safer
  API" — and it is worked through five language-neutral categories: injection,
  authorization, memory, crypto, and exposure. Severity is rated on impact
  rather than certainty and maps onto the existing `blocking` / `concern` /
  `suggestion` contract rather than adding a second ladder. Findings may now
  carry an optional `category` slug (`command-injection`, `path-traversal`,
  `unsafe-ffi`, `hardcoded-secret`, `info-disclosure`, …). The dimension is now
  triggered purely by the paths a change touches; the previous
  language-specific diff-content triggers, which could never fire outside one
  language, are gone.
- Every review finder prompt — both code review and plan review, every
  dimension — now carries **prompt-injection hygiene**: the repository under
  review is untrusted data and cannot issue instructions to a reviewer, and
  text telling a reviewer to skip a file, ignore a finding, stop reviewing, or
  claiming the code is already verified or approved is itself reportable as a
  finding. Applies to the review and plan-review skills `rdm agent-config
  claude --skills` emits and to the workflow scripts it ships alongside them.
- The shipped code-review dimensions no longer hardcode rdm's own Rust
  conventions. `correctness`, `architecture`, `api-docs`, `changelog` and
  `security` now state generic intent — the error-handling conventions the
  project states, its layering contract, the documentation its public API
  requires, its changelog rule, and how it requires a use of the language's
  safety escape hatch to be justified — and direct the reviewing agent to read
  the consuming project's own principles document (`docs/principles.md`,
  falling back to `CLAUDE.md` / `AGENTS.md` in the project root) for the
  specifics. A repo in any language now gets a reviewer that enforces that
  repo's rules instead of applying rustdoc, crate-layout and `// SAFETY:` rules
  to it. This is a prose change only: no config key, CLI
  flag, or pipeline input changed, and pointing the reviewer at a different
  project still requires no code change.
- The shipped review pipeline now grades **at most 5 findings per review unit**
  by default. It ranks a unit's gating findings by severity, then confidence,
  and dispatches a refuter only for the top 5; the rest are still reported, but
  pass through un-refuted and marked `unrefutedReason: 'budget'`. The confidence
  floor still applies to them — the budget skips *grading*, never *filtering* —
  so a bounded review can only ever report MORE work to do, never less. Override
  it per run with `maxRefutations` on `dispatch-phase`, `plan-review`, or
  `review-refute-fix`; `0` is legal and means "grade nothing". There is no
  "uncapped" value — pass a large number instead. The default of 5 is measured,
  not guessed: over the recorded run corpus the finding that actually determined
  the outcome was within the top 5 for 100 % of units at the default tier and
  98.2 % at the `large` tier (`docs/token-baseline.md` § "Phase 4: the chosen
  refutation budget").
- A bounded review now says so, everywhere you would look. The pipeline logs how
  many findings it produced, graded, and passed through; the dispatch OUTCOME
  carries a `reviewBudget` field and appends a short
  `[review budget hit: N produced, M graded, K ungraded]` clause to its summary
  (and therefore to the reason recorded on a parked or blocked item, visible in
  `rdm review blocked`); and an autopilot run summary suffixes a `[budget]` tag
  onto that phase's entry in its `phases completed (...)` line. A review that
  stayed under budget reads exactly as it did before. A bound hit on an *early*
  round stays reported even after a later revision resolves it — for plan-revise
  rounds exactly as for code-rework rounds — and when both gates hit, the clause
  reports the later (code) round's counts, not the earlier plan gate's.
- Review findings now carry an explicit provenance marker, so a report can tell
  four cases apart that used to blur together: graded-and-survived (no marker),
  deliberately skipped as non-gating (`unrefutedReason: 'non-gating'`), cut for
  budget (`unrefutedReason: 'budget'`), and **grading crashed**
  (`refuterError: true`) — the last of which previously carried no marker at all
  and was indistinguishable from a verified survivor.
- `rdm-autopilot` no longer delegates the roadmap-driving loop to
  `.claude/workflows/autopilot.js`. Per the `prose-autopilot-orchestration`
  roadmap's phase 2, the skill now drives the loop itself in prose: it invokes
  `estimate` and `dispatch-phase` as `Workflow`-tool calls and runs every other
  step (`rdm next`, `rdm phase update`, `rdm phase show` read-backs, `rdm model
  resolve mechanical`) as direct Bash commands in its own context instead of
  dispatched mechanical `agent()` subagents — those subagents existed only
  because the headless workflow runtime cannot run Bash itself, a limitation a
  live prose skill does not have. The estimate pre-pass now runs through a real
  `workflow('estimate', …)` call for the first time; previously `autopilot.js`
  reached the same fan-out only via a stamped `estimate-core` copy embedded in
  `lib/autopilot.mjs`, never a genuine `estimate` invocation. `autopilot.js`,
  `lib/autopilot.mjs`, and their `scripts/verify-workflow-autopilot.sh` harness
  are left in place and unexecuted by this skill, pending a later phase's
  retirement decision.

- The review pipeline no longer spawns a refuter for a **`suggestion`**
  finding. Severity is the only thing that turns a finding into an outcome, and
  a `suggestion` gates nothing at any tier, so a refuter's verdict on one could
  never change anything. Such findings now pass straight through marked
  `unrefuted: true` — still subject to the same confidence floor — and both act
  steps handle them under an explicit disposition rule: incorporate the ones
  that improve readability or clarity where the change is not major, file the
  ones worth keeping as follow-up tasks, and skip only the rest with a stated
  reason (recordable as a new `skipped` action, with a `reason`, in the
  code-lane `CODE_ACT` schema) — so a real observation can never evaporate into
  a transient skip reason. `blocking` and `concern` keep their refuter
  — over the measured corpus a `concern` is overturned *more* often than a
  `blocking` finding — and the rule is fail-safe: a finding whose severity is
  missing or unrecognized is still refuted. Measured effect over the recorded
  corpus: 239 of 989 refuters (24.2 %, 20.7 % of refuter tokens) would not have
  been spawned. A refuter that *crashes* still keeps its finding and is
  deliberately not marked `unrefuted`.

- The autonomous-lane Workflow scripts now accept **optional caller-supplied
  arguments** so they no longer spawn a dedicated mechanical subagent for work
  the invoking skill already did: `dispatch-phase` takes
  `phaseMeta`/`taskMeta`/`alreadyInProgress`, `autopilot` takes
  `mechanicalModel`/`phaseList`/`next`, `estimate` takes
  `mechanicalModel`/`phaseList`, `plan-review` takes
  `fetched`/`wontFixedTexts`/`mechanicalModel`, `backlog` takes
  `mechanicalModel`/`report`, `document` takes `mechanicalModel`/`roadmapMeta`,
  and `review-refute-fix` takes `diff`. `dispatch-phase` additionally absorbs the
  branch diff into its implementer instead of running a separate diff agent.
  **Every one of these is optional and behaviour-neutral** — invoking a workflow
  directly via the `Workflow` tool with the previous argument shape produces the
  same outcome as before, exercising the unchanged in-workflow fetch. Across the
  measured corpus this removes 115 of 304 mechanical subagents (~38%); see
  `docs/mechanical-agent-inventory.md` for the full census, the classification
  rule, and the measured delta.
- A caller-supplied `phaseMeta` payload for `dispatch-phase` must now carry the
  phase's non-empty `model` difficulty tier (alongside its body and all five
  resolved model ids) or it is rejected and the in-workflow fetch runs instead —
  so the `rdm-dispatch-phase` and `rdm-do --auto` skills now gather and pass the
  tier. The tier is the sole input to the code-review gate's strictness, and an
  absent one silently fell back to `medium`: a caller that supplied everything
  *but* the tier would have had a `large` phase reviewed at `medium` strictness,
  letting a blocking finding that should have forced a rework round pass
  straight to `reviewed`. A task payload carries no tier and is unaffected.
  Rejection is a fallback, never an error — the run proceeds exactly as it does
  with no payload at all.
- The `rdm-plan-review` skill now reads its target with
  `rdm task show|phase show|roadmap show --format json` itself and passes the
  parsed JSON verbatim, instead of asking a subagent to transcribe it. That
  transcription step had twice written junk over a target's real tag list, and
  schema validation could not catch it because both bad returns were
  schema-valid. A caller-supplied payload must carry the target's `tags` array
  (and, for a roadmap, each phase's `stem`/`body`/`tags`) or it is rejected and
  the in-workflow fetch runs instead — the gate replaces an item's whole tag
  list with `--tags`, so an incomplete payload would silently clear it.
- The distributed `rdm-plan-review`, `rdm-backlog`, `rdm-document`, `rdm-review`
  and `rdm-estimate` skills are **not** yet Workflow shims, so they continue to
  use each workflow's own in-workflow fetch — correct, just not yet cheaper.
  Converting them is tracked by task
  `convert-remaining-skill-templates-to-workflow-shims`. MCP skill variants
  likewise omit the model-derived arguments, since there is no MCP
  model-resolve tool.

- The plan-review gate's `coherence` dimension no longer blocks a plan merely
  because it leaves an implementation decision undecided. A plan may delegate
  decisions to whoever carries it out; an undecided point is now a `concern`
  unless the undecided branches would lead to different goals or outcomes.
  Coherence is `blocking` only when an implementer following the plan as
  written would build the wrong thing.
- The standalone plan-review workflow now caps repeated review rounds on the
  same item: each non-`reviewed` pass records a `## Plan Review Round <N> —
  <outcome>` audit note on the item's body. A finding that is still genuinely
  unresolved keeps the item in `rework`/`escalated` on round 2 exactly as on
  round 1 — repeats are only de-duplicated in the human-facing note, never in
  the pass/fail decision — and a third round escalates to a human instead of
  looping further. The cap only fires on findings that are still unresolved: a
  plan that is genuinely clean by the third pass is reported `reviewed`, not
  escalated. A finding already resolved `wont-fix` on a prior pass is dropped
  from both the report and the outcome, and is never re-raised.

### Changed

- `rdm agent-config claude --skills`/`--mcp` no longer emits
  `.claude/workflows/autopilot.js`: the autopilot loop is now the prose
  `rdm-autopilot` skill, which orchestrates the `dispatch-phase` and
  `estimate` Workflows directly instead of nesting through a stamped
  `autopilot.js`/`lib/autopilot.mjs` copy. The emitted workflow-file count
  drops from 3 to 2 (`dispatch-phase.js`, `review-refute-fix.js`).
- The shipped `rdm-autopilot` skill templates (`rdm agent-config claude
  --skills`/`--mcp`) now carry the same full prose-parity orchestration
  content as the local dogfood `rdm-autopilot` skill, replacing the interim
  "full prose-parity documentation lands in a follow-up phase" placeholder:
  the budget-check semantics (a shared, rework-inclusive dispatch counter
  defaulting to 50), the fetch/classify/work/park drive loop, the
  advance/park read-back confirmation retried up to 2 times, and the exact
  summary format with its known-good stop-reason allowlist (`nothing`,
  `blocked-on-dependencies`, `budget`, `plan-only-exhausted`,
  `mechanical-model-unresolved`). The MCP variant gains a new
  `rdm_phase_show` tool (surfaced via a new `{t_phase_show}` template
  placeholder) so it can read a phase back after an advance/park write,
  mirroring the CLI variant's `rdm phase show --format json` call — MCP has
  no equivalent of that command otherwise.
- The shipped `rdm-autopilot` skill templates (`rdm agent-config claude
  --skills`/`--mcp`) no longer run an `estimate` Workflow pre-pass before
  dispatching phases downstream: every downstream phase now dispatches at
  whatever tier `rdm next` already reports (default `medium`) rather than
  being freshly rated first. `generate_workflows()` never emitted
  `.claude/workflows/estimate.js` in the first place — it references
  `agentType: 'rdm-mechanical'`, which a downstream repo's (nonexistent)
  `.claude/agents/` registry has no definition for and which raises rather
  than degrading silently — so the templates' prior instruction to invoke it
  was a dangling reference the very first dispatch would have hit. The step
  numbering shrinks accordingly (the phase-cursor fetch, drive loop, and
  summary steps renumber down by one), the `mechanicalModel`/`phaseList`
  hoists (and, on MCP, the `rdm_phase_list` tool) are dropped entirely, and
  `mechanical-model-unresolved` is removed from the known-good stop-reason
  allowlist. The local dogfood `.claude/skills/rdm-autopilot` copy is
  unaffected and still invokes the real `estimate` Workflow — only the
  shipped/downstream lane changes.
  `scripts/verify-agent-config-distribution.sh`'s Workflow-invocation check
  is also generalized from an autopilot-only literal-string check to
  `check_workflow_invocations_resolve`, which asserts, across every emitted
  skill, that any "Invoke(ing) the `<name>` Workflow" instruction names a
  Workflow that actually resolves to a file in the same emitted
  `.claude/workflows/` tree.

### Fixed

- The MCP variant of the shipped `rdm-autopilot` skill template
  (`rdm agent-config claude --skills --mcp`) called its own new
  advance/park read-back steps with the wrong argument shape: `stem: S` and
  no `project` field at all, on both the `rdm_phase_update` and
  `rdm_phase_show` calls. The real MCP server's `PhaseUpdateParams`/
  `PhaseParams` require `phase` (not `stem`) plus a mandatory `project`,
  matching every other MCP skill template in this repo — so every phase
  the loop tried to advance to `reviewed` or park as `blocked` would have
  failed at exactly the read-back confirmation mechanism this same phase
  introduced. Both call sites now pass
  `project: {proj_param}, roadmap: "<slug>", phase: S`, and both the
  generated-skill unit tests and `scripts/verify-agent-config-distribution.sh`
  gained a regression assertion pinning the correct argument shape at those
  two call sites and rejecting a reintroduced `stem:`-keyed call.
- The shipped `rdm-autopilot` skill templates (`rdm agent-config claude
  --skills`/`--mcp`) no longer instruct invoking a Workflow literally named
  `autopilot`. The same change that dropped `.claude/workflows/autopilot.js`
  from `generate_workflows()`'s emitted output left the templates'
  "What to do" steps still telling a downstream agent to invoke it — a
  contradiction that would have broken the very first dispatch of the
  emitted skill in any repo that ran `rdm agent-config claude --skills`.
  The steps now describe driving the loop directly and composing the real
  `dispatch-phase` Workflow it still relies on, and both
  `rdm-core`'s generated-skill tests and
  `scripts/verify-agent-config-distribution.sh` gained a planted-mutation-backed
  assertion that the emitted template can never again claim to invoke a
  Workflow named `autopilot`.
- The autonomous code review (`rdm-dispatch-phase`, `rdm-autopilot`, the
  standalone `review-refute-fix` workflow) now mechanically forces `rework`
  whenever the acceptance-criteria table it reports carries a FAIL or PARTIAL
  criterion, regardless of finding severity or refutation — previously this
  guarantee held only to the extent the `ac` dimension happened to emit its
  gaps as `blocking` findings that survived refutation and the confidence
  floor. It also now incorporates any surviving non-blocking (concern or
  suggestion) finding on an otherwise-clean review by size instead of
  silently dropping it: small fixes are applied inline before landing, and
  large ones are filed as a follow-up task tagged `code-review`. When an
  AC-only gap (no blocking finding) triggers a rework pass, the implementer is
  now actually told which acceptance criteria failed and why — previously the
  rework prompt carried only the (empty) findings array, so the retry had no
  signal to act on and would very likely reproduce the same gap. The rework
  summary in every surface that can classify an AC-only-gap rework
  (`dispatch-phase`'s phase and task outcomes, and `review-refute-fix`'s own
  standalone summary) now names the real cause ("unmet acceptance criteria in
  AC table") instead of the misleading "no surviving findings".

## [0.18.0] - 2026-07-25
### Fixed

- The dogfood autonomous-lane workflows' mechanical (fetch/exec) agents now
  resolve and pin to the small/mechanical tier instead of inheriting the
  reviewer or session model. `dispatch-phase`'s `stamp:in-progress` and
  `diff:signals` steps now resolve `models.mechanical` (added to Stage-0's
  batch model resolution, alongside plan/implement/review-find/review-verify)
  instead of borrowing `models.review_find`. The four headless workflows
  (`estimate`, `plan-review`, `backlog`, `document`) each gained a small
  `rdm model resolve mechanical` bootstrap step whose resolved id is threaded
  into every mechanical Bash agent in that file (list/writeback/tier-read;
  fetch/gate-tag-clear; report fetch; roadmap/phase fetch and per-phase
  gather/write) — closing the gap the earlier autopilot-only mechanical-tier
  fix left open (e.g. a live `backlog` run's `fetch:report` agent previously
  ran on `claude-opus-4-8`). Judgment agents (plan/implement/review, the
  estimate rater, the plan-review orchestrator's `act` step, backlog's
  analyzers, document's synthesis step) are unaffected.

### Changed

- The dogfood `autopilot` Workflow's estimate pre-pass now records a
  `## Estimate <difficulty> — <justification>` audit note on each phase it
  rates (previously it persisted `--difficulty` only and dropped the rater's
  justification). The behavior — and the underlying difficulty-writeback logic
  — is now single-sourced with the new standalone `estimate` workflow via the
  shared `estimate-core` block, so both surfaces write the note identically.
- **Breaking:** the shipped `rdm-autopilot`, `rdm-dispatch-phase`, and the
  `--auto` branch of `rdm-do` skill templates (`rdm agent-config claude
  --skills`, both `cli` and `mcp` variants) are rewritten as thin shims that
  invoke the autonomous-lane Workflow scripts (`.claude/workflows/autopilot.js`
  / `dispatch-phase.js`, now emitted alongside the skills — see the prior
  `[Added]` entry) via the `Workflow` tool, instead of re-narrating an 8-step
  plan/plan-review/implement/code-review loop in prose with no runnable
  backing. Every real behavioral guardrail carries forward unchanged: the
  `--permission-mode auto` safety rules (`Edit`-not-`Write`; never `git stash
  -u`/`reset --hard`/`clean -fdx`) now live in the shipped
  `skill-dispatch-phase-{cli,mcp}.md` templates, not just the local dogfood
  copy; `rdm-autopilot`'s four run-mode flags (`--max-phases`, `--plan-only`,
  `--max-plan-revise`, `--max-code-rework`) are documented; `rdm-do`'s new
  `## Auto phase dispatch` / `## Auto task dispatch` sections route `--auto`
  runs into `dispatch-phase`, reading `outcome.status` / `outcome.reason` /
  `outcome.writesCompletion` as data instead of restating the gate policy.
  Existing consumers of the old prose templates should regenerate
  (`rdm agent-config claude --skills --out <dir>`) rather than hand-patch.
- The `rdm-plan-review` skill's documentation has been reorganized and clarified.
  The review pipeline steps (Setup → Find → Consolidate → Categorize & act → Gate)
  now have clear, permanent hand-authored sections that document plan-review's
  domain-specific logic (argument parsing, verdict-determination, and
  `needs-plan-review` tag-clearing gating). The Gate section is now more explicit
  about its fundamentally different role compared to code-review's status-transition
  gate: plan-review gates by clearing or leaving the `needs-plan-review` tag, never
  writing rdm status or land-time completion directives. The review dimensions and
  verdict rules are no longer hand-authored here at all: they are generated into the
  skill from the canonical review source (`.claude/workflows/lib/review.mjs`, via
  `scripts/gen-skill-review.sh --mode plan`), so the skill and `rdm-review` now share
  one specification. Setup, Find, Consolidate, Categorize & act, and Gate remain
  hand-authored **by design** — they carry plan-review's domain-specific logic, not
  review-spec content, and are not pending future automation. **No behavior change**;
  all verdict categories, gating logic, and per-phase handling remain unchanged.

### Removed

- The now-superseded "Mandatory dispatch — no inline work" / inline-collapse
  self-check checklists are gone from the shipped `rdm-autopilot` and
  `rdm-dispatch-phase` templates (both `cli` and `mcp` variants) — they existed
  only to stop an LLM from narrating orchestration it should have dispatched to
  a subagent, and a thin shim that hands off to the `Workflow` tool cannot
  inline-collapse that way. No template documents a `--land` flag on
  `rdm-autopilot` any longer (it never had one); landing to `main` stays
  `rdm-land`'s exclusive, explicit-invocation job.
- **Breaking:** the plan-review Stop hook (`rdm agent-config claude --hooks`'s
  `.claude/hooks/rdm-plan-review-on-create.sh`) and its Pi `agent_end`
  extension (`rdm agent-config pi --hooks`'s
  `.pi/extensions/rdm-plan-review.ts`) are no longer generated, and the
  `--hooks` flag itself is removed from `rdm agent-config claude`/`rdm
  agent-config pi` entirely. It is superseded by plan review now running
  in-flow on the ephemeral implementation-plan lane (`dispatch-phase`,
  `rdm-do`'s `--implementation-plan` review). **That in-flow review is a
  distinct mechanism from, and does not clear, the persisted
  `needs-plan-review` tag** stamped onto roadmaps/phases/tasks by `roadmap
  create` / `phase create` / `task create` when `plan_review` is enabled — it
  reviews an ephemeral plan draft, not the persisted item. With the Stop hook
  retired, clearing `needs-plan-review` on items created via `rdm-roadmap`,
  ad hoc create commands, or `rdm-do` side-task filing has **no automated
  reprompt left and is manual-only** until a filed follow-up task
  (`wire-active-plan-review-tag-gate`) lands: run the `rdm-plan-review` skill
  against the item (`--roadmap`/`--task`/`<roadmap> <phase>`), or
  periodically sweep with `rdm search "" --tag needs-plan-review`. Projects
  that installed the retired hook/extension via `--hooks` should remove
  `.claude/hooks/rdm-plan-review-on-create.sh` and its `hooks.Stop` entry in
  `.claude/settings.json` (or `.pi/extensions/rdm-plan-review.ts`) manually —
  `agent-config` no longer manages them.

### Added

- New dogfood `estimate` Workflow (`.claude/workflows/estimate.js`): rating an
  rdm roadmap's phase difficulties now runs headlessly. Given a roadmap slug
  (optionally narrowed to a single phase number) it lists the phases, filters
  to the ones whose difficulty is unset, rates each in a parallel fan-out, and
  writes the rating back — persisting the difficulty AND appending a
  `## Estimate <difficulty> — <justification>` audit note to the phase body —
  then reads the model tier back from rdm-core for its summary. When narrowed to
  a single phase, the summary reports the other still-unestimated phases as
  `deferred` (not targeted this run) rather than mislabeling them as already
  estimated; only phases that genuinely carry a difficulty are reported as
  skipped. It never passes
  `--model` and never reimplements the difficulty→tier mapping: rdm-core
  (`Difficulty::model_tier`) stays the single home for that policy. Its pure
  core lives once in `.claude/workflows/lib/estimate.mjs` (the `estimate-core`
  marker region) and is stamped byte-identical into every consumer by the new
  `scripts/gen-workflow-estimate.sh` (with a `--check` drift gate); the local
  `rdm-estimate` skill is re-authored as a thin shim over this workflow. New
  dogfood harness `scripts/verify-workflow-estimate.sh` gives hermetic
  estimate-core DRIFT + BEHAVIOR coverage. The shipped `rdm-estimate` skill
  template is unchanged; this workflow is dogfood-only for now.
- New dogfood `backlog` Workflow (`.claude/workflows/backlog.js`): the
  read-only, propose-only `rdm-backlog` grooming pass now runs headlessly. It
  runs `rdm backlog report` once, fans one READ-ONLY analyzer agent out per
  populated signal category (`stale_tasks`, `duplicate_clusters`,
  `tag_clusters`, `archivable_roadmaps`) in parallel, and consolidates the
  results into one ordered, reviewable batch of `{command, rationale}`
  proposals grouped by category plus a merged `## Open questions` section —
  or short-circuits to "Nothing to groom" when the report carries no signals.
  The non-mutation guarantee is structural: the only Bash-executing agent in
  the whole run is the read-only report fetch, and every analyzer is
  explicitly told to propose text only, never to execute a mutating command
  — mirroring `review-refute-fix`'s "READ-ONLY reviewer" framing. The local
  `rdm-backlog` skill is re-authored as a thin shim over this workflow. New
  dogfood harness `scripts/verify-workflow-backlog.sh` asserts the batch
  shape over a seeded report, the empty-report short-circuit, and — against a
  real seeded plan repo — that a run leaves the plan repo's git state
  byte-identical before and after. The shipped `rdm-backlog` skill templates
  are unchanged; this workflow is dogfood-only for now.
- New dogfood `plan-review` Workflow (`.claude/workflows/plan-review.js`): a
  standalone plan-mode review over all four target types — `--task <slug>`,
  `--roadmap <slug>`, a positional `<slug> [phase]`, and
  `--implementation-plan`. It reuses the one canonical review core
  (`buildReviewPipeline('plan')` and `GATE_POLICY.plan`) with no new review
  logic. For `--roadmap` it reviews the roadmap body plus every phase and gates
  each **independently** (a `parallel()` per-phase fan-out): a phase that
  reworks keeps its `needs-plan-review` tag while sibling phases that reach
  `reviewed` have theirs cleared. Clearing is a sibling-preserving
  read-filter-write (a reserved tag like `depends-unlanded` survives), the gate
  never persists an rdm status, and `--implementation-plan` — which has no
  persisted rdm item — is report-only (no body edit, no filed task, no gate).
  An unread or empty plan fails closed (the tag is left in place). The local
  `rdm-plan-review` skill is re-authored as a thin shim over this workflow. The
  shipped `rdm-plan-review` skill templates are unchanged.
- The standalone `review-refute-fix` Workflow tool now returns a full
  `reviewed` / `rework` / `escalated` verdict — with the mapped rdm status,
  `writesCompletion`, and a summary — instead of just a list of surviving
  findings, when invoked as `{ mode: 'code', roadmap, phase }` or
  `{ mode: 'code', task }`: it derives real diff signals from the item's
  worktree (falling open to every review dimension when the diff is
  unavailable, exactly like `dispatch-phase`'s code gate) and runs the same
  canonical review pipeline. An optional `gate: true` persists the mapped
  status via a mechanical `rdm phase update` / `rdm task update` call, for
  headless or ad hoc callers — it never runs `rdm commit` and never writes the
  land-time completion trailer. Existing ad hoc invocations (`mode: 'plan'`, or
  `mode: 'code'` with no roadmap/phase or task) are unaffected and keep
  returning the original `{ mode, survivors }` shape. The interactive
  `rdm-review` skill now delegates its dimension-finding/refuting mechanics to
  this same workflow (invoked with `gate: false`) instead of re-deriving them
  by hand, while keeping its own human-in-the-loop report/act/gate steps
  — including persisting status and amending the completion trailer — exactly
  as before.
- New dogfood harness `scripts/verify-workflow-review-outcome.sh` covers the
  above: the full OUTCOME shape for a clean and a blocking seed, the
  diff-signals fail-open contract, the mutual-exclusion guard on `task` vs
  `roadmap`/`phase`, both legacy backward-compatible shapes, the optional
  headless gate, and that the `rdm-review` skill shim still references the
  workflow while retaining its interactive report/act/gate prose and the
  completion-trailer mechanism.
- New dogfood harness `scripts/verify-agent-config-distribution.sh` proves
  that `rdm agent-config claude --skills` (both the plain CLI and `--mcp`
  variants) emits a self-consistent, working autonomous lane into a
  downstream repo: the 3 workflow scripts land byte-identical to their
  `.claude/workflows/*.js` sources, all 11 skills land with valid
  frontmatter, and every literal `.claude/workflows/<name>.js` reference
  inside an emitted skill resolves to a real file in the same emitted tree —
  with planted-corruption self-tests proving neither gate is vacuous.
- `rdm agent-config claude --skills --out <dir>` now also emits the
  autonomous-lane Workflow-tool scripts (`autopilot.js`, `dispatch-phase.js`,
  `review-refute-fix.js`) under `<dir>/.claude/workflows/`, byte-identical to
  this repo's own dogfood copies in `.claude/workflows/`. Emission is
  Claude-only (Pi has no Workflow-tool runtime) and `--out`-only, not
  `--user` — the scripts still hardcode this repo's own
  `./target/debug/rdm` binary path and `--project rdm` invocation and are
  not yet parameterized for a downstream target repo (tracked as a
  follow-up).
- `rdm hook done-line` prints the land-time `Done:` commit trailer for a phase
  (`--roadmap <slug> --phase <stem>`) or a task (`--task <slug>`). It is now the
  single home of that format string, so the review gate and `rdm-land` amend its
  output onto the branch commit instead of hand-typing the format. It rejects
  both-or-neither of `--phase`/`--task`, an embedded `/`, and `task` used as a
  roadmap slug (a reserved prefix), each with an actionable error.
- The `rdm-review` skill gains a **security** review dimension, triggered when a
  change touches auth, input parsing/validation, path or file handling,
  subprocess/shell invocation, secrets and credentials, deserialization, network
  code, or `unsafe` blocks. It reviews injection, path traversal, secret leakage,
  missing authorization, and unsafe-invariant violations. The pre-existing
  dimensions (`ac`, `correctness`, `tests`, `architecture`, `api-docs`,
  `changelog`) are unchanged, so this is strictly added coverage.

- Tag filtering across the list surfaces. `rdm roadmap list --tag <tag>` and the
  top-level `rdm list --tag <tag>` are new; `rdm task list --tag <tag>` and
  `rdm search --tag <tag>` now accept the flag **repeatedly**, and repeats combine
  with AND (`--tag bug --tag ui` keeps only items carrying both). Matching is
  exact and case-sensitive, passing no `--tag` imposes no constraint, and a filter
  that matches nothing prints the usual empty-state line and exits 0. `--tag`
  composes with the existing `--status`/`--priority`/`--sort` filters, and is
  allowed alongside `roadmap list --archived` (unlike `--sort`/`--priority`).
- List output now surfaces tags when any listed item has them. `rdm task list`
  (text, table, and Markdown) and the Markdown `rdm roadmap list` gain a trailing
  `Tags` column; the default (human) `rdm roadmap list` / `rdm list` view is
  paragraph-shaped and instead appends a ` [tags: bug, ui]` suffix after the
  priority suffix on each tagged line. Untagged items render an empty cell / no
  suffix, and the column is omitted entirely when nothing is tagged. Because
  these renderers are shared, this also changes the MCP `rdm_task_list` and
  `rdm_roadmap_list` tool results. JSON output is unchanged.

- New `rdm tag list --project <name>` command: a read-only inventory of every tag
  in use across a project's roadmaps and tasks, printed most-used-first as
  `cli (3)` / `web-ui (5)` lines (or `No tags in use.` when there are none).
  Counts cover roadmaps and tasks of every status — including `done` and
  `wont-fix` — but not phases or archived roadmaps, and tags are compared
  verbatim, so `CLI` and `cli` are listed separately. Pass `--format json` for a
  machine-readable array of `{"tag", "count", "roadmaps", "tasks"}` objects for
  agent consumption. This answers "which tags exist?"; `rdm search "" --tag <name>`
  answers "what carries this tag?".

- New read-only MCP tool `rdm_backlog_report`, a thin wrapper over the same
  `rdm_core::ops::backlog::report` the CLI's `rdm backlog report` already
  calls, returning the identical four-array JSON shape (`stale_tasks`,
  `duplicate_clusters`, `tag_clusters`, `archivable_roadmaps`). This backs a
  new `rdm-backlog` MCP skill (`rdm agent-config claude --mcp --skills`),
  closing the last cli/mcp skill-set gap: both platforms now emit the same 11
  skills, asserted by a new relative-path parity test
  (`generate_skills_cli_mcp_name_parity`) so a future one-sided addition
  fails CI instead of silently shipping asymmetric skill sets.
- New dogfood `document` Workflow (`.claude/workflows/document.js`): the
  `rdm-document` doc-generation pass now runs headlessly. It validates every
  phase of a roadmap is `done` (aborting with the incomplete-phase list
  otherwise), fans a per-phase git-gather step out in `parallel()` (`git log`
  / `git diff --stat` over each phase's recorded commit SHA, falling back to
  phase-body-only when a phase has no SHA), runs one synthesis agent to draft
  the doc, and a mechanical Bash agent to write it to `--out` (default
  `docs/<slug>.md` — the runtime has no filesystem of its own), returning
  `{ roadmap, aborted, incompletePhases, path, draft }`. It performs no
  status mutation and no approval step of its own: the workflow produces an
  artifact, not a completion signal, and the terminal human review is the
  local `rdm-document` skill's job alone. That skill is re-authored as a thin
  shim over this workflow. New dogfood harness
  `scripts/verify-workflow-document.sh` asserts the draft is produced at the
  expected default/`--out` path, the all-done validation aborts on an
  incomplete roadmap, and a phase with no recorded commit falls back to
  body-only — including against a real seeded plan repo and source repo, not
  just fabricated inputs. The shipped `rdm-document` skill templates are
  unchanged; this workflow is dogfood-only for now.

### Fixed

- The workflow lane's `dispatch-phase` now stamps a phase (or task) `in-progress`
  (best-effort) when it begins working it, right after fetching the item and
  resolving models and before planning starts. Previously a direct `Workflow`
  invocation of `dispatch-phase` — and therefore every autopilot-driven phase —
  jumped straight from `not-started` to `reviewed`/`blocked` with no observable
  in-progress signal in `rdm phase list`, a status search, or the TUI while the
  run was executing, so a crashed or cancelled run looked untouched instead of
  attempted. A `--plan-only` pass is unaffected — it never implements, so it
  never stamps.

- An autonomously produced branch now reaches `rdm-land` ready to land, with no
  manual rebase. `rdm-dispatch-phase` / `rdm-autopilot` deliberately never write
  the `Done:` commit trailer, and their outcome now says so explicitly — it
  carries `writesCompletion: true` on a clean review — so `rdm-land` synthesizes
  the trailer from `rdm hook done-line` and amends it onto the branch tip
  *before* the rebase and fast-forward. Previously the missing trailer had to be
  noticed and repaired by hand after the fact.

- The workflow lane's dispatch outcome classifier no longer marks a phase
  `reviewed` on a failing code review when no rework round ran. It previously
  hard-coded a two-slot "first pass + exactly one rework" shape, so with a
  code-rework budget of 0 the empty post-rework slot made a blocking first-pass
  review classify clean. The classifier now judges the last review round that
  actually ran, however many there were (including zero).

- Removed dangling instructions from the shipped `rdm-review` / `rdm-plan-review`
  skill templates (both `cli` and `mcp` variants) telling the reader to "edit
  `.claude/workflows/lib/review.mjs` and run `scripts/gen-skill-review.sh`" to
  regenerate the review specification section. Those paths are dogfood-only
  tooling in rdm's own source repo and never ship to a consumer repo, so
  following the instruction there was impossible. The generated region is now
  described as fixed content, rendered from rdm's own canonical review source
  at release time, with upstream changes arriving via the next
  `rdm agent-config` regeneration rather than a local edit.

### Changed

- The shipped `rdm-plan-review` skill now carries the same **generated review
  specification** as `rdm-review`: the same severity scale, confidence floor,
  finding format, and `reviewed` / `rework` / `escalated` outcome vocabulary,
  replacing the old PASS / PASS WITH CONCERNS / REWORK verdicts. Its three plan
  dimensions are unchanged (coherence and architectural fit always run;
  unit-of-work runs only for a phase), and the `needs-plan-review` gate is
  unchanged in effect — what used to PASS or PASS WITH CONCERNS now reports
  `reviewed` and clears the tag, and what used to REWORK now reports `rework` or
  `escalated` and leaves it. Per-phase gating under `--roadmap <slug>` and the
  report-only `--implementation-plan` mode (no gate, no mutations) are preserved.
- Plan review now includes a **refute pass**: every finding is graded by a fresh,
  separate read-only agent before it is reported, and refuted or low-confidence
  (<70) findings are dropped. This is deliberate parity with the code review, and
  it means some weakly-evidenced findings that previously held the
  `needs-plan-review` gate closed will no longer do so.
- `rdm-do`'s finalize step now runs an automated code review in **both** modes.
  Previously, interactive `rdm-do` parked the item in `needs-review` for a
  separate review pass to pick up, and `rdm-do --auto --task` finalized with no
  automated review at all. Finalize now invokes the `rdm-review` skill directly
  once the work is committed; the human confirmation gate still decides *whether
  to finalize*, but a review always happens. The review owns the status gate
  (`reviewed` / `in-progress` / `blocked`) and the completion trailer, so
  `needs-review` is now only a transient marker rather than a parking state.
- The autonomous lane's code review now scales to the change. It selects its
  review dimensions from the real branch diff (`git diff main...HEAD`), so
  `tests`, `architecture`, `api-docs`, `changelog`, and `security` run when the
  change actually touches their surface — re-derived on every rework round, so a
  fix that newly touches a public API is reviewed for public API docs. If the
  diff cannot be read, every dimension runs, so coverage only ever fails open.
- The `rdm-review` skill now reports a single outcome — **reviewed**, **rework**,
  or **escalated** — retiring the old PASS / PASS WITH CONCERNS / BLOCKED / FAIL
  quartet. PASS and PASS WITH CONCERNS collapse to `reviewed`, FAIL becomes
  `rework`, and BLOCKED becomes `escalated`; this is the same vocabulary
  `rdm-dispatch-phase` and `rdm-autopilot` already returned, so every surface now
  speaks one language. An **escalated task** is set to `blocked` (with a `[code]`
  reason prefix) rather than being downgraded to `in-progress` — tasks have
  supported `blocked` for some time, and the skill's claim to the contrary was
  stale. Phase behavior is unchanged.
- The `rdm-review` skill's wording is trimmed: the review dimensions, severity
  scale, and verdict rules now live *only* inside the generated "Review
  specification" section (stamped by `scripts/gen-skill-review.sh` from the
  same canonical source `rdm-dispatch-phase`/`rdm-autopilot` use). The
  surrounding Setup/Report/Act/Gate steps no longer restate those definitions
  — they reference the generated section by name instead. No behavior change;
  this only removes duplicated prose that could drift out of sync with the
  canonical definitions.
- `rdm-land` no longer aborts when a reviewed branch is missing its `Done:`
  trailer. Autonomous runs deliberately never write one, so landing now
  synthesizes it via `rdm hook done-line` from the item's identifiers and amends
  it onto the branch tip before rebasing — aborting only when the identifiers are
  unknown or the tip is not the un-landed reviewed commit.

- The autonomous workflow lane's two in-run retry budgets are raised from 1 to 2
  and are now overridable per run. `dispatch-phase` accepts `maxPlanRevise` and
  `maxCodeRework` (default 2 each, counted independently: budget N means N
  reworks after the original attempt, i.e. N + 1 attempts), and autopilot
  forwards them from `--max-plan-revise N` / `--max-code-rework N`. A budget of
  `0` is legal and means "terminate on the first blocking review"; a negative or
  non-integer budget is rejected up front with an actionable message. Autopilot's
  own roadmap-level rework re-dispatch budget (1) and global step budget (50) are
  unchanged. `docs/escalation-protocol.md` § Budgets now documents all four
  budgets, the attempt sequences, and notes that the shipped
  `rdm-core/src/templates/` prose skills remain at 1 pending a follow-up — so
  `agent-config` consumers are unaffected by this change.
- `rdm agent-config` output (both the CLI and `--mcp` variants) now teaches
  tagging as a create-time habit: it tells agents to always pass `--tags` /
  `tags: [...]` when creating a roadmap, phase, or task, warns that `--tags` on
  update *replaces* the existing list, suggests a starting vocabulary of `bug`,
  `enhancement`, `cli`, `core`, `server`, `web-ui`, and `docs` (framed as
  suggestions, not a closed set), and points at `rdm tag list` — instead of the
  previous `rdm search "" --tag <candidate>` — for discovering the tags a project
  already uses before inventing a new one.
- The `rdm-plan-review` skill's Coherence reviewer (CLI and MCP variants) now
  treats a cross-item dependency on unlanded work as non-blocking when the
  target item is annotated: a plan step citing a file or behavior introduced by
  another in-flight (not-yet-landed) roadmap or task is only `blocking` if the
  item does not carry the new reserved `depends-unlanded` tag and does not
  state the dependency explicitly. This closes a false-positive REWORK mode
  where side-tasks filed from inside a roadmap's shared worktree described
  branch-local files as if they already existed on `main`.

### Added

- Tasks can now be `blocked`. `TaskStatus` gains a `blocked` variant, mirroring
  phases: `rdm task update <slug> --status blocked --reason "…"` parks a task with
  a recorded reason (stored in the task's `close_reason`, preserved across later
  status changes and cleared with `--clear-reason`). `blocked` is non-terminal, so
  a blocked task still shows in the default `rdm task list` "active work" view, and
  it is accepted anywhere a task status is parsed (CLI `--status` filter, MCP
  `rdm_task_update`/`rdm_task_list`, and the web status dropdown). This lets the
  autonomous workflow lane park a task the same way it parks a phase.

### Added

- New `autopilot` workflow (`.claude/workflows/autopilot.js`): the active driver
  of the autonomous lane. Given one roadmap slug it runs an estimate pre-pass over
  the roadmap's unestimated phases (one `parallel()` fan-out, persisting each
  difficulty so the model tier auto-derives), then loops `rdm next` → the
  `dispatch-phase` workflow (via the one allowed level of `workflow()` nesting) →
  interpret the OUTCOME → PERSIST status: a `reviewed` phase advances
  (`rdm phase update --status reviewed`), a `rework` phase re-dispatches against a
  per-phase budget and then parks `blocked [code]`, and an `escalated` phase parks
  `blocked [plan]`. It bounds the run with a global step budget and `--max-phases`,
  supports `--plan-only` (each dispatch stops after its plan gate, guarded against
  re-vetting), always ends with a batched summary (phases completed, escalations
  tagged plan/code pointing at `rdm review blocked`, stop reason), and never
  touches `main` — landing stays the separate `rdm-land` skill. `dispatch-phase`
  now accepts a `planOnly` arg and returns early once the plan gate passes. The
  dogfood `rdm-autopilot` skill is rewritten as a thin shim that parses the
  invocation and hands off to the workflow. Gated by
  `scripts/verify-workflow-autopilot.sh` (pure helpers + a state-backed driven
  loop: drive-to-reviewed, rework→park, escalated, budget stops, the estimate
  pre-pass, `--plan-only`, and mid-tier defaulting). Dogfood-only — not emitted by
  `agent-config`.
- New `dispatch-phase` workflow (`.claude/workflows/dispatch-phase.js`): the
  keystone per-phase unit of autonomous execution, a deterministic 4-stage
  pipeline (plan → plan-review → implement → code-review). It seeds a fresh
  implementer with only the phase body plus an independently reviewed plan
  document, runs both review gates through Phase 1's stamped
  `buildReviewPipeline`, bounds itself to at most one plan-revise and one
  code-rework pass, and returns an `{ roadmap, phase, outcome, summary, findings }`
  OUTCOME with `outcome` one of `reviewed` | `rework` | `escalated`. It never
  emits a `Done:` line (landing is a separate, later step). Gated by
  `scripts/verify-workflow-dispatch.sh` (all three outcome branches) plus
  `scripts/verify-workflow-review.sh`. Dogfood-only — not emitted by
  `agent-config`.
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
- rdm now automatically configures a git merge driver so conflicts on auto-generated `INDEX.md`/`projects/*/INDEX.md` files are resolved by regeneration instead of requiring manual `rdm resolve`. `rdm init` writes the tracked `.gitattributes` entries (which travel with clones); every command that opens the plan repo adds the untracked, repo-local `.git/config` driver definition if missing (best-effort — a read-only `.git/config` warns instead of failing the command). (The previously-documented but never-implemented `rdm install-merge-driver` command reference has been removed from the docs — nothing to migrate.)

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
- New `plan_review` config key (`rdm.toml` and global config, `RDM_PLAN_REVIEW`
  env override, default `false`). When enabled, `rdm roadmap create`, `rdm
  phase create`, and `rdm task create` stamp a reserved `needs-plan-review`
  tag onto new items alongside any user-supplied `--tags`, so pending items
  can be listed with `rdm search "" --tag needs-plan-review --type
  phase|task --format json`. Not yet consumed by any skill or hook — this
  lays the sentinel-tag foundation an upcoming plan-review skill will act on.
- New `rdm-plan-review` agent skill (CLI and MCP variants), generated by
  `rdm agent-config --skills` alongside the existing nine skills. Reviews the
  plan of a roadmap, phase, or task (or an in-progress `rdm-do` implementation
  plan via `--implementation-plan`) before implementation begins: it dispatches
  parallel read-only sub-agents for coherence, architectural fit (reading the
  configured principles file, falling back to `CLAUDE.md`/`AGENTS.md`), and,
  for phases, unit-of-work sizing, then consolidates their findings into a
  single **PASS** / **PASS WITH CONCERNS** / **REWORK** verdict. Small findings
  are applied inline to the plan document; large ones are filed as
  `rdm task create ... --tags plan-review` tasks. On PASS or PASS WITH
  CONCERNS it clears the `needs-plan-review` tag (Phase 1's sentinel); on
  REWORK it leaves the tag in place and reports what must change.
- `rdm agent-config claude --hooks` and `rdm agent-config pi --hooks` now also
  emit a plan-review Stop hook/extension (`.claude/hooks/rdm-plan-review-on-create.sh`
  registered in `.claude/settings.json`, or `.pi/extensions/rdm-plan-review.ts`
  for Pi) that reprompts the agent to run the `rdm-plan-review` skill while any
  roadmap, phase, or task carries the `needs-plan-review` sentinel tag (stamped
  when `plan_review` is enabled). It honors `stop_hook_active` the same way the
  existing `needs-review` hook does, and fails open on any query error.
  `merge_stop_hook_into_settings` now composes multiple Stop hooks
  non-destructively and idempotently, so both hooks coexist in the same
  `settings.json`.
- Dogfood the `rdm-plan-review` skill and Stop hook:
  `.claude/skills/rdm-plan-review/SKILL.md` and
  `.claude/hooks/rdm-plan-review-on-create.sh` (registered in
  `.claude/settings.json` alongside the existing
  `rdm-review-on-finalize.sh` Stop hook). Enabled `plan_review = true` for
  rdm's own plan repo, so newly created roadmaps/phases/tasks are tagged
  `needs-plan-review` and gated by `rdm-plan-review` before implementation
  begins, composing with the existing post-implementation
  `rdm-review`/`needs-review` gate.

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
- The `rdm-do` skill now reviews its own drafted implementation plan before execution: after the plan is drafted and before the approval gate, it runs the `rdm-plan-review` skill's new `--implementation-plan` mode (coherence + architectural fit) and surfaces the findings. In `--auto` mode it never waits on the verdict — it folds surviving blocking findings back into the plan text, or files them as a side-work task when they can't be resolved by editing the plan alone. `rdm-plan-review --implementation-plan` is now fully specified: it reviews a plan document handed to it directly (no persisted rdm item), and skips both the tag-gate step and the fix-application half of categorize-and-act, since there is nothing to write to or file against in that mode. The `rdm-roadmap` skill now notes that, when `plan_review` is enabled, newly created roadmaps/phases carry a `needs-plan-review` tag that should be left in place until `rdm-plan-review` clears it.

### Fixed

- The `autopilot` Workflow's estimate pre-pass now persists per-phase difficulty
  and model tiers instead of silently no-opping. Its `[estimate:list]` agent was
  handed a StructuredOutput schema with a top-level `type: 'array'`, which the
  Anthropic tool API rejects (`input_schema.type` must be `'object'`); every call
  400'd, so no tiers were recorded and all phases dispatched at the default
  `medium` tier. The phase-list schema now wraps the array under a `phases` key
  and the pre-pass unwraps it, so a live autopilot run over an unestimated
  roadmap records real Difficulty/Model tiers.
- `rdm roadmap update`, `rdm phase update`, and `rdm task update` no longer read
  stdin or open the interactive editor when neither `--body` nor `--clear-body`
  is given. A tags-only or status-only update (e.g. `rdm task update <slug>
  --status done --no-edit`) previously blocked indefinitely reading stdin when
  stdin was an open pipe that never closed — the exact environment the installed
  `Done:` post-merge/post-commit hooks run in as git subprocesses. Now the body
  is consulted only when explicitly requested: `--body` sets it, `--clear-body`
  clears it, otherwise it is left untouched. **Behavior change:** the previously
  legal `update --tags x < body.md` form (piping a body into an update via stdin)
  is retired — pass `--body` to set body content on an update, which composes
  with `--tags`, `--status`, and the other update flags. `create` is unaffected
  and still accepts a piped-stdin body. The bundled skills and the templates
  emitted by `rdm agent-config --skills` (`rdm-revise`, `rdm-estimate`) that
  previously set a body by piping a heredoc into `update` now pass `--body`
  instead, and the generated agent instructions document the create-vs-update
  body-input difference.
- `rdm serve` now resolves `?at=<sha>` historical-revision requests against the plan repo's real git history by default (when the plan root is a git repository), instead of always returning 404 "the store has no history available." Non-git plan roots keep the previous behavior. Note: this applies to the git-featured build shipped via `rdm-cli` (the default); a standalone `rdm-server` build without the new `git` cargo feature keeps FsStore-only (always-404) behavior.
- `rdm bootstrap doctor` now correctly detects rdm on PATH on Windows by checking for `.exe`, `.bat`, and `.cmd` extensions in addition to the unextended name.
- When running a non-init command against an uninitialized plan repo (no
  `rdm.toml` at the resolved root), the error now clearly guides users to
  `rdm init`: "no plan repo found at {path} — run `rdm init` to create one",
  instead of the opaque "failed to open git repository" error. Commands that
  do not require a repo (`rdm init`, `rdm describe`, `rdm agent-config`, `rdm
  model`, and `rdm bootstrap` and `rdm hook` when git is enabled) proceed
  normally without requiring `rdm.toml`.

### Removed

- **Breaking:** the auto-review Stop hook (`rdm agent-config claude --hooks`'s
  `.claude/hooks/rdm-review-on-finalize.sh`) and its Pi `agent_end` extension
  (`rdm agent-config pi --hooks`'s `.pi/extensions/rdm-review.ts`) are no
  longer generated. Both were a *passive* safety net that re-prompted an agent
  only when an item was left in `needs-review` after implementation; they are
  superseded by *active* review now running on every finalize path
  (`rdm-do`, `dispatch-phase`, and `autopilot` all invoke the canonical review
  before an item can be left in `needs-review`), so the net has nothing left
  to catch. Projects that installed the retired hook/extension via `--hooks`
  should remove `.claude/hooks/rdm-review-on-finalize.sh` and its
  `hooks.Stop` entry in `.claude/settings.json` (or
  `.pi/extensions/rdm-review.ts`) manually — `agent-config` no longer manages
  them. The sibling plan-review Stop hook / Pi extension
  (`rdm-plan-review-on-create.sh` / `rdm-plan-review.ts`), which gates a
  different, still-live concern (the plan, before implementation begins), is
  unaffected and continues to be generated by `--hooks`.

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
