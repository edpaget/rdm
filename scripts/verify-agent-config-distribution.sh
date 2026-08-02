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
#   4. SEMANTIC: asserts every literal `.claude/workflows/<name>.js`
#      reference inside an emitted skill resolves to a real file in the SAME
#      emitted tree — a shim can never ship pointing at an absent workflow.
#      This is checked with an explicit occurrence floor (so the check
#      cannot pass vacuously on zero matches) and per-file exact-reference
#      assertions for the 2 skills known to carry a reference
#      (rdm-dispatch-phase and rdm-do -> dispatch-phase.js). This section
#      also asserts, name-generically and across EVERY emitted skill (not
#      just rdm-autopilot), that no skill's prose instructs invoking a
#      Workflow whose name does not resolve to a file in the same emitted
#      `.claude/workflows/` tree. rdm-autopilot is the prose `rdm-autopilot`
#      skill (workflow-orchestration roadmap, phase 3) and composes only
#      `dispatch-phase` downstream; the `estimate` pre-pass it also runs
#      locally is intentionally dropped from the distributed template (see
#      docs/workflow-vs-prose-boundary.md), so no emitted skill's prose may
#      instruct invoking a Workflow named `estimate` either — the same
#      hazard that got `autopilot.js` itself retired from this surface.
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
#      `.claude/workflows` (no Workflow-tool runtime); `--user` emission
#      never writes `.claude/workflows` either (the scripts hardcode a
#      target repo's own binary path, so they're `--out`-only); and emission
#      succeeds even when `RDM_ROOT` points at a path that does not exist,
#      since `--skills` emission never needs the plan repo.
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
WORKFLOWS="dispatch-phase.js review-refute-fix.js"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- helpers -------------------------------------------------------------

# Asserts <emitted_dir>/.claude/workflows/{dispatch-phase,review-refute-fix}.js
# are byte-identical to this repo's own .claude/workflows/*.js. Prints a
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
# "never invokes an `estimate` Workflow" (rdm-autopilot's own "why no
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

    grep -qF '.claude/workflows/dispatch-phase.js' "$skills_dir/rdm-dispatch-phase/SKILL.md" ||
        fail "$variant: rdm-dispatch-phase/SKILL.md must reference .claude/workflows/dispatch-phase.js"
    grep -qF '.claude/workflows/dispatch-phase.js' "$skills_dir/rdm-do/SKILL.md" ||
        fail "$variant: rdm-do/SKILL.md must reference .claude/workflows/dispatch-phase.js"
    pass "$variant: rdm-dispatch-phase/rdm-do carry their expected exact references"

    # Every emitted skill's prose (not just rdm-autopilot's) must never
    # instruct invoking a Workflow whose name does not resolve to a file in
    # this same emitted tree -- that call would target a file this
    # generator does not emit and would fail at the exact point the skill's
    # contract depends on. rdm-autopilot composes only `dispatch-phase`
    # downstream; its `estimate` pre-pass is intentionally dropped from the
    # distributed template (see docs/workflow-vs-prose-boundary.md), so it
    # must never instruct invoking `estimate` either -- the same hazard that
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
    >>"$SCRATCH_WF/.claude/workflows/dispatch-phase.js"
if check_workflows_byte_identical "$SCRATCH_WF" >/dev/null 2>&1; then
    fail "self-test A: planted corruption in dispatch-phase.js was NOT detected — byte-identical gate is vacuous"
fi
pass "self-test A: planted corruption in dispatch-phase.js correctly turned the byte-identical gate red"

say "5b. Self-test: planted shim reference typo'd to a nonexistent filename"
SCRATCH_SHIM="$TMP/scratch-corrupt-shim"
rm -rf "$SCRATCH_SHIM"
cp -R "$TMP/cli" "$SCRATCH_SHIM"
sed 's/\.claude\/workflows\/dispatch-phase\.js/.claude\/workflows\/dispatch-phase-typo.js/' \
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

# --- 6. negative checks: platform/scope boundaries + plan-repo independence -
say "6a. Negative: Pi emission never writes .claude/workflows"
"$RDM_BIN" agent-config pi --skills --project distro-check --out "$TMP/pi" >/dev/null
if [ -d "$TMP/pi/.claude/workflows" ]; then
    fail "Pi emission must not write .claude/workflows (Pi has no Workflow-tool runtime)"
fi
pass "Pi emission has no .claude/workflows directory"

say "6b. Negative: --user emission never writes .claude/workflows"
mkdir -p "$TMP/user-home"
if HOME="$TMP/user-home" "$RDM_BIN" agent-config claude --skills --user >"$TMP/user-emit.log" 2>&1; then
    if [ -d "$TMP/user-home/.claude/workflows" ]; then
        fail "--user emission must not write .claude/workflows (scripts are --out-only, not user-global)"
    fi
    pass "--user emission has no .claude/workflows directory"
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
    # any longer on either variant — the distributed template's `estimate`
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
    # shims on both variants; recomputed after the `estimate` pre-pass —
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
