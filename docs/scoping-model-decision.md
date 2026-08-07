# Session-Scoped Changeset Attribution: Decision Record

## Executive Summary

This decision record documents the chosen design for handling concurrent sessions sharing one plan repository (`$RDM_ROOT`). The core problem: two concurrent agents can race when both mutate and commit to the same git repository, with no isolation between their changesets. The solution is **Option A: session-scoped changeset identity** — each session journals the paths it touches, and `rdm commit` builds a tree from HEAD plus only the caller's journaled paths. Another session's dirty tree is irrelevant to what lands.

This is a decision-level document; implementation details (journal format, CLI surface granularity, stopping rule exact logic) are phase 4's responsibility.

## Problem Statement

### The Race Condition

When multiple concurrent sessions (e.g., parallel agent invocations, human and agent working together) share one plan repository:

1. Session A creates a task, writes to disk, then waits to call `rdm commit`
2. Session B creates a different task, writes to disk, then calls `rdm commit` first
3. Session B's commit lands both its task AND any uncommitted changes from Session A's workspace
4. Session A's subsequent `rdm commit` now lands work that was already committed by Session B

This violates the principle that each session controls only its own work. The changeset attribution model solves this by making each session's mutations traceable and independently committable.

### Requirements

Any solution must satisfy three binding properties:

- **Stable** across processes within one session — a session identity persists even when spawning subprocesses
- **Distinct** between concurrent sessions sharing one `$RDM_ROOT` — no cross-contamination
- **Automatic** when unset — operators should not be required to manually set session IDs for normal workflows

## Option A: Session-Scoped Changeset Identity

### Design Overview

**Session-scoped changeset identity** outlives a single process. Each session maintains a durable identity for the duration of its work, independent of any individual process's lifetime.

Mutations journal the file paths they touch. When a session calls `rdm commit`, it:

1. Stages only the paths journaled by that session since the last commit
2. Commits those paths to git
3. Leaves other sessions' dirty working-tree changes untouched

The key insight: another session's dirty tree is irrelevant to what lands. Session B can call `rdm commit` while Session A has uncommitted changes in the working tree — Session B's commit includes only the paths Session B touched.

### `rdm status` Visibility

The `rdm status` command can provide two views:

- **Caller's view**: show only the paths this session has journaled since the last commit
- **Tree view**: show all uncommitted changes in the working tree (useful for debugging cross-session state)

This flexibility allows operators to see both focused session progress and the full repository state when needed.

### Practical Example

1. Session A: `rdm task create fix-bug-1 ...` → journals `tasks/fix-bug-1.md`
2. Session B: `rdm task create fix-bug-2 ...` → journals `tasks/fix-bug-2.md`
3. Session B: `rdm commit -m "..."` → commits only `tasks/fix-bug-2.md` (Session B's journal)
4. Session A: `rdm commit -m "..."` → commits only `tasks/fix-bug-1.md` (Session A's journal)

Both commits land cleanly; there is no race.

## Option B (Rejected): Commit Per Mutation

### The Proposal

An alternative approach: every mutation (`task create`, `roadmap update`, etc.) immediately commits to git. No staging area, no deferred commit, no multi-step workflow.

### Strongest Argument for Option B

From the phase direction: "Its strongest argument is real and must be recorded in these terms: there is no staging area, so there is nothing to race on and nothing for an agent to misunderstand — the confusion goes away by deletion rather than by better documentation."

This argument is sound in isolation: by eliminating the staging area altogether, the conceptual model becomes simpler. An agent cannot accidentally conflate its staged changes with another session's work because staging does not exist. The race condition is deleted, not solved.

### Why Option A Wins

Option B loses on **history quality**, not implementation cost.

rdm's original behavior was exactly this: one commit per mutation. The model was simple, but the git log became unreadable. A roadmap creation followed by four phase creations would produce five separate commits, obscuring the logical unit-of-work (one roadmap + its phases, created together). Operators reviewing the history could not see the cohesive story of what was built.

Option A allows batching related mutations into one narrative commit:

```
feat(plan): implement auth-flow roadmap with 3 phases

Done: auth-flow/phase-1-login
Done: auth-flow/phase-2-session-mgmt
Done: auth-flow/phase-3-logout
```

This reads as a coherent change. Option B would have produced seven commits (one roadmap + three phases + three Done: directives on merge).

**The choice: readable history (Option A) over algorithmic simplicity (Option B).**

## Session Identity Resolution Chain

The session identity is determined by a four-rung resolution chain, evaluated in strict precedence order. Each rung must satisfy the three properties (stable, distinct, automatic). This chain is **binding on all implementations** — the exact spellings and precedence listed below are not examples; they are the contract.

### Rung 1: Explicit `RDM_SESSION` Environment Variable

**Highest precedence.** If `RDM_SESSION` is set, use its value as the session identity. This is the escape hatch for scripts, CI pipelines, and cases where explicit identity is required.

- **Stable**: yes — operator-supplied, does not change within the session
- **Distinct**: yes — operator can supply different values for different sessions
- **Automatic**: no — requires explicit setup, but opt-in

### Rung 2: Inherited Lease (Pid + Start Time)

If `RDM_SESSION` is not set, attempt to identify a long-lived parent process that represents the session. The key is **pid plus start time** (not pid alone).

- **Why both?** Pid alone is reusable; processes terminate and their pids are recycled. A child process needs a stable anchor. Start time (when the process was spawned) is immutable and makes each process incarnation unique. The combination of pid + start time forms a stable identifier for the session's root ancestor.
- **Ancestry traversal**: reaching the session ancestor requires traversal beyond direct parent — the immediate parent of an rdm invocation in an agent context is often an ephemeral shell, not the session root. The true session anchor is the grandparent or higher. This requires custom ancestry logic beyond standard library functions.
- **Lease semantics**: the inherited lease outlives the process that inherited it. If a session spawns process A, then A spawns process B, both B and its children inherit A's session identity until the session ends.

- **Stable**: yes — determined once at the session root, inherited by all descendants
- **Distinct**: yes — different session roots have different pids or start times
- **Automatic**: yes — inferred from process ancestry

### Rung 3: Documented Harness Session Variables

If no inherited lease is found (e.g., a top-level process with no long-lived parent), check for documented harness-specific session variables. These provide an adoption path for different execution environments (CI systems, IDE plugins, orchestration tools).

Example: `CLAUDE_CODE_SESSION_ID` — if set by Claude Code, use it as the session identity.

Other harnesses may define additional variables (e.g., `GITHUB_ACTIONS_RUN_ID`, `TERRAFORM_STATE_SESSION`, etc.). The set of recognized variables is extensible and must be documented.

- **Stable**: yes — provided by the harness for the duration of the session
- **Distinct**: yes — each harness invocation gets a unique id
- **Automatic**: yes — harness supplies it

### Rung 4: Fresh Per-Process Changeset (Fallback)

If all else fails, treat each process as its own session. Create a fresh session identity for this process alone.

- This rung **always resolves** — there is never a case where identity resolution fails entirely.
- A fresh per-process changeset means the mutations from this process will be journaled separately and can only be committed by this process (or a session that inherits its identity).
- If a process creates mutations and never calls `rdm commit`, those mutations are journaled but not landed. A subsequent process cannot land them (different session identity).

- **Stable**: only within a single process (identity ends when process exits)
- **Distinct**: yes — each process gets a distinct identity
- **Automatic**: yes — default fallback

### Binding Decisions

The four-rung chain above is **binding**. The exact spellings (`RDM_SESSION`, the reference to pid+start_time, `CLAUDE_CODE_SESSION_ID` as the first harness example) are not guidelines; they are the contract that implementations and downstream consumers can rely on.

The precedence order is also binding: explicit env var always wins, inherited lease is checked before harness variables, fallback is last.

## Key Decisions Binding on All Implementations

### Pid + Start Time is Required

**Measured:** 2026-08-04. Testing confirmed that `parent_id()` (retrieving the immediate parent pid) is insufficient. The session anchor is often the grandparent or higher in the ancestry chain, not the direct parent.

**Why ancestry traversal matters:** an agent invocation typically has this structure:

```
agent (long-lived) — pid: 1000, start: T0
  └─ shell invocation (ephemeral) — pid: 1001, start: T1
    └─ rdm process (ephemeral) — pid: 1002, start: T2
```

The rdm process should inherit the agent's session (pid 1000, start T0), not the shell's (pid 1001, start T1). Reaching the agent requires traversal beyond the immediate parent.

**Standard library limitation:** Rust's `std::process` and `std` do not expose process ancestry traversal or start times. Phase 4 must implement this with platform-specific code (likely via `libc` or equivalent).

**Binding decision:** The session identity mechanism MUST use pid + start time from an inherited long-lived ancestor, not direct parent alone. Phase 4 picks the exact stopping rule and ancestry traversal implementation.

### Stopping Rule: Asymmetric Failure Modes

The stopping rule determines where to halt ancestry traversal — when to decide "this is the session root" and go no higher.

**Two failure directions:**

1. **Stopping too high** (traversing beyond the session root): This merges concurrent sessions. If Session A's root is pid 1000 and Session B's root is pid 2000, but we traverse past both to find a common ancestor (e.g., the invoking shell or init), both sessions are treated as the same identity and their changesets are merged. **This reproduces the original race bug.** Severity: catastrophic.

2. **Stopping too low** (traversing too little): We decide the session root earlier than the true root, so some processes that should share a session get separate identities. A single batch of work might be split across two commits instead of one. **This only fragments one batch.** Severity: suboptimal, but not broken.

**The asymmetry is clear:** HIGH is dangerous (reproduces the bug), LOW is acceptable (single-batch fragmentation).

**Binding decision:** Prefer stopping low. The exact stopping rule (e.g., "stop when you reach a process that was spawned more than N seconds ago" or "stop at the first non-shell process") is phase 4's to decide, informed by this asymmetry.

### Git Hooks Under Agent Spawns

When `rdm hook post-merge` or `rdm hook post-commit` fires as a result of an agent's own `git merge` or `git commit`:

- **The hook inherits the agent's session identity** (via the inherited lease mechanism above or via environment variables).
- **The hook's mutations are journaled to the same session** as the agent.
- This is **correct behavior**: the hook is part of the same session's work; its changes should land in the same commit batch.

When a git hook fires outside any session context (e.g., a manual `git merge` from a terminal, or a commit from a different CI system):

- The hook gets a fresh per-process changeset identity (rung 4 fallback).
- The hook's mutations are journaled separately and can only be committed by that hook process itself.
- **Graceful degradation:** the hook must not assume the `RDM_GIT_SUBPROCESS` short-circuit already fired (see CLAUDE.md for the short-circuit semantics). Identity resolution must re-run and pick rung 4 independently.

**Binding decision:** Hooks must support both paths (in-session via inherited identity, and out-of-session via rung 4 fallback) and degrade gracefully. The `hook_timeout_secs`-bounded execution path must never assume a prior identity resolution succeeded.

## Open Questions (Phase 4 & Phase 5 Responsibilities)

### Journal On-Disk Layout and Serialization

Phase 4 decides:

- **Storage format**: how are journaled paths written to disk? (e.g., `.rdm-session-<id>.journal` files, a single `$RDM_ROOT/.rdm/journal.json`, or embedded in a `.git` object)
- **Granularity**: do we journal individual file paths, or path prefixes, or something coarser?
- **Serialization**: plain text, JSON, binary, or other?

These decisions are phase 4's; the scoping model itself does not depend on them.

### Stopping Rule Exact Threshold

Phase 4 decides the specific stopping condition for ancestry traversal — the rule that determines "when have we reached the session root?"

Possible approaches:

- Timing-based: stop when you reach a process older than N seconds
- Structure-based: stop at the first non-ephemeral process (not a shell, not an rdm subprocess)
- Combination: reach a process that is both old AND non-shell-like

Phase 4 picks the rule, informed by the asymmetry analysis (prefer low).

### CLI Surface for Session Debugging

Phase 4 decides how operators can inspect and debug session identity:

- Does `rdm status --session-only` show the caller's journaled changes?
- Does `rdm config show session` display the current session identity?
- Are there any `--session <id>` override flags?

These are usability and observability decisions; the scoping model is independent of the CLI surface.

### Which Interaction Layers Are Covered?

The scoping model is designed to work with the **CLI** and **MCP** interfaces (both read and mutate the plan repo and call `rdm commit`).

**rdm-server** is an **open question**:

- rdm-server provides a REST API over the plan repository. It mutates the same store as the CLI and MCP interfaces.
- However, rdm-server typically runs as a daemon and **never calls `rdm commit`** — commits are deferred to the orchestrating layer.
- Should rdm-server's mutations be journaled and attributed to a session? Or should rdm-server operate outside the session model, leaving journaling to the application layer?

**This is a phase 5 decision**, informed by rdm-server's deployment model and the operator's choice of orchestration.

Current assumption: the scoping model fully covers CLI and MCP. rdm-server's disposition is deferred to phase 5.

## INDEX.md Consistency Under Partial Commits

### Auto-Generated INDEX.md is Not a Merge Conflict Source

`INDEX.md` is auto-generated from the individual roadmap/phase/task/review markdown files. It is never edited by hand; it is a derived artifact. The source of truth is the individual files.

When `rdm commit` builds a partial tree (only the caller's journaled paths):

1. The individual roadmap/phase/task files that were mutated by this session are included in the commit.
2. After staging the caller's paths, **rdm automatically regenerates INDEX.md from those files** (via `generate_index_for_project` in `rdm-core/src/ops/index.rs`).
3. `INDEX.md` is then included in the same commit as the individual files.

### Mechanical Consistency Guarantee

The partial commit's INDEX.md is **automatically consistent** with the individual files that were just committed, because it is regenerated from them. There is no manual merge conflict risk and no possibility of drift.

Consider the sequence:

1. Session A commits phase file `roadmap-a/phase-1-foo.md`
2. Session B commits phase file `roadmap-b/phase-2-bar.md`
3. Session A's commit regenerates INDEX.md from its committed files (includes roadmap-a entries)
4. Session B's commit regenerates INDEX.md from its committed files (includes roadmap-b entries)

If Session A's commit lands first, then Session B's commit lands second, Session B's regeneration of INDEX.md will include entries from both roadmap-a (unchanged in HEAD since Session A committed) and roadmap-b (newly committed by Session B). Consistency is maintained because INDEX.md is regenerated from the live files, not from a prior snapshot.

### Key Insight

Partial commits do not require special conflict-resolution logic for INDEX.md, because INDEX.md is not a source of truth — it is a cacheable output. Regenerate it from the source files at commit time, and it is always correct.

## The rdm-index Merge Driver is Out of Scope

### What is the rdm-index Merge Driver?

The rdm-index merge driver is custom git merge logic (defined in `rdm-store-git/src/repo.rs` and configured via `.gitattributes`) that fires during `git merge` to automatically regenerate `INDEX.md` when there are conflicts. It ensures that `INDEX.md` is always consistent after a merge.

### Why It Doesn't Apply to Changesets

**Changesets never perform git merges.** The session-scoped changeset model commits from HEAD plus the caller's journaled paths. There is no merge operation — the tree is built directly from HEAD by including only the changed files.

Therefore, the merge driver is not involved in changeset-scoped commits. It is orthogonal.

### Separate Concern

The merge driver is relevant to other scenarios:

- Manual `git merge` of branches
- Multi-party development where different branches are merged into main

But it is not part of the session-scoped changeset model. It is a separate mechanism for a separate problem (merge conflict resolution during integration).

**Binding decision:** The rdm-index merge driver is explicitly out of scope for changesets. Implementations must not invoke merge logic during a changeset-scoped `rdm commit`.

## Appendices

### Measurement Data

- **pid+start-time sufficiency**: confirmed 2026-08-04 via testing that `parent_id()` alone is insufficient for identifying session ancestors in agent contexts. Custom ancestry traversal is required.

### Related Concepts (Not Decided Here)

- **`RDM_GIT_SUBPROCESS` short-circuit** (mentioned in CLAUDE.md): a separate mechanism that prevents `rdm hook post-merge`/`post-commit` from re-triggering themselves when fired by rdm's own git operations. This is distinct from session identity resolution but related in hook execution flow.

- **Phase Commit Hook (`Done:` convention)**: unrelated to session identity. The `Done: <roadmap>/<phase>` directive in commit messages is a separate mechanism for marking phase completion. It operates independently of session boundaries.

### Implementation Notes for Phase 4

When implementing this decision:

1. **Session identity is separate from changeset journaling.** The identity is determined first (via the four-rung chain), then used as a key for the journal.

2. **Journal persistence.** The session identity and journaled paths must persist across process boundaries (e.g., if Process A is a shell that spawns Process B, both must see the same journal). File system storage (e.g., `.git` or a `.rdm` directory) is likely necessary.

3. **Transaction semantics.** A single `rdm commit` is a transaction: stage the caller's paths, regenerate INDEX.md, and commit all together. If any step fails, roll back the entire operation.

4. **Backward compatibility.** Non-session contexts (e.g., a user running rdm manually) should still work. Rung 4 (per-process changeset) ensures this.

### References

- **rdm-core/src/ops/index.rs**: houses `generate_index_for_project` and the INDEX.md regeneration logic.
- **rdm-store-git/src/repo.rs**: defines the rdm-index merge driver configuration.
- **CLAUDE.md (project instructions)**: Session identity resolution and git hook behavior are discussed in the "Hard rule — no direct access to the plan repo" section and surrounding content.

---

**Decision:** Session-scoped changeset identity (Option A) is the chosen design.

**Status:** Documentation only. No implementation code or behavior changes in this phase. Phase 4 carries the implementation; Phase 5 decides rdm-server's scope.
