#!/usr/bin/env node
// Opt-in real Codex check. All installations and logs belong to a fresh temp home.
import assert from 'node:assert/strict';
import {execFileSync, spawn} from 'node:child_process';
import {copyFileSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join, relative, resolve} from 'node:path';
import {runSmokeProcess} from './lib/codex-smoke-process.mjs';

const args = process.argv.slice(2);
assert(args.length === 2 || args.length === 3,
  'Usage: node scripts/verify-codex-coexistence.mjs <rdm-binary> <codex-binary> [auth.json for live invocation]');
const [rdm, codex, auth] = args.map(p => resolve(p));
const root = realpathSync(mkdtempSync(join(tmpdir(), 'rdm-codex-coexistence-')));
console.log(`Coexistence fixture: ${root}`);
const home = join(root, 'home');
const config = join(root, 'config');
const source = join(root, 'source');
const env = {...process.env, HOME: home, CODEX_HOME: config, XDG_CONFIG_HOME: join(home, '.config'),
  GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null'};
for (const key of Object.keys(env)) if (key.startsWith('RDM_')) delete env[key];
for (const path of [home, config, source]) mkdirSync(path, {recursive: true});
const put = (path, body) => {
  mkdirSync(dirname(path), {recursive: true});
  writeFileSync(path, body, {mode: 0o600});
};
const json = (path, value) => put(path, JSON.stringify(value, null, 2) + '\n');
const run = (bin, argv, cwd = source) => execFileSync(bin, argv, {
  env, cwd, encoding: 'utf8', timeout: 120000, maxBuffer: 8 * 1024 * 1024,
  stdio: ['ignore', 'pipe', 'pipe'],
});
const userSkill = join(home, '.agents/skills/rdm-roadmap/SKILL.md');
const plugin = join(home, 'plugins/rdm-coexistence');
const pluginSkill = join(plugin, 'skills/rdm-roadmap/SKILL.md');
const competing = copy => `---\nname: rdm-roadmap\ndescription: ${copy} copy for the isolated coexistence test.\n---\n\nWhen explicitly selected, report ${copy}_COPY_ONLY. Do not mutate anything.\n`;
put(userSkill, competing('USER'));
put(pluginSkill, competing('PLUGIN'));
// Minimal manifest/marketplace shape validated with the Plugin Creator scaffold.
json(join(plugin, '.codex-plugin/plugin.json'), {
  name: 'rdm-coexistence', version: '0.1.0', description: 'Isolated coexistence fixture', skills: './skills/',
});
json(join(home, '.agents/plugins/marketplace.json'), {
  name: 'personal', interface: {displayName: 'Personal'}, plugins: [{
    name: 'rdm-coexistence', source: {source: 'local', path: './plugins/rdm-coexistence'},
    policy: {installation: 'AVAILABLE', authentication: 'ON_INSTALL'}, category: 'Productivity',
  }],
});
run('git', ['init', '-q']);
const installed = JSON.parse(run(codex, ['plugin', 'add', 'rdm-coexistence@personal', '--json']));
const installedSkill = join(installed.installedPath, 'skills/rdm-roadmap/SKILL.md');
const protectedPaths = [userSkill, pluginSkill, installedSkill,
  join(home, '.agents/plugins/marketplace.json'), join(plugin, '.codex-plugin/plugin.json')];
const before = protectedPaths.map(path => readFileSync(path));
run(rdm, ['agent-config', 'codex', '--skills', '--project', 'coexistence', '--out', source]);
const repositorySkill = join(source, '.agents/skills/rdm-roadmap/SKILL.md');
const repositoryBytes = readFileSync(repositorySkill);

async function discover() {
  const child = spawn(codex, ['app-server', '--stdio'], {env, cwd: source, stdio: ['pipe', 'pipe', 'pipe']});
  return await new Promise((resolveResult, reject) => {
    let buffer = '';
    let settled = false;
    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.kill();
      if (error) reject(error); else resolveResult(result);
    };
    const timer = setTimeout(() => finish(new Error('Codex skill discovery timed out')), 30000);
    child.on('error', error => finish(error));
    child.on('exit', code => finish(new Error(`Codex exited before discovery: ${code}`)));
    child.stderr.resume();
    child.stdout.on('data', chunk => {
      buffer += chunk;
      let end;
      while ((end = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
        let message;
        try { message = JSON.parse(line); } catch { continue; }
        if (message.error) return finish(new Error(JSON.stringify(message.error)));
        if (message.id === 1) {
          child.stdin.write(JSON.stringify({method: 'initialized', params: {}}) + '\n');
          child.stdin.write(JSON.stringify({id: 2, method: 'skills/list', params: {cwds: [source], forceReload: true}}) + '\n');
        }
        if (message.id === 2) finish(null, message.result.data[0]);
      }
    });
    child.stdin.on('error', error => finish(error));
    child.stdin.write(JSON.stringify({id: 1, method: 'initialize', params: {
      clientInfo: {name: 'rdm-coexistence-test', version: '1.0'},
    }}) + '\n');
  });
}

const catalog = await discover();
assert.deepEqual(catalog.errors, []);
for (const path of [repositorySkill, userSkill, installedSkill]) {
  assert(catalog.skills.some(skill => skill.path === path && skill.enabled), `Missing enabled skill: ${path}`);
}
assert(catalog.skills.some(skill => skill.path === installedSkill && skill.pluginId === 'rdm-coexistence@personal'));
json(join(root, 'catalog.json'), catalog.skills.filter(skill => skill.name.includes('rdm-')));
let live = false;
const authCopy = join(config, 'auth.json');
try {
  if (auth) {
    // Explicit opt-in only; never symlink or modify the real login. No credentials in logs.
    const output = join(root, 'answer.json');
    const schema = join(root, 'schema.json');
    json(schema, {type: 'object', additionalProperties: false, required: ['selected_path', 'plan_gate'], properties: {
      selected_path: {type: 'string'}, plan_gate: {type: 'string'},
    }});
    const prompt = `Use $rdm-roadmap specifically from ${repositorySkill}, not the user or plugin copy. This is an inspection-only smoke test: read the selected skill, identify its absolute path and the required plan-review gate, then stop. Do not run rdm, create plans, edit files, invoke agents, or use the network. Do not treat an alternative copy as selected merely because its name matches.`;
    const events = await runSmokeProcess(codex, ['exec', '--ephemeral', '--sandbox', 'read-only', '--json', '--cd', source,
      '--output-schema', schema, '--output-last-message', output, prompt], {
      env, cwd: source,
      prepare: () => { put(authCopy, ''); copyFileSync(auth, authCopy); },
      cleanup: () => rmSync(authCopy, {force: true}),
    });
    put(join(root, 'events.jsonl'), events);
    const readObserved = events.trim().split('\n').map(line => JSON.parse(line)).some(event =>
      event.type === 'item.completed' && event.item?.type === 'command_execution' &&
      event.item.exit_code === 0 && event.item.command.includes(repositorySkill) &&
      event.item.aggregated_output.includes('needs-plan-review'));
    assert(readObserved, 'No successful read of the selected repository skill in the CLI trace');
    const answer = JSON.parse(readFileSync(output, 'utf8'));
    assert.equal(answer.selected_path, repositorySkill);
    assert.match(answer.plan_gate, /needs-plan-review|independent.*review/i);
    assert(!JSON.stringify(answer).includes('USER_COPY_ONLY') && !JSON.stringify(answer).includes('PLUGIN_COPY_ONLY'));
    live = true;
  }
} finally {
  if (auth) rmSync(authCopy, {force: true});
  protectedPaths.forEach((path, index) => assert.deepEqual(readFileSync(path), before[index], `Changed competing copy: ${path}`));
  assert.deepEqual(readFileSync(repositorySkill), repositoryBytes, 'Live test modified repository skill');
}
json(join(root, 'result.json'), {
  codexVersion: run(codex, ['--version']).trim(), discoveredCopies: 3, liveInvocation: live,
  selectedPath: relative(root, repositorySkill), competingCopiesUnchanged: true,
  authCopyRemoved: Boolean(auth),
});
console.log(`PASS: three enabled copies discovered; ${live ? 'live repository invocation passed' : 'live invocation NOT RUN (no auth file supplied)'}; competing files unchanged. Evidence: ${root}`);
