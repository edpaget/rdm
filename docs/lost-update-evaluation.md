# Lost-update evaluation

**Roadmap**: `plan-repo-concurrency`, phase 6
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
store path the **first** time this process touches it: a read, an existence
probe, a delete, or a blind staged write. First touch wins, so a later read
served from the staging overlay cannot overwrite the baseline with this
process's own content.

At flush, **before any disk mutation**, every staged path is re-observed on
disk and compared to its baseline. Any mismatch aborts the whole flush with
nothing written and returns `Error::StaleWrite`, whose message names the item
in the `task/<slug>` vocabulary users already use, states that nothing was
written, and says to re-run the command. On a clean flush, each path's baseline
is re-seeded to the content just written.

The check is keyed on **content, never on session id**. That is precisely what
makes a session's own sequential writes safe — the second flush compares
against what the first flush wrote — while still rejecting a genuinely stale
read taken by the same session id in a different process. The two are
different situations and the mechanism must not confuse them.

### Commit-time check (the second half, closing phase 5's residual)

`JournalEntry` gains an optional `digest`: the identity of the bytes that batch
flushed. `create_scoped_commit`'s tree builder compares each journaled write's
current working-tree digest to the journaled one and returns
`Error::ChangesetPathOverwritten` on mismatch rather than committing another
session's bytes under this changeset's message. This is the phase body's own
hint — base-blob identity per journaled path — realized rather than assumed.

## Carve-outs

These are deliberate gaps, named so they are not mistaken for coverage.

**Derived indexes are exempt.** `INDEX.md` and `projects/<p>/INDEX.md`
(`rdm_core::paths::is_derived_path`) are regenerated from disk by every
mutation via `ops::mutate`, so two concurrent sessions legitimately rewrite
them. Without this exemption every concurrent mutation would falsely trip.
Their commit-time correctness is already owned by phase 5's `reconcile_derived`,
which builds them in memory as HEAD-plus-this-changeset rather than reading the
shared on-disk copy.

**Store-bypassing writers are uncovered.** `rdm.toml` is written by the raw
`fs::write` in `rdm-cli/src/paths.rs::save_repo_config`, and `.gitattributes`
by `ensure_gitattributes` (`rdm-store-git/src/repo.rs`). Neither enters the
staging overlay, so neither has a baseline and neither is checked. This is a
known gap inherited from phase 5's own boundary, not an oversight. Both are
effectively write-once, low-contention files.

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
