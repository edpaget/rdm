#!/bin/sh
# Hermetic drift check for tests/golden/*.json — rdm's committed
# machine-facing `--format json` contract snapshots. Matches the
# scripts/verify-*.sh glob CI already runs, so no separate CI wiring is
# needed once this file exists.
#
#   1. Positive path : re-capture + redact the 20-command inventory into a
#      scratch dir and diff every file against the committed
#      tests/golden/*.json — must be byte-identical.
#   2. AC3 reproducibility : capture a SECOND independent fixture (its own
#      fixture_setup, its own FIXTURE_ROOT) and diff it against the first
#      capture — proves two same-day runs agree with each other, not merely
#      that today's run happens to match already-committed goldens. A
#      follow-up grep proves neither capture leaks a raw, un-redacted temp
#      path.
#   3. Planted-mutation self-test (house style): a scratch COPY of the
#      committed tests/golden/ tree gets one field mutated with sed; the
#      diff step against that mutated copy must fail and its output must
#      carry the re-bless hint. The real committed tests/golden/ is never
#      touched.
#   4. Inverse heal assertion: the SAME clean diff from step 1 (a fresh
#      capture against the real, unmutated committed goldens) is the proof
#      that re-running scripts/capture-golden.sh heals the drift the
#      self-test in step 3 just proved it detects — mutating a scratch copy
#      never touched the real tree, so a fresh capture already matches it.
#   5. README static check (AC4): tests/golden/README.md exists, mentions
#      scripts/capture-golden.sh, and documents the additive-field trap.
#
# Run after touching scripts/lib/golden-capture.sh, scripts/capture-golden.sh,
# or any command the golden set covers.
#
# Requires: cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
export RDM_BIN
GOLDEN_DIR="$REPO_ROOT/tests/golden"

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
# shellcheck disable=SC1091
. "$SCRIPT_DIR/lib/golden-capture.sh"

# Per the fixture library's documented convention: register cleanup for
# both this script's own scratch dir AND any live FIXTURE_ROOT immediately
# after sourcing, so an abort mid-script (this file runs under `set -eu`)
# never leaks a fixture.
trap 'rm -rf "$TMP"; fixture_teardown' EXIT INT HUP TERM

REBLESS_HINT="golden JSON drift detected — if this change is intentional, run scripts/capture-golden.sh to re-bless the goldens and commit the result"

# _diff_against_golden <captured-dir> <golden-dir> — diff every file named in
# GOLDEN_NAMES between the two directories, printing a unified diff for
# every mismatch (not just the first) before returning non-zero if any
# mismatch was found.
_diff_against_golden() {
    captured_dir="$1"
    golden_dir="$2"
    drift=0
    for name in $GOLDEN_NAMES; do
        cap_file="$captured_dir/$name.json"
        gold_file="$golden_dir/$name.json"
        if [ ! -f "$cap_file" ]; then
            echo "MISSING captured file: $cap_file" >&2
            drift=1
            continue
        fi
        if [ ! -f "$gold_file" ]; then
            echo "MISSING golden file: $gold_file" >&2
            drift=1
            continue
        fi
        if ! diff -u "$gold_file" "$cap_file" >"$TMP/diff-$name.txt" 2>&1; then
            echo "--- drift in $name.json ---" >&2
            cat "$TMP/diff-$name.txt" >&2
            drift=1
        fi
    done
    return $drift
}

# ---------------------------------------------------------------------------
say "1. Positive path: fresh capture vs. committed tests/golden/*.json"
# ---------------------------------------------------------------------------

CAPTURE_A="$TMP/capture-a"
golden_capture_all "$CAPTURE_A" || fail "golden_capture_all (capture A) failed"
golden_redact "$CAPTURE_A" || fail "golden_redact (capture A) failed"
fixture_teardown

if ! _diff_against_golden "$CAPTURE_A" "$GOLDEN_DIR"; then
    echo "$REBLESS_HINT" >&2
    fail "committed tests/golden/*.json does not match a fresh capture"
fi
ok "fresh capture matches every committed golden file"

# ---------------------------------------------------------------------------
say "2. AC3: same-day double-capture reproducibility"
# ---------------------------------------------------------------------------

CAPTURE_B="$TMP/capture-b"
golden_capture_all "$CAPTURE_B" || fail "golden_capture_all (capture B) failed"
golden_redact "$CAPTURE_B" || fail "golden_redact (capture B) failed"
fixture_teardown

if ! diff -rq "$CAPTURE_A" "$CAPTURE_B" >"$TMP/double-capture-diff.txt" 2>&1; then
    cat "$TMP/double-capture-diff.txt" >&2
    fail "two independent same-day captures disagree after redaction — reproducibility broken"
fi
ok "two independent fixture_setup calls produce byte-identical redacted output"

# A raw, un-redacted temp path (mktemp's "tmp." leaf-directory naming
# convention) must never survive redaction in either capture.
if grep -rlE '/tmp\.[A-Za-z0-9]+' "$CAPTURE_A" "$CAPTURE_B" >"$TMP/leaked-tmpdir.txt" 2>/dev/null; then
    if [ -s "$TMP/leaked-tmpdir.txt" ]; then
        cat "$TMP/leaked-tmpdir.txt" >&2
        fail "a raw temp-dir path leaked past redaction (see files above) — canonical-vs-raw path mismatch?"
    fi
fi
ok "no raw temp-dir path leaked past redaction in either capture"

# ---------------------------------------------------------------------------
say "3. Planted-mutation self-test: the drift detector actually fires"
# ---------------------------------------------------------------------------

MUTATED="$TMP/mutated-golden"
cp -R "$GOLDEN_DIR" "$MUTATED"
if ! grep -q '"status": "done"' "$MUTATED/phase-show.json"; then
    fail "self-test setup: expected \"status\": \"done\" in a scratch copy of phase-show.json — did the seed fixture or golden shape change?"
fi
sed -i.bak 's/"status": "done"/"status": "reviewed"/' "$MUTATED/phase-show.json"
rm -f "$MUTATED/phase-show.json.bak"

self_test_output=$(_diff_against_golden "$CAPTURE_A" "$MUTATED" 2>&1) && self_test_rc=0 || self_test_rc=$?
if [ "$self_test_rc" -eq 0 ]; then
    fail "self-test: diffing against a deliberately mutated golden copy did NOT fail — the drift detector is vacuous"
fi
case "$self_test_output" in
    *'drift in phase-show.json'*) ;;
    *) fail "self-test: the mutated file (phase-show.json) was not named in the drift output" ;;
esac
ok "drift detector fires on a planted mutation, naming the affected file"

# ---------------------------------------------------------------------------
say "4. Inverse heal assertion"
# ---------------------------------------------------------------------------

# The scratch mutation in step 3 only ever touched $MUTATED, never the real
# committed $GOLDEN_DIR — so step 1's clean diff of a fresh capture against
# the REAL committed goldens already proves that re-running
# scripts/capture-golden.sh (which performs the same capture+redact this
# script just did) restores a clean diff after the drift step 3 detected.
if ! _diff_against_golden "$CAPTURE_A" "$GOLDEN_DIR"; then
    fail "the real committed tests/golden/*.json no longer diffs clean against capture A — the self-test in step 3 leaked into the real tree"
fi
ok "a fresh capture against the real (unmutated) committed goldens is clean — the re-bless path heals what step 3 proved it detects"

# ---------------------------------------------------------------------------
say "5. AC4: README documents the re-bless workflow and the additive-field trap"
# ---------------------------------------------------------------------------

README="$GOLDEN_DIR/README.md"
[ -f "$README" ] || fail "tests/golden/README.md is missing"
grep -q 'scripts/capture-golden.sh' "$README" || fail "tests/golden/README.md does not mention scripts/capture-golden.sh"
grep -qi 'additive' "$README" || fail "tests/golden/README.md does not document the additive-field-still-fails caveat"
ok "tests/golden/README.md documents the re-bless command and the additive-field caveat"

say "All golden-JSON drift checks passed."
