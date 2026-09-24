// review-effort.test.mjs — the resolved reasoning effort reaches every review
// agent, decided by EXECUTING the real pipeline against a recording fake agent
// and reading the options each `agent()` call actually received. Nothing here
// reads source text.
//
// The dispatch lane resolves `rdm model resolve review-find|review-verify
// --format json` into a `{model, effort}` profile and hands the pair to the
// review engines as `findModel`/`findEffort` and `verifyModel`/`verifyEffort`.
// These tests pin the contract that makes a dispatched transcript record that
// effort: supplied → every finder (including its one retry) and every refuter
// carries it; not supplied → no call carries an `effort` key at all; invalid →
// rejected before any agent is dispatched. See phase
// model-effort-profiles/phase-6-thread-effort-through-the-lane.
//
// Run by rdm-core/tests/workflow_review_effort.rs, so `cargo nextest run` is
// the gate.

import test from 'node:test';
import assert from 'node:assert/strict';

import { buildReviewPipeline, DIMENSIONS } from '../../.claude/workflows/lib/review.mjs';
import { runPlanReviewDriver, parsePlanArgs } from '../../.claude/workflows/lib/plan-review.mjs';

const referenceParallel = async (tasks) =>
  await Promise.all(tasks.map((t) => (typeof t === 'function' ? t() : t)));

const CLEAN_AC = [{ criterion: 'AC1: it works', status: 'PASS', evidence: 'covered' }];

// A recording fake agent. Every finder returns ONE gating finding (so a refuter
// is dispatched for it); every refuter returns a non-refuting verdict. The
// first dimension's first finder call returns null, so the pipeline's single
// retry is exercised too.
function makeAgent(mode, calls) {
  const firstDim = DIMENSIONS[mode][0].key;
  let nulled = false;
  return async function agent(_prompt, opts) {
    const o = opts || {};
    calls.push({ label: o.label || '?', opts: { ...o } });
    const label = o.label || '';
    if (label.startsWith('refute:')) return { refuted: false, confidence: 95 };
    if (!nulled && label === 'find:' + mode + ':' + firstDim) {
      nulled = true;
      return null;
    }
    const dim = label.split(':')[2];
    const finding = {
      id: dim + '-1',
      concern: dim,
      severity: 'blocking',
      confidence: 90,
      what_fails: 'something in ' + dim,
    };
    if (mode === 'code' && dim === 'ac') return { ac: CLEAN_AC, findings: [finding] };
    return { findings: [finding] };
  };
}

function pipelineFor(mode, agent) {
  return buildReviewPipeline(mode, { agent, parallel: referenceParallel, pipeline: referenceParallel, log: () => {} });
}

const finders = (calls) => calls.filter((c) => c.label.startsWith('find:'));
const refuters = (calls) => calls.filter((c) => c.label.startsWith('refute:'));

for (const mode of ['code', 'plan']) {
  test(mode + ': a supplied effort reaches every finder (incl. the retry) and every refuter', async () => {
    const calls = [];
    await pipelineFor(mode, makeAgent(mode, calls))({
      target: 'x',
      findModel: 'm-find',
      findEffort: 'low',
      verifyModel: 'm-verify',
      verifyEffort: 'xhigh',
      maxRefutations: 50,
    });
    const f = finders(calls);
    const r = refuters(calls);
    assert.ok(f.some((c) => c.label.endsWith(':retry')), 'the retry path was exercised');
    assert.equal(f.length, DIMENSIONS[mode].length + 1, 'one finder per dimension plus one retry');
    assert.equal(r.length, DIMENSIONS[mode].length, 'one refuter per gating finding');
    for (const c of f) {
      assert.equal(c.opts.effort, 'low', c.label + ' carries findEffort');
      assert.equal(c.opts.model, 'm-find', c.label + ' carries findModel');
    }
    for (const c of r) {
      assert.equal(c.opts.effort, 'xhigh', c.label + ' carries verifyEffort');
      assert.equal(c.opts.model, 'm-verify', c.label + ' carries verifyModel');
    }
  });

  test(mode + ': with no effort supplied, no agent call carries an effort key', async () => {
    for (const ctx of [{ target: 'x' }, { target: 'x', findEffort: '', verifyEffort: null }]) {
      const calls = [];
      await pipelineFor(mode, makeAgent(mode, calls))({ ...ctx, maxRefutations: 50 });
      assert.ok(finders(calls).length > 0 && refuters(calls).length > 0, 'the run dispatched finders and refuters');
      for (const c of calls) {
        assert.equal(Object.prototype.hasOwnProperty.call(c.opts, 'effort'), false, c.label + ' has no effort key');
      }
    }
  });

  test(mode + ': an invalid effort is rejected before any agent is dispatched', async () => {
    for (const ctx of [{ findEffort: 'turbo' }, { verifyEffort: 'turbo' }]) {
      const calls = [];
      await assert.rejects(
        () => pipelineFor(mode, makeAgent(mode, calls))({ target: 'x', ...ctx }),
        /turbo/,
      );
      assert.deepEqual(calls, [], 'no agent was dispatched for an invalid effort');
    }
  });
}

test('every Claude effort level is accepted', async () => {
  for (const effort of ['low', 'medium', 'high', 'xhigh', 'max']) {
    const calls = [];
    await pipelineFor('plan', makeAgent('plan', calls))({ target: 'x', findEffort: effort, verifyEffort: effort });
    assert.ok(calls.every((c) => c.opts.effort === effort), effort + ' reached every call');
  }
});

test('plan-review driver: findEffort/verifyEffort args reach the real pipeline\'s finder and refuter calls', async () => {
  const calls = [];
  const agent = makeAgent('plan', calls);
  await runPlanReviewDriver(
    {
      task: 't',
      tags: [],
      findModel: 'm-find',
      findEffort: 'medium',
      verifyModel: 'm-verify',
      verifyEffort: 'high',
      maxRefutations: 50,
    },
    { agent, parallel: referenceParallel, log: () => {}, runPlanReview: pipelineFor('plan', agent) },
  );
  assert.ok(finders(calls).length > 0 && refuters(calls).length > 0, 'the run dispatched finders and refuters');
  for (const c of finders(calls)) assert.equal(c.opts.effort, 'medium', c.label);
  for (const c of refuters(calls)) assert.equal(c.opts.effort, 'high', c.label);
});

test('plan-review driver: no effort args means no effort key on any call', async () => {
  const calls = [];
  const agent = makeAgent('plan', calls);
  await runPlanReviewDriver(
    { task: 't', tags: [] },
    { agent, parallel: referenceParallel, log: () => {}, runPlanReview: pipelineFor('plan', agent) },
  );
  assert.ok(calls.length > 0);
  for (const c of calls) assert.equal(Object.prototype.hasOwnProperty.call(c.opts, 'effort'), false, c.label);
});

test('parsePlanArgs trims and normalises the effort args like the model args', () => {
  const p = parsePlanArgs({ task: 't', findEffort: '  low ', verifyEffort: 'max' });
  assert.equal(p.findEffort, 'low');
  assert.equal(p.verifyEffort, 'max');
  const q = parsePlanArgs({ task: 't', findEffort: '   ', verifyEffort: 7 });
  assert.equal(q.findEffort, null);
  assert.equal(q.verifyEffort, null);
});
