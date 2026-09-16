import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {existsSync, mkdtempSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {test} from 'node:test';
import {runSmokeProcess} from './codex-smoke-process.mjs';

test('cleans up after success, failure, and timeout', async () => {
  for (const [code, timeoutMs, succeeds] of [
    ['console.log("ok")', 2000, true], ['process.exit(3)', 2000, false],
    ['setInterval(() => {}, 1000)', 30, false],
  ]) {
    let cleaned = false;
    const result = runSmokeProcess(process.execPath, ['-e', code], {
      cleanup: () => { cleaned = true; }, timeoutMs,
    });
    if (succeeds) assert.equal((await result).trim(), 'ok');
    else await assert.rejects(result);
    assert(cleaned);
  }
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`cleans up a non-secret marker on ${signal}`, {timeout: 5000}, async () => {
    const dir = mkdtempSync(join(tmpdir(), 'rdm-smoke-signal-'));
    const marker = join(dir, 'not-a-credential');
    writeFileSync(marker, 'test');
    const code = `import {runSmokeProcess} from ${JSON.stringify(new URL('./codex-smoke-process.mjs', import.meta.url).href)};
      import {rmSync} from 'node:fs';
      const result = runSmokeProcess(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
        cleanup: () => rmSync(${JSON.stringify(marker)}, {force: true}), timeoutMs: 2000,
      });
      console.log('ready');
      try { await result; } catch { process.exitCode = 1; }`;
    const worker = spawn(process.execPath, ['--input-type=module', '-e', code], {stdio: ['ignore', 'pipe', 'pipe']});
    try {
      await new Promise((resolve, reject) => {
        worker.on('error', reject);
        worker.stdout.once('data', () => worker.kill(signal));
        worker.on('exit', (exitCode, exitSignal) => {
          try { assert.notEqual(exitCode, 0); assert.equal(exitSignal, null); resolve(); }
          catch (error) { reject(error); }
        });
      });
      assert(!existsSync(marker));
    } finally {
      worker.kill();
      rmSync(dir, {recursive: true, force: true});
    }
  });
}
