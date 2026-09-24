// Generic execution glue for `rdm_devtools::workflow` (see that module's
// rustdoc for the protocol). It loads and runs whatever JavaScript the Rust
// side names and ferries values across a line-delimited JSON channel.
//
// CONTRACT: this file holds no tests, no scenarios, no expected values, no
// assertions, no branches on fixture content and no review logic. It only
// imports modules, compiles function bodies, encodes/decodes values, keeps a
// handle table, forwards callbacks and reports thrown errors. The one host
// primitive it implements, `parallel`, is documented in the Rust module.

import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';

const protocolOut = process.stdout;
const send = (msg) => protocolOut.write(JSON.stringify(msg) + '\n');

// Workflow code may log freely; nothing it prints may reach the protocol
// stream, so every console method is routed to stderr.
for (const method of ['log', 'info', 'warn', 'error', 'debug', 'trace']) {
  console[method] = (...parts) => {
    process.stderr.write(parts.map((p) => (typeof p === 'string' ? p : String(p))).join(' ') + '\n');
  };
}

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

const handles = new Map();
let nextHandle = 1;
function keep(value) {
  const n = nextHandle++;
  handles.set(n, value);
  return n;
}
function lookup(n) {
  if (!handles.has(n)) throw new Error('unknown handle ' + n);
  return handles.get(n);
}

function serializeError(e, file) {
  const out =
    e && typeof e === 'object'
      ? { name: String(e.name || 'Error'), message: String(e.message || ''), stack: String(e.stack || '') }
      : { name: typeof e, message: String(e), stack: '' };
  // Node does not put the module path on an ESM syntax error, so a failed
  // import names the file it was loading.
  if (file) out.file = file;
  return out;
}

function encode(value, seen) {
  if (value === undefined) return { $undefined: true };
  if (value === null || typeof value === 'boolean' || typeof value === 'string') return value;
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw new TypeError('cannot encode non-finite number ' + String(value));
    return value;
  }
  if (typeof value === 'function') return { $fn: keep(value) };
  if (typeof value !== 'object') throw new TypeError('cannot encode a value of type ' + typeof value);
  const stack = seen || new Set();
  if (stack.has(value)) throw new TypeError('cannot encode a cyclic value');
  stack.add(value);
  let out;
  if (Array.isArray(value)) {
    out = value.map((v) => encode(v, stack));
  } else {
    out = {};
    for (const [k, v] of Object.entries(value)) out[k] = encode(v, stack);
  }
  stack.delete(value);
  return out;
}

const callbackFns = new Map();
const pendingCallbacks = new Map();
let nextCallId = 1;
function callbackFn(n) {
  if (!callbackFns.has(n)) {
    callbackFns.set(n, (...args) => {
      const callId = nextCallId++;
      return new Promise((resolve, reject) => {
        pendingCallbacks.set(callId, { resolve, reject });
        send({ op: 'callback', callId, fn: n, args: args.map((a) => encode(a)) });
      });
    });
  }
  return callbackFns.get(n);
}

const hostPrimitives = {
  // Order-preserving; a thunk that throws or rejects resolves to null.
  parallel: (thunks) => Promise.all(thunks.map((t) => Promise.resolve().then(t).catch(() => null))),
};
function hostPrimitive(name) {
  if (Object.prototype.hasOwnProperty.call(hostPrimitives, name)) return hostPrimitives[name];
  return () => {
    throw new Error('unsupported host primitive: ' + name + ' (the rdm_devtools::workflow fake host implements parallel only)');
  };
}

function singleKey(value, key) {
  const keys = Object.keys(value);
  return keys.length === 1 && keys[0] === key;
}

function decode(value) {
  if (Array.isArray(value)) return value.map(decode);
  if (value && typeof value === 'object') {
    if (singleKey(value, '$fn')) return lookup(value.$fn);
    if (singleKey(value, '$ref')) return lookup(value.$ref);
    if (singleKey(value, '$callback')) return callbackFn(value.$callback);
    if (singleKey(value, '$host')) return hostPrimitive(value.$host);
    if (singleKey(value, '$undefined')) return undefined;
    const out = {};
    for (const [k, v] of Object.entries(value)) out[k] = decode(v);
    return out;
  }
  return value;
}

async function handle(msg) {
  switch (msg.op) {
    case 'import':
      return { $ref: keep(await import(pathToFileURL(msg.path).href)) };
    case 'compile':
      return encode(new AsyncFunction(...msg.params, msg.source));
    case 'get':
      return encode(lookup(msg.handle)[msg.member]);
    case 'call': {
      const target = decode(msg.target);
      if (typeof target !== 'function') throw new TypeError('call target is not a function');
      return encode(await target(...decode(msg.args)));
    }
    default:
      throw new Error('unknown request op: ' + String(msg.op));
  }
}

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', (line) => {
  const msg = JSON.parse(line);
  if (msg.op === 'shutdown') process.exit(0);
  if (msg.op === 'reply') {
    const waiter = pendingCallbacks.get(msg.callId);
    if (!waiter) throw new Error('reply for unknown callback ' + msg.callId);
    pendingCallbacks.delete(msg.callId);
    if (msg.error !== undefined) waiter.reject(new Error(String(msg.error)));
    else waiter.resolve(decode(msg.value));
    return;
  }
  Promise.resolve()
    .then(() => handle(msg))
    .then(
      (value) => send({ op: 'ok', id: msg.id, value }),
      (error) => send({ op: 'err', id: msg.id, error: serializeError(error, msg.op === 'import' ? msg.path : '') })
    );
});
input.on('close', () => process.exit(0));
