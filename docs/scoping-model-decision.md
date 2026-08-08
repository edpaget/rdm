# Scoping Model Decision: Session-Scoped Changeset Attribution

## Executive Summary

This document records the decision to implement **Option A: Changeset Attribution** for handling concurrent sessions sharing a single `$RDM_ROOT` plan repository. Sessions are identified by a four-rung resolution chain (highest precedence first: explicit `RDM_SESSION`, inherited lease, harness variables, per-process fallback), and mutations are journaled by session so that `rdm commit` builds git trees from HEAD plus only the caller's journaled paths. Another session's uncommitted changes do not affect what lands. This approach is chosen over Option B (commit per mutation) because it preserves readability of the git history while still eliminating session cross-contamination.

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

- **Precedence**: Lower than explicit `RDM_SESSION` and inherited lease, higher than the fallback
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

### Stopping Rule (Phase 4 to Decide)

At some point in the ancestry chain, traversal must stop — we cannot walk all the way to PID 1. The choice of where to stop involves asymmetric failure modes:

- **Stopping too HIGH** (walking too far up, toward the root): Merges concurrent sessions into one session identity. Two independent agents would share the same lease key and their work would be combined in a single `rdm commit`. This reintroduces the race condition this roadmap exists to eliminate. ⚠️ Unacceptable.
- **Stopping too LOW** (stopping early, staying close to the invoking process): Fragments a single session's work across multiple changesets. If the session has multiple child processes (e.g., an agent spawning multiple tool invocations), each might get its own session identity by accident, and their work would be committed separately. This is suboptimal but does not reintroduce cross-contamination. ✓ Acceptable.

**Preferred asymmetry**: Prefer stopping low. Fragmenting one session's batch is less harmful than merging two sessions' work.

**Who decides**: Phase 4 determines the exact stopping rule (e.g., "stop at the first process with a session variable set", "stop at the session leader", "traverse up to a known harness boundary"), but the asymmetric-failure principle above is binding.

### Git Hooks and Session Identity

When a git hook (such as `post-merge` or `post-commit`) is spawned as a subprocess of an rdm operation (e.g., `rdm commit` runs `git merge --ff-only`, which triggers the `post-merge` hook), the hook inherits the session identity from its parent rdm process and joins the same session's changeset. This is correct behavior — the hook's work is part of the same logical transaction.

When a git hook is spawned outside of any rdm-initiated git operation (e.g., a user runs `git merge` manually), the hook has no inherited session identity and resolves to rung 4 (per-process changeset). In this case, the hook's mutations are isolated to that process and are not combined with other sessions' work.

**Degradation guarantee**: The identity resolution chain must not assume the `RDM_GIT_SUBPROCESS` environment variable or any other short-circuit flag was fired. If a hook cannot determine a session identity through the normal four-rung chain, it must degrade gracefully to rung 4, not error or hang. This is critical for reliability on the bounded `hook_timeout_secs` path.

### Which Interaction Layers Are Covered

**CLI**: Fully covered. The `rdm` command-line tool reads/mutates/commits, and all three operations are scoped to the caller's session identity.

**MCP**: Fully covered. The MCP server exposes read/mutate/commit operations with the same session identity semantics as the CLI. An MCP client can batch its mutations and commit them under a single session.

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

Gated by `rdm-server/tests/mutation_policy.rs` (a real bound listener over a real git-backed plan repo: the staging default does not advance HEAD and does report itself; `--autocommit` lands a commit scoped to this changeset's own paths) and by `ServerOptions`' unit tests in `rdm-server/src/state.rs`. `scripts/verify-scoped-commit.sh` § E3 gates the reconciliation half with real separate processes.

Phase 7 owns the agent-facing instruction rewrite for this surface.

## Phase 4 Implementation (Completed)

### Journal Implementation Details (Decided)

Phase 4 has implemented the session-scoped journal mechanism with the following concrete decisions:

- **On-disk layout**: Journals are stored in the `.git/rdm/` directory (outside the `$RDM_ROOT` committable tree), alongside git metadata. This placement ensures journals are not accidentally committed and are tied to the repository's lifecycle.
- **Serialization format**: JSONL (JSON Lines) is the format. Each line is a complete JSON record representing one journaled mutation, enabling streaming and append-only writes.
- **Implementation**: Core session and journal logic resides in `rdm-core/src/session/` (see `journal.rs`, `lease.rs`, `process.rs`, `mod.rs`), and CLI surface is in `rdm-cli/src/commands/session.rs`.
- **Granularity**: The atomic unit is per-mutation — each create, update, or delete operation writes a journal entry recording the file paths it touched.
- **Cleanup and lifecycle**: Journals are retained until explicitly discarded. The `rdm session gc` command (CLI surface) provides cleanup and orphaned changeset recovery. Cleanup behavior can be refined in later phases based on operational experience.
- **CLI surface for session management**: 
  - `rdm session id` — display the current session identity
  - `rdm session journal` — inspect journaled paths for the current session
  - `rdm session list` — list all known sessions and their metadata
  - `rdm session adopt <session-id>` — adopt an existing session identity
  - `rdm session discard` — mark the current session for cleanup
  - `rdm session gc` — garbage-collect orphaned sessions

## Open Questions (Deferred to Phase 5 or Later)

### INDEX.md Consistency in Partial Commits — **resolved by phase 5**

**How consistency is maintained:** INDEX.md is auto-generated from individual roadmap, phase, task, and review files — it is a computed artifact, not a source of truth. When a partial commit (from HEAD + caller's journaled paths) is created, the INDEX.md in that commit reflects exactly the entities that exist in that tree.

**Phase 4's contribution:** The session journal (described above) records all paths touched by each mutation, including INDEX.md itself. When a partial tree is built from HEAD + journaled paths, INDEX.md is included if and only if this session regenerated it.

**The hazard.** Without this scoping, INDEX.md would carry dangling references. When Session A has written a task file to disk but not committed, and Session B commits:
1. Session B's partial tree correctly excludes Session A's task file (it was not in B's journal).
2. If Session B's INDEX.md were taken from the *live filesystem*, it would include A's task entity — the on-disk index is regenerated by every mutation and so already holds every session's rows.
3. The committed tree would be inconsistent: INDEX.md references an entity whose file is absent.

**The shipped mechanism (`GitRepo::reconcile_derived`, `rdm-store-git/src/commit.rs`).** A derived blob is **never read from disk**. Before the scoped tree is built:

1. HEAD's document bytes are materialized (`collect_blobs_at`); non-UTF-8 blobs are skipped for the projection but keep their HEAD oid in the tree, so nothing is dropped from the commit.
2. This changeset's non-derived writes and deletes are applied on top, in memory.
3. That projection seeds a `rdm_core::store::MemoryStore`, and `rdm_core::ops::index::generate_index` runs against it.
4. **Only** the derived paths this changeset journaled are taken back and written into the tree.

Derived paths the changeset did **not** journal stay at their HEAD oid, so an unrelated project's index is never silently rewritten by an unrelated session's commit.

The mechanism is deliberately a *projection*, not a journal-scoped `Store` view: the projection is HEAD + this changeset, which is exactly the tree being committed, so what the index describes and what the tree contains cannot diverge. It is deterministic by construction — ordered maps throughout, no timestamps, no hash-iteration ordering — pinned by a unit test asserting that committing the same changeset twice against the same HEAD yields identical tree oids, and gated end-to-end by `scripts/verify-scoped-commit.sh` § F (including a self-test proving a disk-sourced derived blob would be caught).

### Merge Driver: Out of Scope (Correctly)

The rdm-index merge driver (`rdm-store-git/src/repo.rs`) automatically regenerates `INDEX.md` when git detects conflicts during a merge. However, changesets never perform a merge — a changeset-scoped commit constructs its tree from HEAD plus the caller's journaled paths (a direct write operation, no three-way merge). Therefore, the rdm-index merge driver is not involved in changeset-scoped commits and is orthogonal to this decision. It remains orthogonal as long as phase 4's partial-tree mechanism avoids merging.

### `rdm status` and `rdm discard` — **resolved by phase 5**

**`rdm status`** shows the caller's changeset by default, with `--all` for the whole tree. The two views come from ONE `StatusReport` partitioned in ONE pass into `user` / `derived` / `others` — deliberately not a changeset filter stacked on top of the derived filter, so the view a user reads and the set `rdm commit` will land cannot drift apart. `is_clean()` still means "nothing at all differs" (the correct gate for the whole-tree actions); `is_changeset_clean()` is the new scoped gate. `others` is named in the output rather than hidden.

**`rdm discard` — the design that shipped.** Default `rdm discard --force` is **changeset-scoped**:

1. restore only this changeset's journaled non-derived paths to HEAD (added ones removed, modified/deleted ones written back);
2. clear this changeset's journal;
3. regenerate the derived indexes **from the resulting disk state**, so another session's still-uncommitted rows survive — and journal that regeneration to this (now empty) changeset, so the session owns what it just rewrote;
4. re-ensure the `.gitattributes` merge-driver mapping, reported as `reinstalled:` exactly as before.

The whole-tree behavior is retained behind an explicit `--all`, which requires `--force` as well and **first names every other live changeset it is about to destroy** (from `journal::list_changesets`). The scoped path lives on `GitStore`, not in the CLI, so the MCP `rdm_discard` tool inherits identical behavior.

Rejected alternative: leaving discard whole-tree and warning. A destructive default that silently deletes a concurrent session's added files is the same class of defect as a sweeping commit, and a warning does not undo it.

## Phase 5 Implementation (Completed)

### The choke point is `create_git_commit`, not `commit_now`

Attribution lives in the tree builder itself. `GitRepo::create_git_commit` takes an explicit `CommitScope`:

- `CommitScope::WholeTree` preserves the historical behavior byte-for-byte (rebuild from disk, one attempt, no compare-and-swap), and is reachable from outside `rdm-store-git` only through the explicitly-named `GitStore::commit_whole_tree`.
- `CommitScope::Changeset(&ChangesetScope)` seeds a flat `path -> blob oid` map from HEAD and applies only the named changeset. A path no one named is never read, never blobbed, and never enters the tree — unreachable rather than filtered.

Both tree builders share one entry-sort comparator (`sort_tree_entries`), so identical content yields an identical tree oid either way and the `treeNotSorted` hazard cannot be re-derived in the new path.

`GitRepo::git_commit` was demoted to `pub(crate)`, so a new out-of-crate committer fails to compile. `scripts/verify-scoped-commit.sh` § D backs that with an allowlist grep (with both self-test arms) that also catches new in-crate callers and callers of the escape hatch, which the compiler cannot object to. Test-seeding callers (`rdm-store-git`'s `#[cfg(test)]` module, `rdm-server/tests/git_history.rs`) are a legitimate whole-tree class and are on the allowlist by name.

`rdm resolve`/pull paths that shell out to a real `git commit`/`git merge` still create whole-tree commits outside this mechanism. That is correct — they are *merge* operations, not session commits — and is stated here so the allowlist is understood rather than quietly widened.

### Journal truncation is correctness

A successful scoped commit removes the landed paths from the journal (`journal::truncate`). Without it, a session's *second* commit would re-include an already-landed path — and by then another session may have edited it, so the re-commit would sweep work this session never did.

### Concurrency: what phase 5 closes, and what it does not

The ref update is a compare-and-swap against the HEAD the tree was seeded from, with one rebuild-and-retry; a second mismatch is an explicit, actionable error, never a lost commit. A best-effort, age-bounded advisory lock under the git dir (5 s wait, 30 s staleness takeover — both far inside the default 30 s `hook_timeout_secs`) narrows the window in the common case; failing to take it proceeds rather than erroring, because the compare-and-swap is the actual correctness mechanism.

Two residuals are deliberately left to **phase 6**:

- A concurrent committer can land between a successful compare-and-swap and this process's post-commit index sync.
- Attribution is **path-level, not content-level**: if another session overwrites a path this changeset journaled, the scoped commit commits the other session's bytes for that path.

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
