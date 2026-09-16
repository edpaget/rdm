import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync, spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

const checkout = fileURLToPath(new URL('../../', import.meta.url));
const realBin = path.join(checkout, 'scripts/rdm-dev.sh');
const runner = path.join(checkout, 'scripts/rdm-codex.mjs');
const alive = pid => { try { process.kill(pid, 0); return true; } catch { return false; } };
async function until(check, message, timeout = 45000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { if (check()) return; await delay(30); }
  throw new Error(message);
}

for (const boundary of ['update', 'commit']) test(`real runner SIGTERM after actual ${boundary}: uncertain, children reaped, explicit reconciliation and fresh rerun`, { timeout: 180000 }, async t => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'runtime-interrupt-')));
  const source = path.join(root, 'source'), plans = path.join(root, 'plans'), bin = path.join(root, 'bin');
  for (const dir of [source, plans, bin]) fs.mkdirSync(dir);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('RDM_') && !key.startsWith('GIT_')));
  Object.assign(env, { RDM_ROOT: plans, RDM_PROJECT: 'fixture', RDM_SESSION: 'seed', RDM_BIN: realBin,
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_AUTHOR_NAME: 'Fixture', GIT_AUTHOR_EMAIL: 'fixture@example.invalid', GIT_COMMITTER_NAME: 'Fixture', GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
    PATH: `${bin}${path.delimiter}${process.env.PATH}` });
  const exec = (command, args, cwd = source, session = 'seed') => execFileSync(command, args, { cwd, env: { ...env, RDM_SESSION: session }, encoding: 'utf8', timeout: 120000, stdio: ['ignore', 'pipe', 'pipe'] });
  const rdm = (args, session) => exec(realBin, args, source, session);
  const scope = ['--roadmap', 'example', '--project', 'fixture'];
  const show = (stem, session) => JSON.parse(rdm(['phase', 'show', stem, ...scope, '--format', 'json'], session));
  const marker = path.join(root, 'boundary.json'), callsFile = path.join(root, 'calls.jsonl'), agentFile = path.join(root, 'agents.jsonl');
  const wrapper = path.join(bin, 'rdm-wrapper.mjs');
  fs.writeFileSync(wrapper, `#!${process.execPath}
import fs from 'node:fs'; import {execFileSync,spawn} from 'node:child_process';
const args=process.argv.slice(2);
fs.appendFileSync(${JSON.stringify(callsFile)},JSON.stringify({args,session:process.env.RDM_SESSION,pid:process.pid})+'\\n');
const result=execFileSync(${JSON.stringify(realBin)},args,{env:process.env,encoding:'utf8',stdio:['ignore','pipe','pipe']});
const match=process.env.FIXTURE_BOUNDARY==='commit'?args[0]==='commit':process.env.FIXTURE_BOUNDARY==='update'&&args[0]==='phase'&&args[1]==='update';
if(match){const descendant=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});fs.writeFileSync(${JSON.stringify(marker)},JSON.stringify({pid:process.pid,descendant:descendant.pid,session:process.env.RDM_SESSION}));setInterval(()=>{},1000);}else process.stdout.write(result);
`, { mode: 0o755 });
  fs.writeFileSync(path.join(bin, 'codex'), `#!${process.execPath}
import fs from 'node:fs';
let prompt='';for await(const chunk of process.stdin)prompt+=chunk;
fs.appendFileSync(${JSON.stringify(agentFile)},JSON.stringify({pid:process.pid})+'\\n');
const stem=prompt.match(/Phase stem: (\\S+)/)[1];
fs.writeFileSync(process.argv[process.argv.indexOf('-o')+1],JSON.stringify({stem,difficulty:'moderate',justification:'Bounded fixture work.'}));
process.stdout.write(JSON.stringify({type:'thread.started',thread_id:'fixture-'+process.pid})+'\\n'+JSON.stringify({type:'turn.completed',usage:{input_tokens:1,output_tokens:1}})+'\\n');
`, { mode: 0o755 });
  let child;
  t.after(() => {
    if (child && child.exitCode === null) child.kill('SIGKILL');
    if (fs.existsSync(callsFile)) for (const call of fs.readFileSync(callsFile, 'utf8').trim().split('\n').map(JSON.parse)) { try { process.kill(-call.pid, 'SIGKILL'); } catch {} }
    if (fs.existsSync(marker)) { const m = JSON.parse(fs.readFileSync(marker)); for (const pid of [m.pid, m.descendant]) { try { process.kill(pid, 'SIGKILL'); } catch {} } }
    fs.rmSync(root, { recursive: true, force: true });
  });
  exec('git', ['init', '-q', '-b', 'main']);
  exec('git', ['config', 'user.name', 'Fixture']); exec('git', ['config', 'user.email', 'fixture@example.invalid']);
  fs.writeFileSync(path.join(source, 'README'), 'fixture'); exec('git', ['add', '.']); exec('git', ['commit', '-qm', 'seed']);
  rdm(['init', '--default-project', 'fixture']);
  exec('git', ['config', 'user.name', 'Fixture'], plans); exec('git', ['config', 'user.email', 'fixture@example.invalid'], plans);
  rdm(['roadmap', 'create', 'example', '--title', 'Example', '--body', 'Fixture roadmap', '--no-edit', '--project', 'fixture']);
  for (const [number, slug] of [[1, 'first'], [2, 'second'], [3, 'other']]) rdm(['phase', 'create', slug, '--number', String(number), '--title', slug, '--body', 'Original body.', '--no-edit', ...scope]);
  rdm(['phase', 'update', 'phase-3-other', '--difficulty', 'hard', '--no-edit', ...scope]);
  rdm(['commit', '-m', 'test: seed']);
  rdm(['phase', 'update', 'phase-3-other', '--body', 'Unrelated pending edit.', '--no-edit', ...scope], 'unrelated');
  const initialHead = exec('git', ['rev-parse', 'HEAD'], plans).trim();
  const spec = { operation: 'estimate', sourceDir: source, planRoot: plans, rdmBin: wrapper, project: 'fixture', session: 'parent', roadmap: 'example', apply: true, runDir: path.join(root, 'run'), rdmTimeoutMs: 60000,
    host: { capabilities: { 'gpt-fixture': ['medium'] }, tiers: Object.fromEntries(['small', 'medium', 'large'].map(tier => [tier, { model: 'gpt-fixture', effort: 'medium' }])) } };
  const specFile = path.join(root, 'spec.json'); fs.writeFileSync(specFile, JSON.stringify(spec));
  let stderr = '', stdout = '', closed = false;
  child = spawn(process.execPath, [runner, specFile], { cwd: source, env: { ...env, FIXTURE_BOUNDARY: boundary }, stdio: ['ignore', 'pipe', 'pipe'] });
  child.stdout.on('data', data => { stdout += data; }); child.stderr.on('data', data => { stderr += data; }); child.on('close', () => { closed = true; });
  try { await until(() => fs.existsSync(marker) || closed || child.exitCode !== null, 'runner did not reach real side effect'); }
  catch (error) { throw new Error(`${error.message}; stderr=${stderr}; calls=${fs.existsSync(callsFile) ? fs.readFileSync(callsFile, 'utf8') : 'none'}`, { cause: error }); }
  assert.equal(closed, false, stderr);
  const boundaryState = JSON.parse(fs.readFileSync(marker));
  child.kill('SIGTERM');
  await until(() => closed, 'runner did not stop after SIGTERM', 10000);
  assert.notEqual(child.exitCode, 0); assert.equal(stdout, ''); assert.match(stderr, /uncertain|cancel/i);
  await until(() => !alive(boundaryState.pid) && !alive(boundaryState.descendant), 'RDM process group survived interruption', 5000);
  const manifest = JSON.parse(fs.readFileSync(path.join(spec.runDir, 'manifest.json')));
  assert.equal(manifest.status, 'failed'); assert.equal(manifest.uncertainWrites, true); assert.equal(manifest.identity.session, boundaryState.session);
  const journal = fs.readFileSync(path.join(spec.runDir, 'journal.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
  assert.ok(journal.some(e => e.type === 'write-uncertain')); assert.ok(journal.every(e => e.type !== 'run-completed'));
  const calls = fs.readFileSync(callsFile, 'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(calls.filter(c => c.args[0] === 'commit').length, boundary === 'commit' ? 1 : 0);
  assert.equal(calls.filter(c => c.args[0] === 'phase' && c.args[1] === 'update').length, boundary === 'commit' ? 2 : 1);
  for (const record of fs.readFileSync(agentFile, 'utf8').trim().split('\n').map(JSON.parse)) assert.equal(alive(record.pid), false);
  assert.equal(show('phase-3-other', 'unrelated').body.trim(), 'Unrelated pending edit.');
  assert.doesNotMatch(exec('git', ['show', 'HEAD:projects/fixture/roadmaps/example/phase-3-other.md'], plans), /Unrelated pending edit/);
  const afterHead = exec('git', ['rev-parse', 'HEAD'], plans).trim();
  if (boundary === 'update') {
    assert.equal(afterHead, initialHead); assert.equal(show('phase-1-first', boundaryState.session).difficulty, 'moderate'); assert.equal(show('phase-2-second').difficulty ?? null, null);
    // Explicit operator reconciliation: inspect the owned session, then commit
    // the already-applied first estimate before allocating a fresh run.
    rdm(['status'], boundaryState.session); rdm(['commit', '-m', 'test: reconcile interrupted estimate'], boundaryState.session);
  } else {
    assert.notEqual(afterHead, initialHead); rdm(['status'], boundaryState.session);
    assert.match(exec('git', ['show', 'HEAD:projects/fixture/roadmaps/example/phase-1-first.md'], plans), /## Estimate/);
  }
  assert.throws(() => exec(process.execPath, [runner, specFile]), /Command failed/);
  const retrySpec = { ...spec, runDir: path.join(root, 'fresh-run') }; fs.writeFileSync(specFile, JSON.stringify(retrySpec));
  const retry = JSON.parse(exec(process.execPath, [runner, specFile]));
  assert.equal(retry.result.estimated.length, boundary === 'update' ? 1 : 0);
  for (const stem of ['phase-1-first', 'phase-2-second']) { const phase = show(stem); assert.equal(phase.difficulty, 'moderate'); assert.equal(phase.body.match(/## Estimate/g).length, 1); }
  assert.equal(show('phase-3-other', 'unrelated').body.trim(), 'Unrelated pending edit.');
  assert.doesNotMatch(exec('git', ['show', 'HEAD:projects/fixture/roadmaps/example/phase-3-other.md'], plans), /Unrelated pending edit/);
});
