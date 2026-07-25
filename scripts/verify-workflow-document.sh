#!/bin/sh
# Hermetic regression for the `document` workflow (headless roadmap-to-docs
# drafting).
#
# `document` (`.claude/workflows/document.js`) validates that every phase of a
# roadmap is `done`, fans out a per-phase git-gather step in parallel()
# (falling back to phase-body-only when a phase has no commit SHA), runs one
# synthesis agent to draft the doc, and a mechanical Bash agent to write it to
# `--out` (default `docs/<slug>.md`). It returns
# { roadmap, aborted, incompletePhases, path, draft } and performs NO status
# mutation and NO approval step — the terminal human review lives in the
# `rdm-document` skill shim, never inside the workflow. Its pure decision core
# lives once in `.claude/workflows/lib/document.mjs` and is copied
# BYTE-IDENTICAL into the workflow script (the Workflow runtime cannot import a
# helper module — see docs/workflow-schemas.md § "Import spike"). This harness
# gates four things:
#
#   1. STATIC INVARIANTS — grep-based assertions on the workflow source and the
#      skill shim: the parallel() fan-out, the --out / rdm-show wiring, the
#      abort-on-incomplete and has-SHA/body-only branches, both document-core
#      markers present, no `--status` mutation or plan-mode/confirmation call
#      anywhere in document.js, no `Date.now(`/`Math.random(` in document.js or
#      lib/document.mjs, the skill shim retains the terminal
#      "not done until reviewed and approved" human-approval language, and the
#      skill shim no longer carries the old step-by-step git-gather prose.
#   2. BLOCK DRIFT — the `document-core` region is byte-identical between the
#      lib source of truth and the stamped workflow script (with a
#      planted-mutation self-test proving the gate is not a no-op).
#   3. BEHAVIOR — the pure decision logic, driven in Node with fabricated
#      args/phase arrays (zero LLM calls): parseDocumentArgs' JSON-string
#      tolerance and defaulting, defaultOutPath/resolveOutPath's --out
#      precedence, computeIncompletePhases' all-done/mixed/empty cases, and
#      buildGitRangeCommands' has-SHA/no-SHA branches and exact command text.
#   4. HERMETIC SEED — a temp git-backed plan repo plus a temp source repo with
#      real commits, seeded via the real target/debug/rdm binary: a
#      fully-done roadmap with two commit-bearing phases, a roadmap with one
#      incomplete phase, and a done phase with no --commit recorded. The real
#      `rdm roadmap show`/`phase show --format json` output is fed through the
#      Section-3 pure functions to prove the all-done/incomplete and
#      has-SHA/no-SHA data paths hold end to end against real rdm output, not
#      just fabricated Node arrays.
#
# Node is used only as a host to unit-test the pure module; it is stdlib-only
# (node:assert), with no package.json / node_modules / third-party packages.
# node is pinned in .mise.toml.
#
# Requires: node (via PATH or `mise exec node --`); a cargo-built
# target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/document.mjs"
WF="$REPO_ROOT/.claude/workflows/document.js"
SKILL="$REPO_ROOT/.claude/skills/rdm-document/SKILL.md"
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
[ -f "$SKILL" ] || fail "skill shim not found: $SKILL"
[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

# Resolve a node command: prefer PATH, fall back to the mise-pinned toolchain.
# Fail hard if node is genuinely unavailable (matches the sibling harnesses'
# tool-guard convention — a silent skip would turn this gate into a no-op).
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

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# ==============================================================================
say "1. Static invariants on document.js and the rdm-document skill shim"
# ==============================================================================

grep -q 'parallel(' "$WF" || fail "document.js must fan out per-phase gathering via parallel("
grep -q -- '--out' "$WF" || fail "document.js must reference --out"
grep -q 'rdm roadmap show' "$WF" || fail "document.js must fetch the roadmap via rdm roadmap show"
grep -q 'rdm phase show' "$WF" || fail "document.js must fetch each phase via rdm phase show"
grep -q -- '--format json' "$WF" || fail "document.js's rdm fetch commands must request --format json"
pass "parallel() fan-out and the rdm roadmap show / phase show --format json wiring are present"

grep -q 'incompletePhases' "$WF" || fail "document.js must carry an incompletePhases abort branch"
grep -q 'aborted: true' "$WF" || fail "document.js must return aborted: true on the incomplete-phase short-circuit"
grep -q 'computeIncompletePhases' "$WF" || fail "document.js must call computeIncompletePhases"
pass "abort-on-incomplete branch present"

grep -q 'hasSha' "$WF" || fail "document.js must carry the hasSha has-SHA/body-only decision"
grep -q 'fallback' "$WF" || fail "document.js must carry the body-only fallback flag"
grep -q 'buildGitRangeCommands' "$WF" || fail "document.js must call buildGitRangeCommands"
pass "has-SHA / body-only fallback branch present"

for marker in ">>> document-core:begin" ">>> document-core:end"; do
    grep -q "$marker" "$LIB" || fail "document-core marker missing from $LIB: $marker"
    grep -q "$marker" "$WF" || fail "document-core marker missing from $WF: $marker"
done
pass "document-core markers present in both lib and workflow"

# AC: the workflow performs no rdm status mutation of its own (it is an
# artifact producer, not a gate) and runs no confirmation/plan-mode call.
if grep -qE -- '--status[[:space:]]' "$WF"; then
    fail "document.js must not mutate any rdm --status — that is the (never-run) job of a gate, not this artifact producer"
fi
if grep -qE "(buildReviewPipeline\(|runPlanGate\(|runPlanReview\(|workflow\('plan-review'|--verdict)" "$WF"; then
    fail "document.js must not invoke a plan-mode/review-pipeline call — approval happens only in the skill shim"
fi
pass "no --status mutation and no plan-mode/confirmation call in document.js"

# AC: no Date.now(/Math.random( in either file (forbidden-primitives rule).
for f in "$WF" "$LIB"; do
    if grep -qF 'Date.now(' "$f"; then
        fail "forbidden Date.now( found in $f"
    fi
    if grep -qF 'Math.random(' "$f"; then
        fail "forbidden Math.random( found in $f"
    fi
done
pass "no Date.now( / Math.random( in document.js or lib/document.mjs"

# AC: the skill shim retains the terminal human-approval language...
if ! grep -qi 'not done until' "$SKILL" || ! grep -qi 'reviewed and approved' "$SKILL"; then
    fail "SKILL.md must retain the terminal 'not done until reviewed and approved' human-approval language"
fi
pass "SKILL.md retains the terminal human-approval language"

# ...and was actually thinned, not just re-saved: the old step-by-step
# git-gather prose (a literal step named "Cross-reference") must be gone.
if grep -q '\*\*Cross-reference\*\*' "$SKILL"; then
    fail "SKILL.md still carries the old step-by-step 'Cross-reference' git-gather prose — it must be thinned into document.js's prompts"
fi
pass "SKILL.md no longer carries the old step-by-step git-gather prose"

grep -q 'Workflow' "$SKILL" || fail "SKILL.md must invoke the document Workflow"
pass "SKILL.md is a thin Workflow-invoking shim"

# ==============================================================================
say "1b. Mechanical-tier pin: mechanical fetch/gather/write agents pinned"
# ==============================================================================

# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/lib/mechanical-tier-check.sh"

agent_option_blocks "$WF" >"$TMP/mech-blocks"
[ -s "$TMP/mech-blocks" ] || fail "AC-MECHANICAL-TIER: could not extract any agent() option blocks from document.js"

assert_label_model "$TMP/mech-blocks" 'fetch:roadmap-meta' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: fetch:roadmap-meta must resolve to model: mechanicalModel"
assert_label_model "$TMP/mech-blocks" 'gather:' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: every gather:<stem> call must resolve to model: mechanicalModel"
assert_label_model "$TMP/mech-blocks" 'write:draft' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: write:draft must resolve to model: mechanicalModel"
pass "AC-MECHANICAL-TIER: fetch:roadmap-meta, gather:<stem>, and write:draft resolve to model: mechanicalModel"

# Negative: synthesize:draft is the judgment/authoring stage and must NOT be
# pinned to the mechanical tier.
assert_label_not_model "$TMP/mech-blocks" 'synthesize:draft' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: synthesize:draft must NOT be pinned to model: mechanicalModel (judgment stage)"
pass "AC-MECHANICAL-TIER: synthesize:draft is left unpinned (judgment stage)"

# Self-test: plant a repoint away from mechanicalModel on write:draft and
# prove the check now fails; restore and prove it passes again.
sed "/label: 'write:draft'/,/^  })/ s/model: mechanicalModel,/model: 'claude-opus-4-8',/" "$WF" >"$TMP/wf.mech-mutant"
agent_option_blocks "$TMP/wf.mech-mutant" >"$TMP/mech-blocks-mutant"
if assert_label_model "$TMP/mech-blocks-mutant" 'write:draft' 'mechanicalModel'; then
    fail "AC-MECHANICAL-TIER: detector missed a write:draft repoint away from mechanicalModel"
fi
pass "AC-MECHANICAL-TIER: detector fires when write:draft is repointed away from mechanicalModel"

# ==============================================================================
say "2. Block drift: the document-core region is byte-identical (lib vs workflow)"
# ==============================================================================

extract_block() {
    awk '
        index($0, ">>> document-core:begin") { infence = 1; next }
        index($0, ">>> document-core:end") { infence = 0 }
        infence { print }
    ' "$1"
}

blocks_equal() {
    extract_block "$1" >"$TMP/_a" 2>/dev/null
    extract_block "$2" >"$TMP/_b" 2>/dev/null
    [ -s "$TMP/_a" ] && diff -q "$TMP/_a" "$TMP/_b" >/dev/null 2>&1
}

extract_block "$LIB" >"$TMP/lib-block"
[ -s "$TMP/lib-block" ] || fail "no document-core block found between markers in $LIB"
extract_block "$WF" >"$TMP/wf-block"
[ -s "$TMP/wf-block" ] || fail "no document-core block found between markers in $WF"

if diff -u "$TMP/lib-block" "$TMP/wf-block" >/dev/null 2>&1; then
    pass "document-core block matches byte-for-byte between lib and workflow"
else
    printf '\n' >&2
    diff -u "$TMP/lib-block" "$TMP/wf-block" >&2 || true
    fail "document-core block DRIFTED — copy the lib block verbatim into $WF"
fi

# Self-test: prove the byte-equality gate is not a no-op.
say "2b. Block drift detector fires on planted drift (self-test)"
cp "$LIB" "$TMP/lib.scratch"
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "scratch copies should match before mutation"
sed 's/vacuously all-done/planted drift/' "$TMP/wf.scratch" >"$TMP/wf.mut" && mv "$TMP/wf.mut" "$TMP/wf.scratch"
if blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch"; then
    fail "byte-equality gate did NOT detect a planted mutation inside the block"
fi
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "restore did not heal the byte-equality gate"
pass "drift detector fails on a planted mutation and heals on restore"

# Non-vacuity: assert the region actually carries every core symbol in BOTH
# files, so a partial hand-mirror cannot slip through by extracting nothing.
for sym in parseDocumentArgs defaultOutPath resolveOutPath computeIncompletePhases buildGitRangeCommands; do
    grep -q "$sym" "$TMP/lib-block" || fail "document-core block in the LIB is missing $sym"
    grep -q "$sym" "$TMP/wf-block" || fail "document-core block in the WORKFLOW is missing $sym (partial mirror?)"
done
pass "document-core block carries every pure function in both files"

# ==============================================================================
say "3. Behavior: pure decision logic driven in Node against fabricated inputs"
# ==============================================================================

cat >"$TMP/test.mjs" <<NODE_TEST
import assert from 'node:assert/strict';
import {
  parseDocumentArgs,
  defaultOutPath,
  resolveOutPath,
  computeIncompletePhases,
  buildGitRangeCommands,
} from '$LIB';

// --- parseDocumentArgs --------------------------------------------------
assert.deepEqual(parseDocumentArgs({ roadmap: 'rm', out: 'x.md' }), { roadmap: 'rm', out: 'x.md' });
assert.deepEqual(parseDocumentArgs({ roadmap: 'rm' }), { roadmap: 'rm', out: '' });
assert.deepEqual(parseDocumentArgs({}), { roadmap: '', out: '' });
assert.deepEqual(parseDocumentArgs(undefined), { roadmap: '', out: '' });
assert.deepEqual(parseDocumentArgs(null), { roadmap: '', out: '' });
assert.deepEqual(parseDocumentArgs('not json'), { roadmap: '', out: '' });
assert.deepEqual(
  parseDocumentArgs(JSON.stringify({ roadmap: 'stringified-rm', out: 'docs/z.md' })),
  { roadmap: 'stringified-rm', out: 'docs/z.md' }
);
assert.deepEqual(parseDocumentArgs(42), { roadmap: '', out: '' });
console.log('parseDocumentArgs: ok');

// --- defaultOutPath / resolveOutPath ------------------------------------
assert.equal(defaultOutPath('my-roadmap'), 'docs/my-roadmap.md');
assert.equal(resolveOutPath({ roadmap: 'my-roadmap', out: '' }), 'docs/my-roadmap.md');
assert.equal(resolveOutPath({ roadmap: 'my-roadmap', out: 'custom/path.md' }), 'custom/path.md');
assert.equal(resolveOutPath({ roadmap: 'my-roadmap' }), 'docs/my-roadmap.md');
console.log('defaultOutPath/resolveOutPath: ok');

// --- computeIncompletePhases ---------------------------------------------
assert.deepEqual(
  computeIncompletePhases([{ stem: 'a', status: 'done' }, { stem: 'b', status: 'done' }]),
  []
);
assert.deepEqual(
  computeIncompletePhases([{ stem: 'a', status: 'done' }, { stem: 'b', status: 'in-progress' }]),
  [{ stem: 'b', status: 'in-progress' }]
);
assert.deepEqual(computeIncompletePhases([]), []);
assert.deepEqual(computeIncompletePhases(undefined), []);
assert.deepEqual(computeIncompletePhases(null), []);
console.log('computeIncompletePhases: ok');

// --- buildGitRangeCommands -------------------------------------------------
const withSha = buildGitRangeCommands('abc123');
assert.equal(withSha.hasSha, true);
assert.equal(withSha.log, 'git log --oneline abc123~1..abc123');
assert.equal(withSha.diffStat, 'git diff --stat abc123~1..abc123');

for (const bad of ['', undefined, null, 0, 42]) {
  const r = buildGitRangeCommands(bad);
  assert.equal(r.hasSha, false, 'expected hasSha:false for ' + JSON.stringify(bad));
  assert.equal(r.log, null);
  assert.equal(r.diffStat, null);
}
console.log('buildGitRangeCommands: ok');

console.log('ALL NODE BEHAVIOR ASSERTIONS PASSED');
NODE_TEST

run_node "$TMP/test.mjs" || fail "Node behavior assertions failed"
pass "pure decision logic behaves correctly against fabricated inputs"

# ==============================================================================
say "4. Hermetic seed: real rdm binary + temp source repo, real SHAs"
# ==============================================================================

# --- 4a. Temp source repo with two real commits, so seeded --commit values
#     are genuine SHAs rather than fabricated strings. ------------------------
SRC="$TMP/source-repo"
mkdir -p "$SRC"
git -C "$SRC" init -q
git -C "$SRC" config user.email "verify@example.invalid"
git -C "$SRC" config user.name "verify-bot"
echo "one" >"$SRC/a.txt"
git -C "$SRC" add a.txt
git -C "$SRC" commit -q -m "feat: add a.txt"
SHA_ONE=$(git -C "$SRC" rev-parse HEAD)
echo "two" >"$SRC/b.txt"
git -C "$SRC" add b.txt
git -C "$SRC" commit -q -m "feat: add b.txt"
SHA_TWO=$(git -C "$SRC" rev-parse HEAD)
pass "temp source repo seeded with two real commits ($SHA_ONE, $SHA_TWO)"

# --- 4b. Temp plan repo, hermetic. -------------------------------------------
PLAN="$TMP/plan"
PROJ="doc-verify"
rdm() { "$RDM_BIN" --root "$PLAN" "$@"; }

mkdir -p "$PLAN"
rdm init --default-project "$PROJ" >/dev/null

# Scenario A: fully-done roadmap, two commit-bearing phases.
rdm roadmap create rm-done --title "Fully Done Roadmap" --body "All phases complete." \
    --no-edit --project "$PROJ" >/dev/null
rdm phase create one --title "Phase One" --number 1 --body "First phase body." \
    --no-edit --roadmap rm-done --project "$PROJ" >/dev/null
rdm phase create two --title "Phase Two" --number 2 --body "Second phase body." \
    --no-edit --roadmap rm-done --project "$PROJ" >/dev/null
rdm phase update phase-1-one --status "done" --commit "$SHA_ONE" --no-edit \
    --roadmap rm-done --project "$PROJ" >/dev/null
rdm phase update phase-2-two --status "done" --commit "$SHA_TWO" --no-edit \
    --roadmap rm-done --project "$PROJ" >/dev/null

# Scenario B: roadmap with one incomplete phase.
rdm roadmap create rm-incomplete --title "Incomplete Roadmap" --body "Still in flight." \
    --no-edit --project "$PROJ" >/dev/null
rdm phase create started --title "Started" --number 1 --body "In progress." \
    --no-edit --roadmap rm-incomplete --project "$PROJ" >/dev/null
rdm phase update phase-1-started --status in-progress --no-edit \
    --roadmap rm-incomplete --project "$PROJ" >/dev/null

# Scenario C: a done phase with NO --commit recorded.
rdm roadmap create rm-no-sha --title "No SHA Roadmap" --body "Done, but no commit recorded." \
    --no-edit --project "$PROJ" >/dev/null
rdm phase create bare --title "Bare" --number 1 --body "Done with no commit." \
    --no-edit --roadmap rm-no-sha --project "$PROJ" >/dev/null
rdm phase update phase-1-bare --status "done" --no-edit \
    --roadmap rm-no-sha --project "$PROJ" >/dev/null

rdm commit -m "seed: document harness fixtures" >/dev/null
pass "seeded fully-done, incomplete, and no-SHA fixture roadmaps"

# --- 4c. Real rdm JSON, fed through the Section-3 pure functions. -----------
rdm roadmap show rm-done --project "$PROJ" --format json --no-body >"$TMP/rm-done.json"
rdm phase show phase-1-one --roadmap rm-done --project "$PROJ" --format json >"$TMP/phase-1-one.json"
rdm phase show phase-2-two --roadmap rm-done --project "$PROJ" --format json >"$TMP/phase-2-two.json"
rdm roadmap show rm-incomplete --project "$PROJ" --format json --no-body >"$TMP/rm-incomplete.json"
rdm phase show phase-1-bare --roadmap rm-no-sha --project "$PROJ" --format json >"$TMP/phase-1-bare.json"

cat >"$TMP/test-real.mjs" <<NODE_TEST
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { computeIncompletePhases, buildGitRangeCommands, resolveOutPath, defaultOutPath } from '$LIB';

function loadJson(path) {
  // rdm may append an informational uncommitted-changes notice after the JSON
  // object; parse only the leading JSON value.
  const text = readFileSync(path, 'utf8');
  let depth = 0;
  let end = -1;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '{') depth++;
    else if (text[i] === '}') {
      depth--;
      if (depth === 0) { end = i + 1; break; }
    }
  }
  return JSON.parse(end === -1 ? text : text.slice(0, end));
}

// Scenario A: fully-done roadmap -> computeIncompletePhases([]) over real phases.
const rmDone = loadJson('$TMP/rm-done.json');
assert.equal(rmDone.phases.length, 2);
assert.deepEqual(computeIncompletePhases(rmDone.phases), []);
console.log('scenario A: real fully-done roadmap yields zero incomplete phases: ok');

// Scenario A: each phase's real commit SHA drives buildGitRangeCommands.
const phaseOne = loadJson('$TMP/phase-1-one.json');
const phaseTwo = loadJson('$TMP/phase-2-two.json');
assert.equal(phaseOne.commit, '$SHA_ONE');
assert.equal(phaseTwo.commit, '$SHA_TWO');
const rangeOne = buildGitRangeCommands(phaseOne.commit);
assert.equal(rangeOne.hasSha, true);
assert.equal(rangeOne.log, 'git log --oneline $SHA_ONE~1..$SHA_ONE');
assert.equal(rangeOne.diffStat, 'git diff --stat $SHA_ONE~1..$SHA_ONE');
const rangeTwo = buildGitRangeCommands(phaseTwo.commit);
assert.equal(rangeTwo.hasSha, true);
console.log('scenario A: real commit SHAs drive buildGitRangeCommands correctly: ok');

// Scenario A: default vs explicit --out resolution against the real slug.
assert.equal(resolveOutPath({ roadmap: rmDone.slug, out: '' }), 'docs/' + rmDone.slug + '.md');
assert.equal(resolveOutPath({ roadmap: rmDone.slug, out: 'custom/out.md' }), 'custom/out.md');
assert.equal(defaultOutPath(rmDone.slug), 'docs/rm-done.md');
console.log('scenario A: resolveOutPath default/--out precedence against real slug: ok');

// Scenario B: real incomplete-phase roadmap surfaces the non-done phase.
const rmIncomplete = loadJson('$TMP/rm-incomplete.json');
const incomplete = computeIncompletePhases(rmIncomplete.phases);
assert.equal(incomplete.length, 1);
assert.equal(incomplete[0].stem, 'phase-1-started');
assert.equal(incomplete[0].status, 'in-progress');
console.log('scenario B: real in-progress phase surfaces via computeIncompletePhases: ok');

// Scenario C: a done phase with no --commit recorded has no 'commit' field
// (rdm serializes it as skip_serializing_if Option::is_none), which
// buildGitRangeCommands must treat as no-SHA.
const phaseBare = loadJson('$TMP/phase-1-bare.json');
assert.equal(phaseBare.status, 'done');
assert.equal(phaseBare.commit, undefined);
const rangeBare = buildGitRangeCommands(phaseBare.commit);
assert.equal(rangeBare.hasSha, false);
assert.equal(rangeBare.log, null);
assert.equal(rangeBare.diffStat, null);
console.log('scenario C: a done phase with no recorded commit falls back to no-SHA: ok');

console.log('ALL HERMETIC-SEED ASSERTIONS PASSED');
NODE_TEST

run_node "$TMP/test-real.mjs" || fail "hermetic real-binary assertions failed"
pass "real rdm JSON output round-trips correctly through the pure decision functions"

# ==============================================================================
say "verify-workflow-document.sh: ALL GREEN"
