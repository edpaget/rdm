#!/bin/sh
# Hermetic regression for session identity and the changeset journal.
#
# Drives REAL separate processes (not in-process fakes) against temp plan
# repos to gate: cross-process identity stability within one session and
# distinctness between two, the rung-1/rung-3 precedence rules, stale-lease
# rejection on a recycled pid, journal exactness and disjointness, the
# invisibility of session state to `rdm status` and to a whole-tree commit
# (with a planted-decoy self-test), the measured resolution cost including a
# real `rdm hook post-commit` run, and a structural grep proving
# rdm-store-git/src/commit.rs stayed free of session/journal references —
# routing committers through the journal belongs to a later phase.
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

RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_G" commit -m "add gamma-one" >/dev/null
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
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_G" commit -m "decoy" >/dev/null
git -C "$REPO_G" ls-tree -r --name-only HEAD >"$TMP/g.tree.decoy"
grep -qE '\.jsonl$' "$TMP/g.tree.decoy" ||
    fail "self-test failed: the ls-tree check cannot see a mis-sited journal, so it proves nothing"
ok "self-test: a mis-sited journal at the repo root IS caught by the same check"
rm -f "$REPO_G/rdm-changesets-decoy.jsonl"
RDM_SESSION=harness-seed "$RDM_BIN" --root "$REPO_G" commit -m "remove decoy" >/dev/null

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
# Section I — commit.rs stays free of session/journal references
# ---------------------------------------------------------------------------
say "Section I: rdm-store-git/src/commit.rs mentions no session, journal, or changeset"

COMMIT_RS="$REPO_ROOT/rdm-store-git/src/commit.rs"
[ -f "$COMMIT_RS" ] || fail "$COMMIT_RS not found"
PATTERN='session\|journal\|changeset'

if grep -qi "$PATTERN" "$COMMIT_RS"; then
    fail "commit.rs references session/journal/changeset — routing committers is a later phase's work, and a stray reference here is exactly the premature coupling this split exists to prevent"
fi
ok "commit.rs is free of session/journal/changeset references"

# Self-test: the grep above must be able to fail.
cp "$COMMIT_RS" "$TMP/commit-mutated.rs"
printf '\n// planted: record_journal(&touched);\n' >>"$TMP/commit-mutated.rs"
grep -qi "$PATTERN" "$TMP/commit-mutated.rs" ||
    fail "self-test failed: the structural grep cannot detect a planted reference, so it proves nothing"
ok "self-test: a planted journal reference IS caught by the same grep"

printf '\n\033[1;32mAll session-identity checks passed.\033[0m\n'
