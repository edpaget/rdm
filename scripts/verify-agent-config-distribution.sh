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
#      with minimally-valid frontmatter, both workflow scripts land under
#      `.claude/workflows/`, and the `rdm-mechanical` agent definition lands
#      under `.claude/agents/` with minimally-valid frontmatter.
#   3. BYTE-IDENTITY: asserts the 2 emitted workflow scripts AND the 1 emitted
#      agent definition are byte-for-byte identical to this repo's own
#      `.claude/workflows/*.js` and `.claude/agents/*.md` — the surfaces
#      `generate_workflows`/`generate_agents` promise to emit verbatim (see
#      their doc comments in `rdm-core/src/agent_config.rs`).
#   3c. AGENT REFERENCE RESOLUTION: sweeps every emitted workflow script for an
#      `agentType: '<name>'` literal and asserts each resolves to a `name:`
#      frontmatter line in the EMITTED `.claude/agents/*.md` set (modeled on
#      `scripts/verify-workflow-review.sh` §2c(iv), but resolved against the
#      DOWNSTREAM tree, not this repo's own copy). The emitted reference set
#      is empty at landing time — no distributed template threads `agentType`
#      yet — so non-vacuity comes from an emitted-agent-definition-count floor
#      (>= 1) plus three planted-corruption self-tests, not an occurrence
#      floor over real references (see the section itself for why).
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
#      `.claude/workflows` or `.claude/agents` (no Workflow-tool runtime) and
#      never prints a cleanup-report line; `--user` emission never writes
#      `.claude/workflows` or `.claude/agents` either (the scripts are
#      project-scoped to a specific checked-out repo, so they're `--out`-only)
#      and never prints a cleanup-report line; and emission succeeds even when
#      `RDM_ROOT` points at a path that does not exist, since `--skills`
#      emission never needs the plan repo.
#   7. DOWNSTREAM EXECUTION: byte-identity is NECESSARY but not SUFFICIENT —
#      it says nothing about whether the emitted lane WORKS anywhere else. So
#      sections 7a-7f stand up a hermetic non-rdm, NON-RUST fixture (a
#      Python/TypeScript source repo with a real feature branch, a docs-only
#      and a CHANGELOG-only negative control, its own `rdm init`-seeded plan
#      repo under project `acme-web`, and its own rdm executable path), emit
#      the lane into it, EXTRACT importable modules from the EMITTED scripts
#      (copy + neutralized top-level `return` + an explicit appended export
#      block, imported as `.mjs`; an inverse transform proves the copy is
#      byte-identical to the emitted file, and the untransformed file provably
#      does NOT import), then EXECUTE that pure logic: `deriveSignals` fires
#      every signal on the fixture's OWN diff, `selectDimensions` returns all
#      seven code dimensions (against a two-dimension docs-only control), every
#      built rdm command names the fixture's binary and honors the
#      project-agnostic allow-list with zero `./target/debug/rdm` or
#      `--project rdm`, and three of those built commands are really RUN
#      against the fixture plan repo (exit 0 + the expected JSON shape + an
#      `in-progress` read-back, with a dropped-`--roadmap` negative control).
#      Four planted corruptions in the EMITTED bytes prove none of it is
#      vacuous, and a composed-pattern self-gate forbids the harness from ever
#      importing one of this repo's non-emitted canonical source modules.
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
# Requires: a cargo-built rdm at target/debug/rdm (from this repo), and `node`
# (on PATH or via `mise exec node --`) for the section 7 downstream driver.

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
# The single shipped agent definition (`generate_agents()`'s sole entry).
AGENTS="rdm-mechanical.md"

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

# Asserts every $AGENTS definition under <emitted_dir>/.claude/agents is
# byte-identical to this repo's own .claude/agents/*.md. Prints a diagnostic
# per drifted file and returns nonzero if any differ.
check_agents_byte_identical() {
    dir=$1
    drifted=0
    for name in $AGENTS; do
        if ! diff -q "$REPO_ROOT/.claude/agents/$name" "$dir/.claude/agents/$name" >/dev/null 2>&1; then
            echo "  drift: $dir/.claude/agents/$name differs from $REPO_ROOT/.claude/agents/$name" >&2
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

# --- 2. structural: skills + workflows + agents land at conventional paths -
say "2. Structural: all 11 skills + 2 workflow scripts + 1 agent definition present with valid frontmatter"
for variant in cli mcp; do
    for skill in $SKILLS; do
        md="$TMP/$variant/.claude/skills/$skill/SKILL.md"
        [ -f "$md" ] || fail "$variant: missing $md"
        assert_valid_frontmatter "$md"
    done
    for wf in $WORKFLOWS; do
        [ -f "$TMP/$variant/.claude/workflows/$wf" ] || fail "$variant: missing .claude/workflows/$wf"
    done
    for agent in $AGENTS; do
        agent_md="$TMP/$variant/.claude/agents/$agent"
        [ -f "$agent_md" ] || fail "$variant: missing .claude/agents/$agent"
        assert_valid_frontmatter "$agent_md"
    done
    pass "$variant: 11 skills (valid frontmatter) + 2 workflow scripts + 1 agent definition (valid frontmatter) present"
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

# --- 3. byte-identity: emitted workflows + agents match this repo's own copies
say "3. Byte-identity: emitted workflow scripts vs $REPO_ROOT/.claude/workflows"
for variant in cli mcp; do
    if check_workflows_byte_identical "$TMP/$variant"; then
        pass "$variant: workflow scripts byte-identical to source"
    else
        fail "$variant: workflow scripts drifted from $REPO_ROOT/.claude/workflows (see drift lines above)"
    fi
done

say "3a. Byte-identity: emitted agent definitions vs $REPO_ROOT/.claude/agents"
for variant in cli mcp; do
    if check_agents_byte_identical "$TMP/$variant"; then
        pass "$variant: agent definitions byte-identical to source"
    else
        fail "$variant: agent definitions drifted from $REPO_ROOT/.claude/agents (see drift lines above)"
    fi
done

# --- 3c. AGENT REFERENCE RESOLUTION -----------------------------------------
# The successor to the removed `scripts/verify-workflow-review.sh` §2b
# distributed-agentType guard: that guard existed only because there was no
# `.claude/agents/` emission surface, so a distributed `agentType` reference
# would raise on first dispatch in every downstream repo with nothing to
# resolve against. Now that `generate_agents()` ships `.claude/agents/`
# alongside `.claude/workflows/`, THIS check catches the same failure the
# moment a real reference exists: it sweeps every emitted workflow script for
# an `agentType: '<name>'` literal and asserts each resolves to a `name:`
# frontmatter line in the EMITTED `.claude/agents/*.md` set (not this repo's
# own copy — a downstream consumer only has what was actually emitted into
# its tree). Modeled on `scripts/verify-workflow-review.sh` §2c(iv)'s
# resolution shape.
#
# The emitted reference set is empty today: none of the two distributed
# workflow scripts threads `agentType` yet (that is a deliberate follow-up,
# not this phase's scope — see `docs/mechanical-agent-inventory.md`). An
# occurrence floor over real references (mirroring the shim-reference `>= 4`
# floor a few sections up) would therefore fail on a correct implementation,
# so non-vacuity instead comes from (a) an emitted-agent-definition-count
# floor (>= 1: proves a real, non-empty definition set exists to resolve
# against even with zero references today) and (b) three planted-corruption
# self-tests below, which are the primary evidence this check has teeth.
say "3c. Agent reference resolution: every emitted agentType literal resolves to an emitted agent definition"
resolve_agent_refs() {
    # $1 = emitted tree root (contains .claude/workflows and .claude/agents).
    tree=$1
    wf_dir="$tree/.claude/workflows"
    agents_dir="$tree/.claude/agents"
    refs_scratch="$TMP/.agent-refs-scratch.txt"
    AGENT_REF_COUNT=0
    AGENT_REF_UNRESOLVED=0
    grep -rhoE "agentType: *'[^']*'" "$wf_dir"/*.js 2>/dev/null |
        sed "s/.*'\(.*\)'/\1/" | sort -u >"$refs_scratch" || : >"$refs_scratch"
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        AGENT_REF_COUNT=$((AGENT_REF_COUNT + 1))
        found=""
        for def in "$agents_dir"/*.md; do
            [ -f "$def" ] || continue
            defname=$(sed -n '/^name:[[:space:]]*/{s/^name:[[:space:]]*//p;q;}' "$def")
            [ "$defname" = "$name" ] && found="$def" && break
        done
        [ -n "$found" ] ||
            {
                echo "  unresolved: $wf_dir references agentType '$name' but no file in $agents_dir declares 'name: $name'" >&2
                AGENT_REF_UNRESOLVED=$((AGENT_REF_UNRESOLVED + 1))
            }
    done <"$refs_scratch"
    rm -f "$refs_scratch"
    [ "$AGENT_REF_UNRESOLVED" -eq 0 ]
}
for variant in cli mcp; do
    if resolve_agent_refs "$TMP/$variant"; then
        pass "$variant: all $AGENT_REF_COUNT emitted agentType reference(s) resolve (0 expected today)"
    else
        fail "$variant: $AGENT_REF_UNRESOLVED unresolved agentType reference(s) (see lines above)"
    fi
    AGENTS_COUNT=$(find "$TMP/$variant/.claude/agents" -name '*.md' -type f | wc -l | tr -d ' ')
    [ "$AGENTS_COUNT" -ge 1 ] ||
        fail "$variant: expected >= 1 emitted agent definition, found $AGENTS_COUNT — the resolution check has nothing real to resolve against"
    pass "$variant: $AGENTS_COUNT emitted agent definition(s) present (non-vacuity floor)"
done

# --- 3c self-tests: prove the resolution check actually has teeth ----------
# Each scratch copy is made from $TMP/cli AFTER the main 3c assertions above
# have already run and passed against the real (uncorrupted) emission, so a
# planted corruption here can never contaminate this run's primary pass/fail
# signal (same discipline as every other planted-mutation self-test in this
# script, e.g. 5j's).
say "3c-i. Self-test: an unresolvable planted agentType reference must turn the check red"
SCRATCH_AGENT_UNRESOLVED="$TMP/scratch-agent-unresolved"
rm -rf "$SCRATCH_AGENT_UNRESOLVED"
cp -R "$TMP/cli" "$SCRATCH_AGENT_UNRESOLVED"
# shellcheck disable=SC2016
printf "\nawait agent(P, { agentType: 'does-not-exist' })\n" \
    >>"$SCRATCH_AGENT_UNRESOLVED/.claude/workflows/$DISPATCH_WF"
if resolve_agent_refs "$SCRATCH_AGENT_UNRESOLVED" >/dev/null 2>&1; then
    fail "self-test 3c-i: a planted unresolvable agentType 'does-not-exist' was NOT detected — the resolution check is vacuous"
fi
pass "self-test 3c-i: a planted unresolvable agentType reference correctly turned the resolution check red"

say "3c-ii. Self-test: a resolvable planted agentType reference must stay green"
SCRATCH_AGENT_RESOLVED="$TMP/scratch-agent-resolved"
rm -rf "$SCRATCH_AGENT_RESOLVED"
cp -R "$TMP/cli" "$SCRATCH_AGENT_RESOLVED"
# shellcheck disable=SC2016
printf "\nawait agent(P, { agentType: 'rdm-mechanical' })\n" \
    >>"$SCRATCH_AGENT_RESOLVED/.claude/workflows/$DISPATCH_WF"
if ! resolve_agent_refs "$SCRATCH_AGENT_RESOLVED" >/dev/null 2>&1; then
    fail "self-test 3c-ii: a planted RESOLVABLE agentType 'rdm-mechanical' incorrectly turned the resolution check red — it is failing on any injected literal, not on unresolvability"
fi
[ "$AGENT_REF_COUNT" -ge 1 ] ||
    fail "self-test 3c-ii: the planted reference was not counted — the check did not actually sweep the corrupted file"
pass "self-test 3c-ii: a planted resolvable agentType reference correctly stayed green ($AGENT_REF_COUNT reference(s) swept)"

say "3c-iii. Self-test: deleting the emitted agent definition must turn a resolvable reference red"
# Starting from 3c-ii's copy (which already carries a resolvable
# 'rdm-mechanical' reference), remove the emitted definition it resolves
# against.
rm -f "$SCRATCH_AGENT_RESOLVED/.claude/agents/rdm-mechanical.md"
if resolve_agent_refs "$SCRATCH_AGENT_RESOLVED" >/dev/null 2>&1; then
    fail "self-test 3c-iii: deleting the emitted rdm-mechanical.md did NOT turn a previously-resolvable reference red — the check is resolving against something other than the emitted tree"
fi
pass "self-test 3c-iii: deleting the emitted agent definition correctly turned a previously-resolvable reference red"

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

# --- 5i. REMOVED -----------------------------------------------------------
# 5i used to assert CHANGELOG.md's [Unreleased] section named all six engines,
# the word BREAKING, and a cleanup sentence — with a planted-mutation
# self-test over the changelog body. It was a release time-bomb, not a code
# gate: prepare-release.yml moves the whole [Unreleased] body into a versioned
# section, so the check went red on main the moment v0.18.1 landed. CLAUDE.md
# now categorically forbids asserting on CHANGELOG.md content anywhere. The
# rename itself is gated by sections 3 (byte-identity), 5g/5h (no stale engine
# references anywhere in-tree) and 5j (end-to-end superseded cleanup), none of
# which read prose. The section number is kept as a gap so 5j's numbering and
# the header's section list stay stable.

# --- 5j. END-TO-END: a downstream re-emit removes the superseded files -----
say "5j. Superseded cleanup end-to-end: a stale downstream tree is cleaned, a custom file is not"
STALE="$TMP/stale"
STALE_WF="$STALE/.claude/workflows"
mkdir -p "$STALE_WF"
# Seed the tree with real PRE-REMOVAL bodies a past release emitted, taken
# verbatim from history rather than reconstructed: the cleanup is
# fingerprint-gated, so only a body this repo genuinely once shipped hashes to
# an entry in SUPERSEDED_WORKFLOWS.
#
# Recover each one from the parent of the last commit that touched its old
# path — NOT from `HEAD:<old path>`. `HEAD` is a moving ref: the moment the
# rename lands, the old paths no longer exist there and a `HEAD:`-anchored
# read hard-fails on every future checkout. Walking to the last commit that
# touched the path works whether or not the rename is committed yet, since
# every historical body is fingerprinted.
recover_pre_removal_body() {
    # $1 = repo-relative path as it was BEFORE removal; $2 = destination file.
    _path=$1
    _dest=$2
    _last=$(git -C "$REPO_ROOT" rev-list -n1 HEAD -- "$_path")
    [ -n "$_last" ] ||
        fail "5j: could not locate any commit in HEAD's history that touched $_path (a shallow clone cannot run this section)"
    git -C "$REPO_ROOT" show "$_last^:$_path" >"$_dest" 2>/dev/null ||
        fail "5j: could not recover a pre-removal body for $_path from $_last^"
    [ -s "$_dest" ] || fail "5j: recovered an empty body for $_path"
}
for stale_pair in "dispatch-phase.js" "review-refute-fix.js"; do
    recover_pre_removal_body "rdm-core/src/templates/workflows/$stale_pair" "$STALE_WF/$stale_pair"
done
# The retired orphan: no successor, recovered the same way.
recover_pre_removal_body rdm-core/src/templates/workflows/autopilot.js "$STALE_WF/autopilot.js"
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
say "6a. Negative: Pi emission never writes .claude/workflows, .claude/agents, or prints a cleanup report"
"$RDM_BIN" agent-config pi --skills --project distro-check --out "$TMP/pi" >"$TMP/pi-emit.log"
if [ -d "$TMP/pi/.claude/workflows" ]; then
    fail "Pi emission must not write .claude/workflows (Pi has no Workflow-tool runtime)"
fi
if [ -d "$TMP/pi/.claude/agents" ]; then
    fail "Pi emission must not write .claude/agents (agent definitions are Workflow-tool-only, same gate as .claude/workflows)"
fi
if grep -qE '^(Removed |Skipped |Failed to remove )' "$TMP/pi-emit.log"; then
    fail "Pi emission printed a cleanup-report line — cleanup must never run outside Platform::Claude && !user:\n$(cat "$TMP/pi-emit.log")"
fi
pass "Pi emission has no .claude/workflows or .claude/agents directory and prints no cleanup-report line"

say "6b. Negative: --user emission never writes .claude/workflows, .claude/agents, or prints a cleanup report"
mkdir -p "$TMP/user-home"
if HOME="$TMP/user-home" "$RDM_BIN" agent-config claude --skills --user >"$TMP/user-emit.log" 2>&1; then
    if [ -d "$TMP/user-home/.claude/workflows" ]; then
        fail "--user emission must not write .claude/workflows (scripts are --out-only, not user-global)"
    fi
    if [ -d "$TMP/user-home/.claude/agents" ]; then
        fail "--user emission must not write .claude/agents (agent definitions are --out-only, same gate as .claude/workflows)"
    fi
    if grep -qE '^(Removed |Skipped |Failed to remove )' "$TMP/user-emit.log"; then
        fail "--user emission printed a cleanup-report line — cleanup must never run against --user:\n$(cat "$TMP/user-emit.log")"
    fi
    pass "--user emission has no .claude/workflows or .claude/agents directory and prints no cleanup-report line"
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
    # dispatch-phase's `rdmBin` arg now DEFAULTS to a plain `rdm` on PATH, so an
    # emitted shim that omits it degrades to whatever rdm the downstream consumer
    # has on PATH rather than hard-breaking. The check is kept for exactly that
    # reason: a shim that names the arg lets a consumer pin a specific build.
    # Asserted on BOTH variants and for ALL THREE shims — it is not a
    # model-derived hoist, so it sits outside the cli-only guards.
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
    # roadmap, which added one `rdmBin` needle to each of the three
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

# Self-test: mangling the rdmBin key in each of the three emitted shims
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

# --- 7. DOWNSTREAM EXECUTION: the emitted lane, exercised in a foreign repo -
#
# Sections 2/3/3b/4/5/6 above prove the emitted bytes are RIGHT. They do not
# prove the emitted lane WORKS somewhere that is neither this repo nor Rust —
# byte-identity is necessary, not sufficient. Sections 7a-7f close that: they
# stand up a hermetic non-rdm, non-Rust consumer repo, emit into it, then
# EXECUTE the emitted engines' pure pipeline logic and one of the rdm commands
# they build against that fixture's own binary and project.
#
# NOT a duplicate of scripts/verify-workflow-dispatch.sh § 9 or
# scripts/verify-workflow-review-outcome.sh § 6: those gate this repo's LOCAL
# .claude/workflows/ copies. These sections assert the same properties on the
# DOWNSTREAM EMITTED artifact, which is the only surface a consumer ever sees.
# Do not delete either as redundant.

say "7a. Downstream fixture: a hermetic non-rdm, non-Rust consumer repo"

FIXTURE="$TMP/fixture"
FIXTURE_REPO="$FIXTURE/repo"
FIXTURE_PLAN="$FIXTURE/plan"
FIXTURE_SCRATCH="$FIXTURE/scratch"
FIXTURE_HOME="$FIXTURE/home"
FIXTURE_TOOLS="$FIXTURE/tools"
FIXTURE_PROJECT="acme-web"
FIXTURE_ROADMAP="checkout-revamp"
FIXTURE_PHASE="phase-1-checkout-form"
FIXTURE_TASK="tidy-cli"
mkdir -p "$FIXTURE_REPO" "$FIXTURE_PLAN" "$FIXTURE_SCRATCH" "$FIXTURE_HOME" "$FIXTURE_TOOLS"

# Hermetic git: the developer's global/system config (init.defaultBranch,
# core.hooksPath -> .githooks, commit signing, commit templates) must not leak
# into the fixture, or `git diff main...HEAD` silently changes meaning.
fixture_git() {
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
        git -C "$FIXTURE_REPO" \
        -c user.name=Fixture -c user.email=fixture@example.invalid \
        -c commit.gpgsign=false "$@"
}

if ! fixture_git init -b main -q >/dev/null 2>&1; then
    fixture_git init -q
    fixture_git checkout -q -b main
fi

mkdir -p "$FIXTURE_REPO/src/acme" "$FIXTURE_REPO/web/src" "$FIXTURE_REPO/bin" \
    "$FIXTURE_REPO/tests" "$FIXTURE_REPO/docs"

cat >"$FIXTURE_REPO/src/acme/api.py" <<'PY_API'
"""Order API."""


def _normalize(order):
    return {"id": order["id"], "total": order["total"]}
PY_API

cat >"$FIXTURE_REPO/src/acme/runner.py" <<'PY_RUNNER'
"""Job runner."""


def run_job(name):
    return name
PY_RUNNER

cat >"$FIXTURE_REPO/web/src/client.ts" <<'TS_CLIENT'
const BASE = "/api";

function join(a: string, b: string): string {
  return a + b;
}
TS_CLIENT

cat >"$FIXTURE_REPO/bin/acme_cli.py" <<'PY_CLI'
"""acme command line."""
import argparse


def build_parser():
    parser = argparse.ArgumentParser(prog="acme")
    return parser
PY_CLI

cat >"$FIXTURE_REPO/tests/test_api.py" <<'PY_TEST'
from src.acme.api import _normalize


def test_normalize():
    assert _normalize({"id": 1, "total": 2})["id"] == 1
PY_TEST

printf 'Usage\n=====\n\nRun acme.\n' >"$FIXTURE_REPO/docs/usage.md"
printf '# Changelog\n\n## [Unreleased]\n' >"$FIXTURE_REPO/CHANGELOG.md"
printf '{\n  "name": "acme-web",\n  "version": "0.1.0"\n}\n' >"$FIXTURE_REPO/package.json"

fixture_git add -A
fixture_git commit -qm "seed: acme base"

# The POSITIVE branch: one commit, engineered so every CONDITIONAL code
# dimension fires — and written in Python/TypeScript ONLY, so what is exercised
# is the language-neutral content vocabulary phases 1-3 introduced. A
# conditional dimension failing to fire here is a regression in those phases,
# not a fixture bug.
fixture_git checkout -q -b feature/checkout
cat >>"$FIXTURE_REPO/web/src/client.ts" <<'TS_ADD'

export function listOrders(limit: number): Promise<string[]> {
  return fetch(BASE + "/orders?limit=" + limit).then((r) => r.json());
}
TS_ADD
cat >>"$FIXTURE_REPO/bin/acme_cli.py" <<'PY_CLI_ADD'


def add_limit(parser):
    parser.add_argument("--limit", type=int, default=10)
    print("done")
    return parser
PY_CLI_ADD
cat >>"$FIXTURE_REPO/src/acme/runner.py" <<'PY_RUNNER_ADD'


def run_remote(cmd):
    import os
    import subprocess

    token = os.environ["ACCESS_TOKEN"]
    subprocess.run([cmd, token], check=True)
PY_RUNNER_ADD
printf -- '- Added a --limit flag to the acme CLI.\n' >>"$FIXTURE_REPO/CHANGELOG.md"
fixture_git add -A
fixture_git commit -qm "feat: add order listing and a remote runner"

# Negative control: a docs-only branch must select the always-on pair and
# nothing else, so the positive result above is discriminating.
fixture_git checkout -q main
fixture_git checkout -q -b feature/docs-only
printf '\nMore usage notes.\n' >>"$FIXTURE_REPO/docs/usage.md"
fixture_git add -A
fixture_git commit -qm "docs: expand usage"

# Second negative control: `changelogTouched` only ever CONFIRMS `userFacing`;
# a CHANGELOG-only diff has no code files and must stay false.
fixture_git checkout -q main
fixture_git checkout -q -b feature/changelog-only
printf -- '- Noted an unrelated change.\n' >>"$FIXTURE_REPO/CHANGELOG.md"
fixture_git add -A
fixture_git commit -qm "docs: changelog note"
fixture_git checkout -q feature/checkout

[ ! -f "$FIXTURE_REPO/Cargo.toml" ] ||
    fail "7a: the fixture must not be a Rust/cargo repo — it exists to exercise the lane somewhere unlike this repo"
FIXTURE_RS=$(find "$FIXTURE_REPO" -name '*.rs' -not -path '*/.git/*' | wc -l | tr -d ' ')
[ "$FIXTURE_RS" -eq 0 ] ||
    fail "7a: the fixture source tree contains $FIXTURE_RS Rust file(s) — it must be non-Rust"
[ "$(fixture_git rev-parse --abbrev-ref HEAD)" = "feature/checkout" ] ||
    fail "7a: the fixture repo is not on the expected feature/checkout branch"
pass "7a: fixture source repo seeded (Python/TypeScript, no Cargo.toml, no *.rs) on feature/checkout"

# The fixture's OWN plan repo, under its OWN project name — neither `rdm` nor
# `distro-check`, and matching parseProjectArg's /^[A-Za-z0-9._-]+$/.
fixture_rdm() {
    HOME="$FIXTURE_HOME" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
        "$RDM_BIN" --root "$FIXTURE_PLAN" "$@"
}
FIXTURE_SEED_LOG="$TMP/fixture-seed.log"
{
    fixture_rdm init --default-project "$FIXTURE_PROJECT"
    fixture_rdm roadmap create "$FIXTURE_ROADMAP" --title "Checkout revamp" \
        --body "Revamp the acme checkout flow." --no-edit --project "$FIXTURE_PROJECT"
    fixture_rdm phase create checkout-form --title "Checkout form" --number 1 \
        --body "Build the acme checkout form." --no-edit \
        --roadmap "$FIXTURE_ROADMAP" --project "$FIXTURE_PROJECT"
    fixture_rdm phase create order-summary --title "Order summary" --number 2 \
        --body "Summarize orders on the confirmation screen." --no-edit \
        --roadmap "$FIXTURE_ROADMAP" --project "$FIXTURE_PROJECT"
    fixture_rdm task create "$FIXTURE_TASK" --title "Tidy the acme CLI" \
        --body "Tidy up the acme command line." --no-edit --project "$FIXTURE_PROJECT"
    fixture_rdm commit -m "seed: acme-web fixture"
} >>"$FIXTURE_SEED_LOG" 2>&1
# The stem the emitted engines will be asked to address must really exist, or
# section 7d's "exit 0" would be measuring the wrong thing.
fixture_rdm phase show "$FIXTURE_PHASE" --roadmap "$FIXTURE_ROADMAP" \
    --project "$FIXTURE_PROJECT" --no-body >>"$FIXTURE_SEED_LOG" 2>&1 ||
    fail "7a: the fixture plan repo does not carry $FIXTURE_PHASE — the seed failed"
pass "7a: fixture plan repo seeded under project '$FIXTURE_PROJECT' (roadmap + 2 phases + 1 task)"

# The fixture's OWN rdm executable path. Everything the emitted engines build
# must name THIS, never this repo's dev build.
FIXTURE_BIN="$FIXTURE_TOOLS/acme-rdm"
ln -s "$RDM_BIN" "$FIXTURE_BIN" 2>/dev/null || cp "$RDM_BIN" "$FIXTURE_BIN"
[ -x "$FIXTURE_BIN" ] || fail "7a: could not stand up the fixture rdm executable at $FIXTURE_BIN"
case "$FIXTURE_BIN" in
    *target/debug*) fail "7a: the fixture binary path must not contain this repo's build directory" ;;
esac
pass "7a: fixture rdm executable at $FIXTURE_BIN"

# node is a NEW dependency of this harness. Resolve it the same way the sibling
# workflow harnesses do, and FAIL rather than skip — a silent skip would make
# every gate below vacuous on a machine without node.
NODE_VIA_MISE=0
if command -v node >/dev/null 2>&1; then
    NODE_VIA_MISE=0
elif command -v mise >/dev/null 2>&1 && mise exec node -- node --version >/dev/null 2>&1; then
    NODE_VIA_MISE=1
else
    fail "node not found on PATH or via 'mise exec node --'. node is pinned in .mise.toml; run 'mise install'."
fi
run_node() {
    if [ "$NODE_VIA_MISE" -eq 1 ]; then
        mise exec node -- node "$@"
    else
        node "$@"
    fi
}
pass "7a: node resolved for the downstream-execution driver"

# --- 7b. Emit into the fixture, then EXTRACT importable modules from the ---
# --- EMITTED scripts --------------------------------------------------------
say "7b. Emitting the lane into the fixture and extracting importable modules from the EMITTED scripts"

"$RDM_BIN" agent-config claude --skills --project "$FIXTURE_PROJECT" --out "$FIXTURE_REPO" >/dev/null

# The fixture emission is a THIRD emission and satisfies the same byte-identity
# contract — runtime-args parameterization changes no bytes (AC7).
if check_workflows_byte_identical "$FIXTURE_REPO"; then
    pass "7b: fixture-emitted workflow scripts are byte-identical to source"
else
    fail "7b: fixture-emitted workflow scripts drifted from $REPO_ROOT/.claude/workflows"
fi

for engine in $WORKFLOWS; do
    [ -f "$FIXTURE_REPO/.claude/workflows/$engine" ] ||
        fail "7b: the fixture emission is missing .claude/workflows/$engine"
    # Derive the PRE-RENAME bare name from the variable rather than writing a
    # second literal (section 0 permits exactly one occurrence of each).
    bare_engine=${engine#rdm-wf-}
    [ ! -e "$FIXTURE_REPO/.claude/workflows/$bare_engine" ] ||
        fail "7b: the fixture emission still carries a pre-rename engine file named $bare_engine"
done
pass "7b: both rdm-wf-* engines emitted; no pre-rename bare filename present"

# The fixture's skill shims must resolve to the rdm-wf-* engines in this same
# emitted tree — a shim left pointing at a pre-rename engine name fails here.
if check_shim_refs_resolve "$FIXTURE_REPO/.claude/skills" "$FIXTURE_REPO/.claude/workflows"; then
    pass "7b: all $SHIM_REF_COUNT fixture shim reference(s) resolve to the emitted rdm-wf-* engines"
else
    fail "7b: $SHIM_REF_UNRESOLVED unresolved shim reference(s) in the fixture emission"
fi
[ "$SHIM_REF_COUNT" -ge 3 ] ||
    fail "7b: expected >= 3 shim references in the fixture emission, found $SHIM_REF_COUNT"
if check_workflow_invocations_resolve "$FIXTURE_REPO/.claude/skills" "$FIXTURE_REPO/.claude/workflows"; then
    pass "7b: all $INVOCATION_COUNT fixture Workflow-invocation instruction(s) resolve"
else
    fail "7b: $INVOCATION_UNRESOLVED unresolved Workflow-invocation instruction(s) in the fixture emission"
fi
[ "$INVOCATION_COUNT" -ge 5 ] ||
    fail "7b: expected >= 5 Workflow-invocation instructions in the fixture emission, found $INVOCATION_COUNT"

# The downstream-execution driver. ONE Node program, three stages, so a
# planted-corruption self-test (7e) can re-run the exact same assertions
# against a corrupted copy of the fixture.
cat >"$TMP/downstream.mjs" <<'NODE_DOWNSTREAM'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const [stage, fixtureRoot, fixtureBin, project, dispatchEngine, reviewEngine] = process.argv.slice(2);

const SRC_REPO = path.join(fixtureRoot, 'repo');
const EMITTED_DIR = path.join(SRC_REPO, '.claude', 'workflows');
const SCRATCH = path.join(fixtureRoot, 'scratch');
const PLAN_DIR = path.join(fixtureRoot, 'plan');
const HOME_DIR = path.join(fixtureRoot, 'home');

// --- The extraction mechanism ------------------------------------------------
// An emitted `.claude/workflows/*.js` has `export const meta` as its ONLY
// export and ends in a top-level `return` (the Workflow runtime wraps the body
// in a function), so importing one as-is fails with
// `SyntaxError: Illegal return statement`, and its pure helpers are
// module-private. The transform below is the ONLY sanctioned way this harness
// reaches them; it must never fall back to this repo's canonical sources, which
// are not emitted at all (see the harness self-gate section).
const WRAP_HEAD = 'const __wf = async function (args, agent, pipeline, parallel, log) {';
const WRAP_TAIL = '};';
const HANDLE = 'const __H = await __wf();';
const SENTINEL = '// --- Driver';

function returnLine(names) {
  return 'return { ' + names.map((n) => n + ': ' + n).join(', ') + ' };';
}

function transform(src, helpers) {
  const names = helpers.concat(['meta']);
  const lines = src.split('\n');

  // (2) `export const meta` must be the ONLY column-0 export — an `export`
  // inside the wrapper function would be a SyntaxError.
  const exportIdx = [];
  lines.forEach((l, i) => {
    if (l.startsWith('export ')) exportIdx.push(i);
  });
  assert.equal(exportIdx.length, 1, 'expected exactly ONE column-0 `export` in the emitted script, found ' + exportIdx.length);
  assert.ok(lines[exportIdx[0]].startsWith('export const meta'), 'the single column-0 export must be `export const meta`');
  lines[exportIdx[0]] = lines[exportIdx[0]].slice('export '.length);
  const metaLines = lines.filter((l) => /^const meta\b/.test(l));
  assert.equal(metaLines.length, 1, 'expected exactly one column-0 `const meta` line after stripping the export keyword');

  // (3) the driver sentinel must occur EXACTLY once.
  const sentinelIdx = [];
  lines.forEach((l, i) => {
    if (l.startsWith(SENTINEL)) sentinelIdx.push(i);
  });
  assert.equal(
    sentinelIdx.length,
    1,
    'expected exactly ONE `' + SENTINEL + '` sentinel in the emitted script, found ' + sentinelIdx.length + ' — a rename or duplication must fail loudly here'
  );

  // (4) every named helper must be a HOISTED function declaration. A future
  // `const x = () => …` refactor would otherwise throw an opaque TDZ
  // ReferenceError from the injected return instead of failing here.
  for (const h of helpers) {
    assert.ok(
      new RegExp('^function ' + h + '\\(', 'm').test(src),
      'helper `' + h + '` is not declared as a column-0 `function ' + h + '(` in the emitted script — the injected return relies on function-declaration hoisting'
    );
  }

  // (4) inject the capture immediately BEFORE the sentinel, and (5) wrap.
  // The wrapper is the NEUTRALIZATION: every original top-level `return`
  // becomes a legal function return AND is made unreachable by the injected
  // one, and the ambient globals become never-bound parameters, so nothing of
  // the driver ever executes.
  lines.splice(sentinelIdx[0], 0, returnLine(names));
  return (
    WRAP_HEAD + '\n' + lines.join('\n') + '\n' + WRAP_TAIL + '\n' + HANDLE + '\n' +
    names.map((n) => 'export const ' + n + ' = __H.' + n + ';').join('\n') + '\n'
  );
}

// The INVERSE transform — provenance. Reproducing the emitted file byte-for-byte
// from the module that was actually imported is what proves the executed code IS
// the emitted code, not a look-alike.
function inverse(out, helpers) {
  const names = helpers.concat(['meta']);
  const lines = out.split('\n');
  assert.equal(lines.pop(), '', 'transformed module must end in a newline');
  for (let i = names.length - 1; i >= 0; i--) {
    assert.equal(lines.pop(), 'export const ' + names[i] + ' = __H.' + names[i] + ';');
  }
  assert.equal(lines.pop(), HANDLE);
  assert.equal(lines.pop(), WRAP_TAIL);
  assert.equal(lines.shift(), WRAP_HEAD);
  const ri = lines.indexOf(returnLine(names));
  assert.ok(ri !== -1, 'injected capture line not found during the inverse transform');
  lines.splice(ri, 1);
  const mi = lines.findIndex((l) => /^const meta\b/.test(l));
  assert.ok(mi !== -1, '`const meta` line not found during the inverse transform');
  lines[mi] = 'export ' + lines[mi];
  return lines.join('\n');
}

const IMPORTED = [];
async function importFromScratch(file) {
  const abs = path.resolve(file);
  assert.ok(
    abs.startsWith(SCRATCH + path.sep),
    'refusing to import ' + abs + ' — every module this harness imports must live under the fixture scratch dir ' + SCRATCH
  );
  IMPORTED.push(abs);
  return import(pathToFileURL(abs).href + '?t=' + process.pid);
}

async function extract(engine, helpers) {
  const emitted = path.resolve(EMITTED_DIR, engine);
  assert.ok(
    emitted.startsWith(path.resolve(SRC_REPO) + path.sep),
    'the module under test must be read from the FIXTURE emitted tree, got ' + emitted
  );
  const src = fs.readFileSync(emitted, 'utf8');
  const out = transform(src, helpers);
  assert.equal(inverse(out, helpers), src, 'the inverse transform did not reproduce ' + emitted + ' byte-for-byte');
  const dest = path.join(SCRATCH, engine + '.mjs');
  fs.mkdirSync(SCRATCH, { recursive: true });
  fs.writeFileSync(dest, out);
  const mod = await importFromScratch(dest);
  assert.equal(mod.meta.name, engine.replace(/\.js$/, ''), 'meta.name must equal the emitted filename stem');
  return { mod, emitted };
}

// Non-vacuity of the transform: the UNTRANSFORMED emitted file must NOT import.
async function assertRawImportRejects(engine) {
  const raw = path.join(SCRATCH, engine + '.raw.mjs');
  fs.mkdirSync(SCRATCH, { recursive: true });
  fs.copyFileSync(path.join(EMITTED_DIR, engine), raw);
  let err = null;
  try {
    await importFromScratch(raw);
  } catch (e) {
    err = e;
  }
  assert.ok(err, 'importing the UNTRANSFORMED emitted ' + engine + ' unexpectedly succeeded — the neutralization step is not load-bearing');
  assert.ok(err instanceof SyntaxError, 'expected a SyntaxError, got ' + err.constructor.name + ': ' + err.message);
  assert.match(err.message, /return/i, 'expected the SyntaxError to name the illegal top-level return, got: ' + err.message);
}

const DISPATCH_HELPERS = [
  'deriveSignals',
  'selectDimensions',
  'projectFlag',
  'resolveRdmBin',
  'parseProjectArg',
  'parseDispatchArgs',
  'buildFetchPrompt',
  'buildTaskFetchPrompt',
  'buildStampInProgressPrompt',
  'buildDiffSignalsPrompt',
  'buildImplementPrompt',
  'buildCodeActPrompt',
];
const REVIEW_HELPERS = [
  'deriveSignals',
  'selectDimensions',
  'findPrompt',
  'refutePrompt',
  'projectFlag',
  'resolveRdmBin',
  'parseProjectArg',
  'buildDiffSignalsPrompt',
];

// --- Shared fixture facts ----------------------------------------------------
const ROADMAP = 'checkout-revamp';
const PHASE = 'phase-1-checkout-form';
const TASK = 'tidy-cli';
const WORKTREE = 'roadmap-' + ROADMAP;
const CFG = { rdmBin: fixtureBin, project: project };

function fixtureGit(args) {
  return execFileSync('git', ['-C', SRC_REPO, ...args], {
    encoding: 'utf8',
    env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
  });
}

// Run EXACTLY the two commands buildDiffSignalsPrompt instructs, on a real
// branch of the fixture repo. The arrays are never hand-authored.
function realDiff(dispatch, branch) {
  const prompt = dispatch.buildDiffSignalsPrompt(WORKTREE, CFG);
  const gitCmds = prompt
    .split('\n')
    .filter((l) => /^ {2}git diff /.test(l))
    .map((l) => l.trim());
  assert.deepEqual(gitCmds, ['git diff --name-only main...HEAD', 'git diff main...HEAD'], 'the emitted diff prompt no longer instructs the two expected git commands');
  fixtureGit(['checkout', '-q', branch]);
  const changedFiles = fixtureGit(gitCmds[0].split(' ').slice(1)).split('\n').filter(Boolean);
  const diffText = fixtureGit(gitCmds[1].split(' ').slice(1));
  return { changedFiles, diffText };
}

function buildAllPrompts(dispatch, review, cfg) {
  return [
    dispatch.buildFetchPrompt(ROADMAP, PHASE, cfg),
    dispatch.buildTaskFetchPrompt(TASK, cfg),
    dispatch.buildStampInProgressPrompt(false, ROADMAP, PHASE, cfg),
    dispatch.buildStampInProgressPrompt(true, '', TASK, cfg),
    dispatch.buildDiffSignalsPrompt(WORKTREE, cfg),
    dispatch.buildImplementPrompt(WORKTREE, 'phase body', 'plan doc', null, cfg),
    dispatch.buildCodeActPrompt('phase', ROADMAP, PHASE, WORKTREE, [{ id: 'f1', severity: 'suggestion' }], cfg),
    review.buildDiffSignalsPrompt(WORKTREE, cfg),
    review.findPrompt('code', { key: 'ac', title: 'AC compliance', focus: 'f' }, { target: PHASE }),
    review.refutePrompt('code', { key: 'ac', title: 'AC compliance', focus: 'f' }, { id: 'f1', what_fails: 'x' }, { target: PHASE }),
  ];
}

// Tokenize every rdm invocation out of the built prompts. The binary token is
// whatever rdm-naming, non-space run precedes a known subcommand, so a
// re-hardcoded path is caught by COMPARISON rather than silently skipped.
const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?[A-Za-z0-9_.-]*rdm[A-Za-z0-9_.-]*)\s+([a-z][a-z-]*)(?:\s+([a-z][a-z-]*))?/g;
const PROJECT_AGNOSTIC = ['model resolve', 'commit', 'status', 'discard'];

function scan(prompts) {
  const out = [];
  for (const p of prompts) {
    for (const line of p.split('\n')) {
      INVOCATION.lastIndex = 0;
      let m;
      while ((m = INVOCATION.exec(line)) !== null) {
        out.push({ bin: m[2], two: m[4] ? m[3] + ' ' + m[4] : m[3], line });
      }
    }
  }
  return out;
}

// Pull a command line back OUT of a built prompt rather than retyping it, and
// assert it is character-identical to the substring of that prompt.
function extractCommand(prompt, needle) {
  const hit = prompt
    .split('\n')
    .filter((l) => l.startsWith('  ' + fixtureBin + ' ') && l.includes(needle))
    .map((l) => l.slice(2));
  assert.equal(hit.length, 1, 'expected exactly one built command containing "' + needle + '", found ' + hit.length);
  assert.ok(prompt.includes('  ' + hit[0]), 'the extracted command is not a verbatim substring of the built prompt');
  return hit[0];
}

function runCommand(cmd) {
  return spawnSync('/bin/sh', ['-c', cmd], {
    cwd: SRC_REPO,
    encoding: 'utf8',
    env: {
      ...process.env,
      RDM_ROOT: PLAN_DIR,
      HOME: HOME_DIR,
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_CONFIG_SYSTEM: '/dev/null',
    },
  });
}

// --- Stages ------------------------------------------------------------------
const dispatch = (await extract(dispatchEngine, DISPATCH_HELPERS)).mod;
const review = (await extract(reviewEngine, REVIEW_HELPERS)).mod;

if (stage === 'extract') {
  await assertRawImportRejects(dispatchEngine);
  await assertRawImportRejects(reviewEngine);
  assert.ok(IMPORTED.length >= 4, 'expected at least four module imports, all from the fixture scratch dir');
  console.log('downstream extract: both emitted engines transformed, provenance-checked and imported');
}

if (stage === 'logic') {
  // --- Signals derived from the fixture's OWN diffs -------------------------
  const positive = realDiff(dispatch, 'feature/checkout');
  fs.writeFileSync(path.join(fixtureRoot, 'signals-input.json'), JSON.stringify(positive, null, 2));
  assert.ok(positive.changedFiles.length >= 3, 'the positive fixture diff must touch several files');
  for (const rustToken of [/(^|[^A-Za-z])fn /, /(^|[^A-Za-z])pub /, /unsafe/, /\.rs\b/]) {
    assert.ok(!rustToken.test(positive.diffText), 'the fixture diff smuggled a Rust token (' + rustToken + ') — the language-neutrality claim would not be exercised');
  }

  const signals = dispatch.deriveSignals({ targetType: 'phase', changedFiles: positive.changedFiles, diffText: positive.diffText });
  for (const key of ['changesLogic', 'missingTests', 'multiModule', 'publicApiChanged', 'userFacing', 'securitySurface']) {
    assert.equal(typeof signals[key], 'boolean', 'signal ' + key + ' must be present and boolean');
    assert.equal(signals[key], true, 'signal ' + key + ' must fire on the fixture feature branch (a Python/TypeScript diff)');
  }
  const dims = dispatch.selectDimensions('code', signals).map((d) => d.key);
  assert.deepEqual(dims, ['ac', 'correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'], 'every CONDITIONAL code dimension must fire on the fixture diff, got: ' + dims.join(','));

  // Negative control: docs-only.
  const docs = realDiff(dispatch, 'feature/docs-only');
  const docsSignals = dispatch.deriveSignals({ targetType: 'phase', changedFiles: docs.changedFiles, diffText: docs.diffText });
  for (const key of ['changesLogic', 'publicApiChanged', 'userFacing', 'securitySurface']) {
    assert.equal(docsSignals[key], false, 'docs-only control: ' + key + ' must be false');
  }
  assert.deepEqual(dispatch.selectDimensions('code', docsSignals).map((d) => d.key), ['ac', 'correctness'], 'docs-only control must select only the always-on pair');

  // CHANGELOG-only control: changelogTouched CONFIRMS userFacing, never triggers it.
  const cl = realDiff(dispatch, 'feature/changelog-only');
  assert.deepEqual(cl.changedFiles, ['CHANGELOG.md'], 'the changelog-only control must touch exactly CHANGELOG.md');
  assert.equal(dispatch.deriveSignals({ targetType: 'phase', changedFiles: cl.changedFiles, diffText: cl.diffText }).userFacing, false, 'a CHANGELOG-only diff must not set userFacing');
  fixtureGit(['checkout', '-q', 'feature/checkout']);

  // Fail-open contract, on the EMITTED artifact.
  assert.equal(dispatch.selectDimensions('code', null).length, 7, 'omitted signals must fail open to every code dimension');
  assert.deepEqual(dispatch.selectDimensions('code', {}).map((d) => d.key), ['ac', 'correctness'], 'an explicit empty signals object means "computed, nothing triggered"');
  assert.deepEqual(review.selectDimensions('code', signals).map((d) => d.key), dims, 'both emitted engines must select the same dimensions for the same signals');

  // --- Every built command names the FIXTURE binary and project -------------
  const prompts = buildAllPrompts(dispatch, review, CFG);
  const occ = scan(prompts);
  assert.ok(occ.length >= 12, 'expected >= 12 rdm invocations across the built prompts, found ' + occ.length + ' — the tokenizer must not pass vacuously');
  const seen = new Set();
  for (const o of occ) {
    assert.equal(o.bin, fixtureBin, 'a built command used "' + o.bin + '" instead of the fixture binary: ' + o.line);
    seen.add(o.two);
    if (PROJECT_AGNOSTIC.includes(o.two)) {
      assert.ok(!o.line.includes('--project'), 'project-agnostic `' + o.two + '` must carry NO project flag: ' + o.line);
    } else {
      assert.ok(o.line.includes(' --project ' + project), 'project-scoped `' + o.two + '` must carry " --project ' + project + '": ' + o.line);
    }
  }
  for (const need of ['phase show', 'phase update', 'task show', 'task update', 'task create', 'worktree add', 'model resolve']) {
    assert.ok(seen.has(need), 'expected at least one built `rdm ' + need + '` command, saw: ' + [...seen].join(', '));
  }
  const joined = prompts.join('\n');
  for (const forbidden of ['target/debug/rdm', '--project rdm']) {
    assert.ok(!joined.includes(forbidden), 'a built prompt contains the rdm-specific literal "' + forbidden + '"');
  }
  assert.ok(!/(^|[\s`])rdm\s/.test(joined), 'a built prompt names a bare `rdm` binary token instead of the fixture binary');

  // Run B: no project configured -> not a single --project anywhere.
  const noProject = buildAllPrompts(dispatch, review, { rdmBin: fixtureBin });
  assert.ok(!noProject.join('\n').includes('--project'), 'with no project configured, no built command may carry a --project flag');

  // Environment-arg guards, on the EMITTED artifact. `rdmBin` DEFAULTS to a
  // plain `rdm` on PATH when absent — the shipped contract a plugin-installed
  // consumer relies on, since it has no repo-local build path to pass — while a
  // present-but-wrong-TYPE value still throws rather than silently degrading.
  assert.equal(dispatch.parseDispatchArgs({ roadmap: 'r', phase: 'p' }).rdmBin, 'rdm', 'the emitted engine must default an absent rdmBin to "rdm"');
  assert.throws(() => dispatch.parseDispatchArgs({ roadmap: 'r', phase: 'p', rdmBin: 42 }), /rdmBin/, 'the emitted engine must still reject a non-string rdmBin');
  for (const bad of ['a b', 'a;rm -rf /', '$(x)']) {
    assert.throws(() => dispatch.parseProjectArg(bad), /project/, 'parseProjectArg must reject "' + bad + '"');
    assert.throws(() => review.parseProjectArg(bad), /project/, 'the review engine must reject "' + bad + '" too');
  }
  assert.equal(dispatch.projectFlag({ project: project }), ' --project ' + project);
  assert.equal(dispatch.projectFlag({}), '');
  assert.equal(dispatch.resolveRdmBin(fixtureBin), fixtureBin);

  console.log('downstream logic: ' + occ.length + ' built rdm invocations, all naming ' + fixtureBin + '; conditional dimensions fired');
}

if (stage === 'exec') {
  assert.equal(process.env.RDM_ROOT, undefined, 'the harness must not carry an ambient RDM_ROOT — the real dogfood plan repo must be unreachable here');

  const fetchPrompt = dispatch.buildFetchPrompt(ROADMAP, PHASE, CFG);
  const showCmd = extractCommand(fetchPrompt, ' phase show ');
  const shown = runCommand(showCmd);
  assert.equal(shown.status, 0, 'the built `phase show` command failed (' + shown.status + '): ' + showCmd + '\n' + shown.stderr);
  const phaseJson = JSON.parse(shown.stdout);
  assert.equal(phaseJson.stem, PHASE, 'unexpected `stem` in the built command output shape');
  assert.ok(typeof phaseJson.body === 'string' && phaseJson.body.length > 0, 'the built command returned an empty body');
  assert.ok(typeof phaseJson.status === 'string' && phaseJson.status.length > 0, 'the built command output has no status field');

  const stampCmd = extractCommand(dispatch.buildStampInProgressPrompt(false, ROADMAP, PHASE, CFG), ' phase update ');
  const stamped = runCommand(stampCmd);
  assert.equal(stamped.status, 0, 'the built `phase update` command failed (' + stamped.status + '): ' + stampCmd + '\n' + stamped.stderr);
  const reread = JSON.parse(runCommand(showCmd).stdout);
  assert.equal(reread.status, 'in-progress', 'the built `phase update` did not persist — read back ' + reread.status);

  const taskShowCmd = extractCommand(dispatch.buildTaskFetchPrompt(TASK, CFG), ' task show ');
  const taskShown = runCommand(taskShowCmd);
  assert.equal(taskShown.status, 0, 'the built `task show` command failed (' + taskShown.status + '): ' + taskShowCmd + '\n' + taskShown.stderr);
  const taskJson = JSON.parse(taskShown.stdout);
  assert.equal(taskJson.slug, TASK, 'unexpected `slug` in the built task command output shape');
  assert.ok(typeof taskJson.body === 'string' && taskJson.body.length > 0, 'the built task command returned an empty body');

  // Negative control: "exit 0" only discriminates if a malformed command fails.
  const broken = stampCmd.replace(' --roadmap ' + ROADMAP, '');
  assert.notEqual(broken, stampCmd, 'the negative control did not actually mutate the command');
  assert.notEqual(runCommand(broken).status, 0, 'a `phase update` with --roadmap dropped still exited 0 — "exit 0" is not a discriminating assertion');

  console.log('downstream exec: 3 built commands executed against the fixture plan repo, all exit 0 with the expected shape');
}
NODE_DOWNSTREAM

if run_node "$TMP/downstream.mjs" extract "$FIXTURE" "$FIXTURE_BIN" "$FIXTURE_PROJECT" "$DISPATCH_WF" "$REVIEW_WF"; then
    pass "7b: both EMITTED engines transformed into importable modules (inverse transform reproduces them byte-for-byte; the untransformed file provably does not import)"
else
    fail "7b: extracting importable modules from the EMITTED engines failed"
fi

# --- 7c. Executed pure logic on the fixture's own diffs --------------------
say "7c. Executed pure logic: conditional dimensions fire on the fixture's real diffs; every built command names the fixture binary and project"
if run_node "$TMP/downstream.mjs" logic "$FIXTURE" "$FIXTURE_BIN" "$FIXTURE_PROJECT" "$DISPATCH_WF" "$REVIEW_WF"; then
    pass "7c: deriveSignals/selectDimensions fired on real Python/TypeScript diffs; zero rdm-specific literals in any built command"
else
    fail "7c: downstream pure-logic assertions failed"
fi

# --- 7d. Real execution against the fixture plan repo ----------------------
say "7d. Real execution: commands BUILT by the emitted engines run against the fixture plan repo"
if run_node "$TMP/downstream.mjs" exec "$FIXTURE" "$FIXTURE_BIN" "$FIXTURE_PROJECT" "$DISPATCH_WF" "$REVIEW_WF"; then
    pass "7d: built phase show / phase update / task show commands executed and exit 0 with the expected output shape"
else
    fail "7d: executing commands built by the emitted engines against the fixture plan repo failed"
fi

# --- 7e. Planted-corruption self-tests on the EMITTED bytes ----------------
say "7e. Self-tests: reintroducing an rdm-specific literal into the EMITTED bytes must turn 7c red"

# assert_corrupt_emitted_is_red <label> <sed-expr> <grep-needle> <engine>
# Copies the pristine fixture, mutates the EMITTED engine file the extractor
# reads (not a re-emission, not a local .claude/ copy), asserts the mutation
# APPLIED, then asserts the section-7c driver goes red on it.
assert_corrupt_emitted_is_red() {
    _label=$1
    _sed=$2
    _needle=$3
    _engine=$4
    _dir="$TMP/fixture-corrupt"
    rm -rf "$_dir"
    cp -R "$FIXTURE" "$_dir"
    _target="$_dir/repo/.claude/workflows/$_engine"
    sed "$_sed" "$_target" >"$_target.new"
    mv "$_target.new" "$_target"
    grep -q -- "$_needle" "$_target" ||
        fail "7e/$_label: the planted mutation did NOT apply to $_target — the self-test would be vacuous"
    if run_node "$TMP/downstream.mjs" logic "$_dir" "$FIXTURE_BIN" "$FIXTURE_PROJECT" "$DISPATCH_WF" "$REVIEW_WF" >/dev/null 2>&1; then
        fail "7e/$_label: the corrupted EMITTED bytes did NOT turn the downstream driver red — the gate is vacuous"
    fi
    rm -rf "$_dir"
    pass "7e/$_label: the planted corruption correctly turned the downstream driver red"
}

# A: the rdm dev-build binary path re-hardcoded into buildFetchPrompt.
assert_corrupt_emitted_is_red "A (binary literal)" \
    "s|const bin = resolveRdmBin(cfg \&\& cfg.rdmBin)|const bin = './target/debug/rdm'|" \
    "target/debug/rdm" "$DISPATCH_WF"
# B: this repo's own project flag re-hardcoded in place of projectFlag(cfg).
assert_corrupt_emitted_is_red "B (project literal)" \
    "s|const proj = projectFlag(cfg)|const proj = ' --project rdm'|" \
    "project rdm" "$DISPATCH_WF"
# C: the export vocabulary emptied — the api-docs dimension must stop firing,
# proving the conditional-dimension assertion is not vacuous.
assert_corrupt_emitted_is_red "C (dimension non-vacuity)" \
    "s|^const EXPORT_CONTENT_PATTERNS = \[|const EXPORT_CONTENT_PATTERNS = []; const EXPORT_CONTENT_PATTERNS_UNUSED = [|" \
    "EXPORT_CONTENT_PATTERNS_UNUSED" "$DISPATCH_WF"
# D: the same binary literal planted in the OTHER engine — one engine passing
# must never cover for the other.
assert_corrupt_emitted_is_red "D (review engine)" \
    "s|resolveRdmBin(cfg \&\& cfg.rdmBin)|'./target/debug/rdm'|" \
    "target/debug/rdm" "$REVIEW_WF"

# --- 7f. Harness self-gate: this file may never import a canonical source --
say "7f. Self-gate: the harness imports nothing under this repo's non-emitted workflow library"

# The canonical modules this repo stamps FROM are never emitted. Importing one
# would certify a local artifact instead of the downstream one — the exact
# "held the disproof in its hands" failure sections 7a-7e exist to close. The
# forbidden patterns are COMPOSED from fragments so this gate can never match
# its own source text.
assert_no_lib_import() {
    _file=$1
    _libdir="$(printf '%s' '.claude/workflows')$(printf '%s' '/lib/')"
    _hits=$(grep -c -- "$_libdir" "$_file" || true)
    [ "$_hits" -eq 0 ] || {
        echo "  forbidden: $_file references $_libdir" >&2
        return 1
    }
    _mjs=$(grep -cE -- "$(printf '%s' 'lib')$(printf '%s' '/[a-z-]*\.mjs')" "$_file" || true)
    [ "$_mjs" -eq 0 ] || {
        echo "  forbidden: $_file references a canonical-source module path" >&2
        return 1
    }
    return 0
}

assert_no_lib_import "$0" ||
    fail "7f: this harness references a non-emitted canonical source module — every module it executes must be derived from the EMITTED tree"
pass "7f: the harness references no non-emitted canonical source module"

MUTANT_HARNESS="$TMP/harness-lib-mutant"
cp "$0" "$MUTANT_HARNESS"
{
    printf '# planted for the 7f self-test: '
    printf '%s' '.claude/workflows'
    printf '%s' '/lib/'
    printf 'review.mjs\n'
} >>"$MUTANT_HARNESS"
if assert_no_lib_import "$MUTANT_HARNESS" >/dev/null 2>&1; then
    fail "7f self-test: a planted canonical-source reference was NOT detected — the self-gate is vacuous"
fi
pass "7f self-test: a planted canonical-source reference correctly turns the self-gate red"

say "8. Confirming $REPO_ROOT git status is unchanged after the whole run"
AFTER_STATUS=$(git -C "$REPO_ROOT" status --porcelain)
if [ "$BEFORE_STATUS" != "$AFTER_STATUS" ]; then
    printf 'before:\n%s\nafter:\n%s\n' "$BEFORE_STATUS" "$AFTER_STATUS" >&2
    fail "$REPO_ROOT git status changed during this run — the harness must only write under its own mktemp -d"
fi
pass "repo git status unchanged (hermetic)"

say "verify-agent-config-distribution.sh: ALL GREEN"
