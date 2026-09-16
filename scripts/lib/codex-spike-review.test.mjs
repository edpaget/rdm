import test from 'node:test';
import assert from 'node:assert/strict';
import {runReviewExperiment, requireCompleteReview} from './codex-spike-review.mjs';

const finding = {id: 'wrong-sum', concern: 'correctness', severity: 'blocking', confidence: 95, what_fails: 'add subtracts'};
test('canonical review barrier, independent refutation and plan driver', async () => {
  const calls = [];
  const agent = async (_prompt, opts) => {
    calls.push(opts.label);
    if (opts.label.startsWith('refute:')) return {refuted: false, confidence: 95};
    if (opts.label === 'find:code:ac') return {ac: [{criterion: 'adds', status: 'FAIL', evidence: 'sum.mjs:1'}]};
    return {findings: opts.label === 'find:code:correctness' ? [finding] : []};
  };
  const result = await runReviewExperiment({agent, target: 'add must add', model: 'fixture-model'});
  assert.equal(result.codeOutcome, 'rework');
  assert.equal(result.plan.outcome, 'reviewed');
  assert.equal(result.code.budget.graded, 1);
  const firstRefute = calls.findIndex(x => x.startsWith('refute:'));
  assert(calls.slice(0, firstRefute).includes('find:code:tests'));
  assert.equal(result.code.coverage.complete, true);
});

test('incomplete canonical coverage cannot become spike acceptance', async () => {
  await assert.rejects(runReviewExperiment({model: 'fixture-model', target: 'adds', agent: async (_, o) => {
    if (o.label.startsWith('find:code:ac')) return null;
    return {findings: []};
  }}), /incomplete/);
});

test('a blocking finding cannot imply an unperformed clean rework round', async () => {
  const result = await runReviewExperiment({model: 'fixture-model', target: 'adds', agent: async (_, o) => {
    if (o.label.startsWith('refute:')) return {refuted: false, confidence: 95};
    if (o.label === 'find:code:ac') return {ac: [{criterion: 'adds', status: 'PASS', evidence: 'fixture'}]};
    return {findings: o.label === 'find:code:correctness' ? [finding] : []};
  }});
  assert.equal(result.codeOutcome, 'rework');
});

test('ungraded budget/error and empty AC remain incomplete', () => {
  const complete = {coverage: {complete: true}, budget: {}, acTable: [{status: 'PASS'}]};
  for (const bad of [
    {...complete, budget: {passedThroughBudget: 1}},
    {...complete, budget: {refuterErrors: 1}},
    {...complete, acTable: []},
  ]) assert.throws(() => requireCompleteReview(bad, true), /incomplete/);
});
