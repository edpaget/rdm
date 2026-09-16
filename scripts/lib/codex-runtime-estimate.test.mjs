import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { runEstimate } from './codex-runtime-estimate.mjs';
import { createRun } from './codex-runtime-state.mjs';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const sourceDir = fileURLToPath(new URL('../../', import.meta.url));
const fixtureRoots = [];
test.after(() => { for (const root of fixtureRoots) rmSync(root, { recursive: true, force: true }); });
function fixture() {
  const planRoot = realpathSync(mkdtempSync(path.join(tmpdir(), 'estimate-unit-plan-'))); fixtureRoots.push(planRoot);
  const git = (...args) => execFileSync('git', args, { cwd: planRoot, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid');
  const persist = () => { writeFileSync(path.join(planRoot, 'phases.json'), JSON.stringify(Object.fromEntries(phases))); git('add', 'phases.json'); git('commit', '-qm', 'test: persist fixture snapshot'); };
  const phases = new Map(['phase-1-a', 'phase-2-b'].map(stem => [stem, { stem, body: 'Preserve `$body`.', tags: ['keep'], difficulty: null, model: null, estimate_snapshot: `snapshot-${stem}` }]));
  persist();
  const journal = new Set();
  const calls = [];
  const ctx = { identity: { sourceDir, planRoot, project: 'fixture', rdmBin: '/fixture/rdm' }, session: 'owned', record: async () => {},
    rdm: async (args, options) => {
      calls.push({ args, options });
      if (args[0] === 'commit') { persist(); journal.clear(); return 'committed'; }
      if (args[0] === 'session' && args[1] === 'journal') return { id: 'owned', paths: [...journal] };
      if (args[1] === 'list') return [...phases.values()].map(p => structuredClone(p));
      const p = phases.get(args[2]);
      if (args[1] === 'show') {
        if (args.includes('--at')) {
          const at = args[args.indexOf('--at') + 1]; assert.match(at, /^[a-f0-9]{40}$/);
          return JSON.parse(git('show', `${at}:phases.json`))[args[2]];
        }
        return structuredClone(p);
      }
      if (args[1] === 'update') {
        assert.ok(args.includes('--expected-estimate-snapshot'), 'conditional estimate write is required');
        if (args[args.indexOf('--expected-estimate-snapshot') + 1] !== p.estimate_snapshot || p.difficulty || p.model) throw new Error('estimate snapshot changed');
        p.body = args[args.indexOf('--body') + 1]; p.difficulty = args[args.indexOf('--difficulty') + 1]; p.model = 'sonnet'; journal.add(args[2]); return '';
      }
      throw new Error('unexpected operation');
    } };
  const agent = async prompt => ({ stem: prompt.match(/Phase stem: (\S+)/)[1], difficulty: 'moderate', justification: 'Small bounded change.' });
  return { ctx, calls, phases, agent, journal };
}

test('preview validates proposals without any writes or commits', async () => {
  const f = fixture(); const result = await runEstimate({ ...f, roadmap: 'example' });
  assert.equal(result.proposed.length, 2); assert.equal(result.applied, false);
  assert.equal(f.calls.filter(c => c.options?.mutating).length, 0);
});
test('apply preserves tags/body, writes only difficulty, scoped commit, second pass no-op', async () => {
  const f = fixture(); const result = await runEstimate({ ...f, roadmap: 'example', apply: true });
  assert.equal(result.estimated.length, 2);
  assert.equal(result.estimated[0].tier, 'sonnet');
  assert.deepEqual(f.phases.get('phase-1-a').tags, ['keep']);
  assert.match(f.phases.get('phase-1-a').body, /^Preserve `\$body`\./);
  assert.ok(f.calls.every(c => !c.args.includes('--model') && !c.args.includes('--all')));
  assert.equal(f.calls.filter(c => c.args[0] === 'commit').length, 1);
  const count = f.calls.filter(c => c.options?.mutating).length;
  await runEstimate({ ...f, roadmap: 'example', apply: true });
  assert.equal(f.calls.filter(c => c.options?.mutating).length, count);
});
test('invalid batch is rejected before any mutation', async () => {
  const f = fixture();
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true, agent: async prompt => ({ ...await f.agent(prompt), justification: 'two\nlines' }) }), /invalid estimate/);
  assert.equal(f.calls.filter(c => c.options?.mutating).length, 0);
});
test('concurrent body/tag changes are detected and never overwritten', async () => {
  const f = fixture();
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true, agent: async prompt => { f.phases.get('phase-1-a').tags.push('concurrent'); return f.agent(prompt); } }), /changed/);
  assert.equal(f.calls.filter(c => c.options?.mutating).length, 0);
});
test('wrong target and rejected judgment both fail the complete batch before writing', async () => {
  for (const badAgent of [async () => ({ stem: 'another-phase', difficulty: 'easy', justification: 'Wrong identity.' }), async () => { throw new Error('agent failed'); }]) {
    const f = fixture();
    await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true, agent: badAgent }), /invalid estimate|agent failed/);
    assert.equal(f.calls.filter(c => c.options?.mutating).length, 0);
  }
});
test('a post-write readback mismatch stops further writes and is uncertain', async () => {
  const f = fixture(); const rdm = f.ctx.rdm;
  f.ctx.rdm = async (args, opts) => { const result = await rdm(args, opts); if (args[1] === 'update') f.phases.get(args[2]).body = 'Concurrent edit after write'; return result; };
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true }), /uncertain.*readback/);
  assert.equal(f.calls.filter(c => c.options?.mutating).length, 1);
});
test('atomic precondition rejects edit after final read without overwriting it', async () => {
  const f = fixture(); const rdm = f.ctx.rdm;
  f.ctx.rdm = async (args, opts) => {
    if (args[1] === 'update') { const p = f.phases.get(args[2]); p.body = 'Concurrent writer wins'; p.estimate_snapshot = 'changed-snapshot'; }
    return rdm(args, opts);
  };
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true }), /snapshot changed/);
  assert.equal(f.phases.get('phase-1-a').body, 'Concurrent writer wins');
  assert.equal(f.phases.get('phase-1-a').difficulty, null);
  assert.equal(f.calls.filter(c => c.args[0] === 'commit').length, 0);
});
test('apply rejects older RDM output without a conditional estimate snapshot', async () => {
  const f = fixture(); for (const phase of f.phases.values()) delete phase.estimate_snapshot;
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true }), /snapshot missing/);
  assert.equal(f.calls.filter(c => c.options?.mutating).length, 0);
});
test('zero-exit commit that skips an estimate never reports applied success', async () => {
  const f = fixture(); const rdm = f.ctx.rdm;
  f.ctx.rdm = async (args, opts) => args[0] === 'commit' ? 'Warning: skipped missing path; nothing committed.' : rdm(args, opts);
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true }), /uncertain.*committed estimate/);
});
test('owned journal must settle even when committed estimates match', async () => {
  const f = fixture(); const rdm = f.ctx.rdm;
  f.ctx.rdm = async (args, opts) => { const value = await rdm(args, opts); if (args[0] === 'commit') f.journal.add('unsettled-target'); return value; };
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true }), /uncertain.*journal/);
});
for (const interrupted of ['update', 'commit']) test(`interruption after ${interrupted} is uncertain, stops without retry or success`, async () => {
  const f = fixture(); const rdm = f.ctx.rdm;
  f.ctx.rdm = async (args, opts) => { const result = await rdm(args, opts); if (args.includes(interrupted)) throw new Error('interrupted'); return result; };
  await assert.rejects(runEstimate({ ...f, roadmap: 'example', apply: true }), /uncertain/);
  assert.equal(f.calls.filter(c => c.args.includes(interrupted)).length, 1);
});

test('real plan repository: preview, scoped apply beside another session, and idempotence', { timeout: 180_000 }, async () => {
  const root = realpathSync(mkdtempSync(path.join(tmpdir(), 'codex-estimate-runtime-')));
  const planRoot = path.join(root, 'plans'); const source = path.join(root, 'source');
  mkdirSync(planRoot); mkdirSync(source);
  const rdmBin = path.join(sourceDir, 'scripts/rdm-dev.sh');
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('RDM_') && !key.startsWith('GIT_')));
  Object.assign(env, { RDM_ROOT: planRoot, RDM_PROJECT: 'fixture', RDM_SESSION: 'seed', RDM_BIN: rdmBin,
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_AUTHOR_NAME: 'Test', GIT_AUTHOR_EMAIL: 'test@example.invalid', GIT_COMMITTER_NAME: 'Test', GIT_COMMITTER_EMAIL: 'test@example.invalid' });
  const exec = (bin, args, cwd = source, session = 'seed') => execFileSync(bin, args, { cwd, env: { ...env, RDM_SESSION: session }, encoding: 'utf8', timeout: 120_000, stdio: ['ignore', 'pipe', 'pipe'] });
  const rdm = (args, session) => exec(rdmBin, args, source, session);
  const scope = ['--project', 'fixture', '--roadmap', 'example'];
  const show = (stem, session) => JSON.parse(rdm(['phase', 'show', stem, ...scope, '--format', 'json'], session));
  try {
    exec('git', ['init', '-b', 'main']);
    exec('git', ['config', 'user.name', 'Test']); exec('git', ['config', 'user.email', 'test@example.invalid']);
    writeFileSync(path.join(source, 'README'), 'fixture'); exec('git', ['add', 'README']); exec('git', ['commit', '-m', 'test: seed source']);
    rdm(['init', '--default-project', 'fixture']);
    exec('git', ['config', 'user.name', 'Test'], planRoot); exec('git', ['config', 'user.email', 'test@example.invalid'], planRoot);
    rdm(['roadmap', 'create', 'example', '--title', 'Example', '--body', 'Fixture', '--no-edit', '--project', 'fixture']);
    for (const [number, slug] of [[1, 'target'], [2, 'other']]) rdm(['phase', 'create', slug, '--number', String(number), '--title', slug, '--body', 'Preserve body.', '--no-edit', ...scope]);
    rdm(['phase', 'update', 'phase-2-other', '--difficulty', 'hard', '--no-edit', ...scope]);
    rdm(['commit', '-m', 'test: seed plans']);
    rdm(['phase', 'update', 'phase-2-other', '--body', 'Unrelated pending change.', '--no-edit', ...scope], 'unrelated');
    const seedHead = exec('git', ['rev-parse', 'HEAD'], planRoot).trim();
    const makeRun = name => createRun({ sourceDir: source, planRoot, rdmBin, project: 'fixture', session: 'parent', operation: 'estimate', runDir: path.join(root, name) });
    const agent = fixture().agent;
    const preview = makeRun('preview');
    const result = await runEstimate({ ctx: preview, roadmap: 'example', agent }); preview.finish(result);
    assert.equal(result.proposed.length, 1); assert.equal(exec('git', ['rev-parse', 'HEAD'], planRoot).trim(), seedHead);
    const apply = makeRun('apply');
    const applied = await runEstimate({ ctx: apply, roadmap: 'example', apply: true, agent }); apply.finish(applied);
    assert.equal(applied.estimated.length, 1); assert.equal(show('phase-1-target').difficulty, 'moderate');
    const committedOther = exec('git', ['show', 'HEAD:projects/fixture/roadmaps/example/phase-2-other.md'], planRoot);
    assert.match(committedOther, /Preserve body\./); assert.doesNotMatch(committedOther, /Unrelated pending change/);
    assert.equal(show('phase-2-other', 'unrelated').body.trim(), 'Unrelated pending change.');
    const applyHead = exec('git', ['rev-parse', 'HEAD'], planRoot).trim(); assert.notEqual(applyHead, seedHead);
    const second = makeRun('second');
    const again = await runEstimate({ ctx: second, roadmap: 'example', apply: true, agent: () => { throw new Error('must not rate already-set targets'); } }); second.finish(again);
    assert.deepEqual(again.estimated, []); assert.equal(exec('git', ['rev-parse', 'HEAD'], planRoot).trim(), applyHead);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
