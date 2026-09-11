# Lost-update evaluation

**Roadmap**: `plan-repo-concurrency`, phase 6 (extended by phase 9, which closed
the deletes carve-out — see § Selected mechanism, "Commit-time delete guard")
**Date**: 2026-08-08
**Outcome**: the window remained open. A mechanism was selected and implemented.

## Question

Phases 4 and 5 built session identity, a per-session changeset journal, and a
scoped `rdm commit`. Scoping fixes *attribution* — whose change lands under
whose message. It does not by itself fix a **lost update**: two sessions
reading the same task, both editing, last writer silently winning.

Whether that window is still open depends on what phases 4 and 5 actually
built. This phase's first job was to establish that, not to assume it. A phase
that correctly does nothing is a legitimate outcome, and this section exists so
that outcome would have been recordable.

## Method

Read the four places a lost update could be prevented, and check whether any of
them observes *content*:

1. the changeset journal (phase 4) — does it record base-blob identity?
2. the scoped commit's compare-and-swap (phase 5) — what does it compare?
3. the filesystem store's flush — is the write conditional on anything?
4. the mutation seam `ops::mutate` — what is the read-modify-write shape?

Plus: does phase 5's own decision record already speak to this?

## Evidence

**The journal records no content identity.** `JournalEntry` was
`{ path: String, kind: JournalKind }` and nothing else
(`rdm-core/src/session/journal.rs:65-72`, before this phase). It answers "which
paths did this session flush", never "what did this session write". So phase
4's journal could *not* already supply optimistic concurrency — the phase body
asked that this be established before adding a second mechanism, and it is
established: there was nothing there to reuse. (What it *could* supply was a
place to put such identity, which is what this phase did.)

**The compare-and-swap is on the git ref, not on content.**
`create_scoped_commit` (`rdm-store-git/src/commit.rs:551+`) seeds a tree from
HEAD, builds it, and updates the `HEAD` ref only if it still points at the oid
the tree was seeded from (`compare_and_swap_head`). That protects the *commit
graph* from a concurrent committer. It reads nothing about whether the
working-tree bytes it just blobbed are the bytes this session wrote. The
sibling `CommitLock` is best-effort and guards only the read-HEAD →
build-tree → update-ref window; it too observes no content.

**The flush is an unconditional overwrite.** `FsStore::commit`
(`rdm-store-fs/src/lib.rs:221-256`, before this phase) drained the staging
overlay and, for each entry, wrote to a temp file and `persist`ed it over the
target. There was no precondition of any kind on the target's current content.
The temp-file + rename gives per-file atomicity — it does not give
conditionality.

**The mutation seam is a read-modify-write.** `ops::mutate`
(`rdm-core/src/ops/mod.rs:70-79`) runs `f` (which reads the entity, edits it,
and stages the write), regenerates the project index, and calls
`store.commit()` exactly once. The read and the write are separated by the
whole body of `f`, and the flush that lands them is the unconditional one
above.

**Phase 5 already recorded this as a residual it deferred here.**
`docs/scoping-model-decision.md` § "Concurrency: what phase 5 closes, and what
it does not" lists two residuals "deliberately left to **phase 6**", the second
of which is:

> Attribution is **path-level, not content-level**: if another session
> overwrites a path this changeset journaled, the scoped commit commits the
> other session's bytes for that path.

## Verdict

**The window remains.** Nothing in the phase 4/5 stack observes content, so
nothing in it can distinguish "I am writing my edit" from "I am overwriting
someone else's".

The concrete instance, and the one the phase body names because `--tags`
replaces the whole list on update:

1. Session A runs `rdm task update fix-bug --tags <list>`. It reads the task,
   whose tags are `[needs-plan-review]`, and stages `[needs-plan-review, ui]`.
2. Session B runs the same command against the same task. It reads the same
   original, and stages `[needs-plan-review, perf]`.
3. A flushes. The file now reads `[needs-plan-review, ui]`.
4. B flushes. The file now reads `[needs-plan-review, perf]`.

A's `ui` tag is gone. Both commands exited 0. Nothing was reported. Substitute
a reserved tag — `needs-plan-review` itself, or `depends-unlanded` — and a gate
silently stops gating.

## Options considered

**(A) Do nothing.** *Rejected.* Recorded explicitly rather than omitted,
because "the phase correctly did nothing" was a permitted outcome and the
reason it does not apply here is specific: the defect is not hypothetical, it
is already written down as a known residual, and phase 5 — the phase that
recorded it — is `blocked`. No later phase inherits it. Doing nothing here
means it is never done.

**(B) Extend the journal with base-blob identity and check only at commit
time.** *Insufficient alone.* By the time `rdm commit` runs, the on-disk bytes
have already been clobbered; the other session's edit is already gone and is
not recoverable from the working tree. Detecting it then is better than not
detecting it, but it converts a silent loss into a loud one rather than
preventing it. **Adopted as the second half** of the selected mechanism, not as
the whole of it.

**(C) A whole-repo mutation lock serializing all mutations.** *Rejected.* It
converts a correctness problem into a liveness problem on the `Done:` hook
path, which is bounded by `hook_timeout_secs` and must never be blocked by
another session's slow mutation. And it does not even solve the problem: a
session that read the item *before* acquiring the lock still writes a stale
edit, correctly serialized.

**(D) Per-item advisory file locks.** *Rejected* for the same read-before-lock
reason as (C), plus lock lifecycle and garbage-collection cost that duplicates
phase 4's lease machinery for a narrower guarantee.

**(E) Content-digest optimistic concurrency at the store flush boundary.**
**Selected.** Described below.

## Selected mechanism

One notion — the sha256 of a file's content, `rdm_core::store::content_digest`
— applied at two boundaries.

### Flush-time precondition (the primary half)

`FsStore` records a `Baseline` (`Absent` | `Present(digest)` | `Unknown`) for a
store path the **first** time it touches it in the current read-modify-write
cycle: a read, an existence probe, a delete, or a blind staged write. First
touch wins within the cycle, so a later read served from the staging overlay
cannot overwrite the baseline with this process's own content.

At flush, **before any disk mutation**, every staged path is re-observed on
disk and compared to its baseline. Any mismatch aborts the whole flush with
nothing written and returns `Error::StaleWrite`, whose message names the item
in the `task/<slug>` vocabulary users already use, states that nothing was
written, and says to re-run the command.

**A baseline's lifetime is one cycle, not the store's.** A clean flush clears
every baseline, exactly as `discard` does, so the next touch of any path
observes disk afresh. Re-seeding only the flushed paths instead would be a
subtle trap: a plain read never overwrites an already-recorded baseline, so a
store that had once written a path would stay pinned to *its own* last bytes.
The first time another session edited that path, every subsequent write to it
would be refused forever — including the re-read-and-retry the error message
recommends — until the process was restarted. Single-shot CLI invocations would
never notice, but a store is not always single-shot: a long-lived host process
can hold one `GitStore` across many operations and flush repeatedly (the
now-retired MCP server was one such caller). Clearing keeps
the protection intact (a drift *within* a cycle is still caught on a reused
store) while keeping the recovery path open.

The check is keyed on **content, never on session id**. That is what makes a
session's own sequential writes safe — each cycle compares against what that
cycle actually observed — while still rejecting a genuinely stale read taken by
the same session id in a different process. The two are different situations and
the mechanism must not confuse them.

### Commit-time check (the second half, closing phase 5's residual)

`JournalEntry` gains an optional `digest`: the identity of the bytes that batch
flushed. `create_scoped_commit`'s tree builder compares each journaled write's
current working-tree digest to the journaled one and returns
`Error::ChangesetPathOverwritten` on mismatch rather than committing another
session's bytes under this changeset's message. This is the phase body's own
hint — base-blob identity per journaled path — realized rather than assumed.

### Commit-time delete guard (phase 9, closing phase 6's deletes carve-out)

Phase 6 shipped the two halves above for journaled **writes** only, and named
the deletes gap as a carve-out. Phase 9 closed it. The delete branch of
`build_changeset_tree` (`rdm-store-git/src/commit.rs`) is no longer a bare
`entries.remove(path)`.

**No new state was added, and that was established before implementing.** The
phase body required checking whether an existing mechanism already supplies the
needed value before adding a second one. Two candidates were read and both
rejected:

* **`FsStore`'s `Baseline`** (`rdm-core/src/store/mod.rs`, recorded and verified
  in `rdm-store-fs/src/lib.rs`) is held in `FsStore.baselines:
  Mutex<BTreeMap<String, Baseline>>` — an **in-memory, per-process** map that
  every clean flush and every discard clears. The window this guard closes spans
  two separate `rdm` invocations (stage the delete in one process, `rdm commit`
  in a later one, per rdm's explicitly batched workflow), so by the time
  `build_changeset_tree` runs the map is empty and structurally cannot supply
  anything. Independently, `Baseline` has **three** states (`Absent |
  Present(digest) | Unknown`) where a journaled `Option<String>` has two, so a
  straight reuse would lose the `Unknown` distinction.
* **A new `JournalEntry` field** was not added either. `JournalEntry` is a
  serialized, publicly documented type whose on-disk compatibility is a stated
  property, so an optional field is a permanent compatibility obligation — and
  the rule below reads nothing from the journal, so the field would have had no
  reader. `JournalEntry::digest` was likewise not reused: it is documented as
  "the bytes this batch flushed" and is `None` on a delete by construction.

**The rule is a presence test, not a content comparison.** A session that
deletes a path leaves it **absent**. So:

* **absent at commit time → match.** The delete proceeds. This covers a
  session's own write-then-delete and create-then-delete inside one uncommitted
  changeset, and a path another session already deleted and landed.
* **present at commit time → mismatch.** Refuse: something refilled a path this
  changeset emptied, and applying the delete would destroy it.

**The reference point is the working tree at commit time, never HEAD.** This
mirrors the write guard, which compares against `std::fs::read(&file)` and never
consults HEAD. A HEAD basis was considered and rejected outright: it conflates
"another session changed this" with "I changed this earlier in my own
uncommitted batch", and would therefore falsely refuse the stage-then-`rdm
commit` batching workflow rdm prescribes.

**Derived indexes are exempt**, exactly as they are in the write loop
(`rdm_core::paths::is_derived_path`). A journaled delete of a derived path is
routed into `ChangesetScope::deletes` by `commit_changeset_id`, and every
mutation regenerates those files, so an index this changeset deleted is
legitimately present again. Their commit-time correctness stays
`reconcile_derived`'s.

**Where the refusal lives.** `Error::ChangesetDeletePathRecreated { item, path }`
is defined in `rdm-core/src/error.rs`, raised from the delete loop of
`build_changeset_tree` in `rdm-store-git/src/commit.rs`, and rendered as HTTP
409 by `rdm-server/src/problem.rs` alongside `StaleWrite` and
`ChangesetPathOverwritten`. It is a **distinct variant** rather than a reuse of
`ChangesetPathOverwritten` because "overwritten" describes the wrong side of a
delete: nothing this changeset wrote was overwritten — the path it left absent
was refilled by someone else. Its message names the item via
`rdm_core::paths::describe_path` (`task/fix-bug`), states that nothing was
committed, and says to re-read the item and re-run the delete if it is still
right.

**Delete-then-recreate never reaches this guard**, and that is asserted rather
than assumed. `read_journal` collapses a path to its **last** recorded kind (a
`BTreeMap` keyed on path), so a path this changeset deleted and then recreated
journals as `Write`, lands in `ChangesetScope::writes` with a digest, and is
owned by the write guard.
(`a_write_recorded_over_a_delete_collapses_to_write` in
`rdm-core/src/session/journal.rs` and
`a_delete_then_recreate_is_routed_to_the_write_guard_not_the_delete_guard` in
`rdm-store-git/src/lib.rs` pin both ends of that routing.)

**Residual: a store-bypassing recreate still trips it.** A raw `fs::write`
outside the store that lands on a path this changeset deleted reads as
"present" and is refused, even when it is the same session that wrote it. This
is consistent with — and covered by — the "Store-bypassing writers are
uncovered" carve-out below, which stays.

## Carve-outs

These are deliberate gaps, named so they are not mistaken for coverage.

**Derived indexes are exempt.** `INDEX.md` and `projects/<p>/INDEX.md`
(`rdm_core::paths::is_derived_path`) are regenerated from disk by every
mutation via `ops::mutate`, so two concurrent sessions legitimately rewrite
them. Without this exemption every concurrent mutation would falsely trip.
Their commit-time correctness is already owned by phase 5's `reconcile_derived`,
which builds them in memory as HEAD-plus-this-changeset rather than reading the
shared on-disk copy. Since phase 8, a derived index whose parent project is
owned by an *uncommitted third session* is deferred rather than failing the
commit: the orphaned subtree is dropped from the generation seed and its rows
return on the owning session's next commit — see
`docs/scoping-model-decision.md` § "INDEX.md Consistency in Partial Commits".

*Historical note:* this exemption becomes moot once INDEX.md generation is
removed from the write path entirely — see
[`docs/index-removal.md`](index-removal.md). The carve-out above is preserved
as an accurate record of why it existed and how `reconcile_derived` worked
while INDEX.md was still generated on every mutation.

**One store-bypassing writer remains uncovered.** `.gitattributes` is written
by `ensure_gitattributes` (`rdm-store-git/src/repo.rs`) as a raw `fs::write`
that never enters the staging overlay, so it has no baseline and is not
checked. (`rdm.toml`'s config-set path used to share this gap via
`rdm-cli/src/paths.rs::save_repo_config`; that writer is gone — `rdm config
set` now routes through `rdm_core::io::save_config`, which writes through the
`Store` and is journaled like everything else.) This is a known gap inherited
from phase 5's own boundary, not an oversight. It is an effectively
write-once, low-contention file.

**The check→act window is narrowed, not eliminated.** Between the digest
comparison and the rename there remains an interval of microseconds in which
another process could write. A best-effort advisory flush lock
(`<git-dir>/rdm/flush.lock`, 5 s wait / 30 s staleness takeover, shared with
the scoped-commit lock via `rdm_core::lock::AdvisoryLock`) narrows it further.
The honest claim is a bound, not closure.

**Phase 5's residual #1 stays open.** A concurrent committer can still land
between a successful compare-and-swap and this process's post-commit index
sync. That is a different window and is not addressed here.

**Non-UTF8 and unreadable files fail open.** A path whose baseline could not be
observed records `Baseline::Unknown` and is skipped. The mechanism exists to
prevent lost updates; it must never become a new way to brick an unrelated
mutation.

**Legacy digest-less journal lines fail open.** A changeset journaled before
`JournalEntry.digest` existed carries no digest, so the commit-time check has
nothing to compare and commits the path as it stands. An in-flight changeset
must never be bricked by an upgrade
(`a_legacy_journal_line_without_a_digest_still_commits` locks this).

**Byte-identical concurrent writes are not conflicts.** The digests match, so
nothing is rejected — correctly, because there is no lost update.

## Harness design

**Why an in-process test cannot gate this.** A test that constructs two
`FsStore` handles in one process does not exercise the real window. The staging
overlay is in-memory and discarded at process exit — that is exactly the layer
that does not span invocations, and therefore exactly the layer the defect
lives below. Such tests exist (`rdm-store-fs`'s unit tests) and are useful for
the state machine, but the acceptance gate must drive two real `rdm` processes.

**Why a seam is needed.** The read → write window inside one `rdm` invocation
is sub-millisecond, so two racing processes cannot be made to interleave at it
reliably. The obvious pause point, `open_editor`, is unusable: `resolve_body`
only reaches it when stdin is a TTY (`rdm-cli/src/commands/mod.rs`), which a
harness is not.

**The seam.** `RDM_HARNESS_FLUSH_BARRIER` names a file. When set,
`FsStore::commit` blocks at its top until that file appears. It follows the
existing `RDM_HARNESS_SESSION_ID` precedent: a documented harness variable,
inert when unset, and **bounded** when set — 60 s, then it proceeds regardless
— so it can never wedge a real run even if a harness dies holding it.

**The gate.** `scripts/verify-lost-update.sh` parks process A at the barrier
mid-`task update --tags`, drives process B to completion, releases A, and
asserts A is refused, A's message names the item, and B's tags survive intact.
It repeats the scenario with no session id set (proving the mechanism is
content-keyed, not identity-keyed), gates the sequential-writes-never-trip
property with real back-to-back invocations, gates the commit-time half, checks
the `Done:` hook path still exits 0, and carries planted-mutation self-tests
proving each section can fail.

Its **section 6** gates the commit-time *delete* guard through the same
two-real-process discipline, and needs no barrier because the window it targets
is the naturally wide gap between staging and `rdm commit`: session A runs `rdm
promote` (whose `store.delete` is the CLI-drivable delete) and does not commit;
session B recreates a task at that same slug and lands its own commit first; A's
delayed `rdm commit` must be refused by name, and B's bytes must still be at
HEAD. Two self-tests bracket it — the same sequence *without* B's recreate must
commit cleanly and genuinely remove the path from HEAD, and a mutant binary with
the guard neutered must reproduce the lost update.

**What the shell gate structurally cannot reach.** Every invocation it drives is
a fresh `rdm` process with a brand-new store, so it can never exercise a store
that outlives one flush — the shape the baseline lifetime above exists for. That
is gated in Rust instead, at both layers: `rdm-store-fs`'s
`a_long_lived_store_re_observes_after_each_flush` (with
`a_stale_write_is_still_refused_on_a_reused_store` proving the protection
survives the clearing). The now-retired `rdm-mcp` crate's
`task_update_survives_an_external_edit_between_tool_calls` previously drove
this same scenario against a real long-lived-server production surface — two
tool calls against one long-lived server with a separate `rdm` process editing
the same task in between — and is gone along with that crate; the store-level
unit test above remains the coverage for the long-lived-store scenario itself.

## Interaction with phase 5

This mechanism lives at the store flush layer and is independent of phase 5's
`blocked` state: it is correct and gated whether or not scoped commits are
unblocked. The commit-time half touches phase 5's code but adds only a
precondition ahead of the existing blob write — it does not revisit
`reconcile_derived`'s known-broken path or the vanished-path reporting.

`docs/scoping-model-decision.md`'s "Two residuals" section is updated to point
here, so the two records cannot contradict each other.

## For phase 7

The agent-facing surfaces phase 7 teaches will encounter `StaleWrite`'s message
and the `describe_path` item vocabulary (`task/<slug>`, `roadmap/<slug>`,
`phase/<roadmap>/<stem>`, `review/<id>` — the same spellings as
`ReviewTarget::label`). Both are intended to be stable. The remedy an agent
should take is always the same: re-run the command, which re-reads the current
content.

## Retry attribution (phase 16)

This section is deliberately separate from Carve-outs above: a carve-out names
a window the mechanism does not check at all. This names a property of a
window the mechanism *does* check correctly — the refusal fires exactly when
it should — but whose prescribed remedy has a consequence worth stating
explicitly, because a session that only reads the refusal and never reads this
document could reasonably expect the retry to restore *its own* original edit
verbatim, and that is not quite what happens.

### The scenario

Every mutating rdm command is a read-modify-write over **current disk**, and a
successful flush writes real bytes to the real path immediately — there is no
in-memory-only staging that a later reader can miss. That single fact drives
the whole sequence below.

1. Session A reads item `X`, edits field `f1`, and flushes (`rdm task update
   X --f1 …`) without committing. `X`'s on-disk bytes now carry A's `f1` edit.
   A's changeset journals that flush's digest for `X`'s path.
2. Session B reads `X` — which, being a fresh read of current disk, already
   carries A's `f1` edit — edits a different field `f2`, and flushes without
   committing either. `X`'s on-disk bytes now carry **both** `f1` and `f2`.
   B's own changeset journals *this* flush's digest for the same path.
3. Session A runs `rdm commit`. `build_changeset_tree` compares `X`'s current
   on-disk digest against the one A's changeset journaled in step 1 and finds
   a mismatch — B's flush in step 2 changed the bytes at that path after A's
   own flush wrote them. A's commit is refused with
   `Error::ChangesetPathOverwritten`, whose message (`rdm-core/src/error.rs`)
   now says plainly that a retry will fold in whatever is on disk, which may
   be another session's already-landed edit.
4. Session A retries by re-running the exact command from step 1
   (`rdm task update X --f1 …`, the Direction section's and the phase body's
   prescribed remedy). That command re-reads `X` from disk as its baseline —
   which already carries both `f1` and `f2` — and re-applies `f1` (a no-op
   against what is already there, since A's own value never changed), so the
   flush is byte-stable and A's changeset journal now agrees with disk. A then
   runs `rdm commit`, which finds a matching digest and lands a single commit,
   under A's message, whose tree holds `X` with both `f1` and `f2` applied —
   the first commit to actually reach HEAD with either edit in it.
5. Session B, which never committed, runs `rdm commit`. B's own journaled
   digest (from step 2) still matches current disk — nothing has touched the
   path since — but that content is now already at HEAD, landed by A's commit
   in step 4. There is nothing left for B's changeset to contribute, so B's
   commit reports "Nothing to commit." rather than creating an empty commit.

### What the landed commit contains, and why that is acceptable

The commit that lands under **A's message** carries **both** `f1` (A's edit)
and `f2` (B's edit), because step 4's read-modify-write cycle read a document
that already had `f2` applied and only re-applied `f1` on top of it. Nothing
is silently lost: B's `f2` edit is present in the tree, in exactly the bytes B
wrote, and is provable by inspecting the landed content — the digest guard's
entire purpose is to make that provable rather than assumed. What shifts is
**attribution**, not data: the commit message and authorship are A's, even
though the tree also carries B's change. B's subsequent `rdm commit` correctly
reports nothing pending: B's own changeset journal (recorded at its step 2
flush) still matches current disk, but that content is already at HEAD via
A's commit, so there is nothing left for B's changeset to land.

This is accepted as the changeset model's deliberate **rebase-onto-current-disk
semantics**, for the same reason phase 5's compare-and-swap and this
document's own flush-time precondition are content-keyed rather than
session-keyed (see Selected mechanism above): rdm's mutating commands are
read-modify-write over "current disk," by design, so that a session's own
sequential edits compose correctly. A retry is not a special case of that
design — it is an ordinary invocation of the same read-modify-write command,
and it behaves exactly as every other invocation of that command behaves: it
reads what is there and edits it. Treating a retry differently (e.g. reading
whatever the journal claims A's *original* baseline was, instead of current
disk) would require the store to remember a stale baseline across the refusal
and thread it back into a plain re-invocation of an ordinary CLI command,
which no other rdm workflow does and which would silently reintroduce the
very "am I overwriting someone else's flush" ambiguity the digest guard
exists to resolve — this time one layer up, at the retry's own flush.

### Rejected alternative: refuse until B commits

The Direction section names this option: hold A's flush pending and refuse to
let A commit until B's changeset has landed, rather than letting A's retry
proceed against B's already-landed content. **Rejected**, for the same reason
option (C) in Options considered above was rejected: it requires new
cross-session state — tracking which sessions are "ahead" of which at a given
path and blocking a commit on another session's future action — that
contradicts the stateless-at-commit-time design the digest guard was built
to keep. It would also convert a correctness signal (the refusal, which fires
today) into a liveness hazard: A's retry-and-commit sequence would have no
bound on how long it waits for B, on a path that includes the `Done:` hook,
which `hook_timeout_secs` requires to never block indefinitely on another
session's pace. The digest guard already gives A everything a well-behaved
retry needs — a loud, before-the-fact refusal naming exactly which item and
path changed underneath it — without introducing a wait.

### Boundary

This section covers only the two-session, same-command-retry sequence the
phase's acceptance criterion names. A's retry that edits something *different*
from its original attempt, or a third concurrent session `C`, are out of
scope — see Edge cases in this phase's own approved plan.
