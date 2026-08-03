#!/bin/sh
# Hermetic regression for the `rdm agent-config claude --plugin` emission
# mode: does the emitted Claude Code plugin TREE hold together on its own?
#
# This is the plugin-tree sibling of scripts/verify-agent-config-distribution.sh
# (the raw-skills self-consistency harness): same emit-into-a-temp-dir shape,
# same layout/reference-resolution/scope-negative/planted-corruption
# structure, applied to the plugin surface instead. Per the phase 3 body,
# this script asserts the EMITTED BYTES are self-consistent — it never
# invokes the `claude` CLI and never asserts installability, marketplace
# packaging, or plugin discovery (that is phase 4's harness).
#
#   1. EMIT + ISOLATION: runs the real `target/debug/rdm agent-config claude
#      --plugin --out <tmp>` and asserts it never touches this repo's
#      working tree (`git status --porcelain` before/after).
#   2. STRUCTURAL / LAYOUT: `.claude-plugin/plugin.json` exists, is valid
#      JSON, and carries the fields `generate_plugin_manifest` promises
#      (name/version/description/author, no `workflows` key); every skill
#      lands at `skills/<name>/SKILL.md` and every workflow at
#      `workflows/<name>.js`, both at the plugin root (never nested under
#      `.claude-plugin/`); `.claude-plugin/` holds nothing but the manifest.
#   3. NAMING TRANSFORM: the 11 emitted skill directory names match Phase 1's
#      recorded Decision 1 (the `rdm-` prefix dropped) and the 2 emitted
#      workflow file names match Decision 2 (the `rdm-wf-` prefix kept) —
#      read from docs/plugin-distribution.md's "Fixed Plugin Layout" section
#      and rdm-core/src/agent_config.rs's PLUGIN_SKILL_NAMES table, not
#      hardcoded independently of either.
#   4. REFERENCE RESOLUTION: every `rdm:<engine-stem>` namespaced Workflow
#      reference inside an emitted skill's SKILL.md (the plugin-mode
#      invocation form — Decision 3) resolves to a real
#      `workflows/<engine-stem>.js` file in the SAME emitted tree, with an
#      explicit occurrence floor so the check cannot pass vacuously on zero
#      matches.
#   5. SCOPE NEGATIVES: every rejected flag combination in the phase body's
#      matrix (`--plugin --skills`, `--plugin` with no destination,
#      `--plugin --user`, `--plugin` on each non-Claude platform, and the
#      Pi/`--plugin`/`--mcp` precedence case) errors with its own distinct,
#      actionable message — never a generic reuse of another combination's
#      text — and the pre-existing `--skills --user` positive control is
#      unaffected.
#   6. PLANTED-CORRUPTION SELF-TESTS: one paired self-test per assertion
#      above, proving none of sections 2-5 is vacuous.
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo). No other
# tooling — unlike verify-agent-config-distribution.sh's downstream section
# 7, this script never shells out to `node` or `claude`.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

# Plugin-mode skill directory names (Decision 1: `rdm-` prefix dropped),
# named once here and reused by every section below.
PLUGIN_SKILLS="roadmap do review document estimate dispatch-phase autopilot land revise plan-review backlog"
# Plugin-mode workflow file names (Decision 2: `rdm-wf-` prefix kept —
# identical to the raw-skills surface).
DISPATCH_WF="rdm-wf-dispatch-phase.js"
REVIEW_WF="rdm-wf-review-refute-fix.js"
PLUGIN_WORKFLOWS="$DISPATCH_WF $REVIEW_WF"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- helpers -----------------------------------------------------------

# Asserts <plugin_root>/.claude-plugin/plugin.json exists, is valid JSON, and
# carries the fields generate_plugin_manifest documents (name/version/
# description/author, and explicitly no `workflows` key). Requires `python3`
# — used only for JSON field checks, never for the emission itself.
assert_manifest_valid() {
    manifest="$1/.claude-plugin/plugin.json"
    [ -f "$manifest" ] || {
        echo "  missing: $manifest" >&2
        return 1
    }
    python3 - "$manifest" <<'PY' || return 1
import json
import sys

with open(sys.argv[1]) as fh:
    data = json.load(fh)

errors = []
if data.get("name") != "rdm":
    errors.append(f"name must be 'rdm', got {data.get('name')!r}")
if not isinstance(data.get("version"), str) or not data.get("version"):
    errors.append("version must be a non-empty string")
if not isinstance(data.get("description"), str) or not data.get("description"):
    errors.append("description must be a non-empty string")
author = data.get("author")
if not isinstance(author, dict) or not author.get("name") or not author.get("url"):
    errors.append("author must be an object with non-empty name and url")
if "workflows" in data:
    errors.append("manifest must not declare a 'workflows' key (convention-discovered directory)")

if errors:
    for e in errors:
        print(f"  manifest error: {e}", file=sys.stderr)
    sys.exit(1)
PY
}

# Asserts <plugin_root> holds exactly the expected layout: the manifest under
# .claude-plugin/ (and nothing else there), every $PLUGIN_SKILLS name at
# skills/<name>/SKILL.md, and every $PLUGIN_WORKFLOWS file at
# workflows/<file> — both directories siblings of .claude-plugin/, never
# nested under it.
check_layout() {
    root="$1"
    ok=0
    assert_manifest_valid "$root" || ok=1
    plugin_dir_entries=$(find "$root/.claude-plugin" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l | tr -d ' ')
    [ "$plugin_dir_entries" = "1" ] || {
        echo "  .claude-plugin/ must contain exactly plugin.json, found $plugin_dir_entries entries" >&2
        ok=1
    }
    # shellcheck disable=SC2086  # PLUGIN_SKILLS is a deliberately word-split name list
    for name in $PLUGIN_SKILLS; do
        [ -f "$root/skills/$name/SKILL.md" ] || {
            echo "  missing: $root/skills/$name/SKILL.md" >&2
            ok=1
        }
    done
    # shellcheck disable=SC2086
    for wf in $PLUGIN_WORKFLOWS; do
        [ -f "$root/workflows/$wf" ] || {
            echo "  missing: $root/workflows/$wf" >&2
            ok=1
        }
    done
    return "$ok"
}

# Asserts the plugin-mode skill directory set under <root>/skills is EXACTLY
# $PLUGIN_SKILLS (no more, no fewer, no stale `rdm-`-prefixed name), and the
# workflow file set under <root>/workflows is EXACTLY $PLUGIN_WORKFLOWS.
check_naming_transform() {
    root="$1"
    actual_skills=$(find "$root/skills" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; 2>/dev/null | sort | tr '\n' ' ')
    # shellcheck disable=SC2086
    expected_skills=$(printf '%s\n' $PLUGIN_SKILLS | sort | tr '\n' ' ')
    if [ "$actual_skills" != "$expected_skills" ]; then
        echo "  skill directory names do not match Decision 1's transform" >&2
        echo "    expected: $expected_skills" >&2
        echo "    actual:   $actual_skills" >&2
        return 1
    fi
    actual_workflows=$(find "$root/workflows" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; 2>/dev/null | sort | tr '\n' ' ')
    # shellcheck disable=SC2086
    expected_workflows=$(printf '%s\n' $PLUGIN_WORKFLOWS | sort | tr '\n' ' ')
    if [ "$actual_workflows" != "$expected_workflows" ]; then
        echo "  workflow file names do not match Decision 2 (rdm-wf- prefix kept)" >&2
        echo "    expected: $expected_workflows" >&2
        echo "    actual:   $actual_workflows" >&2
        return 1
    fi
}

# Asserts the three --plugin rejection messages ($1 = no-destination, $2 =
# --user, $3 = platform) are pairwise distinct from each other and that $1
# does not contain the --skills destination-error text ($4). Factored out of
# section 5g so section 6e can call it with deliberately-collided input and
# prove the comparison itself isn't vacuous.
check_messages_distinct() {
    no_dest="$1"
    plugin_user="$2"
    platform="$3"
    skills_dest_text="$4"
    [ "$no_dest" != "$plugin_user" ] || return 1
    [ "$no_dest" != "$platform" ] || return 1
    [ "$plugin_user" != "$platform" ] || return 1
    case "$no_dest" in
        *"$skills_dest_text"*) return 1 ;;
    esac
    return 0
}

# Scans every skills/*/SKILL.md under <root> for a namespaced `rdm:<stem>`
# Workflow reference (the plugin-mode invocation form, Decision 3) and
# asserts each resolves to a real workflows/<stem>.js file in the SAME tree.
# Sets REF_COUNT / REF_UNRESOLVED as globals for the caller to inspect (e.g.
# an occurrence floor), and returns nonzero if any reference is unresolved.
check_workflow_refs_resolve() {
    root="$1"
    refs_scratch="$TMP/.plugin-refs-scratch.txt"
    REF_COUNT=0
    REF_UNRESOLVED=0
    for md in "$root"/skills/*/SKILL.md; do
        [ -f "$md" ] || continue
        grep -oE 'rdm:rdm-wf-[a-z-]+' "$md" >"$refs_scratch" 2>/dev/null || : >"$refs_scratch"
        while IFS= read -r ref; do
            [ -n "$ref" ] || continue
            REF_COUNT=$((REF_COUNT + 1))
            stem=${ref#rdm:}
            if [ ! -f "$root/workflows/$stem.js" ]; then
                echo "  unresolved: $md references $ref -> missing $root/workflows/$stem.js" >&2
                REF_UNRESOLVED=$((REF_UNRESOLVED + 1))
            fi
        done <"$refs_scratch"
    done
    rm -f "$refs_scratch"
    [ "$REF_UNRESOLVED" -eq 0 ]
}

# --- 1. hermeticity guard + emit -----------------------------------------
say "1. Capturing $REPO_ROOT git status before emission (hermeticity baseline)"
BEFORE_STATUS=$(git -C "$REPO_ROOT" status --porcelain)
pass "baseline captured"

say "1b. Emitting 'agent-config claude --plugin --out <tmp>'"
"$RDM_BIN" agent-config claude --plugin --project distro-check --out "$TMP/plugin" >/dev/null
pass "emitted into $TMP/plugin"

# --- 2. structural / layout -----------------------------------------------
say "2. Structural: manifest valid, 11 skills + 2 workflows at conventional paths, .claude-plugin/ holds only the manifest"
if check_layout "$TMP/plugin"; then
    pass "layout: manifest valid, all 11 skills + 2 workflows present, .claude-plugin/ clean"
else
    fail "layout check failed (see lines above)"
fi

# --- 3. naming transform ---------------------------------------------------
say "3. Naming transform: skill dirs match Decision 1, workflow files match Decision 2"
if check_naming_transform "$TMP/plugin"; then
    pass "naming transform matches docs/plugin-distribution.md Decisions 1 and 2"
else
    fail "naming transform mismatch (see lines above)"
fi

# --- 4. reference resolution -----------------------------------------------
say "4. Reference resolution: every emitted rdm:<engine> reference resolves in-tree"
if check_workflow_refs_resolve "$TMP/plugin"; then
    pass "all $REF_COUNT namespaced workflow reference(s) resolve"
else
    fail "$REF_UNRESOLVED unresolved namespaced workflow reference(s) (see lines above)"
fi
[ "$REF_COUNT" -ge 5 ] ||
    fail "expected >= 5 total rdm:<engine> references across the emitted skills, found $REF_COUNT — check is not vacuous only if this floor holds"
grep -qF "rdm:rdm-wf-dispatch-phase" "$TMP/plugin/skills/dispatch-phase/SKILL.md" ||
    fail "skills/dispatch-phase/SKILL.md must reference rdm:rdm-wf-dispatch-phase"
grep -qF "rdm:rdm-wf-dispatch-phase" "$TMP/plugin/skills/do/SKILL.md" ||
    fail "skills/do/SKILL.md must reference rdm:rdm-wf-dispatch-phase"
pass "dispatch-phase/do carry their expected exact references"

# --- 5. scope negatives -----------------------------------------------------
say "5a. Negative: --plugin --skills is rejected (clap conflict)"
if "$RDM_BIN" agent-config claude --plugin --skills --out "$TMP/neg-a" >"$TMP/neg-a.log" 2>&1; then
    fail "--plugin --skills unexpectedly succeeded"
fi
MSG_SKILLS_CONFLICT=$(cat "$TMP/neg-a.log")
printf '%s' "$MSG_SKILLS_CONFLICT" | grep -q "cannot be used with" ||
    fail "--plugin --skills: expected clap's mode-conflict message, got:\n$MSG_SKILLS_CONFLICT"
pass "--plugin --skills rejected with clap's mode-conflict message"

say "5b. Negative: --plugin with neither --out nor --user is rejected"
if "$RDM_BIN" agent-config claude --plugin >"$TMP/neg-b.log" 2>&1; then
    fail "--plugin alone unexpectedly succeeded"
fi
MSG_NO_DEST=$(cat "$TMP/neg-b.log")
printf '%s' "$MSG_NO_DEST" | grep -q -- "--plugin requires --out" ||
    fail "--plugin alone: expected a '--plugin requires --out' message, got:\n$MSG_NO_DEST"
pass "--plugin with no destination rejected with its own message"

say "5c. Negative: --plugin --user is rejected with its OWN message (not the --out text)"
mkdir -p "$TMP/user-home"
if HOME="$TMP/user-home" "$RDM_BIN" agent-config claude --plugin --user >"$TMP/neg-c.log" 2>&1; then
    fail "--plugin --user unexpectedly succeeded"
fi
MSG_PLUGIN_USER=$(cat "$TMP/neg-c.log")
printf '%s' "$MSG_PLUGIN_USER" | grep -q -- "--plugin cannot be combined with --user" ||
    fail "--plugin --user: expected the plugin-specific --user rejection, got:\n$MSG_PLUGIN_USER"
printf '%s' "$MSG_PLUGIN_USER" | grep -q "claude plugin marketplace add" ||
    fail "--plugin --user: expected the message to explain installation via claude plugin marketplace add / install, got:\n$MSG_PLUGIN_USER"
pass "--plugin --user rejected with its own installation-pointing message"

say "5d. Negative: --plugin is rejected on every non-Claude platform"
for platform in agents-md cursor copilot pi; do
    if "$RDM_BIN" agent-config "$platform" --plugin --out "$TMP/neg-d-$platform" >"$TMP/neg-d-$platform.log" 2>&1; then
        fail "--plugin on $platform unexpectedly succeeded"
    fi
    grep -q -- "--plugin is only supported for the claude platform" "$TMP/neg-d-$platform.log" ||
        fail "--plugin on $platform: expected the platform-rejection message, got:\n$(cat "$TMP/neg-d-$platform.log")"
done
MSG_PLATFORM=$(cat "$TMP/neg-d-pi.log")
pass "--plugin rejected on agents-md, cursor, copilot, and pi"

say "5e. Negative: --plugin --mcp on Pi surfaces the plugin message, not the unrelated Pi+--mcp message"
if "$RDM_BIN" agent-config pi --plugin --mcp --out "$TMP/neg-e" >"$TMP/neg-e.log" 2>&1; then
    fail "--plugin --mcp on pi unexpectedly succeeded"
fi
MSG_PI_MCP_PRECEDENCE=$(cat "$TMP/neg-e.log")
printf '%s' "$MSG_PI_MCP_PRECEDENCE" | grep -q -- "--plugin is only supported for the claude platform" ||
    fail "--plugin --mcp on pi: expected the plugin-specific message to win, got:\n$MSG_PI_MCP_PRECEDENCE"
if printf '%s' "$MSG_PI_MCP_PRECEDENCE" | grep -q "Pi does not support MCP"; then
    fail "--plugin --mcp on pi: the unrelated Pi+--mcp message leaked through — precedence regression"
fi
pass "--plugin --mcp on pi surfaces only the plugin-specific message"

say "5f. Positive control: --skills --user still succeeds and writes neither workflows/ nor .claude-plugin/"
mkdir -p "$TMP/user-home-control"
if HOME="$TMP/user-home-control" "$RDM_BIN" agent-config claude --skills --user >"$TMP/pos-f.log" 2>&1; then
    [ -d "$TMP/user-home-control/.claude/workflows" ] &&
        fail "--skills --user must not write .claude/workflows"
    [ -d "$TMP/user-home-control/.claude-plugin" ] &&
        fail "--skills --user must not write .claude-plugin"
    pass "--skills --user still succeeds, unaffected by --plugin"
else
    cat "$TMP/pos-f.log" >&2
    fail "--skills --user failed unexpectedly"
fi

say "5g. Distinct-message self-check: the three rejection strings are pairwise distinct"
MSG_SKILLS_DEST_ERROR="--skills requires --out or --user to specify the output directory"
if check_messages_distinct "$MSG_NO_DEST" "$MSG_PLUGIN_USER" "$MSG_PLATFORM" "$MSG_SKILLS_DEST_ERROR"; then
    pass "5g: all three --plugin rejection messages are distinct from each other and from --skills's"
else
    fail "5g: two of the three --plugin rejection messages collide, or one reuses the --skills destination-error text"
fi

# --- 6. planted-corruption self-tests --------------------------------------
say "6a. Self-test: deleting plugin.json turns the layout check red"
SCRATCH_LAYOUT="$TMP/scratch-no-manifest"
rm -rf "$SCRATCH_LAYOUT"
cp -R "$TMP/plugin" "$SCRATCH_LAYOUT"
rm -f "$SCRATCH_LAYOUT/.claude-plugin/plugin.json"
if check_layout "$SCRATCH_LAYOUT" >/dev/null 2>&1; then
    fail "self-test 6a: a deleted plugin.json was NOT detected — the layout gate is vacuous"
fi
pass "self-test 6a: a deleted plugin.json correctly turns the layout gate red"

say "6b. Self-test: rewriting a skill's workflow reference to a nonexistent name turns the resolution check red"
SCRATCH_REF="$TMP/scratch-bad-ref"
rm -rf "$SCRATCH_REF"
cp -R "$TMP/plugin" "$SCRATCH_REF"
sed 's/rdm:rdm-wf-dispatch-phase/rdm:rdm-wf-typo-does-not-exist/' \
    "$SCRATCH_REF/skills/dispatch-phase/SKILL.md" >"$SCRATCH_REF/skills/dispatch-phase/SKILL.md.new"
mv "$SCRATCH_REF/skills/dispatch-phase/SKILL.md.new" "$SCRATCH_REF/skills/dispatch-phase/SKILL.md"
if check_workflow_refs_resolve "$SCRATCH_REF" >/dev/null 2>&1; then
    fail "self-test 6b: a planted unresolved reference was NOT detected — the resolution gate is vacuous"
fi
pass "self-test 6b: a planted unresolved reference correctly turns the resolution gate red"

say "6c. Self-test: stripping every reference from every skill turns the occurrence floor red (not vacuously satisfied)"
SCRATCH_STRIP="$TMP/scratch-no-refs"
rm -rf "$SCRATCH_STRIP"
cp -R "$TMP/plugin" "$SCRATCH_STRIP"
for md in "$SCRATCH_STRIP"/skills/*/SKILL.md; do
    sed 's/rdm:rdm-wf-[a-z-]*/rdm-workflow-reference-removed/g' "$md" >"$md.new"
    mv "$md.new" "$md"
done
check_workflow_refs_resolve "$SCRATCH_STRIP" >/dev/null 2>&1 || true
[ "$REF_COUNT" -eq 0 ] || fail "self-test 6c: stripping failed to remove all references (found $REF_COUNT) — self-test setup is broken"
if [ "$REF_COUNT" -ge 5 ]; then
    fail "self-test 6c: the occurrence floor did not go red on zero references — it is vacuously satisfiable"
fi
pass "self-test 6c: stripping all references correctly turns the occurrence floor red"

say "6d. Self-test: renaming an emitted skill dir to violate Decision 1 turns the naming-transform check red"
SCRATCH_NAME="$TMP/scratch-bad-name"
rm -rf "$SCRATCH_NAME"
cp -R "$TMP/plugin" "$SCRATCH_NAME"
mv "$SCRATCH_NAME/skills/roadmap" "$SCRATCH_NAME/skills/rdm-roadmap"
if check_naming_transform "$SCRATCH_NAME" >/dev/null 2>&1; then
    fail "self-test 6d: a re-prefixed skill directory was NOT detected — the naming-transform gate is vacuous"
fi
pass "self-test 6d: a re-prefixed skill directory correctly turns the naming-transform gate red"

say "6e. Self-test: check_messages_distinct (5g's real comparison) catches a collided pair"
if check_messages_distinct "$MSG_NO_DEST" "$MSG_NO_DEST" "$MSG_PLATFORM" "$MSG_SKILLS_DEST_ERROR" >/dev/null 2>&1; then
    fail "self-test 6e: forcing the no-destination and --user messages to collide was NOT detected — 5g's comparison is vacuous"
fi
if check_messages_distinct "$MSG_SKILLS_DEST_ERROR $MSG_NO_DEST" "$MSG_PLUGIN_USER" "$MSG_PLATFORM" "$MSG_SKILLS_DEST_ERROR" >/dev/null 2>&1; then
    fail "self-test 6e: a no-destination message containing the --skills destination-error text was NOT detected — the substring guard is vacuous"
fi
pass "self-test 6e: check_messages_distinct correctly flags a collided pair and a copy-pasted substring"

# --- 7. hermeticity guard: repo git status unchanged after the whole run ---
say "7. Confirming $REPO_ROOT git status is unchanged after the whole run"
AFTER_STATUS=$(git -C "$REPO_ROOT" status --porcelain)
[ "$BEFORE_STATUS" = "$AFTER_STATUS" ] ||
    fail "repo git status changed during this run — emission must be fully isolated to \$TMP.\nbefore:\n$BEFORE_STATUS\nafter:\n$AFTER_STATUS"
pass "repo git status unchanged"

say "All plugin-distribution checks passed."
