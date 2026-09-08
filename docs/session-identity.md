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
| **Creation** | Create a lease **only at depth 1**, the immediate parent. Never higher. Phase 11 measured raising this and rejected it — see [Continuity across ephemeral wrapper shells](#continuity-across-ephemeral-wrapper-shells-phase-11). |

Because creation never ascends, two concurrent sessions can share an identity
only if some ancestor they have in common was itself the immediate parent of an
earlier `rdm` invocation. When the parent is the ephemeral per-invocation shell
an agent spawns — the case the decision record measured — no ancestor is ever
leased, each invocation gets its own changeset, and the session fragments. That
is the safe direction, and rung 3 is what covers this case in practice — phase
11 confirmed both halves of that sentence by measurement, and made `rdm commit`
say so out loud when rung 3 is not in play.

The ascent terminates on all three degenerate shapes: it is bounded at 8, it
stops at pid ≤ 1, and it carries a seen-set so a cycle cannot loop.

### Why rung 3 now precedes rung 2, and lease creation still defers to it

**Phase 10 amendment.** Phase 3's decision record originally specified
checking the inherited lease (rung 2) before the harness variable (rung 3).
That order let two Claude Code sessions launched under one already-leased
ancestor shell **merge onto one changeset**: a parent shell that ran a single
bare, harness-less `rdm` invocation minted a lease at that parent, and every
child launched under it — even one carrying its own distinct
`CLAUDE_CODE_SESSION_ID` — silently adopted that inherited lease instead of
its own harness-derived id. This is exactly the "stopping too HIGH" merging
failure the decision record's stopping-rule asymmetry names as unacceptable,
just reached through rung ordering rather than ancestor depth. Reproduced
2026-09-01 and fixed by phase 10 of the `plan-repo-concurrency` roadmap; see
`scripts/verify-session-identity.sh` § J for the real-process regression.

The fix: a harness-published session id is an **explicit statement of session
membership** and must outrank an inherited on-disk artifact that predates it.
Rung 3 is therefore now *checked* before rung 2. Lease **creation** still
defers until after rung 3 has been consulted — a harness that already
publishes a stable id has nothing for a lease to bootstrap, and creating one
anyway would make rung 3 unreachable in practice for that process's own later
invocations. So the order is:

1. `RDM_SESSION` → rung 1.
2. First non-empty `HARNESS_SESSION_VARS` entry → rung 3.
3. Adopt an existing ancestor lease → rung 2.
4. Create a lease at the immediate parent → still rung 2.
5. Per-process id → rung 4.

The "defers to it" half of this section's title is now trivially true: since
lease bootstrap (step 4) is reached only once both explicit and harness have
already been ruled out, there is no order in which a lease could be created
ahead of a harness check that would still apply. The rung *numbers* are
unaffected — `Rung::Lease` is still rung 2 and `Rung::Harness` is still rung
3, matching `docs/scoping-model-decision.md`'s binding vocabulary; only the
order `resolve_id` evaluates them in has changed.

## Continuity across ephemeral wrapper shells (phase 11)

**Phase 11 evaluated raising lease creation above depth 1, and rejected it on
measurement.** Creation is unchanged. What ships instead is a bounded lease
directory and an honest diagnostic. This section records the reproduction, the
evidence for each candidate, and the safety argument, so a later phase does not
re-derive it.

### The reproduction

Under an agent harness that exports none of `HARNESS_SESSION_VARS` and runs
each tool call in a fresh wrapper shell, every `rdm` invocation is its own
changeset. Reproduced 2026-09-08 on darwin with a long-lived non-shell driver
spawning one `bash -c 'eval "$CMD"; :'` wrapper per call (§ K of the harness
builds the same shape with `sh -c`; the `eval` and the trailing `:` are what
stop the shell exec'ing rdm in place and collapsing the wrapper away):

```
call 1  rdm session id   ->  s-5beab50f628730c2   rung 2
call 2  rdm session id   ->  s-6b36079d1bd94b93   rung 2      (already diverged)
call 3  rdm task create k-item
call 4  rdm commit       ->  "Nothing in this session's changeset to commit.
                              3 uncommitted path(s) are attributed to another changeset"
```

Nothing lands; only `rdm commit --all` works. Before this phase the run also
left **four** lease files, one per invocation, every one naming a pid that died
with its wrapper, plus an orphan journal.

The cause is structural, not a bug: creation stops at depth 1 (see [The shipped
stopping rule](#the-shipped-stopping-rule)) and under this topology depth 1
*is* the wrapper, which outlives nothing.

### The three candidates, and why only two shipped

The decision rule was fixed before measuring: ancestor-minting would ship only
if (i) a per-process command name **and** a session-boundary signal were both
obtainable on Linux and darwin, (ii) the added cost stayed inside § H's
250 000 µs bound, and (iii) an enumerated topology table showed no shape in
which two concurrent sessions select the same anchor.

**(a) Mint the lease at the nearest non-shell ancestor — REJECTED.** It fails
conditions (i) and (iii), each independently fatal.

*Condition (i) — the guard signal does not exist on darwin.* An ascent that
crosses shells is only safe if it stops at a session boundary, which needs a
per-process session id. Linux has one (`/proc/<pid>/stat` field 6). macOS does
not expose one to an unprivileged reader: `ps -Ao sess=` reports `0` for
**every** process on the system (`ps -Ao sess= | sort -u` yields exactly one
distinct value), `tsess` likewise, and there is no `sid` keyword. The rule's
own fail-safe — no session signal, no ascent — would therefore have made the
mechanism permanently inert on darwin while changing behavior on Linux: a
platform-split continuity model, on a project whose own dogfooding host is
darwin.

*Condition (iii) — a real captured topology merges.* Two `rdm`-using scripts
backgrounded from one shell inside an agent harness were expected to be safe,
on the reasoning that the ascent would terminate at the shared interactive
shell and fall back to depth 1. The captured ancestry shows otherwise:

```
script A: sh(48792) -> bash(48791) -> bash(48750) -> claude(70969) -> ...
script B: sh(48793) -> bash(48791) -> bash(48750) -> claude(70969) -> ...
                       ^-------- shared, and all shells --------^   ^ shared non-shell
```

A shell-crossing ascent walks both scripts past three shells to the **same**
`claude` process and mints one anchor for both — merging two independent
sessions. That is precisely the "stopping too high" direction
`docs/scoping-model-decision.md` declares unacceptable. A deny-list of
never-anchor command names does not rescue it: it would have to enumerate every
agent, editor, IDE, runner and supervisor that can sit above a user's shells,
and the one guard that handles them generically is the session boundary that
condition (i) already ruled out.

For completeness, the shapes that *are* handled by a deny-list were captured
too — two tmux panes share a single `tmux` server at depth 2, and a terminal
tab's shell sits under `login` — but a rule that is safe only for the shapes
someone remembered to list is not the safe direction.

**(b) Extend `HARNESS_SESSION_VARS` and document the adoption path — SHIPPED.**
No new variable was invented. `RDM_HARNESS_SESSION_ID` already exists as the
universal adoption path, and no Pi-published session variable was observable to
add (the rule was to add nothing rather than guess a name). What this phase adds
is the documented, verified path below, gated by
`scripts/verify-session-identity.sh` § K5: the same wrapper topology with
`RDM_HARNESS_SESSION_ID` exported resolves **one** id at rung 3 across all
calls, `rdm commit` lands the previous call's mutation, and **zero** leases are
written.

**(c) Say plainly that the harness is unsupported, and how to fix it —
SHIPPED.** `rdm commit` now prints a cause-and-remedy advisory
(`session::continuity_advisory`) on exactly the branches that are symptoms: an
empty changeset over a tree that is not empty. It fires only when no harness
variable is set **and** this invocation inherited nothing — it either minted its
own rung-2 lease (`lease_bootstrapped`) or fell through to rung 4. A caller that
*adopted* a lease has continuity and stays silent, which is what keeps an
ordinary interactive shell quiet from its second command on.

That last qualifier is why the wording is what it is. The condition cannot
distinguish "every call mints its own changeset" from "this is the first call
from a long-lived shell that will inherit fine afterwards" — both are a rung-2
bootstrap, and nothing rdm can read says whether the parent will outlive the
command. So the advisory asserts only what is certainly true of the invocation
in hand ("this invocation started a new changeset rather than joining one"),
states the diagnosis conditionally ("if that happens on every `rdm` call…"),
and names the benign reading explicitly. Claiming the harness is broken
unconditionally would be wrong for the interactive user, and a diagnostic that
is sometimes false is worse than none.

**Lease hygiene — SHIPPED.** `lease::create_at_parent` now runs the existing
bounded `gc` immediately before minting. The fragmenting topology therefore no
longer also leaks: the reproduction above drops from four lease files to one,
and stays there however many tool calls run. `gc` is reused unchanged, so it
still skips entirely when the process table cannot see the caller (an empty
table means "unknown", never "nothing is alive") and still never touches a
journal.

This changes *how many lease files exist*, and nothing about what any of them
means. `live_lease_ids` already ignored a lease whose pid is gone or whose
recorded start time no longer matches, so the set of ids it reports live — and
therefore which changesets `rdm session list` flags `orphaned` — is exactly what
it was before. Anything downstream reading the lease directory should treat this
as a change in population, not in semantics.

### Why this is the safe direction

Fragmentation is what phase 3's binding asymmetry asks for when the two
failures cannot both be avoided: stopping too high merges concurrent sessions
and loses work irrecoverably; stopping too low splits one session's batch,
which is visible, non-destructive, and recoverable via `rdm commit --changeset
<id>` — a route the same advisory prints. Phase 10's no-merge invariant is
untouched because no ascent was added, and § K4 re-asserts it under the wrapper
topology specifically.

### Which harnesses get continuity, and how

| Harness | Rung that carries continuity | What the operator must do |
| --- | --- | --- |
| Claude Code, `CLAUDE_CODE_SESSION_ID` exported (the default) | 3 | Nothing. |
| Claude Code with the variable stripped | 2 per invocation — **fragments** | Export `RDM_HARNESS_SESSION_ID` (below), or use `rdm commit --changeset <id>`. |
| Pi | 3, once a variable is exported | Pi publishes no session variable that rdm could observe, so export one yourself: from a Pi extension or startup hook, run `export RDM_HARNESS_SESSION_ID="$(uuidgen)"` once per session, before any rdm command. |
| Any other agent harness | 3, once a variable is exported | Same `RDM_HARNESS_SESSION_ID` path. Adding a harness's own variable to `HARNESS_SESSION_VARS` is the alternative, and needs only a one-line change. |
| A plain interactive shell | 2, at the shell itself | Nothing — the shell is long-lived, so the lease it mints is inherited by every later invocation. Unchanged by this phase. |
| CI / a one-shot script | 1 | `export RDM_SESSION=<id>` for the job. |
| A git hook rdm itself spawned | inherits the caller's | Nothing. |
| A platform with no readable process table | 4 — **fragments** | Same `RDM_HARNESS_SESSION_ID` path; the `rdm commit` advisory fires and names it. |

The adoption path in full, for any harness that publishes nothing:

```sh
# once per session, before the harness runs any rdm command
export RDM_HARNESS_SESSION_ID="$(uuidgen)"
```

It must be a value that is **stable for the session and distinct between
concurrent sessions** — the same two properties rdm derives for itself on every
other rung. Reusing one fixed string across concurrent sessions would merge
them, which is the failure this whole roadmap exists to remove.


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
| Lease | Once per parent, on the first bare invocation under it | By GC when its pid is dead or recycled; opportunistically during the ancestry walk **and on every creation** |
| Journal | On the first flushed batch of a changeset | **Never automatically** — only by `rdm session discard --force` |

GC (`rdm session gc`, opportunistically during adoption, and — since phase 11
— once on every lease *creation*) removes leases whose owning process is gone
or whose recorded start time no longer matches, bounded at 64 files per pass.
It never touches a journal, so no work is silently destroyed.

The create-path sweep is what keeps a *fragmenting* topology from also being a
*leaking* one. Under a per-tool-call wrapper harness every invocation reaches
lease creation and mints at a parent that dies moments later; sweeping first
holds the directory at roughly one entry instead of one per invocation.
Sweeping happens *before* minting, never after — the entry just written names a
live pid by construction. It cannot disturb a concurrent session, because a
live session's parent is present in the same system-wide table and therefore
never looks stale; `scripts/verify-session-identity.sh` § K3/§ K3b gate the
bound, and `lease.rs`'s
`creating_a_lease_never_sweeps_a_live_concurrent_sessions_lease` gates the
limit on it.

GC is **skipped entirely** when the process table cannot see the calling
process. A table that reads as empty means "unknown", never "nothing is alive";
treating it as the latter would sweep away every live session's lease.

**Orphan recovery.** A session killed mid-batch leaves a journal with no live
lease. `rdm session list` flags it `orphaned: true`; `rdm session adopt <id>`
re-points the caller's immediate-parent lease at it, so the caller's shell
resolves that changeset from then on; `rdm session discard <id> --force` drops
it.

Because adoption repoints the *immediate parent's* lease, it does nothing
useful from inside an ephemeral per-tool-call wrapper shell: the repointed
lease belongs to a wrapper that exits before the next call can inherit it. Use
`rdm commit --changeset <id>` to land such a changeset directly, or
`RDM_SESSION=<id>` to pin it — both routes the `rdm commit` advisory prints.

Adoption works by writing rung-2 state (the parent lease), so it only takes
effect for a caller who would otherwise resolve at rung 2 or below. Since
phase 10, rung 3 (harness) is checked before rung 2, so a caller with a
harness variable set (e.g. `CLAUDE_CODE_SESSION_ID`) would never reach the
repointed lease — `rdm session adopt` detects that up front and refuses with
an actionable error naming the offending variable, rather than reporting
success and silently doing nothing. Adopt from a shell with no harness
variable set instead, or set `RDM_SESSION=<id>` to pin the id explicitly
(rung 1, which always wins).

## Environment variables

| Variable | Rung | Notes |
| --- | --- | --- |
| `RDM_SESSION` | 1 | The explicit escape hatch. Wins outright — no lease read, no ancestry walk, no harness read. |
| `CLAUDE_CODE_SESSION_ID` | 3 | The reference harness entry named by the decision record. |
| `CLAUDE_SESSION_ID` | 3 | |
| `RDM_HARNESS_SESSION_ID` | 3 | The generic hook for a harness with no native variable, and the documented adoption path for any harness that fragments — see [Which harnesses get continuity, and how](#which-harnesses-get-continuity-and-how). Exposed in code as `session::HARNESS_ADOPTION_VAR`, which `continuity_advisory` names as the remedy. |

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
| Parent is an ephemeral per-tool-call wrapper shell | Rung 2, a fresh changeset per invocation; `rdm commit` prints the cause and the remedy |
| Lease creation racing another process at the same parent | Both converge on the winner's id (temp file + hard link + read-back) |

Journal recording is best-effort at the call site (`let _ = …`, mirroring
`HookLogger`'s swallow-failures contract): an unwritable state directory must
never fail a mutation.

## Process backends

| Target | Backend |
| --- | --- |
| Linux | `/proc/<pid>/stat` for every numeric entry in `/proc`. Fields are parsed **after the last `)`**, so a process name containing spaces or parentheses cannot shift `ppid` (field 4) or `starttime` (field 22). |
| Other unix | One `ps -Ao pid=,ppid=,lstart=` spawn. `lstart` is an absolute start time and therefore stable, unlike the relative `etime`; its embedded spaces are handled by splitting only the first two fields. |
| Anything else | An empty table. |

Phase 11 considered adding `comm` and a per-process session id to this table to
support minting above depth 1, and did not: macOS exposes no session id to an
unprivileged reader, so the guard that would have made such an ascent safe
cannot be built portably. No second `ps` spawn was added and the cost below is
unchanged. See [the phase-11
section](#continuity-across-ephemeral-wrapper-shells-phase-11).

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

Re-measured for phase 11 on the same host: **17 546 µs** max over 20 fresh
invocations, unchanged within noise. That is expected — phase 11's decision
rule made a second `ps` spawn conditional on shipping ancestor-minting, which
it did not, so no new per-invocation cost was introduced. The bounded `gc` pass
phase 11 *did* add to the lease-creation path is a single `read_dir` plus at
most `MAX_GC_ENTRIES` small file reads, and it runs only on a rung-2 bootstrap,
not on adoption.

`scripts/verify-session-identity.sh` § H prints the observed maximum and
asserts it under 250 000 µs — a bound, not an exact figure. That is three
orders of magnitude under the 30 s default `hook_timeout_secs`, so identity
resolution cannot put the unattended hook path near its deadline. § H2 runs a
real `Done:`-bearing `rdm hook post-commit` end to end and asserts it exits 0,
applies its directive, and journals its own writes.

## CLI surface

| Command | Purpose |
| --- | --- |
| `rdm session id` | Print this session's changeset id. `--format json` adds `rung`, `resolve_micros`, and `lease_bootstrapped`. Text mode prints the bare id for `$(...)` capture. |
| `rdm session journal [--id <id>]` | Print a changeset's exact journaled path set. |
| `rdm session list` | List every changeset, flagging orphans. |
| `rdm session adopt <id>` | Re-point this session at an existing (usually orphaned) changeset. |
| `rdm session discard <id> --force` | Delete a changeset's journal. Irreversible, hence the flag. |
| `rdm session gc` | Remove leases whose owning process is gone or recycled. |

The JSON field names (`id`, `rung`, `resolve_micros`, `orphaned`, `paths`) are
a stable target for the agent-surface phase; do not rename them casually.
`lease_bootstrapped` (phase 11) is **additive** to that set: it is `true` only
when this invocation reached rung 2 by *creating* a lease rather than
inheriting one, which is how a caller can tell "my shell owns a changeset" from
"every call is minting its own". Text mode is unchanged — still the bare id.

`rdm commit` prints a cause-and-remedy advisory when a caller has no continuity
to inherit and no harness variable set. It appears only on the two branches
that are symptoms — an empty changeset over a non-empty tree — so a successful
commit and a no-op commit against a clean tree both stay quiet, and its text is
phrased to stay true of the one benign case that meets the same condition (the
first command from a long-lived shell).

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

Scoping alone only guarantees a *disjoint* path is safe — it says nothing
about a path two sessions both journaled. For that case the scoped restore
applies a content-digest/presence guard, mirroring the one `rdm commit`
already applies before landing a path: before restoring a journaled **write**,
it compares the file's on-disk content against the digest this session
recorded when it wrote there, and before restoring a journaled **delete**, it
checks the path is still absent. A mismatched digest, or a delete path that
is present again, means another live session's uncommitted work has landed on
a path you also claim since you last touched it — that one path is left
exactly as it is (never reverted to HEAD, never removed) and named on a
`skipped:` line, while every other path you legitimately own in the same
batch still discards normally and the command still exits 0. This is a
per-path skip, not an all-or-nothing refusal: unlike a commit, a discard has
no single-tree-object atomicity constraint forcing it to abandon the whole
batch over one contested path. A vanished or non-UTF8 file, or a legacy
journal line with no recorded digest, fails open exactly as the commit-side
guard does — the guard can only refuse a comparison it can actually make.

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
  A–K as described above. § I is inverted as of phase 5: it now asserts that
  `rdm-store-git/src/commit.rs` *carries* the changeset commit scope while
  resolving no session identity of its own. § J (phase 10) gates the
  harness-id-beats-inherited-lease rule. § K (phase 11) drives real per-call
  `sh -c 'eval …; :'` wrapper shells under a long-lived non-shell driver and
  gates this section's outcome: the wrappers are genuinely distinct live
  processes (K0), each call fragments (K1), `rdm commit` exits 0 and names both
  the cause and the remedy while the work stays recoverable (K2), the lease set
  stays bounded with the dead wrapper's entry swept (K3/K3b), two concurrent
  drivers never merge (K4), the documented `RDM_HARNESS_SESSION_ID` remedy
  really does yield one changeset, a landing commit and zero leases (K5), and
  this document still carries the per-harness table (K6). Two planted-mutation
  self-tests rebuild a mutant in a scratch `CARGO_TARGET_DIR` and prove neither
  half is vacuous: silencing `continuity_advisory` must break K2, and removing
  the create-path sweep must break K3.
- `bash scripts/verify-scoped-commit.sh` — the multi-process scoping harness:
  disjoint concurrent commits (A), the same with no session id set plus
  rung-2 continuity and rung-4 degradation (B/B2/B3), the `Done:` hook path
  (C, distinct — every other section can pass while the hook still sweeps),
  the commit-primitive call-site allowlist (D), `init --remote` /
  `.gitattributes` back-fill / server reconciliation (E), committed-index
  reconciliation (F), scoped discard (G), and shared reads (H).
