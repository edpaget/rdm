#!/bin/sh
# Hermetic regression for the estimate workflow and its shared estimate-core.
#
# estimate (`.claude/workflows/rdm-wf-estimate.js`) rates an rdm roadmap's UNESTIMATED
# phases: it lists the phases, filters to those whose difficulty is unset,
# rates each in a parallel() fan-out, writes back the difficulty AND appends a
# `## Estimate <difficulty> — <justification>` audit note to the phase body, and
# reads the core-derived model tier back from `rdm phase show` for the summary.
# It NEVER passes `--model` and NEVER reimplements the difficulty->tier mapping —
# rdm-core (Difficulty::model_tier) owns that. Its pure estimate core lives once
# in `.claude/workflows/lib/estimate.mjs` (the `estimate-core` marker region) and
# is copied BYTE-IDENTICAL into a single consumer — rdm-wf-estimate.js — by
# scripts/gen-workflow-estimate.sh (the Workflow runtime cannot import a helper
# module; see docs/workflow-schemas.md § "Import spike"). The prose
# `rdm-autopilot` skill's estimate pre-pass invokes this same `rdm-wf-estimate`
# Workflow directly via the Workflow tool rather than reusing a stamped copy of
# this block (workflow-orchestration roadmap, phase 3 retired the earlier
# `autopilot.js`/`lib/autopilot.mjs` stamped copy). This harness gates all of
# that:
#
#   1. BEHAVIOR   — the pure helpers, driven in Node (zero LLM calls): arg
#                   parsing, phase selection, the estimator/writeback/list/tier
#                   prompt contents (note + --difficulty + --body, NO --model),
#                   the summary text, and determinism. These are the assertions
#                   re-homed from verify-workflow-autopilot.sh when the estimate
#                   core moved out of the autopilot-loop block.
#   1b. PIPELINE  — buildEstimatePipeline fed state-backed fakes: rates ONLY the
#                   unestimated phases, writeback carries the justification,
#                   reports the tier read back from showTier (never a JS map),
#                   narrows to a single phase number, is idempotent on re-run
#                   (a re-listed estimated phase is skipped), and is deterministic.
#                   Also drives the failure branches: a writeback reporting
#                   ok:false, and a parallelRate/writeback/showTier that THROWS,
#                   proving the pipeline degrades (logs + skips/continues) instead
#                   of misreporting a failed stem as estimated or aborting the run.
#   2. DRIFT      — scripts/gen-workflow-estimate.sh --check passes on the tree,
#                   with a planted-mutation self-test proving the gate is not a
#                   no-op and heals on restore.
#   3. STATIC     — rdm-wf-estimate.js loads under module semantics; no import/require;
#                   no Date.now / Math.random anywhere in the estimate sources; no
#                   `difficultyToTier` anywhere under .claude/workflows/; no
#                   *_SCHEMA handed to agent() with a top-level type:'array'
#                   (Anthropic tools require 'object'); meta.phases parity; and
#                   the rewritten rdm-estimate SKILL.md is a thin shim referencing
#                   rdm-wf-estimate.js with no retired rating-loop prose.
#   5. HERMETIC   — a temp git-backed plan repo seeded via the REAL target/debug/rdm
#      SEED         binary (mixed estimated/unestimated phases), whose actual
#                   `rdm phase list --format json` output is fed through
#                   selectUnestimated and buildEstimatePipeline with real-binary
#                   deps (real list / phase update --difficulty --body / phase show).
#                   This backs AC1 against the CLI's real JSON shape, so any drift
#                   between the field names the pure JS assumes (stem/difficulty/
#                   model) and what rdm-core emits is caught — not just the
#                   hand-fabricated fakes of sections 1/1b.
#   9. PARAM      — estimate names NO particular rdm executable and NO particular
#                   rdm project: both are RUNTIME args (`rdmBin`, `project`),
#                   the same contract the shipped review engine carries. Per-file literal
#                   zeroing with planted mutants (9a), a driven prompt capture
#                   checking every emitted `rdm <subcommand>` against the
#                   project-agnostic allow-list expressed AS DATA (9b), the
#                   fail-closed `rdmBin` rule (9c), and self-tests proving 9b is
#                   not vacuous (9d).
#
# Node is used only as a host to unit-test the pure module and drive the pipeline
# with fakes; it is stdlib-only (node:assert), with no package.json /
# node_modules / third-party packages. node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

WF_DIR="$REPO_ROOT/.claude/workflows"
LIB="$WF_DIR/lib/estimate.mjs"
WF="$WF_DIR/rdm-wf-estimate.js"
SKILL="$REPO_ROOT/.claude/skills/rdm-estimate/SKILL.md"
GEN="$SCRIPT_DIR/gen-workflow-estimate.sh"
RDM_BIN="$REPO_ROOT/target/debug/rdm"

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -f "$LIB" ] || fail "source module not found: $LIB"
[ -f "$WF" ] || fail "workflow script not found: $WF"
[ -f "$SKILL" ] || fail "rdm-estimate skill not found: $SKILL"
[ -x "$GEN" ] || fail "generator not found or not executable: $GEN"
[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

# Resolve a node command: prefer PATH, fall back to the mise-pinned toolchain.
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

# Parse a workflow script under MODULE semantics and fail on a SyntaxError. Strip
# the leading `export` and wrap in an async function so top-level `return`/`await`
# are legal, while keeping the top-level `const meta` in ONE shared scope so a
# redeclaration is a SyntaxError.
parse_workflow() {
    {
        echo '(async function(){'
        sed 's/^export //' "$1"
        echo '})'
    } |
        run_node --check --input-type=module -
}

# Distinct `phase: '<name>',` literals the workflow actually emits.
emitted_phases() {
    grep -oE "phase: '[A-Za-z]+'," "$1" | sed "s/phase: '//;s/',//" | sort -u
}
# Distinct `{ title: '<name>' }` entries declared in the `meta.phases` array.
declared_phases() {
    awk '/phases: \[/{p=1} p{print} p&&/\],?$/{exit}' "$1" |
        grep -oE "title: '[^']+'" | sed "s/title: '//;s/'\$//" | sort -u
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. BEHAVIOR -------------------------------------------------------------
say "1. Behavior: arg parsing, selection, prompt contents (note + --difficulty + --body, no --model), summary"

cat >"$TMP/behavior.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const m = await import(pathToFileURL(libPath).href);
const {
  parseEstimateArgs,
  selectUnestimated,
  estimateListCommand,
  buildEstimatorPrompt,
  buildEstimateWritebackCommands,
  buildEstimateSummaryText,
} = m;

// --- parseEstimateArgs -------------------------------------------------------
// The required-ROADMAP throw runs FIRST, before the environment axes, so its
// actionable message survives for the most common mis-invocation (§ 9c pins the
// rdmBin half of the same parse).
assert.throws(() => parseEstimateArgs({}), /roadmap slug is required/, 'roadmap slug required');
assert.throws(() => parseEstimateArgs({ roadmap: '' }), /roadmap slug is required/, 'empty roadmap rejected');
assert.deepEqual(
  parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm' }),
  { roadmap: 'rm', phase: null, rdmBin: 'rdm', project: '' },
  'defaults: no phase narrowing, no project flag'
);
assert.equal(parseEstimateArgs({ roadmap: 'rm', phase: 3, rdmBin: 'rdm' }).phase, 3, 'phase number kept');
assert.equal(parseEstimateArgs({ roadmap: 'rm', phase: '2', rdmBin: 'rdm' }).phase, 2, 'phase number coerced from string');
assert.throws(() => parseEstimateArgs({ roadmap: 'rm', phase: 0, rdmBin: 'rdm' }), /positive integer/, 'phase 0 rejected');
assert.throws(() => parseEstimateArgs({ roadmap: 'rm', phase: -1, rdmBin: 'rdm' }), /positive integer/, 'phase negative rejected');
// A caller may stringify the Workflow tool payload; coerce it instead of failing.
assert.equal(parseEstimateArgs('{"roadmap":"rm","rdmBin":"rdm"}').roadmap, 'rm', 'stringified JSON args coerced');
assert.throws(() => parseEstimateArgs('not json'), /roadmap slug is required/, 'non-JSON string falls back to actionable error');
assert.throws(() => parseEstimateArgs('null'), /roadmap slug is required/, 'JSON null rejected without a TypeError');

// --- selectUnestimated (re-homed from the autopilot harness) -----------------
assert.deepEqual(
  selectUnestimated([
    { stem: 'a' },
    { stem: 'b', difficulty: 'hard' },
    { stem: 'c', model: 'small' },
    { stem: 'd', difficulty: 'easy', model: 'small' },
  ]),
  ['a'],
  'only phases with NO difficulty and NO model are unestimated (both must be unset)'
);
assert.deepEqual(selectUnestimated([]), [], 'empty list');
assert.deepEqual(selectUnestimated(null), [], 'non-array tolerated');

// --- estimateListCommand: the read-only command the CALLER runs -------------
// (DELETED, no-mechanical-agents-in-workflows phase 34, commit 4: the
// buildEstimateListPrompt assertions. There is no list agent and no prompt.)
const CFG = { rdmBin: '/fake/bin/rdm', project: 'demo' };
assert.equal(
  estimateListCommand('rm', CFG),
  '/fake/bin/rdm phase list --roadmap rm --project demo --format json',
  'the list command names the roadmap, the injected binary and the project'
);
assert.equal(
  estimateListCommand('rm', { rdmBin: 'rdm' }),
  'rdm phase list --roadmap rm --format json',
  'no project configured -> no project flag at all'
);

// --- buildEstimatorPrompt (now requires a justification) ---------------------
const ratePrompt = buildEstimatorPrompt('some phase body');
assert.ok(ratePrompt.includes('some phase body'), 'estimator embeds the phase body');
assert.ok(ratePrompt.includes('trivial, easy, moderate, hard'), 'estimator lists the difficulty vocabulary');
assert.ok(/justification/i.test(ratePrompt), 'estimator asks for a justification');
assert.ok(ratePrompt.includes('"justification"'), 'estimator return schema includes the justification field');

// --- buildEstimateWritebackCommands: note + --difficulty, NO --model
// (DELETED, phase 34 commit 4: the buildEstimateWritebackPrompt and
// buildEstimateTierPrompt assertions. Neither prompt exists — the writeback is
// command TEXT the caller runs, and the tier is what its last command reads
// back. These assert that text instead.)
// (DELETED, phase 34 rework: the `--body` assertion. The ladder no longer
// performs a whole-document write at all — it appends the note through
// `--append-body`, so the phase body never leaves rdm-core. What the ladder DOES
// when run verbatim is decided by scripts/lib/estimate-writeback.test.mjs
// against the real binary, not by this string.)
const wbCmds = buildEstimateWritebackCommands('phase-1-x', 'hard', 'risky cross-cutting change', 'rm', CFG);
const wb = wbCmds.join('\n');
assert.ok(wb.includes('--difficulty hard'), 'writeback passes --difficulty');
assert.ok(wb.includes('## Estimate'), 'writeback appends a ## Estimate section');
assert.ok(wb.includes('hard — risky cross-cutting change'), 'the note carries "<difficulty> — <justification>"');
const wbUpdateLine = wbCmds.find((l) => l.includes('phase update phase-1-x'));
assert.ok(wbUpdateLine, 'writeback contains a phase update command line');
assert.ok(!wbUpdateLine.includes('--model'), 'the phase update command NEVER passes --model (tier derives in rdm-core)');
assert.ok(wb.includes("<<'RDM_ESTIMATE_EOF'"), 'the note is captured through a QUOTED heredoc');
assert.ok(wb.includes('--roadmap rm'), 'writeback scopes to the roadmap');
// The tier is READ BACK, never computed: the last command re-reads the phase.
assert.ok(
  wbCmds[wbCmds.length - 1].includes('phase show phase-1-x --roadmap rm --project demo --format json'),
  'the last command reads the phase back so the caller can see the core-derived tier'
);

// --- no prompt builder leaks a land/merge/main-mutation/completion directive -
const FORBIDDEN = ['Done:', '--land', '--commit', 'git merge', 'git push', 'checkout main'];
function hasForbidden(s) {
  return FORBIDDEN.some((f) => s.includes(f));
}
const allPrompts = [
  estimateListCommand('rm', CFG),
  buildEstimatorPrompt('a phase body'),
  buildEstimateWritebackCommands('phase-1-x', 'hard', 'why', 'rm', CFG).join('\n'),
];
for (const p of allPrompts) {
  assert.ok(!hasForbidden(p), 'no estimate prompt leaks a land/merge/commit/Done directive:\n' + p);
}
assert.ok(hasForbidden('run rdm phase update --land now'), 'forbidden-string detector catches a planted --land');

// --- buildEstimateSummaryText ------------------------------------------------
const text = buildEstimateSummaryText({
  roadmap: 'rm',
  estimated: [
    { stem: 'phase-1-a', difficulty: 'easy', justification: 'small' },
    { stem: 'phase-2-b', difficulty: 'hard', justification: 'big' },
  ],
  skipped: ['phase-3-c'],
});
assert.ok(text.includes('estimate summary for roadmap/rm'), 'summary names the roadmap');
assert.ok(text.includes('phase-1-a: easy — small'), 'summary lists the difficulty and its justification');
assert.ok(text.includes('phase-2-b: hard — big'), 'summary lists the second phase');
assert.ok(text.includes('skipped, already estimated (1): phase-3-c'), 'summary lists skipped phases');
assert.equal(
  buildEstimateSummaryText({ roadmap: 'rm', estimated: [], skipped: [] }),
  buildEstimateSummaryText({ roadmap: 'rm', estimated: [], skipped: [] }),
  'summary text is deterministic'
);

console.log('all estimate behavior assertions passed');
NODE_TEST

if run_node "$TMP/behavior.mjs" "$LIB"; then
    pass "pure helpers verified (args, selection, prompts, note/no-model, summary, determinism)"
else
    fail "estimate behavior assertions failed"
fi
# DELETED SECTION "1b." (no-mechanical-agents-in-workflows phase 34, commit 4):
# its subject was a mechanical agent, its model pin, or the caller hoist that
# suppressed it. None of those exists any more. Deleted and named, never
# repaired or re-pointed.

# --- 2. DRIFT GATE -----------------------------------------------------------
say "2. Drift: gen-workflow-estimate.sh --check passes on the committed tree"

if "$GEN" --check >/dev/null 2>&1; then
    pass "estimate-core is in sync in rdm-wf-estimate.js"
else
    "$GEN" --check >&2 || true
    fail "estimate-core DRIFTED — run scripts/gen-workflow-estimate.sh"
fi

# Self-test: mutate a consumer's estimate-core region in a scratch clone and prove
# --check FAILS, then restore and prove it heals. We drive the generator against a
# scratch repo so the real tree is never touched.
say "2b. Drift detector fires on planted drift inside a consumer's estimate-core region (self-test)"
# Plant the mutation directly in rdm-wf-estimate.js, run --check, then restore.
cp "$WF" "$TMP/rdm-wf-estimate.js.orig"
# Mutate one line inside the estimate-core region (the summary header string).
sed 's/estimate summary for roadmap/PLANTED DRIFT for roadmap/' "$WF" >"$TMP/rdm-wf-estimate.js.mut"
cp "$TMP/rdm-wf-estimate.js.mut" "$WF"
if "$GEN" --check >/dev/null 2>&1; then
    cp "$TMP/rdm-wf-estimate.js.orig" "$WF"
    fail "drift gate did NOT fire on a planted mutation inside rdm-wf-estimate.js's estimate-core region"
fi
cp "$TMP/rdm-wf-estimate.js.orig" "$WF"
if "$GEN" --check >/dev/null 2>&1; then
    pass "drift detector fires on a planted mutation and heals on restore"
else
    "$GEN" --check >&2 || true
    fail "restore did not heal the drift gate"
fi

# --- 3. STATIC INVARIANTS ----------------------------------------------------
say "3. Static invariants on rdm-wf-estimate.js and the estimate sources"

# 3a. Module parse.
if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "rdm-wf-estimate.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "rdm-wf-estimate.js does NOT parse — fix the SyntaxError"
fi

# 3b. No import/require (the runtime forbids it — sharing is by stamped copy).
if grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >/dev/null 2>&1; then
    grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >&2 || true
    fail "rdm-wf-estimate.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "rdm-wf-estimate.js must not require() (the runtime forbids it)"
fi
grep -q '>>> estimate-core:begin' "$WF" || fail "missing estimate-core:begin marker in rdm-wf-estimate.js"
grep -q '>>> estimate-core:end' "$WF" || fail "missing estimate-core:end marker in rdm-wf-estimate.js"
pass "no import/require; both estimate-core markers present in rdm-wf-estimate.js"

# 3c. No Date.now / Math.random anywhere in the estimate sources.
if grep -nE 'Date\.now\(|Math\.random\(' "$WF" "$LIB" 2>/dev/null; then
    fail "found Date.now( / Math.random( in an estimate source — the runtime forbids them and they break determinism"
fi
printf 'const x = Date.now();\n' >"$TMP/planted-nondeterm.js"
if ! grep -nE 'Date\.now\(|Math\.random\(' "$TMP/planted-nondeterm.js" >/dev/null 2>&1; then
    fail "hygiene grep did NOT catch a planted Date.now() — the detector is broken"
fi
pass "no Date.now / Math.random in rdm-wf-estimate.js or lib/estimate.mjs; detector catches a planted one"

# 3d. No difficultyToTier ANYWHERE under .claude/workflows/ (rdm-core owns the map).
if grep -rn 'difficultyToTier' "$WF_DIR" >/dev/null 2>&1; then
    grep -rn 'difficultyToTier' "$WF_DIR" >&2 || true
    fail "difficultyToTier must not appear anywhere under .claude/workflows/ — rdm-core (Difficulty::model_tier) owns the difficulty->tier policy"
fi
printf 'function difficultyToTier() {}\n' >"$TMP/planted-d2t.js"
grep -q 'difficultyToTier' "$TMP/planted-d2t.js" || fail "difficultyToTier detector broken"
pass "no difficultyToTier anywhere under .claude/workflows/; detector catches a planted one"

# 3e. No *_SCHEMA handed to agent() may declare a top-level type:'array'.
schema_array_offenders() {
    awk '
        /^const [A-Za-z_]+_SCHEMA = \{/ { name = $2; expect = 1; next }
        expect == 1 { if ($0 ~ /type: .array./) print name; expect = 0 }
    ' "$1"
}
OFFENDERS=$(schema_array_offenders "$WF" || true)
if [ -n "$OFFENDERS" ]; then
    printf 'top-level type:array schema(s): %s\n' "$(echo "$OFFENDERS" | tr '\n' ' ')" >&2
    fail "no *_SCHEMA handed to agent() may use a top-level type:'array' (Anthropic tools require 'object'); offending: $OFFENDERS"
fi
# DELETED (no-mechanical-agents-in-workflows phase 34, commit 4): the
# `r.phases` unwrap assertion. PHASE_LIST_SCHEMA and the `estimate:list` agent it
# shaped are gone — the caller passes the parsed array as `phaseList`, so there
# is no wrapper to unwrap.
sed "s/^  type: 'object',/  type: 'array',/" "$WF" >"$TMP/wf.array.scratch"
if [ -z "$(schema_array_offenders "$TMP/wf.array.scratch")" ]; then
    fail "top-level-array detector did NOT fire on a planted type:'array' schema"
fi
pass "no *_SCHEMA uses a top-level type:'array'; detector catches a planted array schema"

# 3f. meta.phases parity.
DECLARED_PHASES=$(declared_phases "$WF")
EMITTED_PHASES=$(emitted_phases "$WF")
if [ "$DECLARED_PHASES" = "$EMITTED_PHASES" ]; then
    pass "meta.phases lists exactly the emitted phase: literals ($(echo "$EMITTED_PHASES" | tr '\n' ' '))"
else
    printf 'declared (meta.phases): %s\n' "$(echo "$DECLARED_PHASES" | tr '\n' ' ')" >&2
    printf 'emitted   (phase: ...): %s\n' "$(echo "$EMITTED_PHASES" | tr '\n' ' ')" >&2
    fail "meta.phases drift: declared phases != emitted phase: literals"
fi

# 3g. THE RATER IS THE ONLY AGENT, AND IT IS NOT PINNED TO A MECHANICAL TIER.
# (DELETED, no-mechanical-agents-in-workflows phase 34, commit 4: the
# `estimate:list` / `estimate:write:` / `estimate:tier:` model-pin assertions and
# their repoint self-test. Those three agents are gone — the caller does the list
# read and runs the returned writeback commands — so there is no mechanical tier
# left to pin. What remains is the negative half, which still has a referent.)
# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/lib/mechanical-tier-check.sh"

agent_option_blocks "$WF" >"$TMP/mech-blocks"
[ -s "$TMP/mech-blocks" ] || fail "could not extract any agent() option blocks from rdm-wf-estimate.js"

RATER_SITES=$(grep -c "label: 'estimate:rate:" "$WF" || true)
[ "$RATER_SITES" -eq 1 ] || fail "rdm-wf-estimate.js must dispatch exactly one kind of agent (the rater); found $RATER_SITES estimate:rate: sites"
OTHER_LABELS=$(grep -oE "label: '[^']*'" "$WF" | grep -vc "estimate:rate:" || true)
[ "$OTHER_LABELS" -eq 0 ] || fail "rdm-wf-estimate.js dispatches a non-rater agent — the rater is the only agent left"
pass "the rater is the only agent rdm-wf-estimate.js dispatches"

assert_label_not_model "$TMP/mech-blocks" 'estimate:rate:' 'mechanicalModel' ||
    fail "estimate:rate:<stem> must NOT be pinned to a mechanical model (judgment stage)"
pass "estimate:rate:<stem> is left unpinned (judgment stage)"

# --- 4. SKILL SHIM -----------------------------------------------------------
say "4. rdm-estimate SKILL.md is a thin shim referencing rdm-wf-estimate.js with no retired rating-loop prose"

grep -qF '.claude/workflows/rdm-wf-estimate.js' "$SKILL" || fail "SKILL.md must reference '.claude/workflows/rdm-wf-estimate.js'"
grep -q 'Workflow' "$SKILL" || fail "SKILL.md must invoke the estimate Workflow"
# The retired step-by-step rating loop prose must be gone.
for retired in "Rate its difficulty as one of" "body=\$(cat <<'EOF'" "Skipping is the override mechanism"; do
    if grep -qF -- "$retired" "$SKILL"; then
        fail "SKILL.md still contains retired rating-loop prose: $retired"
    fi
done
# The shim must not re-narrate the per-phase heredoc writeback command.
if grep -qF -- '--difficulty <difficulty> --body' "$SKILL"; then
    fail "SKILL.md still re-narrates the writeback heredoc command — it should defer to the workflow"
fi
pass "SKILL.md is a thin shim: references rdm-wf-estimate.js, no retired rating-loop prose"

# --- 5. HERMETIC SEED (real target/debug/rdm) --------------------------------
say "5. Hermetic seed: real rdm JSON drives selectUnestimated / buildEstimatePipeline against a temp plan repo"

PLAN="$TMP/plan"
PROJ="est-verify"
ROADMAP="rm-est"
rdmbin() { "$RDM_BIN" --root "$PLAN" "$@"; }

mkdir -p "$PLAN"
rdmbin init --default-project "$PROJ" >/dev/null
rdmbin roadmap create "$ROADMAP" --title "Estimate RM" --body "seed" \
    --no-edit --project "$PROJ" >/dev/null
rdmbin phase create a --title "A" --number 1 --body "phase a body" \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
rdmbin phase create b --title "B" --number 2 --body "phase b body" \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
rdmbin phase create c --title "C" --number 3 --body "phase c body" \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
# Pre-estimate phase 2 so it must be SKIPPED (real rdm derives model=large from
# difficulty=hard — no --model passed).
rdmbin phase update phase-2-b --difficulty hard \
    --no-edit --roadmap "$ROADMAP" --project "$PROJ" >/dev/null
rdmbin commit -m "seed: estimate harness fixtures" >/dev/null
pass "seeded a real roadmap: phase-1-a / phase-3-c unestimated, phase-2-b pre-estimated (hard)"

cat >"$TMP/real.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const [libPath, RDM, PLAN, PROJ, ROADMAP] = process.argv.slice(2);
const m = await import(pathToFileURL(libPath).href);
const { selectUnestimated, buildEstimatePipeline } = m;

// rdm prints clean JSON on stdout (informational notices go to stderr); still
// parse only the leading JSON value defensively (handles a mixed array/object
// with a trailing notice).
function rdm(args) {
  // Capture stdout; silence rdm's informational stderr notices (staged/uncommitted).
  return execFileSync(RDM, ['--root', PLAN, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
}
function parseLeadingJson(text) {
  const s = text.replace(/^\s+/, '');
  let depth = 0;
  let started = false;
  let end = -1;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '{' || c === '[') {
      depth++;
      started = true;
    } else if (c === '}' || c === ']') {
      depth--;
      if (started && depth === 0) {
        end = i + 1;
        break;
      }
    }
  }
  return JSON.parse(end === -1 ? s : s.slice(0, end));
}
function rdmJson(args) {
  return parseLeadingJson(rdm(args));
}

// --- Field-shape fidelity: REAL phase list feeds selectUnestimated -----------
// The real CLI OMITS difficulty/model on an unestimated phase (skip_serializing_if
// none). If selectUnestimated read the wrong field names, this would mis-select.
const listed = rdmJson(['phase', 'list', '--roadmap', ROADMAP, '--project', PROJ, '--format', 'json']);
assert.ok(Array.isArray(listed) && listed.length === 3, 'real phase list returns the three seeded phases');
assert.deepEqual(
  selectUnestimated(listed).slice().sort(),
  ['phase-1-a', 'phase-3-c'],
  'real phase list: only the two truly-unestimated phases select (phase-2-b has difficulty+model set)'
);

// DELETED (no-mechanical-agents-in-workflows phase 34, commit 4): the
// real-binary pipeline drive. Its `list` / `writeback` / `showTier` deps are the
// three mechanical agents this phase removed — the pipeline now takes the phase
// list as an argument and RETURNS the writeback as command text, so there is no
// injected side-effecting dep left to point at the real binary. What survives
// above is the half whose subject is untouched: real `rdm phase list` JSON fed
// through `selectUnestimated`.
console.log('ALL HERMETIC-SEED ASSERTIONS PASSED');
NODE_TEST

if run_node "$TMP/real.mjs" "$LIB" "$RDM_BIN" "$PLAN" "$PROJ" "$ROADMAP"; then
    pass "real rdm JSON round-trips through selectUnestimated / buildEstimatePipeline; tier derives in core; note lands"
else
    fail "hermetic real-binary estimate assertions failed"
fi
# DELETED SECTION "HOIST." (no-mechanical-agents-in-workflows phase 34, commit 4):
# its subject was a mechanical agent, its model pin, or the caller hoist that
# suppressed it. None of those exists any more. Deleted and named, never
# repaired or re-pointed.

# DELETED SECTION "HOIST-SHIM." (no-mechanical-agents-in-workflows phase 34, commit 4):
# its subject was a mechanical agent, its model pin, or the caller hoist that
# suppressed it. None of those exists any more. Deleted and named, never
# repaired or re-pointed.

# --- 9. PARAMETERIZATION ------------------------------------------------------
# estimate names NO particular rdm executable and NO particular rdm project:
# both arrive as RUNTIME args (`rdmBin`, `project`) and are threaded into every
# prompt that shells out. This mirrors scripts/verify-agent-config-distribution.sh
# § 7c, which gates the SAME contract for the shipped review engine — the helpers
# here are copies in shape, not a second contract. Four sub-gates:
#
#   9a — per-file literal zeroing across BOTH copies (lib + workflow), asserted
#        PER FILE so a half-applied edit cannot pass.
#   9b — a DRIVEN prompt capture: run the real workflow under a capturing fake
#        agent, tokenize every emitted `rdm <subcommand>` occurrence, and check
#        it against the project-agnostic allow-list expressed AS DATA.
#   9c — the fail-closed `rdmBin` rule (and the optional-project validation),
#        plus a grep proving it was NOT implemented as an existence preflight.
#   9d — planted-mutation self-tests for 9b.
say "9. Parameterization: no hardcoded rdm binary or project; the environment axes are runtime args"

# --- 9a. Per-file literal zeroing ---------------------------------------------
say "9a. Per-file literal zeroing (lib + workflow)"

# assert_no_env_literals <file> — zero occurrences of THIS repo's dev binary path
# and zero of THIS repo's project flag. Deliberately per-file: a concatenated
# stream would let a zero in one copy mask a hit in the other, which is exactly
# the half-applied-edit failure mode (the estimate-core block is byte-stamped,
# the driver below it is hand-swept). Comments and prose count too — a leftover
# explanatory comment naming either literal is the same staleness hazard.
assert_no_env_literals() {
    _f=$1
    _bin=$(grep -c 'target/debug/rdm' "$_f" || true)
    _proj=$(grep -c -- '--project rdm' "$_f" || true)
    [ "$_bin" -eq 0 ] && [ "$_proj" -eq 0 ]
}

for f in "$LIB" "$WF"; do
    if assert_no_env_literals "$f"; then
        pass "9a: ${f#"$REPO_ROOT"/} carries neither 'target/debug/rdm' nor '--project rdm'"
    else
        grep -n 'target/debug/rdm' "$f" >&2 || true
        grep -n -- '--project rdm' "$f" >&2 || true
        fail "9a: $f still hardcodes this repo's rdm binary and/or project — both must be runtime args"
    fi
done

# Self-test: plant the literals into EACH file in turn and prove the per-file
# check fires on each one individually (so restoring only one cannot go green).
_i=0
for f in "$LIB" "$WF"; do
    _i=$((_i + 1))
    cp "$f" "$TMP/env-mutant-$_i"
    printf '\n// planted: ./target/debug/rdm phase show --project rdm\n' >>"$TMP/env-mutant-$_i"
    if assert_no_env_literals "$TMP/env-mutant-$_i"; then
        fail "9a: the per-file literal check did not fire on a planted literal in $f — the gate is vacuous"
    fi
done
pass "9a: the per-file check fires independently on both planted mutants"

# --- 9b. Driven prompt capture ------------------------------------------------
say "9b. Driven prompt capture: every emitted rdm invocation honors the allow-list"

cat >"$TMP/paramz.mjs" <<'NODE_PARAMZ'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-estimate-paramz-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const run = mod.default;

async function refParallel(thunks) {
  return Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
}

// The injected binary is deliberately NOT a plausible real path, so a
// re-hardcoded './target/debug/rdm' anywhere shows up as a mismatch rather than
// blending in.
const FAKE_BIN = '/fake/bin/rdm';

// The PROJECT-AGNOSTIC ALLOW-LIST, expressed as DATA — the SAME array
// verify-agent-config-distribution.sh § 7c uses (the lane's landed contract,
// not re-derived here). These subcommands reject `--project` outright, so they must
// carry NO project flag; everything else is project-scoped and MUST carry it
// whenever a project was configured.
const PROJECT_AGNOSTIC = ['model resolve', 'commit', 'status', 'discard'];

const PHASES = [{ number: 1, stem: 'phase-1-x', title: 'X', status: 'not-started' }];

// A capturing fake agent. The only agent left is the rater, so the scan's real
// subject has MOVED: the rdm invocations this engine emits are now the WRITEBACK
// COMMANDS it returns, not prompts it hands an agent. `capture` below therefore
// scans both — the rater's prompt (which still names a `phase show`) and the
// returned command lists. `phaseList` is supplied because the engine refuses to
// run without it; there is no agent left for withholding it to exercise.
function makeCapture() {
  const prompts = [];
  const agent = async (prompt, opts) => {
    prompts.push(String(prompt));
    const label = (opts && opts.label) || '';
    if (label.startsWith('estimate:rate:')) {
      return { stem: label.slice('estimate:rate:'.length), difficulty: 'moderate', justification: 'j' };
    }
    throw new Error('unexpected agent label: ' + label);
  };
  return { agent, prompts };
}

// Tokenize `<bin> <subcommand>` occurrences out of a prompt. The binary token is
// whatever non-space run precedes the subcommand, so a re-hardcoded path is
// caught by comparison rather than by being silently skipped. Same regex as
// verify-agent-config-distribution.sh § 7c.
const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?rdm)\s+([a-z][a-z-]*(?:\s+[a-z][a-z-]*)?)/g;

// Only COMMAND-BEARING lines are tokenized. Every command these prompts emit is
// either an indented command line ('  <bin> phase list …') or a backtick-quoted
// inline directive ('Run `<bin> phase show …`'). Flush-left PROSE that merely
// names the tool — the estimator prompt opens "You are a difficulty-estimation
// agent for a single rdm phase." — is not an invocation, and tokenizing it would
// report a false hit whose "binary" is the bare word `rdm`. The non-vacuity
// floors below prove the filter is not silently dropping real commands.
function isCommandLine(line) {
  return /^\s{2,}\S/.test(line) || line.includes('`');
}

function scan(prompts) {
  const out = [];
  for (const p of prompts) {
    for (const line of p.split('\n')) {
      if (!isCommandLine(line)) continue;
      INVOCATION.lastIndex = 0;
      let m;
      while ((m = INVOCATION.exec(line)) !== null) {
        out.push({ bin: m[2], two: m[3], line });
      }
    }
  }
  return out;
}

async function capture(args) {
  const c = makeCapture();
  const summary = await run(Object.assign({ phaseList: PHASES }, args), c.agent, refParallel, () => {});
  const emitted = (summary && summary.estimated ? summary.estimated : []).flatMap((e) => e.writebackCommands || []);
  return scan(c.prompts.concat(emitted));
}

// --- Run A: a project IS configured.
const withProject = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN, project: 'demo' });
assert.ok(withProject.length > 0, 'the scan found no rdm invocations at all — it cannot pass vacuously');

const seen = new Set();
for (const occ of withProject) {
  assert.equal(occ.bin, FAKE_BIN, 'an rdm invocation used ' + occ.bin + ' instead of the injected rdmBin: ' + occ.line);
  seen.add(occ.two);
  const agnostic = PROJECT_AGNOSTIC.includes(occ.two);
  if (agnostic) {
    assert.ok(!occ.line.includes('--project'), 'project-agnostic `rdm ' + occ.two + '` must carry NO project flag: ' + occ.line);
  } else {
    assert.ok(occ.line.includes(' --project demo'), 'project-scoped `rdm ' + occ.two + '` must carry " --project demo": ' + occ.line);
  }
}

// Non-vacuity floors: the scan must actually have reached every command shape,
// not merely found nothing to object to.
// (`phase list` and `model resolve` are no longer emitted by this engine — the
// caller runs them — so they left this floor with the agents that emitted them.)
for (const n of ['phase show', 'phase update']) {
  assert.ok(seen.has(n), 'expected at least one `rdm ' + n + '` occurrence, saw: ' + [...seen].join(', '));
}

// --- Run B: NO project configured -> not a single --project anywhere.
const noProject = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN });
assert.ok(noProject.length > 0, 'the no-project scan found no rdm invocations at all');
for (const occ of noProject) {
  assert.equal(occ.bin, FAKE_BIN, '(no project): an rdm invocation used ' + occ.bin + ': ' + occ.line);
}
const stray = noProject.filter((o) => o.line.includes('--project'));
assert.equal(stray.length, 0, '(no project): expected zero --project occurrences, found: ' + stray.map((o) => o.line).join(' | '));

// Determinism: the same args produce byte-identical prompt captures.
const d1 = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN, project: 'demo' });
const d2 = await capture({ roadmap: 'rm', rdmBin: FAKE_BIN, project: 'demo' });
assert.deepEqual(d2, d1, 'the prompt capture must be deterministic across identical runs');

console.log('all estimate parameterization prompt-capture assertions passed');
NODE_PARAMZ

if run_node "$TMP/paramz.mjs" "$WF"; then
    pass "9b: every emitted rdm invocation uses the injected binary and honors the project-agnostic allow-list (with and without a project)"
else
    fail "9b: parameterization prompt-capture assertions failed"
fi

# --- 9c. Fail-closed rdmBin ---------------------------------------------------
say "9c. Defaulted rdmBin: an absent arg resolves to a plain 'rdm', a wrong-TYPE arg still throws, and neither is an existence preflight"

cat >"$TMP/rdmbin.mjs" <<'NODE_RDMBIN'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const libPath = process.argv[2];
const wfPath = process.argv[3];
const { parseEstimateArgs, projectFlag, resolveRdmBin, parseProjectArg } = await import('file://' + libPath);

// (1a) An ABSENT-ish value DEFAULTS to a plain `rdm` on PATH. A plugin-installed
// consumer has no repo-local build path to pass; this repo's own stale-global
// hazard is handled by RDM_BIN in .mise.toml, not by refusing to resolve.
for (const absent of [undefined, null, '', '   ', '\t']) {
  assert.equal(resolveRdmBin(absent), 'rdm', 'an absent rdmBin (' + JSON.stringify(absent) + ') must default to "rdm"');
  assert.equal(
    parseEstimateArgs({ roadmap: 'rm', rdmBin: absent }).rdmBin,
    'rdm',
    'parseEstimateArgs must default an absent rdmBin (' + JSON.stringify(absent) + ') to "rdm"'
  );
}
assert.equal(parseEstimateArgs({ roadmap: 'rm' }).rdmBin, 'rdm', 'a missing rdmBin key defaults to "rdm"');
assert.equal(parseEstimateArgs(JSON.stringify({ roadmap: 'rm' })).rdmBin, 'rdm', 'a stringified payload defaults rdmBin to "rdm"');

// (1b) A present-but-wrong-TYPE value STILL throws — degrading a `rdmBin: 42`
// typo to PATH would reintroduce the silent-wrong-binary hazard.
for (const bad of [42, {}, [], true]) {
  assert.throws(
    () => parseEstimateArgs({ roadmap: 'rm', rdmBin: bad }),
    /rdmBin/,
    'a non-string rdmBin (' + JSON.stringify(bad) + ') must still throw'
  );
}

// ORDERING: the pre-existing required-roadmap throw still runs FIRST. This
// assertion is UNCHANGED and deliberately kept: it now holds for a weaker
// reason (an absent rdmBin no longer competes to throw at all), but it is the
// only guard that the far more common missing-roadmap mis-invocation keeps its
// actionable message, and it must not be collateral damage of the rdmBin edit.
assert.throws(() => parseEstimateArgs({}), /roadmap slug is required/, 'a payload missing BOTH reports the roadmap first');
assert.throws(() => parseEstimateArgs({ rdmBin: 'rdm' }), /roadmap slug is required/, 'roadmap is still checked first');

// The wrong-TYPE error must stay actionable: it names the sentinel and the
// default that omitting the arg entirely would take.
try {
  parseEstimateArgs({ roadmap: 'rm', rdmBin: 42 });
  assert.fail('expected a throw');
} catch (e) {
  assert.match(e.message, /rdmBin must be a string/, 'the message names the type requirement');
  assert.match(e.message, /"rdm"/, 'the message names the explicit PATH sentinel');
  assert.match(e.message, /PATH/, 'the message names what omitting the arg entirely does');
}

// (2) The explicit sentinel is accepted VERBATIM — a downstream repo that wants
// PATH resolution opts in on purpose.
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm' }).rdmBin, 'rdm', "the 'rdm' sentinel is accepted verbatim");
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: '/opt/x/rdm' }).rdmBin, '/opt/x/rdm', 'an absolute path is accepted verbatim');
assert.equal(resolveRdmBin('rdm'), 'rdm', 'resolveRdmBin passes the sentinel through');
// DISCRIMINATING: the sentinel path is VERBATIM pass-through, not the default
// branch — a trailing space survives, which `return 'rdm'` could not produce.
assert.equal(resolveRdmBin('rdm '), 'rdm ', 'a non-empty value is returned verbatim, not normalized to the default');

// (3) `project` is OPTIONAL, falsy means NO flag, and a hostile value is
// rejected rather than escaped (it is interpolated into a Bash-agent prompt).
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm' }).project, '', 'an absent project means no flag');
for (const falsy of [undefined, null, '', 0, false]) {
  assert.equal(parseProjectArg(falsy), '', 'falsy project ' + JSON.stringify(falsy) + ' means no flag');
  assert.equal(projectFlag({ project: parseProjectArg(falsy) }), '', 'a falsy project emits no flag at all');
}
assert.equal(projectFlag({}), '', 'an empty cfg emits no flag');
assert.equal(projectFlag(null), '', 'a null cfg emits no flag');
assert.equal(projectFlag({ project: 'demo' }), ' --project demo', 'a configured project emits the flag');
for (const hostile of ['a b', 'a;rm -rf /', '$(x)', '`x`', 'a\nb', 'a|b', 7, {}]) {
  assert.throws(() => parseProjectArg(hostile), /project must be a plain project name/, 'hostile project ' + JSON.stringify(hostile) + ' must be rejected');
}
assert.equal(parseEstimateArgs({ roadmap: 'rm', rdmBin: 'rdm', project: 'rdm-atlas.v2_x' }).project, 'rdm-atlas.v2_x', 'a plain project name survives');

// (4) Driving the wrapped workflow with NO rdmBin must now RESOLVE, and every
// rdm command it emits must name the bare `rdm` default — never a repo-local
// build path leaking back in as a hardcoded literal.
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-estimate-rdmbin-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const PHASES = [{ number: 1, stem: 'phase-1-x', title: 'X', status: 'not-started' }];
const prompts = [];
let agentCalls = 0;
const spy = async (prompt, opts) => {
  agentCalls++;
  prompts.push(String(prompt));
  const label = (opts && opts.label) || '';
  if (label.startsWith('estimate:rate:')) {
    return { stem: label.slice('estimate:rate:'.length), difficulty: 'moderate', justification: 'j' };
  }
  return null;
};
const refParallel = async (thunks) => Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
// `phaseList` is supplied because the engine refuses to run without it; the
// EMITTED commands it returns are scanned alongside the rater's prompt, since
// that is where this engine's rdm invocations now live.
const out = await mod.default({ roadmap: 'rm', phaseList: PHASES }, spy, refParallel, () => {});
assert.ok(out !== undefined, 'a Workflow invocation with no rdmBin must resolve, not throw');
assert.ok(agentCalls > 0, 'the run must actually have dispatched agents — otherwise this assertion is vacuous');

// Same command-line filter as § 9b: only indented or backtick-quoted lines are
// invocations; flush-left prose merely naming the tool is not.
const INVOCATION = /(^|[\s`])((?:[^\s`]*\/)?rdm)\s+[a-z][a-z-]*/g;
let invocations = 0;
const emitted = (out && out.estimated ? out.estimated : []).flatMap((e) => e.writebackCommands || []);
for (const p of prompts.concat(emitted)) {
  for (const line of p.split('\n')) {
    if (!(/^\s{2,}\S/.test(line) || line.includes('`'))) continue;
    INVOCATION.lastIndex = 0;
    let m;
    while ((m = INVOCATION.exec(line)) !== null) {
      invocations++;
      assert.equal(m[2], 'rdm', 'an emitted command used ' + m[2] + ' instead of the bare `rdm` default: ' + line);
    }
  }
}
assert.ok(invocations > 0, 'the scan found no rdm invocations at all — it cannot pass vacuously');
for (const p of prompts.concat(emitted)) {
  assert.ok(!p.includes('./target/debug/rdm'), 'a repo-local build path leaked into a prompt or an emitted command under the bare default');
}

console.log('all defaulted rdmBin assertions passed');
NODE_RDMBIN

if run_node "$TMP/rdmbin.mjs" "$LIB" "$WF"; then
    pass "9c: rdmBin defaults to a bare 'rdm' when absent (every emitted command uses it), a wrong-TYPE value still throws, the 'rdm' sentinel passes through verbatim, the roadmap throw still runs first, and project is validated"
else
    fail "9c: defaulted rdmBin assertions failed"
fi

# The guard must NOT be an existence preflight. `which -a rdm` resolves to the
# stale global build in this repo, so an existence check passes while running
# exactly the binary the development-build rule forbids. COMMENT LINES ARE
# STRIPPED FIRST so a rationale comment naming the rejected mechanism is not
# itself flagged.
assert_no_existence_preflight() {
    grep -vE '^[[:space:]]*(//|\*|/\*)' "$1" |
        grep -nE 'which +(-a +)?rdm|command -v|existsSync|accessSync|statSync' >"$TMP/preflight-hits" 2>/dev/null || true
    [ ! -s "$TMP/preflight-hits" ]
}
for f in "$LIB" "$WF"; do
    if ! assert_no_existence_preflight "$f"; then
        cat "$TMP/preflight-hits" >&2
        fail "9c: $f must not implement the rdmBin guard as an existence preflight — the guard is on the ABSENCE of the argument"
    fi
done
pass "9c: no existence preflight (which rdm / command -v / existsSync) in either estimate copy"

cp "$WF" "$TMP/preflight-mutant.js"
printf "\nconst ok = existsSync(rdmBin)\n" >>"$TMP/preflight-mutant.js"
if assert_no_existence_preflight "$TMP/preflight-mutant.js"; then
    fail "9c: the existence-preflight detector missed a planted existsSync call — the gate is vacuous"
fi
pass "9c: the existence-preflight detector fires on planted code while ignoring the rationale prose"

# --- 9d. Planted-mutation self-tests for 9b -----------------------------------
say "9d. Planted-mutation self-tests: the allow-list assertion is not vacuous"

# (i) a builder RE-HARDCODES this repo's dev binary path. (Anchor moved from the
# deleted `phase list` builder to `phase show`, which the writeback commands
# emit — it is the same assertion over the artifact that replaced the one it
# used to read.)
sed "s|const show = bin + ' phase show |const show = './target/debug/rdm' + ' phase show |" "$WF" >"$TMP/pz-mut-bin.js"
if cmp -s "$WF" "$TMP/pz-mut-bin.js"; then
    fail "9d(i): the re-hardcoded-binary mutation did not apply — the self-test is not exercising anything"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-bin.js" >/dev/null 2>&1; then
    fail "9d(i): a re-hardcoded rdm binary was NOT detected — the binary assertion is vacuous"
fi
pass "9d(i): detector fires when a builder re-hardcodes the rdm binary"

# DELETED (no-mechanical-agents-in-workflows phase 34, commit 4): mutation (ii).
# Its subject was `rdm model resolve mechanical` gaining a project flag; this
# engine no longer emits that command at all, so there is nothing to mutate.

# (ii) a project-scoped builder DROPS its flag.
sed "s|+ ' --roadmap ' + slug + proj + ' --format json'|+ ' --roadmap ' + slug + ' --format json'|g" "$WF" >"$TMP/pz-mut-drop.js"
if cmp -s "$WF" "$TMP/pz-mut-drop.js"; then
    fail "9d(ii): the dropped-flag mutation did not apply"
fi
if run_node "$TMP/paramz.mjs" "$TMP/pz-mut-drop.js" >/dev/null 2>&1; then
    fail "9d(ii): a project-scoped command that dropped its project flag was NOT detected"
fi
pass "9d(ii): detector fires when a project-scoped builder drops '+ proj'"

say "verify-workflow-estimate.sh: ALL GREEN"
