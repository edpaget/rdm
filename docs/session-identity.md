# Session Identity and the Changeset Journal (Implementation Record)

This is the **implementation** record for the identity half of session-scoped
changesets. The **decision** record — the four-rung chain, the three properties
(stable / distinct / automatic), the asymmetric-failure principle, and the
evidence behind them — is
[`docs/scoping-model-decision.md`](scoping-model-decision.md) and remains
binding. This document covers only what that record explicitly delegated: the
stopping rule that ships, the on-disk layout, the serialization format, the
lifecycle, the CLI surface, and the measured cost.

**Boundary note: commit behavior is now built on this.** Through phase 4 it was
not — nothing routed a committer through the journal, and
`scripts/verify-session-identity.sh` § I gated that `rdm-store-git/src/commit.rs`
stayed uncoupled. The *scoped commit choke point* phase is that later work, and
it inverted the boundary: `GitRepo::create_git_commit` now takes an explicit
commit scope, `commit_now` is gone in favor of `GitStore::commit_changeset` /
`commit_whole_tree`, `git_status` is now the three-bucket `git_status_report`,
and all five committers — `rdm commit`, `apply_done_directives` (the `Done:`
hooks), the MCP `rdm_commit` tool, `bootstrap`, and `init` — build their tree
from a changeset. § I is inverted to match and now asserts the coupling *is*
present while `commit.rs` still resolves no session identity of its own. What
each surface does with a changeset is described under
[*What a changeset does at commit / status / discard time*](#what-a-changeset-does-at-commit--status--discard-time)
below; the commit-side design record is
[`docs/scoping-model-decision.md`](scoping-model-decision.md).

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
{"paths":[{"path":"projects/demo/tasks/a.md","kind":"write","digest":"<sha256-hex>"},{"path":"INDEX.md","kind":"write","digest":"<sha256-hex>"}]}
```

`kind` is `write` or `delete`. Recording a delete as a delete is load-bearing:
a scoped tree build cannot reproduce a removal it was told was a write.

`digest` is the sha256 of the bytes that batch flushed to the path — base-blob
identity, added in phase 6 so a scoped commit can tell "the content this
changeset wrote" from "whatever is at that path now" and refuse to land another
session's bytes under this changeset's message (see
[`lost-update-evaluation.md`](lost-update-evaluation.md)). It is **optional**,
in both directions:

- absent on a `delete`, which has no bytes to identify;
- absent on lines written before the field existed, and on paths written
  outside the store entirely (`.gitattributes`), which have no staged content.

A line without it stays parsable — the field is `#[serde(default)]` — and every
consumer skips its check rather than failing, so a changeset in flight when rdm
upgrades is not bricked. That is not politeness: `read_journal` silently skips
an unparsable line, so a required field would have made an older changeset
quietly lose its paths. On a path recorded twice, the last digest wins, exactly
as the last kind does.

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

### `RDM_HARNESS_FLUSH_BARRIER`

Not an identity variable — it appears here because it is the second member of
the documented harness-variable family, and it follows
`RDM_HARNESS_SESSION_ID`'s contract exactly: inert when unset, and bounded when
set, so it can never wedge a real run.

It names a file. When set, `FsStore::commit` blocks at the top of the flush
until that file exists, or 60 seconds pass — whichever comes first. It exists
because the read → write window inside one `rdm` invocation is sub-millisecond,
so two racing processes cannot be made to interleave at it by timing alone, and
the lost-update gate (`scripts/verify-lost-update.sh`) needs a *deterministic*
interleave of two real processes. See
[`lost-update-evaluation.md`](lost-update-evaluation.md) § "Harness design".

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

## What a changeset does at commit / status / discard time

Identity and the journal are the *mechanism*; these are the user-facing
surfaces built on them.

### `rdm commit`

Commits **this session's changeset**: the tree is HEAD plus exactly the paths
this session journaled, with its regenerated `INDEX.md` files reconciled in
memory against HEAD (never taken from disk — the on-disk index already holds
every session's rows). A path another session left dirty is structurally
unreachable, not filtered out late.

The reconciled index is complete in one direction only: it never names a path
the commit does not contain, but a document whose *project* was created by
another session that has not committed yet lands in the tree with no index row
for it. The row appears as soon as that session commits — see
`docs/scoping-model-decision.md` § "INDEX.md Consistency in Partial Commits".

| Flag | Meaning |
| --- | --- |
| *(none)* | Commit this session's changeset. |
| `--all` | Whole-tree commit, including every other session's uncommitted work. The machine-global escape hatch. |
| `--changeset <id>` | Commit a *named* changeset — the orphan-recovery path. Find ids with `rdm session list`. |

On success the landed paths are truncated out of the journal, so a second
commit cannot re-commit a path another session has since edited.

**The empty-changeset-but-dirty-tree diagnostic.** When this session's
changeset is empty but the working tree is not — a rung-4 fragmented session,
a raw `fs::write` outside rdm, or work done before this feature shipped —
`rdm commit` does **not** print `Nothing to commit.` and does **not** sweep.
It names the unattributed paths and prints all three recovery routes:

```text
Nothing in this session's changeset to commit.

1 uncommitted path(s) are attributed to another changeset:
  projects/demo/tasks/orphan.md

Recover them with one of:
  rdm session list                 # find the owning changeset
  rdm commit --changeset <id>      # commit that changeset
  rdm commit --all                 # commit the whole working tree
```

A journaled path whose working-tree file has since vanished (a concurrent
discard, a manual `rm`) is skipped and reported, never fatal.

### `rdm status`

Shows this session's changeset by default, `--all` for the whole tree. Output
is one partition into three buckets: this session's own edits, this session's
regenerated indexes (named separately), and a trailing line counting what
belongs to other changesets and pointing at `rdm session list` / `--all`.

### `rdm discard`

Changeset-scoped by default: restores only this session's journaled paths to
HEAD, clears its journal, and regenerates the indexes **from the resulting
disk state** so another session's still-uncommitted rows survive.
`--force --all` retains the whole-tree destruction, and prints the other live
changesets it is about to destroy before doing it.

### `rdm-server`

A long-lived server is **one session**: it resolves a single changeset at
startup (`--changeset <id>`, else `RDM_SESSION`, else the ordinary rung
chain), pins its store to it so every write really is journaled there, and by
default commits nothing. That default is loud on three surfaces rather than
one — a boot `WARN`, a stderr warning per mutation, and an `X-Rdm-Staged`
response header on every mutating response carrying the same text — because
"reported once at boot" is indistinguishable from "not reported" on a server
that has been up for a week. Every response also carries `X-Rdm-Changeset`,
so the id needed for `rdm commit --changeset <id>` is always in reach.
`--autocommit` (or `RDM_SERVER_AUTOCOMMIT=1`) makes each mutation land a
scoped commit instead. The full disposition, including the alternatives
rejected, is in `docs/scoping-model-decision.md` § "Which Interaction Layers
Are Covered".

### Reads are not scoped

Deliberately. `rdm task show`, `rdm search`, and every other read see the
whole working tree, including other sessions' uncommitted items. Scoping
applies to what a *write* action lands or destroys, never to what you can see.

## Gating

- `cargo nextest run` — unit tests in `rdm-core/src/session/**` (the rung chain
  over an injected `ProcessTable`/`EnvSource`, including the pid-recycle case,
  which is otherwise unconstructible), `rdm-store-git` tests for journal
  exactness, scoped-commit/discard behavior and determinism, and
  `rdm-cli/tests/cli_session.rs` / `cli_commit.rs` end to end against the real
  binary.
- `bash scripts/verify-session-identity.sh` — the identity harness: sections
  A–I as described above. § I is inverted as of phase 5: it now asserts that
  `rdm-store-git/src/commit.rs` *carries* the changeset commit scope while
  resolving no session identity of its own.
- `bash scripts/verify-scoped-commit.sh` — the multi-process scoping harness:
  disjoint concurrent commits (A), the same with no session id set plus
  rung-2 continuity and rung-4 degradation (B/B2/B3), the `Done:` hook path
  (C, distinct — every other section can pass while the hook still sweeps),
  the commit-primitive call-site allowlist (D), `init --remote` /
  `.gitattributes` back-fill / server reconciliation (E), committed-index
  reconciliation (F), scoped discard (G), and shared reads (H).
