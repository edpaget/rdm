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

**rdm-server**: Open question for phase 5. The REST API server mutates the same plan repository but does not expose a commit endpoint (it is stateless and never commits). Mutations are written to disk immediately. The question of whether `rdm-server` should journal mutations and provide session scoping (and what "commit" means for a stateless server) is an operational/deployment decision that phase 5 will settle. For now, assume `rdm-server` bypasses the session model entirely.

## Open Questions (Deferred to Phase 4 or Later)

### Journal Implementation Details

Phase 4 must decide:
- **On-disk layout**: Where and how are journals stored? Per-session file, in-memory structure, or other? (Constrained by the requirement to place the journal outside `$RDM_ROOT` committable tree, e.g., `$RDM_ROOT/.git/rdm/` or an XDG state dir.)
- **Serialization format**: JSON, JSONL, binary, or?
- **Granularity**: What is the atomic unit of journaling? Per-mutation, per-batch, or per-operation type?
- **Cleanup and lifecycle**: When and how are old journals pruned? On successful commit, on session exit, or on a schedule? How are orphaned changesets (from sessions that were killed) discovered and recovered?

### INDEX.md Consistency in Partial Commits (Open Question for Phase 4/5)

**Current behavior:** INDEX.md is auto-generated from individual roadmap, phase, task, and review files in the store — it is not a source of truth, but a computed artifact. Today, every mutation invokes `ops::mutate` (rdm-core/src/ops/mod.rs:70–78), which calls `index::generate_index_for_project()` (rdm-core/src/ops/index.rs:113–133). This function scans **the entire on-disk store** via `list_projects` → `build_project_index` → `list_roadmaps`/`list_phases`/`list_tasks`, with no filtering for session-scoped paths. The generated INDEX.md is then staged to disk.

When `git commit` is later called (whether via `rdm commit` or a git hook), the current implementation (rdm-store-git/src/commit.rs:166–199) recursively builds a tree from the entire working directory via `build_tree_from_dir`, staging whatever files are currently on disk. This means if Session A has written a new task file to disk but has not yet committed, and Session B then calls `rdm commit`, the following problem arises:

1. Session B's partial tree correctly excludes Session A's task file (journaled paths are unknown in today's disk-only staging).
2. However, Session B's INDEX.md (regenerated by `ops::mutate` during B's own mutations) was built by scanning the live filesystem, which includes A's task file.
3. The resulting committed tree contains Session B's INDEX.md with a reference to Session A's task entity, but the task file itself is absent from that tree — a dangling reference and cross-session contamination.

**Why this is still an open question:** This is precisely the mechanism that phase 4's constraints identify: "The derived indexes land in every session's journal because `ops::mutate` regenerates them on every mutation. Journaling them is correct; reconciling them at commit time is phase 5's problem." Solving this requires phase 4 to introduce a journal-scoped Store view or similar mechanism so that `generate_index_for_project` (and its `build_project_index` subroutines) read only from HEAD + the caller's journaled paths, not from the live filesystem. This is not a minor tweak — it fundamentally changes how the index-generation functions see the repository state.

**Phase 4's responsibility:** Scope the index-generation functions to respect the changeset boundary:
- Modify `build_project_index` or introduce a wrapper that provides a constrained Store view (e.g., a virtual overlay or journal-backed store) to the `list_roadmaps`/`list_phases`/`list_tasks` calls.
- Ensure that when a session commits, the INDEX.md it includes in the tree reflects only entities that actually exist in that tree.

### Merge Driver: Out of Scope (Correctly)

The rdm-index merge driver (`rdm-store-git/src/repo.rs`) automatically regenerates `INDEX.md` when git detects conflicts during a merge. However, changesets never perform a merge — a changeset-scoped commit constructs its tree from HEAD plus the caller's journaled paths (a direct write operation, no three-way merge). Therefore, the rdm-index merge driver is not involved in changeset-scoped commits and is orthogonal to this decision. It remains orthogonal as long as phase 4's partial-tree mechanism avoids merging.

### CLI Surface for Session Display and Management

Phase 4 must decide how users and agents interact with the session model:
- Does `rdm status` show only the caller's changeset, the whole tree, or both with flags?
- Is there an `rdm session info` or similar command to inspect the current session identity?
- Can users or scripts inspect the journal (for debugging) with a command like `rdm status --debug-journal`?

These are user-facing details that can be refined iteratively; the core model does not depend on them.

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
