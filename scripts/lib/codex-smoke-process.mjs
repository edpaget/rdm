import {spawn} from 'node:child_process';

// Test-only live-process lifecycle. Register interruption cleanup before preparing
// private auth, and await child shutdown before resolving/rejecting. SIGKILL and
// machine failure cannot run cleanup; the smoke docs explain that limitation.
export function runSmokeProcess(bin, args, {
  env = process.env, cwd, prepare = () => {}, cleanup, timeoutMs = 120000,
}) {
  return new Promise((resolve, reject) => {
    let child;
    let timer;
    let finished = false;
    let failure;
    let output = '';
    let bytes = 0;
    const terminate = reason => {
      failure ??= new Error(reason);
      if (child?.pid) {
        try {
          if (process.platform === 'win32') child.kill('SIGKILL');
          else process.kill(-child.pid, 'SIGKILL'); // Only this detached test child group.
        } catch (error) { if (error.code !== 'ESRCH') failure = error; }
      }
    };
    const interrupt = () => terminate('Live Codex check interrupted');
    const finish = error => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      process.removeListener('SIGINT', interrupt);
      process.removeListener('SIGTERM', interrupt);
      try { cleanup(); } catch (cleanupError) { error = cleanupError; }
      if (error) reject(error); else resolve(output);
    };
    process.on('SIGINT', interrupt);
    process.on('SIGTERM', interrupt);
    try {
      prepare();
      child = spawn(bin, args, {env, cwd, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe']});
      child.on('error', finish);
      child.on('close', code => finish(failure ?? (code === 0 ? null : new Error(`Live Codex exited with code ${code}`))));
      child.stdout.on('data', data => {
        bytes += data.length;
        if (bytes > 8 * 1024 * 1024) terminate('Live Codex output exceeded 8 MiB');
        else output += data;
      });
      child.stderr.resume();
      timer = setTimeout(() => terminate('Live Codex check timed out'), timeoutMs);
    } catch (error) { finish(error); }
  });
}
