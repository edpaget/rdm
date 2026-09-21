// plan-review-hoist.test.mjs — the plan-review engine's contract, decided by
// EXECUTING code rather than by grepping it.
//
// Suite A imports the single source of truth (.claude/workflows/lib/plan-review.mjs)
// and drives the real runPlanReviewDriver against a RECORDING fake agent, so the
// central claim of this phase — "no agent in a workflow runs a shell command" —
// is decided by looking at what the driver actually dispatched, not by counting
// literals in the source.
// Suite B executes the SHIPPED artifact (.claude/workflows/rdm-wf-plan-review.js)
// itself, because its runtime entry lives outside the importable lib.
// Suite C covers the implementation-plan path.
// Suite D covers the CALLER-SELECTED REVIEWER SET against the review core's own
// catalogue — asserted against `DIMENSIONS`' keys, never against transcribed
// name literals.
//
// Run by rdm-core/tests/workflow_plan_review_driver.rs, so `cargo nextest run`
// is the gate. This file greps no source text for a string it expects to find.

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

import { runPlanReviewDriver, parsePlanArgs, buildReviewUnits } from '../../.claude/workflows/lib/plan-review.mjs';
import { DIMENSIONS, resolveReviewers } from '../../.claude/workflows/lib/review.mjs';

const checkout = fileURLToPath(new URL('../../', import.meta.url));

// ---------------------------------------------------------------- harness

// A recording fake agent. Every call pushes its label and its prompt, so a test
// can decide WHAT was dispatched and what it was told — which is how "only
// finder and refuter agents run" becomes decidable by execution.
function makeAgent(calls) {
  return async function agent(prompt, opts) {
    calls.push({ label: (opts && opts.label) || '?', prompt: String(prompt), opts: opts || {} });
    return { findings: [], ok: true };
  };
}

const referenceParallel = async (tasks) =>
  await Promise.all(tasks.map((t) => (typeof t === 'function' ? t() : t)));

// driveLib(args, opts) — run the REAL runPlanReviewDriver from the lib with a
// fake review pipeline, so the only agents that could fire are ones the DRIVER
// dispatches on its own account. Under this harness that set must be EMPTY.
async function driveLib(args, opts = {}) {
  const calls = [];
  const contexts = [];
  const logs = [];
  const result = await runPlanReviewDriver(args, {
    agent: makeAgent(calls),
    parallel: referenceParallel,
    log: (m) => logs.push(String(m)),
    runPlanReview: async (ctx) => {
      contexts.push(ctx);
      return { survivors: opts.survivors ? opts.survivors.slice() : [], acTable: null, budget: null, coverage: null };
    },
    findModel: 'model-find',
    verifyModel: 'model-verify',
  });
  return { result, calls, labels: calls.map((c) => c.label), contexts, logs };
}

// driveReal(args, opts) — the same driver, but over the REAL
// buildReviewPipeline('plan') with the recording agent injected, so every agent
// the whole stack dispatches is captured. This is the one that decides AC-1.
import { buildReviewPipeline } from '../../.claude/workflows/lib/review.mjs';
async function driveReal(args) {
  const calls = [];
  const agent = makeAgent(calls);
  const result = await runPlanReviewDriver(args, {
    agent,
    parallel: referenceParallel,
    log: () => {},
    runPlanReview: buildReviewPipeline('plan', {
      agent,
      parallel: referenceParallel,
      pipeline: referenceParallel,
      log: () => {},
    }),
  });
  return { result, calls, labels: calls.map((c) => c.label) };
}

const isJudgmentLabel = (l) => l.startsWith('find:') || l.startsWith('refute:');

// ================================================================ Suite A

test('A1: driving the driver over EVERY target kind dispatches only finder and refuter agents', async () => {
  const runs = [
    ['phase', { roadmap: 'r', phase: 'phase-1-x', tags: ['needs-plan-review'] }],
    ['task', { task: 't', tags: [] }],
    ['roadmap', { roadmap: 'r', phases: [{ stem: 'phase-1-x' }, { stem: 'phase-2-y' }], tags: [] }],
    ['implementation-plan', { implementationPlan: true, planSlug: 'p', persist: true }],
  ];
  for (const [name, args] of runs) {
    const { labels } = await driveReal(args);
    assert.ok(labels.length > 0, name + ': the run dispatched no agent at all — the fake would pass vacuously');
    const foreign = labels.filter((l) => !isJudgmentLabel(l));
    assert.deepEqual(foreign, [], name + ': non-judgment agent(s) dispatched: ' + foreign.join(', '));
  }
});

test('A2: no dispatched prompt asks an agent to RETURN a transcript or an ack', async () => {
  const { calls } = await driveReal({ roadmap: 'r', phase: 'phase-1-x', tags: [] });
  for (const c of calls) {
    assert.equal(/mechanical (fetch|status|body-audit-note|review-persistence) agent/i.test(c.prompt), false,
      c.label + ' is still addressed as a mechanical agent');
    assert.equal(/RAW_STDOUT|PERSIST_ACK|STAMP_ACK|WONTFIX_LIST/.test(c.prompt), false,
      c.label + ' still names a transcription/ack schema');
  }
});

test('A3: each unit is told the rdm command to read its own document, and the target is a REF not a body', async () => {
  const { contexts } = await driveLib({ roadmap: 'r', phase: 'phase-1-x', tags: [] });
  assert.equal(contexts.length, 1);
  assert.equal(contexts[0].target, 'phase/r/phase-1-x', 'the graded target is an identifier');
  assert.match(contexts[0].itemCommand, /phase show phase-1-x --roadmap r .*--format json/);
  assert.match(contexts[0].roadmapCommand, /roadmap show r .*--format json/);

  const task = await driveLib({ task: 't', tags: [] });
  assert.equal(task.contexts[0].target, 'task/t');
  assert.match(task.contexts[0].itemCommand, /task show t .*--format json/);
  assert.equal(task.contexts[0].roadmapCommand, null, 'a task has no parent roadmap to read intent from');
});

test('A4: a roadmap target reviews exactly the phase stems the CALLER named', async () => {
  const swept = await driveLib({ roadmap: 'r', phases: [{ stem: 'phase-1-x' }, { stem: 'phase-2-y' }], tags: [] });
  assert.deepEqual(swept.contexts.map((c) => c.target), ['roadmap/r', 'phase/r/phase-1-x', 'phase/r/phase-2-y']);

  const alone = await driveLib({ roadmap: 'r', tags: [] });
  assert.deepEqual(alone.contexts.map((c) => c.target), ['roadmap/r'],
    'with no stem list the engine reviews the roadmap document ALONE — it never reads one to discover phases');
});

test('A5: a terminal phase stem is excluded from the sweep and reported', async () => {
  const { result, contexts } = await driveLib({
    roadmap: 'r',
    phases: [{ stem: 'phase-1-x', status: 'done' }, { stem: 'phase-2-y', status: 'in-progress' }, { stem: 'phase-3-z', status: 'wont-fix' }],
    tags: [],
  });
  assert.deepEqual(contexts.map((c) => c.target), ['roadmap/r', 'phase/r/phase-2-y']);
  assert.deepEqual(result.skippedPhases.map((p) => p.stem), ['phase-1-x', 'phase-3-z']);
  assert.match(result.summary, /skipped 2 terminal phase/);
  // FAIL-OPEN: an unknown or missing status keeps the phase in the sweep.
  const odd = await driveLib({ roadmap: 'r', phases: [{ stem: 'phase-1-x', status: 'a-future-status' }, { stem: 'phase-2-y' }], tags: [] });
  assert.deepEqual(odd.result.skippedPhases, []);
  assert.equal(odd.contexts.length, 3);
});

test('A6: the gate RETURNS its two commands and names the target; it never writes', async () => {
  const { result, labels } = await driveLib({ roadmap: 'r', phase: 'phase-1-x', tags: ['needs-plan-review', 'depends-unlanded'] });
  assert.deepEqual(labels, [], 'no gate agent may be dispatched');
  const a = result.gateAction;
  assert.equal(a.clearsPlanReviewTag, true);
  assert.deepEqual(a.remainingTags, ['depends-unlanded'], 'the sibling tag is preserved in the written-back list');
  assert.equal(a.commands.length, 2);
  assert.ok(a.commands[0].includes('phase update phase-1-x --roadmap r --tags "depends-unlanded"'), a.commands[0]);
  assert.ok(a.commands[1].includes('commit -m'), a.commands[1]);
  assert.match(result.summary, /gate pending/);
  assert.equal(result.gatePendingCount, 1);
});

test('A7: with NO tags supplied the gate refuses to guess, visibly', async () => {
  const { result } = await driveLib({ roadmap: 'r', phase: 'phase-1-x' });
  assert.equal(result.gateAction.clearsPlanReviewTag, true);
  assert.equal(result.gateAction.tagsUnknown, true);
  assert.deepEqual(result.gateAction.commands, [], 'a --tags "" would silently drop a sibling tag');
  assert.match(result.summary, /gate pending: .*tag list was not supplied/);
});

test('A8: a rework outcome leaves the tag and emits no gate command', async () => {
  const blocking = { id: 'c1', concern: 'coherence', severity: 'blocking', confidence: 95, what_fails: 'ambiguous step' };
  const { result } = await driveLib(
    { roadmap: 'r', phase: 'phase-1-x', tags: ['needs-plan-review'] },
    { survivors: [blocking] }
  );
  assert.equal(result.outcome, 'rework');
  assert.equal(result.gateAction.clearsPlanReviewTag, false);
  assert.deepEqual(result.gateAction.commands, []);
  assert.equal(result.gatePendingCount, 0);
  assert.equal(result.summary.includes('gate pending'), false);
  // The round note is RENDERED and returned, not written.
  assert.match(result.roundNote, /^## Plan Review Round 1 — rework/);
  assert.match(result.roundNote, /\[blocking\] coherence: ambiguous step/);
});

test('A9: the round channel reads the prior reviews the CALLER supplied', async () => {
  const priorReviews = [
    { id: '2026-01-01-0000-aaaa', state: 'submitted', created: '2026-01-01', comments: [] },
    { id: '2026-01-02-0000-bbbb', state: 'submitted', created: '2026-01-02', comments: [] },
  ];
  const blocking = { id: 'c1', concern: 'coherence', severity: 'blocking', confidence: 95, what_fails: 'still ambiguous' };
  const { result } = await driveLib(
    { roadmap: 'r', phase: 'phase-1-x', tags: ['needs-plan-review'], priorReviews },
    { survivors: [blocking] }
  );
  assert.equal(result.units[0].round, 3, 'two recorded reviews put this pass on round 3');
  assert.equal(result.outcome, 'escalated', 'round 3 with a live blocking finding escalates');
});

test('A10: persist RETURNS the ladder naming the unit target; no persisting agent runs', async () => {
  const { result, labels } = await driveLib({ roadmap: 'r', phase: 'phase-1-x', tags: [], persist: true });
  assert.deepEqual(labels, []);
  assert.ok(result.persistScript.includes("review start --on 'phase/r/phase-1-x'"), result.persistScript.slice(0, 200));
  assert.ok(result.persistScript.includes('review submit'));
  const off = await driveLib({ roadmap: 'r', phase: 'phase-1-x', tags: [] });
  assert.equal(Object.prototype.hasOwnProperty.call(off.result, 'persistCommands'), false);
});

test('A11: caller-supplied wont-fix texts suppress a matching finding', async () => {
  const DISMISSED = 'the plan omits regenerating the checked-in plugin tree entirely';
  const finding = { id: 'af-2', concern: 'architectural-fit', severity: 'blocking', confidence: 90, what_fails: DISMISSED };
  const suppressed = await driveLib(
    { roadmap: 'r', phase: 'phase-1-x', tags: [], wontFixedTexts: [DISMISSED] },
    { survivors: [finding] }
  );
  assert.deepEqual(suppressed.result.findings, []);
  assert.equal(suppressed.result.outcome, 'reviewed');
  const kept = await driveLib({ roadmap: 'r', phase: 'phase-1-x', tags: [] }, { survivors: [finding] });
  assert.equal(kept.result.findings.length, 1);
});

test('A12: buildReviewUnits is pure and needs nothing read', () => {
  const built = buildReviewUnits({ kind: 'roadmap', roadmap: 'r', phases: [{ stem: 'phase-1-a', tags: ['x'] }], tags: ['y'] });
  assert.equal(built.units.length, 2);
  assert.equal(built.units[0].kind, 'roadmap');
  assert.deepEqual(built.units[0].tags, ['y']);
  assert.equal(built.units[1].ident, 'phase-1-a');
  assert.deepEqual(built.units[1].tags, ['x']);
  assert.deepEqual(built.skippedPhases, []);
  for (const u of built.units) {
    assert.equal(Object.prototype.hasOwnProperty.call(u, 'body'), false, 'no unit carries a document body');
  }
});

// ================================================================ Suite B
//
// The shipped artifact, EXECUTED. Its runtime entry lives below the copied
// driver block and is therefore unreachable from Suite A's import.

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

async function driveWorkflow(args) {
  const fn = compileWorkflow();
  const calls = [];
  const result = await fn(args, makeAgent(calls), referenceParallel, () => {}, referenceParallel);
  return { result, calls, labels: calls.map((c) => c.label) };
}

test('B0: the workflow artifact compiles and runs under the wrapper', async () => {
  const { result } = await driveWorkflow({ roadmap: 'r', phase: 'phase-1-x', tags: [] });
  assert.equal(result.kind, 'phase');
});

test('B1: the SHIPPED artifact dispatches only finder and refuter agents — no model bootstrap', async () => {
  for (const args of [
    { roadmap: 'r', phase: 'phase-1-x', tags: [] },
    { task: 't', tags: [] },
    { roadmap: 'r', phases: [{ stem: 'phase-1-x' }], tags: [] },
    { implementationPlan: true, planSlug: 'p', persist: true },
  ]) {
    const { labels } = await driveWorkflow(args);
    assert.ok(labels.length > 0, 'the run dispatched no agent at all');
    const foreign = labels.filter((l) => !isJudgmentLabel(l));
    assert.deepEqual(foreign, [], 'non-judgment agent(s) dispatched: ' + foreign.join(', '));
  }
});

test('B1b: the same payload arriving as a JSON STRING behaves identically', async () => {
  const payload = { roadmap: 'r', phase: 'phase-1-x', tags: ['needs-plan-review'] };
  const asObject = await driveWorkflow(payload);
  const asString = await driveWorkflow(JSON.stringify(payload));
  assert.deepEqual(asString.result.gateAction.commands, asObject.result.gateAction.commands);
  assert.deepEqual(asString.labels.filter((l) => !isJudgmentLabel(l)), []);
});

test('B2: the lib and the shipped artifact are both present where this test expects them', () => {
  assert.ok(fs.existsSync(new URL('.claude/workflows/lib/plan-review.mjs', `file://${checkout}`)));
  assert.ok(fs.existsSync(WORKFLOW_PATH));
});

// ================================================================ Suite C

test('C1: the implementation-plan branch grades the PLAN by slug and reads no item document', async () => {
  const { contexts, labels } = await driveLib({ implementationPlan: true, planSlug: 'p', persist: true });
  assert.equal(contexts.length, 1);
  assert.equal(contexts[0].target, 'plan/p');
  assert.match(contexts[0].itemCommand, /plan show p .*--format json/);
  assert.deepEqual(labels, [], 'no mechanical agent may fire on this branch');
});

test('C2: a planSlug returns the ladder persisting to plan/<slug>; without one persist is ignored', async () => {
  const withSlug = await driveLib({ implementationPlan: true, planSlug: 'p', persist: { on: 'plan/p' } });
  assert.equal(withSlug.result.planSlug, 'p');
  assert.ok(withSlug.result.persistScript.includes("review start --on 'plan/p'"));

  const free = await driveLib({ implementationPlan: true, persist: { on: 'plan/p' } });
  assert.equal(Object.prototype.hasOwnProperty.call(free.result, 'planSlug'), false);
  assert.equal(Object.prototype.hasOwnProperty.call(free.result, 'persistCommands'), false);
  assert.ok(free.logs.some((m) => m.includes('persist ignored')), free.logs.join(' | '));
});

test('C3: the implementation-plan branch is report-only — no gate keys at all', async () => {
  const { result } = await driveLib({ implementationPlan: true, planSlug: 'p', persist: true });
  for (const key of ['gateAction', 'gatePendingCount', 'units', 'roundNote']) {
    assert.equal(Object.prototype.hasOwnProperty.call(result, key), false, 'result carries ' + key);
  }
  assert.equal(result.kind, 'implementation-plan');
  assert.equal(typeof result.summary, 'string');
});

test('C4: the surviving parsePlanArgs shape throws still fire before any agent runs', () => {
  assert.throws(() => parsePlanArgs({}), /no target/);
  assert.throws(() => parsePlanArgs(''), /no target/);
  assert.throws(
    () => parsePlanArgs({ roadmap: 'r', phase: 'phase-1-x', planSlug: 'p' }),
    /requires --implementation-plan/
  );
  assert.throws(
    () => parsePlanArgs({ implementationPlan: true, planSlug: 'p', persist: { on: 'plan/other' } }),
    /disagrees with planSlug/
  );
  assert.throws(() => parsePlanArgs({ task: 't', persist: 'yes' }), /persist must be omitted/);
  // ...and a legal planSlug still parses.
  const ok = parsePlanArgs({ implementationPlan: true, planSlug: 'p', persist: true });
  assert.equal(ok.kind, 'implementation-plan');
  assert.equal(ok.planSlug, 'p');
});

test('C5: the retired transport keys are not readable back out of parsePlanArgs', () => {
  const parsed = parsePlanArgs({
    implementationPlan: true,
    planSlug: 'p',
    planText: 'a body that must not be graded',
    fetched: { body: 'x', tags: [] },
    mechanicalModel: 'm',
    gateMode: 'apply',
  });
  for (const gone of ['planText', 'fetched', 'mechanicalModel', 'gateMode']) {
    assert.equal(Object.prototype.hasOwnProperty.call(parsed, gone), false, parsed + ' still carries ' + gone);
  }
});

// ================================================================ Suite D

const planKeys = DIMENSIONS.plan.map((d) => d.key);
const codeKeys = DIMENSIONS.code.map((d) => d.key);

test('D1: an absent set runs every reviewer for the mode', () => {
  for (const [mode, keys] of [['plan', planKeys], ['code', codeKeys]]) {
    assert.deepEqual(resolveReviewers(mode, undefined).map((d) => d.key), keys);
    assert.deepEqual(resolveReviewers(mode, null).map((d) => d.key), keys);
  }
});

test('D2: a caller-supplied set runs exactly those reviewers, in declaration order', () => {
  const picked = [planKeys[2], planKeys[0]];
  assert.deepEqual(
    resolveReviewers('plan', picked).map((d) => d.key),
    planKeys.filter((k) => picked.includes(k)),
    'declaration order, never the order the caller wrote'
  );
  const one = [codeKeys[1]];
  assert.deepEqual(resolveReviewers('code', one).map((d) => d.key), one);
  assert.equal(resolveReviewers('code', codeKeys[1]).length, 1, 'a bare string is accepted as a one-element set');
});

test('D3: an unrecognised name is DROPPED SILENTLY — never an error', () => {
  const sel = resolveReviewers('plan', [planKeys[0], 'coherance', 'not-a-reviewer']);
  assert.deepEqual(sel.map((d) => d.key), [planKeys[0]], 'the typo selected nothing and raised nothing');
});

test('D4: nothing refuses a THIN set; the one refusal is a set that resolves to none', () => {
  assert.equal(resolveReviewers('plan', [planKeys[0]]).length, 1, 'a one-reviewer set is legal');
  assert.throws(() => resolveReviewers('plan', []), /resolved to NO reviewer/);
  assert.throws(() => resolveReviewers('plan', ['nope']), /resolved to NO reviewer/);
  assert.throws(() => resolveReviewers('nonsense', null), /unknown review mode/);
});

test('D5: the set the caller passed is what the pipeline is asked for', async () => {
  const picked = [planKeys[0], planKeys[1]];
  const { contexts } = await driveLib({ roadmap: 'r', phase: 'phase-1-x', tags: [], reviewers: picked });
  assert.deepEqual(contexts[0].reviewers, picked);
  assert.deepEqual(resolveReviewers('plan', contexts[0].reviewers).map((d) => d.key), picked);

  const omitted = await driveLib({ roadmap: 'r', phase: 'phase-1-x', tags: [] });
  assert.equal(omitted.contexts[0].reviewers, null, 'omitted stays omitted — the core decides, not the driver');
});

test('D6: a caller-selected set really narrows the FINDERS the real pipeline dispatches', async () => {
  const picked = [planKeys[0]];
  const { labels } = await driveReal({ roadmap: 'r', phase: 'phase-1-x', tags: [], reviewers: picked });
  const finders = labels.filter((l) => l.startsWith('find:'));
  assert.deepEqual(finders, ['find:plan:' + planKeys[0]]);

  const all = await driveReal({ roadmap: 'r', phase: 'phase-1-x', tags: [] });
  assert.equal(all.labels.filter((l) => l.startsWith('find:')).length, planKeys.length);
});

test('D7: no reviewer entry carries a selection predicate any more', () => {
  for (const mode of Object.keys(DIMENSIONS)) {
    for (const d of DIMENSIONS[mode]) {
      assert.equal(
        Object.prototype.hasOwnProperty.call(d, 'when'),
        false,
        `${mode}/${d.key} still carries a \`when\` predicate — selection belongs to the caller`
      );
    }
  }
});
