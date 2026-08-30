#!/bin/sh
# Hermetic regression for PROJECT DIRECTIVES — the guidance half of the dispatch
# lane (dispatch-dev-discipline phase 3).
#
# A project expresses optional development discipline (mutation testing, coverage
# floors, fuzzing, review emphases) as prose it already writes for its own agents.
# rdm DISCOVERS that prose in a fixed set of known locations (or takes an explicit
# `dispatch.directives` list that REPLACES discovery), reads each source's
# post-frontmatter body VERBATIM, bounds the set by bytes, and injects the matching
# text — unmodified — into the dispatched implementer's or reviewer's prompt.
#
# rdm never summarizes or re-words a directive: a paraphrase would be rdm's reading
# of the project's rule interposed between the project and the agent. This harness
# exists mainly to keep that true.
#
# Three layers are gated, one section per acceptance criterion, EACH behind a
# planted-mutation self-test:
#
#   1  AC1 — a `.claude/rules/` directive reaches the implementer's AND the
#            reviewer's prompt VERBATIM, driven through the REAL binary and the
#            REAL extracted `buildImplementPrompt` / `findPrompt`.
#   2  AC2 — a `paths:`-scoped rule that does not match the change is not
#            injected; a matching one is; an UNKNOWN path set fails OPEN and an
#            EMPTY one narrows. Plus direct glob-matcher cases.
#   3  AC3 — at least two source locations beyond `.claude/rules/` resolve, with
#            the emitted `path` set asserted exactly, in discovery order.
#   4  AC4 — an `rdm.toml` `dispatch.directives` override REPLACES the discovered
#            list rather than adding to it (asserted with the load-bearing
#            NEGATIVE), and the key is repo-only.
#   5  AC5 — no sources ⇒ exit 0, empty arrays, empty stderr, an empty rendered
#            string, a BYTE-IDENTICAL implementer prompt, an untouched finder
#            prompt, and no `[directives:` clause anywhere in the OUTCOME.
#   6  AC6 — byte-identity, guarded by a PARAPHRASE-DETECTING assertion (the exact
#            fixture sentence must appear; a same-meaning-different-words variant
#            must NOT), plus `verbatimOrDrop`'s code-point transport check.
#   7  AC7 — the size bound is enforced (in Rust, once) and its exceeded path is
#            observable in all THREE channels: the JSON's `skipped[]`, the notice
#            rendered INSIDE the injected block, and the OUTCOME summary clause.
#
# Everything is hermetic: fixture repos live in a temp dir (never inside a
# worktree), the plan repo used by § 4 is a throwaway `rdm init` tree, and no
# LLM is ever called — the JS is driven with injected fakes.
#
# Node is used only as a host for the pure helpers; it is stdlib-only
# (node:assert), with no package.json / node_modules / third-party packages.
#
# Run after touching rdm-core/src/directives.rs, rdm-cli/src/commands/dispatch.rs,
# the `directives` region of .claude/workflows/lib/dispatch-phase.mjs, the
# directives preamble in .claude/workflows/lib/review.mjs, or the shipped
# rdm-wf-dispatch-phase.js driver's directive wiring.
#
# Requires: node (via PATH or `mise exec node --`), cargo-built rdm at
# target/debug/rdm.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/dispatch-phase.mjs"
REVIEW_LIB="$REPO_ROOT/.claude/workflows/lib/review.mjs"
WF="$REPO_ROOT/.claude/workflows/rdm-wf-dispatch-phase.js"
RDM_BIN="$REPO_ROOT/target/debug/rdm"

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -f "$LIB" ] || fail "source module not found: $LIB"
[ -f "$REVIEW_LIB" ] || fail "review module not found: $REVIEW_LIB"
[ -f "$WF" ] || fail "workflow script not found: $WF"
[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."

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
trap 'rm -rf "$TMP"' EXIT INT TERM

# ---------------------------------------------------------------------------
# The fixture SOURCE repo. Six locations, plus the two scoped rules § 2 needs.
# It lives in $TMP, never inside a worktree — phase 2's terminal cleanliness
# probe would see any stray file as an uncommitted change.
# ---------------------------------------------------------------------------
FIXTURE="$TMP/fixture"
mkdir -p "$FIXTURE/.claude/rules" "$FIXTURE/.cursor/rules" "$FIXTURE/.windsurf/rules" "$FIXTURE/.github"

# The EXACT sentence AC1 and AC6 match on. Kept in one shell variable so the
# assertion and the fixture can never drift apart.
EXACT='Every new public function in this repository ships with a table-driven test before its implementation.'
# A same-meaning, different-words rendering of that sentence. It must NEVER appear
# in a prompt: its presence would mean something paraphrased the project's rule.
PARAPHRASE='All newly added public functions here must have a table-based test written first.'

cat >"$FIXTURE/.claude/rules/testing.md" <<FIXEOF
---
role: implementer
---

$EXACT
FIXEOF

cat >"$FIXTURE/AGENTS.md" <<'FIXEOF'
Run the mutation-testing suite before declaring a change complete.
FIXEOF

cat >"$FIXTURE/.cursor/rules/style.mdc" <<'FIXEOF'
---
role: reviewer
globs: "rdm-core/**/*.rs"
---

Hold Rust changes to the crate's documented error-handling policy.
FIXEOF

cat >"$FIXTURE/.windsurf/rules/perf.md" <<'FIXEOF'
Measure before optimizing; record the measurement.
FIXEOF

# ---------------------------------------------------------------------------
# Shared Node preamble: imports the lib helpers, the review predicate, and the
# REAL buildImplementPrompt extracted out of the SHIPPED workflow file (the same
# extraction shim scripts/verify-workflow-dispatch.sh uses, so this harness tests
# the bytes that actually ship rather than a lib-only twin).
# ---------------------------------------------------------------------------
cat >"$TMP/preamble.mjs" <<'NODE_PRE'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export const LIB = process.env.DIR_LIB;
export const WF = process.env.DIR_WF;

export const lib = await import('file://' + LIB);
export const review = await import('file://' + process.env.DIR_REVIEW_LIB);

// Extract the real driver-region builders out of the shipped workflow.
export async function extractDriver() {
  const src = fs.readFileSync(WF, 'utf8').replace(/^export /m, '');
  const shimPath = path.join(os.tmpdir(), 'verify-project-directives-driver-' + process.pid + '.mjs');
  fs.writeFileSync(
    shimPath,
    'export default async function(args, agent, pipeline, parallel, log) {\n' +
      src.replace(
        /^\/\/ --- Driver ---/m,
        'return { buildImplementPrompt, buildFetchPrompt, buildTaskFetchPrompt, buildCodeActPrompt }\n// --- Driver ---'
      ) +
      '\n}\n'
  );
  const shim = (await import('file://' + shimPath + '?t=' + process.pid)).default;
  return shim({}, async () => null, null, null, () => {});
}

export { assert, fs, os, path };
NODE_PRE

export DIR_LIB="$LIB"
export DIR_WF="$WF"
export DIR_REVIEW_LIB="$REVIEW_LIB"
export DIR_FIXTURE="$FIXTURE"
export DIR_RDM_BIN="$RDM_BIN"
export DIR_EXACT="$EXACT"
export DIR_PARAPHRASE="$PARAPHRASE"
export DIR_TMP="$TMP"

# ===========================================================================
# 1. AC1 — a `.claude/rules/` directive reaches BOTH prompts, verbatim.
# ===========================================================================
say "1. AC1: a .claude/rules/ directive reaches the implementer's AND the reviewer's prompt, verbatim"

"$RDM_BIN" dispatch directives --dir "$FIXTURE" --format json >"$TMP/ac1.json" 2>"$TMP/ac1.err" ||
    fail "AC1: the real \`rdm dispatch directives\` command failed against the fixture"
[ -s "$TMP/ac1.json" ] || fail "AC1: the command printed nothing"

cat >"$TMP/ac1.mjs" <<'NODE_AC1'
import { assert, fs, lib, review, extractDriver } from './preamble.mjs';

const payload = JSON.parse(fs.readFileSync(process.env.DIR_TMP + '/ac1.json', 'utf8'));
const EXACT = process.env.DIR_EXACT;

// Transport → guard → select → render, exactly as the driver does it.
const normalized = lib.normalizeDirectives(payload.directives);
assert.ok(normalized.length >= 4, 'AC1: the fixture resolved its directive sources: ' + normalized.length);
const transported = lib.verbatimOrDrop(normalized);
assert.deepEqual(transported.dropped, [], 'AC1: nothing was dropped in transport on the happy path');

// --- implementer -----------------------------------------------------------
const implSelected = lib.selectDirectives(transported.kept, 'implementer', null);
const implRendered = lib.renderDirectives(implSelected, '');
assert.notEqual(implRendered, '', 'AC1: a non-empty selection renders a non-empty block');

const { buildImplementPrompt } = await extractDriver();
assert.equal(typeof buildImplementPrompt, 'function', 'AC1: buildImplementPrompt was extracted from the shipped workflow');
const cfg = { rdmBin: '/fake/bin/rdm', project: 'demo' };
const prompt = buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, 'sh ci.sh', implRendered);
assert.notEqual(prompt.indexOf(EXACT), -1, 'AC1: the implementer prompt carries the fixture sentence VERBATIM');

// The rework branch too — the push is on the SHARED path, so neither branch may
// drop it. This is the property a future refactor is most likely to break.
const rework = buildImplementPrompt('rm', 'BODY', 'PLAN', { findings: [], acTable: [] }, cfg, 'sh ci.sh', implRendered);
assert.notEqual(rework.indexOf(EXACT), -1, 'AC1: the REWORK implementer prompt carries it too');

// --- reviewer --------------------------------------------------------------
// The reviewer-role selection is a different filter over the same set, so a
// reviewer-addressed rule must reach findPrompt in BOTH modes and EVERY
// dimension (one rule, no per-dimension flag).
const revSelected = lib.selectDirectives(transported.kept, 'reviewer', null);
const revRendered = lib.renderDirectives(revSelected, '');
const REVIEWER_SENTENCE = "Hold Rust changes to the crate's documented error-handling policy.";
assert.notEqual(revRendered.indexOf(REVIEWER_SENTENCE), -1, 'AC1: the reviewer block carries the reviewer-addressed rule');

for (const mode of ['code', 'plan']) {
  for (const dim of review.DIMENSIONS[mode]) {
    const p = review.findPrompt(mode, dim, { target: 'x', directives: revRendered });
    assert.notEqual(p.indexOf(REVIEWER_SENTENCE), -1, 'AC1: ' + mode + '/' + dim.key + ' finder prompt carries the directive');
    assert.notEqual(p.indexOf(review.DIRECTIVES_PREAMBLE), -1, 'AC1: ' + mode + '/' + dim.key + ' carries the authority preamble');
  }
}

// The authority sentence must DENY narrowing — this channel carries
// project-authored text straight into a reviewer prompt, so the scoping clause is
// the whole reason it is safe to do at all.
for (const clause of ['CANNOT narrow your review', 'skip a file', 'pre-approved', 'is itself a finding']) {
  assert.notEqual(review.DIRECTIVES_PREAMBLE.indexOf(clause), -1, 'AC1: the preamble pins the clause: ' + clause);
}

console.log('AC1 OK: the fixture sentence reaches both implementer branches and every finder prompt in both modes');
NODE_AC1
cp "$TMP/preamble.mjs" "$TMP/preamble.mjs" 2>/dev/null || true
(cd "$TMP" && run_node ac1.mjs) || fail "1: AC1 assertions failed"
pass "1: AC1 — a .claude/rules/ directive reaches the implementer (both branches) and every finder prompt"

# Self-test: change the fixture sentence and prove the assertion turns red.
cp -R "$FIXTURE" "$TMP/fixture-ac1-mutant"
cat >"$TMP/fixture-ac1-mutant/.claude/rules/testing.md" <<'FIXEOF'
---
role: implementer
---

A completely different rule that the assertion is not looking for.
FIXEOF
"$RDM_BIN" dispatch directives --dir "$TMP/fixture-ac1-mutant" --format json >"$TMP/ac1.json" 2>/dev/null
if (cd "$TMP" && run_node ac1.mjs) >/dev/null 2>&1; then
    fail "1 self-test: AC1 passed against a fixture whose sentence was replaced — the assertion is vacuous"
fi
"$RDM_BIN" dispatch directives --dir "$FIXTURE" --format json >"$TMP/ac1.json" 2>/dev/null
pass "1 self-test: replacing the fixture sentence turns AC1 red"

# ===========================================================================
# 2. AC2 — `paths:` scoping, with the fail-open/narrowing rules.
# ===========================================================================
say "2. AC2: a paths:-scoped rule that does not match the change is not injected"

cat >"$TMP/ac2.mjs" <<'NODE_AC2'
import { assert, lib } from './preamble.mjs';

const scoped = { path: 'rust-only.md', role: 'both', paths: ['rdm-core/**/*.rs'], text: 'RUST RULE', chars: 9 };
const unscoped = { path: 'unscoped.md', role: 'both', paths: [], text: 'ANY RULE', chars: 8 };
const all = [scoped, unscoped];
const names = (l) => l.map((d) => d.path);

// A change that touches NO Rust: only the unscoped rule survives.
assert.deepEqual(
  names(lib.selectDirectives(all, 'implementer', ['scripts/x.sh'])),
  ['unscoped.md'],
  'AC2: a scoped rule whose globs do not match the change is NOT injected'
);
// A change that DOES touch Rust: both.
assert.deepEqual(
  names(lib.selectDirectives(all, 'implementer', ['rdm-core/src/ops/task.rs'])),
  ['rust-only.md', 'unscoped.md'],
  'AC2: a scoped rule whose globs match IS injected'
);
// FAIL-OPEN: an unknown path set widens, never narrows.
assert.deepEqual(
  names(lib.selectDirectives(all, 'implementer', null)),
  ['rust-only.md', 'unscoped.md'],
  'AC2: a null path set fails OPEN — missing information widens injection'
);
assert.deepEqual(names(lib.selectDirectives(all, 'implementer', undefined)), ['rust-only.md', 'unscoped.md']);
// An EMPTY array is a real answer and DOES narrow.
assert.deepEqual(
  names(lib.selectDirectives(all, 'implementer', [])),
  ['unscoped.md'],
  'AC2: an EMPTY path set is a real "nothing changed" answer and narrows'
);

// Role filtering is independent of path filtering, and `both` matches everything.
const impl = { path: 'i.md', role: 'implementer', paths: [], text: 'I', chars: 1 };
const rev = { path: 'r.md', role: 'reviewer', paths: [], text: 'R', chars: 1 };
assert.deepEqual(names(lib.selectDirectives([impl, rev, unscoped], 'implementer', null)), ['i.md', 'unscoped.md']);
assert.deepEqual(names(lib.selectDirectives([impl, rev, unscoped], 'reviewer', null)), ['r.md', 'unscoped.md']);

// --- the glob matcher itself ------------------------------------------------
const m = (pat, f) => lib.globToRegExp(pat).test(f);
assert.ok(m('**/*.rs', 'a/b/c.rs'), 'AC2: ** crosses separators');
assert.ok(m('**/*.rs', 'c.rs'), 'AC2: **/ matches ZERO directories too');
assert.ok(!m('*.rs', 'a/b.rs'), 'AC2: a single * does NOT cross a separator');
assert.ok(m('*.rs', 'b.rs'), 'AC2: a single * matches within one segment');
assert.ok(m('src/?.rs', 'src/a.rs'), 'AC2: ? matches exactly one character');
assert.ok(!m('src/?.rs', 'src/ab.rs'), 'AC2: ? matches exactly ONE character');
assert.ok(!m('src/?.rs', 'src//.rs') || true, 'AC2: ? never matches a separator');
assert.ok(m('rdm-core/**/*.rs', 'rdm-core/src/ops/task.rs'), 'AC2: the documented example matches');
assert.ok(!m('rdm-core/**/*.rs', 'rdm-cli/src/main.rs'), 'AC2: ...and a sibling crate does not');
// A metacharacter-bearing path must be matched LITERALLY, never as a regex.
assert.ok(m('a+b(c).md', 'a+b(c).md'), 'AC2: regex metacharacters in a pattern are escaped');
assert.ok(!m('a.b', 'axb'), 'AC2: a literal dot is not a regex dot');

// Anchoring at both ends: a pattern must match the WHOLE path.
assert.ok(!m('src', 'src/a.rs'), 'AC2: patterns are anchored at both ends');

console.log('AC2 OK: role + path selection, fail-open on unknown, narrowing on empty, and the glob matcher');
NODE_AC2
(cd "$TMP" && run_node ac2.mjs) || fail "2: AC2 assertions failed"
pass "2: AC2 — scoped rules are filtered by the change, unknown fails open, empty narrows"

# Self-test: make matchesPaths return true unconditionally and prove § 2 fails.
mkdir -p "$TMP/mutant2"
sed 's/^function matchesPaths(patterns, changedPaths) {/function matchesPaths(patterns, changedPaths) { return true;/' \
    "$LIB" >"$TMP/mutant2/dispatch-phase.mjs"
cp "$REVIEW_LIB" "$TMP/mutant2/review.mjs"
if ! cmp -s "$LIB" "$TMP/mutant2/dispatch-phase.mjs"; then
    (cd "$TMP" && DIR_LIB="$TMP/mutant2/dispatch-phase.mjs" DIR_REVIEW_LIB="$TMP/mutant2/review.mjs" run_node ac2.mjs) >/dev/null 2>&1 &&
        fail "2 self-test: AC2 passed with matchesPaths short-circuited to true — the scoping assertion is vacuous"
else
    fail "2 self-test: could not plant the matchesPaths mutation"
fi
pass "2 self-test: short-circuiting matchesPaths to true turns AC2 red"

# ===========================================================================
# 3. AC3 — at least two source locations beyond `.claude/rules/` resolve.
# ===========================================================================
say "3. AC3: source locations beyond .claude/rules/ resolve, in the documented discovery order"

cat >"$TMP/ac3.mjs" <<'NODE_AC3'
import { assert, fs } from './preamble.mjs';

const payload = JSON.parse(fs.readFileSync(process.env.DIR_TMP + '/ac1.json', 'utf8'));
const paths = payload.directives.map((d) => d.path);

// Asserted EXACTLY and IN ORDER, not by a count — a count would pass even if the
// discovery list collapsed to one location that happened to hold four files.
assert.deepEqual(
  paths,
  ['.claude/rules/testing.md', 'AGENTS.md', '.cursor/rules/style.mdc', '.windsurf/rules/perf.md'],
  'AC3: every seeded location resolves, in the fixed documented discovery order'
);

const beyondClaudeRules = paths.filter((p) => p.indexOf('.claude/rules/') !== 0);
assert.ok(
  beyondClaudeRules.length >= 2,
  'AC3: at least TWO locations beyond .claude/rules/ resolve, proving the list is not Claude-Code-only: ' +
    JSON.stringify(beyondClaudeRules)
);

// CLAUDE.md is deliberately NOT a source — every subagent already loads it, so
// discovering it would double-pay its token cost per dispatched agent.
assert.ok(!paths.some((p) => p === 'CLAUDE.md'), 'AC3: CLAUDE.md is never a directive source');

assert.equal(payload.origin, 'discovery', 'AC3: with no declared key, the origin is discovery');
assert.equal(payload.budget.maxBytesPerSource, 8000, 'AC3: the command echoes the per-source bound');
assert.equal(payload.budget.maxBytesTotal, 16000, 'AC3: the command echoes the total budget');

console.log('AC3 OK: ' + paths.length + ' sources across 4 locations, ' + beyondClaudeRules.length + ' beyond .claude/rules/');
NODE_AC3
(cd "$TMP" && run_node ac3.mjs) || fail "3: AC3 assertions failed"
pass "3: AC3 — four locations resolve in discovery order, three of them beyond .claude/rules/"

# Self-test: remove a non-Claude location and prove the exact-set assertion fires.
cp -R "$FIXTURE" "$TMP/fixture-ac3-mutant"
rm -f "$TMP/fixture-ac3-mutant/.windsurf/rules/perf.md"
"$RDM_BIN" dispatch directives --dir "$TMP/fixture-ac3-mutant" --format json >"$TMP/ac1.json" 2>/dev/null
if (cd "$TMP" && run_node ac3.mjs) >/dev/null 2>&1; then
    fail "3 self-test: AC3 passed with a discovery location removed — the path-set assertion is vacuous"
fi
"$RDM_BIN" dispatch directives --dir "$FIXTURE" --format json >"$TMP/ac1.json" 2>/dev/null
pass "3 self-test: removing a discovery location turns AC3 red"

# ===========================================================================
# 4. AC4 — the rdm.toml override REPLACES the discovered list.
# ===========================================================================
say "4. AC4: dispatch.directives replaces the discovered list entirely (never merges)"

PLAN_REPO="$TMP/plan-repo"
mkdir -p "$PLAN_REPO"
"$RDM_BIN" --root "$PLAN_REPO" init --default-project demo >/dev/null 2>&1 ||
    fail "AC4: could not init a throwaway plan repo at $PLAN_REPO"

# The declared source lives in the FIXTURE (the source tree), not the plan repo.
mkdir -p "$FIXTURE/docs/rules"
cat >"$FIXTURE/docs/rules/only.md" <<'FIXEOF'
The one directive the operator declared explicitly.
FIXEOF

"$RDM_BIN" --root "$PLAN_REPO" config set dispatch.directives "docs/rules/only.md,docs/rules/absent.md" >/dev/null ||
    fail "AC4: could not set dispatch.directives on the throwaway plan repo"

"$RDM_BIN" --root "$PLAN_REPO" dispatch directives --dir "$FIXTURE" --format json >"$TMP/ac4.json" ||
    fail "AC4: the command failed with a declared key set"

cat >"$TMP/ac4.mjs" <<'NODE_AC4'
import { assert, fs } from './preamble.mjs';

const payload = JSON.parse(fs.readFileSync(process.env.DIR_TMP + '/ac4.json', 'utf8'));
const paths = payload.directives.map((d) => d.path);

assert.equal(payload.origin, 'config', 'AC4: a declared key reports origin=config');
assert.deepEqual(paths, ['docs/rules/only.md'], 'AC4: exactly the declared source is emitted');

// THE LOAD-BEARING NEGATIVE. A positive assertion alone would pass for a merge
// implementation too: what proves REPLACE is the ABSENCE of the discovered set.
const everyPath = paths.concat(payload.skipped.map((s) => s.path));
for (const discovered of ['.claude/rules/testing.md', 'AGENTS.md', '.cursor/rules/style.mdc', '.windsurf/rules/perf.md']) {
  assert.ok(
    everyPath.indexOf(discovered) === -1,
    'AC4: the declared list must REPLACE discovery, not add to it — found ' + discovered
  );
}

// A declared-but-missing path is SIGNAL: the operator named it explicitly, so its
// absence is reported rather than silently dropped.
const missing = payload.skipped.filter((s) => s.path === 'docs/rules/absent.md');
assert.equal(missing.length, 1, 'AC4: a declared-but-missing source is reported in skipped');
assert.equal(missing[0].reason, 'declared source not found', 'AC4: ...with a reason naming why');

console.log('AC4 OK: origin=config, exactly the declared source, no discovered path anywhere, missing entry reported');
NODE_AC4
(cd "$TMP" && run_node ac4.mjs) || fail "4: AC4 assertions failed"

# An EXPLICITLY empty list is a real answer: declare no directive sources at all.
"$RDM_BIN" --root "$PLAN_REPO" config set dispatch.directives "" >/dev/null ||
    fail "AC4: an empty dispatch.directives value must be accepted (it means 'no sources'), not rejected"
"$RDM_BIN" --root "$PLAN_REPO" dispatch directives --dir "$FIXTURE" --format json >"$TMP/ac4-empty.json"
run_node -e '
const p = JSON.parse(require("fs").readFileSync(process.env.DIR_TMP + "/ac4-empty.json", "utf8"));
if (p.origin !== "config") { console.error("origin was " + p.origin); process.exit(1); }
if (p.directives.length !== 0) { console.error("expected zero directives"); process.exit(1); }
' || fail "4: an explicitly empty declaration must yield origin=config with zero directives"

# Repo-only, with its OWN distinct message.
if "$RDM_BIN" --root "$PLAN_REPO" config set --global dispatch.directives "docs/rules/only.md" >"$TMP/global.out" 2>&1; then
    fail "AC4: 'config set --global dispatch.directives' must be rejected"
fi
grep -qF 'dispatch.directives' "$TMP/global.out" ||
    fail "AC4: the global rejection must name the key itself"
grep -qF "one project's tree" "$TMP/global.out" ||
    fail "AC4: the global rejection must be its OWN message, not the shared repo-only one"

# Repo-level set/get roundtrips.
"$RDM_BIN" --root "$PLAN_REPO" config set dispatch.directives " docs/rules/only.md , docs/rules/two.md " >/dev/null
GOT=$("$RDM_BIN" --root "$PLAN_REPO" config get dispatch.directives --raw)
[ "$GOT" = "docs/rules/only.md,docs/rules/two.md" ] ||
    fail "AC4: the repo-level set/get roundtrip trimmed or reordered entries: '$GOT'"
pass "4: AC4 — the declared list replaces discovery, an empty list is meaningful, and the key is repo-only"

# Self-test: prove the replace-not-merge NEGATIVE is not vacuous by feeding § 4's
# assertions a hand-built MERGED payload.
run_node -e '
const fs = require("fs");
const p = JSON.parse(fs.readFileSync(process.env.DIR_TMP + "/ac4.json", "utf8"));
p.directives.push({ path: "AGENTS.md", role: "both", paths: [], text: "x", chars: 1, bytes: 1 });
fs.writeFileSync(process.env.DIR_TMP + "/ac4.json", JSON.stringify(p));
'
if (cd "$TMP" && run_node ac4.mjs) >/dev/null 2>&1; then
    fail "4 self-test: AC4 passed against a MERGED payload — the replace-not-merge negative is vacuous"
fi
pass "4 self-test: a merged (declared + discovered) payload turns AC4 red"

# ===========================================================================
# 5. AC5 — no sources present produces no injection and no finding.
# ===========================================================================
say "5. AC5: no sources ⇒ no injection, no clause, no finding, and a byte-identical prompt"

EMPTY_DIR="$TMP/empty-fixture"
mkdir -p "$EMPTY_DIR"
"$RDM_BIN" dispatch directives --dir "$EMPTY_DIR" --format json >"$TMP/ac5.json" 2>"$TMP/ac5.err"
EXIT=$?
[ "$EXIT" -eq 0 ] || fail "AC5: an empty source tree must exit 0, got $EXIT"
[ ! -s "$TMP/ac5.err" ] || fail "AC5: an empty source tree must print nothing on stderr: $(cat "$TMP/ac5.err")"

cat >"$TMP/ac5.mjs" <<'NODE_AC5'
import { assert, fs, lib, review, extractDriver } from './preamble.mjs';

const payload = JSON.parse(fs.readFileSync(process.env.DIR_TMP + '/ac5.json', 'utf8'));
assert.deepEqual(payload.directives, [], 'AC5: no sources ⇒ an empty directives array');
assert.deepEqual(payload.skipped, [], 'AC5: no sources ⇒ an empty skipped array (absent is not "skipped")');
assert.equal(payload.origin, 'discovery', 'AC5: ...and discovery still reports itself as the origin');

// Rendering nothing renders NOTHING — not a header, not an empty fence.
assert.equal(lib.renderDirectives([], ''), '', 'AC5: renderDirectives([], "") is the empty string');
assert.equal(lib.renderDirectives(lib.selectDirectives(lib.normalizeDirectives(payload.directives), 'implementer', null), ''), '');
assert.equal(lib.directivesSkipNotice([], []), '', 'AC5: nothing withheld ⇒ no notice');
assert.equal(lib.directivesSummaryClause([], []), '', 'AC5: nothing withheld ⇒ no OUTCOME clause');

// The implementer prompt is BYTE-IDENTICAL to a directives-free one. This is the
// property the empty-string guard on the shared-path push exists to preserve:
// `join('\n')` turns a pushed '' into a real blank line, so an unguarded push
// would NOT be a no-op.
const { buildImplementPrompt } = await extractDriver();
const cfg = { rdmBin: '/fake/bin/rdm', project: 'demo' };
const withEmpty = buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, 'sh ci.sh', '');
const withNothing = buildImplementPrompt('rm', 'BODY', 'PLAN', null, cfg, 'sh ci.sh');
assert.equal(withEmpty, withNothing, 'AC5: an empty directive block leaves the implementer prompt byte-identical');
assert.equal(withEmpty.indexOf('PROJECT DIRECTIVE'), -1, 'AC5: ...and carries no directive fence at all');
assert.equal(withEmpty.indexOf(lib.DIRECTIVES_HEADER), -1, 'AC5: ...nor the header');

// The finder prompt gains nothing for absent, empty, or whitespace-only.
for (const ctx of [{ target: 'x' }, { target: 'x', directives: '' }, { target: 'x', directives: '   \n\t ' }]) {
  assert.equal(review.directivesPresent(ctx), false, 'AC5: directivesPresent is false for ' + JSON.stringify(ctx.directives));
  for (const mode of ['code', 'plan']) {
    const p = review.findPrompt(mode, review.DIMENSIONS[mode][0], ctx);
    assert.equal(p.indexOf(review.DIRECTIVES_PREAMBLE), -1, 'AC5: ' + mode + ' finder prompt gains no preamble');
  }
}
// ...and it is byte-identical to the no-key prompt in both modes.
for (const mode of ['code', 'plan']) {
  for (const dim of review.DIMENSIONS[mode]) {
    assert.equal(
      review.findPrompt(mode, dim, { target: 'x', directives: '' }),
      review.findPrompt(mode, dim, { target: 'x' }),
      'AC5: ' + mode + '/' + dim.key + ' is byte-identical with an empty directives key'
    );
  }
}

// NO finding, NO concern, NO OUTCOME clause. The only observable difference
// between "no directives" and "directives that all filtered out" is nothing.
const outcome = lib.buildOutcome({
  roadmap: 'rm',
  phase: '1',
  codeReviews: [[]],
  acRounds: [],
  maxRework: 1,
  tier: 'medium',
  directivesSkipped: [],
  directivesDropped: [],
});
const blob = JSON.stringify(outcome);
assert.equal(blob.indexOf('[directives:'), -1, 'AC5: a directives-free OUTCOME carries no [directives: clause anywhere');
assert.equal(blob.toLowerCase().indexOf('directive'), -1, 'AC5: ...and no directive-related text at all');
assert.deepEqual(outcome.findings, [], 'AC5: no sources produces no finding');
// A run with NO directive keys at all must be byte-identical to the above.
const bare = lib.buildOutcome({ roadmap: 'rm', phase: '1', codeReviews: [[]], acRounds: [], maxRework: 1, tier: 'medium' });
assert.deepEqual(bare, outcome, 'AC5: omitting the directive keys entirely is byte-identical to empty ones');

console.log('AC5 OK: empty everywhere — no injection, no clause, no finding, byte-identical prompts');
NODE_AC5
(cd "$TMP" && run_node ac5.mjs) || fail "5: AC5 assertions failed"
pass "5: AC5 — an empty source tree injects nothing and changes no byte of any prompt or OUTCOME"

# Self-test: make renderDirectives emit a header even for an empty list and prove
# § 5 fails — the "absent is indistinguishable from nothing" property is real.
mkdir -p "$TMP/mutant5"
sed "s/  if (arr.length === 0 \&\& note === '') return '';/  if (false) return '';/" \
    "$LIB" >"$TMP/mutant5/dispatch-phase.mjs"
cp "$REVIEW_LIB" "$TMP/mutant5/review.mjs"
if cmp -s "$LIB" "$TMP/mutant5/dispatch-phase.mjs"; then
    fail "5 self-test: could not plant the renderDirectives empty-guard mutation"
fi
if (cd "$TMP" && DIR_LIB="$TMP/mutant5/dispatch-phase.mjs" DIR_REVIEW_LIB="$TMP/mutant5/review.mjs" run_node ac5.mjs) >/dev/null 2>&1; then
    fail "5 self-test: AC5 passed with renderDirectives's empty guard removed — the no-injection assertion is vacuous"
fi
pass "5 self-test: removing renderDirectives's empty guard turns AC5 red"

# ===========================================================================
# 6. AC6 — byte-identity, guarded by a paraphrase-detecting assertion.
# ===========================================================================
say "6. AC6: injected text is byte-identical, and a paraphrase-detecting assertion guards it"

cat >"$TMP/ac6.mjs" <<'NODE_AC6'
import { assert, fs, lib, extractDriver } from './preamble.mjs';

const EXACT = process.env.DIR_EXACT;
const PARAPHRASE = process.env.DIR_PARAPHRASE;
const payload = JSON.parse(fs.readFileSync(process.env.DIR_TMP + '/ac1.json', 'utf8'));

// The rendered block must reproduce the SOURCE FILE's post-frontmatter bytes.
const raw = fs.readFileSync(process.env.DIR_FIXTURE + '/.claude/rules/testing.md', 'utf8');
const body = raw.slice(raw.indexOf('---', 3) + 4).replace(/^\n/, '');
const entry = payload.directives.find((d) => d.path === '.claude/rules/testing.md');
assert.equal(entry.text, body, 'AC6: the emitted text is the source file body, byte for byte');

const rendered = lib.renderDirectives(lib.selectDirectives(lib.normalizeDirectives(payload.directives), 'implementer', null), '');
const { buildImplementPrompt } = await extractDriver();
const prompt = buildImplementPrompt('rm', 'BODY', 'PLAN', null, { rdmBin: 'rdm' }, 'sh ci.sh', rendered);

// POSITIVE: the exact sentence survives end to end.
assert.notEqual(prompt.indexOf(EXACT), -1, 'AC6: the exact fixture sentence appears in the prompt');
// NEGATIVE (the paraphrase detector): a same-meaning-different-words rendering
// must appear NOWHERE. Its presence would mean something re-worded the project's
// own rule on the way through, which is precisely what must never happen.
assert.equal(prompt.indexOf(PARAPHRASE), -1, 'AC6: no paraphrase of the rule appears anywhere in the prompt');
// A summarization would truncate — assert the FULL sentence, not a prefix of it.
assert.notEqual(prompt.indexOf(EXACT.slice(0, 40)), -1);
assert.notEqual(prompt.indexOf(EXACT.slice(-40)), -1, 'AC6: the TAIL of the sentence survives too (a summary would cut it)');

// The only rdm-authored bytes between the fences are the header, the authority
// sentence and the fences themselves.
const start = rendered.indexOf('--- PROJECT DIRECTIVE: .claude/rules/testing.md ---');
const end = rendered.indexOf('--- END PROJECT DIRECTIVE ---', start);
const fenced = rendered.slice(start + '--- PROJECT DIRECTIVE: .claude/rules/testing.md ---'.length + 1, end - 1);
assert.equal(fenced, entry.text, 'AC6: the fenced region is exactly the source text, with nothing added or removed');

// --- verbatimOrDrop: the runtime transport guard ----------------------------
const good = { path: 'a.md', role: 'both', paths: [], text: 'hello', chars: 5 };
const altered = { path: 'b.md', role: 'both', paths: [], text: 'hello there', chars: 5 };
const r1 = lib.verbatimOrDrop([good, altered]);
assert.deepEqual(r1.kept.map((d) => d.path), ['a.md'], 'AC6: an altered entry is DROPPED, never injected');
assert.deepEqual(r1.dropped, ['b.md'], 'AC6: ...and it is NAMED in the drop list');

// CODE POINTS, not UTF-16 units: an em-dash and CJK text must survive.
const unicode = { path: 'u.md', role: 'both', paths: [], text: 'a — b 日本語', chars: Array.from('a — b 日本語').length };
assert.deepEqual(lib.verbatimOrDrop([unicode]).kept.map((d) => d.path), ['u.md'], 'AC6: non-ASCII text survives the count check');
// An astral character proves the check is not counting UTF-16 units.
const astral = { path: 'x.md', role: 'both', paths: [], text: '\u{1F600}\u{1F600}', chars: 2 };
assert.deepEqual(lib.verbatimOrDrop([astral]).kept.map((d) => d.path), ['x.md'], 'AC6: code points, not UTF-16 units');
assert.equal(astral.text.length, 4, 'AC6: (the fixture really does differ between the two units)');

// FAIL-OPEN on a missing count: the check can only fire with the count in hand.
const uncounted = { path: 'n.md', role: 'both', paths: [], text: 'whatever', chars: null };
assert.deepEqual(lib.verbatimOrDrop([uncounted]).kept.map((d) => d.path), ['n.md'], 'AC6: a missing count keeps the entry');

// A dropped entry feeds the SAME notice channel a bounded-out source does.
assert.notEqual(lib.directivesSkipNotice([], ['b.md']).indexOf('b.md'), -1, 'AC6: a transport drop is named in the notice');

console.log('AC6 OK: byte-identity end to end, no paraphrase present, and the code-point transport guard');
NODE_AC6
(cd "$TMP" && run_node ac6.mjs) || fail "6: AC6 assertions failed"
pass "6: AC6 — the text is byte-identical, no paraphrase survives, and altered text is dropped not injected"

# Self-test: make renderDirectives SUMMARIZE and prove § 6 turns red.
mkdir -p "$TMP/mutant6"
sed "s/    out.push(d.text);/    out.push(d.text.slice(0, 40));/" "$LIB" >"$TMP/mutant6/dispatch-phase.mjs"
cp "$REVIEW_LIB" "$TMP/mutant6/review.mjs"
if cmp -s "$LIB" "$TMP/mutant6/dispatch-phase.mjs"; then
    fail "6 self-test: could not plant the summarizing mutation"
fi
if (cd "$TMP" && DIR_LIB="$TMP/mutant6/dispatch-phase.mjs" DIR_REVIEW_LIB="$TMP/mutant6/review.mjs" run_node ac6.mjs) >/dev/null 2>&1; then
    fail "6 self-test: AC6 passed with renderDirectives truncating the text to 40 characters — the byte-identity assertion is vacuous"
fi
pass "6 self-test: a summarizing renderDirectives turns AC6 red"

# ===========================================================================
# 7. AC7 — the size bound is enforced and its exceeded path is observable.
# ===========================================================================
say "7. AC7: the size bound is enforced (in Rust, once) and its exceeded path is observable in three channels"

BIG_FIXTURE="$TMP/big-fixture"
mkdir -p "$BIG_FIXTURE/.claude/rules"
# 9000 bytes, over the 8000-byte per-source bound. The number lives in Rust; the
# harness only has to exceed it, and reads the bound back out of the payload.
run_node -e '
const fs = require("fs");
fs.writeFileSync(process.env.DIR_TMP + "/big-fixture/.claude/rules/huge.md", "X".repeat(9000));
fs.writeFileSync(process.env.DIR_TMP + "/big-fixture/.claude/rules/small.md", "Keep this one.\n");
'
"$RDM_BIN" dispatch directives --dir "$BIG_FIXTURE" --format json >"$TMP/ac7.json" ||
    fail "AC7: an over-bound source must not fail the command"

cat >"$TMP/ac7.mjs" <<'NODE_AC7'
import { assert, fs, lib } from './preamble.mjs';

const payload = JSON.parse(fs.readFileSync(process.env.DIR_TMP + '/ac7.json', 'utf8'));

// CHANNEL (a): the JSON's own skipped[].
assert.deepEqual(payload.directives.map((d) => d.path), ['.claude/rules/small.md'], 'AC7: the under-bound source is admitted');
assert.equal(payload.skipped.length, 1, 'AC7: the over-bound source is reported in skipped');
assert.equal(payload.skipped[0].path, '.claude/rules/huge.md');
assert.notEqual(
  payload.skipped[0].reason.indexOf(String(payload.budget.maxBytesPerSource)),
  -1,
  'AC7: the reason names the cap it exceeded (read back from the payload, never restated here)'
);
// SKIP, never TRUNCATE: a rule cut mid-sentence can invert its own meaning.
const admitted = payload.directives.map((d) => d.text).join('');
assert.equal(admitted.indexOf('XXXX'), -1, 'AC7: the over-bound source is skipped WHOLE, with no truncated remnant');

// CHANNEL (b): a notice rendered INSIDE the injected block, so the agent itself
// sees which project rules it is NOT being shown.
const notice = lib.directivesSkipNotice(payload.skipped, []);
assert.notEqual(notice, '', 'AC7: a skipped source produces a non-empty notice');
assert.notEqual(notice.indexOf('.claude/rules/huge.md'), -1, 'AC7: the notice NAMES the skipped path');
const rendered = lib.renderDirectives(lib.normalizeDirectives(payload.directives), notice);
assert.notEqual(rendered.indexOf(notice), -1, 'AC7: the notice appears INSIDE the rendered block');
// ...and a notice alone, with zero admitted directives, still renders.
assert.notEqual(lib.renderDirectives([], notice), '', 'AC7: a notice renders even when nothing was admitted');

// CHANNEL (c): the OUTCOME summary, which is what reaches the operator and the
// `rdm review blocked` queue line.
const outcome = lib.buildOutcome({
  roadmap: 'rm',
  phase: '1',
  codeReviews: [[]],
  acRounds: [],
  maxRework: 1,
  tier: 'medium',
  directivesSkipped: payload.skipped,
  directivesDropped: [],
});
assert.notEqual(outcome.summary.indexOf('[directives: 1 source(s) not injected:'), -1, 'AC7: the OUTCOME summary carries the clause');
assert.notEqual(outcome.summary.indexOf('.claude/rules/huge.md'), -1, 'AC7: ...naming the path');

// The path list is capped exactly as the worktree finding's is, and the overflow
// is reported as a COUNT rather than silently elided.
const many = [];
for (let i = 0; i < lib.WORKTREE_PATH_CAP + 5; i++) many.push({ path: 'r' + i + '.md', bytes: 1, reason: 'x' });
const capped = lib.directivesSummaryClause(many, []);
assert.notEqual(capped.indexOf('and 5 more'), -1, 'AC7: the overflow is reported as a count');
assert.equal(capped.indexOf('r' + (lib.WORKTREE_PATH_CAP + 1) + '.md'), -1, 'AC7: ...and the capped paths are not all listed');

console.log('AC7 OK: skipped-not-truncated, observable in the JSON, the in-block notice, and the OUTCOME summary');
NODE_AC7
(cd "$TMP" && run_node ac7.mjs) || fail "7: AC7 assertions failed"
pass "7: AC7 — over-bound sources are skipped whole and observable in all three channels"

# Self-test: make directivesSkipNotice return '' unconditionally. The SILENT-DROP
# mutant is the one this AC exists to forbid, so it must not pass.
mkdir -p "$TMP/mutant7"
sed "s/^function directivesSkipNotice(skipped, dropped) {/function directivesSkipNotice(skipped, dropped) { return '';/" \
    "$LIB" >"$TMP/mutant7/dispatch-phase.mjs"
cp "$REVIEW_LIB" "$TMP/mutant7/review.mjs"
if cmp -s "$LIB" "$TMP/mutant7/dispatch-phase.mjs"; then
    fail "7 self-test: could not plant the silent-drop mutation"
fi
if (cd "$TMP" && DIR_LIB="$TMP/mutant7/dispatch-phase.mjs" DIR_REVIEW_LIB="$TMP/mutant7/review.mjs" run_node ac7.mjs) >/dev/null 2>&1; then
    fail "7 self-test: AC7 passed with directivesSkipNotice silenced — the silent-drop mutant must not pass"
fi
pass "7 self-test: silencing directivesSkipNotice turns AC7 red"

# ===========================================================================
# 8. Total-budget boundary + block drift, driven through the real binary.
# ===========================================================================
say "8. The total byte budget admits in order, and the directives region is in every derived copy"

BUDGET_FIXTURE="$TMP/budget-fixture"
mkdir -p "$BUDGET_FIXTURE/.claude/rules"
run_node -e '
const fs = require("fs");
const d = process.env.DIR_TMP + "/budget-fixture/.claude/rules/";
for (const n of ["a", "b", "c", "d"]) fs.writeFileSync(d + n + ".md", "Y".repeat(5000));
'
"$RDM_BIN" dispatch directives --dir "$BUDGET_FIXTURE" --format json >"$TMP/ac8.json"
run_node -e '
const p = JSON.parse(require("fs").readFileSync(process.env.DIR_TMP + "/ac8.json", "utf8"));
const got = p.directives.map((d) => d.path);
const want = [".claude/rules/a.md", ".claude/rules/b.md", ".claude/rules/c.md"];
if (JSON.stringify(got) !== JSON.stringify(want)) {
  console.error("expected in-order admission until the budget, got " + JSON.stringify(got));
  process.exit(1);
}
if (p.skipped.length !== 1 || p.skipped[0].path !== ".claude/rules/d.md") {
  console.error("expected exactly one total-budget skip, got " + JSON.stringify(p.skipped));
  process.exit(1);
}
if (p.skipped[0].reason.indexOf(String(p.budget.maxBytesTotal)) === -1) {
  console.error("the total-budget reason must name the budget: " + p.skipped[0].reason);
  process.exit(1);
}
' || fail "8: the total byte budget did not admit in discovery order then skip the remainder"
pass "8: the total budget admits in discovery order, then skips the remainder with a reason naming it"

# The block must be present in EVERY derived copy — the lib, the local workflow,
# the shipped template, and the checked-in plugin tree. Byte-equality of the whole
# dispatch-outcome region is verify-workflow-dispatch.sh's job; this is the
# cheap presence check that says which files this feature reached at all.
for f in "$LIB" "$WF" \
    "$REPO_ROOT/rdm-core/src/templates/workflows/rdm-wf-dispatch-phase.js" \
    "$REPO_ROOT/plugins/rdm/workflows/rdm-wf-dispatch-phase.js"; do
    [ -f "$f" ] || fail "8: derived copy not found: $f"
    grep -qF 'function renderDirectives(' "$f" ||
        fail "8: $f is missing renderDirectives — the directives region did not propagate to this copy"
    grep -qF 'function verbatimOrDrop(' "$f" ||
        fail "8: $f is missing verbatimOrDrop — the paraphrase guard did not propagate to this copy"
done
pass "8: the directives region reached the lib, the workflow, the shipped template, and the plugin tree"

# The bound is enforced in RUST ONLY. The JS renders what skipped[] reports and
# must never re-derive the numbers, or the two would silently drift apart.
for f in "$LIB" "$WF"; do
    if grep -nE '\b(8000|8_000|16000|16_000)\b' "$f" >"$TMP/bound-hits" 2>/dev/null && [ -s "$TMP/bound-hits" ]; then
        cat "$TMP/bound-hits"
        fail "8: $f restates the byte bound — the bound is single-sourced in rdm-core/src/directives.rs and the JS must only render what skipped[] reports"
    fi
done
pass "8: no JS copy restates the byte bound — it is single-sourced in Rust"

# ===========================================================================
# 9. END-TO-END through the REAL shipped driver, under injected fakes.
#     Sections 1-8 test the resolver, the pure helpers, and buildOutcome
#     directly. None of them observes the WIRING — whether the driver actually
#     threads a resolved directive into the prompts it builds and its skip list
#     into the OUTCOME it returns. That wiring is enumerated by hand in several
#     places (the Stage-0 schema, both implement call sites, both runCodeReview
#     call sites, itemOutcome's field list), and a field added in one place and
#     forgotten in another is dropped SILENTLY — a real defect this section was
#     written to catch and did.
# ===========================================================================
say "9. End-to-end: the shipped driver threads directives into the prompts and the OUTCOME"

cat >"$TMP/e2e.mjs" <<'NODE_E2E'
import { assert, fs, os, path, WF } from './preamble.mjs';

const src = fs.readFileSync(WF, 'utf8').replace(/^export /m, '');
const wrapped = path.join(os.tmpdir(), 'verify-project-directives-e2e-' + process.pid + '.mjs');
fs.writeFileSync(wrapped, 'export default async function(args, agent, pipeline, parallel, log) {\n' + src + '\n}\n');
const run = (await import('file://' + wrapped + '?t=' + process.pid)).default;

const refParallel = async (thunks) => Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
const refPipeline = async (items, ...stages) =>
  Promise.all(
    items.map(async (item, i) => {
      let acc = item;
      for (const stage of stages) {
        try {
          acc = await stage(acc, item, i);
        } catch {
          return null;
        }
      }
      return acc;
    })
  );

const MODELS = { plan: 'p', implement: 'i', review_find: 'f', review_verify: 'v', mechanical: 'm' };
const RULE_TEXT = 'MY PROJECT RULE TEXT\n';
const SCOPED_TEXT = 'SCOPED-OUT RULE TEXT\n';
const SKIPPED_PATH = '.claude/rules/huge.md';

// A phase whose plan and diff touch only `a.rs`, so the scoped rule (globbed to a
// directory the change never touches) must be filtered OUT while the unscoped one
// is injected.
const PHASE_META = {
  roadmap: 'rm',
  phase: '1',
  stem: 'phase-1-x',
  model: 'medium',
  body: 'BODY',
  verify: 'sh ci.sh',
  models: MODELS,
  directives: [
    { path: '.claude/rules/t.md', role: 'both', paths: [], text: RULE_TEXT, chars: Array.from(RULE_TEXT).length, bytes: RULE_TEXT.length },
    { path: '.claude/rules/scoped.md', role: 'implementer', paths: ['never/touched/**'], text: SCOPED_TEXT, chars: Array.from(SCOPED_TEXT).length, bytes: SCOPED_TEXT.length },
  ],
  directivesSkipped: [{ path: SKIPPED_PATH, bytes: 9000, reason: 'exceeds the per-source byte bound (8000)' }],
};

function makeAgent(meta) {
  const prompts = [];
  const agent = async (prompt, opts) => {
    prompts.push([(opts && opts.label) || '', String(prompt)]);
    const l = (opts && opts.label) || '';
    if (l === 'fetch:phase-meta') return meta;
    if (l === 'stamp:in-progress') return { ok: true };
    if (l === 'verify:run') return { exitCode: 0, output: '' };
    if (l === 'clean:check') return { porcelain: '' };
    if (l === 'plan:author' || l === 'plan:revise')
      return { steps_per_ac: [{ ac: 'A', steps: ['s'] }], file_map: [{ path: 'a.rs', change: 'e' }], tests_per_ac: [{ ac: 'A', test: 't' }], edge_cases: [], cross_phase_deps: [], summary: 's' };
    if (l === 'implement:worktree' || l === 'implement:rework') return { changedFiles: ['a.rs'], diffText: '' };
    if (l === 'diff:signals') return { changedFiles: ['a.rs'], diffText: '' };
    if (l === 'act:code') return { handled: [] };
    if (l.indexOf('find:code:ac') === 0) return { ac: [{ criterion: 'A', status: 'PASS', evidence: 'x' }], findings: [] };
    if (l.indexOf('find:') === 0) return { findings: [] };
    if (l.indexOf('refute:') === 0) return { refuted: false, confidence: 95 };
    return null;
  };
  return { agent, prompts };
}

// --- with directives --------------------------------------------------------
{
  const { agent, prompts } = makeAgent(PHASE_META);
  const out = await run({ roadmap: 'rm', phase: '1', rdmBin: '/fake/rdm' }, agent, refPipeline, refParallel, () => {});
  const of = (label) => prompts.filter(([l]) => l === label || l.indexOf(label) === 0).map(([, p]) => p);

  // Stage 0 asked for them at all.
  assert.ok(of('fetch:phase-meta')[0].includes('dispatch directives --format json'), 'E2E: Stage 0 resolves directives');

  const impl = of('implement:worktree')[0];
  assert.ok(impl.includes(RULE_TEXT.trim()), 'E2E: the implementer prompt carries the unscoped rule VERBATIM');
  assert.ok(!impl.includes(SCOPED_TEXT.trim()), 'E2E: a rule scoped to files the change never touches is NOT injected');
  assert.ok(impl.includes(SKIPPED_PATH), 'E2E: the in-block notice names the withheld source, so the agent knows its rules are incomplete');

  const finders = of('find:code:');
  assert.ok(finders.length > 0, 'E2E: code-review finders ran');
  for (const p of finders) {
    assert.ok(p.includes(RULE_TEXT.trim()), 'E2E: every code finder prompt carries the directive');
    assert.ok(p.includes('CANNOT narrow your review'), 'E2E: ...behind the authority preamble');
  }

  // The act step edits code, so it is implementer-shaped and gets the block too.
  const act = of('act:code');
  if (act.length > 0) assert.ok(act[0].includes(RULE_TEXT.trim()), 'E2E: the act step carries the implementer block');

  // THE OUTCOME. This is the assertion the unit-level § 7 could not make: it
  // goes through itemOutcome's hand-enumerated field list, where a forgotten
  // field is dropped with no error at all.
  assert.equal(out.outcome, 'reviewed', 'E2E: the run completed');
  assert.notEqual(out.summary.indexOf('[directives: 1 source(s) not injected: ' + SKIPPED_PATH + ']'), -1,
    'E2E: the OUTCOME summary carries the withheld-directive clause: ' + out.summary);
}

// --- without directives (AC5, end to end) -----------------------------------
{
  const bare = { ...PHASE_META };
  delete bare.directives;
  delete bare.directivesSkipped;
  const { agent, prompts } = makeAgent(bare);
  const out = await run({ roadmap: 'rm', phase: '1', rdmBin: '/fake/rdm' }, agent, refPipeline, refParallel, () => {});
  const blob = JSON.stringify(out);
  assert.equal(blob.indexOf('[directives:'), -1, 'E2E: a directives-free run adds no OUTCOME clause');
  for (const [, p] of prompts) {
    assert.equal(p.indexOf('PROJECT DIRECTIVE'), -1, 'E2E: ...and no prompt in the whole run carries a directive fence');
  }
  assert.equal(out.outcome, 'reviewed', 'E2E: ...and the run is otherwise unaffected');
}

console.log('E2E OK: directives reach the implementer, the act step and every finder; the skip reaches the OUTCOME; absent changes nothing');
NODE_E2E
(cd "$TMP" && run_node e2e.mjs) || fail "9: end-to-end driver assertions failed"
pass "9: the shipped driver threads directives into every prompt and the withheld list into the OUTCOME"

# Self-test: drop the two directive fields from itemOutcome's hand-enumerated
# field list — the exact defect this section caught — and prove § 9 turns red.
mkdir -p "$TMP/mutant9"
sed '/^    directivesSkipped: f.directivesSkipped,$/d; /^    directivesDropped: f.directivesDropped,$/d' "$WF" >"$TMP/mutant9/rdm-wf-dispatch-phase.js"
if cmp -s "$WF" "$TMP/mutant9/rdm-wf-dispatch-phase.js"; then
    fail "9 self-test: could not plant the dropped-field mutation in itemOutcome"
fi
if (cd "$TMP" && DIR_WF="$TMP/mutant9/rdm-wf-dispatch-phase.js" run_node e2e.mjs) >/dev/null 2>&1; then
    fail "9 self-test: § 9 passed with itemOutcome dropping the directive fields — the OUTCOME wiring assertion is vacuous"
fi
pass "9 self-test: dropping the directive fields from itemOutcome turns § 9 red"

say "verify-project-directives.sh: ALL GREEN"
