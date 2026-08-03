#!/bin/sh
# Hermetic regression for the CHECKED-IN Claude Code plugin tree
# (plugins/rdm/) and the repo-root marketplace entry
# (.claude-plugin/marketplace.json) that makes it installable.
#
# Sibling split, mirroring scripts/observe-workflow-listing.sh's rationale:
#
#   scripts/verify-plugin-install.sh   (this file) — HERMETIC. Pure POSIX
#       shell + coreutils; no python3, no node, no jq, no `claude`. Safe for
#       the CI glob, and .github/workflows/ci.yml's `for f in
#       scripts/verify-*.sh` loop picks it up automatically.
#
#   scripts/observe-plugin-install.sh  — NON-HERMETIC. Needs the `claude`
#       CLI to perform a real offline install. `claude` is not a CI
#       dependency and this phase does not add one, so that half is
#       developer-run and deliberately sits OUTSIDE the verify-* glob. It
#       carries no CI coverage.
#
# Its phase-3 sibling scripts/verify-plugin-distribution.sh gates the EMITTED
# bytes (that `agent-config claude --plugin` produces a self-consistent tree).
# This script gates the CHECKED-IN bytes plus installability packaging:
#
#   1. HERMETICITY: the run never touches this repo's working tree.
#   2. DRIFT (version-normalized): plugins/rdm/ is byte-identical to a fresh
#      `rdm agent-config claude --plugin` emission, with ONLY the manifest
#      `version` value normalized away on both sides.
#   3. RUNTIME VERSION: the FRESHLY GENERATED manifest's `version` equals the
#      workspace crate version read from Cargo.toml.
#   4. MARKETPLACE: shape (name/description/owner) plus `source` resolution to
#      a real directory holding .claude-plugin/plugin.json, with a
#      non-empty-entry floor.
#   5. WORKFLOW IDENTITY: every emitted workflows/*.js exists in the
#      checked-in tree and is byte-identical, with equal name sets.
#   6. SKILL INVENTORY: the checked-in skills/ directory set exactly equals
#      the emitted one, each with a valid SKILL.md frontmatter.
#   7. PLANTED-CORRUPTION SELF-TESTS: one per assertion above, each calling
#      the REAL check function against a corrupted scratch copy, proving none
#      of sections 2-6 is vacuous.
#
# ---------------------------------------------------------------------------
# FORBIDDEN — two rules a future editor must not reintroduce:
#
#   (a) No assertion on CHANGELOG.md. CLAUDE.md forbids this categorically:
#       .github/workflows/prepare-release.yml moves the whole [Unreleased]
#       body into a versioned section on release, so any such check goes red
#       on `main` the moment a release lands. This blocked v0.18.1 (CI run
#       30815546603). The changelog rule is enforced by review, not by a gate.
#
#   (b) No CI-run gate coupled to the crate version in COMMITTED bytes. Same
#       failure class, same cause: prepare-release.yml sed-bumps the version
#       in Cargo.toml and stages exactly `Cargo.toml Cargo.lock CHANGELOG.md`
#       (line 104) before pushing to `main` — it regenerates nothing, so
#       plugins/rdm/.claude-plugin/plugin.json goes stale on every release by
#       design. Hence: the drift gate NORMALIZES the manifest `version` on
#       both sides, and the version-currency assertion reads only FRESH
#       generator output. The `99.99.99` literals below are planted-corruption
#       INPUTS, never expectations; the only permitted version sources are
#       crate_version() (Cargo.toml) and freshly generated output.
#       (`rdm --version` does not exist — clap rejects it — so Cargo.toml is
#       the source of truth here.)
# ---------------------------------------------------------------------------
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"

# The canonical checked-in plugin tree and the repo-root marketplace manifest.
PLUGIN_DIR="$REPO_ROOT/plugins/rdm"
MARKETPLACE="$REPO_ROOT/.claude-plugin/marketplace.json"

# Clear rdm-related env vars inherited from the caller's shell. This is
# load-bearing beyond ordinary hermeticity: emission is --project-sensitive,
# and the checked-in tree is generated with NO --project so skill bodies carry
# the generic `--project <PROJECT>` placeholder a downstream consumer needs.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

# The number of skills the plugin surface is expected to ship (Decision 1's
# 11-entry PLUGIN_SKILL_NAMES table). A floor, so the inventory check cannot
# pass vacuously on an empty tree.
EXPECTED_SKILL_COUNT=11
# The number of Workflow engines the plugin surface ships (Decision 2).
MIN_WORKFLOW_COUNT=2
# Fixed placeholder substituted for the manifest `version` value on BOTH sides
# of the drift diff. Not a version, and never compared against one.
VERSION_PLACEHOLDER="0.0.0-NORMALIZED"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- generic helpers -------------------------------------------------------

# Sorted, space-separated basenames of the immediate subdirectories of $1.
dir_names() {
    find "$1" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; 2>/dev/null | sort | tr '\n' ' '
}

# Sorted, space-separated basenames of the immediate files of $1.
file_names() {
    find "$1" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; 2>/dev/null | sort | tr '\n' ' '
}

# Number of whitespace-separated words in $1.
word_count() {
    # shellcheck disable=SC2086  # deliberate word-splitting of a name list
    set -- $1
    printf '%s\n' "$#"
}

# Value of a TWO-space-indented (i.e. top-level) JSON string field. The indent
# is load-bearing: both manifests nest an object (`author` / `owner`) that
# repeats the key `name` at four spaces, and serde_json's to_string_pretty
# alphabetizes keys so the nested one would otherwise win a `^ *` match.
json_top_string() {
    sed -n "s/^  \"$2\": \"\\([^\"]*\\)\".*\$/\\1/p" "$1" | head -1
}

# Value of a FOUR-space-indented JSON string field (the author/owner block).
json_nested_string() {
    sed -n "s/^    \"$2\": \"\\([^\"]*\\)\".*\$/\\1/p" "$1" | head -1
}

# The workspace crate version, read from Cargo.toml. This is the ONLY
# permitted source of an expected version anywhere in this script.
crate_version() {
    awk '
        /^\[workspace\.package\]/ { f = 1; next }
        f && /^\[/               { exit }
        f && /^version *=/       { gsub(/[" ]/, "", $3); print $3; exit }
    ' "$REPO_ROOT/Cargo.toml"
}

# --- assertion helpers (each self-tested in section 7) ---------------------

# Rewrites ONLY the manifest's top-level `version` value to a fixed
# placeholder, in place. Surgical by construction: every other byte of the
# manifest — and every other file in the tree — stays under exact
# byte-identity. Self-test 7g proves this is not a blanket neuter.
normalize_manifest_version() {
    m="$1"
    [ -f "$m" ] || {
        echo "  missing manifest: $m" >&2
        return 1
    }
    sed "s/^\\(  \"version\": \"\\)[^\"]*\\(\".*\\)\$/\\1$VERSION_PLACEHOLDER\\2/" "$m" >"$m.norm" || return 1
    mv "$m.norm" "$m"
}

# check_drift <checked-tree> <fresh-tree>
# Version-normalized byte-identity over the WHOLE tree. `diff -r` catches
# added, removed and changed files, so a stale or hand-edited checked-in tree
# cannot survive. A release-time crate bump cannot move this diff.
check_drift() {
    work="$TMP/drift-work"
    rm -rf "$work"
    mkdir -p "$work"
    cp -R "$1" "$work/checked"
    cp -R "$2" "$work/fresh"
    normalize_manifest_version "$work/checked/.claude-plugin/plugin.json" || return 1
    normalize_manifest_version "$work/fresh/.claude-plugin/plugin.json" || return 1
    if ! diff -r "$work/checked" "$work/fresh" >"$work/diff.out" 2>&1; then
        sed 's/^/  /' "$work/diff.out" >&2
        return 1
    fi
    return 0
}

# check_manifest_version <manifest>
# MUST be called only with FRESHLY GENERATED output. Never point this at
# plugins/rdm/.claude-plugin/plugin.json — that is the release-coupling trap
# rule (b) in the header forbids.
check_manifest_version() {
    want=$(crate_version)
    got=$(json_top_string "$1" version)
    [ -n "$want" ] || {
        echo "  could not read [workspace.package] version from Cargo.toml" >&2
        return 1
    }
    [ -n "$got" ] || {
        echo "  could not read a version field from $1" >&2
        return 1
    }
    [ "$want" = "$got" ] || {
        echo "  generated manifest version '$got' != crate version '$want'" >&2
        return 1
    }
    return 0
}

# All `source` values declared by a marketplace manifest, one per line.
marketplace_sources() {
    sed -n 's/^ *"source": "\([^"]*\)".*$/\1/p' "$1"
}

# All plugin-ENTRY `name` values (six-space indent, inside the plugins array).
marketplace_entry_names() {
    sed -n 's/^      "name": "\([^"]*\)".*$/\1/p' "$1"
}

# check_marketplace_shape <marketplace.json>
check_marketplace_shape() {
    mj="$1"
    ok=0
    [ -f "$mj" ] || {
        echo "  missing marketplace manifest: $mj" >&2
        return 1
    }
    for key in name description; do
        v=$(json_top_string "$mj" "$key")
        [ -n "$v" ] || {
            echo "  marketplace top-level \"$key\" is missing or empty" >&2
            ok=1
        }
    done
    for key in name url; do
        v=$(json_nested_string "$mj" "$key")
        [ -n "$v" ] || {
            echo "  marketplace owner.\"$key\" is missing or empty" >&2
            ok=1
        }
    done
    n=$(marketplace_sources "$mj" | wc -l | tr -d ' ')
    [ "$n" -ge 1 ] || {
        echo "  marketplace declares zero plugin entries — the entry list must not be empty" >&2
        ok=1
    }
    return "$ok"
}

# check_source_resolution <marketplace.json> <repo-root>
# `claude plugin validate --strict` FALSE-PASSES a dangling source (verified:
# exit 0 on "source": "./does-not-exist"), so this check is ours to own. It
# also requires at least one entry, because "every entry resolves" is
# vacuously true of an empty list.
check_source_resolution() {
    mj="$1"
    root="$2"
    ok=0
    srcs="$TMP/.mkt-sources"
    nms="$TMP/.mkt-names"
    marketplace_sources "$mj" >"$srcs"
    marketplace_entry_names "$mj" >"$nms"
    n=$(wc -l <"$srcs" | tr -d ' ')
    m=$(wc -l <"$nms" | tr -d ' ')
    if [ "$n" -lt 1 ]; then
        echo "  marketplace declares zero plugin entries — source resolution would be vacuous" >&2
        return 1
    fi
    if [ "$n" != "$m" ]; then
        echo "  every plugin entry needs both a name and a source (found $m names, $n sources)" >&2
        ok=1
    fi
    i=1
    while [ "$i" -le "$n" ]; do
        src=$(sed -n "${i}p" "$srcs")
        nm=$(sed -n "${i}p" "$nms")
        case "$src" in
            /*) dir="$src" ;;
            *) dir="$root/$src" ;;
        esac
        if [ ! -d "$dir" ]; then
            echo "  marketplace source '$src' does not resolve to a directory ($dir)" >&2
            ok=1
        elif [ ! -f "$dir/.claude-plugin/plugin.json" ]; then
            echo "  marketplace source '$src' has no .claude-plugin/plugin.json" >&2
            ok=1
        else
            pname=$(json_top_string "$dir/.claude-plugin/plugin.json" name)
            [ "$nm" = "$pname" ] || {
                echo "  marketplace entry name '$nm' != plugin manifest name '$pname' at $src" >&2
                ok=1
            }
        fi
        i=$((i + 1))
    done
    return "$ok"
}

# check_workflow_identity <checked-tree> <fresh-tree>
check_workflow_identity() {
    checked="$1/workflows"
    fresh="$2/workflows"
    ok=0
    expected=$(file_names "$fresh")
    actual=$(file_names "$checked")
    if [ "$expected" != "$actual" ]; then
        echo "  checked-in workflows/ file set differs from generator output" >&2
        echo "    expected: $expected" >&2
        echo "    actual:   $actual" >&2
        ok=1
    fi
    count=$(word_count "$expected")
    [ "$count" -ge "$MIN_WORKFLOW_COUNT" ] || {
        echo "  expected >= $MIN_WORKFLOW_COUNT emitted workflow scripts, found $count — floor not met" >&2
        ok=1
    }
    # shellcheck disable=SC2086  # deliberate word-splitting of a name list
    for wf in $expected; do
        if [ ! -f "$checked/$wf" ]; then
            echo "  missing checked-in workflow: $checked/$wf" >&2
            ok=1
        elif ! cmp -s "$checked/$wf" "$fresh/$wf"; then
            echo "  checked-in workflow differs from generator output: $wf" >&2
            ok=1
        fi
    done
    return "$ok"
}

# check_skill_frontmatter <SKILL.md> <expected-name>
check_skill_frontmatter() {
    md="$1"
    want="$2"
    [ -f "$md" ] || {
        echo "  missing: $md" >&2
        return 1
    }
    first=$(head -1 "$md")
    [ "$first" = "---" ] || {
        echo "  $md: first line must be '---' (frontmatter opener), got '$first'" >&2
        return 1
    }
    close=$(awk 'NR > 1 && $0 == "---" { print NR; exit }' "$md")
    [ -n "$close" ] || {
        echo "  $md: no closing '---' frontmatter delimiter" >&2
        return 1
    }
    fm=$(awk -v end="$close" 'NR > 1 && NR < end' "$md")
    got=$(printf '%s\n' "$fm" | sed -n 's/^name: *\(.*\)$/\1/p' | head -1)
    [ "$got" = "$want" ] || {
        echo "  $md: frontmatter name '$got' does not match directory name '$want'" >&2
        return 1
    }
    desc=$(printf '%s\n' "$fm" | sed -n 's/^description: *\(.*\)$/\1/p' | head -1)
    [ -n "$desc" ] || {
        echo "  $md: frontmatter description is missing or empty" >&2
        return 1
    }
    return 0
}

# check_skill_inventory <checked-tree> <fresh-tree>
# The expected name set is DERIVED from generator output, never hand-copied.
check_skill_inventory() {
    ok=0
    expected=$(dir_names "$2/skills")
    actual=$(dir_names "$1/skills")
    if [ "$expected" != "$actual" ]; then
        echo "  checked-in skills/ directory set differs from generator output" >&2
        echo "    expected: $expected" >&2
        echo "    actual:   $actual" >&2
        ok=1
    fi
    count=$(word_count "$expected")
    [ "$count" -ge 1 ] || {
        echo "  generator emitted zero skills — inventory check would be vacuous" >&2
        ok=1
    }
    [ "$count" -eq "$EXPECTED_SKILL_COUNT" ] || {
        echo "  expected exactly $EXPECTED_SKILL_COUNT emitted skills, found $count" >&2
        ok=1
    }
    # shellcheck disable=SC2086  # deliberate word-splitting of a name list
    for name in $expected; do
        check_skill_frontmatter "$1/skills/$name/SKILL.md" "$name" || ok=1
    done
    return "$ok"
}

# --- 1. hermeticity baseline ----------------------------------------------
say "1. Capturing $REPO_ROOT git status (hermeticity baseline)"
BEFORE_STATUS=$(git -C "$REPO_ROOT" status --porcelain)
pass "baseline captured"

say "1b. Emitting a fresh plugin tree into \$TMP (no --project, so bodies keep the generic <PROJECT> placeholder)"
"$RDM_BIN" agent-config claude --plugin --out "$TMP/fresh" >/dev/null
FRESH="$TMP/fresh"
pass "emitted into $FRESH"

[ -d "$PLUGIN_DIR" ] || fail "checked-in plugin tree missing: $PLUGIN_DIR"
[ -f "$MARKETPLACE" ] || fail "marketplace manifest missing: $MARKETPLACE"

# --- 2. version-normalized drift gate -------------------------------------
say "2. Drift (version-normalized): plugins/rdm/ == fresh emission, modulo the manifest version"
if check_drift "$PLUGIN_DIR" "$FRESH"; then
    pass "checked-in tree is byte-identical to generator output (manifest version normalized on both sides)"
else
    fail "checked-in plugin tree has drifted — regenerate with:\n  env -u RDM_ROOT -u RDM_PROJECT cargo run -q -- agent-config claude --plugin --out plugins/rdm"
fi

# --- 3. runtime version assertion (FRESH output only) ----------------------
say "3. Runtime version: the FRESHLY GENERATED manifest version equals the crate version"
if check_manifest_version "$FRESH/.claude-plugin/plugin.json"; then
    pass "generated manifest version == $(crate_version) (from Cargo.toml)"
else
    fail "the generated manifest version does not match Cargo.toml's [workspace.package] version"
fi

# --- 4. marketplace shape + source resolution ------------------------------
say "4. Marketplace: shape (name/description/owner) and a non-empty entry list"
if check_marketplace_shape "$MARKETPLACE"; then
    pass "marketplace manifest carries name, description, owner.name, owner.url and >= 1 plugin entry"
else
    fail "marketplace manifest shape check failed (see lines above)"
fi

say "4b. Marketplace: every plugin entry's source resolves to a real plugin directory"
if check_source_resolution "$MARKETPLACE" "$REPO_ROOT"; then
    pass "every declared source resolves to a directory holding .claude-plugin/plugin.json with a matching name"
else
    fail "marketplace source resolution failed (see lines above)"
fi

# --- 5. workflow byte-identity ---------------------------------------------
say "5. Workflows: every emitted workflows/*.js is present and byte-identical in the checked-in tree"
if check_workflow_identity "$PLUGIN_DIR" "$FRESH"; then
    pass "workflow file sets are equal and every script is byte-identical ($(word_count "$(file_names "$FRESH/workflows")") scripts)"
else
    fail "workflow byte-identity check failed (see lines above)"
fi

# --- 6. skill inventory ----------------------------------------------------
say "6. Skills: the checked-in inventory equals the emitted one, each with valid frontmatter"
if check_skill_inventory "$PLUGIN_DIR" "$FRESH"; then
    pass "$EXPECTED_SKILL_COUNT skills present with matching names and valid SKILL.md frontmatter"
else
    fail "skill inventory check failed (see lines above)"
fi

# --- 7. planted-corruption self-tests --------------------------------------
say "7a. Self-test: a dangling marketplace source turns the source-resolution gate red"
ST_MKT_DANGLING="$TMP/st-dangling.json"
sed 's|"source": "./plugins/rdm"|"source": "./does-not-exist"|' "$MARKETPLACE" >"$ST_MKT_DANGLING"
grep -q '"source": "./does-not-exist"' "$ST_MKT_DANGLING" ||
    fail "self-test 7a setup failed — the planted dangling source was not written"
# `claude plugin validate --strict` exits 0 on exactly this input (verified),
# which is why source resolution is asserted here rather than delegated.
if check_source_resolution "$ST_MKT_DANGLING" "$REPO_ROOT" >/dev/null 2>&1; then
    fail "self-test 7a: a dangling source was NOT detected — the source-resolution gate is vacuous"
fi
pass "self-test 7a: a dangling source correctly turns the source-resolution gate red"

say "7b. Self-test: an empty plugin entry list turns both marketplace gates red"
ST_MKT_EMPTY="$TMP/st-empty.json"
cat >"$ST_MKT_EMPTY" <<'JSON'
{
  "name": "rdm",
  "description": "planted-corruption input: no plugin entries",
  "owner": {
    "name": "Edward Paget",
    "url": "https://github.com/edpaget/rdm"
  },
  "plugins": []
}
JSON
if check_marketplace_shape "$ST_MKT_EMPTY" >/dev/null 2>&1; then
    fail "self-test 7b: an empty plugins array was NOT detected — the non-empty-entry floor is vacuous"
fi
if check_source_resolution "$ST_MKT_EMPTY" "$REPO_ROOT" >/dev/null 2>&1; then
    fail "self-test 7b: an empty plugins array vacuously satisfied source resolution"
fi
pass "self-test 7b: an empty plugin entry list correctly turns both marketplace gates red"

say "7c. Self-test: a marketplace entry name that disagrees with the plugin manifest turns the gate red"
ST_MKT_NAME="$TMP/st-name.json"
sed 's|^      "name": "rdm",|      "name": "not-rdm",|' "$MARKETPLACE" >"$ST_MKT_NAME"
grep -q '"name": "not-rdm"' "$ST_MKT_NAME" ||
    fail "self-test 7c setup failed — the planted entry name was not written"
if check_source_resolution "$ST_MKT_NAME" "$REPO_ROOT" >/dev/null 2>&1; then
    fail "self-test 7c: a mismatched entry name was NOT detected — the name pairing is vacuous"
fi
pass "self-test 7c: a mismatched entry name correctly turns the source-resolution gate red"

say "7d. Self-test: a renamed skill directory turns the skill-inventory gate red"
ST_SKILL="$TMP/st-skill"
rm -rf "$ST_SKILL"
cp -R "$PLUGIN_DIR" "$ST_SKILL"
mv "$ST_SKILL/skills/roadmap" "$ST_SKILL/skills/rdm-roadmap"
if check_skill_inventory "$ST_SKILL" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7d: a renamed skill directory was NOT detected — the inventory gate is vacuous"
fi
pass "self-test 7d: a renamed skill directory correctly turns the skill-inventory gate red"

say "7e. Self-test: a SKILL.md stripped of its frontmatter turns the frontmatter gate red"
ST_FM="$TMP/st-frontmatter"
rm -rf "$ST_FM"
cp -R "$PLUGIN_DIR" "$ST_FM"
printf 'no frontmatter here at all\n' >"$ST_FM/skills/do/SKILL.md"
if check_skill_inventory "$ST_FM" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7e: a frontmatter-less SKILL.md was NOT detected — the frontmatter gate is vacuous"
fi
pass "self-test 7e: a frontmatter-less SKILL.md correctly turns the frontmatter gate red"

say "7f. Self-test: a mutated and a deleted workflow both turn the workflow-identity gate red"
ST_WF="$TMP/st-workflow"
rm -rf "$ST_WF"
cp -R "$PLUGIN_DIR" "$ST_WF"
printf '\n// planted-corruption byte\n' >>"$ST_WF/workflows/rdm-wf-dispatch-phase.js"
if check_workflow_identity "$ST_WF" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7f: a mutated workflow byte was NOT detected — the byte-identity gate is vacuous"
fi
ST_WF_DEL="$TMP/st-workflow-deleted"
rm -rf "$ST_WF_DEL"
cp -R "$PLUGIN_DIR" "$ST_WF_DEL"
rm -f "$ST_WF_DEL/workflows/rdm-wf-review-refute-fix.js"
if check_workflow_identity "$ST_WF_DEL" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7f: a deleted workflow was NOT detected — the name-set equality check is vacuous"
fi
pass "self-test 7f: a mutated byte and a deleted file both correctly turn the workflow-identity gate red"

say "7g. Self-test: a bumped crate version keeps the drift gate GREEN but turns the runtime version assertion RED"
ST_BUMP="$TMP/st-bumped-version"
rm -rf "$ST_BUMP"
cp -R "$PLUGIN_DIR" "$ST_BUMP"
# 99.99.99 is a planted-corruption INPUT, never an expectation.
sed 's/^\(  "version": "\)[^"]*\(".*\)$/\199.99.99\2/' "$ST_BUMP/.claude-plugin/plugin.json" >"$ST_BUMP/.claude-plugin/plugin.json.tmp"
mv "$ST_BUMP/.claude-plugin/plugin.json.tmp" "$ST_BUMP/.claude-plugin/plugin.json"
grep -q '"version": "99.99.99"' "$ST_BUMP/.claude-plugin/plugin.json" ||
    fail "self-test 7g setup failed — the planted version was not written"
if ! check_drift "$ST_BUMP" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7g: a version-only difference turned the drift gate RED — a release-time crate bump would red-light main"
fi
if check_manifest_version "$ST_BUMP/.claude-plugin/plugin.json" >/dev/null 2>&1; then
    fail "self-test 7g: a stale manifest version was NOT detected — the runtime version assertion is vacuous"
fi
pass "self-test 7g: a version bump leaves the drift gate green and is caught only by the runtime assertion"

say "7h. Self-test: a NON-version manifest field change turns the drift gate red (normalization is surgical)"
ST_DESC="$TMP/st-description"
rm -rf "$ST_DESC"
cp -R "$PLUGIN_DIR" "$ST_DESC"
sed 's/^\(  "description": "\)/\1X/' "$ST_DESC/.claude-plugin/plugin.json" >"$ST_DESC/.claude-plugin/plugin.json.tmp"
mv "$ST_DESC/.claude-plugin/plugin.json.tmp" "$ST_DESC/.claude-plugin/plugin.json"
if check_drift "$ST_DESC" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7h: a mutated manifest description was NOT detected — normalization neutered the whole manifest comparison"
fi
pass "self-test 7h: a mutated non-version manifest field correctly turns the drift gate red"

say "7i. Self-test: a mutated SKILL.md body byte turns the drift gate red (drift covers more than the manifest)"
ST_BODY="$TMP/st-skill-body"
rm -rf "$ST_BODY"
cp -R "$PLUGIN_DIR" "$ST_BODY"
printf '\nplanted-corruption line\n' >>"$ST_BODY/skills/autopilot/SKILL.md"
if check_drift "$ST_BODY" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7i: a mutated SKILL.md body was NOT detected — the drift gate does not cover skill bodies"
fi
pass "self-test 7i: a mutated SKILL.md body correctly turns the drift gate red"

say "7j. Self-test: an added stray file turns the drift gate red"
ST_EXTRA="$TMP/st-extra-file"
rm -rf "$ST_EXTRA"
cp -R "$PLUGIN_DIR" "$ST_EXTRA"
printf 'stray\n' >"$ST_EXTRA/workflows/rdm-wf-stray.js"
if check_drift "$ST_EXTRA" "$FRESH" >/dev/null 2>&1; then
    fail "self-test 7j: an added stray file was NOT detected — the drift gate misses additions"
fi
pass "self-test 7j: an added stray file correctly turns the drift gate red"

# --- 8. hermeticity guard --------------------------------------------------
say "8. Confirming $REPO_ROOT git status is unchanged after the whole run"
AFTER_STATUS=$(git -C "$REPO_ROOT" status --porcelain)
[ "$BEFORE_STATUS" = "$AFTER_STATUS" ] ||
    fail "repo git status changed during this run — every write must land in \$TMP.\nbefore:\n$BEFORE_STATUS\nafter:\n$AFTER_STATUS"
pass "repo git status unchanged"

say "All plugin-install checks passed."
