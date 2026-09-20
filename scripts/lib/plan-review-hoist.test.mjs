// plan-review-hoist.test.mjs — the caller-suppliable hoists of the plan-review
// engine, decided by EXECUTING code rather than by grepping it.
//
// Suite A imports the single source of truth (.claude/workflows/lib/plan-review.mjs)
// and drives the real runPlanReviewDriver against a recording fake agent.
// Suite B executes the SHIPPED artifact (.claude/workflows/rdm-wf-plan-review.js)
// itself, because the `model:mechanical` bootstrap lives in that file's runtime
// entry — outside the importable lib — and is therefore undecidable from Suite A.
//
// Run by rdm-core/tests/workflow_plan_review_driver.rs, so `cargo nextest run`
// is the gate. This file greps no source text for a string it expects to find.

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

import { runPlanReviewDriver, hoistedRoadmapBodyOk } from '../../.claude/workflows/lib/plan-review.mjs';

const checkout = fileURLToPath(new URL('../../', import.meta.url));

// ---------------------------------------------------------------- fixtures

const PHASE_BODY = '# Phase 1\n\nDo the thing.\n';

// A roadmap body whose `## Intent` section clears extractIntent's real bar
// (non-empty, and carrying both `Goal` and `Done looks like`).
const ROADMAP_BODY_WITH_INTENT = [
  '# Roadmap r',
  '',
  '## Intent',
  '',
  'Goal: make the roadmap-intent read hoistable.',
  'Done looks like: no fetch:roadmap-intent agent when the caller supplies the body.',
  '',
  '## Notes',
  '',
  'Unrelated trailing section.',
].join('\n');

// What extractIntent must hand back for the body above: the verbatim section,
// right-trimmed, stopping at the next `## ` heading.
const EXPECTED_INTENT = [
  '## Intent',
  '',
  'Goal: make the roadmap-intent read hoistable.',
  'Done looks like: no fetch:roadmap-intent agent when the caller supplies the body.',
].join('\n');

const ROADMAP_BODY_WITHOUT_INTENT = '# Roadmap r\n\nNo recorded intent section here at all.\n';

// `fetched` payloads in exactly the shape hoistedFetchedOk accepts.
const FETCHED_PHASE = { body: PHASE_BODY, tags: ['needs-plan-review'] };
const FETCHED_TASK = { body: PHASE_BODY, tags: [] };
const FETCHED_ROADMAP = {
  body: ROADMAP_BODY_WITH_INTENT,
  tags: [],
  phases: [{ stem: 'phase-1-x', body: PHASE_BODY, tags: [], status: 'not-started' }],
};

// A real fetch:roadmap transcript — marker-delimited, exactly as
// buildRoadmapFetchPrompt instructs the agent to emit it.
const ROADMAP_TRANSCRIPT = [
  '===CMD: roadmap show r --project rdm --format json===',
  JSON.stringify({ slug: 'r', body: ROADMAP_BODY_WITH_INTENT, tags: [], phases: [{ stem: 'phase-1-x', status: 'not-started' }] }),
  '===CMD: phase show phase-1-x --roadmap r --project rdm --format json===',
  JSON.stringify({ roadmap: 'r', stem: 'phase-1-x', body: PHASE_BODY, tags: [] }),
].join('\n');

// ---------------------------------------------------------------- harness

// makeAgent(overrides) — a recording fake agent. Every call pushes its label;
// the canned returns are the real parse contract of each site (a raw JSON
// stdout transcript for the show fetches, a `texts` array for wont-fix).
function makeAgent(labels, overrides = {}) {
  return async function agent(_prompt, opts) {
    const label = (opts && opts.label) || '?';
    labels.push(label);
    if (Object.prototype.hasOwnProperty.call(overrides, label)) {
      const canned = overrides[label];
      return typeof canned === 'function' ? canned() : canned;
    }
    switch (label) {
      case 'model:mechanical':
        return { mechanical: 'model-mech', reviewFind: 'model-find', reviewVerify: 'model-verify' };
      case 'fetch:phase':
        return { transcript: JSON.stringify({ roadmap: 'r', stem: 'phase-1-x', body: PHASE_BODY, tags: [] }) };
      case 'fetch:task':
        return { transcript: JSON.stringify({ slug: 't', body: PHASE_BODY, tags: [] }) };
      case 'fetch:roadmap':
        return { transcript: ROADMAP_TRANSCRIPT };
      case 'fetch:roadmap-intent':
        return { transcript: JSON.stringify({ body: ROADMAP_BODY_WITH_INTENT }) };
      case 'fetch:wontfix':
        return { texts: [] };
      default:
        // Judgment sites (find/refute) and the act/gate writers. Benign.
        return { findings: [], ok: true };
    }
  };
}

const referenceParallel = async (tasks) =>
  await Promise.all(tasks.map((t) => (typeof t === 'function' ? t() : t)));

// driveLib(args, opts) — run the REAL runPlanReviewDriver from the lib with a
// fake review pipeline, so the only agents that fire are the mechanical ones
// this test is about. `gateMode: 'return'` keeps the gate from writing.
async function driveLib(args, opts = {}) {
  const labels = [];
  const contexts = [];
  const logs = [];
  const result = await runPlanReviewDriver(args, {
    agent: makeAgent(labels, opts.agentOverrides || {}),
    parallel: referenceParallel,
    log: (m) => logs.push(String(m)),
    runPlanReview: async (ctx) => {
      contexts.push(ctx);
      return { survivors: [], acTable: null, budget: null, coverage: null };
    },
    mechanicalModel: 'model-mech',
    findModel: 'model-find',
    verifyModel: 'model-verify',
    gateMode: 'return',
  });
  return { result, labels, contexts, logs };
}

const count = (labels, label) => labels.filter((l) => l === label).length;

// ================================================================ Suite A

test('A0: hoistedRoadmapBodyOk is shape-only and independent of anything else', () => {
  assert.equal(hoistedRoadmapBodyOk(ROADMAP_BODY_WITH_INTENT), true);
  assert.equal(hoistedRoadmapBodyOk(ROADMAP_BODY_WITHOUT_INTENT), true, 'content is not this guard’s business');
  assert.equal(hoistedRoadmapBodyOk(''), false);
  assert.equal(hoistedRoadmapBodyOk('   \n\t '), false);
  assert.equal(hoistedRoadmapBodyOk(undefined), false);
  assert.equal(hoistedRoadmapBodyOk(null), false);
  assert.equal(hoistedRoadmapBodyOk(42), false);
  assert.equal(hoistedRoadmapBodyOk({ body: 'x' }), false);
  assert.equal(hoistedRoadmapBodyOk(['x']), false);
});

test('A1: roadmapBody on a phase target suppresses the fetch:roadmap-intent agent', async () => {
  const { labels } = await driveLib({ roadmap: 'r', phase: 'phase-1-x', roadmapBody: ROADMAP_BODY_WITH_INTENT });
  assert.equal(count(labels, 'fetch:roadmap-intent'), 0, `labels: ${labels.join(',')}`);
});

test('A2: the hoisted intent is really CONSUMED, not silently dropped', async () => {
  const { contexts, labels } = await driveLib({
    roadmap: 'r',
    phase: 'phase-1-x',
    roadmapBody: ROADMAP_BODY_WITH_INTENT,
  });
  assert.equal(count(labels, 'fetch:roadmap-intent'), 0);
  assert.equal(contexts.length, 1);
  assert.equal(contexts[0].signals.hasIntent, true);
  assert.equal(contexts[0].intent, EXPECTED_INTENT);
});

test('A3: a hoisted roadmapBody with no ## Intent is fail-SOFT, not an error', async () => {
  const { result, contexts, labels } = await driveLib({
    roadmap: 'r',
    phase: 'phase-1-x',
    roadmapBody: ROADMAP_BODY_WITHOUT_INTENT,
  });
  assert.equal(count(labels, 'fetch:roadmap-intent'), 0, 'a body with no intent is still a satisfied hoist');
  assert.equal(contexts.length, 1, 'the run continued');
  assert.equal(contexts[0].signals.hasIntent, false);
  assert.equal(contexts[0].intent, null);
  assert.notEqual(result.outcome, 'escalated');
  assert.notEqual(result.fetchError, true);
});

test('A3b: a roadmapBody that makes extractIntent THROW still degrades fail-soft', async () => {
  // String(x) on an object whose toString throws is the one way to get a throw
  // out of the hoist branch; it must land on the same catch the agent path uses.
  const hostile = {
    toString() {
      throw new Error('boom');
    },
    trim() {
      return 'non-empty';
    },
  };
  // hoistedRoadmapBodyOk rejects a non-string outright, so the agent path runs
  // instead — the guard itself is the first line of that defense.
  assert.equal(hoistedRoadmapBodyOk(hostile), false);
  const { result, labels } = await driveLib({ roadmap: 'r', phase: 'phase-1-x', roadmapBody: hostile });
  assert.equal(count(labels, 'fetch:roadmap-intent'), 1);
  assert.notEqual(result.fetchError, true);
});

for (const [name, value] of [
  ['absent', undefined],
  ['empty string', ''],
  ['whitespace only', '   \n  '],
  ['an object', {}],
  ['a number', 42],
  ['null', null],
]) {
  test(`A4 (${name}): the in-workflow fetch:roadmap-intent runs exactly once, unchanged`, async () => {
    const args = { roadmap: 'r', phase: 'phase-1-x' };
    if (value !== undefined) args.roadmapBody = value;
    const { result, contexts, labels } = await driveLib(args);
    assert.equal(count(labels, 'fetch:roadmap-intent'), 1, `labels: ${labels.join(',')}`);
    // And the fetched intent still reaches the pipeline, so the fallback is live.
    assert.equal(contexts[0].signals.hasIntent, true);
    assert.equal(contexts[0].intent, EXPECTED_INTENT);
    assert.notEqual(result.fetchError, true);
  });
}

test('A4b: the roadmap-intent AGENT path keeps its fail-soft degradation', async () => {
  const thrown = await driveLib(
    { roadmap: 'r', phase: 'phase-1-x' },
    {
      agentOverrides: {
        'fetch:roadmap-intent': () => {
          throw new Error('agent exploded');
        },
      },
    }
  );
  assert.equal(count(thrown.labels, 'fetch:roadmap-intent'), 1);
  assert.equal(thrown.contexts[0].signals.hasIntent, false);
  assert.equal(thrown.contexts[0].intent, null);
  assert.notEqual(thrown.result.fetchError, true);

  const empty = await driveLib(
    { roadmap: 'r', phase: 'phase-1-x' },
    { agentOverrides: { 'fetch:roadmap-intent': { transcript: '' } } }
  );
  assert.equal(empty.contexts[0].signals.hasIntent, false);
  assert.notEqual(empty.result.fetchError, true);
});

test('A5: roadmapBody is inert on a task target', async () => {
  const withBody = await driveLib({ task: 't', roadmapBody: ROADMAP_BODY_WITH_INTENT });
  const without = await driveLib({ task: 't' });
  assert.deepEqual(withBody.labels, without.labels, 'no agent added or removed');
  assert.equal(count(withBody.labels, 'fetch:roadmap-intent'), 0, 'a task has no parent roadmap');
  assert.equal(withBody.contexts[0].signals.hasIntent, false, 'a task unit never inherits an intent');
  assert.equal(withBody.contexts[0].intent, null);
});

test('A5b: roadmapBody is inert on a roadmap target; the fan-out still fires ONE fetch:roadmap', async () => {
  const withBody = await driveLib({ roadmap: 'r', roadmapBody: ROADMAP_BODY_WITH_INTENT });
  const without = await driveLib({ roadmap: 'r' });
  assert.deepEqual(withBody.labels, without.labels, 'no agent added or removed');
  assert.equal(count(withBody.labels, 'fetch:roadmap'), 1, `labels: ${withBody.labels.join(',')}`);
  assert.equal(count(withBody.labels, 'fetch:roadmap-intent'), 0);
});

test('A6: fetched and roadmapBody are INDEPENDENT in both directions', async () => {
  const fetchedOnly = await driveLib({ roadmap: 'r', phase: 'phase-1-x', fetched: FETCHED_PHASE });
  assert.equal(count(fetchedOnly.labels, 'fetch:phase'), 0, 'fetched suppressed its own agent');
  assert.equal(count(fetchedOnly.labels, 'fetch:roadmap-intent'), 1, 'and only its own');

  const bodyOnly = await driveLib({ roadmap: 'r', phase: 'phase-1-x', roadmapBody: ROADMAP_BODY_WITH_INTENT });
  assert.equal(count(bodyOnly.labels, 'fetch:phase'), 1, 'the reverse');
  assert.equal(count(bodyOnly.labels, 'fetch:roadmap-intent'), 0);

  const both = await driveLib({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    roadmapBody: ROADMAP_BODY_WITH_INTENT,
  });
  assert.equal(count(both.labels, 'fetch:phase'), 0);
  assert.equal(count(both.labels, 'fetch:roadmap-intent'), 0);
  assert.equal(both.contexts[0].signals.hasIntent, true, 'both hoists, intent still consumed');
});

test('A7: the fetched path keeps its FAIL-CLOSED empty-body behavior', async () => {
  const { result, labels } = await driveLib(
    { roadmap: 'r', phase: 'phase-1-x' },
    { agentOverrides: { 'fetch:phase': { transcript: '' } } }
  );
  assert.equal(result.outcome, 'escalated');
  assert.equal(result.fetchError, true);
  assert.equal(result.gateBlockedCount, 0);
  assert.equal(result.gateDeferredCount, 0);
  assert.deepEqual(result.units, []);
  assert.ok(count(labels, 'fetch:phase') >= 1, 'the fetch was attempted');
});

test('A8: wontFixedTexts regression lock — supplied suppresses, omitted dispatches', async () => {
  const supplied = await driveLib({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    wontFixedTexts: [],
  });
  assert.equal(count(supplied.labels, 'fetch:wontfix'), 0, 'an EMPTY array is a legal, meaningful value');

  const omitted = await driveLib({ roadmap: 'r', phase: 'phase-1-x', fetched: FETCHED_PHASE });
  assert.equal(count(omitted.labels, 'fetch:wontfix'), 1);
});

test('A9: the full orchestrator payload dispatches none of the mechanical fetchers (lib half)', async () => {
  const { labels } = await driveLib({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    roadmapBody: ROADMAP_BODY_WITH_INTENT,
    wontFixedTexts: ['Plan review finding: something already dismissed'],
  });
  for (const forbidden of ['fetch:phase', 'fetch:roadmap-intent', 'fetch:wontfix']) {
    assert.equal(count(labels, forbidden), 0, `${forbidden} still ran; labels: ${labels.join(',')}`);
  }
});

test('A10: a roadmap-target hoist still accepts the documented `fetched` + phases shape', async () => {
  const { labels } = await driveLib({ roadmap: 'r', fetched: FETCHED_ROADMAP, wontFixedTexts: [] });
  assert.equal(count(labels, 'fetch:roadmap'), 0);
  assert.equal(count(labels, 'fetch:wontfix'), 0);
});

// ================================================================ Suite B
//
// The shipped artifact, EXECUTED. `model:mechanical` is resolved in
// rdm-wf-plan-review.js's runtime entry, below the copied driver block, so it
// is unreachable from the lib import above. The file declares no import/require,
// probes exactly the four ambient globals plus `args`, has one top-level
// `export` (the meta object, elided below because a function body may not carry
// one) and ends in a top-level `return` — which is what makes the
// AsyncFunction wrapper legal.

const WORKFLOW_PATH = new URL('../../.claude/workflows/rdm-wf-plan-review.js', import.meta.url);
const WORKFLOW_SRC = fs.readFileSync(WORKFLOW_PATH, 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

function compileWorkflow() {
  assert.equal(/^\s*(import|require)\s/m.test(WORKFLOW_SRC), false, 'the runtime cannot import');
  const stripped = WORKFLOW_SRC.replace(/^export const meta = \{[\s\S]*?^\}$/m, '/* meta elided */');
  assert.notEqual(stripped, WORKFLOW_SRC, 'the single top-level export was not found');
  assert.equal(/^\s*export\s/m.test(stripped), false, 'more than one top-level export');
  return new AsyncFunction('args', 'agent', 'parallel', 'log', 'pipeline', stripped);
}

async function driveWorkflow(args, overrides = {}) {
  const fn = compileWorkflow();
  const labels = [];
  const logs = [];
  const result = await fn(
    args,
    makeAgent(labels, overrides),
    referenceParallel,
    (m) => logs.push(String(m)),
    (x) => x
  );
  return { result, labels, logs };
}

test('B0: the workflow artifact compiles and runs under the wrapper', async () => {
  const { result } = await driveWorkflow({ roadmap: 'r', phase: 'phase-1-x' });
  assert.equal(typeof result, 'object');
  assert.equal(result.kind, 'phase');
});

test('B1 (control): with NO hoists, all four mechanical agents run', async () => {
  const { labels } = await driveWorkflow({ roadmap: 'r', phase: 'phase-1-x' });
  for (const expected of ['model:mechanical', 'fetch:phase', 'fetch:roadmap-intent', 'fetch:wontfix']) {
    assert.ok(labels.includes(expected), `${expected} missing; labels: ${labels.join(',')}`);
  }
});

test('B2: the FULL orchestrator payload dispatches none of those four', async () => {
  const { labels } = await driveWorkflow({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    roadmapBody: ROADMAP_BODY_WITH_INTENT,
    wontFixedTexts: ['Plan review finding: something already dismissed'],
    mechanicalModel: 'model-mech',
    findModel: 'model-find',
    verifyModel: 'model-verify',
    persist: { on: 'plan/p' },
  });
  for (const forbidden of ['model:mechanical', 'fetch:phase', 'fetch:roadmap-intent', 'fetch:wontfix']) {
    assert.equal(labels.includes(forbidden), false, `${forbidden} still ran; labels: ${labels.join(',')}`);
  }
});

test('B2b: the same payload arriving as a JSON STRING hoists identically', async () => {
  const payload = {
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    roadmapBody: ROADMAP_BODY_WITH_INTENT,
    wontFixedTexts: [],
    mechanicalModel: 'model-mech',
    findModel: 'model-find',
    verifyModel: 'model-verify',
  };
  const { labels } = await driveWorkflow(JSON.stringify(payload));
  for (const forbidden of ['model:mechanical', 'fetch:phase', 'fetch:roadmap-intent', 'fetch:wontfix']) {
    assert.equal(labels.includes(forbidden), false, `${forbidden} still ran; labels: ${labels.join(',')}`);
  }
});

test('B3: a PARTIAL model trio still spawns the bootstrap (all-or-nothing intact)', async () => {
  const { labels } = await driveWorkflow({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    roadmapBody: ROADMAP_BODY_WITH_INTENT,
    wontFixedTexts: [],
    mechanicalModel: 'model-mech',
    findModel: 'model-find',
  });
  assert.ok(labels.includes('model:mechanical'), `labels: ${labels.join(',')}`);
});

test('B4: an empty bootstrap result keeps the pre-existing fail-closed abort', async () => {
  const { result, labels } = await driveWorkflow(
    { roadmap: 'r', phase: 'phase-1-x' },
    { 'model:mechanical': { mechanical: '', reviewFind: '', reviewVerify: '' } }
  );
  assert.equal(result.outcome, 'escalated');
  assert.equal(result.fetchError, true);
  assert.equal(result.gateBlockedCount, 0);
  assert.equal(result.gateDeferredCount, 0);
  assert.deepEqual(result.units, []);
  assert.match(result.summary, /model\(s\) unresolved/);
  assert.equal(labels.includes('fetch:phase'), false, 'the abort fires before any fetch');
});

test('B5: the lib and the shipped artifact are both present where this test expects them', () => {
  assert.ok(fs.existsSync(new URL('.claude/workflows/lib/plan-review.mjs', `file://${checkout}`)));
  assert.ok(fs.existsSync(WORKFLOW_PATH));
});
