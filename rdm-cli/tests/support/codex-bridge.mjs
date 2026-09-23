// Test transport only. Rust owns scenarios, callbacks, fixtures and assertions.
import readline from 'node:readline';
import {pathToFileURL} from 'node:url';
const lines = readline.createInterface({input: process.stdin});
const pending = new Map();
let sequence = 0;
const send = value => process.stdout.write(`${JSON.stringify(value)}\n`);
function hydrate(value) {
  if (Array.isArray(value)) return value.map(hydrate);
  if (!value || typeof value !== 'object') return value;
  if (value.$callback) return (...args) => new Promise((resolve, reject) => {
    const id = ++sequence;
    pending.set(id, {resolve, reject});
    send({type: 'callback', id, name: value.$callback, args});
  });
  if (value.$builtin === 'parallel') return tasks => Promise.all(tasks.map(task => task().catch(() => null)));
  if (value.$builtin === 'pipeline') return (items, ...stages) => Promise.all(items.map(async item => {
    for (const stage of stages) item = await stage(item);
    return item;
  }));
  if (value.$builtin === 'abortedSignal') return AbortSignal.abort();
  if (value.$builtin === 'noop') return () => {};
  return Object.fromEntries(Object.entries(value).map(([key, entry]) => [key, hydrate(entry)]));
}
lines.on('line', async line => {
  const message = JSON.parse(line);
  if (message.type === 'reply') {
    const call = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) call.reject(new Error(message.error));
    else call.resolve(message.value);
    return;
  }
  try {
    const module = await import(pathToFileURL(message.module));
    let value = await module[message.export](...hydrate(message.args));
    if (message.callArgs) value = await value(...hydrate(message.callArgs));
    if (message.steps) {
      const object = value;
      value = [];
      for (const step of message.steps) {
        try { value.push({value: await object[step.method](...hydrate(step.args)) ?? null}); }
        catch (error) { value.push({error: error.message}); }
      }
    }
    send({type: 'result', value: value ?? null});
  } catch (error) {
    if (message.captureError) send({type: 'result', value: {error: {message: error.message, ...error}}});
    else send({type: 'result', error: error.message});
  }
  lines.close();
  process.stdin.destroy();
});
