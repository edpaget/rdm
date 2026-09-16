// Experimental host adapter: judgment uses an agent; every plan operation uses
// argv-based CLI calls against a newly allocated fixture repository.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const ratingSchema = {
  type: 'object', additionalProperties: false,
  properties: {
    stem: { type: 'string' },
    difficulty: { type: 'string', enum: ['trivial', 'easy', 'moderate', 'hard'] },
    justification: { type: 'string' },
  },
  required: ['stem', 'difficulty', 'justification'],
};

/** Run canonical estimate twice in an isolated git-backed fixture.
 * Retain that repository under evidenceDir when supplied; otherwise remove it.
 * Throws on malformed judgment, incomplete writes, or failed idempotence.
 */
export async function runEstimateExperiment({ rdmBin, sourceDir, evidenceDir, agent }) {
  if (!path.isAbsolute(rdmBin) || !path.isAbsolute(sourceDir) || typeof agent !== 'function') {
    throw new Error('estimate experiment requires absolute binary/source paths and an agent');
  }
  const { buildEstimatePipeline, buildEstimatorPrompt } = await import(
    pathToFileURL(path.join(sourceDir, '.claude/workflows/lib/estimate.mjs')).href
  );
  const parent = evidenceDir ? path.resolve(evidenceDir) : tmpdir();
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(path.join(parent, 'estimate-fixture-'));
  const session = `codex-estimate-${randomUUID()}`;
  const project = 'fixture';
  const roadmap = 'estimate-fixture';
  // Do not inherit alternate plan-repo settings or git repository overrides.
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    !key.startsWith('RDM_') && !key.startsWith('GIT_')));
  Object.assign(env, { RDM_ROOT: root, RDM_PROJECT: project, RDM_SESSION: session, RDM_BIN: rdmBin,
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_AUTHOR_NAME: 'Codex spike fixture', GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
    GIT_COMMITTER_NAME: 'Codex spike fixture', GIT_COMMITTER_EMAIL: 'fixture@example.invalid' });
  const commandLog = [];
  const logs = [];
  const counters = { first: { agentCalls: 0, writes: 0 }, second: { agentCalls: 0, writes: 0 } };
  let pass = 'seed';
  const command = args => {
    commandLog.push({ pass, argv: ['--root', root, ...args] });
    return execFileSync(rdmBin, ['--root', root, ...args], {
      cwd: sourceDir, env, encoding: 'utf8', maxBuffer: 4 * 1024 * 1024,
      timeout: 180_000, stdio: ['ignore', 'pipe', 'pipe'],
    });
  };
  const scope = ['--roadmap', roadmap, '--project', project];
  const json = args => JSON.parse(command([...args, '--format', 'json']));
  const list = () => json(['phase', 'list', ...scope]);
  const show = stem => json(['phase', 'show', stem, ...scope]);
  const snapshot = () => list().map(p => ({ stem: p.stem, ...show(p.stem) }));
  try {
    command(['init', '--default-project', project]);
    command(['roadmap', 'create', roadmap, '--title', 'Estimate fixture', '--body', 'Isolated estimate experiment.', '--no-edit', '--project', project]);
    for (const [number, slug, body] of [
      [1, 'a', 'Add an optional text field to a local record and verify serialization round trips.'],
      [2, 'b', 'Pre-estimated control: preserve this exact phase without modification.'],
      [3, 'c', 'Correct the help text for one existing CLI option and check its rendered output.'],
    ]) command(['phase', 'create', slug, '--title', `Fixture ${slug}`, '--number', String(number), '--body', body, '--no-edit', ...scope]);
    command(['phase', 'update', 'phase-2-b', '--difficulty', 'hard', '--no-edit', ...scope]);
    command(['commit', '-m', 'test: seed isolated estimate fixture']);
    const revision = cwd => execFileSync('git', ['rev-parse', 'HEAD'], {
      cwd, env, encoding: 'utf8', timeout: 10_000, stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    const sourceHead = revision(sourceDir);
    const fixtureSeedHead = revision(root);
    const before = snapshot();
    let failure;
    const capture = fn => async (...args) => {
      try { return await fn(...args); } catch (error) { failure ??= error; throw error; }
    };
    const run = buildEstimatePipeline({
      list: async () => list(),
      log: message => logs.push(message),
      parallelRate: capture(async stems => {
        const results = await Promise.allSettled(stems.map(async stem => {
          counters[pass].agentCalls++;
          return agent(buildEstimatorPrompt(`Phase stem: ${stem}\n${show(stem).body}`), {
            label: `estimate:rate:${stem}`, schema: ratingSchema,
          });
        }));
        // Validate the entire batch before returning any item to writeback.
        return results.map((result, index) => {
          if (result.status === 'rejected') throw result.reason;
          const r = result.value;
          if (!r || typeof r !== 'object' || Array.isArray(r) ||
              Object.keys(r).sort().join(',') !== 'difficulty,justification,stem' ||
              r.stem !== stems[index] || !ratingSchema.properties.difficulty.enum.includes(r.difficulty) ||
              typeof r.justification !== 'string' || !r.justification.trim() || /[\r\n]/.test(r.justification)) {
            throw new Error(`invalid estimate for ${stems[index]}: target, difficulty, and one-line justification required`);
          }
          return r;
        });
      }),
      writeback: capture(async (stem, difficulty, justification) => {
        const current = show(stem);
        if (current.difficulty || current.model) throw new Error(`estimate target is already set: ${stem}`);
        const body = `${current.body || ''}\n\n## Estimate\n\n${difficulty} — ${justification}\n`;
        counters[pass].writes++;
        command(['phase', 'update', stem, '--difficulty', difficulty, '--body', body, '--no-edit', ...scope]);
        const after = show(stem);
        assert.equal(after.difficulty, difficulty);
        assert.equal(after.body.trimEnd(), body.trimEnd());
        return { ok: true };
      }),
      showTier: capture(async stem => {
        const tier = show(stem).model;
        assert.ok(typeof tier === 'string' && tier, 'core tier must be returned');
        return tier;
      }),
    });
    pass = 'first';
    const first = await run({ roadmap });
    if (failure) throw failure;
    assert.equal(first.estimated.length, 2, 'every unset fixture must be estimated');
    const after = snapshot();
    pass = 'second';
    const second = await run({ roadmap });
    if (failure) throw failure;
    assert.deepEqual(counters.second, { agentCalls: 0, writes: 0 });
    assert.deepEqual(second.estimated, []);
    assert.deepEqual(snapshot(), after);
    assert.deepEqual(before.find(p => p.stem === 'phase-2-b'), after.find(p => p.stem === 'phase-2-b'));
    return { identity: { rdmBin, sourceDir, sourceHead, root, fixtureSeedHead, session, project, roadmap }, before, first, second, after,
      counters, commandLog, logs, idempotent: true };
  } catch (error) {
    const failure = new Error(error instanceof Error ? error.message : String(error), { cause: error });
    // This identity contains no credentials and lets retained failed fixtures
    // be inspected through the same session overlay that performed the run.
    failure.fixtureIdentity = { rdmBin, sourceDir, root, session, project, roadmap };
    throw failure;
  } finally {
    if (!evidenceDir) await rm(root, { recursive: true, force: true });
  }
}
