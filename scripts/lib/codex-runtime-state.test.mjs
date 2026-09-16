import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { createRun } from './codex-runtime-state.mjs';

function fixture(t) {
  const root = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'codex-state-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const name of ['source', 'plan']) {
    const dir = path.join(root, name); fs.mkdirSync(dir);
    const git = (...args) => execFileSync('git', ['-C', dir, ...args], { stdio: 'pipe' });
    git('init', '-b', 'main'); git('config', 'user.email', 'fixture@example.test'); git('config', 'user.name', 'Fixture');
    fs.writeFileSync(path.join(dir, 'seed'), 'seed'); git('add', 'seed'); git('commit', '-m', 'seed');
  }
  const binary = path.join(root, 'fake-rdm');
  fs.writeFileSync(binary, `#!/usr/bin/env node
import fs from 'node:fs';
import {spawn} from 'node:child_process';
const args = process.argv.slice(2);
if (args[0] === 'fail') { fs.writeFileSync(process.env.RDM_ROOT + '/effect', 'written'); process.exit(1); }
if (args[0] === 'descendant' || args[0] === 'orphan') {
 const child = spawn(process.execPath, ['-e', 'setTimeout(()=>require("fs").writeFileSync(process.env.RDM_ROOT+"/late-effect","bad"),500)'], {stdio:'inherit'});
 fs.writeFileSync(process.env.RDM_ROOT+'/child-pid', String(child.pid));
 if (args[0] === 'orphan') process.exit(0);
 setTimeout(()=>{},10000);
}
if (args[0] === 'overflow') process.stdout.write('x'.repeat(9*1024*1024));
if (args[0] === 'sleep') { setTimeout(() => {}, 10000); }
if (args[0] === 'bad-json') { console.log('not JSON'); process.exit(0); }
console.log(JSON.stringify({args,cwd:process.cwd(),session:process.env.RDM_SESSION,root:process.env.RDM_ROOT,project:process.env.RDM_PROJECT,rdmEnv:Object.keys(process.env).filter(k=>k.startsWith('RDM_'))}));
`, { mode: 0o700 });
  // .mjs is required regardless of the enclosing checkout package configuration.
  fs.renameSync(binary, binary + '.mjs');
  return { sourceDir: path.join(root, 'source'), planRoot: path.join(root, 'plan'), rdmBin: binary + '.mjs', project: 'fixture', session: 'caller-existing-session', runDir: path.join(root, 'run'), operation: 'estimate' };
}

test('explicit identity, private evidence, owned session and direct argv', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  assert.notEqual(ctx.session, spec.session);
  const result = await ctx.rdm(['phase', 'show', 'literal;$(touch nope)'], { json: true });
  assert.equal(result.session, ctx.session); assert.equal(result.cwd, spec.sourceDir);
  assert.equal(result.root, spec.planRoot); assert.equal(result.project, spec.project);
  assert.equal(result.args[2], 'literal;$(touch nope)');
  assert.equal(fs.statSync(spec.runDir).mode & 0o777, 0o700);
  ctx.record('agent-call', { threadId: 'thread-one' }); ctx.finish({ approved: true });
  const manifest = JSON.parse(fs.readFileSync(path.join(spec.runDir, 'manifest.json')));
  assert.equal(manifest.status, 'completed'); assert.equal(manifest.identity.parentSession, spec.session);
  assert.throws(() => createRun(spec), /exist/i);
  await assert.rejects(() => ctx.rdm(['show']), /finished|closed/i);
});

test('reject implicit paths, sessions, nested source checkout and symlink evidence parent', async t => {
  const spec = fixture(t);
  for (const patch of [{ session: '' }, { project: '' }, { sourceDir: 'relative' }, { rdmBin: 'rdm' }]) {
    assert.throws(() => createRun({ ...spec, ...patch }));
  }
  fs.mkdirSync(path.join(spec.sourceDir, 'nested'));
  assert.throws(() => createRun({ ...spec, sourceDir: path.join(spec.sourceDir, 'nested') }), /root/i);
  const alias = path.join(path.dirname(spec.runDir), 'alias'); fs.symlinkSync(spec.sourceDir, alias);
  assert.throws(() => createRun({ ...spec, runDir: path.join(alias, 'run') }), /symlink|canonical/i);
});

test('write intent is durable before execution and failure prevents success', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  await assert.rejects(() => ctx.rdm(['fail'], { mutating: true }));
  assert.equal(fs.readFileSync(path.join(spec.planRoot, 'effect'), 'utf8'), 'written');
  const events = fs.readFileSync(path.join(spec.runDir, 'journal.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
  assert(events.some(e => e.type === 'write-intent')); assert(events.some(e => e.type === 'write-uncertain'));
  assert.throws(() => ctx.finish({ approved: true }), /uncertain/i);
  ctx.fail(new Error('interrupted'));
  assert.equal(JSON.parse(fs.readFileSync(path.join(spec.runDir, 'manifest.json'))).status, 'failed');
});

test('all-session commits rejected and JSON decode failures poison mutations', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  await assert.rejects(() => ctx.rdm(['commit', '--all'], { mutating: true }), /--all/);
  await assert.rejects(() => ctx.rdm(['bad-json'], { json: true, mutating: true }));
  assert.throws(() => ctx.finish({}), /uncertain/); ctx.fail(new Error('bad JSON'));
});

test('acknowledged writes capture separate plan HEAD and can finish', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  await ctx.rdm(['update'], { json: true, mutating: true }); ctx.finish({ applied: true });
  const events = fs.readFileSync(path.join(spec.runDir, 'journal.jsonl'), 'utf8');
  assert.match(events, /write-acknowledged/); assert.match(events, /planHead/);
});

test('identity overrides and unjournaled commits fail before execution', async t => {
  const ctx = createRun(fixture(t));
  for (const args of [['--root', '/tmp/other', 'show'], ['commit', '--changeset=someone-else'], ['commit']]) {
    await assert.rejects(() => ctx.rdm(args), /override|mutating/);
  }
  ctx.fail(new Error('test complete'));
});

test('bounded mutation timeout is uncertain and forbids automatic retry', async t => {
  const spec = fixture(t); const ctx = createRun({ ...spec, rdmTimeoutMs: 100 });
  await assert.rejects(() => ctx.rdm(['sleep'], { mutating: true }), /ETIMEDOUT|timed out/);
  await assert.rejects(() => ctx.rdm(['update'], { mutating: true }), /uncertain/);
  ctx.fail(new Error('timed out'));
});

test('inherited Git directory overrides cannot redirect source identity', async t => {
  const spec = fixture(t); const original = process.env.GIT_DIR;
  process.env.GIT_DIR = path.join(spec.planRoot, '.git');
  try {
    const ctx = createRun(spec); assert.equal(ctx.identity.sourceRoot, spec.sourceDir); ctx.finish({});
  } finally {
    if (original === undefined) delete process.env.GIT_DIR; else process.env.GIT_DIR = original;
  }
});

test('adapter postwrite uncertainty persists in failed manifest', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  await ctx.rdm(['update'], { mutating: true });
  const error = Object.assign(new Error('readback mismatch'), { uncertainWrites: true });
  ctx.fail(error);
  assert.equal(JSON.parse(fs.readFileSync(path.join(spec.runDir, 'manifest.json'))).uncertainWrites, true);
});

test('caller RDM overrides and harness knobs do not enter direct commands', async t => {
  const spec = fixture(t); const saved = process.env.RDM_CHANGESET;
  process.env.RDM_CHANGESET = 'unrelated';
  try {
    const ctx = createRun(spec); const result = await ctx.rdm(['show'], { json: true });
    assert.deepEqual(result.rdmEnv.sort(), ['RDM_BIN', 'RDM_PROJECT', 'RDM_ROOT', 'RDM_SESSION']);
    ctx.finish({});
  } finally {
    if (saved === undefined) delete process.env.RDM_CHANGESET; else process.env.RDM_CHANGESET = saved;
  }
});

test('manifest records runner identity for deliberate interrupted-run shutdown', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  const manifest = JSON.parse(fs.readFileSync(path.join(spec.runDir, 'manifest.json')));
  assert.equal(manifest.runner.pid, process.pid);
  assert.equal(manifest.runner.ppid, process.ppid);
  assert.equal(manifest.runner.executable, process.execPath);
  assert.deepEqual(manifest.runner.argv, process.argv);
  assert.equal(typeof manifest.runner.hostname, 'string');
  assert(Number.isFinite(Date.parse(manifest.runner.approximateStartedAt)));
  ctx.finish({});
});

async function waitForFile(file) {
  const until = Date.now() + 5000;
  while (!fs.existsSync(file)) {
    if (Date.now() > until) throw new Error('Timed out waiting for subprocess');
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}

test('cancellation stops mutation descendants and cannot report completion', async t => {
  const spec = fixture(t); const controller = new AbortController();
  const ctx = createRun({ ...spec, signal: controller.signal });
  const pending = ctx.rdm(['descendant'], { mutating: true });
  const rejected = assert.rejects(pending, /cancel|abort/i);
  await waitForFile(path.join(spec.planRoot, 'child-pid'));
  controller.abort(); await rejected;
  await new Promise(resolve => setTimeout(resolve, 650));
  assert.equal(fs.existsSync(path.join(spec.planRoot, 'late-effect')), false);
  await assert.rejects(() => ctx.rdm(['update'], { mutating: true }), /cancel|abort/i);
  assert.throws(() => ctx.finish({}), /uncertain|cancel|abort/i);
  ctx.fail(new Error('cancelled'));
  const manifest = JSON.parse(fs.readFileSync(path.join(spec.runDir, 'manifest.json')));
  assert.equal(manifest.status, 'failed'); assert.equal(manifest.uncertainWrites, true);
  assert.match(fs.readFileSync(path.join(spec.runDir, 'journal.jsonl'), 'utf8'), /rdm-started/);
});

test('preaborted run cannot start a mutation or finish', async t => {
  const spec = fixture(t); const controller = new AbortController(); controller.abort();
  const ctx = createRun({ ...spec, signal: controller.signal });
  await assert.rejects(() => ctx.rdm(['fail'], { mutating: true }), /cancel|abort/i);
  assert.equal(fs.existsSync(path.join(spec.planRoot, 'effect')), false);
  assert.throws(() => ctx.finish({}), /cancel|abort/i);
  ctx.fail(new Error('cancelled'));
  assert.doesNotMatch(fs.readFileSync(path.join(spec.runDir, 'journal.jsonl'), 'utf8'), /write-intent/);
});

test('direct process exit reaps leftover subprocess group before return', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  await ctx.rdm(['orphan']);
  await new Promise(resolve => setTimeout(resolve, 650));
  assert.equal(fs.existsSync(path.join(spec.planRoot, 'late-effect')), false);
  ctx.finish({});
});

test('direct command output limits fail mutations conservatively', async t => {
  const spec = fixture(t); const ctx = createRun(spec);
  await assert.rejects(() => ctx.rdm(['overflow'], { mutating: true }), /output exceeded/);
  assert.throws(() => ctx.finish({}), /uncertain/);
  ctx.fail(new Error('output limit'));
});

test('active direct command cannot race successful finalization', async t => {
  const spec = fixture(t); const controller = new AbortController();
  const ctx = createRun({ ...spec, signal: controller.signal });
  const pending = ctx.rdm(['sleep']); const rejected = assert.rejects(pending, /cancel/);
  assert.throws(() => ctx.finish({}), /active/);
  assert.throws(() => ctx.fail(new Error('premature failure')), /active/);
  assert.equal(JSON.parse(fs.readFileSync(path.join(spec.runDir, 'manifest.json'))).status, 'running');
  controller.abort(); await rejected; ctx.fail(new Error('cancelled'));
});
