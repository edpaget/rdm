#!/bin/sh
# Hermetic regression for the lost-update mechanism:
# content-digest optimistic concurrency at the store flush boundary.
#
# Drives REAL separate `rdm` processes against temp plan repos to gate every
# acceptance criterion of
# `plan-repo-concurrency/phase-6-close-the-lost-update-window`, plus phase 9's
# delete-side half (`phase-9-content-checked-deletes`):
#
#   1   docs/lost-update-evaluation.md exists as a standalone record, and —
#       since phase 9 closed the deletes gap — no longer carries the deletes
#       carve-out but DOES name the delete guard, its error variant, and its
#       working-tree-at-commit-time basis
#   2   two real processes interleaved mid-flush: the loser is REFUSED, not
#       silently dropped   (2b repeats it with no session id at all)
#   2c  planted-mutation self-tests: with the check removed the lost update
#       reappears, and a corrupted expectation goes red
#   3   a session's own sequential writes never trip the check
#       (3b multiple flushes inside ONE process; 3c the section's self-test)
#   4   the commit-time half: a path another session overwrote is refused
#       rather than committed under this changeset's message
#   5   the `Done:` hook path stays exit-0 and logs the rejection
#   6   the commit-time DELETE half: a delayed `rdm commit` whose changeset
#       deletes a path another session has since recreated is refused, and
#       that session's file survives at HEAD   (6b the no-recreate self-test;
#       6c the mutant-binary self-test)
#
# An in-process test with two `Store` handles cannot gate any of this: the
# staging overlay is in-memory and discarded at process exit, which is exactly
# the layer that does not span invocations. Hence real processes throughout,
# interleaved at the documented `RDM_HARNESS_FLUSH_BARRIER` seam — except
# section 6, whose window (staging a delete, then committing it later) is
# naturally wide enough to drive with plain sequential invocations.
#
# Run after touching rdm-store-fs's baseline/flush machinery, the journal's
# `digest` field, `create_scoped_commit`'s content check, the delete guard in
# `build_changeset_tree`'s delete loop, or `rdm_core::paths::describe_path`.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo). No network.
# Every wait is bounded and fails loudly — CI runs this unattended.

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
# touches the developer's real plan repo — and so the identity rung in each
# section is unambiguous. A stray CLAUDE_CODE_SESSION_ID would silently move
# section 2b off the rung it exists to test.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_SESSION
unset CLAUDE_CODE_SESSION_ID CLAUDE_SESSION_ID RDM_HARNESS_SESSION_ID
unset RDM_HARNESS_FLUSH_BARRIER
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

# seed_repo <dir> [bin]: a plan repo with one project, one task tagged
# `alpha`, all committed.
#
# Pins RDM_SESSION so the seeding short-circuits at rung 1 before any lease is
# created. Load-bearing, not hygiene: a lease held by this script's own pid
# would be inherited by both driving shells below, and while the mechanism
# under test is content-keyed (so that would not change the outcome), leaving
# it ambiguous would make section 2b's claim unfalsifiable.
seed_repo() {
    _dir=$1
    _bin=${2:-$RDM_BIN}
    mkdir -p "$_dir"
    RDM_SESSION=harness-seed "$_bin" --root "$_dir" init --default-project demo >/dev/null
    RDM_SESSION=harness-seed "$_bin" --root "$_dir" task create fix-bug \
        --title "Fix bug" --body "Body." --tags alpha --no-edit --project demo >/dev/null
    RDM_SESSION=harness-seed "$_bin" --root "$_dir" commit \
        -m "seed: init plan repo, project and task" >/dev/null
}

# tags_of <repo> <slug> [bin]: the task's tags, as one comma-joined line.
tags_of() {
    _t=$("${3:-$RDM_BIN}" --root "$1" task show "$2" --project demo --no-body 2>/dev/null |
        grep -i '^Tags:' | head -1 || true)
    printf '%s' "$_t"
}

# await_parked <pid> <label>: bounded wait for a backgrounded process to reach
# the flush barrier.
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
    while [ "$_i" -lt 30 ]; do
        sleep 0.1
        _i=$((_i + 1))
    done
    if ! kill -0 "$_pid" 2>/dev/null; then
        fail "$_label exited instead of parking at RDM_HARNESS_FLUSH_BARRIER — \
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

# interleave <repo> <a-session-env> <b-session-env> <outdir> [bin]
#
# The core scenario, factored out so sections 2, 2b and 2c drive the IDENTICAL
# sequence and differ only in identity or in the binary under test:
#
#   A starts `task update --tags alpha,from-a` and parks at the flush barrier
#   B runs `task update --tags alpha,from-b` to completion
#   A is released
#
# Writes A's exit status to <outdir>/a.status and its stderr to <outdir>/a.err,
# and B's exit status to <outdir>/b.status.
interleave() {
    _repo=$1
    _a_env=$2
    _b_env=$3
    _out=$4
    _bin=${5:-$RDM_BIN}
    mkdir -p "$_out"

    # `env` with an empty first argument is not portable, so the session
    # variable is passed as a (possibly empty) prefix through `sh -c`.
    RDM_HARNESS_FLUSH_BARRIER="$_out/go-a" \
        sh -c "$_a_env exec '$_bin' --root '$_repo' task update fix-bug \
                --tags alpha,from-a --no-edit --project demo" \
        >"$_out/a.out" 2>"$_out/a.err" &
    _apid=$!

    await_parked "$_apid" "process A"

    set +e
    sh -c "$_b_env exec '$_bin' --root '$_repo' task update fix-bug \
            --tags alpha,from-b --no-edit --project demo" \
        >"$_out/b.out" 2>"$_out/b.err"
    printf '%s' "$?" >"$_out/b.status"
    set -e

    : >"$_out/go-a"
    await_exit "$_apid" "process A" "$_out/a.status"
}

# assert_loser_refused <outdir> <repo> [bin]: the three claims section 2 makes.
assert_loser_refused() {
    _out=$1
    _repo=$2
    _bin=${3:-$RDM_BIN}

    [ "$(cat "$_out/b.status")" = "0" ] ||
        fail "process B (the winner) must succeed; it exited $(cat "$_out/b.status")"

    [ "$(cat "$_out/a.status")" != "0" ] ||
        fail "process A wrote over B's change and exited 0 — the lost update is still open"

    grep -q 'task/fix-bug' "$_out/a.err" ||
        fail "A's error must name the item as task/fix-bug; got: $(cat "$_out/a.err")"

    grep -qi 'nothing was written' "$_out/a.err" ||
        fail "A's error must state that nothing was written; got: $(cat "$_out/a.err")"

    _tags=$(tags_of "$_repo" fix-bug "$_bin")
    case "$_tags" in
        *from-b*) : ;;
        *) fail "B's tag must survive; tags are: $_tags" ;;
    esac
    case "$_tags" in
        *from-a*) fail "A's refused write must not have landed; tags are: $_tags" ;;
        *) : ;;
    esac
}

# ---------------------------------------------------------------------------
# Section 1 — the evaluation record exists
# ---------------------------------------------------------------------------
say "Section 1: docs/lost-update-evaluation.md exists as a standalone record"

DOC="$REPO_ROOT/docs/lost-update-evaluation.md"

[ -f "$DOC" ] || fail "docs/lost-update-evaluation.md is missing"
[ -s "$DOC" ] || fail "docs/lost-update-evaluation.md is empty"
ok "the record exists and is non-empty"

# Headings only, never prose: the acceptance criterion is that the record
# exists and says what was chosen and why, not that it uses particular wording.
for heading in \
    '^## Verdict' \
    '^## Options considered' \
    '^## Selected mechanism' \
    '^## Carve-outs'; do
    grep -qE "$heading" "$DOC" ||
        fail "the record is missing a section matching: $heading"
done
ok "the record carries verdict / options / selected-mechanism / carve-outs sections"

# The do-nothing option must be recorded even though it was not taken — the
# phase permitted it as an outcome, so its rejection has to be on the record.
grep -qiE '\*\*\(A\) Do nothing\.\*\*|do nothing' "$DOC" ||
    fail "the record must state the do-nothing option and why it was not taken"
ok "the do-nothing option is recorded"

# Deletes WERE a named carve-out of phase 6's write-only guard. Phase 9 closed
# them, so this section now asserts the INVERSE of what it used to: the
# carve-out heading must be GONE, the record must name the guard's error
# variant, and the delete loop must no longer defer to phase 9.
DELETE_CARVEOUT='\*\*Journaled deletes are applied unconditionally\.\*\*'
if grep -qE "$DELETE_CARVEOUT" "$DOC"; then
    fail "the deletes carve-out is closed — **Journaled deletes are applied \
unconditionally.** must no longer appear in the record"
fi
DELETE_VARIANT='ChangesetDeletePathRecreated'
grep -q "$DELETE_VARIANT" "$DOC" ||
    fail "the record must name the delete guard's error variant ($DELETE_VARIANT)"
grep -qi 'working tree at commit time' "$DOC" ||
    fail "the record must state the delete guard's basis (the working tree at commit time, not HEAD)"
ok "the record replaces the carve-out with the guard, its variant, and its basis"

# The delete loop itself must no longer defer to phase 9 — it IS phase 9.
DELETE_LOOP_SRC="$REPO_ROOT/rdm-store-git/src/commit.rs"
if grep -q 'phase-9-content-checked-deletes' "$DELETE_LOOP_SRC"; then
    fail "the delete loop in rdm-store-git/src/commit.rs still defers to phase 9"
fi
grep -q "Error::$DELETE_VARIANT" "$DELETE_LOOP_SRC" ||
    fail "the delete loop in rdm-store-git/src/commit.rs must raise Error::$DELETE_VARIANT"
grep -q 'self.root.join(path).exists()' "$DELETE_LOOP_SRC" ||
    fail "the delete loop must be a working-tree presence check"
ok "the delete loop carries the guard as a working-tree presence check"

# The derived-path exemption WAS a named carve-out of this record too. Phase 4
# of `retire-generated-index` deleted the class outright, so — exactly as with
# the deletes carve-out above — this asserts the INVERSE of what it used to:
# the live heading must be gone, the record must name the phase that closed it,
# and no exempting call may survive in the delete loop.
DERIVED_CARVEOUT='^\*\*Derived indexes are exempt\.\*\*'
if grep -qE "$DERIVED_CARVEOUT" "$DOC"; then
    fail "the derived-index carve-out is closed — **Derived indexes are \
exempt.** must no longer stand as a live heading in the record"
fi
grep -q 'phase-4-collapse-derived-path-class' "$DOC" ||
    fail "the record must name the phase that closed the derived-index carve-out"
if grep -q 'is_derived_path' "$DELETE_LOOP_SRC"; then
    fail "the delete loop still calls is_derived_path, which no longer exists"
fi
ok "the derived-index carve-out is recorded as closed, with no exemption left in the loop"

# `ChangesetScope::digests`' rustdoc must no longer imply that a path absent
# from the map is committed unchecked: that fail-open answer is write-scoped
# now, and deletes are guarded by a different mechanism.
if grep -q 'store — is committed unchecked, which is the fail-open answer' "$DELETE_LOOP_SRC"; then
    fail "ChangesetScope::digests' rustdoc still makes the unqualified \
'is committed unchecked' claim, which is no longer true of deletes"
fi
grep -q 'Deletes are \*\*not\*\* committed' "$DELETE_LOOP_SRC" ||
    fail "ChangesetScope::digests' rustdoc must cross-reference the delete guard"
ok "ChangesetScope::digests' rustdoc is scoped to writes and names the delete guard"

# Self-test: each new grep must discriminate. Strip the variant name from a
# scratch copy of each file and prove the check observes its absence.
sed "s/$DELETE_VARIANT/SomeOtherVariant/g" "$DOC" >"$TMP/doc-mutant.md"
if grep -q "$DELETE_VARIANT" "$TMP/doc-mutant.md"; then
    fail "self-test setup failed: the variant name survived the planted mutation in the doc"
fi
grep -q "$DELETE_VARIANT" "$DOC" ||
    fail "self-test failed to leave the real record intact"

sed "s/$DELETE_VARIANT/SomeOtherVariant/g" "$DELETE_LOOP_SRC" >"$TMP/commit-mutant.rs"
if grep -q "Error::$DELETE_VARIANT" "$TMP/commit-mutant.rs"; then
    fail "self-test setup failed: the variant survived the planted mutation in commit.rs"
fi

# And the carve-out's absence must itself be falsifiable: a copy that DOES
# carry the heading has to be observed by the same predicate.
{
    cat "$DOC"
    printf '\n%s\n' '**Journaled deletes are applied unconditionally.** (planted)'
} >"$TMP/doc-carveout-mutant.md"
grep -qE "$DELETE_CARVEOUT" "$TMP/doc-carveout-mutant.md" ||
    fail "self-test: the carve-out predicate cannot see the heading it is supposed to forbid"

# Same, for the derived-index carve-out: a copy that DOES carry the live
# heading must be observed by the predicate that forbids it.
{
    cat "$DOC"
    printf '\n%s\n' '**Derived indexes are exempt.** (planted)'
} >"$TMP/doc-derived-mutant.md"
grep -qE "$DERIVED_CARVEOUT" "$TMP/doc-derived-mutant.md" ||
    fail "self-test: the derived carve-out predicate cannot see the heading it is supposed to forbid"
# ...and it must NOT fire on the real record, which names the closed carve-out
# only in past tense.
grep -qE "$DERIVED_CARVEOUT" "$DOC" &&
    fail "self-test: the derived carve-out predicate fires on the real record"

rm -f "$TMP/doc-mutant.md" "$TMP/commit-mutant.rs" "$TMP/doc-carveout-mutant.md" \
    "$TMP/doc-derived-mutant.md"
ok "self-test: each inverted check discriminates in both directions"

# Self-test: the section must go red when the record is absent.
mv "$DOC" "$TMP/doc-hidden.md"
if [ -f "$DOC" ]; then
    mv "$TMP/doc-hidden.md" "$DOC"
    fail "self-test setup failed: the record is still present after being moved"
fi
mv "$TMP/doc-hidden.md" "$DOC"
[ -f "$DOC" ] || fail "self-test failed to restore the record"
ok "self-test: the existence check observes the file's absence (and restores it)"

# ---------------------------------------------------------------------------
# Section 2 — two real processes, interleaved mid-flush
# ---------------------------------------------------------------------------
say "Section 2: the loser of a concurrent read-modify-write is REFUSED"

REPO_2="$TMP/repo-2"
seed_repo "$REPO_2"
interleave "$REPO_2" "RDM_SESSION=sess-a" "RDM_SESSION=sess-b" "$TMP/out-2"
assert_loser_refused "$TMP/out-2" "$REPO_2"
ok "A was refused with an actionable error naming task/fix-bug; B's tag survived"

# ---------------------------------------------------------------------------
# Section 2b — the same, with no session id anywhere
# ---------------------------------------------------------------------------
say "Section 2b: the same outcome with RDM_SESSION unset (content-keyed, not identity-keyed)"

REPO_2B="$TMP/repo-2b"
seed_repo "$REPO_2B"
interleave "$REPO_2B" "" "" "$TMP/out-2b"
assert_loser_refused "$TMP/out-2b" "$REPO_2B"
ok "identity resolution is irrelevant to the check — as designed"

# ---------------------------------------------------------------------------
# Section 2c — planted-mutation self-tests
# ---------------------------------------------------------------------------
say "Section 2c: planted mutations prove section 2 is not vacuous"

# (i) Remove the flush precondition from a scratch copy of the source, rebuild,
#     and re-run the SAME scenario. The lost update must reappear. This is the
#     only arm that proves the pass above is caused by the mechanism rather
#     than by the two processes never really racing.
MUT="$TMP/mutant"
mkdir -p "$MUT"
(cd "$REPO_ROOT" && git archive HEAD) | tar -x -C "$MUT" 2>/dev/null ||
    fail "could not export a scratch source tree (is this a git checkout?)"

# The phase's own uncommitted work is what we are testing, so overlay the
# working-tree copies of the files that carry it. Keep this list in step with
# the working tree: a file left out that a listed one depends on makes the
# mutant fail to BUILD, which reports as an inconclusive self-test rather than
# as the lost update the arm is looking for. `status.rs`, `ops/index.rs` and
# `rdm-mcp/src/server.rs` are here because they consume API this phase's
# `paths.rs` / `StatusReport` changes altered.
for f in rdm-store-fs/src/lib.rs rdm-core/src/store/mod.rs rdm-core/src/error.rs \
    rdm-core/src/paths.rs rdm-core/src/lock.rs rdm-core/src/lib.rs \
    rdm-core/src/session/journal.rs rdm-core/src/session/mod.rs \
    rdm-cli/src/commands/session.rs rdm-cli/src/commands/status.rs \
    rdm-core/src/ops/index.rs rdm-mcp/src/server.rs \
    rdm-store-git/src/lib.rs \
    rdm-store-git/src/commit.rs rdm-store-git/src/repo.rs \
    rdm-store-git/src/remote.rs rdm-server/src/problem.rs; do
    [ -f "$REPO_ROOT/$f" ] || fail "expected source file missing: $f"
    mkdir -p "$MUT/$(dirname "$f")"
    cp "$REPO_ROOT/$f" "$MUT/$f"
done

MUT_STORE="$MUT/rdm-store-fs/src/lib.rs"
# Neuter the comparison rather than deleting the call site: the repo builds
# with `-D warnings`, so an unreachable helper would fail to compile and the
# self-test would report a build failure instead of a lost update.
grep -q 'if &current != baseline {' "$MUT_STORE" ||
    fail "the flush precondition comparison moved — update this self-test to match"
# POSIX sed, in place via a temp file (no GNU -i).
sed 's|if &current != baseline {|if false \&\& \&current != baseline { // MUTATION|' \
    "$MUT_STORE" >"$MUT_STORE.new"
mv "$MUT_STORE.new" "$MUT_STORE"
grep -q '// MUTATION' "$MUT_STORE" ||
    fail "failed to plant the mutation"

say "  building the mutant (scratch CARGO_TARGET_DIR; ~15s)"
(cd "$MUT" && CARGO_TARGET_DIR="$MUT/target" cargo build -q -p rdm-cli --offline) ||
    fail "the mutant build failed — the self-test cannot run"
MUT_BIN="$MUT/target/debug/rdm"
[ -x "$MUT_BIN" ] || fail "the mutant binary was not produced at $MUT_BIN"

REPO_2C="$TMP/repo-2c"
seed_repo "$REPO_2C" "$MUT_BIN"
interleave "$REPO_2C" "RDM_SESSION=sess-a" "RDM_SESSION=sess-b" "$TMP/out-2c" "$MUT_BIN"

[ "$(cat "$TMP/out-2c/a.status")" = "0" ] ||
    fail "self-test is inconclusive: the mutant's process A failed for some \
other reason (exit $(cat "$TMP/out-2c/a.status")): $(cat "$TMP/out-2c/a.err")"
MUT_TAGS=$(tags_of "$REPO_2C" fix-bug "$MUT_BIN")
case "$MUT_TAGS" in
    *from-b*) fail "planted mutation did not reproduce the lost update — section 2 \
may be passing for a reason other than the precondition (tags: $MUT_TAGS)" ;;
    *from-a*) : ;;
    *) fail "unexpected mutant tag state: $MUT_TAGS" ;;
esac
ok "with the precondition removed, B's tag is silently lost — section 2's pass is caused by it"

# (ii) Corrupt the expectation itself: an item name that is NOT what the error
#      says must not match, or the grep in `assert_loser_refused` is vacuous.
if grep -q 'task/not-the-item' "$TMP/out-2/a.err"; then
    fail "the item-name assertion is vacuous: it matches an item that was never involved"
fi
ok "self-test: the item-name assertion discriminates (a wrong name does not match)"

# ---------------------------------------------------------------------------
# Section 3 — a session's own sequential writes never trip the check
# ---------------------------------------------------------------------------
say "Section 3: back-to-back invocations in one session all succeed"

# run_sequence <repo> <session-env>: eight real invocations against one item
# (plus a roadmap/phase pair), each a separate process, all in one session.
# The two successive `--tags` updates are the shape that matters: a genuine
# read-modify-write of a list this same session wrote moments earlier.
run_sequence() {
    _repo=$1
    _env=$2
    _i=0
    for cmd in \
        "task create seq-item --title Seq --body B --tags one --no-edit --project demo" \
        "task update seq-item --tags one,two --no-edit --project demo" \
        "task update seq-item --tags one,two,three --no-edit --project demo" \
        "task update seq-item --status in-progress --no-edit --project demo" \
        "task update seq-item --body Rewritten. --no-edit --project demo" \
        "roadmap create seq-map --title Map --body B --no-edit --project demo" \
        "phase create build --title Build --number 1 --body B --no-edit --roadmap seq-map --project demo" \
        "phase update 1 --status in-progress --no-edit --roadmap seq-map --project demo" \
        "commit -m sequential-batch"; do
        _i=$((_i + 1))
        set +e
        # shellcheck disable=SC2086 # the command words are intentionally split
        sh -c "$_env exec '$RDM_BIN' --root '$_repo' $cmd" >/dev/null 2>"$TMP/seq.err"
        _st=$?
        set -e
        [ "$_st" = "0" ] || fail "sequential invocation $_i ($cmd) exited $_st: $(cat "$TMP/seq.err")"
    done
}

REPO_3="$TMP/repo-3"
seed_repo "$REPO_3"
run_sequence "$REPO_3" "RDM_SESSION=solo"
case "$(tags_of "$REPO_3" seq-item)" in
    *one*two*three*) ok "nine sequential invocations under one session id all succeeded" ;;
    *) fail "the final state does not reflect the last write: $(tags_of "$REPO_3" seq-item)" ;;
esac

REPO_3N="$TMP/repo-3-nosession"
seed_repo "$REPO_3N"
run_sequence "$REPO_3N" ""
ok "the same sequence with no session id set also succeeded — not an artifact of rung 1"

# ---------------------------------------------------------------------------
# Section 3b — multiple flushes inside ONE process
# ---------------------------------------------------------------------------
say "Section 3b: one process flushing twice (a two-directive Done: batch)"

REPO_3B="$TMP/repo-3b"
seed_repo "$REPO_3B"
RDM_SESSION=hooky "$RDM_BIN" --root "$REPO_3B" task create second-item \
    --title Second --body B --no-edit --project demo >/dev/null
RDM_SESSION=hooky "$RDM_BIN" --root "$REPO_3B" commit -m "add second item" >/dev/null

# A real commit on the plan repo's default branch carrying two directives.
: >"$REPO_3B/trigger.txt"
git -C "$REPO_3B" add trigger.txt >/dev/null
git -C "$REPO_3B" commit -q -m "work

Done: task/fix-bug
Done: task/second-item
"
set +e
(cd "$REPO_3B" && RDM_SESSION=hooky "$RDM_BIN" --root "$REPO_3B" hook post-commit) >/dev/null 2>&1
HOOK_ST=$?
set -e
[ "$HOOK_ST" = "0" ] || fail "the two-directive hook exited $HOOK_ST"

for slug in fix-bug second-item; do
    RDM_SESSION=hooky "$RDM_BIN" --root "$REPO_3B" task show "$slug" \
        --project demo --no-body | grep -qi 'Status: *done' ||
        fail "the Done: directive for $slug did not apply"
done
ok "both directives applied from a single process that flushed more than once"

# ---------------------------------------------------------------------------
# Section 3c — self-test proving section 3 can fail
# ---------------------------------------------------------------------------
say "Section 3c: section 3's assertion is not vacuous"

# Section 3's whole content is "every invocation exited 0", which passes
# vacuously if the harness cannot observe a nonzero exit in the first place.
# So: plant a genuinely failing invocation of the SAME shape (same binary,
# same repo, same session, same subcommand) and confirm the identical
# predicate catches it.
set +e
RDM_SESSION=solo "$RDM_BIN" --root "$REPO_3" task update no-such-item \
    --tags one --no-edit --project demo >/dev/null 2>&1
BAD_ST=$?
set -e
[ "$BAD_ST" != "0" ] ||
    fail "self-test: a doomed invocation exited 0 — section 3's exit-status \
predicate cannot distinguish success from failure, so its pass means nothing"
ok "self-test: the exit-status predicate catches a planted failure"

# And the state assertion must discriminate too: the tags it read are not just
# any string that happens to be present.
case "$(tags_of "$REPO_3" seq-item)" in
    *never-written-tag*)
        fail "self-test: the tag assertion matches a tag that was never written"
        ;;
    *) ok "self-test: the final-state assertion discriminates" ;;
esac

# ---------------------------------------------------------------------------
# Section 4 — the commit-time half
# ---------------------------------------------------------------------------
say "Section 4: committing a path another session overwrote is refused"

REPO_4="$TMP/repo-4"
seed_repo "$REPO_4"

# A flushes its edit but does not commit.
RDM_SESSION=cs-a "$RDM_BIN" --root "$REPO_4" task update fix-bug \
    --tags alpha,owned-by-a --no-edit --project demo >/dev/null

# B reads the CURRENT content (so B is legitimately allowed) and flushes over it.
RDM_SESSION=cs-b "$RDM_BIN" --root "$REPO_4" task update fix-bug \
    --tags alpha,owned-by-b --no-edit --project demo >/dev/null

# A now commits. Its journal still claims the path, but the bytes are B's.
set +e
RDM_SESSION=cs-a "$RDM_BIN" --root "$REPO_4" commit -m "A's message" \
    >"$TMP/c4.out" 2>"$TMP/c4.err"
C4_ST=$?
set -e

[ "$C4_ST" != "0" ] ||
    fail "A committed B's bytes under A's message; output: $(cat "$TMP/c4.out")"
grep -q 'task/fix-bug' "$TMP/c4.err" ||
    fail "the commit-time refusal must name the item; got: $(cat "$TMP/c4.err")"
grep -qi 'another session overwrote it' "$TMP/c4.err" ||
    fail "the commit-time refusal must say why; got: $(cat "$TMP/c4.err")"

# And it must be a refusal, not a partial landing.
git -C "$REPO_4" log -1 --pretty=%s | grep -q "A's message" &&
    fail "a refused commit must not have landed"
ok "the scoped commit refused rather than landing another session's bytes"

# Self-test: without the overwrite, the same commit must succeed — so the
# refusal above is caused by the overwrite, not by the commit path being broken.
REPO_4B="$TMP/repo-4b"
seed_repo "$REPO_4B"
RDM_SESSION=cs-a "$RDM_BIN" --root "$REPO_4B" task update fix-bug \
    --tags alpha,owned-by-a --no-edit --project demo >/dev/null
RDM_SESSION=cs-a "$RDM_BIN" --root "$REPO_4B" commit -m "A's message" >/dev/null ||
    fail "self-test: an un-overwritten changeset must still commit cleanly"
ok "self-test: the same commit succeeds when nothing overwrote the path"

# ---------------------------------------------------------------------------
# Section 5 — the Done: hook path stays exit-0
# ---------------------------------------------------------------------------
say "Section 5: a rejection on the hook path is logged, never fatal"

REPO_5="$TMP/repo-5"
seed_repo "$REPO_5"
: >"$REPO_5/trigger.txt"
git -C "$REPO_5" add trigger.txt >/dev/null
git -C "$REPO_5" commit -q -m "work

Done: task/fix-bug
"

# Park the hook at its flush, overwrite the target from another session, then
# release. The hook must still exit 0: it runs under `hook_timeout_secs` and
# blocking or failing a `git commit` over a plan-repo conflict is worse than
# the conflict.
mkdir -p "$TMP/out-5"
(
    cd "$REPO_5" &&
        RDM_SESSION=hook-a RDM_HARNESS_FLUSH_BARRIER="$TMP/out-5/go" \
            "$RDM_BIN" --root "$REPO_5" hook post-commit
) >"$TMP/out-5/hook.out" 2>&1 &
HPID=$!

await_parked "$HPID" "the post-commit hook"

RDM_SESSION=other "$RDM_BIN" --root "$REPO_5" task update fix-bug \
    --tags alpha,from-other --no-edit --project demo >/dev/null

: >"$TMP/out-5/go"
await_exit "$HPID" "the post-commit hook" "$TMP/out-5/hook.status"
HOOK5_ST=$(cat "$TMP/out-5/hook.status")

[ "$HOOK5_ST" = "0" ] ||
    fail "the hook exited $HOOK5_ST — a plan-repo conflict must never fail the invoking git commit"
ok "the hook exited 0 despite the conflict"

HOOK_LOG="$REPO_5/.git/rdm-hook.log"
[ -f "$HOOK_LOG" ] || fail "the hook wrote no log at $HOOK_LOG"
grep -q 'error' "$HOOK_LOG" ||
    fail "the hook must LOG the rejection rather than swallowing it silently: $(cat "$HOOK_LOG")"
ok "the rejection is recorded in the hook log"

# ---------------------------------------------------------------------------
# Section 6 — the commit-time DELETE half
# ---------------------------------------------------------------------------
say "Section 6: committing a delete of a path another session recreated is refused"

# scenario_6 <repo> <bin> <outdir> <recreate: yes|no>
#
# Drives the delete window with plain sequential invocations — no flush
# barrier. The window this targets is not the sub-millisecond read→write gap
# inside one command; it is the arbitrarily wide gap rdm's own workflow
# prescribes between staging a mutation and running `rdm commit`.
#
#   A  `rdm promote fix-bug` — this is the CLI-drivable delete: promote_task
#      ends in `store.delete(&task_path)`. A does NOT commit.
#   B  creates a task at the same slug and lands its own commit first.
#   A  runs its delayed `rdm commit`.
#
# Writes A's exit status to <outdir>/a.status, stdout/stderr to a.out/a.err.
scenario_6() {
    _repo=$1
    _bin=$2
    _out=$3
    _recreate=$4
    mkdir -p "$_out"
    seed_repo "$_repo" "$_bin"

    RDM_SESSION=del-a "$_bin" --root "$_repo" promote fix-bug \
        --roadmap-slug promoted --project demo >/dev/null 2>&1 ||
        fail "A's promote must succeed (it is what stages the delete)"

    # Guard against a vacuous run: if promote ever stopped journaling a
    # DELETE of the task path, this whole section would pass with the guard
    # never consulted.
    RDM_SESSION=del-a "$_bin" --root "$_repo" session journal \
        >"$_out/journal.txt" 2>&1 ||
        fail "could not read A's changeset journal"
    grep -qE '^[[:space:]]*delete[[:space:]]+projects/demo/tasks/fix-bug\.md$' \
        "$_out/journal.txt" ||
        fail "A's changeset must journal a DELETE of projects/demo/tasks/fix-bug.md, \
or this section proves nothing; journal: $(cat "$_out/journal.txt")"

    if [ "$_recreate" = "yes" ]; then
        RDM_SESSION=del-b "$_bin" --root "$_repo" task create fix-bug \
            --title "B's task" --body "B's bytes." --no-edit --project demo >/dev/null 2>&1 ||
            fail "B must be able to create a task at the slug A emptied"
        RDM_SESSION=del-b "$_bin" --root "$_repo" commit -m "B's message" >/dev/null 2>&1 ||
            fail "B must land its own commit first"
    fi

    set +e
    RDM_SESSION=del-a "$_bin" --root "$_repo" commit -m "A's message" \
        >"$_out/a.out" 2>"$_out/a.err"
    printf '%s' "$?" >"$_out/a.status"
    set -e
}

REPO_6="$TMP/repo-6"
scenario_6 "$REPO_6" "$RDM_BIN" "$TMP/out-6" yes

[ "$(cat "$TMP/out-6/a.status")" != "0" ] ||
    fail "A's delayed commit landed its stale delete over B's file; output: $(cat "$TMP/out-6/a.out")"
grep -q 'task/fix-bug' "$TMP/out-6/a.err" ||
    fail "the delete refusal must name the item; got: $(cat "$TMP/out-6/a.err")"
grep -qi 'recreated' "$TMP/out-6/a.err" ||
    fail "the delete refusal must say the path was recreated; got: $(cat "$TMP/out-6/a.err")"
grep -qi 'nothing was committed' "$TMP/out-6/a.err" ||
    fail "the delete refusal must state that nothing landed; got: $(cat "$TMP/out-6/a.err")"

# The claim that actually matters: B's work is still there.
git -C "$REPO_6" show HEAD:projects/demo/tasks/fix-bug.md 2>/dev/null |
    grep -q "B's bytes" ||
    fail "B's file must survive A's delayed commit at HEAD"
if git -C "$REPO_6" log -1 --pretty=%s | grep -q "A's message"; then
    fail "a refused commit must not have landed"
fi
ok "A was refused by name, nothing landed, and B's file is intact at HEAD"

# Self-test 6b: the SAME sequence without B's recreate must commit cleanly and
# genuinely remove the path — so the refusal above is caused by the recreate,
# not by the delete path being broken.
say "Section 6b: without the recreate, the same delayed commit succeeds"
REPO_6B="$TMP/repo-6b"
scenario_6 "$REPO_6B" "$RDM_BIN" "$TMP/out-6b" no
[ "$(cat "$TMP/out-6b/a.status")" = "0" ] ||
    fail "self-test: an un-recreated delete must still commit cleanly; \
exit $(cat "$TMP/out-6b/a.status"): $(cat "$TMP/out-6b/a.err")"
if git -C "$REPO_6B" show HEAD:projects/demo/tasks/fix-bug.md >/dev/null 2>&1; then
    fail "self-test: the delete must really have removed the path from the landed tree"
fi
ok "self-test: the delete lands and removes the path when nobody recreated it"

# Self-test 6c: neuter the guard in a scratch source tree, rebuild, and re-run
# the scenario. The lost update must reappear — this is the only arm proving
# section 6's pass is caused by the guard rather than by the window never
# opening.
say "Section 6c: a planted mutation reproduces the lost update"

MUT2="$TMP/mutant-delete"
mkdir -p "$MUT2"
(cd "$REPO_ROOT" && git archive HEAD) | tar -x -C "$MUT2" 2>/dev/null ||
    fail "could not export a scratch source tree (is this a git checkout?)"
for f in rdm-store-fs/src/lib.rs rdm-core/src/store/mod.rs rdm-core/src/error.rs \
    rdm-core/src/paths.rs rdm-core/src/lock.rs rdm-core/src/lib.rs \
    rdm-core/src/session/journal.rs rdm-core/src/session/mod.rs \
    rdm-cli/src/commands/session.rs rdm-cli/src/commands/status.rs \
    rdm-core/src/ops/index.rs rdm-mcp/src/server.rs \
    rdm-store-git/src/lib.rs \
    rdm-store-git/src/commit.rs rdm-store-git/src/repo.rs \
    rdm-store-git/src/remote.rs rdm-server/src/problem.rs; do
    [ -f "$REPO_ROOT/$f" ] || fail "expected source file missing: $f"
    mkdir -p "$MUT2/$(dirname "$f")"
    cp "$REPO_ROOT/$f" "$MUT2/$f"
done

MUT2_COMMIT="$MUT2/rdm-store-git/src/commit.rs"
grep -q 'self.root.join(path).exists()' "$MUT2_COMMIT" ||
    fail "the delete guard's presence check moved — update this self-test to match"
# Short-circuit the presence test rather than deleting the branch: removing the
# `return Err(...)` outright would leave an unconstructed variant and the
# self-test would report a build failure instead of a lost update. POSIX sed,
# in place via a temp file (no GNU -i).
sed 's|self.root.join(path).exists()|false /* MUTATION */|' \
    "$MUT2_COMMIT" >"$MUT2_COMMIT.new"
mv "$MUT2_COMMIT.new" "$MUT2_COMMIT"
grep -q 'MUTATION' "$MUT2_COMMIT" || fail "failed to plant the delete-guard mutation"

say "  building the mutant (scratch CARGO_TARGET_DIR; ~15s)"
(cd "$MUT2" && CARGO_TARGET_DIR="$MUT2/target" cargo build -q -p rdm-cli --offline) ||
    fail "the mutant build failed — the self-test cannot run"
MUT2_BIN="$MUT2/target/debug/rdm"
[ -x "$MUT2_BIN" ] || fail "the mutant binary was not produced at $MUT2_BIN"

REPO_6C="$TMP/repo-6c"
scenario_6 "$REPO_6C" "$MUT2_BIN" "$TMP/out-6c" yes
[ "$(cat "$TMP/out-6c/a.status")" = "0" ] ||
    fail "self-test is inconclusive: the mutant's A failed for some other \
reason (exit $(cat "$TMP/out-6c/a.status")): $(cat "$TMP/out-6c/a.err")"
if git -C "$REPO_6C" show HEAD:projects/demo/tasks/fix-bug.md >/dev/null 2>&1; then
    fail "planted mutation did not reproduce the lost update — section 6 may be \
passing for a reason other than the delete guard"
fi
ok "with the guard removed, B's file is silently destroyed — section 6's pass is caused by it"

# ---------------------------------------------------------------------------
say "All sections passed."
