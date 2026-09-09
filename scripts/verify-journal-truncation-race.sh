#!/bin/sh
# Hermetic regression for the changeset journal's truncation race:
# `journal::truncate` under concurrent `journal::record` appends.
#
# Gates `plan-repo-concurrency/phase-15-concurrent-journal-truncation-race`:
#
#   1   the AC's stress scenario — N=40 parallel mutations across two projects
#       plus M=6 interleaved commits, all under ONE shared RDM_SESSION —
#       leaves every task present, no AUTHORED file stranded, a journal that
#       claims nothing, and a final `rdm commit` with nothing to do and no
#       "belong to another changeset" line. It also disentangles the
#       separately-filed pre-flush index-read race
#       (`index-regen-reads-before-flush-lock`): a derived INDEX.md may still
#       be dirty here for that unrelated reason, and ONE quiescent
#       regeneration must be enough to settle it, after which the tree is
#       asserted fully clean
#   2   determinism, not luck: two REAL processes interleaved at the
#       documented RDM_HARNESS_JOURNAL_BARRIER seam — a commit parked inside
#       `truncate` while a full `rdm task create` runs to completion — and the
#       concurrently appended records survive, including the regenerated
#       INDEX.md files, which name paths the parked commit is landing
#   2b  the mutant-binary self-test: `truncate` reverted to its
#       read-modify-write form, rebuilt, and re-run through section 2's
#       interleave. The loss MUST reappear, or section 2 proves nothing
#   3   assertion self-tests: corrupted expectations must go red
#   4   the semantic-empty contract — "empty" means `read_journal` folds to
#       zero entries, NOT that the file is gone. Truncation is append-only,
#       so gating on file absence would make CI intermittently red for a
#       correct implementation
#   5   `rdm session gc` — the ONE non-append operation — driven concurrently
#       with an active append, from a process sharing neither the session id
#       nor a lease with the appender. No lease can ever name a rung-1 or
#       rung-3 changeset, so gc genuinely cannot tell that anyone is appending;
#       the parked append holds the kernel-enforced journal lock shared, so
#       gc must be EXCLUDED — it leaves the journal exactly as it found it —
#       and the record must land
#   5b  the same interleave against a mutant with the journal lock stripped
#       from both the append and the sweep: gc must rewrite the journal under
#       the parked append and the record must be lost — otherwise section 5
#       proves nothing
#   6   the other half of the exclusion, and the window the earlier draft left
#       open: compaction's length compare-and-swap was checked once, early,
#       and nothing re-checked the journal before the `rename`, so an append
#       made in between landed in the doomed inode and was renamed away. A
#       real `rdm session gc` is parked at RDM_HARNESS_COMPACT_BARRIER —
#       after its CAS, before its rename, holding the lock exclusively — while
#       a real `rdm task create` arrives; the append must WAIT the sweep out
#       and land in the rewritten journal
#   6b  the mutant self-test for section 6: with the lock stripped, the append
#       lands in the inode the parked sweep renames over, and MUST be lost
#   7   the last writer that used to ignore the protocol: `rdm session discard
#       --force`, which destroys a changeset deliberately and did it with a
#       bare `remove_file`. A real discard is parked mid-destruction while a
#       real `rdm task create` appends under the same shared session id; the
#       append must survive, and the discard must still retire everything it
#       read
#   7b  the mutant self-test for section 7: with the unlink restored, that
#       record MUST be lost and its file left behind as unattributed dirt
#
# The window between a commit reading its journal and truncating it is opened
# and closed inside one `rdm` invocation, so two real processes cannot be made
# to interleave at it by timing alone. Hence the barrier seam, which follows
# RDM_HARNESS_FLUSH_BARRIER's contract exactly: inert when unset, bounded when
# set.
#
# Run after touching `journal::record` / `truncate` / `read_journal` /
# `append_line` / `compact` / `gc_changesets` / `discard_changeset` in
# rdm-core/src/session/journal.rs, the `lock_journal` / `journal_lock_path`
# helpers in the same file, either `journal::truncate` call site in rdm-store-git
# (`commit_changeset_id`, `commit_whole_tree`), the `rdm session gc` sweep in
# rdm-cli/src/commands/session.rs, or any of the RDM_HARNESS_JOURNAL_BARRIER /
# RDM_HARNESS_APPEND_BARRIER / RDM_HARNESS_COMPACT_BARRIER seams.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo). No network.
# Every wait is bounded and fails loudly — CI runs this unattended.
#
# Cost: the run is dominated by ONE cold `cargo build -p rdm-cli --offline`
# under a scratch CARGO_TARGET_DIR, shared by sections 2b, 5b, 6b and 7b
# rather than built four times. Measured on a 2026 laptop: ~13s for that build and
# ~57s for the whole script; a 2-core CI runner pays proportionally more for
# the build (~82s of CPU) and roughly two minutes overall. If that ever becomes
# unacceptable, the remedy is a cheaper build — never a skipped or weakened
# mutant self-test, without which sections 2, 5, 6 and 7 prove nothing.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# Clear rdm-related env inherited from the caller's shell so the run never
# touches the developer's real plan repo, and so every section's session
# identity is the one it sets. An inherited RDM_HARNESS_JOURNAL_BARRIER is the
# dangerous one: it would park every process in section 1 against a file that
# never appears, turning the stress run into a silent 60-second no-op.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_SESSION
unset CLAUDE_CODE_SESSION_ID CLAUDE_SESSION_ID RDM_HARNESS_SESSION_ID
unset RDM_HARNESS_FLUSH_BARRIER RDM_HARNESS_JOURNAL_BARRIER RDM_HARNESS_APPEND_BARRIER
unset RDM_HARNESS_COMPACT_BARRIER
# An inherited git environment (set whenever this runs under a git hook) would
# point every `git -C <temp-repo>` query below at the invoking repo instead.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

# `rdm <entity> create` reads a body from stdin when none is passed inline, so
# an inherited open stdin would hang the run. Detach it once, for this script
# and everything it spawns.
exec </dev/null

export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# seed_repo <dir> [bin]: a plan repo with TWO projects, each holding one
# committed task.
#
# Two projects because the reported symptom names BOTH index files — the
# top-level INDEX.md and a per-project one — and a single-project repo would
# exercise only half of it.
#
# Pins RDM_SESSION so the seeding short-circuits at rung 1 before any lease is
# created: a lease held by this script's own pid would be inherited by every
# process below and quietly merge sections that mean to be distinct.
seed_repo() {
    _dir=$1
    _bin=${2:-$RDM_BIN}
    mkdir -p "$_dir"
    RDM_SESSION=harness-seed "$_bin" --root "$_dir" init --default-project alpha >/dev/null
    RDM_SESSION=harness-seed "$_bin" --root "$_dir" project create beta \
        --title "Beta" >/dev/null
    for _p in alpha beta; do
        RDM_SESSION=harness-seed "$_bin" --root "$_dir" task create "seed-$_p" \
            --title "Seed $_p" --body "Body." --no-edit --project "$_p" >/dev/null
    done
    RDM_SESSION=harness-seed "$_bin" --root "$_dir" commit \
        -m "seed: init plan repo, two projects and their tasks" >/dev/null
}

# journal_paths <repo> [bin]: how many paths the shared changeset still claims.
#
# Reads through `rdm session journal`, which routes through the tombstone-aware
# fold — the same reader every consumer uses. Deliberately NOT a line count of
# the raw .jsonl: truncation is append-only, so raw lines outlive what they
# claim (see section 4).
journal_paths() {
    _bin=${2:-$RDM_BIN}
    RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$1" session journal --format json |
        tr ',' '\n' | sed -n 's/.*"path":"\([^"]*\)".*/\1/p' | sort
}

# await_parked <pid> <label>: bounded wait for a backgrounded process to reach
# the truncation barrier.
#
# There is no readiness file to poll, so this asserts the property that
# matters: after enough time to have finished, the process is still alive —
# which it can only be because the barrier parked it. A process that was NOT
# parked exits in well under a second, so this fails loudly rather than
# silently degrading into "the two never actually raced".
await_parked() {
    _pid=$1
    _label=$2
    _i=0
    while [ "$_i" -lt 40 ]; do
        sleep 0.1
        _i=$((_i + 1))
    done
    if ! kill -0 "$_pid" 2>/dev/null; then
        fail "$_label exited instead of parking at its harness barrier — \
the two processes never interleaved, so this section proves nothing"
    fi
}

# await_exit <pid> <label> <status-file>: bounded `wait`, writing the exit
# status to <status-file>.
#
# The status goes to a file rather than to stdout because `wait` only works on
# a child of the *invoking* shell, and a command substitution would run this in
# a subshell that owns no such child. The bound is on the release having
# actually taken effect: a barrier that never released would otherwise hang CI
# for its own 60 s ceiling per invocation.
await_exit() {
    _pid=$1
    _label=$2
    _statusfile=$3
    _i=0
    while kill -0 "$_pid" 2>/dev/null; do
        sleep 0.2
        _i=$((_i + 1))
        if [ "$_i" -gt 150 ]; then
            kill -9 "$_pid" 2>/dev/null || true
            fail "$_label did not exit within 30s of releasing the barrier"
        fi
    done
    set +e
    wait "$_pid"
    _st=$?
    set -e
    printf '%s' "$_st" >"$_statusfile"
}

# interleave <repo> <outdir> [bin]
#
# The deterministic scenario, factored out so sections 2 and 2b drive the
# IDENTICAL sequence and differ only in the binary under test:
#
#   A stages a task, then starts `rdm commit` and parks inside `truncate`
#   B runs a full `rdm task create` to completion — appending its own record
#     AND rewriting both INDEX.md rows, which are paths A is landing
#   A is released
#
# Both run under ONE session id, which is the whole point: at rung 3 every
# parallel subagent and MCP call shares one, so "the only appender is the same
# shell" is false.
interleave() {
    _repo=$1
    _out=$2
    _bin=${3:-$RDM_BIN}
    mkdir -p "$_out"

    RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$_repo" task create a-item \
        --title "A item" --body "Body." --no-edit --project alpha >/dev/null

    RDM_HARNESS_JOURNAL_BARRIER="$_out/go-a" RDM_SESSION="$SHARED_SESSION" \
        "$_bin" --root "$_repo" commit -m "A: land the staged item" \
        >"$_out/a.out" 2>"$_out/a.err" &
    _apid=$!

    await_parked "$_apid" "process A (rdm commit)"

    set +e
    RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$_repo" task create b-item \
        --title "B item" --body "Body." --no-edit --project beta \
        >"$_out/b.out" 2>"$_out/b.err"
    printf '%s' "$?" >"$_out/b.status"
    set -e

    : >"$_out/go-a"
    await_exit "$_apid" "process A (rdm commit)" "$_out/a.status"
}

# gc_interleave <repo> <outdir> [bin]
#
# The gc-versus-append scenario, factored out so section 5 and its mutant
# self-test drive the IDENTICAL sequence and differ only in the binary:
#
#   B stages a task and parks INSIDE its journal append at
#     RDM_HARNESS_APPEND_BARRIER — descriptor open, line not yet written
#   G runs `rdm session gc` as a DIFFERENT process under a DIFFERENT session
#     id. It TRIES to compact B's changeset because nothing tells it not to:
#     rung 1 never creates a lease, so `live_lease_ids` cannot name B's id,
#     and `current` only ever excludes G's own. What stops it is the journal
#     lock B holds shared while parked
#   B is released and completes its write
#
# TWO claims are staged BEFORE B starts. Non-empty so an unexcluded compaction
# would take its `rename` exit rather than its `remove_file` one — the sharper
# of the two, since the replaced journal still looks perfectly healthy
# afterwards while B's bytes sit in an unlinked inode. Two rather than one so
# the outcome is *observable*: compaction always folds to exactly one line, so
# two lines before and two after is the evidence that gc was excluded, while
# two-then-one (the mutant) is the evidence that it really rewrote the file.
gc_interleave() {
    _repo=$1
    _out=$2
    _bin=${3:-$RDM_BIN}
    mkdir -p "$_out"
    _journal="$_repo/.git/rdm/changesets/$SHARED_SESSION.jsonl"

    for _pre in pre-item pre-item-two; do
        RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$_repo" task create "$_pre" \
            --title "Pre $_pre" --body "Body." --no-edit --project alpha >/dev/null
    done

    RDM_HARNESS_APPEND_BARRIER="$_out/go-b" RDM_SESSION="$SHARED_SESSION" \
        "$_bin" --root "$_repo" task create b-item \
        --title "B item" --body "Body." --no-edit --project beta \
        >"$_out/b.out" 2>"$_out/b.err" &
    _bpid=$!

    await_parked "$_bpid" "process B (rdm task create)"

    # Line counts are the evidence of what compaction did: it always folds to
    # exactly one line, so two-then-two means the sweep was excluded and
    # two-then-one means it rewrote the file under the parked append.
    wc -l <"$_journal" >"$_out/lines.before" 2>/dev/null || : >"$_out/lines.before"
    RDM_SESSION="gc-runner" "$_bin" --root "$_repo" session gc \
        >"$_out/g.out" 2>"$_out/g.err" ||
        fail "session gc failed: $(cat "$_out/g.err")"
    wc -l <"$_journal" >"$_out/lines.after" 2>/dev/null || : >"$_out/lines.after"

    : >"$_out/go-b"
    await_exit "$_bpid" "process B (rdm task create)" "$_out/b.status"
}

SHARED_SESSION="shared-changeset"

# ---------------------------------------------------------------------------
# Section 1 — the AC's stress scenario
# ---------------------------------------------------------------------------
say "Section 1: 40 parallel mutations x 6 concurrent commits under one RDM_SESSION"

REPO_1="$TMP/repo-1"
seed_repo "$REPO_1"

N_MUTATIONS=40
M_COMMITS=6
PIDS=""

_i=1
while [ "$_i" -le "$N_MUTATIONS" ]; do
    if [ $((_i % 2)) -eq 0 ]; then _proj=alpha; else _proj=beta; fi
    RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" task create "stress-$_i" \
        --title "Stress $_i" --body "Body $_i." --no-edit --project "$_proj" \
        >"$TMP/s1-create-$_i.out" 2>&1 &
    PIDS="$PIDS $!"
    # Interleave the commits INTO the create stream rather than after it, so
    # each one really does run while other processes are appending.
    if [ $((_i % (N_MUTATIONS / M_COMMITS))) -eq 0 ]; then
        RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" commit \
            -m "stress: interleaved commit at $_i" \
            >"$TMP/s1-commit-$_i.out" 2>&1 &
        PIDS="$PIDS $!"
    fi
    _i=$((_i + 1))
done

# Bounded wait on the whole fan-out. CI runs this unattended, so a hung child
# must fail loudly rather than stall the job.
_waited=0
for _pid in $PIDS; do
    while kill -0 "$_pid" 2>/dev/null; do
        sleep 0.2
        _waited=$((_waited + 1))
        if [ "$_waited" -gt 900 ]; then
            kill -9 "$_pid" 2>/dev/null || true
            fail "the stress fan-out did not finish within ~180s"
        fi
    done
    wait "$_pid" 2>/dev/null || true
done
ok "every stress process exited"

# (a) all 40 tasks exist.
MISSING=0
_i=1
while [ "$_i" -le "$N_MUTATIONS" ]; do
    if [ $((_i % 2)) -eq 0 ]; then _proj=alpha; else _proj=beta; fi
    RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" task show "stress-$_i" \
        --project "$_proj" --no-body >/dev/null 2>&1 || MISSING=$((MISSING + 1))
    _i=$((_i + 1))
done
[ "$MISSING" -eq 0 ] || fail "$MISSING of $N_MUTATIONS stress tasks are missing"
ok "all $N_MUTATIONS tasks exist"

# The commits raced the creates, so some work is legitimately still staged
# when the fan-out ends. Land it with one final scoped commit — the point of
# the section is that NOTHING is stranded, not that the races happened to
# settle in a particular order.
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" commit \
    -m "stress: land whatever the fan-out left staged" >"$TMP/s1-final.out" 2>&1 ||
    fail "the settling commit failed: $(cat "$TMP/s1-final.out")"

# (b-i) No AUTHORED path may be left dirty. This is the lost-append symptom
#       stated exactly: a task file this changeset wrote but no longer claims
#       is a journal entry that was destroyed.
git -C "$REPO_1" status --porcelain >"$TMP/s1.status"
grep -v 'INDEX\.md$' "$TMP/s1.status" >"$TMP/s1.authored" || true
[ -s "$TMP/s1.authored" ] &&
    fail "authored files are dirty after the fan-out — journaled paths were \
lost, exactly the reported symptom:
$(cat "$TMP/s1.authored")"
ok "no authored file is left uncommitted"

# (b-ii) A derived index may still be dirty here, and that is a DIFFERENT
#        defect: `ops::mutate` regenerates INDEX.md from a disk snapshot taken
#        before the flush lock, so under a fan-out the last writer can persist
#        a staler index than the one already committed. It is filed separately
#        as task `index-regen-reads-before-flush-lock` per this phase's
#        Direction, and is disentangled here rather than hidden: one QUIESCENT
#        regeneration — no concurrency, so no stale snapshot — must be enough
#        to settle it. If a lost journal append were the cause, regenerating
#        would not help, because the paths would not be journaled at all.
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" index \
    >"$TMP/s1-index.out" 2>&1 ||
    fail "the quiescent index regeneration failed: $(cat "$TMP/s1-index.out")"
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" commit \
    -m "stress: land the regenerated indexes" >"$TMP/s1-index-commit.out" 2>&1 ||
    fail "committing the regenerated indexes failed: $(cat "$TMP/s1-index-commit.out")"
grep -qi 'another changeset' "$TMP/s1-index-commit.out" &&
    fail "the regenerated indexes are attributed to another changeset — the \
reported symptom is still present: $(cat "$TMP/s1-index-commit.out")"
ok "one quiescent regeneration settles the derived indexes"

# (b-iii) NOW the tree must be clean, with nothing whatsoever left over.
git -C "$REPO_1" status --porcelain >"$TMP/s1.status"
[ -s "$TMP/s1.status" ] &&
    fail "the working tree is still dirty after a quiescent regeneration, so \
the residue is not the separately-filed index race:
$(cat "$TMP/s1.status")"
ok "git status --porcelain is empty"

# (c) the journal claims nothing.
journal_paths "$REPO_1" >"$TMP/s1.journal"
[ -s "$TMP/s1.journal" ] &&
    fail "the changeset still claims paths after everything landed:
$(cat "$TMP/s1.journal")"
ok "the changeset journal claims no paths"

RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" session list \
    >"$TMP/s1.list" 2>&1
grep -q "$SHARED_SESSION" "$TMP/s1.list" &&
    fail "session list still reports the fully-committed changeset: $(cat "$TMP/s1.list")"
ok "session list reports no changeset with work outstanding"

# (d) a final commit has nothing to do, and nothing is unattributed.
set +e
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" commit \
    -m "stress: should be a no-op" >"$TMP/s1.noop" 2>&1
S1_NOOP_STATUS=$?
set -e
[ "$S1_NOOP_STATUS" = "0" ] ||
    fail "the no-op commit failed (exit $S1_NOOP_STATUS): $(cat "$TMP/s1.noop")"
grep -qi 'another changeset' "$TMP/s1.noop" &&
    fail "the reported symptom is still present — the indexes are attributed \
elsewhere: $(cat "$TMP/s1.noop")"
grep -qi 'not attributed to any changeset' "$TMP/s1.noop" &&
    fail "paths were left unattributed: $(cat "$TMP/s1.noop")"
ok "the final commit is a clean no-op with nothing unattributed"

# ---------------------------------------------------------------------------
# Section 2 — determinism, not luck
# ---------------------------------------------------------------------------
say "Section 2: a record appended while a commit is parked inside truncate survives"

REPO_2="$TMP/repo-2"
seed_repo "$REPO_2"
interleave "$REPO_2" "$TMP/out-2"

[ "$(cat "$TMP/out-2/b.status")" = "0" ] ||
    fail "process B (the concurrent mutation) failed: $(cat "$TMP/out-2/b.err")"
[ "$(cat "$TMP/out-2/a.status")" = "0" ] ||
    fail "process A (the parked commit) failed: $(cat "$TMP/out-2/a.err")"
ok "both processes exited 0"

journal_paths "$REPO_2" >"$TMP/s2.journal"

# B's own new file: a path the parked commit was NOT landing. The old
# whole-file rewrite destroyed this unconditionally.
grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s2.journal" ||
    fail "B's task is gone from the journal — the truncation destroyed a \
concurrent append. Journal now holds:
$(cat "$TMP/s2.journal")"
ok "B's own record survived the truncation"

# The sharp half. A staged its task under project `alpha`, so its changeset
# claims the top-level INDEX.md and lands it — that path IS in A's tombstone.
# B, mutating project `beta`, regenerates that same top-level INDEX.md with
# different bytes while A is parked. A path-keyed tombstone sweeps B's record
# here; a content-keyed one keeps it, because those bytes are not the bytes
# that landed. This is what produced the reported dirty INDEX.md files.
grep -q 'regenerated index file' "$TMP/out-2/a.out" ||
    fail "the parked commit did not land any index file, so this section is \
not exercising a concurrently-rewritten LANDED path: $(cat "$TMP/out-2/a.out")"
grep -q '^INDEX.md$' "$TMP/s2.journal" ||
    fail "INDEX.md is gone from the journal — B's rewrite of a path A landed \
was swept, so the tree is left dirty and unattributed. Journal now holds:
$(cat "$TMP/s2.journal")"
ok "B's rewrite of the index A landed survived its tombstone"

# `projects/alpha/INDEX.md` is the control: A landed it and B never touched
# it, so it must be GONE. Truncation still has to work.
grep -q '^projects/alpha/INDEX.md$' "$TMP/s2.journal" &&
    fail "a path A landed and nobody re-recorded is still journaled — \
truncation stopped working, so the changeset can re-commit it later"
ok "a landed path nobody re-recorded is correctly dropped"

# And the survivors are committable: the tree ends clean with nothing stranded.
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_2" commit \
    -m "B: land what the interleave left staged" >"$TMP/s2.commit" 2>&1 ||
    fail "the follow-up commit failed: $(cat "$TMP/s2.commit")"
grep -qi 'another changeset' "$TMP/s2.commit" &&
    fail "the follow-up commit reports paths belonging to another changeset: \
$(cat "$TMP/s2.commit")"
git -C "$REPO_2" status --porcelain >"$TMP/s2.status"
[ -s "$TMP/s2.status" ] &&
    fail "the tree is dirty after the interleave settled:
$(cat "$TMP/s2.status")"
ok "the survivors are committable and the tree ends clean"

# ---------------------------------------------------------------------------
# Section 2b — the mutant-binary self-test
# ---------------------------------------------------------------------------
say "Section 2b: with truncate reverted to read-modify-write, the loss reappears"

MUT="$TMP/mutant"
mkdir -p "$MUT"
(cd "$REPO_ROOT" && git archive HEAD) | tar -x -C "$MUT" 2>/dev/null ||
    fail "could not export a scratch source tree (is this a git checkout?)"

# The phase's own uncommitted work is what we are testing, so overlay the
# working-tree copies of the files that carry it.
for f in rdm-core/src/session/journal.rs rdm-core/src/session/mod.rs \
    rdm-core/src/session/lease.rs rdm-core/src/lock.rs rdm-core/src/lib.rs \
    rdm-core/src/error.rs rdm-core/src/paths.rs rdm-core/src/store/mod.rs \
    rdm-store-fs/src/lib.rs rdm-store-git/src/lib.rs rdm-store-git/src/commit.rs \
    rdm-cli/src/commands/session.rs rdm-server/src/problem.rs; do
    [ -f "$REPO_ROOT/$f" ] || fail "expected source file missing: $f"
    mkdir -p "$MUT/$(dirname "$f")"
    cp "$REPO_ROOT/$f" "$MUT/$f"
done

MUT_JOURNAL="$MUT/rdm-core/src/session/journal.rs"
grep -q '^pub fn truncate(' "$MUT_JOURNAL" ||
    fail "'truncate' is no longer a top-level fn — update this self-test to match"

# Replace `truncate`'s whole body with the pre-fix read-modify-write form,
# keeping the post-fix signature so the mutant still compiles under -D
# warnings. The body is delimited by the fn's opening line and the next line
# that is exactly `}` at column 0, which is how rustfmt renders every
# top-level item in this file.
# The barrier sits INSIDE the mutant's read -> write window, which is the
# window the fix deletes. In the shipped code there is no such window, so its
# barrier necessarily sits just before its single append; here it marks the
# moment the committer has read and is about to rewrite, which is exactly
# where a concurrent record used to be destroyed.
cat >"$MUT/mutant-body.txt" <<'MUTBODY'
    let landed_paths: Vec<String> = landed.iter().map(|e| e.path.clone()).collect();
    let remaining: Vec<JournalEntry> = read_journal(paths, id)?
        .into_iter()
        .filter(|e| !landed_paths.contains(&e.path))
        .collect();
    harness_barrier(HARNESS_JOURNAL_BARRIER);
    let path = changeset_path(paths, id);
    if remaining.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        };
    }
    let line = serde_json::to_string(&JournalLine { paths: remaining })
        .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    std::fs::write(&path, format!("{line}\n"))?;
    Ok(())
    // MUTATION
MUTBODY

awk -v bodyfile="$MUT/mutant-body.txt" '
    /^pub fn truncate\(/ && !done {
        print
        while ((getline line < bodyfile) > 0) print line
        close(bodyfile)
        skip = 1
        done = 1
        next
    }
    skip && /^}$/ { skip = 0; print; next }
    skip { next }
    { print }
' "$MUT_JOURNAL" >"$MUT_JOURNAL.new"
mv "$MUT_JOURNAL.new" "$MUT_JOURNAL"
grep -q '// MUTATION' "$MUT_JOURNAL" || fail "failed to plant the mutation"

# The second and third mutations, for sections 5b and 6b: the journal lock
# stripped from both sides. Each arm of `lock_journal`'s mode match reports the
# lock as taken without ever asking the kernel, so an append proceeds bare into
# whatever inode the journal path names right now and a compaction rewrites
# without waiting for anyone — exactly the pre-fix behaviour on both sides.
# Mutating the two match arms keeps every binding used, so the mutant still
# compiles clean.
grep -q '^            LockMode::Shared => file.try_lock_shared(),$' "$MUT_JOURNAL" ||
    fail "the append's lock is not the line this self-test mutates — update it"
sed 's|^            LockMode::Shared => file.try_lock_shared(),$|            LockMode::Shared => Ok(()), // MUTATION-APPEND|' \
    "$MUT_JOURNAL" >"$MUT_JOURNAL.new"
mv "$MUT_JOURNAL.new" "$MUT_JOURNAL"
grep -q '// MUTATION-APPEND' "$MUT_JOURNAL" ||
    fail "failed to plant the append mutation"

grep -q '^            LockMode::Exclusive => file.try_lock(),$' "$MUT_JOURNAL" ||
    fail "compaction's lock is not the line this self-test mutates — update it"
sed 's|^            LockMode::Exclusive => file.try_lock(),$|            LockMode::Exclusive => Ok(()), // MUTATION-COMPACT|' \
    "$MUT_JOURNAL" >"$MUT_JOURNAL.new"
mv "$MUT_JOURNAL.new" "$MUT_JOURNAL"
grep -q '// MUTATION-COMPACT' "$MUT_JOURNAL" ||
    fail "failed to plant the compaction mutation"

# The fourth mutation, for section 7b: `discard_changeset` reverted to the bare
# `remove_file` it used to be — the one writer in the journal module that
# ignored the append protocol outright. The barrier is planted where the fix
# puts it (between reading what the changeset claims and destroying it), so
# section 7's interleave drives both binaries identically.
grep -q '^pub fn discard_changeset(' "$MUT_JOURNAL" ||
    fail "'discard_changeset' is no longer a top-level fn — update this self-test"

cat >"$MUT/mutant-discard.txt" <<'MUTDISCARD'
    harness_barrier(HARNESS_JOURNAL_BARRIER);
    match std::fs::remove_file(changeset_path(paths, id)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::Io(e)),
    }
    // MUTATION-DISCARD
MUTDISCARD

awk -v bodyfile="$MUT/mutant-discard.txt" '
    /^pub fn discard_changeset\(/ && !done {
        print
        while ((getline line < bodyfile) > 0) print line
        close(bodyfile)
        skip = 1
        done = 1
        next
    }
    skip && /^}$/ { skip = 0; print; next }
    skip { next }
    { print }
' "$MUT_JOURNAL" >"$MUT_JOURNAL.new"
mv "$MUT_JOURNAL.new" "$MUT_JOURNAL"
grep -q '// MUTATION-DISCARD' "$MUT_JOURNAL" ||
    fail "failed to plant the discard mutation"

# The mutant must not also inherit the tombstone-aware fold's protection from
# the OTHER direction, but it legitimately keeps `read_journal` — the pre-fix
# code read the same way. Only the write half is reverted.
say "  building the mutant (scratch CARGO_TARGET_DIR; ~1-2 min)"
(cd "$MUT" && CARGO_TARGET_DIR="$MUT/target" cargo build -q -p rdm-cli --offline) ||
    fail "the mutant build failed — the self-test cannot run"
MUT_BIN="$MUT/target/debug/rdm"
[ -x "$MUT_BIN" ] || fail "the mutant binary was not produced at $MUT_BIN"

REPO_2B="$TMP/repo-2b"
seed_repo "$REPO_2B" "$MUT_BIN"
interleave "$REPO_2B" "$TMP/out-2b" "$MUT_BIN"

[ "$(cat "$TMP/out-2b/a.status")" = "0" ] ||
    fail "self-test is inconclusive: the mutant's process A failed for some \
other reason (exit $(cat "$TMP/out-2b/a.status")): $(cat "$TMP/out-2b/a.err")"

journal_paths "$REPO_2B" "$MUT_BIN" >"$TMP/s2b.journal"
if grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s2b.journal"; then
    fail "the planted mutation did NOT reproduce the loss of B's own record — \
section 2 may be passing for a reason other than append-only truncation. \
Mutant journal:
$(cat "$TMP/s2b.journal")"
fi
if grep -q '^INDEX.md$' "$TMP/s2b.journal"; then
    fail "the planted mutation did NOT reproduce the loss of B's index \
rewrite — section 2's sharp assertion may be vacuous. Mutant journal:
$(cat "$TMP/s2b.journal")"
fi
ok "with the rewrite restored, BOTH of B's records are silently lost — section 2's pass is caused by the fix"

git -C "$REPO_2B" status --porcelain >"$TMP/s2b.status"
[ -s "$TMP/s2b.status" ] ||
    fail "the mutant left a clean tree, so the reproduction is not the reported \
symptom — re-check the interleave"
ok "and the mutant leaves the reported dirty tree behind"

# ---------------------------------------------------------------------------
# Section 3 — assertion self-tests
# ---------------------------------------------------------------------------
say "Section 3: planted corruptions prove the assertions above are not vacuous"

# (i) A path that is NOT in the journal must not match the greps section 2
#     uses, or those greps would pass against anything.
if grep -q '^projects/beta/tasks/not-a-real-item.md$' "$TMP/s2.journal"; then
    fail "the journal grep matches a path that was never written — it is vacuous"
fi
ok "the journal grep does not match an invented path"

# (ii) A deliberately dirty tree must trip section 1's cleanliness check, or
#      that check would pass on any tree.
printf 'planted\n' >"$REPO_1/planted-dirt.md"
git -C "$REPO_1" status --porcelain >"$TMP/s3.status"
[ -s "$TMP/s3.status" ] ||
    fail "git status --porcelain reports nothing for a planted untracked file — \
section 1's cleanliness assertion is vacuous"
rm -f "$REPO_1/planted-dirt.md"
ok "the cleanliness assertion catches a planted dirty file"

# (iii) A journal that DOES claim a path must make section 1's emptiness check
#       fail, or "claims nothing" would be unfalsifiable.
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" task create planted-claim \
    --title "Planted" --body "Body." --no-edit --project alpha >/dev/null
journal_paths "$REPO_1" >"$TMP/s3.journal"
[ -s "$TMP/s3.journal" ] ||
    fail "a freshly staged task claims no paths — the journal reader is vacuous"
ok "the emptiness assertion catches a journal that still claims work"
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_1" commit \
    -m "cleanup: land the planted claim" >/dev/null 2>&1

# ---------------------------------------------------------------------------
# Section 4 — the semantic-empty contract
# ---------------------------------------------------------------------------
say "Section 4: 'empty' is the fold, not the file"

REPO_4="$TMP/repo-4"
seed_repo "$REPO_4"
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_4" task create solo \
    --title "Solo" --body "Body." --no-edit --project alpha >/dev/null
RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_4" commit \
    -m "land the solo task" >/dev/null

journal_paths "$REPO_4" >"$TMP/s4.journal"
[ -s "$TMP/s4.journal" ] &&
    fail "the changeset still claims paths after a clean commit:
$(cat "$TMP/s4.journal")"
ok "the fold reports zero entries"

# The file may still hold lines, and that is CORRECT: truncation is an append,
# so the tombstone is itself a line. Gating on file absence would make this
# harness intermittently red for a correct implementation, and would also go
# red the moment compaction legitimately loses its length compare-and-swap.
JOURNAL_FILE="$REPO_4/.git/rdm/changesets/$SHARED_SESSION.jsonl"
if [ -f "$JOURNAL_FILE" ]; then
    grep -q '"landed"' "$JOURNAL_FILE" ||
        fail "the journal file survives but holds no tombstone — truncation did \
not append one, so it is not append-only"
    ok "the surviving file holds a tombstone line, and the fold resolves it to empty"
else
    ok "the journal file was compacted away; the fold agrees it is empty"
fi

# `rdm session gc` is the quiescent sweep, and it must leave the fold's answer
# unchanged while removing the file.
RDM_SESSION=gc-runner "$RDM_BIN" --root "$REPO_4" session gc >"$TMP/s4.gc" 2>&1 ||
    fail "session gc failed: $(cat "$TMP/s4.gc")"
grep -qi 'changeset journal' "$TMP/s4.gc" ||
    fail "session gc does not report its changeset sweep: $(cat "$TMP/s4.gc")"
[ -f "$JOURNAL_FILE" ] &&
    fail "session gc left a fully-committed journal on disk: $JOURNAL_FILE"
ok "session gc sweeps the fully-committed journal at a quiescent moment"

journal_paths "$REPO_4" >"$TMP/s4b.journal"
[ -s "$TMP/s4b.journal" ] &&
    fail "the fold changed after gc — sweeping must be observationally inert"
ok "the fold's answer is unchanged by the sweep"

# ---------------------------------------------------------------------------
# Section 5 — the one non-append operation, run against an active appender
# ---------------------------------------------------------------------------
say "Section 5: a sweep arriving during an append is excluded, and the record lands"

# Why this is a section and not a footnote: `gc_changesets` decides a changeset
# is safe to rewrite when no live LEASE names it, and rungs 1 and 3 — an
# explicit RDM_SESSION and a harness-published id, the two rungs under which
# parallel subagents share a changeset at all — resolve without ever creating
# one. So the liveness check is blind exactly where this phase's whole
# reproduction lives, and gc invoked from an unrelated shell will try to
# compact a journal several processes are appending to. The journal lock is
# what stops it: the parked append holds it shared, so the sweep cannot take
# the exclusive hold it needs and skips.
REPO_5="$TMP/repo-5"
seed_repo "$REPO_5"
gc_interleave "$REPO_5" "$TMP/out-5"

[ "$(cat "$TMP/out-5/b.status")" = "0" ] ||
    fail "process B (the appender) failed: $(cat "$TMP/out-5/b.err")"
ok "the appender exited 0 across the sweep"

LINES_BEFORE=$(tr -d ' ' <"$TMP/out-5/lines.before")
LINES_AFTER=$(tr -d ' ' <"$TMP/out-5/lines.after")
[ "$LINES_BEFORE" = "2" ] ||
    fail "expected two journal lines for gc to try to collapse, got '$LINES_BEFORE' — \
the interleave is not set up the way this section assumes"
[ "$LINES_AFTER" = "2" ] ||
    fail "gc rewrote the journal under the parked append (lines after: \
'$LINES_AFTER'), so the append's shared lock did not exclude it"
ok "gc left the journal exactly as it found it: excluded by the parked append's lock"

journal_paths "$REPO_5" >"$TMP/s5.journal"
grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s5.journal" ||
    fail "B's record is gone — the compaction destroyed an append it could not \
see. Journal now holds:
$(cat "$TMP/s5.journal")"
ok "B's record landed once the append was released"

grep -q '^projects/alpha/tasks/pre-item-two.md$' "$TMP/s5.journal" ||
    fail "a claim staged before the sweep is gone. Journal now holds:
$(cat "$TMP/s5.journal")"
ok "and every claim staged before the sweep is still claimed"

RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_5" commit \
    -m "land what the gc interleave left staged" >"$TMP/s5.commit" 2>&1 ||
    fail "the follow-up commit failed: $(cat "$TMP/s5.commit")"
grep -qi 'another changeset' "$TMP/s5.commit" &&
    fail "the follow-up commit disowns paths after the sweep: $(cat "$TMP/s5.commit")"
git -C "$REPO_5" status --porcelain >"$TMP/s5.status"
[ -s "$TMP/s5.status" ] &&
    fail "the tree is dirty after the gc interleave settled:
$(cat "$TMP/s5.status")"
ok "everything the interleave left is committable and the tree ends clean"

# ---------------------------------------------------------------------------
# Section 5b — the mutant self-test for section 5
# ---------------------------------------------------------------------------
say "Section 5b: with the journal lock stripped, gc rewrites under the append and destroys the record"

REPO_5B="$TMP/repo-5b"
seed_repo "$REPO_5B" "$MUT_BIN"
gc_interleave "$REPO_5B" "$TMP/out-5b" "$MUT_BIN"

[ "$(cat "$TMP/out-5b/b.status")" = "0" ] ||
    fail "self-test is inconclusive: the mutant's appender failed for some \
other reason (exit $(cat "$TMP/out-5b/b.status")): $(cat "$TMP/out-5b/b.err")"
[ "$(tr -d ' ' <"$TMP/out-5b/lines.after")" = "1" ] ||
    fail "the mutant's gc never compacted, so this self-test is inconclusive"

journal_paths "$REPO_5B" "$MUT_BIN" >"$TMP/s5b.journal"
if grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s5b.journal"; then
    fail "the planted mutation did NOT reproduce the loss — section 5 may be \
passing for a reason other than the journal lock. Mutant journal:
$(cat "$TMP/s5b.journal")"
fi
ok "without the lock the record is silently lost — section 5's pass is caused by the fix"

git -C "$REPO_5B" status --porcelain >"$TMP/s5b.status"
[ -s "$TMP/s5b.status" ] ||
    fail "the mutant left a clean tree, so the loss is not observable — \
re-check the interleave"
ok "and the lost record leaves the reported dirty tree behind"

# ---------------------------------------------------------------------------
# Section 6 — the CAS-to-rename window, driven across real processes
# ---------------------------------------------------------------------------
say "Section 6: an append arriving while a sweep is mid-rewrite waits, then lands"

# What this section closes. Compaction checks the journal's byte length once,
# right after its fold, and nothing re-checks the journal immediately before
# the destructive `rename` — the earlier draft re-verified only its LOCK
# ownership there. An append landing in that window was written into the inode
# the rename was about to discard, reported success (the path still named that
# inode at the instant it checked), and was gone. Section 5's interleave cannot
# reach this: it drives the append's open-vs-write window, not compaction's
# CAS-vs-rename one.
#
# The fix makes the window unreachable rather than narrower: compaction holds
# the kernel-enforced journal lock exclusively from before its read until after
# its rename, and every append takes the same lock shared BEFORE it opens the
# journal. So an append arriving here waits for the sweep to finish and then
# opens the file the rename left behind.
#
# Everything here is two real processes. The parked `rdm session gc` is a real
# process holding the real lock inside its real critical section; the `rdm
# task create` is a real process making a real journal append, started while
# the sweep is parked and given a full second in which it could write blind.
window_interleave() {
    _repo=$1
    _out=$2
    _bin=${3:-$RDM_BIN}
    mkdir -p "$_out"
    _journal="$_repo/.git/rdm/changesets/$SHARED_SESSION.jsonl"

    # `rdm session gc` sweeps every unowned changeset in turn, so the parked
    # process below would otherwise park on whichever it reaches first — the
    # seeding changeset, whose journal is fully committed. Sweep that away
    # quiescently first, so the barrier is guaranteed to park the compaction of
    # the changeset this section is actually about.
    RDM_SESSION="gc-runner" "$_bin" --root "$_repo" session gc >/dev/null 2>&1 ||
        fail "the pre-sweep failed"
    _left=$(find "$_repo/.git/rdm/changesets" -name '*.jsonl' 2>/dev/null | wc -l)
    [ "$(echo "$_left" | tr -d ' ')" = "0" ] ||
        fail "the pre-sweep left $_left changeset journal(s) behind, so the \
parked compaction below may park on the wrong one"

    # Two claims, so compaction takes its `rename` exit (a non-empty fold) —
    # the sharper of the two, since a replaced journal looks healthy afterwards
    # while the lost bytes sit in an unlinked inode.
    for _pre in pre-item pre-item-two; do
        RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$_repo" task create "$_pre" \
            --title "Pre $_pre" --body "Body." --no-edit --project alpha >/dev/null
    done
    wc -l <"$_journal" >"$_out/lines.before" 2>/dev/null || : >"$_out/lines.before"

    RDM_HARNESS_COMPACT_BARRIER="$_out/go-g" RDM_SESSION="gc-runner" \
        "$_bin" --root "$_repo" session gc \
        >"$_out/g.out" 2>"$_out/g.err" &
    _gpid=$!

    await_parked "$_gpid" "process G (rdm session gc)"

    # A real append, arriving inside the parked sweep's critical section. Under
    # the shipped binary it blocks on the shared lock; under the mutant it
    # writes straight into the inode the sweep is about to rename over.
    RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$_repo" task create b-item \
        --title "B item" --body "Body." --no-edit --project beta \
        >"$_out/b.out" 2>"$_out/b.err" &
    _bpid=$!

    # A full second in which a blind append would have landed and returned.
    sleep 1

    : >"$_out/go-g"
    await_exit "$_gpid" "process G (rdm session gc)" "$_out/g.status"
    await_exit "$_bpid" "process B (rdm task create)" "$_out/b.status"
    wc -l <"$_journal" >"$_out/lines.after" 2>/dev/null || : >"$_out/lines.after"
}

REPO_6="$TMP/repo-6"
seed_repo "$REPO_6"
window_interleave "$REPO_6" "$TMP/out-6"

[ "$(cat "$TMP/out-6/b.status")" = "0" ] ||
    fail "the appender failed: $(cat "$TMP/out-6/b.err")"
[ "$(cat "$TMP/out-6/g.status")" = "0" ] ||
    fail "the parked gc failed: $(cat "$TMP/out-6/g.err")"
ok "both the parked sweep and the appender exited 0"

[ "$(tr -d ' ' <"$TMP/out-6/lines.before")" = "2" ] ||
    fail "expected two journal lines for the parked compaction to collapse, \
got '$(tr -d ' ' <"$TMP/out-6/lines.before")' — the interleave is not set up \
the way this section assumes"
[ "$(tr -d ' ' <"$TMP/out-6/lines.after")" = "2" ] ||
    fail "expected exactly the compacted line followed by the record appended \
after it, got '$(tr -d ' ' <"$TMP/out-6/lines.after")' line(s) — either the \
sweep never rewrote, or the append did not wait for it"
ok "the journal is the compacted line plus the record appended after the rename"

journal_paths "$REPO_6" >"$TMP/s6.journal"
grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s6.journal" ||
    fail "the record appended inside the sweep's critical section was renamed \
away. Journal now holds:
$(cat "$TMP/s6.journal")"
ok "the append waited the sweep out and its record survived"

grep -q '^projects/alpha/tasks/pre-item-two.md$' "$TMP/s6.journal" ||
    fail "the sweep dropped a claim it had already folded, which is a loss in \
the other direction. Journal now holds:
$(cat "$TMP/s6.journal")"
ok "and every claim the sweep had already folded is still claimed"

RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_6" commit \
    -m "land what the window interleave left staged" >"$TMP/s6.commit" 2>&1 ||
    fail "the follow-up commit failed: $(cat "$TMP/s6.commit")"
grep -qi 'another changeset' "$TMP/s6.commit" &&
    fail "the follow-up commit disowns paths after the interleave: $(cat "$TMP/s6.commit")"
git -C "$REPO_6" status --porcelain >"$TMP/s6.status"
[ -s "$TMP/s6.status" ] &&
    fail "the tree is dirty after the window interleave settled:
$(cat "$TMP/s6.status")"
ok "everything the interleave left is committable and the tree ends clean"

# ---------------------------------------------------------------------------
# Section 6b — the mutant self-test for section 6
# ---------------------------------------------------------------------------
say "Section 6b: with the journal lock stripped, the append lands in the doomed inode and is lost"

REPO_6B="$TMP/repo-6b"
seed_repo "$REPO_6B" "$MUT_BIN"
window_interleave "$REPO_6B" "$TMP/out-6b" "$MUT_BIN"

[ "$(cat "$TMP/out-6b/b.status")" = "0" ] ||
    fail "self-test is inconclusive: the mutant's appender failed for some \
other reason (exit $(cat "$TMP/out-6b/b.status")): $(cat "$TMP/out-6b/b.err")"
[ "$(tr -d ' ' <"$TMP/out-6b/lines.after")" = "1" ] ||
    fail "the mutant's sweep did not rewrite the journal over the blind append \
(lines after: '$(tr -d ' ' <"$TMP/out-6b/lines.after")'), so this self-test is \
inconclusive"

journal_paths "$REPO_6B" "$MUT_BIN" >"$TMP/s6b.journal"
if grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s6b.journal"; then
    fail "the planted mutation did NOT reproduce the loss — section 6 may be \
passing for a reason other than the journal lock. Mutant journal:
$(cat "$TMP/s6b.journal")"
fi
ok "without the lock the record is silently lost — section 6's pass is caused by the fix"

git -C "$REPO_6B" status --porcelain >"$TMP/s6b.status"
[ -s "$TMP/s6b.status" ] ||
    fail "the mutant left a clean tree, so the loss is not observable — \
re-check the interleave"
ok "and the lost record leaves the reported dirty tree behind"

# ---------------------------------------------------------------------------
# Section 7 — a deliberate destruction, run against an active appender
# ---------------------------------------------------------------------------
say "Section 7: a record appended while \`session discard\` runs survives it"

# What this section closes. Sections 2, 5 and 6 all cover a writer that MEANS
# to preserve the journal. `rdm session discard --force` means to destroy it,
# and used to do that with a bare `remove_file` — an unlink that takes a
# sibling's concurrent append with it exactly as an unguarded compaction would.
# Under one shared session id that sibling is an ordinary parallel subagent, so
# this is the same rung-3 shape as section 2 arriving from the other writer.
# The remedy is not a lock: it is for the discard to retire what it READ,
# through the same content-keyed tombstone every other writer appends.
discard_interleave() {
    _repo=$1
    _out=$2
    _bin=${3:-$RDM_BIN}
    mkdir -p "$_out"

    # Something for the discard to actually destroy, so a pass cannot come
    # from a no-op discard.
    RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$_repo" task create doomed-item \
        --title "Doomed item" --body "Body." --no-edit --project alpha >/dev/null

    RDM_HARNESS_JOURNAL_BARRIER="$_out/go-d" RDM_SESSION="discard-runner" \
        "$_bin" --root "$_repo" session discard "$SHARED_SESSION" --force \
        >"$_out/d.out" 2>"$_out/d.err" &
    _dpid=$!

    await_parked "$_dpid" "process D (rdm session discard)"

    set +e
    RDM_SESSION="$SHARED_SESSION" "$_bin" --root "$_repo" task create b-item \
        --title "B item" --body "Body." --no-edit --project beta \
        >"$_out/b.out" 2>"$_out/b.err"
    printf '%s' "$?" >"$_out/b.status"
    set -e

    : >"$_out/go-d"
    await_exit "$_dpid" "process D (rdm session discard)" "$_out/d.status"
}

REPO_7="$TMP/repo-7"
seed_repo "$REPO_7"
discard_interleave "$REPO_7" "$TMP/out-7"

[ "$(cat "$TMP/out-7/b.status")" = "0" ] ||
    fail "process B (the appender) failed: $(cat "$TMP/out-7/b.err")"
[ "$(cat "$TMP/out-7/d.status")" = "0" ] ||
    fail "process D (the discard) failed: $(cat "$TMP/out-7/d.err")"
ok "both the discard and the concurrent appender exited 0"

journal_paths "$REPO_7" >"$TMP/s7.journal"
grep -q '^projects/alpha/tasks/doomed-item.md$' "$TMP/s7.journal" &&
    fail "the discard did not retire what it read, so this section is asserting \
against a no-op. Journal now holds:
$(cat "$TMP/s7.journal")"
ok "the discard really did retire everything it read"

grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s7.journal" ||
    fail "B's record is gone — the discard destroyed an append it never read. \
Journal now holds:
$(cat "$TMP/s7.journal")"
ok "the concurrently appended record survived the discard"

RDM_SESSION="$SHARED_SESSION" "$RDM_BIN" --root "$REPO_7" commit \
    -m "land what survived the discard interleave" >"$TMP/s7.commit" 2>&1 ||
    fail "the follow-up commit failed: $(cat "$TMP/s7.commit")"
git -C "$REPO_7" ls-tree -r --name-only HEAD >"$TMP/s7.tree"
grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s7.tree" ||
    fail "B's file was never committed, so its surviving record bought nothing:
$(cat "$TMP/s7.commit")"
ok "and B's file is committed by its own session, not stranded as unattributed dirt"

# ---------------------------------------------------------------------------
# Section 7b — the mutant self-test for section 7
# ---------------------------------------------------------------------------
say "Section 7b: with the discard back to a bare remove_file, that record is lost"

REPO_7B="$TMP/repo-7b"
seed_repo "$REPO_7B" "$MUT_BIN"
discard_interleave "$REPO_7B" "$TMP/out-7b" "$MUT_BIN"

[ "$(cat "$TMP/out-7b/b.status")" = "0" ] ||
    fail "self-test is inconclusive: the mutant's appender failed for some \
other reason (exit $(cat "$TMP/out-7b/b.status")): $(cat "$TMP/out-7b/b.err")"

journal_paths "$REPO_7B" "$MUT_BIN" >"$TMP/s7b.journal"
if grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s7b.journal"; then
    fail "the planted mutation did NOT reproduce the loss — section 7 may be \
passing for a reason other than the discard's tombstone. Mutant journal:
$(cat "$TMP/s7b.journal")"
fi
ok "with the unlink restored the record is silently lost — section 7's pass is caused by the fix"

RDM_SESSION="$SHARED_SESSION" "$MUT_BIN" --root "$REPO_7B" commit \
    -m "try to land what the mutant left" >"$TMP/s7b.commit" 2>&1 || true
git -C "$REPO_7B" ls-tree -r --name-only HEAD >"$TMP/s7b.tree"
if grep -q '^projects/beta/tasks/b-item.md$' "$TMP/s7b.tree"; then
    fail "the mutant committed B's file anyway, so the loss is not observable — \
re-check the interleave"
fi
git -C "$REPO_7B" status --porcelain >"$TMP/s7b.status"
grep -q 'projects/beta/tasks/b-item.md' "$TMP/s7b.status" ||
    fail "the mutant left neither a commit nor a dirty file for B, so the \
reproduction is not the reported symptom:
$(cat "$TMP/s7b.status")"
ok "and B's file is left behind as unattributed dirt — the reported symptom"

printf '\n\033[1;32mAll sections passed.\033[0m\n'
