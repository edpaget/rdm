import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, writeFile, rm, readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {runCodex, validateSchema, boundedParallel} from './codex-spike-process.mjs';
import * as transport from './codex-process.mjs';

const schema = {type: 'object', additionalProperties: false, required: ['ok'], properties: {
  ok: {type: 'boolean'}, note: {type: 'string'},
}};
test('spike compatibility entrypoint reuses the runtime transport', () => {
  assert.equal(runCodex, transport.runCodex);
  assert.equal(validateSchema, transport.validateSchema);
  assert.equal(boundedParallel, transport.boundedParallel);
});
async function fixture(t, mode = 'ok') {
  const cwd = await mkdtemp(join(tmpdir(), 'codex-process-test-'));
  t.after(() => rm(cwd, {recursive: true, force: true}));
  const bin = join(cwd, 'fake-codex');
  await writeFile(bin, `#!${process.execPath}
import {writeFileSync, readFileSync} from 'node:fs';
import {spawn} from 'node:child_process';
const args = process.argv.slice(2);
const mode = process.env.FAKE_MODE;
const emit = value => console.log(JSON.stringify(value));
writeFileSync('argv.json', JSON.stringify(args));
writeFileSync('schema.json', readFileSync(args[args.indexOf('--output-schema')+1]));
if (['auth', 'rate', 'unknown-model', 'nonzero'].includes(mode)) {console.error('Authorization: Bearer sk-fake-secret-token'); console.error('authentication failed'); process.exit(1);}
if (mode === 'descendant') {
 const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {stdio: 'inherit'});
 writeFileSync('descendant.pid', String(child.pid));
 setInterval(() => {}, 1000);
}
else if (mode === 'hang') {setInterval(() => {}, 1000);}
else if (mode === 'death') {process.kill(process.pid, 'SIGKILL');}
else {
emit({type:'thread.started', thread_id:'thread-one'});
emit({type:'turn.started'});
if (mode === 'malformed') console.log('{');
if (mode === 'error') emit({type:'error', message:'secret'});
if (mode === 'failed') emit({type:'turn.failed', error:{message:'secret'}});
if (mode === 'command-nonzero') emit({type:'item.completed',item:{type:'command_execution',status:'failed',exit_code:1}});
if (mode === 'command-inconsistent') emit({type:'item.completed',item:{type:'command_execution',status:'failed',exit_code:0}});
if (mode === 'command-interrupted') emit({type:'item.completed',item:{type:'command_execution',status:'failed',exit_code:null}});
if (mode === 'tool-failed') emit({type:'item.completed',item:{type:'mcp_tool_call',status:'failed'}});
if (mode === 'duplicate') emit({type:'thread.started', thread_id:'thread-two'});
if (mode === 'flood') console.log('x'.repeat(9*1024*1024));
writeFileSync(args[args.indexOf('-o')+1], JSON.stringify(mode === 'invalid' ? {ok:'yes'} : {ok:true, note:null}));
if (mode !== 'truncated') emit({type:'turn.completed', usage:{input_tokens:10,cached_input_tokens:2,output_tokens:3}});
}
`, {mode: 0o700});
  return {bin, cwd, prompt: 'Say ok', schema, model: 'test-model', effort: 'medium', env: {...process.env, FAKE_MODE: mode}};
}

test('validates canonical subset and fails closed on unknown keywords', () => {
  assert.equal(validateSchema({ok: true}, schema), true);
  assert.throws(() => validateSchema({ok: 'yes'}, schema), /schema/i);
  assert.throws(() => validateSchema({ok: true, extra: 1}, schema), /schema/i);
  assert.throws(() => validateSchema('x', {type: 'string', pattern: 'x'}), /unsupported/i);
  assert.throws(() => validateSchema(101, {type: 'integer', minimum: 0, maximum: 100}), /schema/i);
});
test('fresh subprocess uses strict nullable schema, stdin and safe argv', async t => {
  const opts = await fixture(t);
  const result = await runCodex(opts);
  assert.deepEqual(result.value, {ok: true});
  assert.equal(result.threadId, 'thread-one');
  assert.equal(result.usage.input_tokens, 10);
  const args = JSON.parse(await readFile(join(opts.cwd, 'argv.json'), 'utf8'));
  assert.ok(args.includes('--ephemeral'));
  assert.ok(args.includes('read-only'));
  assert.ok(!args.includes('--ignore-rules'));
  assert.ok(!args.includes(opts.prompt));
  const strict = JSON.parse(await readFile(join(opts.cwd, 'schema.json'), 'utf8'));
  assert.deepEqual(strict.required, ['ok', 'note']);
  assert.ok(strict.properties.note.anyOf.some(s => s.type === 'null'));
});
test('judgment subprocess disables optional external capabilities and escalation', async t => {
  const opts = await fixture(t);
  await runCodex(opts);
  const args = JSON.parse(await readFile(join(opts.cwd, 'argv.json'), 'utf8'));
  const configs = args.flatMap((arg, i) => arg === '-c' ? [args[i + 1]] : []);
  for (const config of ['approval_policy="never"', 'web_search="disabled"',
    'features.apps=false', 'features.plugins=false', 'features.remote_plugin=false',
    'features.browser_use=false', 'features.computer_use=false', 'features.hooks=false',
    'features.multi_agent=false', 'features.multi_agent_v2=false',
    'features.workspace_dependencies=false', 'features.image_generation=false']) {
    assert.ok(configs.includes(config), `missing role restriction: ${config}`);
  }
  assert.ok(args.includes('--ignore-user-config'));
  assert.ok(!args.includes('--ignore-rules'));
  assert.ok(!args.includes('--dangerously-bypass-approvals-and-sandbox'));
});
test('completed turn may recover from an unsuccessful exploratory shell command', async t => {
  const opts = await fixture(t, 'command-nonzero');
  assert.deepEqual((await runCodex(opts)).value, {ok:true});
});
for (const mode of ['command-inconsistent', 'command-interrupted', 'tool-failed', 'auth', 'rate', 'unknown-model', 'nonzero', 'death', 'malformed', 'error', 'failed', 'duplicate', 'truncated', 'invalid', 'flood']) {
  test(`rejects ${mode} without exposing provider diagnostics`, async t => {
    const opts = await fixture(t, mode);
    await assert.rejects(runCodex(opts), error => !/secret|sensitive/.test(error.message));
  });
}
test('timeout and abort await shutdown', async t => {
  const opts = await fixture(t, 'hang');
  await assert.rejects(runCodex({...opts, timeoutMs: 150}), /timed out/);
  const controller = new AbortController();
  const running = runCodex({...opts, signal: controller.signal});
  setTimeout(() => controller.abort(), 150);
  await assert.rejects(running, /cancelled/);
});
test('resume requires matching thread identity', async t => {
  const opts = await fixture(t);
  await assert.rejects(runCodex({...opts, resumeThreadId: 'different'}), /thread/i);
  assert.equal((await runCodex({...opts, resumeThreadId: 'thread-one'})).threadId, 'thread-one');
});
test('canonical labels work and failure diagnostics remain private and redacted', async t => {
  const opts = await fixture(t, 'auth');
  const evidenceDir = join(opts.cwd, 'evidence');
  await assert.rejects(runCodex({...opts, label: 'find:code:ac', evidenceDir}), /subprocess failed/);
  const diagnostic = await readFile(join(evidenceDir, 'find:code:ac.stderr.txt'), 'utf8');
  assert.match(diagnostic, /authentication failed/);
  assert.doesNotMatch(diagnostic, /sk-fake-secret-token/);
});
test('bounded parallel preserves order and waits for in-flight failure cleanup', async () => {
  let active = 0, maximum = 0;
  const result = await boundedParallel([0, 1, 2, 3].map(i => async () => {
    maximum = Math.max(maximum, ++active);
    await new Promise(resolve => setTimeout(resolve, 15));
    active--;
    return i;
  }), 2);
  assert.deepEqual(result, [0, 1, 2, 3]);
  assert.equal(maximum, 2);
  await assert.rejects(boundedParallel([async () => {throw Error('failed');}, async () => {
    active++; await new Promise(resolve => setTimeout(resolve, 20)); active--;
  }], 2), /failed/);
  assert.equal(active, 0);
});
test('parallel failures cannot silently succeed for falsy rejection values', async () => {
  let succeeded = false;
  await boundedParallel([async () => {throw null;}]).then(() => {succeeded = true;}, () => {});
  assert.equal(succeeded, false);
});
test('cancellation also stops descendant processes', {skip: process.platform === 'win32'}, async t => {
  const opts = await fixture(t, 'descendant');
  const controller = new AbortController();
  const running = runCodex({...opts, signal: controller.signal});
  const rejected = assert.rejects(running, /cancelled/);
  let pid;
  for (let attempt = 0; attempt < 200; attempt++) {
    try {pid = Number(await readFile(join(opts.cwd, 'descendant.pid'), 'utf8')); break;}
    catch {await new Promise(resolve => setTimeout(resolve, 10));}
  }
  controller.abort();
  await rejected;
  assert.ok(pid, 'fixture descendant started');
  for (let attempt = 0; attempt < 100; attempt++) {
    try {process.kill(pid, 0);}
    catch (error) {assert.equal(error.code, 'ESRCH'); return;}
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.fail('descendant survived cancellation');
});
