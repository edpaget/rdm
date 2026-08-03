#!/bin/sh
# Observe a REAL offline install of rdm's Claude Code plugin, end to end:
# `plugin validate --strict` -> `marketplace add` -> `install rdm@rdm` ->
# assert the installed tree.
#
# Why this exists as its own script rather than a section of
# scripts/verify-plugin-install.sh: this half needs the `claude` CLI, which is
# NOT a CI dependency, and the plugin-distribution roadmap deliberately does
# not add one (adding `claude` to .mise.toml or .github/workflows/ci.yml is an
# explicit non-goal). Per the convention scripts/observe-workflow-listing.sh
# establishes, a client-dependent check is named `observe-*` so it falls
# OUTSIDE ci.yml's `for f in scripts/verify-*.sh` glob.
#
#   *** THIS SCRIPT CARRIES NO CI COVERAGE. It is developer-run. ***
#
# Everything the repo CAN check hermetically — that plugins/rdm/ matches
# generator output modulo the manifest version, that the marketplace entry's
# `source` resolves, that the workflow bytes and the 11-skill inventory are
# intact — is gated unconditionally by scripts/verify-plugin-install.sh.
# This script closes the remaining gap: that the packaged artifact actually
# INSTALLS, offline, and that what lands on disk is what we emitted.
#
# Two verified gaps in the `claude` CLI shape this script:
#
#   1. `claude plugin validate --strict` FALSE-PASSES a marketplace whose
#      plugin `source` points at a nonexistent directory (exit 0 on
#      "source": "./does-not-exist"). Source resolution is therefore owned by
#      verify-plugin-install.sh, not delegated to the CLI.
#
#   2. `claude plugin details` reports Skills / Agents / Hooks / MCP servers /
#      LSP servers and has NO Workflows category at all (confirmed against the
#      official claude-security plugin, which ships workflows and shows none).
#      Workflows ARE fully supported by the runtime; the inventory simply does
#      not enumerate them. So workflow presence is asserted ON THE FILESYSTEM
#      below, never via `plugin details`.
#
# Isolation: both CLAUDE_CONFIG_DIR and the marketplace copy are mktemp'd and
# trap-cleaned, and the run positively proves the invoking user's real
# ~/.claude config is byte-unchanged.
#
# Exit codes:
#   0  pass
#   1  fail
#   2  NOTICE — `claude` is absent from PATH, so nothing was observed. This is
#      deliberately distinct from 0: a skip is not a pass.
#
# Requires: a cargo-built rdm at target/debug/rdm, and `claude` on PATH.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
# Multi-line diagnostic: $1 is the headline, every later argument is printed on
# its own indented continuation line. printf expands backslash escapes only in
# its FORMAT string, never in %s data, so a literal "\n" embedded in a fail()
# message would render as the two characters \n rather than a line break. Use
# this helper whenever a failure carries an expected-vs-actual payload.
fail_lines() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$1" >&2
    shift
    for _line in "$@"; do
        printf '  %s\n' "$_line" >&2
    done
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

# --- 0. missing-claude notice (must precede any setup) ---------------------
if ! command -v claude >/dev/null 2>&1; then
    printf '\n\033[1;33m[NOTICE]\033[0m claude was not found on PATH — the real-install observation was SKIPPED.\n' >&2
    printf '          This is NOT a pass. Install the Claude Code CLI and re-run to observe.\n' >&2
    printf '          The hermetic half (scripts/verify-plugin-install.sh) covers everything CI gates.\n' >&2
    exit 2
fi

[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

# Clear rdm-related env vars. `--project`-sensitivity is load-bearing: the
# emitted tree must carry the generic <PROJECT> placeholder, matching the
# checked-in one.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

CFG=$(mktemp -d)
MKT=$(mktemp -d)
trap 'rm -rf "$CFG" "$MKT"' EXIT INT HUP TERM

# Exported UNCONDITIONALLY — never inherited, never falling back to
# $HOME/.claude on any path.
CLAUDE_CONFIG_DIR="$CFG"
export CLAUDE_CONFIG_DIR

# --- helpers ---------------------------------------------------------------

# Every `claude` invocation goes through this, so config isolation is
# re-asserted at each call site rather than assumed from the export above.
run_claude() {
    [ "${CLAUDE_CONFIG_DIR:-}" = "$CFG" ] ||
        fail "CLAUDE_CONFIG_DIR is '${CLAUDE_CONFIG_DIR:-<unset>}', expected '$CFG' — refusing to invoke claude against a non-isolated config"
    claude "$@"
}

dir_names() {
    find "$1" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; 2>/dev/null | sort | tr '\n' ' '
}

file_names() {
    find "$1" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; 2>/dev/null | sort | tr '\n' ' '
}

word_count() {
    # shellcheck disable=SC2086  # deliberate word-splitting of a name list
    set -- $1
    printf '%s\n' "$#"
}

json_string() {
    sed -n "s/^ *\"$2\": \"\\([^\"]*\\)\".*\$/\\1/p" "$1" | head -1
}

# Bare (unquoted) JSON scalar, e.g. `"enabled": true,`. Kept to BRE so it
# behaves identically under BSD and GNU sed — `\|` alternation is not portable.
json_bool() {
    sed -n "s/^ *\"$2\": *\\([a-z][a-z]*\\).*\$/\\1/p" "$1" | head -1
}

# The workspace crate version, read from Cargo.toml. NOTE: `rdm --version`
# does not exist (clap rejects it), so this is the source of truth. Never
# hardcode a version here.
crate_version() {
    awk '
        /^\[workspace\.package\]/ { f = 1; next }
        f && /^\[/               { exit }
        f && /^version *=/       { gsub(/[" ]/, "", $3); print $3; exit }
    ' "$REPO_ROOT/Cargo.toml"
}

# Snapshots ONLY the real-config files a plugin install would touch, plus the
# presence/absence of the two rdm-named directories an install would create.
# Deliberately NOT a whole-tree checksum of ~/.claude: a concurrently running
# claude session writes history/projects/statsig there, which would make this
# assertion flaky rather than meaningful.
snapshot_real_config() {
    out="$1"
    : >"$out"
    for f in \
        "$HOME/.claude/settings.json" \
        "$HOME/.claude/plugins/config.json" \
        "$HOME/.claude/plugins/installed_plugins.json" \
        "$HOME/.claude/plugins/known_marketplaces.json"; do
        if [ -f "$f" ]; then
            printf 'FILE %s %s\n' "$(cksum <"$f")" "$f" >>"$out"
        else
            printf 'ABSENT-FILE %s\n' "$f" >>"$out"
        fi
    done
    for d in \
        "$HOME/.claude/plugins/cache/rdm" \
        "$HOME/.claude/plugins/marketplaces/rdm"; do
        if [ -d "$d" ]; then
            printf 'PRESENT-DIR %s\n' "$d" >>"$out"
        else
            printf 'ABSENT-DIR %s\n' "$d" >>"$out"
        fi
    done
}

CRATE_VERSION=$(crate_version)
[ -n "$CRATE_VERSION" ] || fail "could not read [workspace.package] version from $REPO_ROOT/Cargo.toml"

# --- 1. snapshot the real config BEFORE anything -------------------------
say "1. Snapshotting the invoking user's real ~/.claude config (before)"
snapshot_real_config "$MKT/.real-config-before"
sed 's/^/    /' "$MKT/.real-config-before"
pass "before-snapshot captured"

say "1b. Isolation: CLAUDE_CONFIG_DIR=$CFG (mktemp'd, trap-cleaned)"
pass "config isolated"

# --- 2. build the temp marketplace ----------------------------------------
say "2. Building a temp marketplace at $MKT"
mkdir -p "$MKT/.claude-plugin"
cp "$REPO_ROOT/.claude-plugin/marketplace.json" "$MKT/.claude-plugin/marketplace.json"
# The plugin tree under the temp marketplace is a FRESH emission rather than a
# copy of plugins/rdm. Rationale: the install asserts `plugin list`'s version
# equals the crate version, and prepare-release.yml bumps Cargo.toml without
# regenerating, so the committed manifest is legitimately stale right after a
# release. Nothing is lost — verify-plugin-install.sh separately proves the
# committed tree equals fresh output modulo exactly that version field.
"$RDM_BIN" agent-config claude --plugin --out "$MKT/plugins/rdm" >/dev/null
EMITTED="$MKT/plugins/rdm"
pass "marketplace manifest copied and a fresh plugin tree emitted into $EMITTED"

# --- 3. validate --strict, both manifests ---------------------------------
say "3. claude plugin validate --strict on the COMMITTED plugin tree (read-only, in place)"
run_claude plugin validate "$REPO_ROOT/plugins/rdm" --strict
pass "committed plugin manifest validates under --strict"

say "3b. claude plugin validate --strict on the temp marketplace"
run_claude plugin validate "$MKT" --strict
pass "marketplace manifest validates under --strict"

# --- 4. marketplace add + install (offline, no auth) ----------------------
say "4. claude plugin marketplace add $MKT"
run_claude plugin marketplace add "$MKT"
pass "marketplace added into the isolated config"

say "4b. claude plugin install rdm@rdm"
run_claude plugin install rdm@rdm
pass "plugin installed"

# --- 5. assert the install ------------------------------------------------
say "5. claude plugin list --json: exactly one entry, enabled, at the crate version"
run_claude plugin list --json >"$MKT/.list.json"
sed 's/^/    /' "$MKT/.list.json"
ENTRY_COUNT=$(grep -c '"id":' "$MKT/.list.json" || true)
[ "$ENTRY_COUNT" = "1" ] ||
    fail "expected exactly 1 installed plugin, found $ENTRY_COUNT"
LIST_ID=$(json_string "$MKT/.list.json" id)
[ "$LIST_ID" = "rdm@rdm" ] || fail "installed plugin id is '$LIST_ID', expected 'rdm@rdm'"
LIST_ENABLED=$(json_bool "$MKT/.list.json" enabled)
[ "$LIST_ENABLED" = "true" ] || fail "installed plugin is not enabled (enabled=$LIST_ENABLED)"
LIST_VERSION=$(json_string "$MKT/.list.json" version)
[ "$LIST_VERSION" = "$CRATE_VERSION" ] ||
    fail "installed plugin version is '$LIST_VERSION', expected the crate version '$CRATE_VERSION'"
pass "one entry: id=rdm@rdm, enabled=true, version=$CRATE_VERSION"

# installPath is read OUT of the JSON, never reconstructed — the cache path
# embeds the version, so reconstructing it would break on every bump.
INSTALL_PATH=$(json_string "$MKT/.list.json" installPath)
[ -n "$INSTALL_PATH" ] || fail "could not read installPath from plugin list --json"
[ -d "$INSTALL_PATH" ] || fail "installPath does not exist: $INSTALL_PATH"
case "$INSTALL_PATH" in
    "$CFG"/*) : ;;
    *) fail "installPath '$INSTALL_PATH' is outside the isolated config dir '$CFG' — isolation failed" ;;
esac
pass "installPath resolves inside the isolated config: $INSTALL_PATH"

say "5b. Installed skill inventory equals the emitted inventory"
EXPECTED_SKILLS=$(dir_names "$EMITTED/skills")
INSTALLED_SKILLS=$(dir_names "$INSTALL_PATH/skills")
[ "$(word_count "$EXPECTED_SKILLS")" -ge 1 ] ||
    fail "the emitted tree declares zero skills — this comparison would be vacuous"
[ "$EXPECTED_SKILLS" = "$INSTALLED_SKILLS" ] ||
    fail_lines "installed skill inventory differs from the emitted one." \
        "expected: $EXPECTED_SKILLS" \
        "actual:   $INSTALLED_SKILLS"
pass "$(word_count "$INSTALLED_SKILLS") skills installed: $INSTALLED_SKILLS"

say "5c. Installed workflow scripts, asserted ON THE FILESYSTEM (never via plugin details — see gap 2)"
EXPECTED_WORKFLOWS=$(file_names "$EMITTED/workflows")
[ "$(word_count "$EXPECTED_WORKFLOWS")" -ge 1 ] ||
    fail "the emitted tree declares zero workflows — this comparison would be vacuous"
# shellcheck disable=SC2086  # deliberate word-splitting of a name list
for wf in $EXPECTED_WORKFLOWS; do
    [ -f "$INSTALL_PATH/workflows/$wf" ] ||
        fail "installed plugin is missing workflow: $INSTALL_PATH/workflows/$wf"
    cmp -s "$INSTALL_PATH/workflows/$wf" "$EMITTED/workflows/$wf" ||
        fail "installed workflow differs from the emitted bytes: $wf"
done
INSTALLED_WORKFLOWS=$(file_names "$INSTALL_PATH/workflows")
[ "$EXPECTED_WORKFLOWS" = "$INSTALLED_WORKFLOWS" ] ||
    fail_lines "installed workflow file set differs from the emitted one." \
        "expected: $EXPECTED_WORKFLOWS" \
        "actual:   $INSTALLED_WORKFLOWS"
pass "$(word_count "$INSTALLED_WORKFLOWS") workflow scripts installed byte-identical: $INSTALLED_WORKFLOWS"

say "5d. Corroboration only: claude plugin details rdm"
run_claude plugin details rdm >"$MKT/.details.txt"
sed 's/^/    /' "$MKT/.details.txt"
pass "details rendered (note: it has no Workflows category — 5c is the authority)"

# --- 6. prove the real ~/.claude is unmodified ----------------------------
say "6. Snapshotting the real ~/.claude config (after) and requiring it byte-unchanged"
snapshot_real_config "$MKT/.real-config-after"
if ! diff -u "$MKT/.real-config-before" "$MKT/.real-config-after" >"$MKT/.real-config-diff"; then
    sed 's/^/    /' "$MKT/.real-config-diff" >&2
    fail "the invoking user's real ~/.claude config CHANGED during this run — isolation failed"
fi
grep -q '^ABSENT-DIR .*/plugins/cache/rdm$' "$MKT/.real-config-after" ||
    fail "\$HOME/.claude/plugins/cache/rdm exists — the install leaked out of the isolated config"
grep -q '^ABSENT-DIR .*/plugins/marketplaces/rdm$' "$MKT/.real-config-after" ||
    fail "\$HOME/.claude/plugins/marketplaces/rdm exists — the marketplace leaked out of the isolated config"
pass "real ~/.claude config is byte-identical before and after, with no rdm cache or marketplace directory"

say "6b. Positive proof the write landed in the isolated dir instead"
[ -d "$CFG/plugins/cache/rdm/rdm/$CRATE_VERSION" ] ||
    fail "expected $CFG/plugins/cache/rdm/rdm/$CRATE_VERSION to exist"
pass "$CFG/plugins/cache/rdm/rdm/$CRATE_VERSION exists"

say "All plugin-install observations passed (developer-run; NOT covered by CI)."
