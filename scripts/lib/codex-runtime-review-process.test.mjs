import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { execFileSync, spawn } from 'node:child_process';

const checkout = fileURLToPath(new URL('../../', import.meta.url));
const realBin = path.join(checkout, 'scripts/rdm-dev.sh');
const records = file => fs.readFileSync(file, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);

function fixture(t, operation, invalid) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'runtime-review-process-')));
  const source = path.join(root, 'source'), plans = path.join(root, 'plans'), bin = path.join(root, 'bin');
  for (const dir of [source, plans, bin]) fs.mkdirSync(dir);
  const events = path.join(root, 'events.jsonl');
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('RDM_') && !key.startsWith('GIT_')));
  Object.assign(env, { RDM_ROOT: plans, RDM_PROJECT: 'fixture', RDM_SESSION: 'seed', RDM_BIN: realBin,
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_AUTHOR_NAME: 'Fixture', GIT_AUTHOR_EMAIL: 'fixture@example.invalid', GIT_COMMITTER_NAME: 'Fixture', GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
    PATH: `${bin}${path.delimiter}${process.env.PATH}` });
  const exec = (command, args, cwd = source) => execFileSync(command, args, { cwd, env, encoding: 'utf8', timeout: 120000, stdio: ['ignore', 'pipe', 'pipe'] });
  const git = (...args) => exec('git', args).trim();
  const rdm = args => exec(realBin, args);
  let child;
  t.after(() => {
    if (child && child.exitCode === null) child.kill('SIGKILL');
    if (fs.existsSync(events)) for (const event of records(events)) { try { process.kill(-event.pid, 'SIGKILL'); } catch {} }
    fs.rmSync(root, { recursive: true, force: true });
  });
  git('init', '-q', '-b', 'main'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid');
  fs.writeFileSync(path.join(source, 'sum.mjs'), 'export const add = (a, b) => a + b;\n');
  git('add', '.'); git('commit', '-qm', 'base'); const base = git('rev-parse', 'HEAD');
  fs.writeFileSync(path.join(source, 'sum.mjs'), 'export const add = (a, b) => a - b;\n');
  git('add', '.'); git('commit', '-qm', 'head'); const head = git('rev-parse', 'HEAD');
  rdm(['init', '--default-project', 'fixture']);
  exec('git', ['config', 'user.name', 'Fixture'], plans); exec('git', ['config', 'user.email', 'fixture@example.invalid'], plans);
  rdm(['roadmap', 'create', 'example', '--title', 'Example', '--body', 'Review fixture.', '--no-edit', '--project', 'fixture']);
  rdm(['commit', '-m', 'test: seed reviews']);
  const planHead = exec('git', ['rev-parse', 'HEAD'], plans);
  const planFile = path.join(root, 'implementation-plan.md');
  fs.writeFileSync(planFile, 'Implement add(a,b) using subtraction. Acceptance criterion: add(2,3) returns 5. Add a test and run node --test.');
  fs.writeFileSync(path.join(bin, 'codex'), `#!${process.execPath}
import fs from 'node:fs';
let prompt='';for await(const chunk of process.stdin)prompt+=chunk;
const args=process.argv.slice(2),schema=JSON.parse(fs.readFileSync(args[args.indexOf('--output-schema')+1],'utf8'));
const role=schema.properties.refuted?'refuter':'finder';
const dimension=prompt.match(/Your single dimension is [^\\n]*?\\(([^)]+)\\)\\./)?.[1];
const record=type=>fs.appendFileSync(${JSON.stringify(events)},JSON.stringify({type,pid:process.pid,args,schema,prompt,role,dimension})+'\\n');
record('start');
setTimeout(()=>{
 const finding={id:'planted-subtraction',concern:dimension,severity:'blocking',confidence:95,what_fails:'Subtraction cannot satisfy addition acceptance criteria.',location:'sum.mjs:1'};
 const value=role==='refuter'?{refuted:false,confidence:95}:schema.properties.ac?{ac:[{criterion:'add(2,3) returns 5',status:'FAIL',evidence:'sum.mjs:1 subtracts'}]}:{findings:['correctness','coherence'].includes(dimension)?[finding]:[]};
 fs.writeFileSync(args[args.indexOf('-o')+1],role==='refuter'&&${JSON.stringify(invalid)}?'{malformed-refuter':JSON.stringify(value));
 process.stdout.write(JSON.stringify({type:'thread.started',thread_id:'fixture-'+process.pid})+'\\n'+JSON.stringify({type:'turn.completed',usage:{input_tokens:1,output_tokens:1}})+'\\n');
 record('end');
},role==='refuter'?10:150);
`, { mode: 0o755 });
  let runner = path.join(checkout, 'scripts/rdm-codex.mjs');
  // The optional mutation runs a temporary module, leaving shared source untouched.
  if (process.env.CODEX_REVIEW_TEST_MUTATION === '1') {
    const runtimeUrl = new URL('./codex-runtime.mjs', import.meta.url);
    let mutated = fs.readFileSync(runtimeUrl, 'utf8');
    const mapping = "role === 'refuter' ? 'review-verify'";
    assert.ok(mutated.includes(mapping), 'mutation must replace actual role mapping');
    mutated = mutated.replace(mapping, "role === 'refuter' ? 'review-find'");
    mutated = mutated.replace(/from (['"])(\.[^'"]+)\1/g, (_, quote, relative) => `from ${quote}${new URL(relative, runtimeUrl).href}${quote}`);
    const copy = path.join(root, 'wrong-refuter-model.mjs'); fs.writeFileSync(copy, mutated);
    runner = path.join(root, 'mutated-runner.mjs');
    fs.writeFileSync(runner, `import fs from 'node:fs';import {runRuntime} from ${JSON.stringify(pathToFileURL(copy).href)};try {console.log(JSON.stringify(await runRuntime(JSON.parse(fs.readFileSync(process.argv[2],'utf8')))));}catch(error){console.error(error.message);process.exitCode=1;}`);
  }
  const spec = { operation, sourceDir: source, planRoot: plans, rdmBin: realBin, project: 'fixture', session: 'parent', runDir: path.join(root, 'run'), concurrency: 2, base, head,
    target: 'Acceptance criteria: add(2,3) returns 5.', planFile,
    host: { capabilities: { 'gpt-fixture-find': ['medium'], 'gpt-fixture-verify': ['high'] }, tiers: {
      small: { model: 'gpt-fixture-find', effort: 'medium' }, medium: { model: 'gpt-fixture-find', effort: 'medium' }, large: { model: 'gpt-fixture-verify', effort: 'high' },
    } } };
  return {
    spec, events,
    async run() {
      const specFile = path.join(root, 'spec.json'); fs.writeFileSync(specFile, JSON.stringify(spec));
      let stdout = '', stderr = '';
      child = spawn(process.execPath, [runner, specFile], { cwd: source, env, stdio: ['ignore', 'pipe', 'pipe'] });
      child.stdout.on('data', data => { stdout += data; }); child.stderr.on('data', data => { stderr += data; });
      const timer = setTimeout(() => child.kill('SIGTERM'), 60000);
      try { const status = await new Promise((resolve, reject) => { child.on('error', reject); child.on('close', resolve); }); return { status, stdout, stderr }; }
      finally { clearTimeout(timer); }
    },
    assertNoWrites() {
      assert.equal(git('rev-parse', 'HEAD'), head); assert.equal(git('status', '--porcelain'), '');
      assert.equal(exec('git', ['rev-parse', 'HEAD'], plans), planHead); assert.equal(exec('git', ['status', '--porcelain'], plans), '');
      assert.equal(records(path.join(spec.runDir, 'journal.jsonl')).some(event => event.type === 'write-intent'), false);
    },
  };
}

for (const operation of ['code-review', 'plan-review']) {
  test(`full ${operation} runner maps models, fresh contexts and finder barrier before planted refutation`, { timeout: 180000 }, async t => {
    const f = fixture(t, operation, false); const run = await f.run();
    assert.equal(run.status, 0, run.stderr);
    const report = JSON.parse(run.stdout).result;
    assert.equal(report.coverage.complete, true); assert.equal(report.budget.graded, 1);
    assert.ok((operation === 'code-review' ? report.survivors : report.findings).some(finding => finding.id === 'planted-subtraction'));
    if (operation === 'code-review') assert.equal(report.outcome, 'rework');
    const events = records(f.events), starts = events.filter(event => event.type === 'start');
    const finders = starts.filter(event => event.role === 'finder'), refuters = starts.filter(event => event.role === 'refuter');
    assert.ok(finders.length >= 3); assert.equal(refuters.length, 1); assert.equal(new Set(starts.map(event => event.pid)).size, starts.length);
    const firstRefuter = events.findIndex(event => event.type === 'start' && event.role === 'refuter');
    assert.equal(events.slice(0, firstRefuter).filter(event => event.type === 'end' && event.role === 'finder').length, finders.length);
    for (const call of starts) {
      assert.equal(call.args[call.args.indexOf('-m') + 1], call.role === 'finder' ? 'gpt-fixture-find' : 'gpt-fixture-verify');
      assert.ok(call.args.includes(`model_reasoning_effort="${call.role === 'finder' ? 'medium' : 'high'}"`));
      assert.equal(call.args[call.args.indexOf('--sandbox') + 1], 'read-only'); assert.ok(call.args.includes('--ephemeral')); assert.equal(call.args.includes('resume'), false);
      assert.ok(call.schema.properties); assert.ok(call.prompt.length > 100);
    }
    const journal = records(path.join(f.spec.runDir, 'journal.jsonl'));
    const resolved = journal.filter(event => event.type === 'model-resolved');
    assert.equal(resolved.find(event => event.data.step === 'review-find').data.core.tier, 'medium');
    assert.equal(resolved.find(event => event.data.step === 'review-verify').data.core.tier, 'large');
    assert.equal(journal.filter(event => event.type === 'agent-completed').length, starts.length);
    assert.equal(JSON.parse(fs.readFileSync(path.join(f.spec.runDir, 'manifest.json'))).status, 'completed');
    f.assertNoWrites();
  });
  test(`full ${operation} runner rejects malformed independent refuter JSON`, { timeout: 180000 }, async t => {
    const f = fixture(t, operation, true); const run = await f.run();
    assert.notEqual(run.status, 0); assert.equal(run.stdout, ''); assert.match(run.stderr, /incomplete|malformed/i);
    assert.equal(records(f.events).filter(event => event.type === 'start' && event.role === 'refuter').length, 1);
    const journal = records(path.join(f.spec.runDir, 'journal.jsonl'));
    assert.ok(journal.some(event => event.type === 'agent-failed'));
    assert.equal(journal.some(event => event.type === 'run-completed'), false);
    assert.equal(JSON.parse(fs.readFileSync(path.join(f.spec.runDir, 'manifest.json'))).status, 'failed');
    f.assertNoWrites();
  });
}
