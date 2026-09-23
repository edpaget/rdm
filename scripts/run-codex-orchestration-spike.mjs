// Opt-in research runner, not a supported dispatch runtime. Uses saved Codex
// authentication in place. All judgment children use the read-only sandbox.
import {mkdtemp, mkdir, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve, dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {runCodex, boundedParallel} from './lib/codex-spike-process.mjs';
import {runReviewExperiment} from './lib/codex-spike-review.mjs';
import {runEstimateExperiment} from './lib/codex-spike-estimate.mjs';

const sourceDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const [model, effort = 'medium'] = process.argv.slice(2);
if (!model || process.argv.length > 4) throw new Error('Usage: node scripts/run-codex-orchestration-spike.mjs MODEL [EFFORT]');
const evidenceDir = await mkdtemp(join(tmpdir(), 'rdm-codex-spike-'));
console.log(`Evidence: ${evidenceDir}`);
const bin = process.env.RDM_CODEX_BIN || 'codex';
const rdmBin = join(sourceDir, 'scripts/rdm-dev.sh');
const env = {...process.env, GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_AUTHOR_NAME: 'Spike fixture', GIT_AUTHOR_EMAIL: 'spike@example.invalid',
  GIT_COMMITTER_NAME: 'Spike fixture', GIT_COMMITTER_EMAIL: 'spike@example.invalid'};
for (const key of Object.keys(env)) if (/^GIT_(DIR|WORK_TREE|INDEX_FILE|CONFIG_COUNT|CONFIG_KEY_|CONFIG_VALUE_)/.test(key)) delete env[key];
const fixture = join(evidenceDir, 'source');
await mkdir(fixture);
const git = (...args) => execFileSync('git', args, {cwd: fixture, env, encoding: 'utf8'}).trim();
git('init', '-q', '-b', 'main');
await writeFile(join(fixture, 'AGENTS.md'), '# Fixture conventions\nPure arithmetic helpers. Tests use node:test. No external dependencies.\n');
await writeFile(join(fixture, 'sum.mjs'), 'export function add(a, b) { return a + b; }\n');
await writeFile(join(fixture, 'sum.test.mjs'), "import test from 'node:test';\nimport assert from 'node:assert/strict';\nimport {add} from './sum.mjs';\ntest('zero', () => assert.equal(add(0, 0), 0));\n");
git('add', '.'); git('-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'test: seed arithmetic fixture');
const base = git('rev-parse', 'HEAD');
await writeFile(join(fixture, 'sum.mjs'), 'export function add(a, b) { return a - b; }\n');
git('add', 'sum.mjs'); git('-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'fix: planted arithmetic regression');
const head = git('rev-parse', 'HEAD');
const diff = git('diff', `${base}..${head}`);
const planFile = join(evidenceDir, 'implementation-plan.md');
await writeFile(planFile, 'Implement sum.mjs add(a,b) using addition; first add a failing test asserting add(2,3) === 5, then implement and run node --test. Keep the existing API and add no dependencies.\n');
const calls = [];
let sequence = 0;
let active = 0;
let peak = 0;
const controller = new AbortController();
const interrupt = () => controller.abort();
process.on('SIGINT', interrupt);
process.on('SIGTERM', interrupt);
const agent = async (prompt, options) => {
  const index = sequence++;
  const label = options.label;
  const started = Date.now();
  active++; peak = Math.max(peak, active);
  console.log(`Starting ${label}`);
  try {
    const result = await runCodex({bin, cwd: fixture, prompt, schema: options.schema,
      model, effort, evidenceDir: join(evidenceDir, `call-${index}`), label,
      signal: controller.signal, timeoutMs: 180000});
    calls.push({index, label, started, finished: Date.now(), threadId: result.threadId,
      usage: result.usage, elapsedMs: result.elapsedMs, model, effort, outcome: 'completed'});
    console.log(`Completed ${label} (${result.elapsedMs} ms)`);
    return result.value;
  } catch (error) {
    calls.push({index, label, started, finished: Date.now(), outcome: 'failed', error: error.message});
    console.log(`Failed ${label}: ${error.message}`);
    throw error;
  } finally { active--; }
};
const report = {
  experiment: 'canonical Codex execution spike', model, effort,
  sourceHead: execFileSync('git', ['rev-parse', 'HEAD'], {cwd: sourceDir, encoding: 'utf8'}).trim(),
  codexVersion: execFileSync(bin, ['--version'], {encoding: 'utf8'}).trim(),
  fixture: {base, head, diff, diffSha256: createHash('sha256').update(diff).digest('hex')},
  evidenceDir, calls, billedCost: 'not exposed by CLI', contextWindowOccupancy: 'not exposed by CLI',
};
try {
  report.review = await runReviewExperiment({agent, model, planFile, parallel: thunks => boundedParallel(thunks, 2),
    target: `Review only ${base}..${head} in ${fixture}. Acceptance criterion: add(2,3) returns 5. Diff:\n${diff}`});
  assert.equal(report.review.codeOutcome, 'rework', 'planted regression must prevent approval');
  assert(report.review.code.budget.graded > 0, 'live run must exercise a refuter');
  assert.equal(report.review.plan.outcome, 'reviewed', 'simple implementation plan should pass');
  const reviewCalls = calls.slice();
  const finders = reviewCalls.filter(c => c.label.startsWith('find:code:'));
  const refuters = reviewCalls.filter(c => c.label.startsWith('refute:code:'));
  assert(refuters.every(r => finders.every(f => f.finished <= r.started)), 'finder barrier');
  report.estimate = await runEstimateExperiment({rdmBin, sourceDir, evidenceDir: join(evidenceDir, 'estimate'), agent});
  const ids = calls.filter(c => c.outcome === 'completed').map(c => c.threadId);
  assert.equal(new Set(ids).size, ids.length, 'independent judgment threads must be fresh');
  assert.equal(git('status', '--porcelain'), '', 'review children must not modify fixture');
  report.success = true;
} catch (error) {
  report.success = false;
  report.error = error.message;
  process.exitCode = 1;
} finally {
  report.peakConcurrency = peak;
  report.finishedAt = new Date().toISOString();
  report.calls.sort((a, b) => a.index - b.index);
  await writeFile(join(evidenceDir, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  process.removeListener('SIGINT', interrupt);
  process.removeListener('SIGTERM', interrupt);
  console.log(`Result: ${join(evidenceDir, 'report.json')} (success=${report.success})`);
}
