#!/bin/sh
# Observe the RENDERED skill/slash-command listing and assert the `rdm-wf-`
# engine-prefix contract against it.
#
# Why this exists as its own script rather than a section of
# `verify-workflow-review.sh`: the listing is rendered by the Claude Code
# client from `.claude/`, not by anything in this repo, so the only way to
# check it is to ask a real client. That makes this NON-HERMETIC — it needs
# the `claude` CLI, credentials, and a network round trip — which is exactly
# why it must not sit inside a harness that CI runs. Run this deliberately
# when changing engine names; paste the emitted capture as the evidence.
#
# The expected names are DERIVED from the tree (each engine's `meta.name`, each
# skill's frontmatter `name`), never hardcoded here — so this script asserts
# "the listing agrees with the tree", which is the actual contract, and cannot
# drift into asserting a stale hand-copied list.
#
# A prior hermetic `--self-test-only` half (verified the assertion logic
# against a pinned PRE-rename listing) was retired by the operator amendment
# to the `retire-static-grep-harnesses` plan (2026-09-23): it exercised no
# `claude` CLI, no network, nothing real — a hermetic stand-in built entirely
# from source-derived text, which is exactly the grep-based-static-check shape
# the amendment removes everywhere else. `verify-workflow-review.sh` § 2d no
# longer invokes this script; § 2d now gates only the `rdm-wf-*.js` engine
# filename set and the shipped template copies' byte-identity to the local
# engines (its meta.name-parity and engine/skill-disjointness checks were
# retired in the same pass). Only the live capture below — checked against
# what a real client actually renders — remains, and it is not CI-run.
#
# Requires: the `claude` CLI on PATH, authenticated.

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
# names, so it cannot go stale either. (The `spike-agent-type` exemption is gone
# with the file: it was deleted along with the mechanical lane it probed.)
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

# --- live capture (non-hermetic) ---------------------------------------------
say "live: capturing the rendered listing from a claude process rooted at this repo"

command -v claude >/dev/null 2>&1 ||
    fail "the \`claude\` CLI is not on PATH — this needs a real client to render the listing."

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
