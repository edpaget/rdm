#!/bin/sh
# Hermetic regression for the backlog-grooming workflow.
#
# backlog (`.claude/workflows/rdm-wf-backlog.js`) is the headless successor to the
# read-only, propose-only `rdm-backlog` skill: it runs `rdm backlog report`
# ONCE, fans one READ-ONLY analyzer agent out per POPULATED signal category
# (stale_tasks, duplicate_clusters, tag_clusters, archivable_roadmaps) in
# parallel, and consolidates the results into one ordered, reviewable batch of
# `{command, rationale}` proposals grouped by category plus a merged
# `## Open questions` section. It NEVER mutates the plan repo — the only
# Bash-executing agent in the whole run is the read-only report fetch, and
# every analyzer is explicitly told it may only propose text. Its pure control
# core lives once in `.claude/workflows/lib/backlog.mjs` and is copied
# BYTE-IDENTICAL into the workflow script (the Workflow runtime cannot import a
# helper module — see docs/workflow-schemas.md § "Import spike"). This harness
# gates:
#
#   1. BEHAVIOR      — the pure helpers, driven in Node (zero LLM calls): arg
#                       parsing (including the `--older-than 0` / empty `--tag`
#                       meaningful-vs-unset distinction), report
#                       parsing/defaulting, category selection and ordering,
#                       the fetch-report prompt's command threading, and batch
#                       rendering (subsection-per-category, omission of empty
#                       subsections, the aggregated Open questions section).
#   1b. DRIVEN LOOP   — buildBacklogPipeline fed fakes (a fake agent/parallel):
#                       the empty-report short-circuit (zero analyzer calls),
#                       a fully-populated report producing all four
#                       subsections, a fetch error propagating (never
#                       laundered into "Nothing to groom"), and a single
#                       analyzer crash degrading gracefully to an open
#                       question while the rest of the batch stays intact.
#   2. ZERO-MUTATION  — against a REAL seeded plan repo and the real
#                       ./target/debug/rdm binary: the real (read-only)
#                       `rdm backlog report --format json` output is fed
#                       through the pipeline with fake analyzers, and the
#                       plan repo's git HEAD, working-tree status, and a
#                       recursive file checksum are asserted byte-identical
#                       before and after the run.
#   3. BLOCK DRIFT    — the `backlog-groom` region is byte-identical between
#                       the lib source of truth and the stamped workflow
#                       script (with a planted-mutation self-test).
#   4. STATIC INVARIANTS — grep-based: exactly one Bash-executing agent
#                       directive ("Run exactly this command") in the whole
#                       file, and its command template never contains a
#                       mutating verb (with a planted-mutation self-test);
#                       no import/require; both markers present; meta.phases
#                       parity with the emitted `phase:` literals; no
#                       Date.now(/Math.random( anywhere.
#   5. MODULE PARSE   — rdm-wf-backlog.js loads under module semantics (no
#                       SyntaxError), with a planted duplicate-meta self-test.
#   6. SKILL SHIM     — .claude/skills/rdm-backlog/SKILL.md is a thin shim
#                       pointing at the `rdm-wf-backlog` Workflow tool, with the old
#                       per-category command-template prose removed.
#
# Node is used only as a host to unit-test the pure module and drive the
# pipeline with fakes; it is stdlib-only (node:assert), with no package.json /
# node_modules / third-party packages. node is pinned in .mise.toml.
#
# Run after touching .claude/workflows/lib/backlog.mjs, rdm-wf-backlog.js, or
# .claude/skills/rdm-backlog/SKILL.md.
#
# Requires: node (via PATH or `mise exec node --`), cargo-built rdm at
# target/debug/rdm (for the ZERO-MUTATION section).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

LIB="$REPO_ROOT/.claude/workflows/lib/backlog.mjs"
WF="$REPO_ROOT/.claude/workflows/rdm-wf-backlog.js"
SKILL="$REPO_ROOT/.claude/skills/rdm-backlog/SKILL.md"
RDM_BIN="$REPO_ROOT/target/debug/rdm"

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

# Parse a workflow script under MODULE semantics and fail on a SyntaxError.
# Strips the leading `export` and wraps in an async function so top-level
# `return`/`await` are legal, while keeping the top-level `const meta` in ONE
# shared scope so a redeclaration is a SyntaxError.
parse_workflow() {
    {
        echo '(async function(){'
        sed 's/^export //' "$1"
        echo '})'
    } |
        run_node --check --input-type=module -
}

# Distinct `phase: '<name>'` literals the workflow actually emits (trailing
# comma or closing brace both occur across call sites, so neither is anchored).
emitted_phases() {
    grep -oE "phase: '[A-Za-z]+'" "$1" | sed "s/phase: '//;s/'//" | sort -u
}

# Distinct `{ title: '<name>' }` entries declared in the `meta.phases` array.
declared_phases() {
    awk '/phases: \[/{p=1} p{print} p&&/^\]$/{exit}' "$1" |
        grep -oE "title: '[A-Za-z]+'" | sed "s/title: '//;s/'//" | sort -u
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# =============================================================================
say "1 & 1b. Behavior + driven pipeline: pure helpers and buildBacklogPipeline fed fakes"
# =============================================================================

cat >"$TMP/behavior.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const libPath = process.argv[2];
const m = await import(pathToFileURL(libPath).href);
const {
  parseBacklogArgs,
  CATEGORY,
  ANALYSIS_SCHEMA,
  parseBacklogReport,
  selectCategories,
  normalizeAnalysis,
  consolidateBatch,
  buildBacklogPipeline,
  buildFetchReportPrompt,
  promptStaleTasks,
  promptDuplicateClusters,
  promptTagClusters,
  promptArchivableRoadmaps,
} = m;

// --- CATEGORY registry -------------------------------------------------------
assert.deepEqual(
  CATEGORY.map((c) => c.key),
  ['stale_tasks', 'duplicate_clusters', 'tag_clusters', 'archivable_roadmaps'],
  'fixed category order'
);
for (const cat of CATEGORY) {
  assert.equal(typeof cat.analyzerPrompt, 'function', cat.key + ' has an analyzerPrompt builder');
}

// --- ANALYSIS_SCHEMA ---------------------------------------------------------
assert.equal(ANALYSIS_SCHEMA.type, 'object', 'ANALYSIS_SCHEMA is a top-level object (Anthropic tools require object)');
assert.deepEqual(ANALYSIS_SCHEMA.required, ['proposals', 'openQuestions']);

// --- parseBacklogArgs ---------------------------------------------------------
assert.deepEqual(parseBacklogArgs({}), { project: null, olderThan: null, tag: null }, 'all-optional defaults');
assert.deepEqual(parseBacklogArgs(undefined), { project: null, olderThan: null, tag: null }, 'undefined args tolerated');
assert.equal(parseBacklogArgs({ olderThan: 0 }).olderThan, 0, '--older-than 0 is meaningful, not dropped as falsy');
assert.equal(parseBacklogArgs({ olderThan: '0' }).olderThan, 0, 'string "0" coerced to int 0');
assert.equal(parseBacklogArgs({ olderThan: 5 }).olderThan, 5, 'positive olderThan');
assert.equal(parseBacklogArgs({ tag: '' }).tag, '', 'an explicit empty --tag is meaningful, distinct from unset');
assert.equal(parseBacklogArgs({}).tag, null, 'omitted tag is unset (null)');
assert.equal(parseBacklogArgs({ tag: undefined }).tag, null, 'explicit undefined tag collapses to unset');
assert.equal(parseBacklogArgs({ tag: 'bug' }).tag, 'bug', 'ordinary tag threaded through');
assert.equal(parseBacklogArgs({ project: 'rdm' }).project, 'rdm', 'project threaded through');
assert.equal(parseBacklogArgs({ project: '' }).project, null, 'empty project string collapses to unset');
assert.throws(() => parseBacklogArgs({ olderThan: -1 }), /non-negative integer/, 'negative olderThan rejected');
assert.throws(() => parseBacklogArgs({ olderThan: 'abc' }), /non-negative integer/, 'non-numeric olderThan rejected');
// A caller may stringify the Workflow tool payload; coerce it instead of failing.
assert.equal(parseBacklogArgs('{"project":"p","olderThan":3}').project, 'p', 'stringified JSON args coerced');
assert.equal(parseBacklogArgs('{"project":"p","olderThan":3}').olderThan, 3, 'stringified args keep field coercion');
assert.deepEqual(parseBacklogArgs('not json'), { project: null, olderThan: null, tag: null }, 'non-JSON string falls back to defaults');
assert.deepEqual(parseBacklogArgs('null'), { project: null, olderThan: null, tag: null }, 'JSON null falls back to defaults, no TypeError');

// --- parseBacklogReport -------------------------------------------------------
assert.deepEqual(
  parseBacklogReport({}),
  { stale_tasks: [], duplicate_clusters: [], tag_clusters: [], archivable_roadmaps: [] },
  'missing arrays default to empty'
);
const populated = {
  stale_tasks: [{ slug: 'a', title: 'A', status: 'open' }],
  duplicate_clusters: [{ members: [{ slug: 'b', title: 'B' }] }],
  tag_clusters: [{ tag: 't', tasks: [{ slug: 'c', title: 'C' }] }],
  archivable_roadmaps: [{ roadmap: 'r', title: 'R', phase_count: 2 }],
};
assert.deepEqual(parseBacklogReport(populated), populated, 'a fully populated report round-trips verbatim');
assert.deepEqual(parseBacklogReport(JSON.stringify(populated)), populated, 'a JSON string is parsed');
assert.throws(() => parseBacklogReport(null), /returned no data/, 'null report is a fetch error, not an empty report');
assert.throws(() => parseBacklogReport(undefined), /returned no data/, 'undefined report is a fetch error');
assert.throws(() => parseBacklogReport('not json'), /valid JSON/, 'non-JSON string is a fetch error');
assert.throws(() => parseBacklogReport(42), /returned no data/, 'a primitive is a fetch error');

// --- selectCategories ---------------------------------------------------------
assert.deepEqual(selectCategories({}).map((c) => c.key), [], 'all-empty report selects nothing');
assert.deepEqual(
  selectCategories(populated).map((c) => c.key),
  ['stale_tasks', 'duplicate_clusters', 'tag_clusters', 'archivable_roadmaps'],
  'a fully populated report selects all four, in CATEGORY order'
);
assert.deepEqual(
  selectCategories({ stale_tasks: [], duplicate_clusters: [], tag_clusters: [{ tag: 't', tasks: [] }], archivable_roadmaps: [] }).map(
    (c) => c.key
  ),
  ['tag_clusters'],
  'only the populated category is selected'
);
assert.deepEqual(selectCategories(null), [], 'null report tolerated, selects nothing');

// --- buildFetchReportPrompt: the ONE Bash-executing directive ----------------
const p0 = buildFetchReportPrompt({});
assert.ok(p0.includes('Run exactly this command'), 'fetch prompt is the executable directive');
assert.ok(p0.includes('./target/debug/rdm backlog report --format json'), 'base command present');
assert.ok(!p0.includes('--older-than'), 'unset olderThan omitted');
assert.ok(!p0.includes('--tag'), 'unset tag omitted');
assert.ok(!p0.includes('--project'), 'unset project omitted');
const p1 = buildFetchReportPrompt({ project: 'rdm', olderThan: 0, tag: 'bug' });
assert.ok(p1.includes('--older-than 0'), '--older-than 0 threaded through (not dropped as falsy)');
assert.ok(p1.includes('--tag bug'), '--tag threaded through');
assert.ok(p1.includes('--project rdm'), '--project threaded through');
const p2 = buildFetchReportPrompt({ tag: '' });
assert.ok(!p2.includes('--tag'), 'an explicitly empty tag is not forwarded as a CLI flag (nothing to filter on)');

// --- analyzer prompts inline the old skill's grooming rules ------------------
const stale = promptStaleTasks([{ slug: 's' }], { project: 'rdm' });
assert.ok(stale.includes('READ-ONLY'), 'stale-tasks prompt is framed read-only');
assert.ok(stale.includes('--status wont-fix'), 'retire rule present');
assert.ok(stale.includes('--roadmap-slug'), 'consolidate-into-new-roadmap rule present');
assert.ok(stale.includes('--into'), 'consolidate-into-existing-roadmap rule present');
assert.ok(stale.includes('NEVER execute'), 'non-mutation instruction present');

const dup = promptDuplicateClusters([{ members: [] }], {});
assert.ok(dup.includes('task merge'), 'merge rule present');
assert.ok(dup.includes('survivor'), 'survivor-pick rule present');

const tag = promptTagClusters([{ tag: 't', tasks: [] }], {});
assert.ok(tag.includes('roadmap list'), 'existing-roadmap read-only lookup present');
assert.ok(tag.includes('--roadmap-slug'), 'new-roadmap rule present');
assert.ok(tag.includes('mutually exclusive'), 'never-both-into-and-roadmap-slug rule present');

const arch = promptArchivableRoadmaps([{ roadmap: 'r' }], {});
assert.ok(arch.includes('roadmap archive'), 'archive rule present');
assert.ok(arch.includes('Never add --force'), 'no --force rule present');

// --- normalizeAnalysis ---------------------------------------------------------
assert.equal(normalizeAnalysis(null), null, 'null passes through as null');
assert.equal(normalizeAnalysis(undefined), null, 'undefined passes through as null');
assert.deepEqual(normalizeAnalysis({}), { proposals: [], openQuestions: [] }, 'missing arrays default to empty');
assert.deepEqual(
  normalizeAnalysis({ proposals: [{ command: 'x', rationale: 'y' }], openQuestions: ['q'] }),
  { proposals: [{ command: 'x', rationale: 'y' }], openQuestions: ['q'] },
  'well-formed result passes through'
);

// --- consolidateBatch ----------------------------------------------------------
const emptyBatch = consolidateBatch([]);
assert.ok(emptyBatch.includes('## Open questions'), 'Open questions section always present');
assert.ok(emptyBatch.includes('None.'), 'empty Open questions renders None.');
assert.ok(!/^### /m.test(emptyBatch), 'no category subsection when there is nothing to consolidate');

const oneCategoryBatch = consolidateBatch([
  { cat: CATEGORY[3], result: { proposals: [{ command: 'rdm roadmap archive terminal-rm', rationale: 'all terminal' }], openQuestions: [] } },
]);
assert.ok(oneCategoryBatch.includes('### Archivable roadmaps'), 'the one populated category gets a subsection');
assert.ok(oneCategoryBatch.includes('rdm roadmap archive terminal-rm'), 'its proposal is rendered');
assert.equal((oneCategoryBatch.match(/^### /gm) || []).length, 1, 'exactly one subsection when only one category ran');

const mixedBatch = consolidateBatch([
  { cat: CATEGORY[0], result: { proposals: [{ command: 'rdm task update s --status wont-fix', rationale: 'stale' }], openQuestions: [] } },
  { cat: CATEGORY[1], result: { proposals: [], openQuestions: ['no clear survivor'] } },
  { cat: CATEGORY[2], result: null },
]);
assert.ok(mixedBatch.includes('### Stale tasks'), 'category with a proposal gets a subsection');
assert.ok(!mixedBatch.includes('### Duplicate clusters'), 'category with zero proposals gets no subsection, even though it ran');
assert.ok(!mixedBatch.includes('### Tag clusters'), 'category whose analyzer failed gets no subsection');
assert.ok(mixedBatch.includes('no clear survivor'), "the duplicate cluster's open question is preserved");
assert.ok(mixedBatch.includes('analysis failed for this category'), 'a failed analyzer is surfaced as an open question, not silently dropped');

// --- buildBacklogPipeline: empty-report short-circuit ------------------------
{
  let agentCalls = 0;
  const fakeAgent = async () => {
    agentCalls++;
    return { proposals: [], openQuestions: [] };
  };
  const fakeParallel = async (fns) => Promise.all(fns.map((fn) => fn()));
  const pipeline = buildBacklogPipeline({
    fetchReport: async () => ({ stale_tasks: [], duplicate_clusters: [], tag_clusters: [], archivable_roadmaps: [] }),
    agent: fakeAgent,
    parallel: fakeParallel,
    log: () => {},
  });
  const result = await pipeline({});
  assert.deepEqual(
    result,
    { groomed: false, summary: 'Nothing to groom — the backlog report returned no signals' },
    'empty report short-circuits to the exact Nothing-to-groom message'
  );
  assert.equal(agentCalls, 0, 'no analyzer agent runs when there is nothing to groom');
}

// --- buildBacklogPipeline: fully populated report ----------------------------
{
  let agentCalls = 0;
  const seenLabels = [];
  const fakeParallel = async (fns) => Promise.all(fns.map((fn) => fn()));
  const pipeline = buildBacklogPipeline({
    fetchReport: async (cfg) => {
      assert.equal(cfg.project, null, 'fetchReport receives the parsed cfg');
      return populated;
    },
    agent: async (prompt, opts) => {
      agentCalls++;
      seenLabels.push(opts.label);
      assert.equal(opts.schema, ANALYSIS_SCHEMA, 'every analyzer call is forced to ANALYSIS_SCHEMA');
      return { proposals: [{ command: 'rdm ' + opts.label, rationale: 'r-' + opts.label }], openQuestions: ['q-' + opts.label] };
    },
    parallel: fakeParallel,
    log: () => {},
  });
  const result = await pipeline({});
  assert.equal(result.groomed, true);
  assert.equal(agentCalls, 4, 'one analyzer call per populated category');
  assert.deepEqual(
    seenLabels.sort(),
    ['analyze:archivable_roadmaps', 'analyze:duplicate_clusters', 'analyze:stale_tasks', 'analyze:tag_clusters'].sort(),
    'one uniquely-labeled analyzer call per category'
  );
  for (const title of ['Stale tasks', 'Duplicate clusters', 'Tag clusters', 'Archivable roadmaps']) {
    assert.ok(result.summary.includes('### ' + title), 'batch contains a ' + title + ' subsection');
  }
  assert.ok(result.summary.includes('## Open questions'), 'batch contains the trailing Open questions section');
}

// --- buildBacklogPipeline: fetch error surfaces, never laundered -------------
{
  const pipeline = buildBacklogPipeline({
    fetchReport: async () => {
      throw new Error('rdm backlog report exited non-zero');
    },
    agent: async () => ({ proposals: [], openQuestions: [] }),
    parallel: async (fns) => Promise.all(fns.map((fn) => fn())),
  });
  await assert.rejects(() => pipeline({}), /exited non-zero/, 'a fetch failure propagates as a real error');
}

// --- buildBacklogPipeline: one analyzer crashes, the rest survive ------------
{
  const pipeline = buildBacklogPipeline({
    fetchReport: async () => populated,
    agent: async (prompt, opts) => {
      if (opts.label === 'analyze:stale_tasks') throw new Error('analyzer crashed');
      return { proposals: [{ command: 'rdm ' + opts.label, rationale: 'ok' }], openQuestions: [] };
    },
    parallel: async (fns) => Promise.all(fns.map((fn) => fn())),
    log: () => {},
  });
  const result = await pipeline({});
  assert.equal(result.groomed, true, 'a single crashed analyzer does not abort the run');
  assert.ok(result.summary.includes('analysis failed for this category'), 'the crashed category degrades to an open question');
  assert.ok(result.summary.includes('### Tag clusters'), 'the other categories still produced their subsections');
  assert.ok(!result.summary.includes('### Stale tasks'), 'the crashed category never gets a subsection');
}

// --- determinism: repeated runs on the same input produce identical output --
{
  const pipeline = buildBacklogPipeline({
    fetchReport: async () => populated,
    agent: async (prompt, opts) => ({ proposals: [{ command: 'rdm ' + opts.label, rationale: 'r' }], openQuestions: [] }),
    parallel: async (fns) => Promise.all(fns.map((fn) => fn())),
    log: () => {},
  });
  const a = await pipeline({});
  const b = await pipeline({});
  assert.equal(a.summary, b.summary, 'identical input produces byte-identical output across runs');
}

// --- missing deps.fetchReport is a loud configuration error ------------------
assert.throws(
  () => buildBacklogPipeline({ agent: async () => ({}), parallel: async (fns) => Promise.all(fns.map((fn) => fn())) }),
  /deps.fetchReport is required/,
  'buildBacklogPipeline demands an injected fetchReport (no ambient equivalent exists)'
);

console.log('all backlog pipeline assertions passed');
NODE_TEST

if run_node "$TMP/behavior.mjs" "$LIB"; then
    pass "pure helpers + buildBacklogPipeline (empty/full/error/crash/determinism) all behave correctly"
else
    fail "backlog behavior/driven-pipeline assertions failed"
fi

# =============================================================================
say "1c. Driver: whole-file execution of rdm-wf-backlog.js's mechanical-model bootstrap gate"
# =============================================================================
# Section 1/1b only drive buildBacklogPipeline from lib/backlog.mjs — they
# never execute rdm-wf-backlog.js's own driver tail (the model:mechanical bootstrap
# + if/else gate around the pipeline call). That tail is hand-authored,
# top-level code in the workflow script itself, so it needs its own
# Node-executed test: wrap the real file body in an async function taking
# (agent, parallel, log, args) as closures, run it with fakes, and assert on
# the ACTUAL RETURN VALUE — this is what would have caught the
# `ReferenceError: result is not defined` regression that a static grep for
# `model: mechanicalModel` near fetch:report could never see.

cat >"$TMP/driver.mjs" <<'NODE_TEST'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const wfPath = process.argv[2];
const src = fs.readFileSync(wfPath, 'utf8').replace(/^export const meta/m, 'const meta');
const wrapped = '(async function (agent, parallel, log, args) {\n' + src + '\n})';
const fn = vm.runInNewContext(wrapped, {});

const populatedReport = {
  stale_tasks: [],
  duplicate_clusters: [{ members: [{ slug: 'b', title: 'B' }] }],
  tag_clusters: [],
  archivable_roadmaps: [],
};

// --- Scenario A: model:mechanical resolves to an empty string ----------------
// No fetch:report or analyze:* agent call may ever fire, and the driver must
// return a defined, structured result instead of throwing.
{
  const seenLabels = [];
  const fakeAgent = async (prompt, opts) => {
    seenLabels.push(opts.label);
    if (opts.label === 'model:mechanical') return { model: '' };
    throw new Error('unexpected agent call with label ' + opts.label + ' after an unresolved mechanical model');
  };
  const fakeParallel = async (fns) => Promise.all(fns.map((f) => f()));
  const logs = [];
  const fakeLog = (m) => logs.push(m);

  const result = await fn(fakeAgent, fakeParallel, fakeLog, {});

  assert.ok(result !== undefined, 'an unresolved mechanical model must not leave the driver returning undefined');
  assert.equal(result.groomed, false, 'an unresolved mechanical model never reports groomed: true');
  assert.equal(result.fetchError, true, 'an unresolved mechanical model is surfaced as a fetchError');
  assert.deepEqual(seenLabels, ['model:mechanical'], 'no fetch:report or analyze:* agent ever fires once the model is unresolved');
  assert.ok(
    logs.some((m) => /mechanical model could not be resolved/.test(m)),
    'the unresolved-model path logs an explanatory message'
  );
}

// --- Scenario B: model:mechanical resolves normally, report is populated -----
// fetch:report must be pinned to the resolved model, and the driver returns
// the pipeline's real groomed result.
{
  const seenLabels = [];
  const seenModels = {};
  const fakeAgent = async (prompt, opts) => {
    seenLabels.push(opts.label);
    seenModels[opts.label] = opts.model;
    if (opts.label === 'model:mechanical') return { model: 'claude-haiku-mechanical' };
    if (opts.label === 'fetch:report') return populatedReport;
    if (opts.label.indexOf('analyze:') === 0) return { proposals: [{ command: 'rdm ' + opts.label, rationale: 'r' }], openQuestions: [] };
    throw new Error('unexpected label ' + opts.label);
  };
  const fakeParallel = async (fns) => Promise.all(fns.map((f) => f()));
  const logs = [];
  const fakeLog = (m) => logs.push(m);

  const result = await fn(fakeAgent, fakeParallel, fakeLog, {});

  assert.equal(result.groomed, true, 'a populated report with a resolved mechanical model grooms normally');
  assert.equal(seenModels['fetch:report'], 'claude-haiku-mechanical', 'fetch:report is pinned to the resolved mechanical model');
  assert.ok(seenLabels.includes('analyze:duplicate_clusters'), 'the populated category is analyzed');
}

console.log('all backlog driver-tail assertions passed');
NODE_TEST

if run_node "$TMP/driver.mjs" "$WF"; then
    pass "rdm-wf-backlog.js driver tail: unresolved-model gate returns cleanly, resolved-model path pins fetch:report"
else
    fail "rdm-wf-backlog.js driver-tail execution assertions failed (the mechanical-model bootstrap gate is broken)"
fi

# =============================================================================
say "2. Zero-mutation: a real seeded plan repo is byte-identical before and after a run"
# =============================================================================

PLAN="$TMP/plan"
PROJ="backlog-wf-proj"
rdm() { "$RDM_BIN" --root "$PLAN" "$@"; }

# Hermetic env: never touch the developer's real plan repo or git identity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true
export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

mkdir -p "$PLAN"
rdm init --default-project "$PROJ" >/dev/null

# Same fixture recipe as scripts/verify-backlog-groom-loop.sh: a duplicate
# pair, a tag cluster, a stale task, and a fully-terminal (archivable) roadmap.
rdm task create dup-a --title "Fix login bug on mobile" --tags bug --no-edit --project "$PROJ" >/dev/null
rdm task create dup-b --title "Fix login bug on mobile devices" --tags mobile --no-edit --project "$PROJ" >/dev/null
rdm task create tag-a --title "Refactor the settings loader" --tags cluster-tag --no-edit --project "$PROJ" >/dev/null
rdm task create tag-b --title "Document the export pipeline" --tags cluster-tag --no-edit --project "$PROJ" >/dev/null
rdm task create stale-one --title "Stale One" --body "Retire me." --no-edit --project "$PROJ" >/dev/null
rdm roadmap create terminal-rm --title "Terminal Roadmap" --body "A finished roadmap." --no-edit --project "$PROJ" >/dev/null
rdm phase create only --title "Only" --number 1 --body "the only phase" --no-edit --roadmap terminal-rm --project "$PROJ" >/dev/null
rdm phase update phase-1-only --status "done" --no-edit --roadmap terminal-rm --project "$PROJ" >/dev/null
rdm commit -m "seed: backlog workflow zero-mutation fixtures" >/dev/null
pass "seeded a real plan repo with all four signal categories"

rdm backlog report --older-than 0 --format json --project "$PROJ" >"$TMP/report.json"
grep -q '"slug": "dup-a"' "$TMP/report.json" || fail "seeded report must surface dup-a"
pass "real 'rdm backlog report' surfaces the seeded signals"

# Snapshot: HEAD, working-tree status, and a recursive checksum of every
# tracked+untracked file under the plan repo (excluding .git internals).
snapshot() {
    (
        cd "$PLAN"
        git rev-parse HEAD
        git status --porcelain
        find . -type f -not -path './.git/*' | LC_ALL=C sort | xargs -I{} shasum {} 2>/dev/null
    )
}

BEFORE=$(snapshot)

# Drive the real pipeline: fetchReport returns the REAL report JSON (already
# fetched above, read-only); every analyzer is a deterministic fake — no LLM
# calls, and critically, no further rdm invocation of any kind.
cat >"$TMP/zero-mutation.mjs" <<'NODE_TEST'
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';

const libPath = process.argv[2];
const reportPath = process.argv[3];
const { buildBacklogPipeline } = await import(pathToFileURL(libPath).href);

const report = JSON.parse(fs.readFileSync(reportPath, 'utf8'));

const pipeline = buildBacklogPipeline({
  fetchReport: async () => report,
  agent: async (prompt, opts) => ({
    proposals: [{ command: 'rdm ' + opts.label + ' <slug>', rationale: 'deterministic fixture proposal' }],
    openQuestions: [],
  }),
  parallel: async (fns) => Promise.all(fns.map((fn) => fn())),
  log: () => {},
});

const result = await pipeline({ project: 'backlog-wf-proj' });
if (!result.groomed) throw new Error('expected the seeded report to be groomed, got: ' + JSON.stringify(result));
for (const title of ['Stale tasks', 'Duplicate clusters', 'Tag clusters', 'Archivable roadmaps']) {
  if (!result.summary.includes('### ' + title)) {
    throw new Error('expected a ' + title + ' subsection in: ' + result.summary);
  }
}
console.log('zero-mutation driven run produced a full batch');
NODE_TEST

run_node "$TMP/zero-mutation.mjs" "$LIB" "$TMP/report.json" || fail "zero-mutation driven pipeline run failed"

AFTER=$(snapshot)

if [ "$BEFORE" = "$AFTER" ]; then
    pass "plan repo git HEAD, working-tree status, and file checksums are byte-identical before and after the run"
else
    printf 'BEFORE:\n%s\n\nAFTER:\n%s\n' "$BEFORE" "$AFTER" >&2
    fail "the backlog pipeline mutated the plan repo — zero-mutation guarantee violated"
fi

# =============================================================================
say "3. Block drift: the backlog-groom region is byte-identical (lib vs workflow)"
# =============================================================================

extract_block() {
    awk '
        index($0, ">>> backlog-groom:begin") { infence = 1; next }
        index($0, ">>> backlog-groom:end") { infence = 0 }
        infence { print }
    ' "$1"
}

blocks_equal() {
    extract_block "$1" >"$TMP/_a" 2>/dev/null
    extract_block "$2" >"$TMP/_b" 2>/dev/null
    [ -s "$TMP/_a" ] && diff -q "$TMP/_a" "$TMP/_b" >/dev/null 2>&1
}

extract_block "$LIB" >"$TMP/lib-block"
[ -s "$TMP/lib-block" ] || fail "no backlog-groom block found between markers in $LIB"
extract_block "$WF" >"$TMP/wf-block"
[ -s "$TMP/wf-block" ] || fail "no backlog-groom block found between markers in $WF"

if diff -u "$TMP/lib-block" "$TMP/wf-block" >/dev/null 2>&1; then
    pass "backlog-groom block matches byte-for-byte between lib and workflow"
else
    printf '\n' >&2
    diff -u "$TMP/lib-block" "$TMP/wf-block" >&2 || true
    fail "backlog-groom block DRIFTED — copy the lib block verbatim into $WF"
fi

say "3b. Block drift detector fires on planted drift (self-test)"
cp "$LIB" "$TMP/lib.scratch"
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "scratch copies should match before mutation"
sed 's/analysis failed for this category/planted drift/' "$TMP/wf.scratch" >"$TMP/wf.mut" && mv "$TMP/wf.mut" "$TMP/wf.scratch"
if blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch"; then
    fail "byte-equality gate did NOT detect a planted mutation inside the block"
fi
cp "$WF" "$TMP/wf.scratch"
blocks_equal "$TMP/lib.scratch" "$TMP/wf.scratch" || fail "restore did not heal the byte-equality gate"
pass "drift detector fails on a planted mutation and heals on restore"

# =============================================================================
say "4. Static invariants on the workflow source"
# =============================================================================

# Exactly TWO Bash-executing agent directives in the whole file — the Stage-0
# report fetch and the mechanical-model bootstrap resolve. No analyzer prompt
# may say "Run exactly this command".
DIRECTIVES=$(grep -c "Run exactly this command" "$WF" || true)
[ "$DIRECTIVES" -eq 2 ] || fail "expected exactly two 'Run exactly this command' directives in rdm-wf-backlog.js, found $DIRECTIVES"
printf 'Run exactly this command\nRun exactly this command\nRun exactly this command\n' >"$TMP/planted-three-directives.js"
[ "$(grep -c "Run exactly this command" "$TMP/planted-three-directives.js")" -eq 3 ] ||
    fail "directive-count detector broken — missed a planted third occurrence"
pass "exactly two Bash-executing agent directives in rdm-wf-backlog.js (report fetch + mechanical-model resolve)"

# That one directive's command template (buildFetchReportPrompt's body) must
# never contain a mutating verb — extracted from `function buildFetchReportPrompt`
# to the next top-level `function `/`const `/`}` at column 0.
extract_fetch_report_fn() {
    awk '
        /^function buildFetchReportPrompt/ { collect = 1 }
        collect { print }
        collect && /^}$/ { exit }
    ' "$1"
}
extract_fetch_report_fn "$WF" >"$TMP/fetch-report-fn"
[ -s "$TMP/fetch-report-fn" ] || fail "could not extract buildFetchReportPrompt from $WF"
grep -q "Run exactly this command" "$TMP/fetch-report-fn" ||
    fail "buildFetchReportPrompt must be the function containing the one executable directive"

FORBIDDEN_VERBS="rdm task create|rdm task update|rdm task merge|rdm roadmap archive|rdm promote|rdm commit|rdm discard"
if grep -qE "$FORBIDDEN_VERBS" "$TMP/fetch-report-fn"; then
    grep -nE "$FORBIDDEN_VERBS" "$TMP/fetch-report-fn" >&2 || true
    fail "the report-fetch executable command template must never contain a mutating verb"
fi
pass "the report-fetch executable command template contains no mutating verb"

# The mechanical-model bootstrap's command template (buildMechanicalModelPrompt's
# body) must also never contain a mutating verb — same extraction pattern.
extract_mechanical_model_fn() {
    awk '
        /^function buildMechanicalModelPrompt/ { collect = 1 }
        collect { print }
        collect && /^}$/ { exit }
    ' "$1"
}
extract_mechanical_model_fn "$WF" >"$TMP/mechanical-model-fn"
[ -s "$TMP/mechanical-model-fn" ] || fail "could not extract buildMechanicalModelPrompt from $WF"
grep -q "Run exactly this command" "$TMP/mechanical-model-fn" ||
    fail "buildMechanicalModelPrompt must be the function containing the mechanical-model directive"
if grep -qE "$FORBIDDEN_VERBS" "$TMP/mechanical-model-fn"; then
    grep -nE "$FORBIDDEN_VERBS" "$TMP/mechanical-model-fn" >&2 || true
    fail "the mechanical-model executable command template must never contain a mutating verb"
fi
pass "the mechanical-model executable command template contains no mutating verb"

say "4b. Planted-mutation self-test on the executable command template"
cp "$WF" "$TMP/wf.mutverb.scratch"
sed "s/if (c.project) cmd += ' --project ' + c.project;/if (c.project) cmd += ' --project ' + c.project; cmd += ' \&\& rdm task create x';/" \
    "$WF" >"$TMP/wf.mutverb.scratch"
extract_fetch_report_fn "$TMP/wf.mutverb.scratch" >"$TMP/fetch-report-fn.mutant"
if ! grep -qE "$FORBIDDEN_VERBS" "$TMP/fetch-report-fn.mutant"; then
    fail "planted-mutation self-test broken — the injected 'rdm task create' was not detected"
fi
extract_fetch_report_fn "$WF" >"$TMP/fetch-report-fn.orig"
if grep -qE "$FORBIDDEN_VERBS" "$TMP/fetch-report-fn.orig"; then
    fail "the real file must still pass after the self-test"
fi
pass "planted-mutation self-test: detector fires on injected verb, real file still passes"

# No import/require (the runtime forbids it); both markers present.
if grep -nE '(^|[^A-Za-z_])import[ (]' "$WF" >/dev/null 2>&1; then
    fail "rdm-wf-backlog.js must not import (the runtime forbids it — sharing is by stamped copy)"
fi
if grep -nE '(^|[^A-Za-z_])require\(' "$WF" >/dev/null 2>&1; then
    fail "rdm-wf-backlog.js must not require() (the runtime forbids it)"
fi
grep -q '>>> backlog-groom:begin' "$WF" || fail "missing backlog-groom:begin marker"
grep -q '>>> backlog-groom:end' "$WF" || fail "missing backlog-groom:end marker"
pass "no import/require; both backlog-groom markers present"

# No Date.now(/Math.random( anywhere in either file.
for f in "$LIB" "$WF"; do
    if grep -qF 'Date.now(' "$f" || grep -qF 'Math.random(' "$f"; then
        fail "$f must not contain Date.now(/Math.random( — the pipeline must be deterministic"
    fi
done
printf 'const t = Date.now()\n' >"$TMP/planted-datenow.js"
grep -qF 'Date.now(' "$TMP/planted-datenow.js" || fail "Date.now( detector broken"
printf 'const r = Math.random()\n' >"$TMP/planted-mathrandom.js"
grep -qF 'Math.random(' "$TMP/planted-mathrandom.js" || fail "Math.random( detector broken"
pass "no Date.now(/Math.random( in lib or workflow; detectors catch planted ones"

# meta.phases must list EXACTLY the distinct emitted `phase:` literals.
DECLARED_PHASES=$(declared_phases "$WF")
EMITTED_PHASES=$(emitted_phases "$WF")
if [ "$DECLARED_PHASES" = "$EMITTED_PHASES" ]; then
    pass "meta.phases lists exactly the emitted phase: literals ($(echo "$EMITTED_PHASES" | tr '\n' ' '))"
else
    printf 'declared (meta.phases): %s\n' "$(echo "$DECLARED_PHASES" | tr '\n' ' ')" >&2
    printf 'emitted   (phase: ...): %s\n' "$(echo "$EMITTED_PHASES" | tr '\n' ' ')" >&2
    fail "meta.phases drift: declared phases != emitted phase: literals"
fi
sed "s/phase: 'Report',/phase: 'Ghost',/" "$WF" >"$TMP/wf.phase.scratch"
if [ "$(declared_phases "$TMP/wf.phase.scratch")" = "$(emitted_phases "$TMP/wf.phase.scratch")" ]; then
    fail "meta.phases consistency check did NOT catch a planted undeclared phase"
fi
pass "meta.phases consistency detector catches a planted undeclared phase"

# =============================================================================
say "4c. Mechanical-tier pin: fetch:report resolves to the mechanical model"
# =============================================================================

# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/lib/mechanical-tier-check.sh"

agent_option_blocks "$WF" >"$TMP/mech-blocks"
[ -s "$TMP/mech-blocks" ] || fail "AC-MECHANICAL-TIER: could not extract any agent() option blocks from rdm-wf-backlog.js"

assert_label_model "$TMP/mech-blocks" 'fetch:report' 'mechanicalModel' ||
    fail "AC-MECHANICAL-TIER: fetch:report must resolve to model: mechanicalModel"
pass "AC-MECHANICAL-TIER: fetch:report resolves to model: mechanicalModel"

# Self-test: plant a repoint from mechanicalModel to a hardcoded wrong model
# and prove the check now fails; restore and prove it passes again.
sed "s/model: mechanicalModel,/model: 'claude-opus-4-8',/" "$WF" >"$TMP/wf.mech-mutant"
agent_option_blocks "$TMP/wf.mech-mutant" >"$TMP/mech-blocks-mutant"
if assert_label_model "$TMP/mech-blocks-mutant" 'fetch:report' 'mechanicalModel'; then
    fail "AC-MECHANICAL-TIER: detector missed a fetch:report repoint away from mechanicalModel"
fi
pass "AC-MECHANICAL-TIER: detector fires when fetch:report is repointed away from mechanicalModel"

# =============================================================================
say "5. Module parse: rdm-wf-backlog.js loads under module semantics (no SyntaxError)"
# =============================================================================

if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "rdm-wf-backlog.js parses under module semantics (top-level meta declared once)"
else
    parse_workflow "$WF" >&2 || true
    fail "rdm-wf-backlog.js does NOT parse — fix the SyntaxError"
fi

say "5b. Parse gate fires on a planted syntax error (self-test)"
cp "$WF" "$TMP/wf.parse.scratch"
printf '\nlet meta = null\n' >>"$TMP/wf.parse.scratch"
if parse_workflow "$TMP/wf.parse.scratch" >/dev/null 2>&1; then
    fail "parse gate did NOT catch a planted duplicate top-level 'meta' declaration"
fi
if parse_workflow "$WF" >/dev/null 2>&1; then
    pass "parse gate fails on a planted syntax error and passes the unmodified file"
else
    fail "parse gate regressed on the unmodified file after the self-test"
fi

# =============================================================================
say "6. Skill shim: rdm-backlog/SKILL.md is thin and points at the Workflow tool"
# =============================================================================

grep -q 'Workflow' "$SKILL" || fail "SKILL.md must mention the Workflow tool"
grep -qE "backlog.*[Ww]orkflow|[Ww]orkflow.*'backlog'" "$SKILL" ||
    fail "SKILL.md must invoke the 'backlog' Workflow"
if grep -q '## Grooming analysis' "$SKILL"; then
    fail "SKILL.md still carries the old per-category 'Grooming analysis' prose — that logic now lives in lib/backlog.mjs"
fi
if grep -qF 'stale_tasks** — for each task' "$SKILL"; then
    fail "SKILL.md still carries the old stale_tasks command-template prose"
fi
LINES=$(wc -l <"$SKILL" | tr -d ' ')
[ "$LINES" -le 60 ] || fail "SKILL.md is $LINES lines — expected a thin shim (~40-60 lines), the old prose may not be fully removed"
pass "SKILL.md is a thin shim ($LINES lines) invoking the backlog Workflow, old prose removed"

# --- HOIST: caller-supplied mechanicalModel / report --------------------------
# Phase 3 of the workflow-token-reduction roadmap eliminates mechanical
# subagents by never spawning them (docs/mechanical-agent-inventory.md). In
# rdm-wf-backlog.js both hoists live in the DRIVER REGION's realDeps only; the copied
# `backlog-groom` block is untouched. Both are OPTIONAL — the original agent
# call is reached through a fall-through and is never deleted — and neither
# weakens the propose-only contract: `rdm backlog report` is read-only whoever
# runs it, and the zero-mutation section above still gates that independently.
say "HOIST. rdm-wf-backlog.js driver region: mechanicalModel / report hoists and their fallbacks"

cat >"$TMP/hoist.mjs" <<'NODE_HOIST'
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const wfPath = process.argv[2];
let src = fs.readFileSync(wfPath, 'utf8');
src = src.replace(/^export /m, '');
const wrapperPath = path.join(os.tmpdir(), 'verify-workflow-backlog-hoist-wrapped.mjs');
fs.writeFileSync(wrapperPath, 'export default async function(args, agent, parallel, log) {\n' + src + '\n}\n');
const mod = await import('file://' + wrapperPath + '?t=' + process.pid);
const run = mod.default;

const REPORT = {
  stale_tasks: [{ slug: 's1', title: 'Stale one', status: 'open', age_days: 99 }],
  duplicate_clusters: [],
  tag_clusters: [],
  archivable_roadmaps: [],
};

function makeAgent(o) {
  o = o || {};
  const calls = [];
  const agent = async (prompt, opts) => {
    const label = (opts && opts.label) || '';
    calls.push({ label, prompt, opts });
    if (label === 'model:mechanical') return { model: o.model === undefined ? 'agent-haiku' : o.model };
    if (label === 'fetch:report') return o.report === undefined ? REPORT : o.report;
    if (label.startsWith('analyze:')) return { proposals: [], openQuestions: [] };
    return {};
  };
  return { agent, calls, count: (l) => calls.filter((c) => c.label === l).length };
}
const refParallel = async (thunks) => Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null)));
const nolog = () => {};

{
  const a = makeAgent({});
  const out = await run({ mechanicalModel: 'hoisted-haiku', report: REPORT }, a.agent, refParallel, nolog);
  assert.equal(a.count('model:mechanical'), 0, 'hoisted mechanicalModel -> no model:mechanical agent call');
  assert.equal(a.count('fetch:report'), 0, 'hoisted report -> no fetch:report agent call');
  assert.equal(a.count('analyze:stale_tasks'), 1, 'the hoisted report really drives the analyzer fan-out');
  assert.ok(out && typeof out.summary === 'string', 'the hoisted path still returns a summary');
}
{
  const a = makeAgent({});
  const outPlain = await run({}, a.agent, refParallel, nolog);
  assert.equal(a.count('model:mechanical'), 1, 'no hoist -> exactly one model:mechanical agent call');
  assert.equal(a.count('fetch:report'), 1, 'no hoist -> exactly one fetch:report agent call');
  const b = makeAgent({});
  const outHoisted = await run({ mechanicalModel: 'agent-haiku', report: REPORT }, b.agent, refParallel, nolog);
  assert.deepEqual(outHoisted, outPlain, 'the result is deep-equal with and without the hoists');
}
for (const [name, bad] of [
  ['null', null],
  ['empty string', ''],
  ['wrong type', 7],
]) {
  const a = makeAgent({});
  await run({ mechanicalModel: bad, report: REPORT }, a.agent, refParallel, nolog);
  assert.equal(a.count('model:mechanical'), 1, 'malformed mechanicalModel (' + name + ') falls back to the agent');
}
for (const [name, bad] of [
  ['null', null],
  ['array', []],
  ['missing one signal array', { stale_tasks: [], duplicate_clusters: [], tag_clusters: [] }],
  ['a signal key that is not an array', { ...REPORT, tag_clusters: 'none' }],
]) {
  const a = makeAgent({});
  await run({ mechanicalModel: 'hoisted-haiku', report: bad }, a.agent, refParallel, nolog);
  assert.equal(a.count('fetch:report'), 1, 'malformed report (' + name + ') falls back to the agent');
}
{
  const a = makeAgent({});
  await run(JSON.stringify({ mechanicalModel: 'hoisted-haiku', report: REPORT }), a.agent, refParallel, nolog);
  assert.equal(a.count('model:mechanical'), 0, 'a stringified args payload still surfaces mechanicalModel');
  assert.equal(a.count('fetch:report'), 0, 'a stringified args payload still surfaces report');
}
console.log('backlog hoist assertions passed');
NODE_HOIST

if run_node "$TMP/hoist.mjs" "$WF"; then
    pass "backlog hoist/fallback verified against the real driver under a recording fake agent"
else
    fail "backlog hoist/fallback assertions failed against $WF"
fi

assert_wf_mutant_fails() {
    mutant=$1
    desc=$2
    if cmp -s "$WF" "$mutant"; then
        fail "HOIST: planted mutation was a no-op — $desc"
    fi
    if run_node "$TMP/hoist.mjs" "$mutant" >/dev/null 2>&1; then
        fail "HOIST: assertions PASSED against a driver that $desc — they are vacuous"
    fi
    pass "HOIST: assertions fire when the driver $desc"
}

# (1) Drop the fetch:report fallback: return the (possibly absent) hoist always.
awk '
    index($0, "  fetchReport: async function (cfg) {") { print; print "    return rawBacklogArgs.report"; skipping = 1; next }
    skipping && index($0, "  },") == 1 { skipping = 0; print; next }
    skipping { next }
    { print }
' "$WF" >"$TMP/mutant-no-report-fallback.js"
assert_wf_mutant_fails "$TMP/mutant-no-report-fallback.js" "drops the fetch:report fallback"

# (2) Weaken the report shape guard to "anything object-ish".
sed 's/^  return \[.stale_tasks., .duplicate_clusters., .tag_clusters., .archivable_roadmaps.\].filter((k) => !Array.isArray(r\[k\]))$/  return true \&\& [].filter((k) => !Array.isArray(r[k]))/' "$WF" >"$TMP/mutant-weak-report-guard.js"
assert_wf_mutant_fails "$TMP/mutant-weak-report-guard.js" "weakens the report shape guard to any object"

# (3) Drop the model:mechanical fallback.
sed "s/^    if (typeof rawBacklogArgs.mechanicalModel === 'string' \&\& rawBacklogArgs.mechanicalModel.trim() !== '') {\$/    if (true) {/" "$WF" >"$TMP/mutant-no-model-fallback.js"
assert_wf_mutant_fails "$TMP/mutant-no-model-fallback.js" "drops the model:mechanical fallback"

# --- SHIM: the LOCAL rdm-backlog shim gathers and passes both hoists ----------
# `.claude/skills/rdm-backlog/SKILL.md` is a LOCAL dogfood shim; its distributed
# template (rdm-core/src/templates/skill-backlog-{cli,mcp}.md) is NOT a Workflow
# shim yet (tracked by task convert-remaining-skill-templates-to-workflow-shims),
# so this check belongs here and NOT in verify-agent-config-distribution.sh.
say "HOIST-SHIM. .claude/skills/rdm-backlog/SKILL.md gathers and passes mechanicalModel + report"

assert_shim_gathers() {
    grep -qF 'rdm model resolve mechanical' "$1" || return 1
    grep -qF 'rdm backlog report --format json' "$1" || return 1
    [ "$(grep -cF 'mechanicalModel' "$1")" -ge 2 ] || return 1
    [ "$(grep -cF 'report' "$1")" -ge 2 ] || return 1
    return 0
}
assert_shim_gathers "$SKILL" ||
    fail "HOIST-SHIM: $SKILL must gather 'rdm model resolve mechanical' and 'rdm backlog report --format json' and pass mechanicalModel + report (each named at least twice)"
pass "HOIST-SHIM: the local shim gathers and passes both hoisted args"

sed 's/mechanicalModel/mechModel/g' "$SKILL" >"$TMP/shim-typo.md"
if assert_shim_gathers "$TMP/shim-typo.md"; then
    fail "HOIST-SHIM: detector missed a typo'd arg key in the shim"
fi
pass "HOIST-SHIM: detector fires on a typo'd arg key in the shim"

say "verify-workflow-backlog.sh: ALL GREEN"
