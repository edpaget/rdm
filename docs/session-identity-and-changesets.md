# Session Identity and Changesets: Resolving Plan Repo Concurrency

## Problem Statement

When multiple concurrent sessions share a single `$RDM_ROOT` (plan repository), rdm's current behavior fails: whichever session runs `rdm commit` first wins, committing its changes plus any other session's staged edits still on disk. The losing session's work is silently bundled into the first session's commit without its knowledge or consent. This is a correctness bug, not a UX issue — two independent agents racing on a plan repo will corrupt each other's edits and diverge from intent.

## Solution: Option A — Changeset Attribution

Adopt **changeset attribution**: each session owns a scoped changeset identity that outlives individual processes. Mutations journal the paths they touch (per session). `rdm commit` builds a tree from HEAD plus only the caller's journaled paths. Another session's dirty tree is irrelevant to what lands. `rdm status` can show either the caller's changeset or the whole tree.

This preserves the high-quality git history rdm has achieved: one commit per `roadmap create`, one commit per phase update batch, not 20 commits per `task create` nor a waterfall of `--squash` cleanup. It replaces the current in-process staging area with a session-scoped journal — the same API surface, but session-aware instead of process-local.

### Why Option A

The staging area works for single-session use (the normal case). For concurrent sessions, a staging area is a **shared mutable namespace** between independent actors — the invariant breaks because there is no owner. Changesets solve this by giving each session a journal. Multiple journals coexist; each session commits only its own paths. A session starting fresh (no `RDM_SESSION` set, no inherited lease) gets a per-process changeset that behaves exactly like the old model (writes stage, reads see staged, commit clears it), so single-session users see no difference.

### Why Not Option B: Commit Per Mutation

Option B proposes deleting the staging area entirely. Every mutation auto-commits, so there is no race because there is nothing to race on. This is the **strongest argument for Option B, and it must be recorded verbatim**:

> There is no staging area, so there is nothing to race on and nothing for an agent to misunderstand — the confusion goes away by deletion rather than by better documentation. It loses on history quality, not implementation cost.

This is true and candid. The tradeoff is real. Option B was rdm's behavior in an earlier version and was abandoned because one commit per `task create` makes the log unreadable as a narrative — `git log --oneline | head -50` is useless if it's 50 individual micro-commits instead of a cohesive story. Option A preserves the narrative by letting human judgment decide what goes in one commit — it loses simplicity to win history quality. That is an acceptable tradeoff for a tool that users review and manage long-term.

---

## Session Identity: Four-Rung Resolution Chain

A session's identity must be three properties: **stable** across processes in one session, **distinct** between concurrent sessions sharing one `$RDM_ROOT`, **automatic** when unset. The following chain satisfies all three, in precedence order (first match wins, each spelling is binding):

1. **Explicit `RDM_SESSION` environment variable** (always wins; the escape hatch for scripts and CI)
2. **Inherited lease** keyed on **pid + start time** of the nearest long-lived ancestor process (not the immediate parent, which is often ephemeral)
3. **Harness session variables** (documented, extensible list; adoption path for new integration layers — examples: `CLAUDE_CODE_SESSION_ID`, analogous variables from other orchestration systems)
4. **Fresh per-process changeset** (fallback; no session continuity, only per-process staging like the old model)

### Why This Chain

- **Rung 1 (explicit)** enables scripts and CI pipelines to opt into session grouping by setting `RDM_SESSION=my-automation-run-123`. Without this, each subprocess landing a commit would be its own session.
- **Rung 2 (pid + start time)** is the stable identity for long-running parents (e.g., a Claude Code agent, a CI job runner). Parent PID alone is reusable (PIDs wrap on all systems); start time makes it distinct. Reaching the true stable ancestor requires ancestry traversal beyond `std::env::parent_id()` (which returns one level), yielding both pid and start time as a side effect. Rung 2 prefers stopping **low** (early termination of the walk) over stopping **high**, because:
  - Stopping **too high** (e.g., at the CI runner's PID) would merge concurrent CI jobs into one session — property 2 (distinct) fails.
  - Stopping **too low** (e.g., at the shell spawning a command) only fragments one batch into multiple sessions — property 2 still holds, but the session spans fewer mutations than intended. Fragmentation is recoverable by hand; merging is a data loss bug.
- **Rung 3 (harness variables)** provides an adoption path for integration layers that do not expose parent process info reliably (or at all). Each harness documents its variable name once, and that name is consulted the same way `RDM_PROJECT` checks `RDM_PROJECT` env var, then config.
- **Rung 4 (fallback)** always resolves, so no layer is ever forced to error. It also ensures phase 4 can reject a whole-tree fallback without needing one — if identity resolution fails, rung 4 yields a per-process changeset that doesn't race (it can only collide with itself).

### Note on `RDM_PROJECT` Precedence

The `RDM_PROJECT` flag > env > config chain is **not** a model for session identity. That chain terminates in repo-global `default_project`, so two concurrent sessions resolving to the same default both get the same `--project` — failing property 2. Session identity instead uses a global namespace (explicit override or inferred from process ancestry) so that every session gets a distinct identity, unless the user explicitly opts into grouping via rung 1.

---

## Session Identity: Mechanism

Phase 4 will implement the resolution chain above. The key mechanism is: **pid + start time** uniquely identifies a process family.

- The pid is the OS process ID.
- The start time is the process birth timestamp from the OS (available via `/proc/<pid>/stat` on Linux, `p_birth` on macOS via getrusage-like interfaces, or WMI on Windows).
- Together, pid + start time survive PID wrap and distinguish unrelated process families.

Reaching the right ancestor (not the immediate parent, which is often an ephemeral shell) requires walking the process tree upward until stopping criteria are met. Phase 4 records which rule ships (e.g., "stop at the first process whose start time is >N seconds old" or "stop at the first CLI or server process, not an ephemeral shell").

---

## Git Hooks and Session Identity

When `rdm hook post-merge` or `rdm hook post-commit` is invoked by git:

- The hook is spawned as a subprocess of the git merge/commit that is itself running under a session's control.
- The hook **inherits the session identity** from that session (via rung 2 ancestor walk or rung 1 explicit override).
- This is correct behavior: the hook's mutations belong to the same session's changeset.

When a hook is invoked outside any rdm session (e.g., a bare `git merge` in the plan repo, not from any agent context):

- The hook's identity resolution falls through to rung 4 (no inherited lease, no harness variable).
- It commits only what it writes (the `Done:` directive stamping and any index regeneration).
- This is also correct: work outside any session commits only on its own terms.

The existing `RDM_GIT_SUBPROCESS` short-circuit (which avoids re-running the `Done:` directive pipeline if rdm itself invoked the git merge) must **degrade to rung 4 rather than error** on timeout or subprocess detection failure. If the short-circuit misfires or the timeout clock runs out, the hook MUST still run (yielding a per-process changeset at worst) rather than failing the merge.

---

## Interaction Layers

This design applies to:

- **CLI** (confirmed in scope for this decision record)
- **MCP server** (confirmed in scope; mutations are journaled per session)
- **`rdm-server` REST API** (open for phase 5; rdm-server never commits, only writes, so its journaled mutations would never land — phase 5 decides whether to journal at all, or leave rdm-server stateless)

---

## INDEX.md Consistency

The merge driver setup (`.gitattributes` routing `INDEX.md` to `rdm-index`, `.git/config` configuring the `rdm-index` driver command) is irrelevant to this design.

**Why:** A git merge driver only fires during git merge operations, when two branch histories diverge and git attempts to auto-resolve conflicts in a specific file. Changesets perform **direct commits** from HEAD without merging: a changeset commit is `rdm commit <message>` building a new tree from HEAD + journaled paths and writing a commit, not `git merge --ff-only`. Since changesets bypass merges entirely, the merge driver **never fires**.

**How INDEX.md stays consistent:** `rdm commit` regenerates `INDEX.md` from the actual committed files (scanning roadmaps, tasks, phases, reviews in the staging tree before commit) and includes that regenerated content in the commit. This happens pre-commit, so INDEX.md is always consistent with the files rdm knows about. The merge driver becomes obsolete and can be removed (future work), but its presence in `.gitattributes` and `.git/config` is harmless — it simply never runs.

---

## Phase 4 Deferred

This decision record specifies **WHAT** the model is (session identity chain, pid+start-time key, changeset attribution). Phase 4 specifies **HOW** to implement it. The following are out of scope for this phase:

1. **Journal on-disk layout** — where and how session state is stored (e.g., `.rdm/sessions/`, `~/.cache/rdm/`, git config, JSON files)
2. **Serialization format** — encoding of session identity, journaled paths, change log (binary, JSON, plain text)
3. **Granularity** — whether to journal individual mutations or batch them by rdm command (e.g., one journal entry per `task create`, or coalesce a `phase update --status in-progress && phase update --body ...` into one entry)
4. **CLI surface** — flag names and semantics for explicit session overrides, querying sessions, inspecting journals, etc.

Phase 4 carries the implementation; this record provides the specification.

---

## Acceptance Criteria

- [x] Decision record documents Option A (changeset attribution model)
- [x] Records rejection of Option B with the strongest argument preserved verbatim
- [x] Documents the four-rung session identity resolution chain with exact spellings: `RDM_SESSION`, inherited lease, harness variables, per-process fallback
- [x] Explains pid + start time as the stable, distinct, automatic mechanism
- [x] Documents the stopping-rule asymmetry and preference for stopping low
- [x] Explains git hook implications (inherited lease, degradation to rung 4, fail-open on timeout)
- [x] Documents which layers are in scope (CLI, MCP) and defers rdm-server to phase 5
- [x] Explains INDEX.md consistency via pre-commit regeneration (not merge driver)
- [x] Rules out the merge driver by name and mechanism
- [x] Marks journal layout, serialization, granularity, and CLI surface as phase 4's responsibility
