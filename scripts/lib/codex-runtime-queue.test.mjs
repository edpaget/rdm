import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { execFileSync, spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

const checkout = fileURLToPath(new URL('../../', import.meta.url));
const realBin = path.join(checkout, 'scripts/rdm-dev.sh');
const alive = pid => { try { process.kill(pid, 0); return true; } catch { return false; } };
const records = file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse) : [];
async function until(check, message, timeout = 60000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { if (check()) return; await delay(20); }
  throw new Error(message);
}

function fixture(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'runtime-queue-')));
  const source = path.join(root, 'source'), plans = path.join(root, 'plans'), bin = path.join(root, 'bin');
  for (const dir of [source, plans, bin]) fs.mkdirSync(dir);
  const events = path.join(root, 'agents.jsonl');
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('RDM_') && !key.startsWith('GIT_')));
  Object.assign(env, { RDM_ROOT: plans, RDM_PROJECT: 'fixture', RDM_SESSION: 'seed', RDM_BIN: realBin,
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_AUTHOR_NAME: 'Fixture', GIT_AUTHOR_EMAIL: 'fixture@example.invalid', GIT_COMMITTER_NAME: 'Fixture', GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
    PATH: `${bin}${path.delimiter}${process.env.PATH}` });
  const exec = (command, args, cwd = source) => execFileSync(command, args, { cwd, env, encoding: 'utf8', timeout: 120000, stdio: ['ignore', 'pipe', 'pipe'] });
  const rdm = args => exec(realBin, args);
  let child;
  t.after(() => {
    if (child && child.exitCode === null) child.kill('SIGKILL');
    for (const event of records(events).filter(event => event.type === 'start')) {
      try { process.kill(-event.pid, 'SIGKILL'); } catch {}
      try { process.kill(event.descendant, 'SIGKILL'); } catch {}
    }
    fs.rmSync(root, { recursive: true, force: true });
  });
  exec('git', ['init', '-q', '-b', 'main']);
  exec('git', ['config', 'user.name', 'Fixture']); exec('git', ['config', 'user.email', 'fixture@example.invalid']);
  fs.writeFileSync(path.join(source, 'README'), 'fixture'); exec('git', ['add', '.']); exec('git', ['commit', '-qm', 'seed']);
  rdm(['init', '--default-project', 'fixture']);
  exec('git', ['config', 'user.name', 'Fixture'], plans); exec('git', ['config', 'user.email', 'fixture@example.invalid'], plans);
  rdm(['roadmap', 'create', 'example', '--title', 'Example', '--body', 'Queue fixture', '--no-edit', '--project', 'fixture']);
  for (let number = 1; number <= 5; number++) rdm(['phase', 'create', `target-${number}`, '--number', String(number), '--title', `Target ${number}`, '--body', 'Unset estimate.', '--no-edit', '--roadmap', 'example', '--project', 'fixture']);
  rdm(['commit', '-m', 'test: seed queue']);
  const initialHead = exec('git', ['rev-parse', 'HEAD'], plans);
  fs.writeFileSync(path.join(bin, 'codex'), `#!${process.execPath}
import fs from 'node:fs'; import {spawn} from 'node:child_process';
let prompt='';for await(const chunk of process.stdin)prompt+=chunk;
const stem=prompt.match(/Phase stem: (\\S+)/)[1];
const descendant=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});
const record=type=>fs.appendFileSync(${JSON.stringify(events)},JSON.stringify({type,pid:process.pid,descendant:descendant.pid,stem})+'\\n');
record('start');
if(process.env.QUEUE_FIXTURE_CANCEL==='1')setInterval(()=>{},1000);
else setTimeout(()=>{
 fs.writeFileSync(process.argv[process.argv.indexOf('-o')+1],JSON.stringify({stem,difficulty:'moderate',justification:'Bounded fixture work.'}));
 process.stdout.write(JSON.stringify({type:'thread.started',thread_id:'fixture-'+process.pid})+'\\n'+JSON.stringify({type:'turn.completed',usage:{input_tokens:1,output_tokens:1}})+'\\n');
 record('end');process.exit(0);
},750);
`, { mode: 0o755 });
  let runner = path.join(checkout, 'scripts/rdm-codex.mjs');
  // Opt-in mutation check copies the implementation into a temporary module;
  // it never changes the live source shared with other tests or reviewers.
  if (process.env.CODEX_QUEUE_TEST_MUTATION === '1') {
    const runtimeUrl = new URL('./codex-runtime.mjs', import.meta.url);
    let mutated = fs.readFileSync(runtimeUrl, 'utf8');
    const semaphore = /    let active=0; const queue=\[\];[\s\S]*?    const parallel=/;
    assert.match(mutated, semaphore, 'mutation must replace the actual agent semaphore');
    mutated = mutated.replace(semaphore, '    const agent=rawAgent;\n    const parallel=');
    mutated = mutated.replace(/from (['"])(\.[^'"]+)\1/g, (_, quote, relative) => `from ${quote}${new URL(relative, runtimeUrl).href}${quote}`);
    const copy = path.join(root, 'unbounded-runtime.mjs'); fs.writeFileSync(copy, mutated);
    runner = path.join(root, 'mutated-runner.mjs');
    fs.writeFileSync(runner, `import fs from 'node:fs';import {runRuntime} from ${JSON.stringify(pathToFileURL(copy).href)};try {console.log(JSON.stringify(await runRuntime(JSON.parse(fs.readFileSync(process.argv[2],'utf8')))));}catch(error){console.error(error.message);process.exitCode=1;}`);
  }
  const spec = { operation: 'estimate', sourceDir: source, planRoot: plans, rdmBin: realBin, project: 'fixture', session: 'parent', roadmap: 'example', apply: false, concurrency: 2, runDir: path.join(root, 'run'), rdmTimeoutMs: 120000 };
  return {
    events, spec,
    launch(cancel) {
      const specFile = path.join(root, 'spec.json'); fs.writeFileSync(specFile, JSON.stringify(spec));
      let stdout = '', stderr = '', closed = false;
      child = spawn(process.execPath, [runner, specFile], { cwd: source, env: { ...env, QUEUE_FIXTURE_CANCEL: cancel ? '1' : '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
      child.stdout.on('data', data => { stdout += data; }); child.stderr.on('data', data => { stderr += data; }); child.on('close', () => { closed = true; });
      return { child, get stdout() { return stdout; }, get stderr() { return stderr; }, get closed() { return closed; } };
    },
    assertNoWrites() {
      assert.equal(exec('git', ['rev-parse', 'HEAD'], plans), initialHead);
      assert.equal(exec('git', ['status', '--porcelain'], plans), '');
      assert.equal(records(path.join(spec.runDir, 'journal.jsonl')).filter(event => event.type === 'write-intent').length, 0);
    },
  };
}

test('runtime estimate preview bounds five actual Codex children to two and completes all', { timeout: 180000 }, async t => {
  const f = fixture(t); const run = f.launch(false);
  await until(() => run.closed, 'preview did not complete');
  assert.equal(run.child.exitCode, 0, run.stderr);
  const result = JSON.parse(run.stdout);
  assert.equal(result.result.proposed.length, 5);
  let active = 0, peak = 0;
  const events = records(f.events);
  for (const event of events) { active += event.type === 'start' ? 1 : -1; peak = Math.max(peak, active); }
  assert.equal(peak, 2, 'the real runtime agent semaphore must limit live child calls');
  assert.equal(active, 0); assert.equal(events.filter(event => event.type === 'start').length, 5);
  assert.equal(new Set(events.filter(event => event.type === 'start').map(event => event.stem)).size, 5);
  for (const event of events) await until(() => !alive(event.pid) && !alive(event.descendant), 'completed child group survived', 5000);
  assert.equal(JSON.parse(fs.readFileSync(path.join(f.spec.runDir, 'manifest.json'))).status, 'completed');
  f.assertNoWrites();
});

test('runtime cancellation kills active judgments without launching the queued three', { timeout: 180000 }, async t => {
  const f = fixture(t); const run = f.launch(true);
  await until(() => records(f.events).length >= 2 || run.closed, 'active judgments did not start');
  assert.equal(run.closed, false, run.stderr);
  run.child.kill('SIGTERM');
  await until(() => run.closed, 'runner did not stop on cancellation', 10000);
  assert.notEqual(run.child.exitCode, 0); assert.equal(run.stdout, ''); assert.match(run.stderr, /cancel/i);
  const events = records(f.events);
  assert.equal(events.length, 2, 'queued judgments must never launch child processes after cancellation');
  for (const event of events) await until(() => !alive(event.pid) && !alive(event.descendant), 'cancelled child group survived', 5000);
  const journal = records(path.join(f.spec.runDir, 'journal.jsonl'));
  assert.equal(journal.filter(event => event.type === 'agent-started').length, 2);
  assert.equal(journal.some(event => event.type === 'run-completed'), false);
  assert.equal(JSON.parse(fs.readFileSync(path.join(f.spec.runDir, 'manifest.json'))).status, 'failed');
  f.assertNoWrites();
});
