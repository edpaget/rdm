// Experimental subprocess seam for the phase-2 spike, not a production adapter.
import {spawn} from 'node:child_process';
import {mkdtemp, writeFile, readFile, rm, mkdir, stat} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {performance} from 'node:perf_hooks';

const LIMIT = 8 * 1024 * 1024;
const KEYWORDS = new Set(['type', 'properties', 'required', 'additionalProperties', 'items', 'enum',
  'minimum', 'maximum', 'minLength', 'maxLength', 'minItems', 'maxItems', 'description', 'title']);
const TYPES = new Set(['object', 'array', 'string', 'boolean', 'integer', 'number', 'null']);
// Diagnostics are private local evidence, never console output. Redact common
// credential spellings; callers must not put credentials in model prompts.
function redact(text) {
  return text.replace(/\bBearer\s+\S+/gi, 'Bearer [REDACTED]')
    .replace(/\bsk-[A-Za-z0-9_-]+/g, '[REDACTED]')
    .replace(/((?:api[_-]?key|access[_-]?token|refresh[_-]?token|authorization)\s*[=:]\s*)[^\s,}]+/gi, '$1[REDACTED]');
}
function checkSchema(schema) {
  if (!schema || typeof schema !== 'object' || Array.isArray(schema)) throw Error('Unsupported schema');
  for (const key of Object.keys(schema)) if (!KEYWORDS.has(key)) throw Error(`Unsupported schema keyword: ${key}`);
  if (!TYPES.has(schema.type)) throw Error('Unsupported schema type');
  if (schema.type === 'object') {
    if (schema.additionalProperties !== false || !schema.properties) throw Error('Unsupported open object schema');
    for (const child of Object.values(schema.properties)) checkSchema(child);
    for (const key of schema.required ?? []) if (!Object.hasOwn(schema.properties, key)) throw Error('Invalid required schema property');
  }
  if (schema.type === 'array') checkSchema(schema.items);
}

/** Validate the closed JSON Schema subset used by the canonical spike workflows. */
export function validateSchema(value, schema) {
  checkSchema(schema);
  function validate(value, rule, path) {
    const fail = () => {throw Error(`Response failed schema validation at ${path}`);};
    const matches = rule.type === 'null' ? value === null
      : rule.type === 'array' ? Array.isArray(value)
      : rule.type === 'object' ? value !== null && typeof value === 'object' && !Array.isArray(value)
      : rule.type === 'integer' ? Number.isInteger(value)
      : rule.type === 'number' ? typeof value === 'number' && Number.isFinite(value)
      : typeof value === rule.type;
    if (!matches) fail();
    if (rule.enum && !rule.enum.some(entry => JSON.stringify(entry) === JSON.stringify(value))) fail();
    if (typeof value === 'number' && (value < (rule.minimum ?? -Infinity) || value > (rule.maximum ?? Infinity))) fail();
    if (typeof value === 'string' && ([...value].length < (rule.minLength ?? 0) || [...value].length > (rule.maxLength ?? Infinity))) fail();
    if (rule.type === 'object') {
      for (const key of rule.required ?? []) if (!Object.hasOwn(value, key)) fail();
      for (const key of Object.keys(value)) {
        if (!Object.hasOwn(rule.properties, key)) fail();
        validate(value[key], rule.properties[key], `${path}.${key}`);
      }
    }
    if (rule.type === 'array') {
      if (value.length < (rule.minItems ?? 0) || value.length > (rule.maxItems ?? Infinity)) fail();
      value.forEach((entry, index) => validate(entry, rule.items, `${path}[${index}]`));
    }
  }
  validate(value, schema, '$');
  return true;
}

function strictSchema(schema) {
  const result = {...schema};
  if (schema.type === 'object') {
    result.required = Object.keys(schema.properties);
    result.properties = Object.fromEntries(Object.entries(schema.properties).map(([key, child]) => [key,
      (schema.required ?? []).includes(key) ? strictSchema(child) : {anyOf: [strictSchema(child), {type: 'null'}]},
    ]));
  }
  if (schema.type === 'array') result.items = strictSchema(schema.items);
  return result;
}
function removeOptionalNulls(value, schema) {
  if (schema.type === 'object' && value && typeof value === 'object' && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(schema.properties)) {
      if (value[key] === null && !(schema.required ?? []).includes(key) && child.type !== 'null') delete value[key];
      else if (Object.hasOwn(value, key)) removeOptionalNulls(value[key], child);
    }
  } else if (schema.type === 'array' && Array.isArray(value)) value.forEach(entry => removeOptionalNulls(entry, schema.items));
}

/** Run fresh read-only Codex execution; reject incomplete/error streams and invalid responses. */
export async function runCodex({bin = 'codex', cwd, prompt, schema, model, effort = 'medium',
  timeoutMs = 120000, signal, env = process.env, evidenceDir, label = 'call', resumeThreadId,
  persistSession = false}) {
  checkSchema(schema);
  if (!model || typeof model !== 'string') throw Error('Explicit model is required');
  if (!['minimal', 'low', 'medium', 'high', 'xhigh'].includes(effort)) throw Error('Unsupported reasoning effort');
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw Error('Positive timeout is required');
  if (typeof prompt !== 'string') throw Error('Prompt must be a string');
  if (signal?.aborted) throw Error('Codex call cancelled');
  if (!/^[a-zA-Z0-9_.:-]+$/.test(label) || label === '.' || label === '..') throw Error('Invalid evidence label');
  const temp = await mkdtemp(join(tmpdir(), 'rdm-codex-spike-'));
  const started = performance.now();
  let capturedStdout = '', capturedStderr = '';
  try {
    const schemaPath = join(temp, 'schema.json'), outputPath = join(temp, 'output.json');
    await writeFile(schemaPath, JSON.stringify(strictSchema(schema)), {mode: 0o600});
    const args = ['exec', '--sandbox', 'read-only', '-c', `model_reasoning_effort=${JSON.stringify(effort)}`];
    if (resumeThreadId) args.push('resume');
    args.push('--ignore-user-config', '--json', '--output-schema', schemaPath, '-m', model, '-o', outputPath);
    if (!persistSession && !resumeThreadId) args.push('--ephemeral');
    if (resumeThreadId) args.push(resumeThreadId);
    args.push('-');
    const stdout = await new Promise((resolve, reject) => {
      const child = spawn(bin, args, {cwd, env, detached: process.platform !== 'win32', stdio: ['pipe', 'pipe', 'pipe']});
      let bytes = 0, output = '', failure, closed = false;
      const killGroup = () => {
        if (!child.pid) return;
        try {
          if (process.platform === 'win32') child.kill('SIGKILL');
          else process.kill(-child.pid, 'SIGKILL');
        } catch (error) {if (error.code !== 'ESRCH') failure ??= Error('Unable to terminate Codex child group');}
      };
      const stop = reason => {failure ??= Error(reason); killGroup();};
      const abort = () => stop('Codex call cancelled');
      const interrupt = () => stop('Codex call cancelled by process signal');
      const timer = setTimeout(() => stop('Codex call timed out'), timeoutMs);
      signal?.addEventListener('abort', abort, {once: true});
      process.on('SIGINT', interrupt);
      process.on('SIGTERM', interrupt);
      child.on('error', () => {failure ??= Error('Unable to start Codex subprocess');});
      child.stdin.on('error', () => {if (!closed) stop('Codex stdin failed');});
      child.on('exit', () => killGroup()); // Also remove descendants left behind by a dead parent.
      child.on('close', (code, deathSignal) => {
        closed = true;
        clearTimeout(timer);
        signal?.removeEventListener('abort', abort);
        process.removeListener('SIGINT', interrupt);
        process.removeListener('SIGTERM', interrupt);
        if (failure) reject(failure);
        else if (code !== 0 || deathSignal) reject(Error(`Codex subprocess failed (exit ${code ?? 'signal'})`));
        else resolve(output);
      });
      const consume = (data, keep) => {
        bytes += Buffer.byteLength(data);
        if (bytes > LIMIT) stop('Codex output exceeded 8 MiB');
        else if (keep) {output += data; capturedStdout = output;}
        else capturedStderr += data;
      };
      child.stdout.setEncoding('utf8');
      child.stdout.on('data', data => consume(data, true));
      child.stderr.on('data', data => consume(data, false));
      if (signal?.aborted) abort();
      child.stdin.end(prompt);
    });
    let events;
    try {
      if (!stdout.endsWith('\n')) throw Error();
      events = stdout.trim().split('\n').map(line => JSON.parse(line));
      if (events.some(event => !event || typeof event.type !== 'string')) throw Error();
    } catch {throw Error('Codex returned malformed or truncated JSONL');}
    if (events.some(event => event.type === 'error' || event.type === 'turn.failed' ||
      event.item?.status === 'failed' || event.item?.type === 'error')) throw Error('Codex stream reported failure');
    const threads = events.filter(event => event.type === 'thread.started');
    const completions = events.filter(event => event.type === 'turn.completed');
    if (threads.length !== 1 || typeof threads[0].thread_id !== 'string' || !threads[0].thread_id ||
      completions.length !== 1 || events.at(-1).type !== 'turn.completed') throw Error('Codex stream missing unique thread/completion');
    if (resumeThreadId && threads[0].thread_id !== resumeThreadId) throw Error('Resumed Codex thread identity changed');
    if ((await stat(outputPath)).size > LIMIT) throw Error('Codex final output exceeded 8 MiB');
    let value;
    try {value = JSON.parse(await readFile(outputPath, 'utf8'));}
    catch {throw Error('Codex final response is missing or malformed');}
    removeOptionalNulls(value, schema);
    validateSchema(value, schema);
    return {value, threadId: threads[0].thread_id, usage: completions[0].usage ?? null,
      elapsedMs: Math.round(performance.now() - started), events};
  } finally {
    try {
      if (evidenceDir) {
        await mkdir(evidenceDir, {recursive: true, mode: 0o700});
        await writeFile(join(evidenceDir, `${label}.jsonl`), redact(capturedStdout), {mode: 0o600, flag: 'wx'});
        await writeFile(join(evidenceDir, `${label}.stderr.txt`), redact(capturedStderr), {mode: 0o600, flag: 'wx'});
      }
    } finally {await rm(temp, {recursive: true, force: true});}
  }
}

/** Run at most limit tasks, preserving order and awaiting all active work on failure. */
export async function boundedParallel(thunks, limit = 2) {
  if (!Number.isInteger(limit) || limit < 1) throw Error('Positive concurrency limit is required');
  const values = new Array(thunks.length);
  let next = 0, failure, failed = false;
  async function worker() {
    while (!failed && next < thunks.length) {
      const index = next++;
      try {values[index] = await thunks[index]();}
      catch (error) {if (!failed) failure = error; failed = true;}
    }
  }
  await Promise.all(Array.from({length: Math.min(limit, thunks.length)}, worker));
  if (failed) throw failure;
  return values;
}
