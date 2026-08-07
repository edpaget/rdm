# Session Identity and the Changeset Journal (Implementation Record)

This is the **implementation** record for the identity half of session-scoped
changesets. The **decision** record — the four-rung chain, the three properties
(stable / distinct / automatic), the asymmetric-failure principle, and the
evidence behind them — is
[`docs/scoping-model-decision.md`](scoping-model-decision.md) and remains
binding. This document covers only what that record explicitly delegated: the
stopping rule that ships, the on-disk layout, the serialization format, the
lifecycle, the CLI surface, and the measured cost.

**Boundary note: no commit behavior changed here.** Nothing routes a committer
through the journal. `GitRepo::git_commit`, `commit_now`, `git_status`,
`default_commit_message`, and `apply_done_directives` are untouched, and
`rdm-store-git/src/commit.rs` was not edited at all — a structural grep in
`scripts/verify-session-identity.sh` § I gates that it stays that way. Routing
the five committers, reconciling the journaled derived index against HEAD, and
scoping `rdm status` / `rdm discard` are the *scoped commit choke point*
phase's work.

## The shipped stopping rule

The decision record fixed the asymmetry and left the rule to this phase:

> Stopping too **high** merges concurrent sessions into one identity — this
> reintroduces the race. Stopping too **low** fragments one session's work
> across changesets — suboptimal but safe. **Prefer stopping low.**

The rule that ships is two halves, deliberately asymmetric:

| Half | Rule |
| --- | --- |
| **Adoption** | Ascend up to `MAX_ANCESTOR_DEPTH` (8) ancestors looking for an *existing* valid lease. Take the first (lowest) match. |
| **Creation** | Create a lease **only at depth 1**, the immediate parent. Never higher. |

Because creation never ascends, two concurrent sessions can share an identity
only if some ancestor they have in common was itself the immediate parent of an
earlier `rdm` invocation. When the parent is the ephemeral per-invocation shell
an agent spawns — the case the decision record measured — no ancestor is ever
leased, each invocation gets its own changeset, and the session fragments. That
is the safe direction, and rung 3 is what covers this case in practice.

The ascent terminates on all three degenerate shapes: it is bounded at 8, it
stops at pid ≤ 1, and it carries a seen-set so a cycle cannot loop.

### Why rung 2 both precedes rung 3 and defers to it

The decision record's precedence is explicit: inherited lease outranks harness
variable. Rung 2 is therefore *checked* first. But lease **creation** happens
after rung 3 has been consulted, because a harness that already publishes a
stable id has nothing for a lease to bootstrap — and creating one anyway would
make rung 3 unreachable in practice. So the order is:

1. `RDM_SESSION` → rung 1.
2. Adopt an existing ancestor lease → rung 2.
3. First non-empty `HARNESS_SESSION_VARS` entry → rung 3.
4. Create a lease at the immediate parent → still rung 2.
5. Per-process id → rung 4.

This is an amendment to nothing: the decision record fixed the precedence of
*resolved identities*, and the resolved precedence above is exactly that order.

## On-disk layout

```
<git-dir>/rdm/
├── leases/<pid>.lease            one JSON object
└── changesets/<id>.jsonl         one JSON object per line, append-only
```

### Why `<git-dir>/rdm/`

This placement is forced, not preferred. `rdm-store-git` builds git trees by
walking the working directory (`build_tree_from_dir`, `commit.rs`) and computes
status the same way (`collect_working_tree`, `commit.rs`). Both skip exactly one
name — `.git` — at every recursion level, and **neither consults `.gitignore`**,
so a gitignore entry would exclude nothing from rdm's commit path. Siting the
state inside the git directory is therefore the only placement that is
invisible to a whole-tree commit and to `rdm status` without changing either
walk.

**Any future refactor of those two walks must preserve the `.git` skip**, or
session state starts landing in commits. `scripts/verify-session-identity.sh`
§ G gates this end to end, including a planted-decoy self-test proving the
`git ls-tree` assertion is not vacuous.

A linked worktree is fine: `git_dir()` then points at
`<main>/.git/worktrees/<name>`, possibly outside `$RDM_ROOT`, which the walks
never see — and two worktrees of one plan repo correctly get separate journals,
because they are separate working trees.

When there is no git directory (a plain `FsStore` build), the state falls back
to `$XDG_STATE_HOME/rdm/sessions/<sha256(root)[..16]>/`, with
`$HOME/.local/state` standing in for an unset `XDG_STATE_HOME`. If neither is
set there is no state directory at all, rung 2 is simply unavailable, and
resolution lands on rung 3 or 4 — never an error. **Session state is never
written under `$RDM_ROOT` outside `.git`.**

Directories are created lazily on first write, never at store-open time, so a
read-only plan repo still opens.

### Lease format

```json
{"id":"s-1f2e3d4c5b6a7988","start_time":"Mon Aug  4 12:34:56 2026","created_utc":"2026-08-07T12:00:00+00:00"}
```

The file is keyed by pid alone and stores the start time *inside*, so a pid
recycle is **observable** rather than silently invisible: adoption compares the
recorded `start_time` byte-for-byte against the live process table's value, and
a mismatch means the lease belongs to a dead process. Byte comparison (never
arithmetic) is what makes clock skew and a non-monotonic wall clock unable to
cause a false adoption.

Creation is atomic and exclusive: the content is written to a temp file and
hard-linked into place, so a concurrent reader can never observe a half-written
lease. When the link loses the race, the winner's lease is read back and
adopted — which is what makes two racing creators *converge* on one id instead
of splitting the session.

### Journal format

One line per `Store::commit` batch:

```json
{"paths":[{"path":"projects/demo/tasks/a.md","kind":"write"},{"path":"INDEX.md","kind":"write"}]}
```

`kind` is `write` or `delete`. Recording a delete as a delete is load-bearing:
a scoped tree build cannot reproduce a removal it was told was a write.

Two properties fall out of the layout rather than out of discipline:

- **Disjointness** — concurrent sessions have distinct ids and therefore
  distinct files, so one session physically cannot write into another's journal.
- **Lock-free appends** — each batch is one `write_all` of one complete line to
  a file opened `O_APPEND`, which POSIX does not interleave. No lock, no
  read-modify-write race.

Reads dedupe and sort the union across lines (last recorded kind wins per
path); appends stay O(1). **There is no compaction in this phase**, deliberately
— a long autopilot session's journal grows without bound in line count, which
is a size concern, not a correctness one.

### Exactness

The batch's membership is snapshotted from `FsStore::staged_paths()` *before*
`commit` drains the staging overlay, and recorded *after* the flush succeeds.
So the journal can be neither a superset of what landed nor a claim about a
batch that failed, and an empty flush records nothing at all rather than an
empty line.

The derived `INDEX.md` files land in **every** session's journal, because
`ops::mutate` regenerates them on every mutation. That is correct here — every
session really did write them — and it is asserted deliberately in
`scripts/verify-session-identity.sh` § F. Reconciling the shared derived index
at commit time belongs to the scoped-commit phase.

## Lifecycle

| Object | Created | Removed |
| --- | --- | --- |
| Lease | On the first bare invocation under a parent | By GC when its pid is dead or recycled; opportunistically during the ancestry walk |
| Journal | On the first flushed batch of a changeset | **Never automatically** — only by `rdm session discard --force` |

GC (`rdm session gc`, and opportunistically during adoption) removes leases
whose owning process is gone or whose recorded start time no longer matches,
bounded at 64 files per pass. It never touches a journal, so no work is
silently destroyed.

GC is **skipped entirely** when the process table cannot see the calling
process. A table that reads as empty means "unknown", never "nothing is alive";
treating it as the latter would sweep away every live session's lease.

**Orphan recovery.** A session killed mid-batch leaves a journal with no live
lease. `rdm session list` flags it `orphaned: true`; `rdm session adopt <id>`
re-points the caller's immediate-parent lease at it, so the caller's shell
resolves that changeset from then on; `rdm session discard <id> --force` drops
it.

## Environment variables

| Variable | Rung | Notes |
| --- | --- | --- |
| `RDM_SESSION` | 1 | The explicit escape hatch. Wins outright — no lease read, no ancestry walk, no harness read. |
| `CLAUDE_CODE_SESSION_ID` | 3 | The reference harness entry named by the decision record. |
| `CLAUDE_SESSION_ID` | 3 | |
| `RDM_HARNESS_SESSION_ID` | 3 | The generic hook for a harness with no native variable. |

`HARNESS_SESSION_VARS` is the **extension point**: it is an ordered, documented
list, and wiring in a new harness means adding an entry, nothing more. Rung 3
ids are purely derived (`h-<sha256(var-name ‖ NUL ‖ value)[..8]>`), so two
unrelated processes agree with zero on-disk state and no lease is created.

Ids are sanitized because an id becomes a **file name**: the value is trimmed,
restricted to `[A-Za-z0-9._-]`, capped at 64 bytes, and rejected when nothing
usable survives or the result is only dots. A blank or all-punctuation
`RDM_SESSION` falls through to the next rung rather than erroring.

Nothing special-cases `RDM_GIT_SUBPROCESS`. The decision record requires
resolution not to assume that short-circuit fired, so a git hook resolves
through the same chain whether or not rdm spawned it.

## Degradation

`resolve_session` has **no `Result` in its signature**, which makes the
never-error guarantee structural rather than disciplinary. Every fallible
sub-step falls through:

| Failure | Outcome |
| --- | --- |
| Empty / unreadable process table | Rung 4 |
| Corrupt or unreadable lease | Lease removed; ascent continues |
| Recycled pid (start-time mismatch) | Lease removed; ascent continues |
| Unwritable state directory | Rung 4 |
| Blank `RDM_SESSION` | Falls through to the next rung |
| Neither `HOME` nor `XDG_STATE_HOME` (non-git build) | No state dir; rung 3 or 4 |
| Missing `/proc`, absent or sandboxed `ps`, unparsable output | Empty table → rung 4 |

Journal recording is best-effort at the call site (`let _ = …`, mirroring
`HookLogger`'s swallow-failures contract): an unwritable state directory must
never fail a mutation.

## Process backends

| Target | Backend |
| --- | --- |
| Linux | `/proc/<pid>/stat` for every numeric entry in `/proc`. Fields are parsed **after the last `)`**, so a process name containing spaces or parentheses cannot shift `ppid` (field 4) or `starttime` (field 22). |
| Other unix | One `ps -Ao pid=,ppid=,lstart=` spawn. `lstart` is an absolute start time and therefore stable, unlike the relative `etime`; its embedded spaces are handled by splitting only the first two fields. |
| Anything else | An empty table. |

No `unsafe` anywhere, and no new crate dependency — `sha2`, `serde_json`, and
`chrono` were already `rdm-core` dependencies.

## Cost

`rdm session id --format json` reports `resolve_micros`, the wall time of the
memoized first resolution.

| Platform | Backend | Max over 20 fresh invocations |
| --- | --- | --- |
| macOS (darwin 25.5, arm64) | one `ps -Ao pid=,ppid=,lstart=` spawn | **~16–27 ms** (16 099 µs on the recorded run) |

The single `ps` spawn is the dominant term by an order of magnitude and is
memoized once per process, so an `rdm` invocation pays it at most once no
matter how many stores or commits it opens.

`scripts/verify-session-identity.sh` § H prints the observed maximum and
asserts it under 250 000 µs — a bound, not an exact figure. That is three
orders of magnitude under the 30 s default `hook_timeout_secs`, so identity
resolution cannot put the unattended hook path near its deadline. § H2 runs a
real `Done:`-bearing `rdm hook post-commit` end to end and asserts it exits 0,
applies its directive, and journals its own writes.

## CLI surface

| Command | Purpose |
| --- | --- |
| `rdm session id` | Print this session's changeset id. `--format json` adds `rung` and `resolve_micros`. Text mode prints the bare id for `$(...)` capture. |
| `rdm session journal [--id <id>]` | Print a changeset's exact journaled path set. |
| `rdm session list` | List every changeset, flagging orphans. |
| `rdm session adopt <id>` | Re-point this session at an existing (usually orphaned) changeset. |
| `rdm session discard <id> --force` | Delete a changeset's journal. Irreversible, hence the flag. |
| `rdm session gc` | Remove leases whose owning process is gone or recycled. |

The JSON field names (`id`, `rung`, `resolve_micros`, `orphaned`, `paths`) are
a stable target for the agent-surface phase; do not rename them casually.

## Gating

- `cargo nextest run` — unit tests in `rdm-core/src/session/**` (the rung chain
  over an injected `ProcessTable`/`EnvSource`, including the pid-recycle case,
  which is otherwise unconstructible), `rdm-store-git` tests for journal
  exactness and commit/status invisibility, and `rdm-cli/tests/cli_session.rs`
  end to end against the real binary.
- `bash scripts/verify-session-identity.sh` — the multi-process harness:
  sections A–I as described above.
