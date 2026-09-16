// Real source-bound standalone regression; no live LLM calls.
import assert from 'node:assert/strict';
import * as review from '../.claude/workflows/lib/review.mjs';
const complete = { complete: true, selected: ['ac', 'correctness'], ran: ['ac', 'correctness'], failed: [], acDimensionRan: true };
const ac = [{ criterion: 'works', status: 'PASS', evidence: 'test' }];
assert.equal(review.classifyOutcome({ planFindings: [], codeReviews: [[]], evidence: { coverage: { ...complete, complete: false, failed: ['ac'], acDimensionRan: false }, acTable: null } }), 'escalated');
for (const evidence of [
  { coverage: complete, acTable: null },
  { coverage: complete, acTable: [] },
  { coverage: complete, acTable: [{ status: 'INVALID' }] },
  { coverage: { ...complete, complete: false, failed: ['correctness'] }, acTable: ac },
  { coverage: complete, acTable: ac, budget: { passedThroughBudget: 1 } },
  { coverage: complete, acTable: ac, survivors: [{ severity: 'concern', refuterError: true }] },
]) assert.equal(review.classifyOutcome({ codeReviews: [[]], evidence }), 'escalated');
assert.equal(review.classifyOutcome({ codeReviews: [[]], evidence: { coverage: complete, acTable: ac, survivors: [{ severity: 'suggestion', unrefuted: true, unrefutedReason: 'non-gating' }] } }), 'reviewed');
assert.equal(review.classifyOutcome({ codeReviews: [[]], evidence: { coverage: { complete: false, last: complete }, acTable: ac } }), 'reviewed');
console.log('automatic completeness regressions passed');

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
const root = path.resolve(import.meta.dirname, '..');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'rdm-review-source-'));
const binary = path.join(root, 'target/debug/rdm');
const sourceRepo = path.join(temp, 'source repo');
const planRepo = path.join(temp, 'plans');
const env = { ...process.env, RDM_ROOT: planRepo, RDM_PROJECT: 'verify', RDM_SESSION: 'verify-review-source',
  GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1', GIT_AUTHOR_NAME: 'Test', GIT_AUTHOR_EMAIL: 'test@example.invalid', GIT_COMMITTER_NAME: 'Test', GIT_COMMITTER_EMAIL: 'test@example.invalid' };
for (const key of Object.keys(env)) if (/^RDM_(STAGE|FORMAT|PLAN_REPO|SOURCE_REPO)/.test(key) || /^GIT_(DIR|WORK_TREE|INDEX_FILE|CONFIG_COUNT|CONFIG_KEY_|CONFIG_VALUE_)/.test(key)) delete env[key];
const run = (bin, args, cwd = sourceRepo) => execFileSync(bin, args, { cwd, env, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim();
const rdm = (args, cwd = sourceRepo) => run(binary, args, cwd);
const git = (args, cwd = sourceRepo) => run('git', args, cwd);
const shell = (command, cwd = sourceRepo) => run('/bin/sh', ['-eu', '-c', command], cwd);
const json = (args, cwd) => JSON.parse(rdm([...args, '--format', 'json'], cwd));
const srcPath = path.join(root, '.claude/workflows/rdm-wf-review-refute-fix.js');
const raw = fs.readFileSync(srcPath, 'utf8');
const code = raw.replace(/export const meta\s*=\s*\{[\s\S]*?\n\}/, '');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const driver = new AsyncFunction('args', 'agent', 'pipeline', 'parallel', 'log', code);
const parallel = async (thunks) => Promise.all(thunks.map(async (fn) => { try { return await fn(); } catch { return null; } }));
const pipeline = async (items, ...stages) => { let out = items; for (const stage of stages) out = await stage(out); return out; };
// Retain the survivors-only invocation shapes and dispatch hygiene contracts.
const legacyAgent = async (_prompt, options) => options.label === 'find:code:ac' ? { ac, findings: [] } : { findings: [] };
for (const mode of ['code', 'plan']) {
  const legacy = await driver({ mode, context: { target: 'legacy' } }, legacyAgent, pipeline, parallel, () => {});
  assert.equal(legacy.mode, mode); assert.ok(legacy.survivors.every((f) => f.severity === 'suggestion')); 
  assert.equal(legacy.outcome, undefined, 'legacy report does not approve');
}
await assert.rejects(() => driver({ mode: 'code', task: 'repair', roadmap: 'alpha', phase: 'phase-1-work' }, legacyAgent, pipeline, parallel, () => {}));
const region = raw.slice(raw.indexOf('// >>> review-refute-fix:end'));
assert.equal((region.match(/classifyOutcome\(/g) || []).length, 1);
assert.equal((region.match(/= *buildReviewPipeline\('code'\)/g) || []).length, 1);
assert.doesNotMatch(raw, /Done:|Date\.now\(|Math\.random\(/);
assert.equal(raw, fs.readFileSync(path.join(root, 'rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js'), 'utf8'));
try {
  fs.mkdirSync(sourceRepo);
  git(['init', '-b', 'main']);
  git(['commit', '--allow-empty', '-m', 'initial']);
  const base = git(['rev-parse', 'HEAD']);
  rdm(['init', '--default-project', 'verify']);
  rdm(['roadmap', 'create', 'alpha', '--title', 'Alpha', '--body', 'Goal: correct source', '--no-edit']);
  rdm(['phase', 'create', 'work', '--roadmap', 'alpha', '--number', '1', '--title', 'Work', '--body', '## Acceptance Criteria\n- works', '--no-edit']);
  rdm(['task', 'create', 'repair', '--title', 'Repair', '--body', '## Acceptance Criteria\n- works', '--no-edit', '--no-plan-review']);
  rdm(['plan', 'create', 'implementation', '--implements', 'phase/alpha/phase-1-work', '--title', 'Implementation', '--body', 'Implement works.', '--no-edit']);
  const planReview = json(['review', 'start', '--on', 'plan/implementation', '--author', 'independent', '--body', 'Independently reviewed.', '--no-edit']);
  rdm(['review', 'submit', planReview.id, '--verdict', 'approve', '--no-edit']);
  rdm(['plan', 'create', 'task-implementation', '--implements', 'task/repair', '--title', 'Task implementation', '--body', 'Implement repair.', '--no-edit']);
  const taskPlanReview = json(['review', 'start', '--on', 'plan/task-implementation', '--author', 'independent', '--body', 'Independently reviewed.', '--no-edit']);
  rdm(['review', 'submit', taskPlanReview.id, '--verdict', 'approve', '--no-edit']);
  rdm(['commit', '-m', 'chore(plan): fixture']);
  const shared = fs.realpathSync(rdm(['worktree', 'add', 'alpha']));
  const stale = fs.realpathSync(rdm(['worktree', 'add', 'alpha/phase-1-work']));
  fs.writeFileSync(path.join(shared, 'file with spaces.txt'), 'implemented\n');
  git(['add', '.'], shared); git(['commit', '-m', 'feat: implementation'], shared);
  const head = git(['rev-parse', 'HEAD'], shared);
  rdm(['config', 'set', 'gates.reviewed', 'true']);
  rdm(['commit', '-m', 'chore(plan): enable gate']);
  const defaultArgs = { mode: 'code', roadmap: 'alpha', phase: 'phase-1-work', rdmBin: binary, project: 'verify', base, implements: 'plan/implementation' };
  const finding = { id: 'issue', concern: 'correctness', severity: 'concern', confidence: 90, location: 'file with spaces.txt:1', what_fails: 'example', why: 'example', recommendation: 'fix' };
  async function execute(args = {}, options = {}) {
    const calls = [];
    let resolved;
    const agent = async (prompt, opts) => {
      calls.push({ prompt, ...opts });
      if (opts.label.startsWith('source:') && opts.label !== 'source:acceptance') {
        if (options.moveBeforeRevalidate && opts.label === 'source:revalidate') {
          git(['commit', '--allow-empty', '-m', 'external movement'], shared); options.moveBeforeRevalidate = false;
        }
        const command = prompt.split('\n').find((line) => line.includes(' review source --on '));
        resolved = JSON.parse(shell(command)); // ACTUAL resolver; no fabricated correct identity.
        if (options.wrongReturnedItem) resolved.item = 'task/other';
        return resolved;
      }
      if (opts.label === 'source:acceptance') {
        const command = prompt.split('\n').find((line) => line.includes(' show '));
        return { acceptance: JSON.parse(shell(command)).body };
      }
      if (opts.label.startsWith('find:')) {
        assert.ok(prompt.includes(shared), 'finder reads the resolved shared checkout');
        assert.ok(prompt.includes(head), 'finder reads pinned head');
        assert.ok(prompt.includes('Acceptance criteria:'), 'criteria threaded');
        if (options.failDimension && opts.label.startsWith('find:code:' + options.failDimension)) return null;
        if (opts.label === 'find:code:ac') return options.invalidAc ? { ac: [{ status: 'INVALID' }] } : { ac: [{ ...ac[0], status: options.acFail ? 'FAIL' : 'PASS' }], findings: [] };
        return { findings: opts.label === 'find:code:correctness' ? (options.findings || []) : [] };
      }
      if (opts.label.startsWith('refute:')) {
        assert.ok(prompt.includes(shared) && prompt.includes(head), 'refuter pinned too');
        if (options.refuterCrash) throw new Error('injected refuter crash');
        return { refuted: false, confidence: 90, reason: 'verified' };
      }
      if (opts.label === 'persist:review') {
        if (options.persistFailure) return { ok: false };
        assert.ok(!prompt.includes('worktree add'), 'persistence never reselects or creates checkout');
        assert.ok(!prompt.includes('--on phase/'), 'no item approval fallback');
        const start = prompt.indexOf('Run these commands IN ORDER');
        const command = prompt.slice(prompt.indexOf('\n', start) + 1, prompt.indexOf('\nANCHORING FALLBACK'));
        const output = shell(command + '\nprintf "\\n%s\\n" "$RDM_REVIEW_ID"');
        return { ok: true, reviewId: output.split('\n').at(-1), anchored: 0, wholeDocument: 0 };
      }
      if (opts.label === 'gate:persist') {
        if (options.gateFailure) return { ok: false, head, branch: resolved.branch, status: 'needs-review' };
        const lines = prompt.split('\n').filter((line) => line.startsWith(binary + ' '));
        let stamped = false;
        let status;
        for (const line of lines) {
          const output = shell(line, shared);
          if (line.includes(' review pending ')) {
            const pending = JSON.parse(output);
            assert.ok(pending.some((p) => p.review_sha === head && p.branch === resolved.branch)); stamped = true;
          }
          if (line.includes(' show ')) status = JSON.parse(output).status;
        }
        assert.ok(stamped);
        return { ok: true, head: resolved.head, branch: resolved.branch, status };
      }
      throw new Error('unexpected agent ' + opts.label);
    };
    const result = await driver({ ...defaultArgs, ...args }, agent, pipeline, parallel, () => {});
    return { result, calls };
  }
  // Wrong/empty caller diff cannot bypass the real resolver.
  const clean = await execute({ diff: { changedFiles: [], diffText: 'fake' }, persist: true, gate: true });
  assert.equal(clean.result.outcome, 'reviewed', JSON.stringify(clean.result));
  assert.equal(clean.result.source.path, shared);
  assert.equal(clean.result.source.head, head);
  assert.equal(clean.result.source.base, base);
  assert.ok(clean.calls.some((call) => call.label === 'source:resolve'));
  const persisted = json(['review', 'show', clean.result.reviewId]);
  assert.equal(persisted.target.head, head);
  assert.equal(persisted.target.base, base);
  assert.equal(persisted.implements, 'rdm:plan/implementation');
  assert.equal(json(['phase', 'show', 'phase-1-work', '--roadmap', 'alpha']).status, 'reviewed');
  assert.equal(git(['rev-parse', 'HEAD'], stale), base, 'stale branch unchanged');
  assert.equal(git(['status', '--porcelain'], stale), '', 'stale checkout unchanged');
  assert.equal(json(['worktree', 'list']).length, 2, 'no task checkout created');
  const task = await execute({ roadmap: undefined, phase: undefined, task: 'repair', source: shared, expectedHead: head, persist: true, gate: true, implements: 'plan/task-implementation' });
  assert.equal(task.result.outcome, 'reviewed', JSON.stringify(task.result));
  assert.equal(json(['worktree', 'list']).length, 2);
  for (const options of [{ failDimension: 'ac' }, { failDimension: 'correctness' }, { invalidAc: true }, { findings: [finding], refuterCrash: true }]) {
    const { result, calls } = await execute({ persist: true }, options);
    assert.equal(result.outcome, 'escalated'); assert.equal(result.writesCompletion, false);
    assert.ok(!calls.filter((c) => c.label === 'persist:review').some((c) => c.prompt.includes('--verdict approve')));
  }
  const overflow = await execute({ maxRefutations: 0, tier: 'small' }, { findings: [finding] });
  assert.equal(overflow.result.outcome, 'escalated', 'ungraded concern cannot approve even at small tier');
  assert.equal(overflow.result.reviewBudget.passedThroughBudget, 1);
  const suggestion = await execute({ findModel: 'finder-model', verifyModel: 'refuter-model' }, { findings: [{ ...finding, severity: 'suggestion' }] });
  assert.equal(suggestion.result.outcome, 'reviewed', 'intentional non-gating suggestion remains non-gating');
  assert.ok(suggestion.calls.filter((c) => c.label.startsWith('find:')).every((c) => c.model === 'finder-model'));
  const modeled = await execute({ findModel: 'finder-model', verifyModel: 'refuter-model' }, { findings: [finding] });
  assert.ok(modeled.calls.filter((c) => c.label.startsWith('refute:')).every((c) => c.model === 'refuter-model'));
  assert.ok(!modeled.calls.some((c) => c.label === 'persist:review' || c.label === 'gate:persist'), 'mutations default off');
  for (const project of ['bad name', 'bad;command', 42]) await assert.rejects(() => execute({ project }));
  await assert.rejects(() => execute({ rdmBin: 42 }));
  assert.equal((await execute({ persist: { on: 'plan/implementation' } })).result.outcome, 'escalated', 'explicit wrong persistence target cannot approve');
  const rework = await execute({ gate: true }, { acFail: true });
  assert.equal(rework.result.outcome, 'rework', JSON.stringify(rework.result));
  assert.equal(json(['phase', 'show', 'phase-1-work', '--roadmap', 'alpha']).status, 'in-progress');

  assert.equal((await execute({}, { acFail: true })).result.outcome, 'rework');
  assert.equal((await execute({ gate: true }, { gateFailure: true })).result.outcome, 'escalated');
  assert.equal((await execute({ persist: true }, { persistFailure: true })).result.outcome, 'escalated');
  assert.equal((await execute({}, { wrongReturnedItem: true })).result.outcome, 'escalated');
  assert.equal((await execute({ base: head })).result.outcome, 'escalated');
  assert.equal((await execute({ base: head, noCode: true })).result.outcome, 'reviewed');
  assert.equal((await execute({ expectedHead: base })).result.outcome, 'escalated');
  assert.equal((await execute({ source: stale })).result.outcome, 'escalated');
  assert.equal((await execute({ source: temp })).result.outcome, 'escalated');
  assert.equal((await execute({}, { moveBeforeRevalidate: true })).result.outcome, 'escalated');
  const moved = git(['rev-parse', 'HEAD'], shared);
  assert.throws(() => rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--status', 'reviewed', '--source', shared, '--base', base, '--expected-head', head, '--no-edit'], shared), /HEAD moved/);
  // Old approval cannot release the newly observed head (even without expected-head).
  assert.throws(() => rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--status', 'reviewed', '--source', shared, '--base', base, '--expected-head', moved, '--no-edit'], shared));
  console.log('real standalone source, persistence, status and incomplete evidence regressions passed');
} finally { fs.rmSync(temp, { recursive: true, force: true }); }
