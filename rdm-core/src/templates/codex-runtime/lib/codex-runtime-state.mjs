/** Durable per-run evidence and direct, bounded RDM operations. */
import fs from 'node:fs';
import path from 'node:path';
import { hostname } from 'node:os';
import { randomUUID } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';

const MAX_BUFFER = 8 * 1024 * 1024;

/** Execute git argv in an explicit checkout; callers must validate revision arguments. */
export function safeGit(sourceDir, args) {
  return execFileSync('git', ['-C', sourceDir, ...args], {
    encoding: 'utf8', timeout: 30_000, maxBuffer: MAX_BUFFER, stdio: ['ignore', 'pipe', 'pipe'],
    env: gitEnvironment(),
  }).trim();
}

function gitEnvironment() {
  const env = { ...process.env, GIT_TERMINAL_PROMPT: '0' };
  // A caller's git override must not redirect identity or persistence checks.
  for (const key of Object.keys(env)) if (key.startsWith('RDM_') || (key.startsWith('GIT_') && key !== 'GIT_TERMINAL_PROMPT')) delete env[key];
  return env;
}

/** Bind both direct commands and judgment children to the runtime-owned identity. */
export function rdmEnvironment(identity) {
  return { ...gitEnvironment(), RDM_BIN: identity.rdmBin, RDM_ROOT: identity.planRoot,
    RDM_PROJECT: identity.project, RDM_SESSION: identity.session };
}

function absoluteExisting(value, name, directory) {
  if (typeof value !== 'string' || !path.isAbsolute(value)) throw new Error(`${name} must be explicit and absolute`);
  const real = fs.realpathSync(value);
  const stat = fs.statSync(real);
  if (directory ? !stat.isDirectory() : !stat.isFile()) throw new Error(`${name} has wrong filesystem type`);
  if (!directory) fs.accessSync(real, fs.constants.X_OK);
  return real;
}

function textIdentity(value, name) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9_.-]{0,159}$/.test(value)) throw new Error(`Invalid explicit ${name}`);
  return value;
}

function syncDirectory(dir) {
  const fd = fs.openSync(dir, 'r');
  try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}

function atomicJson(dir, name, value) {
  const tmp = path.join(dir, `.${name}-${randomUUID()}`);
  const fd = fs.openSync(tmp, 'wx', 0o600);
  try { fs.writeFileSync(fd, JSON.stringify(value, null, 2) + '\n'); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  fs.renameSync(tmp, path.join(dir, name)); syncDirectory(dir);
}

/** Run one cancellable argv process and remove its remaining process group. */
function directRdm(bin, args, { cwd, env, timeout, signal, started }) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(new Error('RDM command cancelled')); return; }
    const child = spawn(bin, args, { cwd, env, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] });
    let failure;
    let bytes = 0;
    const output = [];
    const errors = [];
    const killGroup = () => {
      if (!child.pid) return;
      try {
        if (process.platform === 'win32') child.kill('SIGKILL');
        else process.kill(-child.pid, 'SIGKILL');
      } catch (error) { if (error.code !== 'ESRCH') failure ??= new Error('Could not terminate RDM process group'); }
    };
    const stop = message => { failure ??= new Error(message); killGroup(); };
    const abort = () => stop('RDM command cancelled');
    const timer = setTimeout(() => stop('RDM command timed out'), timeout);
    signal?.addEventListener('abort', abort, { once: true });
    const collect = chunks => chunk => {
      bytes += chunk.length;
      if (bytes > MAX_BUFFER) stop('RDM command output exceeded limit');
      else chunks.push(chunk);
    };
    child.stdout.on('data', collect(output)); child.stderr.on('data', collect(errors));
    child.on('error', error => { failure ??= error; });
    child.on('exit', killGroup);
    child.on('close', (status, deathSignal) => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort);
      if (signal?.aborted) failure ??= new Error('RDM command cancelled');
      if (!failure && (status !== 0 || deathSignal)) failure = new Error(`RDM command failed (exit ${status ?? deathSignal}): ${Buffer.concat(errors).toString('utf8')}`);
      if (failure) { failure.status = status; failure.signal = deathSignal; reject(failure); }
      else resolve(Buffer.concat(output).toString('utf8'));
    });
    try { started({ pid: child.pid ?? null, processGroup: process.platform === 'win32' ? null : child.pid ?? null }); }
    catch (error) { failure ??= error; killGroup(); }
    if (signal?.aborted) abort();
  });
}

/**
 * Create a fresh run. Paths, project, session, operation and unused runDir are required.
 * Each run mints its own RDM session, so commits cannot include caller-session changes.
 * Async rdm() accepts exact argv (including explicit format options); json parses stdout;
 * env forwards explicit RDM_* values to that one child (identity keys cannot be overridden).
 * Failed mutations are uncertain, never retried, and permanently prohibit finish().
 * Existing evidence is never reopened or overwritten. Recovery requires a new run.
 */
export function createRun(spec) {
  const sourceDir = absoluteExisting(spec.sourceDir, 'sourceDir', true);
  const planRoot = absoluteExisting(spec.planRoot, 'planRoot', true);
  const rdmBin = absoluteExisting(spec.rdmBin, 'rdmBin', false);
  const project = textIdentity(spec.project, 'project');
  const parentSession = textIdentity(spec.session, 'session');
  const operation = textIdentity(spec.operation, 'operation');
  const timeout = spec.rdmTimeoutMs ?? 120_000;
  if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 600_000) throw new Error('rdmTimeoutMs must be 1..600000');
  const sourceRoot = fs.realpathSync(safeGit(sourceDir, ['rev-parse', '--show-toplevel']));
  if (sourceRoot !== sourceDir) throw new Error('sourceDir must be the checkout root');
  if (fs.realpathSync(safeGit(planRoot, ['rev-parse', '--show-toplevel'])) !== planRoot) throw new Error('planRoot must be the plan checkout root');
  if (planRoot === sourceDir) throw new Error('planRoot must be separate from sourceDir');
  const sourceBranch = safeGit(sourceDir, ['symbolic-ref', '--quiet', '--short', 'HEAD']);
  const sourceHead = safeGit(sourceDir, ['rev-parse', '--verify', 'HEAD']);
  const planHead = safeGit(planRoot, ['rev-parse', '--verify', 'HEAD']);
  if (typeof spec.runDir !== 'string' || !path.isAbsolute(spec.runDir)) throw new Error('runDir must be explicit and absolute');
  const runDir = path.resolve(spec.runDir);
  const parent = path.dirname(runDir);
  if (fs.realpathSync(parent) !== parent) throw new Error('runDir parent must be canonical without symlinks');
  // Exclusive mkdir also rejects files and dangling symlinks.
  fs.mkdirSync(runDir, { mode: 0o700 }); syncDirectory(parent);
  const session = `codex-${randomUUID()}`;
  const identity = Object.freeze({ sourceDir, sourceRoot, sourceBranch, sourceHead, planRoot, planHead, rdmBin, project, parentSession, session });
  const runner = { pid: process.pid, ppid: process.ppid, executable: process.execPath, argv: [...process.argv], hostname: hostname(), approximateStartedAt: new Date(Date.now() - process.uptime() * 1000).toISOString() };
  const manifest = { version: 1, runner, runId: randomUUID(), operation, identity, status: 'running', startedAt: new Date().toISOString(), uncertainWrites: false };
  let closed = false;
  let callNumber = 0;
  let activeCalls = 0;
  atomicJson(runDir, 'manifest.json', manifest);
  const ensureOpen = () => { if (closed) throw new Error('Run is closed/finished'); };
  const record = (type, data = {}) => {
    ensureOpen();
    const fd = fs.openSync(path.join(runDir, 'journal.jsonl'), 'a', 0o600);
    try { fs.writeFileSync(fd, JSON.stringify({ type, at: new Date().toISOString(), data }) + '\n'); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
    syncDirectory(runDir);
  };
  record('run-started', { identity });
  const finalize = (status, result) => {
    ensureOpen();
    record(`run-${status}`, result);
    atomicJson(runDir, 'result.json', result);
    Object.assign(manifest, { status, finishedAt: new Date().toISOString() });
    atomicJson(runDir, 'manifest.json', manifest); closed = true;
    return result;
  };
  return Object.freeze({
    identity, session, runDir, record,
    async rdm(args, { json = false, mutating = false, env = {} } = {}) {
      ensureOpen();
      if (spec.signal?.aborted) throw new Error('Run cancelled');
      if (activeCalls) throw new Error('A direct RDM command is already running');
      if (!Array.isArray(args) || args.length === 0 || args.some(arg => typeof arg !== 'string' || arg.includes('\0'))) throw new Error('RDM arguments must be a nonempty string array');
      if (args.some(arg => arg === '--all' || arg.startsWith('--all='))) throw new Error('RDM --all is forbidden; commits must use the runtime-owned session');
      if (args.some(arg => ['--root', '--changeset', '--session'].some(flag => arg === flag || arg.startsWith(flag + '=')))) throw new Error('RDM identity override is forbidden');
      if (args[0] === 'commit' && !mutating) throw new Error('Commit requires mutating: true');
      if (manifest.uncertainWrites && mutating) throw new Error('An uncertain write requires manual reconciliation before a new run');
      if (env === null || typeof env !== 'object' || Object.entries(env).some(([key, value]) => !/^RDM_[A-Z0-9_]+$/.test(key) || typeof value !== 'string' || value.includes('\0'))) throw new Error('RDM env overrides must be RDM_* string values');
      const callId = ++callNumber;
      const before = { callId, args, env, session, planHead: safeGit(planRoot, ['rev-parse', '--verify', 'HEAD']) };
      record(mutating ? 'write-intent' : 'read-started', before);
      activeCalls++;
      try {
        const stdout = await directRdm(rdmBin, args, {
          cwd: sourceDir, timeout, signal: spec.signal,
          started: data => record('rdm-started', { callId, ...data }),
          // Caller-forwarded RDM_* values go FIRST so the runtime-owned identity
          // (RDM_BIN/ROOT/PROJECT/SESSION) always wins over them.
          env: { ...env, ...rdmEnvironment(identity) },
        });
        if (spec.signal?.aborted) throw new Error('Run cancelled');
        const value = json ? JSON.parse(stdout) : stdout;
        record(mutating ? 'write-acknowledged' : 'read-completed', { callId, planHead: safeGit(planRoot, ['rev-parse', '--verify', 'HEAD']), stdout });
        return value;
      } catch (error) {
        if (mutating) {
          manifest.uncertainWrites = true;
          atomicJson(runDir, 'manifest.json', manifest);
        }
        record(mutating ? 'write-uncertain' : 'read-failed', { callId, message: error.message, status: error.status ?? null, signal: error.signal ?? null });
        throw error;
      } finally { activeCalls--; }
    },
    finish(result) {
      if (spec.signal?.aborted) throw new Error('Cannot complete cancelled run');
      if (activeCalls) throw new Error('Cannot complete run with active commands');
      if (manifest.uncertainWrites) throw new Error('Cannot complete run with uncertain writes');
      return finalize('completed', result);
    },
    fail(error) {
      ensureOpen();
      if (activeCalls) throw new Error('Cannot fail run with active commands; abort and await them first');
      if (error?.uncertainWrites === true) {
        manifest.uncertainWrites = true;
        atomicJson(runDir, 'manifest.json', manifest);
        record('write-uncertain', { message: error.message, origin: 'adapter-readback' });
      }
      return finalize('failed', { error: error?.message ?? String(error), uncertainWrites: manifest.uncertainWrites });
    },
  });
}
