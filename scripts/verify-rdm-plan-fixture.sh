#!/bin/sh
# Hermetic self-test for scripts/lib/rdm-plan-fixture.sh.
#
# No consumer of the fixture library has landed yet (golden-JSON snapshots
# and verify-plugin-loop.sh are later phases of the editor-integration
# roadmap), so this harness proves the library's four contracts directly:
#
#   AC1 setup/teardown : fixture_setup/fixture_teardown/fixture_code_repo
#                        stand up and tear down an isolated plan (+ code)
#                        repo with the documented seed set, and the three
#                        guard clauses (RDM_BIN missing/non-executable, a
#                        re-entrant fixture_setup, fixture_code_repo called
#                        out of order) fail loudly without clobbering state
#                        or leaking a teardown-forced HOME/XDG_* unset.
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

# shellcheck disable=SC1091
. "$SCRIPT_DIR/lib/rdm-plan-fixture.sh"

# Per the library's documented convention: register cleanup for both this
# script's own scratch dir AND any live FIXTURE_ROOT immediately after
# sourcing, so an abort mid-script (this file runs under `set -eu`) never
# leaks a fixture instead of merely relying on every call site remembering
# to pair fixture_setup with fixture_teardown.
trap 'rm -rf "$TMP"; fixture_teardown' EXIT INT HUP TERM

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
say "AC1b: guard clauses (RDM_BIN check, re-entrant setup, code-repo ordering, HOME/XDG preservation)"
# ---------------------------------------------------------------------------

# A fixture_teardown call that never ran a completed fixture_setup in this
# shell (the exact call sequence just exercised above) must leave
# HOME/XDG_* completely untouched, not force them unset.
pre_home="${HOME:-<unset>}"
pre_xdg_config="${XDG_CONFIG_HOME:-<unset>}"
pre_xdg_data="${XDG_DATA_HOME:-<unset>}"
pre_xdg_state="${XDG_STATE_HOME:-<unset>}"
fixture_teardown
[ "${HOME:-<unset>}" = "$pre_home" ] || fail "fixture_teardown with no prior setup changed HOME"
[ "${XDG_CONFIG_HOME:-<unset>}" = "$pre_xdg_config" ] || fail "fixture_teardown with no prior setup changed XDG_CONFIG_HOME"
[ "${XDG_DATA_HOME:-<unset>}" = "$pre_xdg_data" ] || fail "fixture_teardown with no prior setup changed XDG_DATA_HOME"
[ "${XDG_STATE_HOME:-<unset>}" = "$pre_xdg_state" ] || fail "fixture_teardown with no prior setup changed XDG_STATE_HOME"
ok "fixture_teardown with no prior setup leaves HOME/XDG_* completely untouched"

# RDM_BIN unset must fail fixture_setup loudly (return 1, actionable
# message) rather than proceeding.
SAVED_RDM_BIN="$RDM_BIN"
unset RDM_BIN
if fixture_setup 2>"$TMP/rdm-bin-unset.err"; then
    RDM_BIN="$SAVED_RDM_BIN"
    export RDM_BIN
    fixture_teardown
    fail "fixture_setup should fail when RDM_BIN is unset"
fi
grep -q 'RDM_BIN is not set' "$TMP/rdm-bin-unset.err" ||
    fail "fixture_setup's RDM_BIN-unset error message is not actionable: $(cat "$TMP/rdm-bin-unset.err")"
ok "fixture_setup fails loudly when RDM_BIN is unset"

# RDM_BIN pointed at a non-executable file must fail the same way.
RDM_BIN="$TMP/not-executable"
: >"$RDM_BIN"
export RDM_BIN
if fixture_setup 2>"$TMP/rdm-bin-noexec.err"; then
    RDM_BIN="$SAVED_RDM_BIN"
    export RDM_BIN
    fixture_teardown
    fail "fixture_setup should fail when RDM_BIN is not executable"
fi
grep -q 'RDM_BIN is not set' "$TMP/rdm-bin-noexec.err" ||
    fail "fixture_setup's RDM_BIN-non-executable error message is not actionable: $(cat "$TMP/rdm-bin-noexec.err")"
ok "fixture_setup fails loudly when RDM_BIN is not executable"

RDM_BIN="$SAVED_RDM_BIN"
export RDM_BIN

# fixture_code_repo called before any fixture_setup must fail cleanly
# rather than mangling a path under an empty FIXTURE_ROOT.
if fixture_code_repo 2>"$TMP/code-repo-order.err"; then
    fail "fixture_code_repo should fail when called before fixture_setup"
fi
grep -q 'call fixture_setup before fixture_code_repo' "$TMP/code-repo-order.err" ||
    fail "fixture_code_repo's ordering error message is not actionable: $(cat "$TMP/code-repo-order.err")"
[ -z "${FIXTURE_CODE_REPO:-}" ] || fail "fixture_code_repo should not export FIXTURE_CODE_REPO when it fails"
ok "fixture_code_repo fails cleanly when called before fixture_setup"

# fixture_setup called twice without an intervening fixture_teardown must
# fail without clobbering the first call's FIXTURE_ROOT/HOME state.
fixture_setup
first_root="$FIXTURE_ROOT"
first_home="$HOME"
if fixture_setup 2>"$TMP/reentrant.err"; then
    fixture_teardown
    fail "fixture_setup should fail when called again without an intervening fixture_teardown"
fi
grep -q 'already set and still exists' "$TMP/reentrant.err" ||
    fail "fixture_setup's re-entrant-setup error message is not actionable: $(cat "$TMP/reentrant.err")"
[ "$FIXTURE_ROOT" = "$first_root" ] || fail "a rejected re-entrant fixture_setup call must not clobber FIXTURE_ROOT"
[ "$HOME" = "$first_home" ] || fail "a rejected re-entrant fixture_setup call must not clobber HOME"
[ -d "$first_root" ] || fail "the first fixture_setup's FIXTURE_ROOT must still exist after a rejected re-entrant call"
fixture_teardown
ok "fixture_setup fails without clobbering state when called twice without an intervening teardown"

# ---------------------------------------------------------------------------
say "AC1c: fixture_teardown restores HOME/XDG_* to their exact captured original values"
# ---------------------------------------------------------------------------

# A hardcoded or swapped restore (e.g. restoring HOME from the saved
# XDG_CONFIG_HOME, or any other value-mixup bug) must be caught: set
# HOME/XDG_* to known, distinct sentinel values BEFORE fixture_setup runs,
# run a full setup -> teardown cycle, and assert the ORIGINAL sentinel
# values come back exactly — not merely that they are non-empty.
_orig_home="${HOME:-<unset>}"
_orig_xdg_config="${XDG_CONFIG_HOME:-<unset>}"
_orig_xdg_data="${XDG_DATA_HOME:-<unset>}"
_orig_xdg_state="${XDG_STATE_HOME:-<unset>}"

HOME="$TMP/sentinel-home"
XDG_CONFIG_HOME="$TMP/sentinel-xdg-config"
XDG_DATA_HOME="$TMP/sentinel-xdg-data"
XDG_STATE_HOME="$TMP/sentinel-xdg-state"
export HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"

fixture_setup
[ "$HOME" = "$FIXTURE_ROOT/home" ] || fail "fixture_setup did not reroot HOME under FIXTURE_ROOT"
fixture_teardown

[ "$HOME" = "$TMP/sentinel-home" ] ||
    fail "fixture_teardown restored HOME to the wrong value (expected $TMP/sentinel-home, got ${HOME:-<unset>})"
[ "$XDG_CONFIG_HOME" = "$TMP/sentinel-xdg-config" ] ||
    fail "fixture_teardown restored XDG_CONFIG_HOME to the wrong value (expected $TMP/sentinel-xdg-config, got ${XDG_CONFIG_HOME:-<unset>})"
[ "$XDG_DATA_HOME" = "$TMP/sentinel-xdg-data" ] ||
    fail "fixture_teardown restored XDG_DATA_HOME to the wrong value (expected $TMP/sentinel-xdg-data, got ${XDG_DATA_HOME:-<unset>})"
[ "$XDG_STATE_HOME" = "$TMP/sentinel-xdg-state" ] ||
    fail "fixture_teardown restored XDG_STATE_HOME to the wrong value (expected $TMP/sentinel-xdg-state, got ${XDG_STATE_HOME:-<unset>})"
ok "fixture_teardown restores HOME/XDG_* to their exact pre-fixture sentinel values"

# Put the real values back before continuing.
if [ "$_orig_home" = "<unset>" ]; then unset HOME; else
    HOME="$_orig_home"
    export HOME
fi
if [ "$_orig_xdg_config" = "<unset>" ]; then unset XDG_CONFIG_HOME; else
    XDG_CONFIG_HOME="$_orig_xdg_config"
    export XDG_CONFIG_HOME
fi
if [ "$_orig_xdg_data" = "<unset>" ]; then unset XDG_DATA_HOME; else
    XDG_DATA_HOME="$_orig_xdg_data"
    export XDG_DATA_HOME
fi
if [ "$_orig_xdg_state" = "<unset>" ]; then unset XDG_STATE_HOME; else
    XDG_STATE_HOME="$_orig_xdg_state"
    export XDG_STATE_HOME
fi

# ---------------------------------------------------------------------------
say "AC1d: an ambient RDM_*-prefixed env var never leaks into the fixture"
# ---------------------------------------------------------------------------

# rdm-cli reads several RDM_*-prefixed vars beyond RDM_ROOT/RDM_PROJECT —
# notably RDM_PLAN_REVIEW (auto-stamps needs-plan-review on every created
# item) and RDM_REVIEW_AUTHOR. Export a representative pair before
# fixture_setup, exactly as a developer shell running this repo's own
# plan_review-enabled dogfood config might, and prove fixture_setup's
# wildcard RDM_* sweep clears them before the first rdm invocation.
RDM_PLAN_REVIEW=true
RDM_REVIEW_AUTHOR=ambient-author
export RDM_PLAN_REVIEW RDM_REVIEW_AUTHOR

fixture_setup

[ -z "${RDM_PLAN_REVIEW:-}" ] || fail "fixture_setup left RDM_PLAN_REVIEW set to ${RDM_PLAN_REVIEW}"
[ -z "${RDM_REVIEW_AUTHOR:-}" ] || fail "fixture_setup left RDM_REVIEW_AUTHOR set to ${RDM_REVIEW_AUTHOR}"

leak_json="$TMP/ac1d-seed.json"
"$RDM_BIN" --root "$FIXTURE_PLAN" tree --format json --project "$FIXTURE_PROJECT" >"$leak_json"
if grep -q 'needs-plan-review' "$leak_json"; then
    fail "an ambient RDM_PLAN_REVIEW leaked into the seed: seeded items are stamped needs-plan-review"
fi

fixture_teardown
ok "fixture_setup's RDM_* sweep clears RDM_PLAN_REVIEW/RDM_REVIEW_AUTHOR before the seed runs"

# ---------------------------------------------------------------------------
say "AC1e: fixture_setup fails loudly (non-zero, actionable message) when a seed command fails"
# ---------------------------------------------------------------------------

# A wrapper around the real rdm binary that fails one specific seeding
# invocation (the very first one _fixture_seed makes) and otherwise
# delegates untouched, proving fixture_setup no longer swallows a mid-seed
# failure and reports success anyway (it previously returned whatever the
# LAST command — `rdm commit` — happened to exit with).
FAKE_RDM="$TMP/fake-rdm-seed-failure.sh"
cat >"$FAKE_RDM" <<EOF
#!/bin/sh
case " \$* " in
    *" roadmap create sample-roadmap "*)
        echo "fake-rdm: forced failure" >&2
        exit 2
        ;;
esac
exec "$RDM_BIN" "\$@"
EOF
chmod +x "$FAKE_RDM"

REAL_RDM_BIN="$RDM_BIN"
RDM_BIN="$FAKE_RDM"
export RDM_BIN

if fixture_setup 2>"$TMP/seed-fail.err"; then
    RDM_BIN="$REAL_RDM_BIN"
    export RDM_BIN
    fixture_teardown
    fail "fixture_setup should fail (non-zero) when a seed command fails, not report success"
fi
grep -q "^_fixture_seed: 'rdm roadmap create sample-roadmap " "$TMP/seed-fail.err" ||
    fail "fixture_setup's seed-failure error message is not actionable: $(cat "$TMP/seed-fail.err")"
# The failed run must still leave FIXTURE_ROOT in place for the caller's
# own fixture_teardown to clean up (per the library's documented
# failure-path-cleanup contract), not unset it out from under a caller
# that relies on it for teardown.
[ -n "${FIXTURE_ROOT:-}" ] || fail "a failed fixture_setup unset FIXTURE_ROOT, breaking the caller's teardown"
[ -d "$FIXTURE_ROOT" ] || fail "a failed fixture_setup's FIXTURE_ROOT ($FIXTURE_ROOT) does not exist"

fixture_teardown
RDM_BIN="$REAL_RDM_BIN"
export RDM_BIN
ok "fixture_setup fails loudly and names the failing command when a seed command fails"

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
