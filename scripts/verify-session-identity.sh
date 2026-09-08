#!/bin/sh
# Hermetic regression for session identity and the changeset journal.
#
# Drives REAL separate processes (not in-process fakes) against temp plan
# repos to gate: cross-process identity stability within one session and
# distinctness between two, the rung-1/rung-3 precedence rules, stale-lease
# rejection on a recycled pid, journal exactness and disjointness, the
# invisibility of session state to `rdm status` and to a whole-tree commit
# (with a planted-decoy self-test), the measured resolution cost including a
# real `rdm hook post-commit` run, a structural grep proving
# rdm-store-git/src/commit.rs carries the changeset commit scope while
# resolving no session identity of its own (§ I; inverted in phase 5, which
# introduced exactly the coupling it used to forbid), and (§ J, phase 10) that
# a harness-published session id ALWAYS wins over an already-inherited
# ancestor lease: two real child processes of one already-leased ancestor,
# each carrying its own distinct CLAUDE_CODE_SESSION_ID, resolve distinct
# rung-3 ids that diverge from the planted ancestor lease, each stable across
# repeat invocations within the same process — plus a Section J self-test
# that neuters the harness check, rebuilds a mutant binary in a scratch
# CARGO_TARGET_DIR, and confirms the SAME scenario reproduces the phase-10
# merging bug on it (the rebuild-a-mutant pattern also used by
# verify-lost-update.sh's Section 2c).
#
# Run after touching rdm-core/src/session/**, GitStore's journal wiring,
# FsStore::staged_paths, or the `rdm session` CLI surface.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo). No network.

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
# touches the developer's real plan repo — and, critically, so the rung under
# test in each section is unambiguous. A stray CLAUDE_CODE_SESSION_ID would
# silently move sections A/B from rung 2 to rung 3.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_SESSION
unset CLAUDE_CODE_SESSION_ID CLAUDE_SESSION_ID RDM_HARNESS_SESSION_ID
# § K's wrapper payload variable. Cleared here so an inherited value can never
# be what a wrapper evals.
unset RDM_K_CMD
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

# seed_repo <dir>: stand up a plan repo with one project and an initial commit.
#
# Every seeding command pins RDM_SESSION, which short-circuits at rung 1 BEFORE
# any lease is created. That is load-bearing, not hygiene: if the harness
# script's own pid held a lease in the repo under test, the two driving shells
# in section A would both ascend to it and adopt ONE id — merging the very
# sessions the section exists to prove distinct.
seed_repo() {
    _dir=$1
    mkdir -p "$_dir"
    # `--default-project` is load-bearing for section H2: `rdm hook
    # post-commit` resolves its project through the standard chain, and with no
    # default it aborts before ever applying a `Done:` directive.
    RDM_SESSION=harness-seed "$RDM_BIN" --root "$_dir" init --default-project demo >/dev/null
    RDM_SESSION=harness-seed "$RDM_BIN" --root "$_dir" commit \
        -m "seed: init plan repo and project" >/dev/null
}

# json_lines <file>: keep only the one-object-per-invocation JSON lines,
# discarding any advisory chatter.
json_lines() { grep '^{' "$1" || true; }

# field_str <key> — reads stdin, prints each line's string value for <key>.
field_str() { sed -n 's/.*"'"$1"'":"\([^"]*\)".*/\1/p'; }

# field_num <key> — reads stdin, prints each line's numeric value for <key>.
field_num() { sed -n 's/.*"'"$1"'":\([0-9][0-9]*\).*/\1/p'; }

# paths_of <json-file>: the sorted set of "path" values in a journal payload.
paths_of() { tr ',' '\n' <"$1" | sed -n 's/.*"path":"\([^"]*\)".*/\1/p' | sort; }

# all_same <file>: true when every line is identical and there is at least one.
all_same() { [ "$(sort -u "$1" | wc -l | tr -d ' ')" = "1" ] && [ -s "$1" ]; }

# A driving shell script: N `rdm session id --format json` invocations from one
# long-lived parent. The trailing `:` keeps the shell from exec'ing the last
# command into itself, which would make the final rdm a *sibling* of its
# predecessors rather than a child of the same parent.
write_sequence() {
    _out=$1
    _repo=$2
    _n=$3
    {
        echo '#!/bin/sh'
        _i=0
        while [ "$_i" -lt "$_n" ]; do
            echo "\"$RDM_BIN\" --root \"$_repo\" session id --format json"
            _i=$((_i + 1))
        done
        echo ':'
    } >"$_out"
}

# mutant_tree <dir>: export HEAD into <dir>, then overlay the WORKING-TREE copy
# of every source file this harness's self-tests depend on.
#
# The overlay is what lets a self-test run against work that is not committed
# yet. It must stay a whole-set copy rather than a per-mutation list: a mutant
# built from HEAD plus one overlaid file fails to COMPILE the moment any other
# uncommitted file in the set changed with it (e.g. a new struct field and its
# initializer), and a self-test that cannot build reports a harness failure
# instead of the regression it exists to detect.
MUTANT_OVERLAY="rdm-core/src/session/mod.rs
rdm-core/src/session/lease.rs
rdm-core/src/session/journal.rs
rdm-core/src/session/process.rs
rdm-cli/src/commands/commit.rs
rdm-cli/src/commands/session.rs
rdm-store-git/src/lib.rs
rdm-store-git/src/commit.rs"

# One scratch target dir shared by every mutant build. Each mutant is a
# distinct source tree, so its own crates rebuild, but the third-party
# dependency graph is identical across them and cargo builds it once — which is
# most of a cold build. Kept out of the repo's own target/ so a mutant can never
# leave a doctored artifact behind for a later `cargo` run to pick up.
MUTANT_TARGET_DIR="$TMP/mutant-target"

mutant_tree() {
    _dst=$1
    mkdir -p "$_dst"
    (cd "$REPO_ROOT" && git archive HEAD) | tar -x -C "$_dst" 2>/dev/null ||
        fail "could not export a scratch source tree (is this a git checkout?)"
    echo "$MUTANT_OVERLAY" | while IFS= read -r _f; do
        [ -n "$_f" ] || continue
        [ -f "$REPO_ROOT/$_f" ] || fail "mutant overlay names a missing file: $_f"
        mkdir -p "$_dst/$(dirname "$_f")"
        cp "$REPO_ROOT/$_f" "$_dst/$_f"
    done
}

# ---------------------------------------------------------------------------
# Section A — two concurrent sequences: stable within, distinct between
# ---------------------------------------------------------------------------
say "Section A: cross-process stability and inter-session distinctness"

REPO_A="$TMP/repo-a"
seed_repo "$REPO_A"
write_sequence "$TMP/seq-a.sh" "$REPO_A" 3
write_sequence "$TMP/seq-b.sh" "$REPO_A" 3

sh "$TMP/seq-a.sh" >"$TMP/out_a" 2>"$TMP/err_a" &
PID_A=$!
sh "$TMP/seq-b.sh" >"$TMP/out_b" 2>"$TMP/err_b" &
PID_B=$!
wait "$PID_A" || fail "sequence A exited non-zero: $(cat "$TMP/err_a")"
wait "$PID_B" || fail "sequence B exited non-zero: $(cat "$TMP/err_b")"

json_lines "$TMP/out_a" | field_str id >"$TMP/ids_a"
json_lines "$TMP/out_b" | field_str id >"$TMP/ids_b"

[ "$(wc -l <"$TMP/ids_a" | tr -d ' ')" = "3" ] ||
    fail "expected 3 ids from sequence A, got: $(cat "$TMP/ids_a")"
[ "$(wc -l <"$TMP/ids_b" | tr -d ' ')" = "3" ] ||
    fail "expected 3 ids from sequence B, got: $(cat "$TMP/ids_b")"
all_same "$TMP/ids_a" || fail "sequence A's ids differ across processes: $(cat "$TMP/ids_a")"
all_same "$TMP/ids_b" || fail "sequence B's ids differ across processes: $(cat "$TMP/ids_b")"
ok "each sequence resolves ONE id across three separate processes"

ID_A=$(head -n 1 "$TMP/ids_a")
ID_B=$(head -n 1 "$TMP/ids_b")
[ -n "$ID_A" ] || fail "sequence A produced an empty id"
[ "$ID_A" != "$ID_B" ] ||
    fail "two concurrent sequences MERGED onto one id ($ID_A) — this is the roadmap's bug"
ok "two concurrent sequences resolve distinct ids ($ID_A vs $ID_B)"

# ---------------------------------------------------------------------------
# Section B — same assertion with nothing set, plus the rung and the leases
# ---------------------------------------------------------------------------
say "Section B: bare environment, rung 2, and the on-disk lease consequence"

REPO_B="$TMP/repo-b"
seed_repo "$REPO_B"
write_sequence "$TMP/seq-c.sh" "$REPO_B" 3
write_sequence "$TMP/seq-d.sh" "$REPO_B" 3

# Nothing is set by the caller: the four session variables were unset at the
# top of this script and nothing below re-exports them, so the ONLY mechanism
# that can produce stability here is the inherited ancestor lease.
for _v in RDM_SESSION CLAUDE_CODE_SESSION_ID CLAUDE_SESSION_ID RDM_HARNESS_SESSION_ID; do
    eval "_set=\${$_v+set}"
    [ "${_set:-}" != "set" ] || fail "$_v leaked into the bare-environment section"
done

sh "$TMP/seq-c.sh" >"$TMP/out_c" 2>&1 &
PID_C=$!
sh "$TMP/seq-d.sh" >"$TMP/out_d" 2>&1 &
PID_D=$!
wait "$PID_C" || fail "bare sequence C exited non-zero"
wait "$PID_D" || fail "bare sequence D exited non-zero"

json_lines "$TMP/out_c" | field_str id >"$TMP/ids_c"
json_lines "$TMP/out_d" | field_str id >"$TMP/ids_d"
all_same "$TMP/ids_c" || fail "bare sequence C is unstable: $(cat "$TMP/ids_c")"
all_same "$TMP/ids_d" || fail "bare sequence D is unstable: $(cat "$TMP/ids_d")"
ID_C=$(head -n 1 "$TMP/ids_c")
ID_D=$(head -n 1 "$TMP/ids_d")
[ "$ID_C" != "$ID_D" ] || fail "bare sequences merged onto one id ($ID_C)"
ok "the bare-environment run reproduces section A's stability and distinctness"

# Without this, a bug that fell through to rung 4 would pass the distinctness
# half above while silently failing the stability half in a later refactor.
json_lines "$TMP/out_c" | field_num rung >"$TMP/rungs_c"
json_lines "$TMP/out_d" | field_num rung >"$TMP/rungs_d"
cat "$TMP/rungs_c" "$TMP/rungs_d" >"$TMP/rungs_all"
[ "$(sort -u "$TMP/rungs_all")" = "2" ] ||
    fail "expected every bare invocation on rung 2, got: $(sort -u "$TMP/rungs_all" | tr '\n' ' ')"
ok "every bare invocation resolved on rung 2 (the inherited lease)"

LEASES_B="$REPO_B/.git/rdm/leases"
LEASE_COUNT=$(find "$LEASES_B" -name '*.lease' | wc -l | tr -d ' ')
[ "$LEASE_COUNT" = "2" ] ||
    fail "expected exactly 2 lease files (one per driving shell), got $LEASE_COUNT"
grep -q "\"id\":\"$ID_C\"" "$LEASES_B/$PID_C.lease" ||
    fail "shell $PID_C's lease does not record the id it resolved ($ID_C)"
grep -q "\"id\":\"$ID_D\"" "$LEASES_B/$PID_D.lease" ||
    fail "shell $PID_D's lease does not record the id it resolved ($ID_D)"
ok "exactly two leases exist, keyed by the two driving shell pids, carrying their ids"

# A degenerate parent (rdm invoked directly by this script, whose own ancestry
# may be arbitrary) must still print an id and exit 0.
"$RDM_BIN" --root "$REPO_B" session id >"$TMP/degenerate.out" 2>&1 ||
    fail "a direct invocation must still resolve: $(cat "$TMP/degenerate.out")"
[ -s "$TMP/degenerate.out" ] || fail "a direct invocation printed no id"
ok "a degenerate-parent invocation still resolves and exits 0"

# ---------------------------------------------------------------------------
# Section C — rung 1 and rung 3 precedence
# ---------------------------------------------------------------------------
say "Section C: RDM_SESSION beats everything; rung 3 is derived, not lease-backed"

REPO_C="$TMP/repo-c"
seed_repo "$REPO_C"

harness_var_run() {
    _script=$1
    _value=$2
    _extra=$3
    {
        echo '#!/bin/sh'
        echo "export CLAUDE_CODE_SESSION_ID=\"$_value\""
        [ -z "$_extra" ] || echo "export RDM_SESSION=\"$_extra\""
        echo "\"$RDM_BIN\" --root \"$REPO_C\" session id --format json"
        echo ':'
    } >"$_script"
    sh "$_script"
}

harness_var_run "$TMP/c1.sh" abc123 "" >"$TMP/c1.out"
harness_var_run "$TMP/c2.sh" abc123 "" >"$TMP/c2.out"
C1=$(json_lines "$TMP/c1.out" | field_str id)
C2=$(json_lines "$TMP/c2.out" | field_str id)
[ "$C1" = "$C2" ] ||
    fail "the same harness variable from two different parents produced different ids: $C1 vs $C2"
[ "$(json_lines "$TMP/c1.out" | field_num rung)" = "3" ] ||
    fail "expected rung 3 from a harness variable, got $(json_lines "$TMP/c1.out" | field_num rung)"
ok "two unrelated parent shells sharing CLAUDE_CODE_SESSION_ID agree on one rung-3 id"

harness_var_run "$TMP/c3.sh" different-value "" >"$TMP/c3.out"
C3=$(json_lines "$TMP/c3.out" | field_str id)
[ "$C3" != "$C1" ] || fail "a different harness value produced the same id ($C1)"
ok "a different harness value derives a different id"

harness_var_run "$TMP/c4.sh" abc123 explicit-1 >"$TMP/c4.out"
[ "$(json_lines "$TMP/c4.out" | field_str id)" = "explicit-1" ] ||
    fail "RDM_SESSION did not win over the harness variable"
[ "$(json_lines "$TMP/c4.out" | field_num rung)" = "1" ] ||
    fail "expected rung 1 with RDM_SESSION set"
ok "RDM_SESSION overrides the harness variable and reports rung 1"

# Rung 3 is derived, so it must leave no lease behind for anyone to inherit.
[ ! -d "$REPO_C/.git/rdm/leases" ] || {
    [ "$(find "$REPO_C/.git/rdm/leases" -name '*.lease' | wc -l | tr -d ' ')" = "0" ] ||
        fail "rung 3 created a lease; it is supposed to need no on-disk state"
}
ok "rung 3 needs no on-disk state"

# ---------------------------------------------------------------------------
# Section D — a recycled pid does not adopt a stale lease
# ---------------------------------------------------------------------------
say "Section D: a live pid carrying a stale start time is not adopted"

REPO_D="$TMP/repo-d"
seed_repo "$REPO_D"
LEASES_D="$REPO_D/.git/rdm/leases"
SENTINEL="s-deadbeefdeadbeef"

{
    echo '#!/bin/sh'
    echo "mkdir -p \"$LEASES_D\""
    # \$\$ is this shell's pid — the immediate parent of the rdm below. The
    # recorded start_time is deliberately bogus, which is exactly what a
    # recycled pid looks like from rdm's side.
    echo "printf '%s' '{\"id\":\"$SENTINEL\",\"start_time\":\"not-the-live-start-time\",\"created_utc\":\"2026-01-01T00:00:00Z\"}' > \"$LEASES_D/\$\$.lease\""
    echo "echo \"\$\$\" > \"$TMP/recycle.pid\""
    echo "\"$RDM_BIN\" --root \"$REPO_D\" session id --format json"
    echo ':'
} >"$TMP/recycle.sh"
sh "$TMP/recycle.sh" >"$TMP/recycle.out" 2>&1 || fail "the recycle run exited non-zero"

RECYCLE_PID=$(cat "$TMP/recycle.pid")
RECYCLE_ID=$(json_lines "$TMP/recycle.out" | field_str id)
[ -n "$RECYCLE_ID" ] || fail "the recycle run printed no id"
[ "$RECYCLE_ID" != "$SENTINEL" ] ||
    fail "a stale lease was adopted — pid recycling would silently merge two sessions"
if [ -f "$LEASES_D/$RECYCLE_PID.lease" ]; then
    grep -q "$SENTINEL" "$LEASES_D/$RECYCLE_PID.lease" &&
        fail "the stale lease survived on disk and could still be adopted later"
fi
ok "the stale lease is neither adopted nor left on disk"

# ---------------------------------------------------------------------------
# Section E — a journal lists exactly its mutation's paths
# ---------------------------------------------------------------------------
say "Section E: the journal is neither a superset nor empty"

REPO_E="$TMP/repo-e"
seed_repo "$REPO_E"
{
    echo '#!/bin/sh'
    echo "\"$RDM_BIN\" --root \"$REPO_E\" task create alpha-one --title \"Alpha\" --no-edit --project demo >/dev/null"
    echo "\"$RDM_BIN\" --root \"$REPO_E\" session journal --format json"
    echo ':'
} >"$TMP/e.sh"
sh "$TMP/e.sh" >"$TMP/e.out" 2>"$TMP/e.err" || fail "section E run failed: $(cat "$TMP/e.err")"

json_lines "$TMP/e.out" >"$TMP/e.json"
paths_of "$TMP/e.json" >"$TMP/e.paths"
[ -s "$TMP/e.paths" ] || fail "the journal is empty after a mutation"
cat >"$TMP/e.expected" <<'EOF'
INDEX.md
projects/demo/INDEX.md
projects/demo/tasks/alpha-one.md
EOF
# Whole-set equality, deliberately: a `grep -q` containment check would pass on
# a superset, which is precisely the failure mode that would make a later
# scoped commit unsound.
diff -u "$TMP/e.expected" "$TMP/e.paths" ||
    fail "the journal is not exactly the mutation's own paths"
ok "the journal lists exactly the task file plus the two regenerated indexes"

# ---------------------------------------------------------------------------
# Section F — two coexisting journals are disjoint
# ---------------------------------------------------------------------------
say "Section F: concurrent journals never contain each other's paths"

REPO_F="$TMP/repo-f"
seed_repo "$REPO_F"
for _pair in "alpha:alpha-one" "beta:beta-one"; do
    _name=${_pair%%:*}
    _slug=${_pair##*:}
    {
        echo '#!/bin/sh'
        echo "\"$RDM_BIN\" --root \"$REPO_F\" task create $_slug --title \"T\" --no-edit --project demo >/dev/null"
        echo "\"$RDM_BIN\" --root \"$REPO_F\" session journal --format json"
        echo ':'
    } >"$TMP/f-$_name.sh"
done
# Two distinct long-lived parents, hence two coexisting changesets. They run
# one after the other so the assertion is about identity scoping and not about
# whichever of two racing writers last regenerated the shared index — that
# lost-update window is a separate phase's subject.
sh "$TMP/f-alpha.sh" >"$TMP/f-alpha.out" 2>&1 || fail "session alpha failed"
sh "$TMP/f-beta.sh" >"$TMP/f-beta.out" 2>&1 || fail "session beta failed"

json_lines "$TMP/f-alpha.out" >"$TMP/f-alpha.json"
json_lines "$TMP/f-beta.out" >"$TMP/f-beta.json"
paths_of "$TMP/f-alpha.json" >"$TMP/f-alpha.paths"
paths_of "$TMP/f-beta.json" >"$TMP/f-beta.paths"

grep -q '^projects/demo/tasks/alpha-one.md$' "$TMP/f-alpha.paths" ||
    fail "alpha's journal is missing its own task"
grep -q '^projects/demo/tasks/beta-one.md$' "$TMP/f-alpha.paths" &&
    fail "alpha's journal contains beta's task — the journals are not disjoint"
grep -q '^projects/demo/tasks/beta-one.md$' "$TMP/f-beta.paths" ||
    fail "beta's journal is missing its own task"
grep -q '^projects/demo/tasks/alpha-one.md$' "$TMP/f-beta.paths" &&
    fail "beta's journal contains alpha's task — the journals are not disjoint"
ok "neither journal contains the other session's task"

# The shared derived index lands in BOTH journals, and that is correct: every
# session really did rewrite it, because ops::mutate regenerates it on every
# mutation. Reconciling that at commit time belongs to the scoped-commit phase
# and is deliberately not pre-solved here.
grep -q '^INDEX.md$' "$TMP/f-alpha.paths" || fail "alpha's journal omits the derived index"
grep -q '^INDEX.md$' "$TMP/f-beta.paths" || fail "beta's journal omits the derived index"
ok "the shared derived index is journaled by both sessions, as intended"

# ---------------------------------------------------------------------------
# Section G — invisible to `rdm status` and to a whole-tree commit
# ---------------------------------------------------------------------------
say "Section G: session state is invisible to rdm status and to git"

REPO_G="$TMP/repo-g"
seed_repo "$REPO_G"
{
    echo '#!/bin/sh'
    echo "\"$RDM_BIN\" --root \"$REPO_G\" task create gamma-one --title \"Gamma\" --no-edit --project demo >/dev/null"
    echo "\"$RDM_BIN\" --root \"$REPO_G\" status"
    echo ':'
} >"$TMP/g.sh"
sh "$TMP/g.sh" >"$TMP/g.status" 2>&1 || fail "section G mutation/status failed"

[ -d "$REPO_G/.git/rdm/changesets" ] || fail "no journal was written, so this section is vacuous"
for _needle in leases changesets .git; do
    grep -q "$_needle" "$TMP/g.status" &&
        fail "rdm status named session state ($_needle): $(cat "$TMP/g.status")"
done
ok "rdm status names no lease, changeset, or .git path"

# `--all` is the whole-tree opt-in this section is explicitly about: the
# mutation above was made by a different (rung-2) session, so the scoped
# default would correctly decline to commit it. What is under test here is
# that even a whole-tree sweep never picks up lease or journal state.
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_G" commit --all -m "add gamma-one" >/dev/null
git -C "$REPO_G" ls-tree -r --name-only HEAD >"$TMP/g.tree"
grep -q '^projects/demo/tasks/gamma-one.md$' "$TMP/g.tree" ||
    fail "the commit did not contain the mutation, so this section is vacuous"
for _needle in 'rdm/leases' 'rdm/changesets' '\.lease$' '\.jsonl$'; do
    grep -qE "$_needle" "$TMP/g.tree" &&
        fail "a whole-tree commit swept up session state ($_needle)"
done
[ -z "$(git -C "$REPO_G" status --porcelain)" ] ||
    fail "git status is dirty after the commit: $(git -C "$REPO_G" status --porcelain)"
ok "the commit contains no lease or journal path and leaves git clean"

# Self-test: prove the ls-tree assertion above is not vacuous by planting a
# mis-sited journal at the repo root and showing the same check WOULD catch it.
printf '{"paths":[]}\n' >"$REPO_G/rdm-changesets-decoy.jsonl"
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_G" commit --all -m "decoy" >/dev/null
git -C "$REPO_G" ls-tree -r --name-only HEAD >"$TMP/g.tree.decoy"
grep -qE '\.jsonl$' "$TMP/g.tree.decoy" ||
    fail "self-test failed: the ls-tree check cannot see a mis-sited journal, so it proves nothing"
ok "self-test: a mis-sited journal at the repo root IS caught by the same check"
rm -f "$REPO_G/rdm-changesets-decoy.jsonl"
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_G" commit --all -m "remove decoy" >/dev/null

# ---------------------------------------------------------------------------
# Section H — measured resolution cost, and the real hook path
# ---------------------------------------------------------------------------
say "Section H: resolution cost against the hook_timeout_secs budget"

REPO_H="$TMP/repo-h"
seed_repo "$REPO_H"
: >"$TMP/h.micros"
_i=0
while [ "$_i" -lt 20 ]; do
    "$RDM_BIN" --root "$REPO_H" session id --format json >"$TMP/h.one" 2>&1 ||
        fail "cost run $_i failed: $(cat "$TMP/h.one")"
    json_lines "$TMP/h.one" | field_num resolve_micros >>"$TMP/h.micros"
    _i=$((_i + 1))
done
[ "$(wc -l <"$TMP/h.micros" | tr -d ' ')" = "20" ] || fail "expected 20 cost samples"
MAX_MICROS=$(sort -n "$TMP/h.micros" | tail -n 1)
printf '      max resolve_micros over 20 fresh invocations: %s µs\n' "$MAX_MICROS"
# A bound, not an exact figure: three orders of magnitude under the 30 s
# default hook_timeout_secs, so identity resolution cannot put the unattended
# hook path anywhere near its deadline.
[ "$MAX_MICROS" -lt 250000 ] ||
    fail "resolution cost ${MAX_MICROS}µs exceeds the 250000µs bound"
ok "resolution stays well inside the hook deadline budget"

say 'Section H2: a real rdm hook post-commit still completes within the deadline'
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_H" task create hooked \
    --title "Hooked" --no-edit --project demo >/dev/null
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_H" commit -m "add hooked" >/dev/null
git -C "$REPO_H" commit --allow-empty -q -m "chore: land

Done: task/hooked"
# The hook resolves its repo from the CWD, exactly as git invokes it, so the
# driving shell must run inside the plan repo rather than merely pass --root.
{
    echo '#!/bin/sh'
    echo "cd \"$REPO_H\" || exit 1"
    echo "\"$RDM_BIN\" --root \"$REPO_H\" hook post-commit"
    echo ':'
} >"$TMP/h2.sh"
sh "$TMP/h2.sh" >"$TMP/h2.out" 2>&1 || fail "rdm hook post-commit exited non-zero: $(cat "$TMP/h2.out")"
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_H" task show hooked --project demo \
    --no-body >"$TMP/h2.show" 2>&1
grep -qi '^Status: *done' "$TMP/h2.show" ||
    fail "the Done: directive did not land, so the hook path was not really exercised"
find "$REPO_H/.git/rdm/changesets" -name '*.jsonl' | grep -q . ||
    fail "the hook produced no journal, so identity did not resolve on the hook path"
ok "the hook path completes, applies its directive, and journals its own writes"

# ---------------------------------------------------------------------------
# Section I — commit.rs IS coupled to the changeset model
# ---------------------------------------------------------------------------
# INVERTED as of `plan-repo-concurrency/phase-5-scope-commits-to-changesets`.
#
# Through phase 4 this section FORBADE the coupling: routing committers through
# the journal was a later phase's work, and a stray reference here would have
# been premature. Phase 5 is that later phase — attribution now lives in the
# tree builder itself — so the same file must now *carry* the coupling, and the
# original assertion would fail the build on arrival. The section is inverted
# rather than deleted so the boundary stays gated in both directions.
say "Section I: rdm-store-git/src/commit.rs carries the changeset commit scope"

COMMIT_RS="$REPO_ROOT/rdm-store-git/src/commit.rs"
[ -f "$COMMIT_RS" ] || fail "$COMMIT_RS not found"

for needle in 'ChangesetScope' 'CommitScope' 'changeset'; do
    grep -q "$needle" "$COMMIT_RS" ||
        fail "commit.rs no longer mentions '$needle' — the scoped commit path is the mechanism that keeps one session's commit from sweeping another's paths; losing it silently reverts phase 5"
done
ok "commit.rs declares the changeset commit scope"

# The coupling must be to the *path list*, not to ambient process state: the
# tree builder takes the paths it is given so it stays testable without a
# resolved session. `rdm_core::session::journal` may be referenced in prose.
if grep -q 'resolve_session\|resolve_system_session\|SessionPaths::' "$COMMIT_RS"; then
    fail "commit.rs resolves session identity itself — the scope must arrive as a caller-supplied path list, so the tree builder stays testable without ambient process state"
fi
ok "commit.rs takes a supplied path list and resolves no session identity of its own"

# Self-test: both directions of the assertion must be able to fail.
sed 's/ChangesetScope/WholeTreeOnly/g' "$COMMIT_RS" >"$TMP/commit-unscoped.rs"
grep -q 'ChangesetScope' "$TMP/commit-unscoped.rs" &&
    fail "self-test failed: a commit.rs with the scope removed still matches, so the assertion proves nothing"
ok "self-test: a commit.rs stripped of the changeset scope IS caught"

cp "$COMMIT_RS" "$TMP/commit-ambient.rs"
printf '\n// planted: let s = rdm_core::session::resolve_session(paths);\n' >>"$TMP/commit-ambient.rs"
grep -q 'resolve_session' "$TMP/commit-ambient.rs" ||
    fail "self-test failed: a planted ambient-session resolution is not detected, so that assertion proves nothing"
ok "self-test: a planted ambient session resolution IS caught"

# ---------------------------------------------------------------------------
# Section J — a harness variable beats an already-inherited ancestor lease
# ---------------------------------------------------------------------------
# Reproduces and gates the phase-10 fix: a parent shell that ran one bare
# (harness-less) `rdm` invocation mints an ancestor lease; two of its children,
# each carrying its own distinct CLAUDE_CODE_SESSION_ID, must resolve their
# own harness-derived (rung 3) ids rather than being forced onto that lease.
say "Section J: a harness-published id beats an already-inherited ancestor lease"

REPO_J="$TMP/repo-j"
seed_repo "$REPO_J"

# The edge case this section depends on: the four session vars must still be
# unset here, exactly as asserted at the top of Section B, or the "plant"
# step below would resolve via rung 1/3 instead of minting a bare rung-2
# lease — breaking the premise the rest of the section relies on.
for _v in RDM_SESSION CLAUDE_CODE_SESSION_ID CLAUDE_SESSION_ID RDM_HARNESS_SESSION_ID; do
    eval "_set=\${$_v+set}"
    [ "${_set:-}" != "set" ] || fail "$_v leaked into Section J's plant step"
done

# (a) Plant: one bare invocation, directly from this script's own process, so
# the lease is created keyed at THIS shell's pid — the ancestor the two
# children below share.
"$RDM_BIN" --root "$REPO_J" session id --format json >"$TMP/j_plant.out" ||
    fail "the plant invocation failed: $(cat "$TMP/j_plant.out")"
[ "$(json_lines "$TMP/j_plant.out" | field_num rung)" = "2" ] ||
    fail "expected the plant invocation to land on rung 2, got: $(cat "$TMP/j_plant.out")"
LEASES_J="$REPO_J/.git/rdm/leases"
LEASE_COUNT_J=$(find "$LEASES_J" -name '*.lease' | wc -l | tr -d ' ')
[ "$LEASE_COUNT_J" = "1" ] ||
    fail "expected exactly 1 planted lease file, got $LEASE_COUNT_J"
LEASED_ID=$(json_lines "$TMP/j_plant.out" | field_str id)
[ -n "$LEASED_ID" ] || fail "the plant invocation produced no id"
ok "planted one ancestor lease ($LEASED_ID) with nothing but a bare invocation"

# (b) Two children of that same ancestor, each with its own harness session
# id, each invoking `rdm session id` TWICE before exiting (repeat-invocation
# determinism within one process run).
child_run() {
    _value=$1
    _out=$2
    (
        CLAUDE_CODE_SESSION_ID="$_value"
        export CLAUDE_CODE_SESSION_ID
        "$RDM_BIN" --root "$REPO_J" session id --format json
        "$RDM_BIN" --root "$REPO_J" session id --format json
    ) >"$_out" 2>&1
}

child_run child-one "$TMP/j_child1.out" &
PID_J1=$!
child_run child-two "$TMP/j_child2.out" &
PID_J2=$!
wait "$PID_J1" || fail "child-one exited non-zero: $(cat "$TMP/j_child1.out")"
wait "$PID_J2" || fail "child-two exited non-zero: $(cat "$TMP/j_child2.out")"

json_lines "$TMP/j_child1.out" | field_str id >"$TMP/j_ids1"
json_lines "$TMP/j_child2.out" | field_str id >"$TMP/j_ids2"
json_lines "$TMP/j_child1.out" | field_num rung >"$TMP/j_rungs1"
json_lines "$TMP/j_child2.out" | field_num rung >"$TMP/j_rungs2"

[ "$(wc -l <"$TMP/j_ids1" | tr -d ' ')" = "2" ] ||
    fail "expected 2 ids from child-one, got: $(cat "$TMP/j_ids1")"
[ "$(wc -l <"$TMP/j_ids2" | tr -d ' ')" = "2" ] ||
    fail "expected 2 ids from child-two, got: $(cat "$TMP/j_ids2")"

# (c) Distinctness, repeat-invocation stability, and rung.
all_same "$TMP/j_ids1" || fail "child-one's repeat invocations disagree: $(cat "$TMP/j_ids1")"
all_same "$TMP/j_ids2" || fail "child-two's repeat invocations disagree: $(cat "$TMP/j_ids2")"
ok "the same child invoked twice resolves the same id"

CHILD1_ID=$(head -n 1 "$TMP/j_ids1")
CHILD2_ID=$(head -n 1 "$TMP/j_ids2")
[ "$CHILD1_ID" != "$CHILD2_ID" ] ||
    fail "two children with distinct CLAUDE_CODE_SESSION_ID values merged onto one id ($CHILD1_ID)"
ok "two children of one leased ancestor with distinct harness ids resolve distinct ids"

cat "$TMP/j_rungs1" "$TMP/j_rungs2" >"$TMP/j_rungs_all"
[ "$(sort -u "$TMP/j_rungs_all")" = "3" ] ||
    fail "expected every child invocation on rung 3, got: $(sort -u "$TMP/j_rungs_all" | tr '\n' ' ')"
ok "every child invocation resolved on rung 3 (the harness variable)"

# (d) Cross-check: neither child adopted the ancestor's lease, and no new
# lease was created along the way. This is a live-behavior assertion, not
# the section's planted-mutation self-test (that is (e) below) — it would
# pass just as well on a build that happened to route both children through
# rung 4 instead of rung 3, for instance.
[ "$CHILD1_ID" != "$LEASED_ID" ] ||
    fail "child-one adopted the planted ancestor lease ($LEASED_ID) instead of its own harness id — the phase-10 merging bug is back"
[ "$CHILD2_ID" != "$LEASED_ID" ] ||
    fail "child-two adopted the planted ancestor lease ($LEASED_ID) instead of its own harness id — the phase-10 merging bug is back"
ok "neither child adopted the planted ancestor lease"

LEASE_COUNT_J_AFTER=$(find "$LEASES_J" -name '*.lease' | wc -l | tr -d ' ')
[ "$LEASE_COUNT_J_AFTER" = "1" ] ||
    fail "expected still exactly 1 lease file after both children ran (rung 3 needs no on-disk state), got $LEASE_COUNT_J_AFTER"
ok "rung 3 needs no on-disk state — no lease was created for either child"

# (e) Planted-mutation self-test: neuter the harness check the way it read
# before phase 10 fixed the ordering, rebuild in a scratch CARGO_TARGET_DIR,
# and re-run the exact plant-then-two-children scenario against the mutant —
# it must reproduce the phase-10 merging bug (both children silently adopt
# the ancestor's lease). This is the automated proof that (a)-(d) above
# actually depend on the fix rather than merely describing it; mirrors the
# rebuild-a-mutant pattern used by Section 2c of verify-lost-update.sh.
say "Section J (self-test): planting the phase-10 regression and re-running the scenario"

MUT_J="$TMP/mutant-j"
mutant_tree "$MUT_J"
MUT_J_MOD="$MUT_J/rdm-core/src/session/mod.rs"
grep -q 'if let Some((var, raw)) = active_harness_var(env) {' "$MUT_J_MOD" ||
    fail "the harness-check call site moved — update this self-test to match"
sed 's|if let Some((var, raw)) = active_harness_var(env) {|if let Some((var, raw)) = active_harness_var(env) \&\& false { // MUTATION: reintroduces the phase-10 merging bug|' \
    "$MUT_J_MOD" >"$MUT_J_MOD.new"
mv "$MUT_J_MOD.new" "$MUT_J_MOD"
grep -q '// MUTATION' "$MUT_J_MOD" || fail "failed to plant the Section J mutation"

say "  building the mutant (scratch CARGO_TARGET_DIR; ~15s)"
(cd "$MUT_J" && CARGO_TARGET_DIR="$MUTANT_TARGET_DIR" cargo build -q -p rdm-cli --offline) ||
    fail "the mutant build failed — the self-test cannot run"
# Copy the binary out of the shared target dir immediately: the next
# mutant's build overwrites that path.
cp "$MUTANT_TARGET_DIR/debug/rdm" "$TMP/rdm-mutant-j" ||
    fail "the mutant binary was not produced at $MUTANT_TARGET_DIR/debug/rdm"
MUT_J_BIN="$TMP/rdm-mutant-j"
[ -x "$MUT_J_BIN" ] || fail "the mutant binary was not produced at $MUT_J_BIN"

REPO_J_MUT="$TMP/repo-j-mut"
mkdir -p "$REPO_J_MUT"
RDM_SESSION=harness-seed "$MUT_J_BIN" --root "$REPO_J_MUT" init --default-project demo >/dev/null
RDM_SESSION=harness-seed "$MUT_J_BIN" --root "$REPO_J_MUT" commit \
    -m "seed: init plan repo and project" >/dev/null

"$MUT_J_BIN" --root "$REPO_J_MUT" session id --format json >"$TMP/j_plant_mut.out" 2>&1 ||
    fail "the mutant's plant invocation failed: $(cat "$TMP/j_plant_mut.out")"
LEASED_ID_MUT=$(json_lines "$TMP/j_plant_mut.out" | field_str id)
[ -n "$LEASED_ID_MUT" ] || fail "the mutant's plant invocation produced no id"

child_run_mut() {
    _value=$1
    _out=$2
    (
        CLAUDE_CODE_SESSION_ID="$_value"
        export CLAUDE_CODE_SESSION_ID
        "$MUT_J_BIN" --root "$REPO_J_MUT" session id --format json
    ) >"$_out" 2>&1
}
child_run_mut child-one "$TMP/j_child1_mut.out" &
PID_JM1=$!
child_run_mut child-two "$TMP/j_child2_mut.out" &
PID_JM2=$!
wait "$PID_JM1" || fail "the mutant's child-one exited non-zero: $(cat "$TMP/j_child1_mut.out")"
wait "$PID_JM2" || fail "the mutant's child-two exited non-zero: $(cat "$TMP/j_child2_mut.out")"

CHILD1_ID_MUT=$(json_lines "$TMP/j_child1_mut.out" | field_str id)
CHILD2_ID_MUT=$(json_lines "$TMP/j_child2_mut.out" | field_str id)

if [ "$CHILD1_ID_MUT" = "$LEASED_ID_MUT" ] && [ "$CHILD2_ID_MUT" = "$LEASED_ID_MUT" ]; then
    ok "self-test: neutering the harness check DOES reproduce the phase-10 merging bug (both children merged onto $LEASED_ID_MUT)"
else
    fail "self-test failed: the mutant did not reproduce the phase-10 merging bug \
(child-one=$CHILD1_ID_MUT child-two=$CHILD2_ID_MUT leased=$LEASED_ID_MUT) — \
Section J would not catch a real regression of the fix"
fi

# ---------------------------------------------------------------------------
# Section K — continuity across ephemeral per-call wrapper shells
# ---------------------------------------------------------------------------
# Phase 11. An agent harness that exports no session variable and runs each
# tool call in a FRESH wrapper shell gives rdm nothing to key a session on that
# outlives one command: creation stops at depth 1 (see rdm-core/src/session/
# lease.rs for why phase 11 measured raising it and rejected the change), and
# depth 1 is that wrapper. So every invocation resolves its own changeset.
#
# Phase 11 kept that fragmentation — it is the safe direction of phase 3's
# binding asymmetry — and this section gates the two things that had to become
# true instead, which is exactly the second branch the phase's acceptance
# criterion allows:
#
#   * `rdm commit` NAMES the cause and the remedy rather than fragmenting
#     silently (K2), and the remedy it names actually works (K5); and
#   * the fragmenting topology no longer LEAKS a dead-pid lease per invocation
#     (K3/K3b), because creation now sweeps before it mints.
#
# K4 re-asserts the no-merge invariant phase 10 fences, under this topology.
say "Section K: per-call wrapper shells — fragmentation is named, bounded, and remediable"

REPO_K="$TMP/repo-k"
seed_repo "$REPO_K"
LEASES_K="$REPO_K/.git/rdm/leases"

# A long-lived NON-shell process standing in for the agent harness itself, so
# the ancestry under test is wrapper -> agentd -> …, matching a real harness
# rather than a chain of shells.
#
# A symlink, not a copy: on macOS, copying a code-signed system binary such as
# /bin/sh produces an image the kernel SIGKILLs on exec, while a symlink execs
# the real binary and still reports the symlink's own name as its `comm`.
AGENTD="$TMP/agentd"
ln -s /bin/sh "$AGENTD"

# sq <string>: print <string> single-quoted so a shell re-parsing it sees the
# bytes verbatim. Embedded single quotes become '\'' in the usual way.
#
# This is not fastidiousness. The generated drivers below assign a command line
# to a variable and `eval` it one level down; an argument carrying a space or a
# quote (`--title "K item"`) would otherwise split the assignment into
# `VAR=value command args`, leaving the variable UNCHANGED in the driver — and
# the wrapper would then eval whatever the variable held before. Quote this
# wrong and the section does not merely fail, it forks endlessly.
sq() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }

# write_wrapper_driver <out> <repo> <cmds-file>
#
# Emits a driver script that runs each line of <cmds-file> (arguments to `rdm`)
# inside its OWN wrapper shell, reproducing one-tool-call-per-shell.
#
# The `eval` plus the trailing `:` is load-bearing, not style: `sh -c '<one
# command>'` execs that command in place, the wrapper process vanishes, and rdm
# ends up parented directly by the long-lived driver — under which continuity
# works trivially and this whole section would pass for the wrong reason. K0
# below asserts the wrappers really are distinct, live processes.
#
# The payload variable is RDM_K_CMD, a name `k_drive` deliberately does not use:
# the driver's own invocation must never be reachable from inside a wrapper, or
# a mis-quoted payload turns a failed assertion into a self-re-invoking driver.
write_wrapper_driver() {
    _out=$1
    _repo=$2
    _cmds=$3
    {
        echo '#!/bin/sh'
        # Exported EMPTY first: if a payload assignment below were ever
        # mis-quoted, the wrapper evals nothing and exits, rather than
        # inheriting and re-running something.
        echo 'RDM_K_CMD='
        echo 'export RDM_K_CMD'
        while IFS= read -r _line; do
            [ -n "$_line" ] || continue
            _full="\"$RDM_BIN\" --root \"$_repo\" $_line"
            printf 'RDM_K_CMD=%s\n' "$(sq "$_full")"
            # SC2016: this printf writes the wrapper's source text; `$RDM_K_CMD`
            # must reach the generated script unexpanded.
            # shellcheck disable=SC2016
            printf 'sh -c '\''eval "$RDM_K_CMD"; :'\''\n'
        done <"$_cmds"
        echo ':'
    } >"$_out"
}

# k_drive <driver>: run <driver> under the agentd, so every wrapper shell it
# spawns has a real non-shell process as its grandparent.
#
# The driver path travels as a positional argument, never through the
# environment: nothing a wrapper can read ever names the driver itself.
# SC2016: `$1` is the agentd shell's own positional argument, supplied after
# the -c string; expanding it here would defeat the point.
# shellcheck disable=SC2016
k_drive() { "$AGENTD" -c 'sh "$1"; :' sh "$1"; }

# --- K0: the topology is really what the section claims ---------------------
# Without this, every assertion below could be describing a shape the harness
# never actually built.
{
    echo '#!/bin/sh'
    echo 'RDM_K_CMD='
    echo 'export RDM_K_CMD'
    _i=0
    while [ "$_i" -lt 2 ]; do
        # Each wrapper reports its OWN pid and its parent's, so the section can
        # prove the wrappers are distinct live processes sharing one driver.
        # shellcheck disable=SC2016
        printf 'RDM_K_CMD=%s\n' "$(sq 'printf "wrapper=%s driver=%s\n" "$$" "$PPID"')"
        # shellcheck disable=SC2016
        printf 'sh -c '\''eval "$RDM_K_CMD"; :'\''\n'
        _i=$((_i + 1))
    done
    echo ':'
} >"$TMP/k0.sh"
k_drive "$TMP/k0.sh" >"$TMP/k0.out" 2>&1 || fail "K0 driver failed: $(cat "$TMP/k0.out")"
K0_WRAPPERS=$(sed -n 's/^wrapper=\([0-9]*\).*/\1/p' "$TMP/k0.out" | sort -u | wc -l | tr -d ' ')
K0_DRIVERS=$(sed -n 's/.*driver=\([0-9]*\)$/\1/p' "$TMP/k0.out" | sort -u | wc -l | tr -d ' ')
[ "$K0_WRAPPERS" = "2" ] ||
    fail "expected 2 distinct wrapper pids (the shells did not survive their command), got $K0_WRAPPERS"
[ "$K0_DRIVERS" = "1" ] ||
    fail "expected both wrappers under ONE driver, got $K0_DRIVERS distinct parents"
ok "each tool call really runs in its own live wrapper shell under one driver"

# --- K1: every wrapper invocation resolves its own changeset ----------------
# The shipped behavior, asserted as the fact it is rather than left implied.
_i=0
: >"$TMP/k1.cmds"
while [ "$_i" -lt 5 ]; do
    echo 'session id --format json' >>"$TMP/k1.cmds"
    _i=$((_i + 1))
done
write_wrapper_driver "$TMP/k1.sh" "$REPO_K" "$TMP/k1.cmds"
k_drive "$TMP/k1.sh" >"$TMP/k1.out" 2>&1 || fail "K1 driver failed: $(cat "$TMP/k1.out")"

json_lines "$TMP/k1.out" | field_str id >"$TMP/k1.ids"
[ "$(wc -l <"$TMP/k1.ids" | tr -d ' ')" = "5" ] ||
    fail "expected 5 ids from 5 wrapper calls, got: $(cat "$TMP/k1.out")"
K1_DISTINCT=$(sort -u "$TMP/k1.ids" | wc -l | tr -d ' ')
[ "$K1_DISTINCT" = "5" ] ||
    fail "expected 5 distinct fragmented ids under a wrapper harness, got $K1_DISTINCT — \
if continuity now works here, phase 11's recorded decision changed and § K must be re-written"
[ "$(json_lines "$TMP/k1.out" | field_num rung | sort -u)" = "2" ] ||
    fail "expected every wrapper call on rung 2"
[ "$(json_lines "$TMP/k1.out" | grep -c '"lease_bootstrapped":true')" = "5" ] ||
    fail "expected every wrapper call to report lease_bootstrapped=true"
ok "each wrapper call bootstraps its own rung-2 changeset (the recorded, kept behavior)"

# --- K2 (the acceptance criterion): the commit names cause AND remedy -------
{
    echo 'task create k-item --title "K item" --no-edit --project demo'
    echo 'commit -m "k: land the wrapper batch"'
} >"$TMP/k2.cmds"
write_wrapper_driver "$TMP/k2.sh" "$REPO_K" "$TMP/k2.cmds"
k_drive "$TMP/k2.sh" >"$TMP/k2.out" 2>&1 ||
    fail "K2 driver exited non-zero — a fragmented harness must still exit 0: $(cat "$TMP/k2.out")"

grep -q 'CLAUDE_CODE_SESSION_ID' "$TMP/k2.out" ||
    fail "the commit output does not name the CAUSE (the harness publishes no session variable): $(cat "$TMP/k2.out")"
grep -q 'RDM_HARNESS_SESSION_ID' "$TMP/k2.out" ||
    fail "the commit output does not name the REMEDY (export RDM_HARNESS_SESSION_ID): $(cat "$TMP/k2.out")"
grep -q 'Cause:' "$TMP/k2.out" || fail "the advisory has no 'Cause:' label"
grep -q 'Remedy:' "$TMP/k2.out" || fail "the advisory has no 'Remedy:' label"
ok "a fragmented rdm commit exits 0 and names both the cause and the remedy"

# The other half of "never silently": the work is still there and still
# recoverable by the routes the same output prints.
git -C "$REPO_K" show --name-only HEAD >"$TMP/k2.head" 2>&1
if grep -q 'projects/demo/tasks/k-item.md' "$TMP/k2.head"; then
    fail "the wrapper commit unexpectedly landed the task — continuity now works, so § K must be re-written against AC1's first branch"
fi
K2_ORPHAN=$("$RDM_BIN" --root "$REPO_K" session list --format json |
    tr ',' '\n' | sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | head -n 1)
[ -n "$K2_ORPHAN" ] || fail "no changeset was recorded, so the mutation journaled nothing"
"$RDM_BIN" --root "$REPO_K" commit --changeset "$K2_ORPHAN" \
    -m "k: recover the orphaned changeset" >"$TMP/k2.recover" 2>&1 ||
    fail "the recovery route the advisory prints does not work: $(cat "$TMP/k2.recover")"
git -C "$REPO_K" show --name-only HEAD | grep -q 'projects/demo/tasks/k-item.md' ||
    fail "rdm commit --changeset did not land the fragmented work"
ok "the fragmented work is not lost — the printed recovery route lands it"

# --- K3: no dead-pid lease accumulates per invocation -----------------------
# The pre-phase-11 behavior left one lease per invocation, every one naming a
# pid that died with its wrapper. Creation now sweeps before it mints, so the
# directory stays bounded no matter how many tool calls run.
K3_COUNT=$(find "$LEASES_K" -name '*.lease' | wc -l | tr -d ' ')
[ "$K3_COUNT" -ge 1 ] ||
    fail "no lease at all after 7 wrapper invocations — the count assertion below would be vacuous"
[ "$K3_COUNT" -le 2 ] ||
    fail "leases accumulated per invocation: $K3_COUNT files after 7 wrapper calls (expected <= 2)"
ok "7 wrapper invocations left $K3_COUNT lease file(s), not 7"

# --- K3b: a dead anchor's lease is swept by the next invocation -------------
# Named explicitly so the bound above cannot be satisfied by a directory that
# merely happens to be small: the specific dead file must be gone.
K3B_STALE=$(find "$LEASES_K" -name '*.lease' | head -n 1)
[ -n "$K3B_STALE" ] || fail "expected a lease file to observe"
K3B_PID=$(basename "$K3B_STALE" .lease)
if kill -0 "$K3B_PID" 2>/dev/null; then
    fail "the observed lease names a LIVE pid ($K3B_PID); K3b needs the dead-wrapper case"
fi
echo 'session id --format json' >"$TMP/k3b.cmds"
write_wrapper_driver "$TMP/k3b.sh" "$REPO_K" "$TMP/k3b.cmds"
k_drive "$TMP/k3b.sh" >"$TMP/k3b.out" 2>&1 || fail "K3b driver failed: $(cat "$TMP/k3b.out")"
[ ! -e "$K3B_STALE" ] ||
    fail "the dead wrapper's lease ($K3B_PID) survived the next invocation — the create-path sweep did not run"
K3B_COUNT=$(find "$LEASES_K" -name '*.lease' | wc -l | tr -d ' ')
[ "$K3B_COUNT" -le 2 ] || fail "lease count grew to $K3B_COUNT after another invocation"
ok "the dead wrapper's lease is swept by the next invocation; the set stays bounded"

# --- K4: two concurrent drivers never merge (the phase-10 invariant) --------
REPO_K4="$TMP/repo-k4"
seed_repo "$REPO_K4"
echo 'session id --format json' >"$TMP/k4.cmds"
echo 'session id --format json' >>"$TMP/k4.cmds"
write_wrapper_driver "$TMP/k4a.sh" "$REPO_K4" "$TMP/k4.cmds"
write_wrapper_driver "$TMP/k4b.sh" "$REPO_K4" "$TMP/k4.cmds"
k_drive "$TMP/k4a.sh" >"$TMP/k4a.out" 2>&1 &
PID_K4A=$!
k_drive "$TMP/k4b.sh" >"$TMP/k4b.out" 2>&1 &
PID_K4B=$!
wait "$PID_K4A" || fail "K4 driver A failed: $(cat "$TMP/k4a.out")"
wait "$PID_K4B" || fail "K4 driver B failed: $(cat "$TMP/k4b.out")"
json_lines "$TMP/k4a.out" | field_str id >"$TMP/k4a.ids"
json_lines "$TMP/k4b.out" | field_str id >"$TMP/k4b.ids"
[ -s "$TMP/k4a.ids" ] && [ -s "$TMP/k4b.ids" ] || fail "K4 produced no ids"
if grep -qxF -f "$TMP/k4a.ids" "$TMP/k4b.ids"; then
    fail "two concurrent wrapper drivers shared a changeset id — the phase-10 merging direction is back"
fi
ok "two concurrent drivers never share an id"

# --- K5: the remedy the advisory prints actually delivers continuity --------
# AC1's second branch is only worth anything if the remedy is true. This is the
# same topology as K1/K2 with RDM_HARNESS_SESSION_ID exported, and it must
# produce ONE changeset, a commit that lands, and NO advisory.
REPO_K5="$TMP/repo-k5"
seed_repo "$REPO_K5"
{
    echo 'session id --format json'
    echo 'task create k5-item --title "K5 item" --no-edit --project demo'
    echo 'commit -m "k5: land under one changeset"'
    echo 'session id --format json'
} >"$TMP/k5.cmds"
write_wrapper_driver "$TMP/k5.sh" "$REPO_K5" "$TMP/k5.cmds"
{
    echo '#!/bin/sh'
    echo 'RDM_HARNESS_SESSION_ID=k5-agent-run'
    echo 'export RDM_HARNESS_SESSION_ID'
    echo "exec sh \"$TMP/k5.sh\""
} >"$TMP/k5-outer.sh"
k_drive "$TMP/k5-outer.sh" >"$TMP/k5.out" 2>&1 || fail "K5 driver failed: $(cat "$TMP/k5.out")"

json_lines "$TMP/k5.out" | field_str id >"$TMP/k5.ids"
all_same "$TMP/k5.ids" ||
    fail "the documented remedy did not give one id across wrapper calls: $(cat "$TMP/k5.ids")"
[ "$(json_lines "$TMP/k5.out" | field_num rung | sort -u)" = "3" ] ||
    fail "expected rung 3 under RDM_HARNESS_SESSION_ID"
git -C "$REPO_K5" show --name-only HEAD | grep -q 'projects/demo/tasks/k5-item.md' ||
    fail "the remedy did not make mutate-then-commit land across wrapper calls: $(cat "$TMP/k5.out")"
if grep -q 'RDM_HARNESS_SESSION_ID=<' "$TMP/k5.out"; then
    fail "the advisory fired at a caller that already has continuity"
fi
[ ! -d "$REPO_K5/.git/rdm/leases" ] ||
    [ "$(find "$REPO_K5/.git/rdm/leases" -name '*.lease' | wc -l | tr -d ' ')" = "0" ] ||
    fail "the remedy path created a lease; rung 3 needs no on-disk state"
ok "exporting RDM_HARNESS_SESSION_ID gives one changeset, a landing commit, no lease, and no advisory"

# --- K6: the document records which harnesses get continuity (AC3) ----------
# A grep-level guard that the harness-continuity table is not silently dropped
# by a later edit. It gates presence, not prose.
DOC_K="$REPO_ROOT/docs/session-identity.md"
[ -f "$DOC_K" ] || fail "docs/session-identity.md is missing"
grep -q 'Continuity across ephemeral wrapper shells' "$DOC_K" ||
    fail "docs/session-identity.md has no phase-11 continuity section"
for _needle in 'RDM_HARNESS_SESSION_ID' 'Claude Code' 'Pi' 'plain interactive shell' 'no readable process table'; do
    grep -qi -- "$_needle" "$DOC_K" ||
        fail "docs/session-identity.md's harness-continuity table no longer covers '$_needle'"
done
ok "docs/session-identity.md records the phase-11 outcome and the per-harness table"

# ---------------------------------------------------------------------------
# Section K (self-tests) — two planted mutations, each rebuilt and re-run
# ---------------------------------------------------------------------------
# Same export-HEAD + working-tree-overlay + scratch CARGO_TARGET_DIR pattern as
# Section J. Each proves one half of § K is not vacuous.

# --- self-test 1: neuter the advisory --------------------------------------
say "Section K (self-test 1): silencing the advisory must break K2"

MUT_K1="$TMP/mutant-k1"
mutant_tree "$MUT_K1"
MUT_K1_MOD="$MUT_K1/rdm-core/src/session/mod.rs"
grep -q 'pub fn continuity_advisory' "$MUT_K1_MOD" ||
    fail "continuity_advisory moved — update this self-test to match"
# Neuter the RESULT, not a guard: returning None unconditionally is what
# "there is no advisory" means. Suppressing a guard instead would make the
# advisory fire MORE often, which is not the regression under test. Replacing
# the final expression also leaves no unreachable code for -D warnings to
# reject, so the mutant still builds.
grep -q '^    Some(lines.join(' "$MUT_K1_MOD" ||
    fail "continuity_advisory's return expression moved — update this self-test to match"
sed 's|^    Some(lines.join(.*$|    let _ = lines; // MUTATION: silences the phase-11 advisory\n    None|' \
    "$MUT_K1_MOD" >"$MUT_K1_MOD.new"
mv "$MUT_K1_MOD.new" "$MUT_K1_MOD"
grep -q '// MUTATION' "$MUT_K1_MOD" || fail "failed to plant the § K self-test 1 mutation"

say "  building mutant 1 (scratch CARGO_TARGET_DIR; ~15s)"
(cd "$MUT_K1" && CARGO_TARGET_DIR="$MUTANT_TARGET_DIR" cargo build -q -p rdm-cli --offline) ||
    fail "mutant 1 failed to build — the self-test cannot run"
# Copy the binary out of the shared target dir immediately: the next
# mutant's build overwrites that path.
cp "$MUTANT_TARGET_DIR/debug/rdm" "$TMP/rdm-mutant-k1" ||
    fail "the mutant binary was not produced at $MUTANT_TARGET_DIR/debug/rdm"
MUT_K1_BIN="$TMP/rdm-mutant-k1"
[ -x "$MUT_K1_BIN" ] || fail "mutant 1 binary was not produced"

REPO_MK1="$TMP/repo-mk1"
mkdir -p "$REPO_MK1"
RDM_SESSION=harness-seed "$MUT_K1_BIN" --root "$REPO_MK1" init --default-project demo >/dev/null
RDM_SESSION=harness-seed "$MUT_K1_BIN" --root "$REPO_MK1" commit -m "seed" >/dev/null
{
    echo 'task create m-item --title "M item" --no-edit --project demo'
    echo 'commit -m "mut: land"'
} >"$TMP/mk1.cmds"
RDM_BIN_SAVED="$RDM_BIN"
RDM_BIN="$MUT_K1_BIN"
write_wrapper_driver "$TMP/mk1.sh" "$REPO_MK1" "$TMP/mk1.cmds"
RDM_BIN="$RDM_BIN_SAVED"
k_drive "$TMP/mk1.sh" >"$TMP/mk1.out" 2>&1 || true
if grep -q 'RDM_HARNESS_SESSION_ID' "$TMP/mk1.out"; then
    fail "self-test 1 failed: the mutant still printed the remedy, so K2's assertion \
would pass on a build with no advisory at all"
fi
ok "self-test 1: silencing continuity_advisory DOES break K2's cause/remedy assertion"

# --- self-test 2: neuter the create-path sweep ------------------------------
say "Section K (self-test 2): removing the create-path sweep must break K3"

MUT_K2="$TMP/mutant-k2"
mutant_tree "$MUT_K2"
MUT_K2_LEASE="$MUT_K2/rdm-core/src/session/lease.rs"
grep -q '^    gc(paths, procs);' "$MUT_K2_LEASE" ||
    fail "the create-path gc call site moved — update this self-test to match"
sed 's|^    gc(paths, procs);|    // MUTATION: removes the phase-11 create-path sweep|' \
    "$MUT_K2_LEASE" >"$MUT_K2_LEASE.new"
mv "$MUT_K2_LEASE.new" "$MUT_K2_LEASE"
grep -q '// MUTATION' "$MUT_K2_LEASE" || fail "failed to plant the § K self-test 2 mutation"

say "  building mutant 2 (scratch CARGO_TARGET_DIR; ~15s)"
(cd "$MUT_K2" && CARGO_TARGET_DIR="$MUTANT_TARGET_DIR" cargo build -q -p rdm-cli --offline) ||
    fail "mutant 2 failed to build — the self-test cannot run"
# Copy the binary out of the shared target dir immediately: the next
# mutant's build overwrites that path.
cp "$MUTANT_TARGET_DIR/debug/rdm" "$TMP/rdm-mutant-k2" ||
    fail "the mutant binary was not produced at $MUTANT_TARGET_DIR/debug/rdm"
MUT_K2_BIN="$TMP/rdm-mutant-k2"
[ -x "$MUT_K2_BIN" ] || fail "mutant 2 binary was not produced"

REPO_MK2="$TMP/repo-mk2"
mkdir -p "$REPO_MK2"
RDM_SESSION=harness-seed "$MUT_K2_BIN" --root "$REPO_MK2" init --default-project demo >/dev/null
RDM_SESSION=harness-seed "$MUT_K2_BIN" --root "$REPO_MK2" commit -m "seed" >/dev/null
: >"$TMP/mk2.cmds"
_i=0
while [ "$_i" -lt 5 ]; do
    echo 'session id --format json' >>"$TMP/mk2.cmds"
    _i=$((_i + 1))
done
RDM_BIN_SAVED="$RDM_BIN"
RDM_BIN="$MUT_K2_BIN"
write_wrapper_driver "$TMP/mk2.sh" "$REPO_MK2" "$TMP/mk2.cmds"
RDM_BIN="$RDM_BIN_SAVED"
k_drive "$TMP/mk2.sh" >"$TMP/mk2.out" 2>&1 || fail "mutant 2 driver failed: $(cat "$TMP/mk2.out")"
MK2_COUNT=$(find "$REPO_MK2/.git/rdm/leases" -name '*.lease' | wc -l | tr -d ' ')
if [ "$MK2_COUNT" -le 2 ]; then
    fail "self-test 2 failed: without the create-path sweep the mutant still left only \
$MK2_COUNT lease(s) after 5 invocations — K3's bound would pass on a build that leaks"
fi
ok "self-test 2: removing the sweep DOES reproduce the per-invocation lease leak ($MK2_COUNT files)"

printf '\n\033[1;32mAll session-identity checks passed.\033[0m\n'
