#!/bin/sh
# Hermetic regression for the distribution boundary: does a repo that only
# ran `rdm agent-config claude --skills` actually get a working autonomous
# lane, or does something only look right because it's checked in here?
#
# Every other verify-*.sh in this repo drives `.claude/` — the dogfooded,
# hand-maintained copies. None of them ever runs the actual generator and
# inspects what it writes into a fresh, unrelated repo. This script closes
# that gap. It:
#
#   1. Runs the real `target/debug/rdm agent-config claude --skills` (both
#      the plain CLI variant and the `--mcp` variant) into hermetic temp
#      dirs, under a project name ("distro-check") distinct from this repo's
#      own dogfood project ("rdm"), and asserts the whole run never touches
#      this repo's working tree (`git status --porcelain` before/after).
#   2. STRUCTURAL: asserts all 11 skills land at their conventional paths
#      with minimally-valid frontmatter, and both workflow scripts land
#      under `.claude/workflows/`.
#   3. BYTE-IDENTITY: asserts the 2 emitted workflow scripts are byte-for-byte
#      identical to this repo's own `.claude/workflows/*.js` — the one
#      surface `generate_workflows` promises to emit verbatim (see its doc
#      comment in `rdm-core/src/agent_config.rs`).
#   3b. CLEANUP (nothing superseded present): re-emitting into a tree that
#      holds only current files is idempotent and non-destructive —
#      re-emission prints no `Removed` line, the two current engine scripts
#      are rewritten (not removed — still byte-identical to source), and an
#      unrelated user-authored file planted in the same output directory
#      survives byte-for-byte. The complementary "a stale file IS removed"
#      case is section 5j, which seeds real pre-rename bodies.
#   4. SEMANTIC: asserts every literal `.claude/workflows/<name>.js`
#      reference inside an emitted skill resolves to a real file in the SAME
#      emitted tree — a shim can never ship pointing at an absent workflow.
#      This is checked with an explicit occurrence floor (so the check
#      cannot pass vacuously on zero matches) and per-file exact-reference
#      assertions for the 2 skills known to carry a reference
#      (rdm-dispatch-phase and rdm-do -> the dispatch engine, $DISPATCH_WF).
#      This section
#      also asserts, name-generically and across EVERY emitted skill (not
#      just rdm-autopilot), that no skill's prose instructs invoking a
#      Workflow whose name does not resolve to a file in the same emitted
#      `.claude/workflows/` tree. rdm-autopilot is the prose `rdm-autopilot`
#      skill (workflow-orchestration roadmap, phase 3) and composes only
#      `rdm-wf-dispatch-phase` downstream; the `rdm-wf-estimate` pre-pass it also runs
#      locally is intentionally dropped from the distributed template (see
#      docs/workflow-vs-prose-boundary.md), so no emitted skill's prose may
#      instruct invoking a Workflow named `rdm-wf-estimate` either — the same
#      hazard that got `autopilot.js` itself retired from this surface.
#   5j. SUPERSEDED CLEANUP END-TO-END: seeds an output directory with the
#      real PRE-RENAME bodies of the two renamed engines plus the retired
#      `autopilot.js` orphan (recovered from git history), re-emits into it,
#      and asserts all three are removed while a same-shaped
#      `not-superseded.js` and a user-authored `custom-local.js` both
#      survive. Carries a non-vacuity precondition (all five present before
#      the emit) and a planted-corruption self-test (re-plant one superseded
#      file, confirm the removal assertion turns red).
#   5. PLANTED-MUTATION SELF-TESTS: corrupts a scratch copy of the emission
#      (one byte appended to a workflow script; one shim reference
#      rewritten to a typo'd filename; an "invoke the autopilot workflow"
#      sentence appended to rdm-autopilot/SKILL.md; an "invoke the
#      `nonexistent-workflow` Workflow" sentence appended to a DIFFERENT
#      skill, rdm-dispatch-phase/SKILL.md) and asserts every check above
#      turns red on the corrupted copy — proving none of the gates is
#      vacuous, including that the generalized invocation-resolution guard
#      catches both an unfamiliar Workflow name and a skill other than
#      rdm-autopilot.
#   6. NEGATIVE / PLAN-REPO INDEPENDENCE: Pi emission never writes
#      `.claude/workflows` (no Workflow-tool runtime) and never prints a
#      cleanup-report line; `--user` emission never writes
#      `.claude/workflows` either (the scripts hardcode a target repo's own
#      binary path, so they're `--out`-only) and never prints a
#      cleanup-report line; and emission succeeds even when `RDM_ROOT` points
#      at a path that does not exist, since `--skills` emission never needs
#      the plan repo.
#
# Deliberately OUT OF SCOPE (see CLAUDE.md's Dogfooding section and the
# `distribute-workflow-lane` roadmap's phase-4 notes for why): a full-body
# diff of the emitted skills against this repo's checked-in
# `.claude/skills/*/SKILL.md`. 9 of the 11 dogfood copies are intentionally
# hand-customized for this repo's own dev-build usage (`./target/debug/rdm`
# paths, a dev-repo banner, `rdm-plan-review`'s hand-authored Gate section)
# and will never be byte-identical to generic `--project`-only generator
# output. Only the workflow scripts carry a byte-identical contract; skills
# are gated structurally (existence + frontmatter + reference resolution)
# instead.
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo).

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

SKILLS="rdm-roadmap rdm-do rdm-document rdm-review rdm-estimate rdm-dispatch-phase rdm-autopilot rdm-land rdm-revise rdm-backlog rdm-plan-review"
# The engine filenames are named ONCE here; every other site in this script
# routes through these two variables. The self-check right below fails loudly
# if a future edit re-hardcodes either literal a second time.
DISPATCH_WF="rdm-wf-dispatch-phase.js"
REVIEW_WF="rdm-wf-review-refute-fix.js"
WORKFLOWS="$DISPATCH_WF $REVIEW_WF"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 0. single-place self-check on this script's own engine literals -------
# The `rdm-wf-` rename made "one engine name, one place" a contract, not a
# preference: a re-hardcoded literal is exactly how a future half-rename slips
# through. Each engine filename may appear EXACTLY ONCE in this file — in its
# variable assignment above.
for engine_literal in $WORKFLOWS; do
    # The variable assignment itself is the one permitted occurrence; the
    # `$WORKFLOWS` expansion above reaches it without adding a second literal.
    occurrences=$(grep -c -- "$engine_literal" "$0" || true)
    [ "$occurrences" -eq 1 ] ||
        fail "0: '$engine_literal' appears $occurrences time(s) in $0 — it must be named in exactly one place (\$DISPATCH_WF / \$REVIEW_WF); route the new site through the variable"
done
pass "0: each engine filename is named exactly once in this script"

# --- helpers -------------------------------------------------------------

# Asserts every $WORKFLOWS engine script under <emitted_dir>/.claude/workflows
# is byte-identical to this repo's own .claude/workflows/*.js. Prints a
# diagnostic per drifted file and returns nonzero if any differ.
check_workflows_byte_identical() {
    dir=$1
    drifted=0
    for name in $WORKFLOWS; do
        if ! diff -q "$REPO_ROOT/.claude/workflows/$name" "$dir/.claude/workflows/$name" >/dev/null 2>&1; then
            echo "  drift: $dir/.claude/workflows/$name differs from $REPO_ROOT/.claude/workflows/$name" >&2
            drifted=1
        fi
    done
    [ "$drifted" -eq 0 ]
}

# Scans every *.md under <skills_dir>/*/SKILL.md for literal
# `.claude/workflows/<name>.js` references and asserts each resolves to a
# real file under <workflows_dir>. Sets SHIM_REF_COUNT and
# SHIM_REF_UNRESOLVED as globals for the caller to inspect (e.g. the >= 4
# occurrence floor), and returns nonzero if any reference is unresolved.
check_shim_refs_resolve() {
    skills_dir=$1
    workflows_dir=$2
    refs_scratch="$TMP/.shim-refs-scratch.txt"
    SHIM_REF_COUNT=0
    SHIM_REF_UNRESOLVED=0
    for md in "$skills_dir"/*/SKILL.md; do
        [ -f "$md" ] || continue
        grep -oE '\.claude/workflows/[A-Za-z0-9_-]+\.js' "$md" >"$refs_scratch" 2>/dev/null || : >"$refs_scratch"
        while IFS= read -r ref; do
            [ -n "$ref" ] || continue
            SHIM_REF_COUNT=$((SHIM_REF_COUNT + 1))
            name=${ref#.claude/workflows/}
            if [ ! -f "$workflows_dir/$name" ]; then
                echo "  unresolved: $md references $ref -> missing $workflows_dir/$name" >&2
                SHIM_REF_UNRESOLVED=$((SHIM_REF_UNRESOLVED + 1))
            fi
        done <"$refs_scratch"
    done
    rm -f "$refs_scratch"
    [ "$SHIM_REF_UNRESOLVED" -eq 0 ]
}

# Scans every emitted SKILL.md under <skills_dir> for an unsubstituted `{t_*}`
# MCP-tool placeholder. `render_mcp_skill` substitutes only the placeholders
# named in that skill's tuple list, so a dropped tuple silently ships the
# literal `{t_phase_list}` text — including inside `allowed-tools` frontmatter,
# where it names no real tool. Prints a diagnostic per offender and returns
# nonzero if any survive.
check_no_unsubstituted_placeholders() {
    skills_dir=$1
    leaked=0
    for md in "$skills_dir"/*/SKILL.md; do
        [ -f "$md" ] || continue
        if grep -n '{t_' "$md" >&2; then
            echo "  unsubstituted: $md ships a literal {t_*} tool placeholder (see line above)" >&2
            leaked=1
        fi
    done
    [ "$leaked" -eq 0 ]
}

# Scans EVERY *.md under <skills_dir>/*/SKILL.md for the "Invoke(ing) the
# `<name>` Workflow" phrasing every genuine invocation call site across all 11
# templates already uses consistently (verified by grep against the real
# templates), pulls out `<name>`, and asserts <workflows_dir>/<name>.js
# exists. Name-generic and skill-generic: it is not specific to "autopilot"
# or to the rdm-autopilot skill. Sets INVOCATION_COUNT/INVOCATION_UNRESOLVED
# as globals for the caller to inspect, and returns nonzero if any invoked
# name fails to resolve.
#
# The match requires "invok(e|ing) the" contiguous with the backtick-quoted
# name immediately followed by "Workflow"/"workflow" (allowing surrounding
# `**` bold markers) -- NOT just co-occurrence anywhere in the file. This
# deliberately does not trip on purely descriptive, non-invoking mentions
# like "unlike the `autopilot` workflow's advance/park loop" (skill-do), or
# "never invokes an `rdm-wf-estimate` Workflow" (rdm-autopilot's own "why no
# estimate pre-pass" note) -- neither is followed immediately by the
# backtick-quoted name after an "invok(e|ing) the" prefix.
check_workflow_invocations_resolve() {
    skills_dir=$1
    workflows_dir=$2
    matches_scratch="$TMP/.workflow-invocations-scratch.txt"
    INVOCATION_COUNT=0
    INVOCATION_UNRESOLVED=0
    for md in "$skills_dir"/*/SKILL.md; do
        [ -f "$md" ] || continue
        # shellcheck disable=SC2016
        grep -ioE '[a-z]nvok(e|ing) the \*{0,2}`[A-Za-z0-9_-]+`\*{0,2} workflow' "$md" \
            >"$matches_scratch" 2>/dev/null || : >"$matches_scratch"
        while IFS= read -r match; do
            [ -n "$match" ] || continue
            # shellcheck disable=SC2016
            name=$(printf '%s\n' "$match" | grep -oE '`[A-Za-z0-9_-]+`' | tr -d '`')
            [ -n "$name" ] || continue
            INVOCATION_COUNT=$((INVOCATION_COUNT + 1))
            if [ ! -f "$workflows_dir/$name.js" ]; then
                echo "  unresolved: $md instructs invoking a Workflow named '$name' -> missing $workflows_dir/$name.js" >&2
                INVOCATION_UNRESOLVED=$((INVOCATION_UNRESOLVED + 1))
            fi
        done <"$matches_scratch"
    done
    rm -f "$matches_scratch"
    [ "$INVOCATION_UNRESOLVED" -eq 0 ]
}

# Minimal frontmatter validity: starts with a `---` fence, has a closing
# `---` fence, and declares a `name:` field.
assert_valid_frontmatter() {
    md=$1
    first_line=$(head -n 1 "$md")
    [ "$first_line" = "---" ] || fail "$md: frontmatter must start with '---' (found: $first_line)"
    fence_count=$(grep -c '^---$' "$md")
    [ "$fence_count" -ge 2 ] || fail "$md: frontmatter missing closing '---' fence"
    grep -q '^name:' "$md" || fail "$md: frontmatter missing a 'name:' field"
}

# --- 0. hermeticity guard: capture repo git status before any emission ------
say "0. Capturing $REPO_ROOT git status before emission (hermeticity baseline)"
BEFORE_STATUS=$(git -C "$REPO_ROOT" status --porcelain)
pass "baseline captured"

# --- 1. emit cli + mcp variants into hermetic temp dirs ---------------------
say "1. Emitting 'agent-config claude --skills' (cli and --mcp variants)"
"$RDM_BIN" agent-config claude --skills --project distro-check --out "$TMP/cli" >/dev/null
"$RDM_BIN" agent-config claude --skills --mcp --project distro-check --out "$TMP/mcp" >/dev/null
pass "emitted into $TMP/cli and $TMP/mcp"

# --- 2. structural: skills + workflows land at conventional paths ----------
say "2. Structural: all 11 skills + 2 workflow scripts present with valid frontmatter"
for variant in cli mcp; do
    for skill in $SKILLS; do
        md="$TMP/$variant/.claude/skills/$skill/SKILL.md"
        [ -f "$md" ] || fail "$variant: missing $md"
        assert_valid_frontmatter "$md"
    done
    for wf in $WORKFLOWS; do
        [ -f "$TMP/$variant/.claude/workflows/$wf" ] || fail "$variant: missing .claude/workflows/$wf"
    done
    pass "$variant: 11 skills (valid frontmatter) + 2 workflow scripts present"
done

# --- 2b. every {t_*} tool placeholder is substituted in the emitted skills --
say "2b. Substitution: no emitted skill ships a literal {t_*} tool placeholder"
for variant in cli mcp; do
    if check_no_unsubstituted_placeholders "$TMP/$variant/.claude/skills"; then
        pass "$variant: every {t_*} placeholder substituted to a real mcp__rdm__ tool"
    else
        fail "$variant: an emitted skill ships an unsubstituted {t_*} placeholder (see lines above) — a tuple is missing from its render_mcp_skill tools list"
    fi
done

# --- 3. byte-identity: emitted workflows match this repo's own copies -------
say "3. Byte-identity: emitted workflow scripts vs $REPO_ROOT/.claude/workflows"
for variant in cli mcp; do
    if check_workflows_byte_identical "$TMP/$variant"; then
        pass "$variant: workflow scripts byte-identical to source"
    else
        fail "$variant: workflow scripts drifted from $REPO_ROOT/.claude/workflows (see drift lines above)"
    fi
done

# --- 3b. cleanup mechanism (empty table): re-emission is idempotent --------
# --- and non-destructive ----------------------------------------------------
say "3b. Cleanup (nothing superseded present): re-emission is idempotent and non-destructive"

CLI_WORKFLOWS_DIR="$TMP/cli/.claude/workflows"
UNRELATED_FILE="$CLI_WORKFLOWS_DIR/notes.txt"
UNRELATED_CONTENT="these are my own notes, rdm did not write this file"
printf '%s\n' "$UNRELATED_CONTENT" >"$UNRELATED_FILE"

BEFORE_DP_SUM=$(shasum -a 256 "$CLI_WORKFLOWS_DIR/$DISPATCH_WF" | awk '{print $1}')
BEFORE_RRF_SUM=$(shasum -a 256 "$CLI_WORKFLOWS_DIR/$REVIEW_WF" | awk '{print $1}')
BEFORE_UNRELATED_SUM=$(shasum -a 256 "$UNRELATED_FILE" | awk '{print $1}')

"$RDM_BIN" agent-config claude --skills --project distro-check --out "$TMP/cli" >"$TMP/reemit-cli.log"

if grep -q '^Removed ' "$TMP/reemit-cli.log"; then
    fail "3b: re-emission into a tree holding no superseded file printed a 'Removed' line — the cleanup must only ever touch names SUPERSEDED_WORKFLOWS carries:\n$(cat "$TMP/reemit-cli.log")"
fi
pass "3b: re-emission printed no 'Removed' line (no superseded file present)"

if ! check_workflows_byte_identical "$TMP/cli"; then
    fail "3b: re-emitted workflow scripts drifted from $REPO_ROOT/.claude/workflows (see drift lines above) — rewrite-not-remove is violated"
fi
AFTER_DP_SUM=$(shasum -a 256 "$CLI_WORKFLOWS_DIR/$DISPATCH_WF" | awk '{print $1}')
AFTER_RRF_SUM=$(shasum -a 256 "$CLI_WORKFLOWS_DIR/$REVIEW_WF" | awk '{print $1}')
[ "$BEFORE_DP_SUM" = "$AFTER_DP_SUM" ] ||
    fail "3b: $DISPATCH_WF checksum changed across re-emission ($BEFORE_DP_SUM -> $AFTER_DP_SUM)"
[ "$BEFORE_RRF_SUM" = "$AFTER_RRF_SUM" ] ||
    fail "3b: $REVIEW_WF checksum changed across re-emission ($BEFORE_RRF_SUM -> $AFTER_RRF_SUM)"
pass "3b: both conventionally-named workflow files were rewritten, not removed, and stayed byte-identical to source"

[ -f "$UNRELATED_FILE" ] || fail "3b: planted unrelated file $UNRELATED_FILE was removed — unrelated-file guarantee is violated"
AFTER_UNRELATED_SUM=$(shasum -a 256 "$UNRELATED_FILE" | awk '{print $1}')
[ "$BEFORE_UNRELATED_SUM" = "$AFTER_UNRELATED_SUM" ] ||
    fail "3b: planted unrelated file $UNRELATED_FILE changed across re-emission — it must never be touched"
pass "3b: planted unrelated file survived re-emission byte-for-byte, untouched"

# --- 4. semantic: every shim reference resolves within the emitted tree ----
say "4. Semantic: every emitted shim's workflow reference resolves in-tree"
for variant in cli mcp; do
    skills_dir="$TMP/$variant/.claude/skills"
    workflows_dir="$TMP/$variant/.claude/workflows"
    if check_shim_refs_resolve "$skills_dir" "$workflows_dir"; then
        pass "$variant: all $SHIM_REF_COUNT shim reference(s) resolve"
    else
        fail "$variant: $SHIM_REF_UNRESOLVED unresolved shim reference(s) (see lines above)"
    fi
    [ "$SHIM_REF_COUNT" -ge 3 ] ||
        fail "$variant: expected >= 3 total shim references (dispatch-phase skill x1, do skill x2), found $SHIM_REF_COUNT — check is not vacuous only if this floor holds"

    grep -qF ".claude/workflows/$DISPATCH_WF" "$skills_dir/rdm-dispatch-phase/SKILL.md" ||
        fail "$variant: rdm-dispatch-phase/SKILL.md must reference .claude/workflows/$DISPATCH_WF"
    grep -qF ".claude/workflows/$DISPATCH_WF" "$skills_dir/rdm-do/SKILL.md" ||
        fail "$variant: rdm-do/SKILL.md must reference .claude/workflows/$DISPATCH_WF"
    pass "$variant: rdm-dispatch-phase/rdm-do carry their expected exact references"

    # Every emitted skill's prose (not just rdm-autopilot's) must never
    # instruct invoking a Workflow whose name does not resolve to a file in
    # this same emitted tree -- that call would target a file this
    # generator does not emit and would fail at the exact point the skill's
    # contract depends on. rdm-autopilot composes only `rdm-wf-dispatch-phase`
    # downstream; its `rdm-wf-estimate` pre-pass is intentionally dropped from the
    # distributed template (see docs/workflow-vs-prose-boundary.md), so it
    # must never instruct invoking `rdm-wf-estimate` either -- the same hazard that
    # got `autopilot.js` itself retired from this surface.
    if check_workflow_invocations_resolve "$skills_dir" "$workflows_dir"; then
        pass "$variant: all $INVOCATION_COUNT Workflow-invocation instruction(s) across every emitted skill resolve"
    else
        fail "$variant: $INVOCATION_UNRESOLVED unresolved Workflow-invocation instruction(s) (see lines above)"
    fi
    [ "$INVOCATION_COUNT" -ge 5 ] ||
        fail "$variant: expected >= 5 total Workflow-invocation instructions across all skills, found $INVOCATION_COUNT — check is not vacuous only if this floor holds"
done

# --- 5. planted-mutation self-tests: prove neither gate above is vacuous ---
say "5a. Self-test: planted byte corruption in an emitted workflow script"
SCRATCH_WF="$TMP/scratch-corrupt-workflow"
rm -rf "$SCRATCH_WF"
cp -R "$TMP/cli" "$SCRATCH_WF"
printf '\n// planted corruption for verify-agent-config-distribution.sh self-test\n' \
    >>"$SCRATCH_WF/.claude/workflows/$DISPATCH_WF"
if check_workflows_byte_identical "$SCRATCH_WF" >/dev/null 2>&1; then
    fail "self-test A: planted corruption in $DISPATCH_WF was NOT detected — byte-identical gate is vacuous"
fi
pass "self-test A: planted corruption in $DISPATCH_WF correctly turned the byte-identical gate red"

say "5b. Self-test: planted shim reference typo'd to a nonexistent filename"
SCRATCH_SHIM="$TMP/scratch-corrupt-shim"
rm -rf "$SCRATCH_SHIM"
cp -R "$TMP/cli" "$SCRATCH_SHIM"
sed "s|\.claude/workflows/$DISPATCH_WF|.claude/workflows/typo-$DISPATCH_WF|" \
    "$SCRATCH_SHIM/.claude/skills/rdm-dispatch-phase/SKILL.md" >"$SCRATCH_SHIM/.claude/skills/rdm-dispatch-phase/SKILL.md.new"
mv "$SCRATCH_SHIM/.claude/skills/rdm-dispatch-phase/SKILL.md.new" "$SCRATCH_SHIM/.claude/skills/rdm-dispatch-phase/SKILL.md"
if check_shim_refs_resolve "$SCRATCH_SHIM/.claude/skills" "$SCRATCH_SHIM/.claude/workflows" >/dev/null 2>&1; then
    fail "self-test B: planted reference typo was NOT detected — shim-reference gate is vacuous"
fi
pass "self-test B: planted reference typo correctly turned the shim-reference gate red"

say "5c. Self-test: planted unsubstituted {t_*} placeholder in an emitted skill"
SCRATCH_PH="$TMP/scratch-unsubstituted-placeholder"
rm -rf "$SCRATCH_PH"
cp -R "$TMP/mcp" "$SCRATCH_PH"
# Mimic exactly what a dropped ("t_next", "rdm_next") tuple in
# skill_autopilot_mcp would produce: the literal placeholder survives
# rendering. (Not {t_phase_list}: that tuple — and the tool it named — was
# removed from rdm-autopilot's MCP template entirely once the estimate
# pre-pass was dropped downstream, so it no longer resolves anything here
# and planting it would make this self-test vacuous.)
sed 's/mcp__rdm__rdm_next/{t_next}/g' \
    "$SCRATCH_PH/.claude/skills/rdm-autopilot/SKILL.md" >"$SCRATCH_PH/.claude/skills/rdm-autopilot/SKILL.md.new"
mv "$SCRATCH_PH/.claude/skills/rdm-autopilot/SKILL.md.new" "$SCRATCH_PH/.claude/skills/rdm-autopilot/SKILL.md"
grep -q '{t_next}' "$SCRATCH_PH/.claude/skills/rdm-autopilot/SKILL.md" ||
    fail "self-test C: could not plant the placeholder — rdm-autopilot no longer resolves {t_next}, so this self-test is vacuous"
if check_no_unsubstituted_placeholders "$SCRATCH_PH/.claude/skills" >/dev/null 2>&1; then
    fail "self-test C: planted {t_next} placeholder was NOT detected — the substitution gate is vacuous"
fi
pass "self-test C: planted {t_next} placeholder correctly turned the substitution gate red"

say "5d. Self-test: planted invocation of a nonexistent Workflow name ('autopilot') in rdm-autopilot/SKILL.md"
SCRATCH_AP="$TMP/scratch-autopilot-workflow-invocation"
rm -rf "$SCRATCH_AP"
cp -R "$TMP/cli" "$SCRATCH_AP"
# shellcheck disable=SC2016
printf '\nInvoke the `autopilot` workflow via the Workflow tool. Planted for verify-agent-config-distribution.sh self-test D.\n' \
    >>"$SCRATCH_AP/.claude/skills/rdm-autopilot/SKILL.md"
if check_workflow_invocations_resolve "$SCRATCH_AP/.claude/skills" "$SCRATCH_AP/.claude/workflows" >/dev/null 2>&1; then
    fail "self-test D: planted 'autopilot' workflow-invocation instruction was NOT detected — the check is vacuous"
fi
pass "self-test D: planted 'autopilot' workflow-invocation instruction correctly turned the check red"

say "5e. Self-test: planted invocation of a nonexistent Workflow name ('nonexistent-workflow') in a DIFFERENT skill (rdm-dispatch-phase)"
SCRATCH_DP="$TMP/scratch-dispatch-phase-workflow-invocation"
rm -rf "$SCRATCH_DP"
cp -R "$TMP/cli" "$SCRATCH_DP"
# shellcheck disable=SC2016
printf '\nInvoke the `nonexistent-workflow` Workflow via the Workflow tool. Planted for verify-agent-config-distribution.sh self-test E.\n' \
    >>"$SCRATCH_DP/.claude/skills/rdm-dispatch-phase/SKILL.md"
if check_workflow_invocations_resolve "$SCRATCH_DP/.claude/skills" "$SCRATCH_DP/.claude/workflows" >/dev/null 2>&1; then
    fail "self-test E: planted 'nonexistent-workflow' invocation in rdm-dispatch-phase/SKILL.md was NOT detected — the generalized guard is vacuous outside rdm-autopilot and/or outside the literal name 'autopilot'"
fi
pass "self-test E: planted 'nonexistent-workflow' invocation in a non-autopilot skill correctly turned the generalized check red"

say "5f. Self-test: a BARE, pre-rename Workflow name must not resolve (half-completed-sweep regression)"
SCRATCH_BARE="$TMP/scratch-bare-prerename-invocation"
rm -rf "$SCRATCH_BARE"
cp -R "$TMP/cli" "$SCRATCH_BARE"
# The pre-rename bare engine name. After the `rdm-wf-` rename no file by this
# name is emitted, so a shim still instructing "invoke the `dispatch-phase`
# Workflow" would dispatch into thin air — the precise failure mode of a sweep
# that renamed the FILES but missed a shim's invocation prose.
# shellcheck disable=SC2016
printf '\nInvoke the `dispatch-phase` Workflow via the Workflow tool. Planted for verify-agent-config-distribution.sh self-test F.\n' \
    >>"$SCRATCH_BARE/.claude/skills/rdm-dispatch-phase/SKILL.md"
if check_workflow_invocations_resolve "$SCRATCH_BARE/.claude/skills" "$SCRATCH_BARE/.claude/workflows" >/dev/null 2>&1; then
    fail "self-test F: a planted BARE pre-rename 'dispatch-phase' invocation still resolved — a half-completed rename would ship undetected"
fi
pass "self-test F: a planted BARE pre-rename 'dispatch-phase' invocation correctly failed to resolve"

# --- 5g. no emitted or in-repo name was double-prefixed --------------------
say "5g. No front-door skill was renamed: zero double-prefixed names anywhere"
# The headline trap of the `rdm-wf-` rename: `rdm-dispatch-phase` (skill,
# frozen) strictly contains `dispatch-phase` (engine, renamed), so an
# unanchored substitution corrupts every front-door name into a
# double-prefixed one. tests/fixtures is excluded: it holds a frozen corpus
# that quotes this very corruption as prose. The two patterns are COMPOSED
# from a prefix variable rather than written literally, so this gate can
# never match its own source text.
NS=rdm-
if grep -rn "${NS}${NS}\|${NS}wf-${NS}" "$REPO_ROOT" \
    --exclude-dir=target --exclude-dir=.git --exclude-dir=node_modules \
    --exclude-dir=fixtures >"$TMP/double-prefix.txt" 2>/dev/null; then
    fail "5g: found double-prefixed names (an unanchored substitution corrupted a front-door skill name):\n$(cat "$TMP/double-prefix.txt")"
fi
pass "5g: no double-prefixed ($NS$NS / ${NS}wf-$NS) name anywhere in the repo"

ACTUAL_SKILL_DIRS=$(find "$REPO_ROOT/.claude/skills" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort | tr '\n' ' ')
# shellcheck disable=SC2086  # SKILLS is a deliberately word-split name list
EXPECTED_SKILL_DIRS=$(printf '%s\n' $SKILLS | sort | tr '\n' ' ')
[ "$ACTUAL_SKILL_DIRS" = "$EXPECTED_SKILL_DIRS" ] ||
    fail "5g: .claude/skills/ directory names changed.\n  expected: $EXPECTED_SKILL_DIRS\n  actual:   $ACTUAL_SKILL_DIRS"
pass "5g: all 11 rdm-* skill directory names are unchanged"

# --- 5h. single-sourcing statics on the Rust and generator surfaces --------
say "5h. Single-sourcing: generate_workflows(), the generators, and the non-lists"
AGENT_CONFIG_RS="$REPO_ROOT/rdm-core/src/agent_config.rs"
INCLUDE_COUNT=$(grep -c 'include_str!("templates/workflows/' "$AGENT_CONFIG_RS" || true)
[ "$INCLUDE_COUNT" -eq 2 ] ||
    fail "5h: expected exactly 2 templates/workflows include_str! sites in agent_config.rs (one per SHIPPED engine), found $INCLUDE_COUNT — the four local-only engines must stay unshipped"
pass "5h: exactly 2 shipped-engine include_str! sites in generate_workflows()"

for gen in gen-workflow-review.sh gen-workflow-estimate.sh; do
    set_count=$(grep -c '^set -- ' "$REPO_ROOT/scripts/$gen" || true)
    [ "$set_count" -eq 1 ] ||
        fail "5h: $gen must keep exactly ONE consumer list (\`set --\`), found $set_count"
done
pass "5h: both real generator consumer lists are still a single list each"

# The two scripts the phase plan verified carry NO engine list at all: assert
# the absence rather than editing a list that does not exist.
[ "$(grep -cE '\.claude/workflows/[a-z-]+\.js' "$REPO_ROOT/scripts/gen-skill-review.sh" || true)" -eq 0 ] ||
    fail "5h: gen-skill-review.sh gained an engine .js reference — it lists skill TEMPLATE filenames only"
[ "$(grep -cE '\.(js|mjs)' "$REPO_ROOT/scripts/lib/mechanical-tier-check.sh" || true)" -eq 0 ] ||
    fail "5h: scripts/lib/mechanical-tier-check.sh gained a .js/.mjs reference — it must stay engine-name-free"
pass "5h: gen-skill-review.sh and mechanical-tier-check.sh carry no engine list (confirmed, untouched)"

# --- 5i. CHANGELOG records the rename as a breaking change -----------------
say "5i. CHANGELOG: the engine rename is recorded as BREAKING with automatic cleanup"
CHANGELOG="$REPO_ROOT/CHANGELOG.md"
UNRELEASED=$(awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{f=0} f' "$CHANGELOG")
check_changelog() {
    body=$1
    printf '%s' "$body" | grep -q 'BREAKING' || return 1
    printf '%s' "$body" | grep -q 'removes the superseded' || return 1
    for engine in dispatch-phase review-refute-fix backlog document estimate plan-review; do
        printf '%s' "$body" | grep -q "rdm-wf-$engine" || return 1
    done
    return 0
}
check_changelog "$UNRELEASED" ||
    fail "5i: CHANGELOG.md [Unreleased] must name all six rdm-wf-* engines, the word BREAKING, and the phrase 'removes the superseded'"
pass "5i: CHANGELOG [Unreleased] records the rename as BREAKING with automatic superseded-file removal"

# Non-vacuity: the same check must FAIL against the pre-change CHANGELOG.
PREV_CHANGELOG=$(git -C "$REPO_ROOT" show HEAD:CHANGELOG.md 2>/dev/null || true)
if [ -n "$PREV_CHANGELOG" ]; then
    PREV_UNRELEASED=$(printf '%s\n' "$PREV_CHANGELOG" | awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{f=0} f')
    if check_changelog "$PREV_UNRELEASED"; then
        fail "5i: the CHANGELOG check also passes against HEAD's CHANGELOG — it is not actually testing this change"
    fi
    pass "5i: the CHANGELOG check correctly fails against HEAD's CHANGELOG (non-vacuous)"
fi

# --- 5j. END-TO-END: a downstream re-emit removes the superseded files -----
say "5j. Superseded cleanup end-to-end: a stale downstream tree is cleaned, a custom file is not"
STALE="$TMP/stale"
STALE_WF="$STALE/.claude/workflows"
mkdir -p "$STALE_WF"
# Seed the tree with the exact PRE-RENAME bodies a past release emitted, taken
# verbatim from git rather than reconstructed: the rename changed more than the
# meta.name line, so only history holds the real bytes a downstream repo would
# be carrying.
for stale_pair in "dispatch-phase.js" "review-refute-fix.js"; do
    git -C "$REPO_ROOT" show "HEAD:rdm-core/src/templates/workflows/$stale_pair" \
        >"$STALE_WF/$stale_pair" 2>/dev/null ||
        fail "5j: could not recover the pre-rename $stale_pair body from HEAD — this harness must run against a tree where the rename is not yet committed, or the fingerprints must be regenerated"
done
# The retired orphan: no successor, recovered from the commit that removed it.
AUTOPILOT_PATH=rdm-core/src/templates/workflows/autopilot.js
AUTOPILOT_DELETED_AT=$(git -C "$REPO_ROOT" rev-list -n1 --all -- "$AUTOPILOT_PATH")
[ -n "$AUTOPILOT_DELETED_AT" ] ||
    fail "5j: could not locate the commit that retired $AUTOPILOT_PATH"
git -C "$REPO_ROOT" show "$AUTOPILOT_DELETED_AT^:$AUTOPILOT_PATH" \
    >"$STALE_WF/autopilot.js" 2>/dev/null ||
    fail "5j: could not recover the retired autopilot.js body from git history"
# Two survivors: one name the table does not carry, one purely user-authored.
printf 'export const meta = { name: "not-superseded" };\n' >"$STALE_WF/not-superseded.js"
printf 'my own local engine, rdm did not write this\n' >"$STALE_WF/custom-local.js"

# Non-vacuity: every seeded file must be present BEFORE the emit, so a
# post-emit absence can never be trivially true.
for seeded in dispatch-phase.js review-refute-fix.js autopilot.js not-superseded.js custom-local.js; do
    [ -f "$STALE_WF/$seeded" ] || fail "5j: seed failed — $seeded is not present before the emit"
done
pass "5j: all five seeded files present before the emit (non-vacuity precondition)"

"$RDM_BIN" agent-config claude --skills --project distro-check --out "$STALE" >"$TMP/stale-emit.log"

assert_stale_cleaned() {
    for removed in dispatch-phase.js review-refute-fix.js autopilot.js; do
        [ ! -e "$STALE_WF/$removed" ] || return 1
    done
    return 0
}
assert_stale_cleaned ||
    fail "5j: a superseded file survived the re-emit — cleanup did not fire:\n$(ls -1 "$STALE_WF")\n$(cat "$TMP/stale-emit.log")"
pass "5j: all three superseded files (2 renamed + 1 retired orphan) were removed"

for kept in "$DISPATCH_WF" "$REVIEW_WF"; do
    [ -f "$STALE_WF/$kept" ] || fail "5j: $kept was not emitted into the cleaned tree"
    diff -q "$REPO_ROOT/.claude/workflows/$kept" "$STALE_WF/$kept" >/dev/null ||
        fail "5j: $kept is not byte-identical to source after the cleanup emit"
done
pass "5j: both rdm-wf-* engines exist and are byte-identical to source"

[ -f "$STALE_WF/not-superseded.js" ] ||
    fail "5j: not-superseded.js was removed — the cleanup is over-broad (it must only touch names the table carries)"
[ -f "$STALE_WF/custom-local.js" ] ||
    fail "5j: custom-local.js was removed — a user-authored file must never be touched"
pass "5j: not-superseded.js and custom-local.js both survived (cleanup discriminates, it does not sweep)"

grep -q '^Removed ' "$TMP/stale-emit.log" ||
    fail "5j: the emit removed files but printed no 'Removed ' report line"
pass "5j: the emit reported its removals"

# Planted-corruption self-test: re-plant a superseded file after the emit and
# confirm the SAME assertion helper turns red — proving 5j's removal check is
# not passing for some reason unrelated to the cleanup.
printf 'replanted\n' >"$STALE_WF/dispatch-phase.js"
if assert_stale_cleaned; then
    fail "5j self-test: a re-planted dispatch-phase.js did NOT turn the removal assertion red — the assertion is vacuous"
fi
rm -f "$STALE_WF/dispatch-phase.js"
pass "5j self-test: a re-planted superseded file correctly turns the removal assertion red"

# --- 6. negative checks: platform/scope boundaries + plan-repo independence -
say "6a. Negative: Pi emission never writes .claude/workflows or prints a cleanup report"
"$RDM_BIN" agent-config pi --skills --project distro-check --out "$TMP/pi" >"$TMP/pi-emit.log"
if [ -d "$TMP/pi/.claude/workflows" ]; then
    fail "Pi emission must not write .claude/workflows (Pi has no Workflow-tool runtime)"
fi
if grep -qE '^(Removed |Skipped |Failed to remove )' "$TMP/pi-emit.log"; then
    fail "Pi emission printed a cleanup-report line — cleanup must never run outside Platform::Claude && !user:\n$(cat "$TMP/pi-emit.log")"
fi
pass "Pi emission has no .claude/workflows directory and prints no cleanup-report line"

say "6b. Negative: --user emission never writes .claude/workflows or prints a cleanup report"
mkdir -p "$TMP/user-home"
if HOME="$TMP/user-home" "$RDM_BIN" agent-config claude --skills --user >"$TMP/user-emit.log" 2>&1; then
    if [ -d "$TMP/user-home/.claude/workflows" ]; then
        fail "--user emission must not write .claude/workflows (scripts are --out-only, not user-global)"
    fi
    if grep -qE '^(Removed |Skipped |Failed to remove )' "$TMP/user-emit.log"; then
        fail "--user emission printed a cleanup-report line — cleanup must never run against --user:\n$(cat "$TMP/user-emit.log")"
    fi
    pass "--user emission has no .claude/workflows directory and prints no cleanup-report line"
else
    cat "$TMP/user-emit.log" >&2
    fail "--user emission failed unexpectedly"
fi

say "6c. Plan-repo independence: emission succeeds with RDM_ROOT pointed at a nonexistent path"
if RDM_ROOT="$TMP/does-not-exist-plan-repo" "$RDM_BIN" agent-config claude --skills --project distro-check --out "$TMP/no-plan-repo" >"$TMP/no-plan-repo.log" 2>&1; then
    pass "emission succeeded with a nonexistent RDM_ROOT (mirrors agent_config_skills_does_not_require_plan_repo)"
else
    cat "$TMP/no-plan-repo.log" >&2
    fail "emission failed with a nonexistent RDM_ROOT — --skills emission must not require the plan repo to exist"
fi

# --- 7. hermeticity guard: repo git status unchanged after the whole run ---
# --- 6d. HOIST ARGS on the three REAL shims --------------------------------
# Phase 3 of the workflow-token-reduction roadmap teaches the Workflow-invoking
# shims to gather mechanical values themselves and pass them through `args`, so
# the workflow never spawns a dedicated subagent for them
# (docs/mechanical-agent-inventory.md).
#
# SCOPE: this check is deliberately limited to the THREE emitted skills that are
# actually Workflow shims — rdm-autopilot, rdm-dispatch-phase and rdm-do. The
# other five (rdm-plan-review, rdm-backlog, rdm-document, rdm-review,
# rdm-estimate) are NOT shims on the distribution surface: their templates still
# dispatch their own Agent/Bash prose and contain zero `.claude/workflows`
# references. Their hoists live only in this repo's LOCAL dogfood copies and are
# gated by each workflow's own harness (verify-workflow-{review,backlog,document,
# estimate,review-outcome}.sh). Converting those ten templates is tracked by task
# convert-remaining-skill-templates-to-workflow-shims — extending this check to
# them would fail immediately and pressure an out-of-scope conversion.
say "6d. Hoist args: the three real Workflow shims gather and pass their optional args"

# assert_shim_hoists <emitted-root> <variant> — each of the three shims must
# name the arg keys it passes AND the command/tool it gathers them with. The MCP
# variant deliberately omits the model-derived hoists (no MCP model-resolve
# tool), so the two variants have different, explicitly-listed expectations.
assert_shim_hoists() {
    root=$1
    variant=$2
    HOIST_REF_COUNT=0
    HOIST_FAILURE=""

    _need() {
        f=$1
        needle=$2
        if grep -qF -e "$needle" "$f"; then
            HOIST_REF_COUNT=$((HOIST_REF_COUNT + 1))
        else
            HOIST_FAILURE="$variant: $(basename "$(dirname "$f")") is missing '$needle'"
            return 1
        fi
    }

    ap="$root/.claude/skills/rdm-autopilot/SKILL.md"
    dp="$root/.claude/skills/rdm-dispatch-phase/SKILL.md"
    do_="$root/.claude/skills/rdm-do/SKILL.md"

    # rdm-autopilot: next on both variants. No mechanicalModel/phaseList hoist
    # any longer on either variant — the distributed template's `rdm-wf-estimate`
    # pre-pass is intentionally dropped downstream (see
    # docs/workflow-vs-prose-boundary.md), so there is nothing left to feed it.
    _need "$ap" 'next' || return 1
    if [ "$variant" = cli ]; then
        _need "$ap" 'rdm next --roadmap <slug> --format json' || return 1
    fi
    # `next` must be documented as one-shot on both variants, or a caller could
    # cache it and re-dispatch the same phase forever.
    _need "$ap" 'one-shot, on the first loop iteration only' || return 1
    # dispatch-phase's `rdmBin` arg is REQUIRED and fail-closed (no ambient PATH
    # fallback), so an emitted shim that omits it hard-breaks the downstream lane
    # on its first dispatch. Asserted on BOTH variants and for ALL THREE shims —
    # it is not a model-derived hoist, so it sits outside the cli-only guards.
    _need "$ap" 'rdmBin' || return 1
    if [ "$variant" = mcp ]; then
        # MCP has no `rdm phase show` CLI command to read a write back with, so
        # the advance/park confirmation step needs its own dedicated tool.
        _need "$ap" 'mcp__rdm__rdm_phase_show' || return 1
        # The advance/park read-back calls must use the same project/roadmap/
        # phase argument shape every other MCP template uses (server-side
        # PhaseUpdateParams/PhaseParams both require `project` and `phase`,
        # never a `stem` field) — a prior regression sent `stem:` with no
        # `project:` and would have failed against the real MCP server.
        _need "$ap" 'project: "distro-check", roadmap: "<slug>", phase: S' || return 1
        if grep -qF 'with `stem: S,' "$ap"; then
            HOIST_FAILURE="$variant: rdm-autopilot regressed to the wrong 'stem:' MCP arg shape"
            return 1
        fi
    fi

    # rdm-dispatch-phase: alreadyInProgress + rdmBin on both; phaseMeta/taskMeta
    # CLI only.
    _need "$dp" 'alreadyInProgress' || return 1
    _need "$dp" 'rdmBin' || return 1
    if [ "$variant" = cli ]; then
        _need "$dp" 'phaseMeta' || return 1
        _need "$dp" 'taskMeta' || return 1
        _need "$dp" 'phase show <phase> --roadmap <slug>' || return 1
        _need "$dp" 'rdm model resolve mechanical' || return 1
        _need "$dp" '--status in-progress' || return 1
    fi

    # rdm-do --auto: same contract, both flows.
    _need "$do_" 'alreadyInProgress' || return 1
    _need "$do_" 'rdmBin' || return 1
    if [ "$variant" = cli ]; then
        _need "$do_" 'phaseMeta' || return 1
        _need "$do_" 'taskMeta' || return 1
        _need "$do_" 'rdm model resolve mechanical' || return 1
    fi

    return 0
}

for variant in cli mcp; do
    if assert_shim_hoists "$TMP/$variant" "$variant"; then
        pass "$variant: all $HOIST_REF_COUNT hoist-arg reference(s) present across the three real shims"
    else
        fail "$HOIST_FAILURE"
    fi
    # Occurrence floor, so the check can never pass vacuously: CLI asserts
    # >= 16 references, MCP >= 9 (raised from 13/6 by the project-agnostic-lane
    # roadmap, which added one REQUIRED `rdmBin` needle to each of the three
    # shims on both variants; recomputed after the `rdm-wf-estimate` pre-pass —
    # and its mechanicalModel/phaseList hoist — was dropped from the
    # distributed rdm-autopilot template; MCP retains the rdm_phase_show
    # read-back hoist, added alongside the {t_phase_show} placeholder, and
    # the project/roadmap/phase argument-shape check on the advance/park
    # read-back calls, added after those calls were found using the wrong
    # `stem`-keyed, `project`-less argument shape). A drop below the floor
    # means a shim silently stopped gathering.
    if [ "$variant" = cli ]; then
        [ "$HOIST_REF_COUNT" -ge 16 ] ||
            fail "cli: expected >= 16 hoist-arg references across the three real shims, found $HOIST_REF_COUNT"
    else
        [ "$HOIST_REF_COUNT" -ge 9 ] ||
            fail "mcp: expected >= 9 hoist-arg references across the three real shims, found $HOIST_REF_COUNT"
    fi
done
pass "hoist-arg occurrence floors hold for both variants"

# Negative: the five NON-shim skills must NOT have been dragged into this — if a
# future edit turns them into shims, that is a deliberate change belonging to
# task convert-remaining-skill-templates-to-workflow-shims, and this assertion
# is the reminder to move their checks here at the same time.
for variant in cli mcp; do
    for skill in rdm-plan-review rdm-backlog rdm-document rdm-review rdm-estimate; do
        md="$TMP/$variant/.claude/skills/$skill/SKILL.md"
        if grep -qF '.claude/workflows' "$md"; then
            fail "$variant: $skill became a Workflow shim — move its hoist-arg check into section 6d (see task convert-remaining-skill-templates-to-workflow-shims)"
        fi
    done
done
pass "the five non-shim skills are still non-shims — their hoists correctly stay on the local dogfood copies"

# --- 6e. Self-test: a typo'd arg key in a real shim must be caught ----------
say "6e. Self-test: planted typo in a shim's hoist-arg key"
cp -R "$TMP/cli" "$TMP/cli-hoist-typo"
sed 's/phaseMeta/phaseMata/g' "$TMP/cli/.claude/skills/rdm-dispatch-phase/SKILL.md" \
    >"$TMP/cli-hoist-typo/.claude/skills/rdm-dispatch-phase/SKILL.md"
if assert_shim_hoists "$TMP/cli-hoist-typo" cli; then
    fail "6e: hoist-arg check did not detect a typo'd 'phaseMeta' key — the check is vacuous"
fi
pass "6e: hoist-arg check detects a typo'd arg key ($HOIST_FAILURE)"

# Self-test: mangling the REQUIRED rdmBin key in each of the three emitted shims
# in turn must be caught — one shim carrying it cannot cover for another.
for shim in rdm-dispatch-phase rdm-do rdm-autopilot; do
    rm -rf "$TMP/cli-rdmbin-typo"
    cp -R "$TMP/cli" "$TMP/cli-rdmbin-typo"
    sed 's/rdmBin/rdmBn/g' "$TMP/cli/.claude/skills/$shim/SKILL.md" \
        >"$TMP/cli-rdmbin-typo/.claude/skills/$shim/SKILL.md"
    if assert_shim_hoists "$TMP/cli-rdmbin-typo" cli; then
        fail "6e: hoist-arg check did not detect a mangled 'rdmBin' key in $shim — the check is vacuous"
    fi
done
pass "6e: hoist-arg check detects a mangled rdmBin key in each of the three shims independently"

say "6f. Self-test: a shim that stops gathering must be caught"
cp -R "$TMP/cli" "$TMP/cli-hoist-drop"
sed 's/rdm next --roadmap <slug> --format json/rdm next --roadmap <slug> --format jso/g' \
    "$TMP/cli/.claude/skills/rdm-autopilot/SKILL.md" \
    >"$TMP/cli-hoist-drop/.claude/skills/rdm-autopilot/SKILL.md"
if assert_shim_hoists "$TMP/cli-hoist-drop" cli; then
    fail "6f: hoist-arg check did not detect a mangled gathering command — the check is vacuous"
fi
pass "6f: hoist-arg check detects a mangled gathering command ($HOIST_FAILURE)"

say "7. Confirming $REPO_ROOT git status is unchanged after the whole run"
AFTER_STATUS=$(git -C "$REPO_ROOT" status --porcelain)
if [ "$BEFORE_STATUS" != "$AFTER_STATUS" ]; then
    printf 'before:\n%s\nafter:\n%s\n' "$BEFORE_STATUS" "$AFTER_STATUS" >&2
    fail "$REPO_ROOT git status changed during this run — the harness must only write under its own mktemp -d"
fi
pass "repo git status unchanged (hermetic)"

say "verify-agent-config-distribution.sh: ALL GREEN"
