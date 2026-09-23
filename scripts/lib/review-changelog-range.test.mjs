// review-changelog-range.test.mjs — proves the `changelog` code-mode reviewer
// grades the reviewed RANGE, not each individual commit, by EXECUTING the real
// pipeline and inspecting the prompt it actually dispatched — never by
// grepping the source file's prose.
//
// A dispatch review covers a phase's implementation commit plus any fix-up
// commits made while triaging that phase's own review. When a behavior change
// lands in one commit and its changelog entry is corrected by a later commit
// in the same range, the range's head still carries an accurate entry — the
// reviewer must not grade the earlier commit `blocking` in isolation. See
// phase agent-orchestrated-dispatch/phase-43-changelog-reviewer-grades-the-range.
//
// Run by rdm-core/tests/workflow_review_changelog_range.rs, so `cargo nextest
// run` is the gate.

import test from 'node:test';
import assert from 'node:assert/strict';

import { buildReviewPipeline } from '../../.claude/workflows/lib/review.mjs';

// A recording fake agent — the harness pattern from
// scripts/lib/plan-review-hoist.test.mjs's makeAgent: every call pushes its
// label and prompt, and returns an empty findings array so the pipeline
// completes without any further agent dispatch.
function makeAgent(calls) {
  return async function agent(prompt, opts) {
    calls.push({ label: (opts && opts.label) || '?', prompt: String(prompt) });
    return { findings: [] };
  };
}

const referenceParallel = async (tasks) =>
  await Promise.all(tasks.map((t) => (typeof t === 'function' ? t() : t)));

test('the changelog finder prompt grades the reviewed range, not each commit', async () => {
  const calls = [];
  const agent = makeAgent(calls);
  const runReview = buildReviewPipeline('code', {
    agent,
    parallel: referenceParallel,
    pipeline: referenceParallel,
    log: () => {},
  });

  await runReview({ target: 'change/deadbeef', reviewers: ['changelog'] });

  const call = calls.find((c) => c.label === 'find:code:changelog');
  assert.ok(call, 'the changelog finder was dispatched');

  // States range-level grading: the range's head, not a single commit, is
  // where the entry must be accurate.
  assert.match(
    call.prompt,
    /reviewed range/i,
    'the prompt should describe grading the reviewed range',
  );
  assert.match(
    call.prompt,
    /head/i,
    'the prompt should describe the range\'s head as the point graded',
  );

  // Does NOT still say the old per-commit-only wording.
  assert.equal(
    /SAME commit/.test(call.prompt),
    false,
    'the prompt must no longer require the entry in the SAME commit',
  );
});
