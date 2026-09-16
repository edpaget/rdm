import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { runEstimateExperiment } from './codex-spike-estimate.mjs';

const sourceDir = fileURLToPath(new URL('../..', import.meta.url));
const rdmBin = path.join(sourceDir, 'scripts/rdm-dev.sh');
const rating = (_, { label }) => ({ stem: label.split(':').at(-1), difficulty: 'moderate', justification: 'A bounded fixture change; literal `code` and $text.' });

test('real CLI estimates only unset phases, derives tiers, and skips every write and agent on rerun', async () => {
  const calls = [];
  const evidence = await runEstimateExperiment({ rdmBin, sourceDir, agent: async (prompt, opts) => {
    calls.push(opts.label);
    assert.match(prompt, /--- PHASE BODY ---/);
    assert.match(prompt, /Return JSON/);
    return rating(prompt, opts);
  } });
  assert.deepEqual(calls.sort(), ['estimate:rate:phase-1-a', 'estimate:rate:phase-3-c']);
  assert.equal(evidence.first.estimated.length, 2);
  assert.deepEqual(evidence.counters.first, { agentCalls: 2, writes: 2 });
  assert.deepEqual(evidence.first.skipped, ['phase-2-b']);
  for (const item of evidence.first.estimated) assert.equal(item.tier, 'medium');
  assert.deepEqual(evidence.second.estimated, []);
  assert.deepEqual(evidence.counters.second, { agentCalls: 0, writes: 0 });
  assert.equal(evidence.idempotent, true);
  assert.deepEqual(evidence.before.find(p => p.stem === 'phase-2-b'), evidence.after.find(p => p.stem === 'phase-2-b'));
});

for (const [name, mutate] of [
  ['missing target', r => ({ ...r, stem: undefined })],
  ['unknown target', r => ({ ...r, stem: 'phase-999-foreign' })],
  ['wrong known target', r => ({ ...r, stem: 'phase-2-b' })],
  ['missing result', () => null],
  ['invalid difficulty', r => ({ ...r, difficulty: 'extreme' })],
  ['empty justification', r => ({ ...r, justification: '  ' })],
  ['multiline justification', r => ({ ...r, justification: 'first\nsecond' })],
]) test(`rejects ${name} before writing any estimate`, async () => {
  const evidenceDir = await mkdtemp(path.join(tmpdir(), 'codex-estimate-test-'));
  try {
    let identity;
    await assert.rejects(runEstimateExperiment({ rdmBin, sourceDir, evidenceDir,
      agent: async (prompt, opts) => opts.label.endsWith('phase-3-c') ? mutate(rating(prompt, opts)) : rating(prompt, opts),
    }), error => {
      assert.match(error.message, /invalid estimate/);
      identity = error.fixtureIdentity;
      return true;
    });
    assert.ok(identity?.session, 'failed experiments expose the fixture session for overlay-aware audit');
    const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
      !key.startsWith('RDM_') && !key.startsWith('GIT_')));
    Object.assign(env, { RDM_ROOT: identity.root, RDM_PROJECT: identity.project,
      RDM_SESSION: identity.session, RDM_BIN: identity.rdmBin,
      GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null' });
    const cli = args => execFileSync(identity.rdmBin, ['--root', identity.root, ...args], {
      cwd: identity.sourceDir, env, encoding: 'utf8', timeout: 180_000, stdio: ['ignore', 'pipe', 'pipe'],
    });
    const scope = ['--roadmap', identity.roadmap, '--project', identity.project];
    // A filesystem read would miss writes staged in this session's overlay.
    const assertUnset = stem => {
      const phase = JSON.parse(cli(['phase', 'show', stem, ...scope, '--format', 'json']));
      assert.equal(phase.difficulty, undefined, `${stem} must remain unset in the failed run's session`);
      assert.equal(phase.model, undefined);
      assert.doesNotMatch(phase.body, /## Estimate/);
    };
    for (const stem of ['phase-1-a', 'phase-3-c']) assertUnset(stem);
    if (name === 'missing target') {
      // Prove this check detects an uncommitted partial write in that session.
      cli(['phase', 'update', 'phase-1-a', '--difficulty', 'moderate', '--no-edit', ...scope]);
      assert.throws(() => assertUnset('phase-1-a'), /must remain unset/);
    }
  } finally { await rm(evidenceDir, { recursive: true, force: true }); }
});
