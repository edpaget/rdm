import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {execFileSync, spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {resolveModels, reviewPlan, reviewCode, createJudgmentAgent} from './codex-runtime.mjs';
const host = {capabilities: {'gpt-6-astra':['medium','high']}, tiers: {small:{model:'gpt-6-astra',effort:'medium'},medium:{model:'gpt-6-astra',effort:'medium'},large:{model:'gpt-6-astra',effort:'high'}}};
function ctx(root) {return {identity:{sourceDir:root,project:'fixture'},runDir:root,record(){},rdm(args){return {step:args[2],tier:args[2]==='review-verify'?'large':'medium',model:'sonnet'};}};}
function fixture(t) {const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'runtime-review-'))); t.after(()=>fs.rmSync(root,{recursive:true,force:true})); const git=(...args)=>execFileSync('git',args,{cwd:root,encoding:'utf8',env:{...process.env,GIT_AUTHOR_NAME:'Test',GIT_AUTHOR_EMAIL:'test@example.invalid',GIT_COMMITTER_NAME:'Test',GIT_COMMITTER_EMAIL:'test@example.invalid'}}).trim(); git('init','-q'); fs.writeFileSync(path.join(root,'a.js'),'export const a = 1;\n');git('add','.');git('commit','-qm','base');const base=git('rev-parse','HEAD');fs.writeFileSync(path.join(root,'a.js'),'export const a = 2;\n');git('add','.');git('commit','-qm','head');return {root,git,base,head:git('rev-parse','HEAD')};}
const deps={parallel:async ts=>Promise.all(ts.map(t=>t())),pipeline:async (xs,...stages)=>Promise.all(xs.map(async x=>{for(const s of stages)x=await s(x);return x;})),log(){},agent:async (_p,o)=>o.label==='find:code:ac'?{findings:[],ac:[{criterion:'a is 2',status:'PASS',evidence:'a.js:1'}]}:{findings:[]}};
test('core effective tiers select separate host bindings; invalid overrides fail closed',()=>{const c=ctx('/tmp');const models=resolveModels(c,host,['review-find','review-verify']);assert.equal(models['review-verify'].effort,'high');assert.equal(models['review-find'].model,'gpt-6-astra');assert.throws(()=>resolveModels(c,{...host,steps:{'review-find':{tier:'small'}}},['review-find']),/tier/);assert.throws(()=>resolveModels(c,{...host,tiers:{medium:{model:'sonnet',effort:'medium'}}},['review-find']),/model/);});
test('judgment rejects mechanical and unknown roles before process execution',async()=>{const agent=createJudgmentAgent(ctx('/tmp'),{},{});await assert.rejects(agent('x',{agentType:'mechanical'}),/role/);await assert.rejects(agent('x',{agentType:'unrecognized'}),/role/);});
test('plan reviews arbitrary content and reject drift',async t=>{const {root}=fixture(t);const file=path.join(root,'plan.md');fs.writeFileSync(file,'Implement a = 2 and test it.');const c=ctx(root);const result=await reviewPlan(c,{planFile:file},deps,{});assert.equal(result.coverage.complete,true);await assert.rejects(reviewPlan(c,{planFile:file},{...deps,agent:async()=>{fs.writeFileSync(file,'changed');return {findings:[]};}},{}),/changed/);});
test('code review pins clean range, requires AC, and detects source drift',async t=>{const {root,git,base,head}=fixture(t);const c=ctx(root);const spec={base,head,target:'Acceptance criteria: a is 2.'};const result=await reviewCode(c,spec,deps,{},'medium');assert.equal(result.outcome,'reviewed');await assert.rejects(reviewCode(c,{...spec,head:base},deps,{},'medium'),/HEAD/);await assert.rejects(reviewCode(c,{...spec,base:'--all'},deps,{},'medium'),/revision/);await assert.rejects(reviewCode(c,spec,{...deps,agent:async()=>({findings:[],ac:[]})},{},'medium'),/incomplete/);await assert.rejects(reviewCode(c,spec,{...deps,agent:async(p,o)=>{fs.writeFileSync(path.join(root,'a.js'),'changed');return deps.agent(p,o);}},{},'medium'),/changed|clean/);});

test('host bindings retain effective core floor and validate each step capability without fallback', () => {
  const c = ctx('/tmp'); const calls = []; const records = [];
  c.rdm = args => { calls.push(args); return { step: args[2], tier: 'large', model: 'opus' }; };
  c.record = (type, data) => records.push({ type, data });
  const configured = { ...host, steps: { 'review-find': { tier: 'large', model: 'gpt-6-astra', effort: 'medium' } } };
  const models = resolveModels(c, configured, ['review-find', 'review-verify'], 'small');
  assert.deepEqual(calls.map(args => args.slice(0, 5)), [['model', 'resolve', 'review-find', '--tier', 'small'], ['model', 'resolve', 'review-verify', '--tier', 'small']]);
  assert.equal(models['review-find'].tier, 'large'); assert.equal(models['review-find'].effort, 'medium');
  assert.equal(models['review-verify'].effort, 'high'); assert.equal(records[0].data.core.model, 'opus');
  for (const binding of [{ tier: 'small', model: 'gpt-6-astra', effort: 'medium' }, { tier: 'large', model: 'gpt-6-astra', effort: 'low' }, { tier: 'large', model: 'opus', effort: 'high' }, { tier: 'large', model: 'missing-model', effort: 'high' }]) {
    assert.throws(() => resolveModels(c, { ...host, capabilities: { ...host.capabilities, opus: ['high'] }, steps: { 'review-find': binding } }, ['review-find']), /tier|Unsupported/);
  }
  assert.throws(() => resolveModels(c, { ...host, tiers: {} }, ['review-find']), /Unsupported/);
});

function phaseContext(root, git) {
  const c = ctx(root);
  const item = { roadmap: 'example', body: 'Acceptance criteria: a is 2.', model: 'medium', tags: [] };
  const worktree = { item: 'example', path: root, branch: git('symbolic-ref', '--short', 'HEAD') };
  const calls = [];
  c.rdm = args => { calls.push(args); return args[0] === 'phase' ? structuredClone(item) : [structuredClone(worktree)]; };
  return { c, item, worktree, calls };
}

test('phase reviews verify actual checkout identity and explicitly scope phase reads', async t => {
  const { root, git, base, head } = fixture(t); const { c, worktree, calls } = phaseContext(root, git);
  const spec = { base, head, item: { type: 'phase', roadmap: 'example', phase: 'phase-3-runtime' } };
  assert.equal((await reviewCode(c, spec, deps, {}, 'medium')).outcome, 'reviewed');
  assert.ok(calls.filter(args => args[0] === 'phase').every(args => args.includes('--project') && args[args.indexOf('--project') + 1] === 'fixture'));
  let agentCalls = 0;
  worktree.branch = 'wrong-branch';
  await assert.rejects(reviewCode(c, spec, { ...deps, agent: async () => { agentCalls++; return { findings: [] }; } }, {}, 'medium'), /worktree identity/);
  assert.equal(agentCalls, 0);
  worktree.branch = git('symbolic-ref', '--short', 'HEAD'); worktree.path = path.dirname(root);
  await assert.rejects(reviewCode(c, spec, deps, {}, 'medium'), /worktree identity/);
});

test('phase body drift during independent review rejects the original result', async t => {
  const { root, git, base, head } = fixture(t); const { c, item } = phaseContext(root, git);
  const spec = { base, head, item: { type: 'phase', roadmap: 'example', phase: 'phase-3-runtime' } };
  await assert.rejects(reviewCode(c, spec, { ...deps, agent: async (prompt, options) => { item.body = 'Changed acceptance criteria.'; return deps.agent(prompt, options); } }, {}, 'medium'), /changed/);
});

test('HEAD changing to another clean commit during review rejects the result', async t => {
  const { root, git, base, head } = fixture(t); let moved = false;
  await assert.rejects(reviewCode(ctx(root), { base, head, target: 'Acceptance criteria: a is 2.' }, { ...deps, agent: async (prompt, options) => {
    if (!moved) { moved = true; git('commit', '--allow-empty', '-qm', 'concurrent commit'); }
    return deps.agent(prompt, options);
  } }, {}, 'medium'), /changed/);
  assert.equal(git('status', '--porcelain'), ''); assert.notEqual(git('rev-parse', 'HEAD'), head);
});

const finding = id => ({ id, concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'Fixture failure', location: 'a.js:1' });
test('refuter failure and budget overflow are incomplete rather than clean reviews', async t => {
  const { root, base, head } = fixture(t); const spec = { base, head, target: 'Acceptance criteria: a is 2.' };
  for (const overflow of [false, true]) {
    let refutations = 0;
    const agent = async (prompt, options) => {
      if (options.label.startsWith('refute:')) { refutations++; if (!overflow) throw new Error('refuter crashed'); return { refuted: true, confidence: 100 }; }
      if (options.label === 'find:code:correctness') return { findings: Array.from({ length: overflow ? 6 : 1 }, (_, index) => finding(`fixture-${index}`)) };
      return deps.agent(prompt, options);
    };
    await assert.rejects(reviewCode(ctx(root), spec, { ...deps, agent }, {}, 'medium'), /incomplete/);
    assert.equal(refutations, overflow ? 5 : 1);
  }
});

test('missing finder coverage is incomplete despite clean remaining dimensions', async t => {
  const { root, base, head } = fixture(t);
  await assert.rejects(reviewCode(ctx(root), { base, head, target: 'Acceptance criteria: a is 2.' }, { ...deps, parallel: tasks => Promise.all(tasks.map(task => task().catch(() => null))), agent: async (prompt, options) => {
    if (options.label === 'find:code:correctness') throw new Error('finder unavailable');
    return deps.agent(prompt, options);
  } }, {}, 'medium'), /incomplete/);
});

test('pre-aborted judgment creates no child evidence or started event', async t => {
  const { root } = fixture(t); const controller = new AbortController(); controller.abort(); const c = ctx(root); const records = [];
  c.record = (...args) => records.push(args);
  const agent = createJudgmentAgent(c, { plan: { model: 'gpt-6-astra', effort: 'medium' } }, { signal: controller.signal });
  await assert.rejects(agent('Rate fixture', { agentType: 'estimator', label: 'estimate:rate:fixture', schema: { type: 'object', additionalProperties: false, properties: {}, required: [] } }), /abort|cancel/i);
  assert.deepEqual(records, []); assert.equal(fs.existsSync(path.join(root, 'call-1')), false);
});

test('CLI invalid specs fail clearly without success output or run evidence', t => {
  const { root } = fixture(t); const cli = fileURLToPath(new URL('../rdm-codex.mjs', import.meta.url));
  const specFile = path.join(root, 'invalid-spec.json'); const runDir = path.join(root, 'evidence');
  for (const content of ['{bad json', JSON.stringify({ operation: 'unknown', runDir }), JSON.stringify({ operation: 'estimate', concurrency: 0, runDir })]) {
    fs.writeFileSync(specFile, content);
    const result = spawnSync(process.execPath, [cli, specFile], { encoding: 'utf8', timeout: 10000 });
    assert.equal(result.status, 1); assert.match(result.stderr, /Codex runtime failed:/); assert.equal(result.stdout, ''); assert.equal(fs.existsSync(runDir), false);
  }
});
