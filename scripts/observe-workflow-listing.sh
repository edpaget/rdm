#!/bin/sh
# Observe the RENDERED skill/slash-command listing and assert the `rdm-wf-`
# engine-prefix contract against it.
#
# Why this exists as its own script rather than a section of
# `verify-workflow-review.sh`: the listing is rendered by the Claude Code
# client from `.claude/`, not by anything in this repo, so the only way to
# check it is to ask a real client. That makes the live half NON-HERMETIC — it
# needs the `claude` CLI, credentials, and a network round trip — which is
# exactly why it must not sit inside a harness that CI runs. Everything the
# repo CAN check hermetically (engine filenames, `meta.name`-equals-stem
# parity, engine/skill name disjointness) is already gated by
# `verify-workflow-review.sh` § 2d. This script closes the remaining gap: that
# what the client actually RENDERS matches what the tree declares.
#
# Two halves, deliberately separable:
#
#   --self-test-only   HERMETIC. Runs the assertion logic against a pinned
#                      PRE-rename listing and requires it to turn red. Proves
#                      the assertions discriminate rather than passing on
#                      anything. Needs no `claude`, no network, no credentials,
#                      so `verify-workflow-review.sh` § 2d wires it in and CI
#                      runs it on every commit.
#
#   (default)          LIVE. Runs the self-test, then captures the real listing
#                      from a `claude -p` process rooted at this repo and
#                      asserts the contract against it. Run this deliberately
#                      when changing engine names; paste the emitted capture as
#                      the evidence.
#
# The expected names are DERIVED from the tree (each engine's `meta.name`, each
# skill's frontmatter `name`), never hardcoded here — so this script asserts
# "the listing agrees with the tree", which is the actual contract, and cannot
# drift into asserting a stale hand-copied list.
#
# Requires (live half only): the `claude` CLI on PATH, authenticated.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
WF_DIR="$REPO_ROOT/.claude/workflows"
SKILLS_DIR="$REPO_ROOT/.claude/skills"

SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT INT TERM

say() { printf '\n=== %s ===\n' "$1"; }
pass() { printf '  ok: %s\n' "$1"; }
fail() {
    printf '\nFAIL: %s\n' "$1" >&2
    exit 1
}

SELF_TEST_ONLY=0
[ "${1:-}" = "--self-test-only" ] && SELF_TEST_ONLY=1

# --- expected names, derived from the tree ----------------------------------

# Every engine's meta.name. These are the entries an engine contributes.
declared_engine_names() {
    for engine in "$WF_DIR"/*.js; do
        [ -f "$engine" ] || continue
        sed -n "s/^  name: '\(.*\)',$/\1/p" "$engine" | head -1
    done
}

# Every skill's frontmatter name. These are the front doors that must NOT move.
declared_skill_names() {
    for skill in "$SKILLS_DIR"/*/SKILL.md; do
        [ -f "$skill" ] || continue
        sed -n 's/^name: *\(.*\)$/\1/p' "$skill" | head -1
    done
}

# The bare pre-rename form of each prefixed engine — the names that must be
# ABSENT from the listing. Derived by stripping the prefix off the declared
# names, so it cannot go stale either. `spike-agent-type` is an exempt spike
# artifact: it carries no prefix, contributes no bare form, and is skipped.
bare_engine_names() {
    declared_engine_names | sed -n 's/^rdm-wf-//p'
}

# Every assertion below is of the form "each derived name must (not) appear".
# If a derivation yields NOTHING — wrong repo root, a moved directory, a
# `meta.name` format change that stops matching the sed — every one of those
# assertions passes over an empty set and the script reports success while
# checking nothing. Fail loudly instead.
[ -d "$WF_DIR" ] || fail "no workflows directory at $WF_DIR — refusing to assert over an empty derived name set"
[ -d "$SKILLS_DIR" ] || fail "no skills directory at $SKILLS_DIR — refusing to assert over an empty derived name set"
[ "$(declared_engine_names | grep -c .)" -gt 0 ] ||
    fail "derived ZERO engine names from $WF_DIR — every engine assertion would pass vacuously"
[ "$(bare_engine_names | grep -c .)" -gt 0 ] ||
    fail "derived ZERO rdm-wf- prefixed engines from $WF_DIR — the bare-name assertion would pass vacuously"
[ "$(declared_skill_names | grep -c .)" -gt 0 ] ||
    fail "derived ZERO skill names from $SKILLS_DIR — the front-door assertion would pass vacuously"

# --- the assertion under test ------------------------------------------------

# assert_listing <listing-file> — returns 0 if the listing satisfies the
# contract, 1 otherwise. Diagnostics go to stderr.
assert_listing() {
    listing=$1
    bad=0

    # Sanity: a malformed capture (model preamble, an error page, an empty
    # answer) must not be mistaken for a clean listing. Every front door is
    # present in any healthy listing, so require at least one before trusting
    # any ABSENCE conclusion drawn below.
    if ! grep -qx 'rdm-do' "$listing"; then
        echo "  capture does not contain the 'rdm-do' front door — listing looks malformed, not clean" >&2
        return 1
    fi

    # 1. Every engine the tree declares is rendered.
    declared_engine_names | while read -r name; do
        [ -n "$name" ] || continue
        grep -qx "$name" "$listing" || echo "  MISSING engine entry: $name"
    done >"$SCRATCH/missing-engines"
    if [ -s "$SCRATCH/missing-engines" ]; then
        cat "$SCRATCH/missing-engines" >&2
        bad=1
    fi

    # 2. No bare pre-rename engine name survives. This is the half that a
    #    partially-completed sweep fails.
    bare_engine_names | while read -r name; do
        [ -n "$name" ] || continue
        grep -qx "$name" "$listing" && echo "  BARE pre-rename engine entry still rendered: $name"
    done >"$SCRATCH/bare-engines"
    if [ -s "$SCRATCH/bare-engines" ]; then
        cat "$SCRATCH/bare-engines" >&2
        bad=1
    fi

    # 3. Every `rdm-*` front door is rendered under its ORIGINAL name. This is
    #    the half that a bare-word substitution fails.
    declared_skill_names | while read -r name; do
        [ -n "$name" ] || continue
        grep -qx "$name" "$listing" || echo "  MISSING front-door skill entry: $name"
    done >"$SCRATCH/missing-skills"
    if [ -s "$SCRATCH/missing-skills" ]; then
        cat "$SCRATCH/missing-skills" >&2
        bad=1
    fi

    # 4. No double-prefixed entry — the specific corruption an unanchored
    #    s/<name>/rdm-wf-<name>/ produces in a name that already starts `rdm-`.
    #    The two patterns are COMPOSED from a prefix variable rather than
    #    spelled out, so this script's own source can never trip the repo-wide
    #    version of the same check (verify-agent-config-distribution.sh § 5g),
    #    which greps every tracked file for exactly these two shapes.
    ns=rdm-
    if grep -nE "${ns}${ns}|${ns}wf-${ns}" "$listing" >&2; then
        echo "  double-prefixed listing entry (see above)" >&2
        bad=1
    fi

    return "$bad"
}

# --- self-test (hermetic) ----------------------------------------------------
#
# A pinned capture of the PRE-rename listing, taken from a real `claude -p`
# rooted at `main` while this phase was implemented. Feeding it to
# assert_listing must fail: the six prefixed entries are missing and the six
# bare ones are present. If this ever passes, the assertions above have gone
# vacuous and the live half proves nothing.
say "self-test: the assertions must reject a pre-rename listing"

cat >"$SCRATCH/pre-rename-listing" <<'EOF'
rdm-autopilot
rdm-backlog
rdm-dispatch-phase
rdm-do
rdm-document
rdm-estimate
rdm-land
rdm-plan-review
rdm-review
rdm-revise
rdm-roadmap
backlog
dispatch-phase
document
estimate
plan-review
review-refute-fix
spike-agent-type
EOF

if assert_listing "$SCRATCH/pre-rename-listing" 2>/dev/null; then
    fail "self-test: the pinned PRE-rename listing was accepted — assert_listing is vacuous"
fi
pass "a pre-rename listing is correctly rejected"

# The mirror image: a listing built from exactly what the tree declares must be
# ACCEPTED. Without this, a check that rejects everything would also pass the
# test above.
{
    declared_skill_names
    declared_engine_names
} >"$SCRATCH/synthetic-listing"
assert_listing "$SCRATCH/synthetic-listing" >/dev/null 2>&1 ||
    fail "self-test: a listing matching the tree exactly was REJECTED — assert_listing rejects everything and proves nothing"
pass "a listing matching the tree is correctly accepted"

if [ "$SELF_TEST_ONLY" -eq 1 ]; then
    say "self-test only — skipping the live capture"
    printf '\nAll good (hermetic half).\n'
    exit 0
fi

# --- live capture (non-hermetic) ---------------------------------------------
say "live: capturing the rendered listing from a claude process rooted at this repo"

command -v claude >/dev/null 2>&1 ||
    fail "the \`claude\` CLI is not on PATH — the live half needs a real client to render the listing.
  Run with --self-test-only to exercise just the hermetic assertion check."

PROMPT='List, one per line and nothing else, the exact name of every entry available to your Skill tool (both skills and workflows). No commentary, no bullets, no grouping - just the bare names.'

# Rooted at REPO_ROOT: the listing is resolved by walking up from the cwd, so
# the cwd is what selects which tree's engines are rendered.
(cd "$REPO_ROOT" && printf '%s\n' "$PROMPT" | claude -p --output-format text) \
    >"$SCRATCH/live-listing-raw" 2>"$SCRATCH/live-listing-err" ||
    fail "\`claude -p\` failed. stderr:
$(cat "$SCRATCH/live-listing-err")"

# Normalize: strip surrounding whitespace and drop blank lines, so a stray
# indent or trailing space cannot fail an exact-line match.
sed 's/^[[:space:]]*//; s/[[:space:]]*$//' "$SCRATCH/live-listing-raw" |
    grep -v '^$' >"$SCRATCH/live-listing"

printf '\n--- rendered listing (evidence; paste this into the commit body) ---\n'
cat "$SCRATCH/live-listing"
printf -- '--- end rendered listing ---\n\n'

assert_listing "$SCRATCH/live-listing" ||
    fail "the RENDERED listing does not satisfy the rdm-wf- engine-prefix contract (see diagnostics above).
  Note: a Claude Code client only watches directories that existed at ITS session start, so a
  freshly-renamed tree can render stale names in a long-running session. This script spawns a
  NEW client each run, so it does not have that problem — a failure here is a real failure."
pass "every declared engine renders under its rdm-wf- name"
pass "no bare pre-rename engine name is rendered"
pass "every rdm-* front door renders under its original name"
pass "no double-prefixed entry is rendered"

printf '\nAll good.\n'
