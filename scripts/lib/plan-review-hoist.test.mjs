// plan-review-hoist.test.mjs — the caller-suppliable hoists of the plan-review
// engine, decided by EXECUTING code rather than by grepping it.
//
// Suite A imports the single source of truth (.claude/workflows/lib/plan-review.mjs)
// and drives the real runPlanReviewDriver against a recording fake agent.
// Suite D covers the CALLER-SELECTED REVIEWER SET against the review core's own
// catalogue — asserted against `DIMENSIONS`' keys, never against transcribed
// name literals.
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

import { runPlanReviewDriver, parsePlanArgs } from '../../.claude/workflows/lib/plan-review.mjs';
import { DIMENSIONS, resolveReviewers } from '../../.claude/workflows/lib/review.mjs';

const checkout = fileURLToPath(new URL('../../', import.meta.url));

// ---------------------------------------------------------------- fixtures

const PHASE_BODY = '# Phase 1\n\nDo the thing.\n';

// A roadmap body carrying a recorded `## Intent` section. Nothing transcribes it
// any more — the `intent-alignment` reviewer reads it itself when a caller
// selects that reviewer — so this fixture exists only as realistic roadmap text
// for the fan-out.
const ROADMAP_BODY_WITH_INTENT = [
  '# Roadmap r',
  '',
  '## Intent',
  '',
  'Goal: let the caller select the reviewers.',
  'Done looks like: no signals object and no transcribed intent in any review context.',
  '',
  '## Notes',
  '',
  'Unrelated trailing section.',
].join('\n');

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

// bodyCheckOf(text) — the real ROADMAP_BODY_CHECK_SCHEMA-shaped { length,
// firstLine } a fetch:*-body-check site is contracted to report for a given
// body, computed the same way roadmapBodyVerified compares them (length,
// first line up to the first newline). Used as the default canned
// fetch:plan-body-check / fetch:roadmap-body-check response so a test that
// isn't specifically about the body-check mechanism gets a body-check that
// genuinely agrees with the primary fetch, rather than an accidental mismatch
// or an accidental "unavailable" (which would silently mask a real
// disagreement bug).
function bodyCheckOf(text) {
  const str = String(text || '');
  const nl = str.indexOf('\n');
  return { length: str.length, firstLine: nl === -1 ? str : str.slice(0, nl) };
}

// makeAgent(overrides) — a recording fake agent. Every call pushes its label,
// and (when a `calls` sink is supplied) its PROMPT alongside that label, so a
// test can decide what a mechanical site was actually told rather than only
// that it fired. The canned returns are the real parse contract of each site (a
// raw JSON stdout transcript for the show fetches, a `texts` array for wont-fix).
function makeAgent(labels, overrides = {}, calls = null) {
  return async function agent(prompt, opts) {
    const label = (opts && opts.label) || '?';
    labels.push(label);
    if (calls) calls.push({ label, prompt: String(prompt) });
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
      case 'fetch:plan':
        return { transcript: JSON.stringify({ slug: 'p', body: PLAN_TEXT }) };
      case 'fetch:plan-body-check':
        return bodyCheckOf(PLAN_TEXT);
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
  const calls = [];
  const contexts = [];
  const logs = [];
  const result = await runPlanReviewDriver(args, {
    agent: makeAgent(labels, opts.agentOverrides || {}, calls),
    parallel: referenceParallel,
    log: (m) => logs.push(String(m)),
    runPlanReview: async (ctx) => {
      contexts.push(ctx);
      return { survivors: opts.survivors ? opts.survivors.slice() : [], acTable: null, budget: null, coverage: null };
    },
    mechanicalModel: 'model-mech',
    findModel: 'model-find',
    verifyModel: 'model-verify',
    gateMode: 'return',
  });
  return { result, labels, calls, contexts, logs };
}

const count = (labels, label) => labels.filter((l) => l === label).length;

// ================================================================ Suite A

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
    wontFixedTexts: ['Plan review finding: something already dismissed'],
  });
  for (const forbidden of ['fetch:phase', 'fetch:wontfix']) {
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

test('B1 (control): with NO hoists, the mechanical agents run', async () => {
  const { labels } = await driveWorkflow({ roadmap: 'r', phase: 'phase-1-x' });
  for (const expected of ['model:mechanical', 'fetch:phase', 'fetch:wontfix']) {
    assert.ok(labels.includes(expected), `${expected} missing; labels: ${labels.join(',')}`);
  }
});

test('B2: the FULL orchestrator payload dispatches none of those three', async () => {
  const { labels } = await driveWorkflow({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    wontFixedTexts: ['Plan review finding: something already dismissed'],
    mechanicalModel: 'model-mech',
    findModel: 'model-find',
    verifyModel: 'model-verify',
    persist: { on: 'plan/p' },
  });
  for (const forbidden of ['model:mechanical', 'fetch:phase', 'fetch:wontfix']) {
    assert.equal(labels.includes(forbidden), false, `${forbidden} still ran; labels: ${labels.join(',')}`);
  }
});

test('B2b: the same payload arriving as a JSON STRING hoists identically', async () => {
  const payload = {
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    wontFixedTexts: [],
    mechanicalModel: 'model-mech',
    findModel: 'model-find',
    verifyModel: 'model-verify',
  };
  const { labels } = await driveWorkflow(JSON.stringify(payload));
  for (const forbidden of ['model:mechanical', 'fetch:phase', 'fetch:wontfix']) {
    assert.equal(labels.includes(forbidden), false, `${forbidden} still ran; labels: ${labels.join(',')}`);
  }
});

test('B3: a PARTIAL model trio still spawns the bootstrap (all-or-nothing intact)', async () => {
  const { labels } = await driveWorkflow({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
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

// ================================================================ Suite C
//
// The dispatch's plan review grades the IMPLEMENTATION PLAN, not the item
// document. Every assertion below is decided by driving the real
// runPlanReviewDriver — nothing here greps source text.

// A plan body sharing no distinctive wording with PHASE_BODY, so "the phase body
// did not reach the engine" is decidable by substring.
const PLAN_TEXT = [
  '# Plan — thread the implementation plan through the review',
  '',
  '## Steps',
  '',
  '1. Rewrite the branch to grade planText.',
  '2. Persist the verdict to the plan document.',
  '',
  '## Acceptance criteria',
  '',
  '- [ ] The graded text is the plan.',
].join('\n');

const PERSIST_ACK = { ok: true, reviewId: '2026-09-20-1200-abcd', attempted: 1, anchored: 1, degraded: 0 };

// The full dispatch payload, minus whatever a given test wants to vary. This
// is the real `rdm-dispatch-phase` shape post code review `2026-09-21-1218-
// d618` (AC4 finding ac-4-dual-supply-divergence): `planSlug` alone, no
// `planText` — the two are mutually exclusive from parsePlanArgs onward, and
// the dispatch orchestrator never supplies `planText` (see SKILL.md step 6).
// The default fake agent's `fetch:plan` / `fetch:plan-body-check` resolve this
// to PLAN_TEXT (see makeAgent), so every test below still grades PLAN_TEXT —
// it now arrives via the slug-resolve path instead of being passed directly.
function planArgs(extra = {}) {
  return Object.assign(
    {
      implementationPlan: true,
      planSlug: 'p',
      persist: { on: 'plan/p' },
      wontFixedTexts: [],
    },
    extra
  );
}

test('C1: the graded target is the PLAN, resolved by slug, and no ITEM document is fetched at all', async () => {
  const { contexts, labels } = await driveLib(planArgs(), {
    agentOverrides: { 'persist:review:plan:p': PERSIST_ACK },
  });
  assert.equal(contexts.length, 1);
  assert.equal(contexts[0].target, PLAN_TEXT);
  assert.equal(
    contexts[0].target.includes('Do the thing.'),
    false,
    'the phase body leaked into the graded text'
  );
  // No ITEM document (phase/roadmap/task) is ever reachable from this branch —
  // only the plan itself, via fetch:plan (+ its independent fetch:plan-body-check).
  for (const itemFetch of ['fetch:phase', 'fetch:roadmap', 'fetch:task']) {
    assert.equal(count(labels, itemFetch), 0, `${itemFetch} ran; labels: ${labels.join(',')}`);
  }
  assert.equal(count(labels, 'fetch:plan'), 1, `labels: ${labels.join(',')}`);
});

test('C2: the implementation-plan branch threads the caller reviewer set, and no document but the plan', async () => {
  // The caller names three reviewers; `unit-of-work` is deliberately NOT one of
  // them, which is how a non-phase target keeps it out now that nothing strips
  // it after the fact.
  const chosen = ['coherence', 'architectural-fit', 'restraint'];
  const { contexts } = await driveLib(planArgs({ reviewers: chosen }), {
    agentOverrides: { 'persist:review:plan:p': PERSIST_ACK },
  });
  assert.deepEqual(contexts[0].reviewers, chosen);
  assert.equal(Object.prototype.hasOwnProperty.call(contexts[0], 'signals'), false, 'no signals channel survives');
  assert.equal(Object.prototype.hasOwnProperty.call(contexts[0], 'intent'), false, 'no transcribed intent survives');

  // Omitted: the engine is handed nothing and the core runs every reviewer.
  const all = await driveLib(planArgs(), { agentOverrides: { 'persist:review:plan:p': PERSIST_ACK } });
  assert.equal(all.contexts[0].reviewers, null);
});

test('C3: a planSlug persists the verdict to plan/<slug>; without one, persistIgnored still holds', async () => {
  const { result, labels, calls } = await driveLib(planArgs(), {
    agentOverrides: { 'persist:review:plan:p': PERSIST_ACK },
  });
  assert.equal(count(labels, 'persist:review:plan:p'), 1, `labels: ${labels.join(',')}`);
  const persistCall = calls.find((c) => c.label === 'persist:review:plan:p');
  assert.ok(persistCall.prompt.includes('plan/p'), 'the persist prompt does not name the plan document');
  assert.equal(result.reviewId, PERSIST_ACK.reviewId);
  assert.equal(result.planSlug, 'p');
  assert.ok(result.reviewPersistence, 'the accounting is reported');

  // Free-form pasted plan text, no slug: nothing to write to.
  const free = await driveLib({ implementationPlan: true, planText: PLAN_TEXT, persist: { on: 'plan/p' } });
  assert.equal(
    free.labels.some((l) => l.startsWith('persist:review:')),
    false,
    `labels: ${free.labels.join(',')}`
  );
  assert.ok(
    free.logs.some((m) => m.includes('persist ignored')),
    `the ignore was not logged; logs: ${free.logs.join(' | ')}`
  );
  assert.equal(Object.prototype.hasOwnProperty.call(free.result, 'planSlug'), false);
  assert.equal(Object.prototype.hasOwnProperty.call(free.result, 'reviewId'), false);
  assert.equal(Object.prototype.hasOwnProperty.call(free.result, 'reviewPersistence'), false);
});

test('C4: no act step and no gate write fires, and the report-only shape is kept', async () => {
  const { result, labels } = await driveLib(planArgs(), {
    agentOverrides: { 'persist:review:plan:p': PERSIST_ACK },
  });
  for (const l of labels) {
    assert.equal(l.startsWith('act:'), false, `an act step ran: ${l}`);
    assert.equal(l.startsWith('gate:clear-tag:'), false, `a gate write ran: ${l}`);
  }
  for (const key of ['gateAction', 'gateBlocked', 'gateDeferred', 'units']) {
    assert.equal(Object.prototype.hasOwnProperty.call(result, key), false, `result carries ${key}`);
  }
  assert.equal(result.kind, 'implementation-plan');
  assert.equal(typeof result.summary, 'string');
});

test('C5: caller-supplied wont-fix texts suppress a matching finding on this path', async () => {
  const DISMISSED = 'the plan omits regenerating the checked-in plugin tree entirely';
  const finding = {
    id: 'af-2',
    concern: 'architectural-fit',
    severity: 'blocking',
    confidence: 'high',
    what_fails: DISMISSED,
  };

  const suppressed = await driveLib(planArgs({ wontFixedTexts: [DISMISSED] }), {
    survivors: [finding],
    agentOverrides: { 'persist:review:plan:p': PERSIST_ACK },
  });
  assert.deepEqual(suppressed.result.findings, [], 'an already-dismissed finding resurfaced');

  const omitted = await driveLib(
    (() => {
      const a = planArgs();
      delete a.wontFixedTexts;
      return a;
    })(),
    { survivors: [finding], agentOverrides: { 'persist:review:plan:p': PERSIST_ACK } }
  );
  assert.equal(omitted.result.findings.length, 1, 'omitting the key must not suppress anything');
  assert.equal(
    omitted.labels.some((l) => l === 'fetch:wontfix'),
    false,
    'no fetch:wontfix agent is reachable in this mode'
  );
});

test('C6 (standalone regression): a phase target still grades the item body and still gates', async () => {
  const { result, contexts } = await driveLib({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    wontFixedTexts: [],
  });
  assert.equal(result.kind, 'phase');
  assert.ok(contexts[0].target.includes('Do the thing.'), 'the phase body is still the graded text');
  assert.equal(contexts[0].target.includes('## Acceptance criteria'), false, 'no plan text leaked in');
  assert.ok(result.gateAction, 'the gate action is still produced');
  assert.equal(result.gateAction.clearsPlanReviewTag, true);
});

test('C7: illegal planSlug / persist.on / dual-supply combinations throw before any agent runs; planSlug alone still parses', () => {
  assert.throws(
    () => parsePlanArgs({ roadmap: 'r', phase: 'phase-1-x', planSlug: 'p', planText: PLAN_TEXT }),
    /planSlug p requires --implementation-plan/
  );
  // The RETIRED throw: a planSlug with no planText used to be illegal (`was
  // given with no planText`). It now parses cleanly — planText is optional
  // when planSlug is present, and the driver resolves the body itself via
  // fetch:plan + fetch:plan-body-check (see C8/C9 below), rather than
  // requiring it transcribed into this call's arguments.
  const slugOnly = parsePlanArgs({ implementationPlan: true, planSlug: 'p' });
  assert.equal(slugOnly.planSlug, 'p');
  assert.equal(slugOnly.planText, '');
  assert.equal(slugOnly.kind, 'implementation-plan');
  // The NEW throw (code review 2026-09-21-1218-d618, finding
  // ac-4-dual-supply-divergence): planSlug alongside a non-empty planText used
  // to be legal, graded planText verbatim, and persisted to the
  // planSlug-derived ref — an AC4 violation, since the two could disagree. Dual
  // supply is now rejected outright, in every combination, including when
  // persist.on would otherwise have been legal or would itself have
  // disagreed.
  assert.throws(
    () => parsePlanArgs({ implementationPlan: true, planSlug: 'p', planText: PLAN_TEXT }),
    /planSlug p was given alongside a non-empty planText/
  );
  assert.throws(
    () => parsePlanArgs({ implementationPlan: true, planSlug: 'p', planText: PLAN_TEXT, persist: { on: 'plan/p' } }),
    /planSlug p was given alongside a non-empty planText/
  );
  assert.throws(
    () => parsePlanArgs({ implementationPlan: true, planSlug: 'p', planText: PLAN_TEXT, persist: { on: 'plan/other' } }),
    /planSlug p was given alongside a non-empty planText/,
    'the dual-supply throw must fire before the persist.on mismatch is even reached'
  );
  // A whitespace-only planText alongside planSlug is NOT dual supply — it is
  // the same "genuinely empty" shape as omitting planText outright.
  const whitespaceText = parsePlanArgs({ implementationPlan: true, planSlug: 'p', planText: '   \n  ' });
  assert.equal(whitespaceText.planSlug, 'p');
  assert.equal(whitespaceText.kind, 'implementation-plan');
  // persist.on mismatch still throws on its own, when planText is absent.
  assert.throws(
    () => parsePlanArgs({ implementationPlan: true, planSlug: 'p', persist: { on: 'plan/other' } }),
    /disagrees with planSlug p/
  );
  // The legal combination still parses: planSlug alone (no planText), persist
  // on the derived ref.
  const ok = parsePlanArgs({ implementationPlan: true, planSlug: 'p', persist: { on: 'plan/p' } });
  assert.equal(ok.planSlug, 'p');
  assert.equal(ok.persistIgnored, false);
  const derived = parsePlanArgs({ implementationPlan: true, planSlug: 'p', persist: true });
  assert.equal(derived.persistIgnored, false, 'an unqualified persist:true is honored and the ref is derived');
});

test('C8: planSlug-only resolves the body via exactly one fetch:plan (+ one fetch:plan-body-check that agrees), grades it, and still persists', async () => {
  const args = planArgs();
  const { result, contexts, labels, calls } = await driveLib(args, {
    agentOverrides: { 'persist:review:plan:p': PERSIST_ACK },
  });
  assert.equal(count(labels, 'fetch:plan'), 1, `labels: ${labels.join(',')}`);
  const fetchCall = calls.find((c) => c.label === 'fetch:plan');
  assert.ok(fetchCall, 'no fetch:plan call was recorded');
  assert.ok(fetchCall.prompt.includes('plan show p'), 'the fetch prompt does not name the plan slug');
  // The independent verification call (finding
  // correctness-fetch-plan-no-content-integrity-check) fires exactly once and
  // names the same slug, re-reading the document rather than reusing fetch:plan's
  // own transcript.
  assert.equal(count(labels, 'fetch:plan-body-check'), 1, `labels: ${labels.join(',')}`);
  const bodyCheckCall = calls.find((c) => c.label === 'fetch:plan-body-check');
  assert.ok(bodyCheckCall, 'no fetch:plan-body-check call was recorded');
  assert.ok(bodyCheckCall.prompt.includes('plan show p'), 'the body-check prompt does not name the plan slug');
  assert.equal(contexts.length, 1);
  assert.equal(contexts[0].target, PLAN_TEXT, 'the fetched body is not what was graded');
  assert.equal(count(labels, 'persist:review:plan:p'), 1, `labels: ${labels.join(',')}`);
  assert.equal(result.reviewId, PERSIST_ACK.reviewId);
  assert.equal(result.planSlug, 'p');
});

test('C9: a slug/body identity mismatch on fetch:plan fails closed after one retry — escalated, no persistence, no body-check', async () => {
  const args = planArgs();
  let attempts = 0;
  const { result, labels } = await driveLib(args, {
    agentOverrides: {
      'fetch:plan': () => {
        attempts++;
        // Wrong slug in the payload — the identity check must reject this,
        // regardless of how plausible the body looks.
        return { transcript: JSON.stringify({ slug: 'not-p', body: PLAN_TEXT }) };
      },
      'persist:review:plan:p': PERSIST_ACK,
    },
  });
  assert.equal(attempts, 2, 'expected exactly one bounded retry after the first untrustworthy fetch');
  assert.equal(count(labels, 'fetch:plan'), 2, `labels: ${labels.join(',')}`);
  // A null fetchedBody after the primary retry already fails closed — the
  // body-check call is guarded on a non-null candidate and must not fire.
  assert.equal(count(labels, 'fetch:plan-body-check'), 0, `labels: ${labels.join(',')}`);
  assert.equal(result.kind, 'implementation-plan');
  assert.equal(result.outcome, 'escalated');
  assert.equal(result.fetchError, true);
  assert.deepEqual(result.findings, []);
  assert.equal(result.planSlug, 'p');
  assert.equal(
    labels.some((l) => l.startsWith('persist:review:')),
    false,
    `a persist ran despite the fetch failing closed; labels: ${labels.join(',')}`
  );
  assert.equal(Object.prototype.hasOwnProperty.call(result, 'reviewId'), false);
  assert.equal(Object.prototype.hasOwnProperty.call(result, 'reviewPersistence'), false);
});

test('C10: a fetch:plan-body-check disagreement discards a schema-valid, identity-correct fetch:plan payload and fails closed', async () => {
  // The exact shape the finding recorded: fetch:plan returns a fabricated
  // one-line status sentence with the RIGHT slug (clears extractPlanFromJson
  // outright), while the independent body-check reports the length/first-line
  // of the REAL document. The two must disagree, and disagreement must win.
  const args = planArgs();
  const { result, labels } = await driveLib(args, {
    agentOverrides: {
      'fetch:plan': { transcript: JSON.stringify({ slug: 'p', body: 'Fetched the plan successfully.' }) },
      'fetch:plan-body-check': bodyCheckOf(PLAN_TEXT),
      'persist:review:plan:p': PERSIST_ACK,
    },
  });
  assert.equal(count(labels, 'fetch:plan'), 1, `labels: ${labels.join(',')}`);
  assert.equal(count(labels, 'fetch:plan-body-check'), 1, `labels: ${labels.join(',')}`);
  assert.equal(result.kind, 'implementation-plan');
  assert.equal(result.outcome, 'escalated');
  assert.equal(result.fetchError, true);
  assert.deepEqual(result.findings, []);
  assert.equal(
    labels.some((l) => l.startsWith('persist:review:')),
    false,
    `a persist ran despite the body-check disagreeing; labels: ${labels.join(',')}`
  );
  assert.equal(Object.prototype.hasOwnProperty.call(result, 'reviewId'), false);
});

test('C11: fetch:plan-body-check UNAVAILABLE (throws) proceeds unverified rather than failing closed', async () => {
  const args = planArgs();
  const { result, contexts, labels } = await driveLib(args, {
    agentOverrides: {
      'fetch:plan-body-check': () => {
        throw new Error('body-check agent exploded');
      },
      'persist:review:plan:p': PERSIST_ACK,
    },
  });
  assert.equal(count(labels, 'fetch:plan-body-check'), 1, `labels: ${labels.join(',')}`);
  assert.equal(contexts[0].target, PLAN_TEXT, 'the fetch:plan body is still graded when the check is unavailable');
  assert.notEqual(result.outcome, 'escalated');
  assert.notEqual(result.fetchError, true);
  assert.equal(result.reviewId, PERSIST_ACK.reviewId);
});

test('C12: bounded-retry RECOVERY — first fetch:plan untrustworthy, second good — the recovered body is what is graded and persisted', async () => {
  const args = planArgs();
  let attempts = 0;
  const { result, contexts, labels } = await driveLib(args, {
    agentOverrides: {
      'fetch:plan': () => {
        attempts++;
        if (attempts === 1) {
          // Wrong slug on the first attempt — untrustworthy, must be retried.
          return { transcript: JSON.stringify({ slug: 'not-p', body: PLAN_TEXT }) };
        }
        return { transcript: JSON.stringify({ slug: 'p', body: PLAN_TEXT }) };
      },
      'persist:review:plan:p': PERSIST_ACK,
    },
  });
  assert.equal(attempts, 2, 'expected exactly one bounded retry');
  assert.equal(count(labels, 'fetch:plan'), 2, `labels: ${labels.join(',')}`);
  assert.equal(count(labels, 'fetch:plan-body-check'), 1, 'the body-check runs once, against the RECOVERED body');
  assert.equal(contexts.length, 1);
  assert.equal(contexts[0].target, PLAN_TEXT, 'the recovered body, not a discarded first attempt, is what was graded');
  assert.equal(result.reviewId, PERSIST_ACK.reviewId, 'the recovered body is what was persisted');
  assert.notEqual(result.fetchError, true);
});

test('C13: extractPlanFromJson\'s empty/whitespace-body rejection is exercised end-to-end, not just via slug mismatch', async () => {
  const args = planArgs();
  let attempts = 0;
  const { result, labels } = await driveLib(args, {
    agentOverrides: {
      'fetch:plan': () => {
        attempts++;
        // Right slug, but a whitespace-only body — non-empty check must reject
        // this on identity grounds alone, before the body-check ever runs.
        return { transcript: JSON.stringify({ slug: 'p', body: '   \n  ' }) };
      },
      'persist:review:plan:p': PERSIST_ACK,
    },
  });
  assert.equal(attempts, 2, 'expected exactly one bounded retry');
  assert.equal(count(labels, 'fetch:plan'), 2, `labels: ${labels.join(',')}`);
  assert.equal(count(labels, 'fetch:plan-body-check'), 0, 'extractPlanFromJson rejected before a body-check could run');
  assert.equal(result.outcome, 'escalated');
  assert.equal(result.fetchError, true);
  assert.equal(
    labels.some((l) => l.startsWith('persist:review:')),
    false,
    `a persist ran despite the body being blank; labels: ${labels.join(',')}`
  );
});

test('C14: the other two fetch:plan failure shapes — a throwing agent and an unparseable transcript — both degrade to the same fail-closed shape', async () => {
  const thrown = await driveLib(planArgs(), {
    agentOverrides: {
      'fetch:plan': () => {
        throw new Error('agent crashed');
      },
    },
  });
  assert.equal(thrown.result.outcome, 'escalated');
  assert.equal(thrown.result.fetchError, true);
  assert.equal(count(thrown.labels, 'fetch:plan-body-check'), 0);

  const unparseable = await driveLib(planArgs(), {
    agentOverrides: { 'fetch:plan': { transcript: 'error: no such plan p' } },
  });
  assert.equal(unparseable.result.outcome, 'escalated');
  assert.equal(unparseable.result.fetchError, true);
  assert.equal(count(unparseable.labels, 'fetch:plan-body-check'), 0);
});

// ================================================================ Suite D
//
// The CALLER-SELECTED REVIEWER SET, decided by executing `resolveReviewers` and
// by driving the real plan driver. Every expectation is derived from
// `DIMENSIONS`' own keys — no name literal is transcribed into an assertion, so
// renaming a reviewer cannot leave this suite silently asserting a ghost.

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

test('D5: the set the caller passed is what the pipeline is asked for, and shows in coverage', async () => {
  const picked = [planKeys[0], planKeys[1]];
  const { contexts } = await driveLib({
    roadmap: 'r',
    phase: 'phase-1-x',
    fetched: FETCHED_PHASE,
    wontFixedTexts: [],
    reviewers: picked,
  });
  assert.equal(contexts.length, 1);
  assert.deepEqual(contexts[0].reviewers, picked);
  // And what the core would select from it — the visible-coverage contract.
  assert.deepEqual(resolveReviewers('plan', contexts[0].reviewers).map((d) => d.key), picked);

  const omitted = await driveLib({ roadmap: 'r', phase: 'phase-1-x', fetched: FETCHED_PHASE, wontFixedTexts: [] });
  assert.equal(omitted.contexts[0].reviewers, null, 'omitted stays omitted — the core decides, not the driver');
});

test('D6: no reviewer entry carries a selection predicate any more', () => {
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
