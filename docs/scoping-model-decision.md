# Scoping Model Decision: Session-Scoped Changeset Attribution

## Executive Summary

This document records the decision to implement **Option A: Changeset Attribution** for handling concurrent sessions sharing a single `$RDM_ROOT` plan repository. Sessions are identified by a four-rung resolution chain (highest precedence first: explicit `RDM_SESSION`, harness variables, inherited lease, per-process fallback — reordered by phase 10; see the note below), and mutations are journaled by session so that `rdm commit` builds git trees from HEAD plus only the caller's journaled paths. Another session's uncommitted changes do not affect what lands. This approach is chosen over Option B (commit per mutation) because it preserves readability of the git history while still eliminating session cross-contamination.

## Problem Statement

The rdm CLI mutates a shared plan repository (`$RDM_ROOT`) that may be accessed by multiple concurrent sessions or agents. Without session scoping:

- **Race condition (session cross-contamination)**: Session A stages mutations (writes files to disk) but does not commit. Session B stages its own mutations and commits. The git commit captures both A's and B's changes, even though B's `rdm commit` should only land B's work. This violates the principle that each session controls only its own work product.

- **Staging area semantics**: The current system uses disk-based staging (mutations write directly to the shared working tree). Without session scoping, there is no way to distinguish which uncommitted changes belong to which session.

- **Requirement**: The model must satisfy three properties simultaneously:
  1. **Stable** across processes in one session
  2. **Distinct** between concurrent sessions sharing one `$RDM_ROOT`
  3. **Automatic** when unset (no burden on users or agents to manually configure)

## Option A: Changeset Attribution (Chosen)

**Design:** Each session is assigned a stable, unique changeset identity that outlives individual processes. Mutations are journaled by session — each mutation records the file paths it touches. When `rdm commit` is invoked, it:

1. Identifies the caller's session identity (resolving the four-rung chain below)
2. Retrieves the journal of paths mutated by that session
3. Builds a git tree from HEAD plus only the caller's journaled paths
4. Commits that partial tree

**Consequence:** Another session's uncommitted changes are irrelevant to what lands. If Session A has written `roadmap-a/phase-1.md` to disk but not committed, and Session B commits, the resulting git tree contains only the paths Session B journaled — roadmap-a's uncommitted file is excluded, even though it exists in the working directory. The git log shows a clean history: each commit reflects exactly one session's batch of work.

**Status display:** `rdm status` can be configured to show either the caller's own journaled changeset or the entire working tree, depending on context.

## Option B: Commit Per Mutation (Rejected)

One commit per `task create`, `phase create`, `task update`, etc. This eliminates the staging area entirely and makes cross-session contamination impossible by definition — there is nothing to race on.

**Rejection rationale:**

The strongest argument for this option is real and must be recorded exactly as follows:

> There is no staging area, so there is nothing to race on and nothing for an agent to misunderstand — the confusion goes away by deletion rather than by better documentation.

This simplification by deletion is genuine. However, Option B loses on history quality, not on implementation cost. The rdm project has had this behavior and dropped it because one commit per `task create` makes the log unreadable as a narrative. Users expect to batch related mutations (a roadmap plus all its phases, or a status update plus a follow-on task) into a single commit message that describes the unit of work. Option A preserves that quality while still solving the concurrency problem.

## Session Identity Chain

Session identity is resolved in this order (highest precedence first). The spellings below are **binding**, not examples; implementations must use these exact terms.

> **Rung labels vs. evaluation order.** The "Rung 1"–"Rung 4" labels below (and `Rung::number()` / the CLI's `rung` JSON field) are fixed, binding identifiers for each mechanism — they do **not** describe the order in which `resolve_id` checks them. Phase 3 originally specified checking rung 2 (inherited lease) before rung 3 (harness variable), matching the section order below. Phase 10 amended this: the actual checked order is **1 (explicit) → 3 (harness) → 2 (lease) → 4 (per-process)**. A harness-published session id is an explicit statement of session membership and must outrank an inherited on-disk lease — see the Rung 3 "Precedence" bullet below and `docs/session-identity.md` § "Why rung 3 now precedes rung 2, and lease creation still defers to it" for the mechanism and the merging bug this fixed.

### Rung 1: Explicit `RDM_SESSION` (Always Wins)

An environment variable `RDM_SESSION=<value>` set by the invoker always takes precedence. This is the escape hatch for scripts and CI systems that need deterministic, reproducible identities.

- **Properties**: Stable (caller controls it), Distinct (each caller can pick a unique value), Automatic (only if explicitly set; optional)

### Rung 2: Inherited Lease (Pid + Start Time)

When `RDM_SESSION` is not set, rdm resolves a long-lived ancestor process and uses its PID plus start time as a session key. This provides a stable lease that outlives individual child processes (such as a shell spawned by an agent, or an agent task spawning `rdm` subprocesses).

- **Key composition**: `<ancestor_pid>:<ancestor_start_time>` (the exact separator and format are phase 4's responsibility)
- **Ancestor selection**: Not just the immediate parent (which may be an ephemeral shell), but the nearest process that represents a long-lived session. This requires ancestry traversal beyond what `std::os::unix::process::parent_id()` provides, which returns only one level. The start time must be obtained from the same ancestor to ensure uniqueness.
- **Mechanism determination**: Phase 4 picks the exact traversal rule, but the pid+start-time key itself is binding here.
- **Properties**: Stable (fixed for all processes in one session), Distinct (different sessions/invocations have different start times), Automatic (no user configuration needed)

### Rung 3: Documented Harness Session Variables

An extensible list of harness-specific environment variables (e.g., `CLAUDE_CODE_SESSION_ID` from Claude Code) can provide session identity if set. This is an adoption path: tools that already track their own session IDs can be wired in here.

- **Precedence**: Lower than explicit `RDM_SESSION`; higher than the inherited lease (as of phase 10) and the fallback. Without this, a parent shell that ran one bare, harness-less `rdm` invocation would mint a lease at that parent, and every child launched under it — regardless of its own distinct harness session id — would silently inherit that lease and merge onto one changeset. See `docs/session-identity.md` for the reproduction and the fix.
- **Properties**: Stable (harness maintains it), Distinct (harness-specific, not reused across tools), Automatic (if the harness sets it)

### Rung 4: Per-Process Changeset (Always Resolves)

When none of the above provide a session identity, each process gets a fresh, unique changeset. This is the fallback that always resolves and ensures the model never fails to make progress.

- **Consequence**: Mutations from this process are never combined with other processes' work in a single `rdm commit` (since they have no shared session identity), and they will not be committed if another session calls `rdm commit` first.
- **Properties**: Stable (for a single process only), Distinct (by definition, each process is unique), Automatic (requires no configuration; rung 4 always resolves)

### Why Not the RDM_PROJECT Model for Session Identity

The `RDM_PROJECT` precedence chain (flag > env > config, terminating in repo-global `default_project`) is **not** a suitable model for session identity. When two concurrent sessions resolve to the same `default_project`, they both get the same project identifier — but they are distinct sessions and must have distinct session identities. Session identity instead must use a mechanism that is globally unique across all invocations (explicit override or process ancestry), ensuring that every session gets a distinct identity unless the user explicitly opts into grouping via rung 1. This asymmetry is intentional: project selection is user-scoped (one user may work on multiple projects), whereas session identity is session-scoped (one session is one session, period).

## Key Decisions (Binding on Phase 4)

The following decisions are binding and constrain phase 4's implementation. Phase 4 is responsible for the items marked "phase 4 to decide" below.

### Session Identity is Mandatory

Every mutation and commit operation must resolve a session identity by walking the four-rung chain above. The identity must be stable across the lifetime of that session, distinct from other concurrent sessions, and automatically determined without user intervention.

### Ancestry Traversal Required

The inherited-lease mechanism (rung 2) requires traversing multiple generations of parent processes to find the long-lived ancestor, not just calling `std::os::unix::process::parent_id()` (which returns only the immediate parent). This is necessary because an agent may spawn an ephemeral shell to run `rdm`, and that shell's parent is not the long-lived session — the grandparent (or further ancestor) is. The ancestor's start time must also be obtained to ensure the lease key is globally unique.

- **Measured limitation**: An analysis on 2026-08-04 confirmed that `parent_id()` alone is insufficient and that ancestry traversal beyond the standard library is required.

### Stopping Rule (resolved by phase 4)

At some point in the ancestry chain, traversal must stop — we cannot walk all the way to PID 1. The choice of where to stop involves asymmetric failure modes:

- **Stopping too HIGH** (walking too far up, toward the root): Merges concurrent sessions into one session identity. Two independent agents would share the same lease key and their work would be combined in a single `rdm commit`. This reintroduces the race condition this roadmap exists to eliminate. ⚠️ Unacceptable.
- **Stopping too LOW** (stopping early, staying close to the invoking process): Fragments a single session's work across multiple changesets. If the session has multiple child processes (e.g., an agent spawning multiple tool invocations), each might get its own session identity by accident, and their work would be committed separately. This is suboptimal but does not reintroduce cross-contamination. ✓ Acceptable.

**Preferred asymmetry**: Prefer stopping low. Fragmenting one session's batch is less harmful than merging two sessions' work.

**Who decides**: Phase 4 determined the exact stopping rule — see `docs/session-identity.md` § "The shipped stopping rule". The asymmetric-failure principle above is binding on it.

**Phase 11 amendment (creation anchor only).** Phase 11 re-opened the CREATION half of that rule — whether a lease could be minted at the nearest non-shell ancestor instead of the immediate parent, so that an agent harness spawning one wrapper shell per tool call would not fragment — and, on measurement, **left it unchanged**. The asymmetry above is what decided it: the captured ancestries showed the proposed ascent selecting one common anchor for two independent sessions, i.e. the ⚠️ unacceptable direction. Nothing here is revised: the binding asymmetry, the rung numbering, and the adoption half of the rule all stand exactly as written. The evidence, the rejected alternatives, and the per-harness continuity table live in `docs/session-identity.md` § "Continuity across ephemeral wrapper shells (phase 11)".

### Git Hooks and Session Identity

When a git hook (such as `post-merge` or `post-commit`) is spawned as a subprocess of an rdm operation (e.g., an agent's own `git merge --ff-only` — the `rdm-land` fast-forward — fires `post-merge`, which runs `rdm hook post-merge` under that agent's lease; `rdm commit` itself never fires a hook), the hook inherits the session identity from its parent rdm process and joins the same session's changeset. This is correct behavior — the hook's work is part of the same logical transaction.

When a git hook is spawned outside of any rdm-initiated git operation (e.g., a user runs `git merge` manually), the hook has no inherited session identity and resolves to rung 4 (per-process changeset). In this case, the hook's mutations are isolated to that process and are not combined with other sessions' work.

**Degradation guarantee**: The identity resolution chain must not assume the `RDM_GIT_SUBPROCESS` environment variable or any other short-circuit flag was fired. If a hook cannot determine a session identity through the normal four-rung chain, it must degrade gracefully to rung 4, not error or hang. This is critical for reliability on the bounded `hook_timeout_secs` path.

### Which Interaction Layers Are Covered

**CLI**: Fully covered. The `rdm` command-line tool reads/mutates/commits, and all three operations are scoped to the caller's session identity.

> **2026-09-12 — the MCP server was removed.** It previously exposed read/mutate/commit operations with the same session identity semantics as the CLI, including batching mutations and committing them under a single session. That surface is gone; script against the CLI directly, or drive `rdm-server`'s REST API.

**rdm-server**: **Settled by phase 5 — staging-only by default, autocommit opt-in.**

The REST API server mutates the same plan repository through `ops::mutate`, at 22 call sites across six handler modules, and had zero production committers. Left alone under session scoping its writes would have been journaled, excluded from every CLI commit, and stripped from the committed index while staying readable on disk forever — landed never, with no error. The commit-call-site gate cannot catch that: a surface with zero committers passes it vacuously.

Three dispositions were considered:

- **(a) Commit per request through the scoped path.** Rejected as the *default*: a viewer/editor that commits on every PATCH turns ordinary browsing into history noise, and an operator running the server against a shared plan repo has no way to batch. Retained as an explicit opt-in, because a headless deployment with no human at a CLI genuinely needs it.
- **(b) Staging-only with a documented reconciliation command.** **Chosen as the default.** The server is one long-lived process and therefore one session: it resolves a changeset id at startup, journals every mutation to it, and never commits on its own.
- **(c) Deliberately excluded from the model.** Rejected: exclusion is exactly the silent-loss shape above.

What ships:

- `MutationPolicy::StagingOnly` (default) / `MutationPolicy::Autocommit` (`--autocommit`, or `RDM_SERVER_AUTOCOMMIT=1`), in `rdm-server/src/state.rs`. The argv/env precedence lives in `ServerOptions::resolve`, split out of `main` so it is unit-testable rather than hand-verified.
- The changeset id is resolved at startup (`--changeset <id>`, else `RDM_SESSION`, else the ordinary rung chain). It is not merely *advertised*: `AppState::store()` passes it to the store factory, which pins it via `GitStore::with_session_id`, so the writes really do journal to the id the responses name. (Passing it down rather than exporting `RDM_SESSION` is deliberate — mutating process-global environment state is `unsafe` on a running server, and racy besides.)
- All 22 handler sites call ONE shared `AppState::post_mutate()` helper rather than a commit primitive, so the policy lives in a single place and the commit-call-site gate stays satisfied. It is called **after** `ops::mutate` returns (the store holds staged writes in memory until `ops::mutate`'s own `Store::commit` flushes them, so a commit attempted from inside the closure would see neither the write nor its journal entry) and **unconditionally** for a mutation.

**Loud per mutation, not per process.** The acceptance criterion this disposition answers to is that a server mutation *reaches a commit or fails loudly*. Under the staging-only default it does not reach a commit, so the "loudly" half has to carry the whole weight — and a boot line alone does not, because a server that has been up for a week has long scrolled it away. Staging is therefore reported on three surfaces:

1. **At boot** — a `WARN` naming the changeset and the reconciliation command.
2. **On every mutation** — `post_mutate` emits the same warning to stderr for *that* mutation.
3. **On every mutating response** — the `X-Rdm-Staged` header carries that warning text verbatim, so an HTTP client that never sees the server's stderr is told anyway. Safe methods (`GET`/`HEAD`/`OPTIONS`) staged nothing and are not tagged.

Under `--autocommit` the mutation does reach a commit, so none of the three fire. An autocommit that *fails*, or that lands nothing because the changeset claims no paths, is reported as an `ERROR` naming the reconciliation command — the write is on disk and attributed either way, so a failure degrades to the staging-only case rather than losing anything, and never fails the request.

**The consequence surfaced to the operator**, verbatim from the boot line:

```text
WARN: mutations are staged, not committed. They are attributed to changeset
'<id>' and are surfaced on every mutating response as X-Rdm-Changeset /
X-Rdm-Staged. Reconcile with: rdm commit --changeset <id>
```

Gated by `rdm-server/tests/mutation_policy.rs` (a real bound listener over a real git-backed plan repo: the staging default does not advance HEAD and does report itself; `--autocommit` lands a commit scoped to this changeset's own paths) and by `ServerOptions`' unit tests in `rdm-server/src/state.rs`. `rdm-cli/tests/cli_commit.rs`'s `commit_by_changeset_id_is_the_orphan_recovery_path` and `status_defaults_to_the_callers_changeset_and_all_shows_everything` gate the reconciliation half with real separate `rdm` processes under distinct session ids (see [`test-migration-inventory.md`](test-migration-inventory.md) § 9, row E3).

Phase 7 owns the agent-facing instruction rewrite for this surface.

## Phase 4 Implementation (Completed)

### Journal Implementation Details (Decided)

Phase 4 has implemented the session-scoped journal mechanism with the following concrete decisions:

- **On-disk layout**: Journals are stored in the `.git/rdm/` directory (outside the `$RDM_ROOT` committable tree), alongside git metadata. This placement ensures journals are not accidentally committed and are tied to the repository's lifecycle.
- **Serialization format**: JSONL (JSON Lines) is the format. Each line is a complete JSON record representing one journaled mutation, enabling streaming and append-only writes.
- **Implementation**: Core session and journal logic resides in `rdm-core/src/session/` (see `journal.rs`, `lease.rs`, `process.rs`, `mod.rs`), and CLI surface is in `rdm-cli/src/commands/session.rs`.
- **Granularity**: The atomic unit is per-mutation — each create, update, or delete operation writes a journal entry recording the file paths it touched.
- **Cleanup and lifecycle**: Journals are retained until explicitly discarded. `rdm session gc` removes leases whose owning process is gone or whose pid was recycled and never touches a journal; an orphaned changeset is recovered with `rdm session adopt <id>`. Cleanup behavior can be refined in later phases based on operational experience.
- **CLI surface for session management**: 
  - `rdm session id` — display the current session identity
  - `rdm session journal` — inspect journaled paths for the current session
  - `rdm session list` — list all known sessions and their metadata
  - `rdm session adopt <session-id>` — adopt an existing session identity
  - `rdm session discard <id> --force` — delete a changeset's journal
  - `rdm session gc` — garbage-collect orphaned sessions

## Open Questions (Deferred to Phase 5 or Later)

### INDEX.md Consistency in Partial Commits — **resolved by phase 5, completed by phase 8; mechanism deleted**

> **Historical — the mechanism below no longer exists.**
> `retire-generated-index` phase 4 deleted `reconcile_derived`, the
> orphaned-subtree prune, `ChangesetScope::derived` and
> `rdm_core::paths::is_derived_path` outright. With them went the
> `tree ⊇ index` divergence this section records as an accepted cost: it
> **ceased to exist** rather than being maintained. Nothing regenerates an
> index at commit time, so a commit reads no project, cannot fail to find one,
> and produces no index for a divergence to open up in. An `INDEX.md` is now
> an ordinary file, committed only as bytes some session wrote.
>
> The section is retained, not deleted, because it is the binding record of
> why `plan-repo-concurrency`'s phases 5 and 8 — a separate, already-completed
> roadmap, not a phase of `retire-generated-index` — shaped the mechanism the
> way they did. **Everything below is in the past tense and describes code
> that has been removed.** See [`docs/index-removal.md`](index-removal.md).

**How consistency was maintained:** INDEX.md was auto-generated from individual roadmap, phase, task, and review files — a computed artifact, not a source of truth. When a partial commit (from HEAD + caller's journaled paths) was created, the INDEX.md in that commit reflected exactly the entities that existed in that tree.

**Phase 4's contribution:** The session journal (described above) recorded all paths touched by each mutation, including INDEX.md itself. When a partial tree was built from HEAD + journaled paths, INDEX.md was included if and only if this session had regenerated it.

**The hazard.** Without this scoping, INDEX.md would have carried dangling references. When Session A had written a task file to disk but not committed, and Session B committed:
1. Session B's partial tree correctly excluded Session A's task file (it was not in B's journal).
2. If Session B's INDEX.md had been taken from the *live filesystem*, it would have included A's task entity — the on-disk index was regenerated by every mutation and so already held every session's rows.
3. The committed tree would have been inconsistent: INDEX.md referencing an entity whose file is absent.

**The mechanism that shipped (`GitRepo::reconcile_derived`, `rdm-store-git/src/commit.rs` — since deleted).** A derived blob was **never read from disk**. Before the scoped tree was built:

1. HEAD's document bytes were materialized (`collect_blobs_at`); non-UTF-8 blobs were skipped for the projection but kept their HEAD oid in the tree, so nothing was dropped from the commit.
2. This changeset's non-derived writes and deletes were applied on top, in memory.
3. **(phase 8)** Every `projects/<p>/` subtree whose `projects/<p>/project.md` was absent from that projection was dropped before generation. Such a parent was owned by a *third* session that had not committed yet, so it was visible to neither HEAD nor this changeset. Without the drop, `list_reviews`/`list_roadmaps`/`list_tasks` — all of which check the `project.md` sentinel before enumerating — raised `ProjectNotFound`, and the whole commit aborted with a misleading `project not found: <p>` for a project the user had just created.
4. That projection seeded a `rdm_core::store::MemoryStore`, and `rdm_core::ops::index::generate_index` ran against it.
5. **Only** the derived paths this changeset had journaled were taken back and written into the tree. A `projects/<p>/INDEX.md` journaled for a subtree step 3 pruned was never generated, so it was simply not produced — which is what kept the commit free of an orphan project index.

Derived paths the changeset did **not** journal stayed at their HEAD oid, so an unrelated project's index was never silently rewritten by an unrelated session's commit.

The mechanism was deliberately a *projection*, not a journal-scoped `Store` view: the projection was HEAD + this changeset, which is exactly the tree being committed. The guarantee that bought was **one-directional**: the index could never name a path the tree did not contain — no dangling row, and no `projects/<p>/INDEX.md` for a project whose `project.md` the commit lacked. The converse did **not** hold: a document whose project was owned by an uncommitted third session landed in the tree while the index carried no row for it (see below). It was deterministic by construction — ordered maps and sets throughout, no timestamps, no hash-iteration ordering — pinned by unit tests asserting that committing the same changeset twice against the same HEAD yielded identical tree oids.

#### What the drop cost, and why it was chosen

The divergence step 3 introduced was `tree ⊇ index`, never the reverse. A dangling row is a corrupt artifact; an omitted row is a stale-but-valid one, and the omission was repaired with no user action: `journal::truncate` trims only the paths a commit actually landed, so the deferred `projects/<p>/INDEX.md` stayed in the deferring session's changeset, and the moment the owning session committed its `project.md`, that session's own `reconcile_derived` seeded from a HEAD which now held the deferred document and regenerated every row. (`rdm index` also rebuilt unconditionally.) This rested on truncation staying landed-paths-only — if it had ever widened to cover journaled-but-unlanded paths, the heal would have broken and the trade would have become indefensible.

**The accepted consequence, stated explicitly:** the committing session's own new document was absent from the index *it* committed. That was accepted, because the alternative — emitting `projects/<p>/INDEX.md` for a project whose manifest the commit does not contain — was exactly the orphan the guarantee above forbade. Nothing was lost from the tree: the document itself still landed.

**None of this is live any more.** A commit takes the bytes a session wrote and nothing else. `rdm index` itself is gone (`retire-generated-index` phase 5; see [`index-removal.md`](index-removal.md)) — nothing generates or repairs an INDEX.md any longer, and any copy still tracked from before removal is just an ordinary file like any other.

Two alternatives were rejected at the time:

- **Fail accurately** — keep the hard failure but name the real cause ("another uncommitted session owns `projects/<p>/project.md`"). Rejected: an accurate error is still a blocked workflow, and the blocked workflow was a routine one (one session creates the container, another adds content under it before the first lands) whose only recovery was to go ask the other session to commit.
- **Synthesize a placeholder `project.md` into the seed** so the project still generates. Rejected: it would have emitted a `projects/<p>/INDEX.md` for a project whose `project.md` the commit did not contain — the orphan case above.

Two shapes were out of scope by construction rather than by defense: a `projects/<p>/INDEX.md` already in HEAD without its manifest, and a changeset that *deletes* a `project.md` while HEAD keeps the rest of the subtree. Both would have left an inherited index in the tree, and both were unreachable through rdm — a `project.md` is always created and committed alongside its index, and rdm has no project-delete command at all. The prune was deliberately scoped to what generation saw; it never deleted an inherited path from the tree, because deleting inherited paths is precisely the sweeping behavior scoping exists to prevent.

### Merge Driver: Out of Scope (Correctly) — **superseded: the driver is gone**

> **Superseded by `retire-generated-index/phase-3-retire-merge-driver`.** The
> `rdm-index` merge driver has been retired outright, in both halves: rdm no
> longer writes `.gitattributes` and no longer installs a
> `[merge "rdm-index"]` section in `.git/config`. With mutations no longer
> regenerating `INDEX.md` (phase 2), the driver had no job left — it existed
> solely to resolve conflicts on a file rdm rewrote from both sides of a
> merge. `INDEX.md` now merges with git's built-in three-way merge; see
> [`docs/file-formats.md`](file-formats.md) § "INDEX.md and merges". The
> reasoning below is retained because it remains the binding record of *why*
> the driver was orthogonal to changeset scoping, which is what let it be
> removed without touching the scoping model.

The rdm-index merge driver (`rdm-store-git/src/repo.rs`) automatically regenerated `INDEX.md` when git detected conflicts during a merge. However, changesets never perform a merge — a changeset-scoped commit constructs its tree from HEAD plus the caller's journaled paths (a direct write operation, no three-way merge). Therefore, the rdm-index merge driver was not involved in changeset-scoped commits and was orthogonal to this decision. It remains orthogonal as long as phase 5's partial-tree mechanism avoids merging.

### `rdm status` and `rdm discard` — **resolved by phase 5**

**`rdm status`** shows the caller's changeset by default, with `--all` for the whole tree. The two views come from ONE `StatusReport` partitioned in ONE pass into `user` / `others` / `unattributed`, so the view a user reads and the set `rdm commit` will land cannot drift apart. `is_clean()` still means "nothing at all differs" (the correct gate for the whole-tree actions); `is_changeset_clean()` is the new scoped gate. `others` and `unattributed` are named in the output rather than hidden. *(A fourth `derived` bucket, for the indexes rdm regenerated, was removed by `retire-generated-index` phase 4 along with the derived-path class itself; a dirty `INDEX.md` is now an ordinary entry in whichever of the three buckets owns it.)*

**`rdm discard` — the design that shipped.** Default `rdm discard --force` is **changeset-scoped**:

1. restore only this changeset's journaled paths to HEAD (added ones removed, modified/deleted ones written back);
2. clear this changeset's journal.

> **Superseded in part by `retire-generated-index`.** As shipped there was a
> third step — "regenerate the derived indexes **from the resulting disk
> state**, so another session's still-uncommitted rows survive, and journal
> that regeneration to this (now empty) changeset" — and a fourth,
> "re-ensure the `.gitattributes` merge-driver mapping, reported as
> `reinstalled:`". Phase 2 retired the third (a discard restores an
> `INDEX.md` this changeset claims rather than regenerating it — regenerating
> would re-dirty what the discard just cleaned) and phase 3 retired the fourth
> along with the merge driver itself; phase 4 then deleted the derived-path
> class the third step named, so step 1 above covers an `INDEX.md` with no
> qualifier. The discard now leaves a tree that matches HEAD exactly, and
> `reinstalled:` is no longer a line `rdm discard` can print.

The whole-tree behavior is retained behind an explicit `--all`, which requires `--force` as well and **first names every other live changeset it is about to destroy** (from `journal::list_changesets`). The scoped path lives on `GitStore`, not in the CLI, so every store-backed caller inherits identical behavior.

Rejected alternative: leaving discard whole-tree and warning. A destructive default that silently deletes a concurrent session's added files is the same class of defect as a sweeping commit, and a warning does not undo it.

## Phase 5 Implementation (Completed)

### The choke point is `create_git_commit`, not `commit_now`

Attribution lives in the tree builder itself. `GitRepo::create_git_commit` takes an explicit `CommitScope`:

- `CommitScope::WholeTree` preserves the historical behavior byte-for-byte (rebuild from disk, one attempt, no compare-and-swap), and is reachable from outside `rdm-store-git` only through the explicitly-named `GitStore::commit_whole_tree`.
- `CommitScope::Changeset(&ChangesetScope)` seeds a flat `path -> blob oid` map from HEAD and applies only the named changeset. A path no one named is never read, never blobbed, and never enters the tree — unreachable rather than filtered.

Both tree builders share one entry-sort comparator (`sort_tree_entries`), so identical content yields an identical tree oid either way and the `treeNotSorted` hazard cannot be re-derived in the new path.

`GitRepo::git_commit` was demoted to `pub(crate)`, so a new out-of-crate committer fails to compile. `scripts/verify-scoped-commit.sh` previously backed that in-crate with an allowlist grep over every commit-primitive call site (with both self-test arms); that section (§ D) was retired by the operator amendment to the `retire-static-grep-harnesses` plan (2026-09-23), which extended grep-only-harness retirement to Rust-source greps as well as prose. Test-seeding callers (`rdm-store-git`'s `#[cfg(test)]` module, `rdm-server/tests/git_history.rs`) remain a legitimate whole-tree class.

`rdm resolve`/pull paths that shell out to a real `git commit`/`git merge` still create whole-tree commits outside this mechanism. That is correct — they are *merge* operations, not session commits — and is stated here so the allowlist is understood rather than quietly widened.

### Journal truncation is correctness

A successful scoped commit removes the landed paths from the journal (`journal::truncate`). Without it, a session's *second* commit would re-include an already-landed path — and by then another session may have edited it, so the re-commit would sweep work this session never did.

### Concurrency: what phase 5 closes, and what it does not

The ref update is a compare-and-swap against the HEAD the tree was seeded from, with one rebuild-and-retry; a second mismatch is an explicit, actionable error, never a lost commit. A best-effort, age-bounded advisory lock under the git dir (5 s wait, 30 s staleness takeover — both far inside the default 30 s `hook_timeout_secs`) narrows the window in the common case; failing to take it proceeds rather than erroring, because the compare-and-swap is the actual correctness mechanism.

Two residuals were deliberately left to **phase 6**:

- **Still open.** A concurrent committer can land between a successful compare-and-swap and this process's post-commit index sync.
- **Closed by phase 6.** Attribution was **path-level, not content-level**: if another session overwrote a path this changeset journaled, the scoped commit committed the other session's bytes for that path. Phase 6 added base-blob identity to the journal (`JournalEntry::digest`) and a matching check in the scoped tree builder, which now refuses with `Error::ChangesetPathOverwritten` rather than committing another session's content. That is the commit-time half of a two-part mechanism; the flush-time half prevents the overwrite happening at all. The evaluation that selected it is [`docs/lost-update-evaluation.md`](lost-update-evaluation.md).

The advisory lock described above is now `rdm_core::lock::AdvisoryLock`, shared with the filesystem store's flush lock so the two cannot diverge. Its durations still live at each call site, because the commit path shortens them under `cfg(test)` and the flush path does not.

## Appendix A: Measured Data

**Measurement date**: 2026-08-04

- **Parent ID limitation**: `std::os::unix::process::parent_id()` returns only the immediate parent process. For an agent that spawns a shell to run `rdm`, the shell's parent is the ephemeral process, not the long-lived agent session. Reaching the session requires ancestry traversal.
- **Necessity of start time**: PID reuse on Unix means that a process ID alone is not globally unique across time. The start time must be combined with the PID to ensure the lease key is unique.
- **Lease key composition**: The inherited lease is identified by the combination **pid plus start time** of the nearest long-lived ancestor process. This composite key is globally unique across all invocations and ensures distinct sessions even when process IDs are reused.

## Appendix B: Implementation Notes

### Binding Spellings

The following environment variable names and terms are **not** up for reinterpretation:
- `RDM_SESSION` — the explicit escape hatch
- `CLAUDE_CODE_SESSION_ID` — example of harness variable; extensible list in phase 4
- `inherited lease` — rung 2's mechanism name
- `per-process changeset` — rung 4's mechanism name

### Journal Isolation

Once a session's journal is created (or inherited from a parent), all mutations within that session are written to the same journal. The journal is not shared across sessions. This is the core isolation mechanism.

### Partial Tree Commitment

When `rdm commit` is called:
1. The git tree is constructed from two sources:
   - The current HEAD tree (the baseline)
   - The caller's journaled paths (the additions/changes)
2. This hybrid tree is what is committed to git. It is a clean snapshot of the work from one session only.
3. Any uncommitted changes from other sessions are excluded from this tree by definition.

### Determinism and Idempotency

The session identity resolution must be deterministic: invoking `rdm` twice in the same process context must resolve to the same session identity. This is necessary for batching and for reliable error recovery.
