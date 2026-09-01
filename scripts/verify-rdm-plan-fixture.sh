#!/bin/sh
# Hermetic self-test for scripts/lib/rdm-plan-fixture.sh.
#
# No consumer of the fixture library has landed yet (golden-JSON snapshots
# and verify-plugin-loop.sh are later phases of the editor-integration
# roadmap), so this harness proves the library's four contracts directly:
#
#   AC1 setup/teardown : fixture_setup/fixture_teardown/fixture_code_repo
#                        stand up and tear down an isolated plan (+ code)
#                        repo with the documented seed set.
#   AC2 determinism     : two same-day fixture_setup runs yield
#                        byte-identical `--format json` output once exactly
#                        the header-documented volatile fields are redacted
#                        — and a raw-diff scope check proves nothing else
#                        varies.
#   AC3 isolation        : a sentinel directory standing in for the real
#                        RDM_ROOT is left byte-for-byte and mtime-for-mtime
#                        untouched, even when RDM_ROOT is exported into the
#                        environment before fixture_setup runs; a static
#                        grep proves the library never names the real plan
#                        repo or expands $RDM_ROOT.
#   AC4 lint             : a soft, tool-presence-guarded shellcheck/shfmt
#                        pass (CI's own blanket globs already cover both
#                        files once committed).
#
# Run after touching scripts/lib/rdm-plan-fixture.sh.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
export RDM_BIN

if [ ! -x "$RDM_BIN" ]; then
    echo "error: $RDM_BIN not found or not executable — run 'cargo build' first." >&2
    exit 1
fi

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# shellcheck disable=SC1091
. "$SCRIPT_DIR/lib/rdm-plan-fixture.sh"

# file_mtime <path> — portable mtime-in-seconds (BSD stat on macOS, GNU
# stat elsewhere).
file_mtime() {
    stat -f '%m' "$1" 2>/dev/null || stat -c '%Y' "$1"
}

# snapshot_dir <dir> — a deterministic recursive listing of every regular
# file under <dir> paired with its mtime, one per line, sorted by path.
snapshot_dir() {
    dir="$1"
    find "$dir" -type f | LC_ALL=C sort | while IFS= read -r f; do
        printf '%s %s\n' "$f" "$(file_mtime "$f")"
    done
}

# ---------------------------------------------------------------------------
say "AC1: fixture_setup / fixture_code_repo / fixture_teardown contract"
# ---------------------------------------------------------------------------

fixture_setup
[ -d "$FIXTURE_PLAN" ] || fail "FIXTURE_PLAN ($FIXTURE_PLAN) does not exist after fixture_setup"

seed_json="$TMP/ac1-seed.json"
"$RDM_BIN" --root "$FIXTURE_PLAN" tree --format json --project "$FIXTURE_PROJECT" >"$seed_json"

grep -q '"name": "sample-roadmap"' "$seed_json" || fail "seed missing sample-roadmap"
grep -q '"name": "phase-1-seed-one"' "$seed_json" || fail "seed missing phase-1-seed-one"
grep -q '"name": "phase-2-seed-two"' "$seed_json" || fail "seed missing phase-2-seed-two"
grep -q '"name": "phase-3-seed-three"' "$seed_json" || fail "seed missing phase-3-seed-three"
grep -q '"name": "fixture-task-open"' "$seed_json" || fail "seed missing fixture-task-open"
grep -q '"name": "fixture-task-active"' "$seed_json" || fail "seed missing fixture-task-active"
grep -q '"name": "fixture-task-done"' "$seed_json" || fail "seed missing fixture-task-done"
grep -q '"status": "done"' "$seed_json" || fail "seed missing a done status"
grep -q '"status": "in-progress"' "$seed_json" || fail "seed missing an in-progress status"
grep -q '"status": "not-started"' "$seed_json" || fail "seed missing a not-started status"
grep -q '"status": "open"' "$seed_json" || fail "seed missing an open status"
ok "fixture_setup stands up FIXTURE_PLAN with the fixed seed set"

fixture_code_repo
[ -n "${FIXTURE_CODE_REPO:-}" ] || fail "fixture_code_repo did not export FIXTURE_CODE_REPO"
[ "$(git -C "$FIXTURE_CODE_REPO" rev-parse --abbrev-ref HEAD)" = "main" ] ||
    fail "fixture_code_repo is not on branch main"
commit_count=$(git -C "$FIXTURE_CODE_REPO" rev-list --count HEAD)
[ "$commit_count" = "1" ] || fail "fixture_code_repo should have exactly one commit, has $commit_count"
ok "fixture_code_repo stands up a one-commit code repo on main"

removed_root="$FIXTURE_ROOT"
fixture_teardown
[ ! -d "$removed_root" ] || fail "fixture_teardown did not remove FIXTURE_ROOT ($removed_root)"
[ -z "${FIXTURE_ROOT:-}" ] || fail "FIXTURE_ROOT should be unset after fixture_teardown"
[ -z "${FIXTURE_PLAN:-}" ] || fail "FIXTURE_PLAN should be unset after fixture_teardown"
[ -z "${FIXTURE_PROJECT:-}" ] || fail "FIXTURE_PROJECT should be unset after fixture_teardown"
[ -z "${FIXTURE_CODE_REPO:-}" ] || fail "FIXTURE_CODE_REPO should be unset after fixture_teardown"
ok "fixture_teardown removes FIXTURE_ROOT and unsets every exported var"

# fixture_teardown must be a safe no-op with no prior fixture_setup.
fixture_teardown
ok "fixture_teardown is a safe no-op when called again with no prior setup"

# ---------------------------------------------------------------------------
say "AC2: two same-day runs stay byte-identical after redacting documented fields"
# ---------------------------------------------------------------------------

# capture_run <out-dir> — a full fixture_setup, a capture of every JSON
# surface phase 3's golden snapshots care about (tree/list/search plus
# roadmap/phase/task show, which carry the created/completed date fields
# tree/list/search never do), the seed commit SHA, and FIXTURE_PLAN's own
# absolute path — then fixture_teardown.
capture_run() {
    out_dir="$1"
    fixture_setup
    seed_sha=$(git -C "$FIXTURE_PLAN" log -1 --format=%H)
    {
        echo "FIXTURE_PLAN_PATH=$FIXTURE_PLAN"
        echo "SEED_COMMIT_SHA=$seed_sha"
        echo "=== tree ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" tree --format json --project "$FIXTURE_PROJECT"
        echo "=== list ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" list --format json --project "$FIXTURE_PROJECT"
        echo "=== search ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" search "" --format json --project "$FIXTURE_PROJECT"
        echo "=== roadmap-show ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" roadmap show sample-roadmap --format json --project "$FIXTURE_PROJECT"
        echo "=== phase-show-1 ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" phase show 1 --roadmap sample-roadmap --format json --project "$FIXTURE_PROJECT"
        echo "=== phase-show-2 ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" phase show 2 --roadmap sample-roadmap --format json --project "$FIXTURE_PROJECT"
        echo "=== task-show-open ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" task show fixture-task-open --format json --project "$FIXTURE_PROJECT"
        echo "=== task-show-done ==="
        "$RDM_BIN" --root "$FIXTURE_PLAN" task show fixture-task-done --format json --project "$FIXTURE_PROJECT"
    } >"$out_dir/raw.json"
    echo "$FIXTURE_ROOT" >"$out_dir/root.txt"
    fixture_teardown
}

RUN1="$TMP/run1"
RUN2="$TMP/run2"
mkdir -p "$RUN1" "$RUN2"
capture_run "$RUN1"
capture_run "$RUN2"

ROOT1=$(cat "$RUN1/root.txt")
ROOT2=$(cat "$RUN2/root.txt")

# redact <src> <this-run's-FIXTURE_ROOT> <dst> — replace exactly the
# header-documented volatile fields with fixed tokens: (a) the absolute temp
# root, (b) created/completed dates, (c) commit-SHA-shaped fields.
redact() {
    src="$1"
    root="$2"
    dst="$3"
    esc_root=$(printf '%s' "$root" | sed 's/\./\\./g')
    sed \
        -e "s#$esc_root#<TMPROOT>#g" \
        -e 's/"created": "[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]"/"created": "<DATE>"/' \
        -e 's/"completed": "[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]"/"completed": "<DATE>"/' \
        -e 's/^SEED_COMMIT_SHA=.*/SEED_COMMIT_SHA=<SHA>/' \
        -e 's/"commit": "[0-9a-f]*"/"commit": "<SHA>"/' \
        -e 's/"applied_commit": "[0-9a-f]*"/"applied_commit": "<SHA>"/' \
        -e 's/"review_sha": "[0-9a-f]*"/"review_sha": "<SHA>"/' \
        "$src" >"$dst"
}

redact "$RUN1/raw.json" "$ROOT1" "$RUN1/redacted.json"
redact "$RUN2/raw.json" "$ROOT2" "$RUN2/redacted.json"

if ! diff -u "$RUN1/redacted.json" "$RUN2/redacted.json" >"$TMP/redacted.diff"; then
    fail "redacted two-run JSON differs beyond the documented volatile fields:
$(cat "$TMP/redacted.diff")"
fi
ok "redacted two-run JSON is byte-identical"

diff -u "$RUN1/raw.json" "$RUN2/raw.json" >"$TMP/raw.diff" || true

# Every changed content line in the RAW (unredacted) diff must match one of
# the documented volatile-field patterns, or an absolute path under a
# /tmp-style prefix — proving nothing OUTSIDE the documented list differs,
# not merely that the documented fields happen to match.
scope_violations=$(awk '
    /^\+\+\+/ || /^---/ || /^@@/ { next }
    /^[+-]/ {
        line = $0
        if (line ~ /^[+-]FIXTURE_PLAN_PATH=/) next
        if (line ~ /^[+-]SEED_COMMIT_SHA=/) next
        if (line ~ /"created": "[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]"/) next
        if (line ~ /"completed": "[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]"/) next
        if (line ~ /"commit": "[0-9a-f]+"/) next
        if (line ~ /"applied_commit": "[0-9a-f]+"/) next
        if (line ~ /"review_sha": "[0-9a-f]+"/) next
        if (line ~ /tmp\./) next
        if (line ~ /\/tmp\//) next
        print line
    }
' "$TMP/raw.diff")

if [ -n "$scope_violations" ]; then
    fail "raw two-run diff touches an undocumented field:
$scope_violations"
fi
ok "raw two-run diff touches only documented volatile fields (temp paths, dates, seed commit SHA)"

# ---------------------------------------------------------------------------
say "AC3: never reads or writes the real plan repo"
# ---------------------------------------------------------------------------

SENTINEL=$(mktemp -d)
echo "sentinel marker" >"$SENTINEL/marker.txt"
mkdir -p "$SENTINEL/nested"
echo "nested file" >"$SENTINEL/nested/file.txt"

snapshot_dir "$SENTINEL" >"$TMP/sentinel-before.txt"

# Simulate a caller shell that already had a real plan repo exported —
# fixture_setup must unset this before ever invoking rdm.
RDM_ROOT="$SENTINEL"
export RDM_ROOT
fixture_setup
fixture_code_repo
fixture_teardown
unset RDM_ROOT

snapshot_dir "$SENTINEL" >"$TMP/sentinel-after.txt"

if ! diff -u "$TMP/sentinel-before.txt" "$TMP/sentinel-after.txt" >"$TMP/sentinel.diff"; then
    fail "the sentinel dir standing in for the real RDM_ROOT was touched:
$(cat "$TMP/sentinel.diff")"
fi
ok "a full setup -> seed -> code-repo -> teardown cycle leaves a sentinel RDM_ROOT untouched"

# shellcheck disable=SC2016  # a literal pattern for grep, not a shell expansion
if grep -n 'rdm-atlas-repo\|\$RDM_ROOT' "$SCRIPT_DIR/lib/rdm-plan-fixture.sh"; then
    fail "rdm-plan-fixture.sh must never name the real plan repo or expand \$RDM_ROOT"
fi
ok "the library contains no literal real-plan-repo reference and never expands \$RDM_ROOT"

# ---------------------------------------------------------------------------
say "AC4: shellcheck / shfmt (soft local signal; CI's blanket globs are authoritative)"
# ---------------------------------------------------------------------------

LIB_FILE="$SCRIPT_DIR/lib/rdm-plan-fixture.sh"
SELF_FILE="$SCRIPT_DIR/verify-rdm-plan-fixture.sh"

if command -v shellcheck >/dev/null 2>&1; then
    if shellcheck "$LIB_FILE" "$SELF_FILE"; then
        ok "shellcheck clean"
    else
        fail "shellcheck reported findings against $LIB_FILE / $SELF_FILE"
    fi
else
    echo "NOTICE: shellcheck not installed — skipping local lint (CI runs it separately)"
fi

if command -v shfmt >/dev/null 2>&1; then
    if shfmt -d "$LIB_FILE" "$SELF_FILE"; then
        ok "shfmt clean"
    else
        fail "shfmt reported a formatting diff against $LIB_FILE / $SELF_FILE"
    fi
else
    echo "NOTICE: shfmt not installed — skipping local format check (CI runs it separately)"
fi

say "All checks passed."
