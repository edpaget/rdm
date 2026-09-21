// estimate-writeback.test.mjs — what the estimate engine's returned writeback
// commands DO, decided by running them.
//
// `rdm-wf-estimate` no longer writes anything itself: `buildEstimatePipeline`
// rates each unestimated phase and hands the caller `writebackCommands` — real
// `rdm` invocations, as text, for the orchestrator to run. That move took the
// only execution those invocations ever got away from them, while making a
// wrong binary path or a wrong/missing project flag cheaper to emit and no
// louder to get wrong.
//
// So this file runs them. It seeds a real plan repo with the real `rdm` binary,
// feeds the binary's own `phase list --format json` into the real pipeline, and
// then executes what it gets back VERBATIM — `writebackScript`, byte for byte,
// with no substitution of any kind, which is what a returned command ladder
// means everywhere else in this lane. It then asserts on plan state read back
// through the binary: the difficulty landed, rdm-core derived the tier from it,
// and the `## Estimate` audit note is in the body alongside the text that was
// there before.
//
// Running it verbatim is the assertion, not a convenience. An earlier revision
// emitted a `--body` whole-document write with a placeholder standing in for the
// current body; run as emitted, that replaced the phase's real body with the
// placeholder text and exited 0. The ladder now appends through `--append-body`
// and holds no body at all, so there is nothing left to splice — and this file
// exercises the invited usage rather than a careful one.
//
// Nothing here asserts on prompt or command TEXT. The commands are run; the
// claims are about the plan repo afterwards. A wrong binary path fails to
// execute (the shell's PATH deliberately excludes any installed `rdm`), and a
// wrong or missing `--project` flag misses the seeded project — whose name is
// deliberately NOT the repo's default — and fails too.
//
// Run by rdm-cli/tests/workflow_estimate_writeback.rs, so `cargo nextest run`
// is the gate.

import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import { buildEstimatePipeline } from '../../.claude/workflows/lib/estimate.mjs';

// --------------------------------------------------------------- environment

const RDM = requiredEnv('RDM_ESTIMATE_TEST_BIN');
const PLAN_ROOT = requiredEnv('RDM_ESTIMATE_TEST_ROOT');

function requiredEnv(name) {
  const v = process.env[name];
  if (!v) {
    throw new Error(
      `${name} is unset. This file executes real rdm commands and is meant to be run by ` +
        'rdm-cli/tests/workflow_estimate_writeback.rs under `cargo nextest run`, which supplies ' +
        'the freshly built binary and an empty temp plan repo.'
    );
  }
  return v;
}

const ROADMAP = 'rm-est';
// Deliberately NOT the plan repo's default project (seeded as `decoy` below):
// every emitted command must carry `--project`, or it resolves the wrong
// project and fails.
const PROJECT = 'est-verify';

// The environment the emitted commands run in. `PATH` carries only the system
// directories, so a command naming a bare `rdm` — or any path other than the
// binary under test — cannot be satisfied by a developer's installed rdm.
// `RDM_ROOT` is how the plan repo is located, since the emitted commands
// (correctly) carry no `--root`. No `RDM_PROJECT`: the flag is the only thing
// that can scope them.
const SHELL_ENV = {
  PATH: '/usr/bin:/bin',
  HOME: PLAN_ROOT,
  XDG_CONFIG_HOME: `${PLAN_ROOT}/nonexistent-config`,
  RDM_ROOT: PLAN_ROOT,
};

// --------------------------------------------------------------- harness

/** Run a shell script the way an orchestrator would, returning its stdout. */
function sh(script) {
  return execFileSync('/bin/bash', ['-euo', 'pipefail', '-c', script], {
    encoding: 'utf8',
    env: SHELL_ENV,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

/** Invoke the binary directly — used only to seed and to read state back. */
function rdm(args) {
  return execFileSync(RDM, ['--root', PLAN_ROOT, ...args], {
    encoding: 'utf8',
    env: SHELL_ENV,
    stdio: ['ignore', 'pipe', 'ignore'],
  });
}

function rdmJson(args) {
  return JSON.parse(rdm(args));
}

function showJson(stem) {
  return rdmJson(['phase', 'show', stem, '--roadmap', ROADMAP, '--project', PROJECT, '--format', 'json']);
}

function listPhases() {
  return rdmJson(['phase', 'list', '--roadmap', ROADMAP, '--project', PROJECT, '--format', 'json']);
}

/**
 * Run the pipeline over the binary's real phase list. Only the difficulty
 * RATING is faked — it is the one judgment call in the loop, and the value it
 * returns is what the writeback has to carry through to disk. The justification
 * deliberately carries a backtick, a `$`, an em-dash and a double quote: the
 * returned command text captures the note through a quoted heredoc, and that
 * only holds if those ride through literally.
 */
async function runPipeline() {
  const rated = [];
  const run = buildEstimatePipeline({
    log: () => {},
    parallelRate: async (stems) => {
      rated.push(stems.slice());
      return stems.map((stem) => ({
        stem,
        difficulty: 'moderate',
        justification: `touches \`${stem}\` — costs $HOME and a "quoted" clause`,
      }));
    },
  });
  const summary = await run({
    roadmap: ROADMAP,
    project: PROJECT,
    rdmBin: RDM,
    phaseList: listPhases(),
  });
  return { summary, rated };
}

/**
 * Execute one phase's returned writeback ladder EXACTLY as emitted — the whole
 * `writebackScript`, in one shell session, with nothing substituted, edited or
 * reordered. This is the usage the returned data invites; anything the ladder
 * cannot do unaided it must not claim to do.
 *
 * Returns the JSON printed by the LAST command in the ladder, which is the
 * read-back the engine tells its caller is how the resulting tier is observed.
 * It is re-run on its own only to get that JSON unmixed from the update's human
 * status line — the verbatim run above it is what does the work.
 */
function runWritebackVerbatim(entry) {
  sh(entry.writebackScript);
  return JSON.parse(sh(entry.writebackCommands[entry.writebackCommands.length - 1]));
}

// --------------------------------------------------------------- seed

// A real plan repo, built with the real binary. `decoy` is the repo default, so
// the project flag the emitted commands carry is load-bearing. Phase 3 is
// pre-estimated, so the run must leave it alone.
rdm(['init', '--default-project', 'decoy']);
rdm(['project', 'create', PROJECT, '--title', 'Estimate Verify']);
rdm(['roadmap', 'create', ROADMAP, '--title', 'Estimate RM', '--body', 'seed', '--no-edit', '--project', PROJECT]);
for (const [slug, number, body] of [
  ['a', '1', 'ORIGINAL BODY A'],
  ['b', '2', 'ORIGINAL BODY B'],
  ['c', '3', 'ORIGINAL BODY C'],
]) {
  rdm([
    'phase', 'create', slug,
    '--title', slug.toUpperCase(),
    '--number', number,
    '--body', body,
    '--no-edit',
    '--roadmap', ROADMAP,
    '--project', PROJECT,
  ]);
}
rdm(['phase', 'update', 'phase-3-c', '--difficulty', 'hard', '--no-edit', '--roadmap', ROADMAP, '--project', PROJECT]);

// --------------------------------------------------------------- tests

test('the returned writeback commands land the difficulty, the core-derived tier and the audit note', async () => {
  const { summary, rated } = await runPipeline();

  assert.deepEqual(
    rated,
    [['phase-1-a', 'phase-2-b']],
    'the real phase list selects exactly the two unestimated phases for rating'
  );
  assert.deepEqual(
    summary.estimated.map((e) => e.stem),
    ['phase-1-a', 'phase-2-b'],
    'both rated phases come back with writeback commands'
  );
  assert.deepEqual(summary.skipped, ['phase-3-c'], 'the pre-estimated phase is never rated');

  for (const e of summary.estimated) {
    const readBack = runWritebackVerbatim(e);

    assert.equal(readBack.difficulty, 'moderate', `${e.stem}: the rated difficulty was persisted`);
    assert.equal(
      readBack.model,
      'medium',
      `${e.stem}: rdm-core derived the tier from the difficulty — the commands pass no --model`
    );

    const body = readBack.body || '';
    const original = `ORIGINAL BODY ${e.stem.slice(-1).toUpperCase()}`;
    assert.ok(
      body.includes(original),
      `${e.stem}: the pre-existing body survived the ladder run verbatim — the exact loss the ` +
        'placeholder-in-a---body-heredoc revision caused, silently and with exit 0'
    );
    assert.ok(body.includes('## Estimate'), `${e.stem}: the audit note landed`);
    assert.ok(
      body.includes(`moderate — ${e.justification}`),
      `${e.stem}: the note carries "<difficulty> — <justification>" with its backtick, $ and quote intact`
    );
    assert.ok(
      body.indexOf(original) < body.indexOf('## Estimate'),
      `${e.stem}: the note is appended after the existing body, not prepended over it`
    );
  }

  // The read-back the commands perform agrees with an independent read.
  for (const e of summary.estimated) {
    const shown = showJson(e.stem);
    assert.equal(shown.difficulty, 'moderate');
    assert.equal(shown.model, 'medium');
  }
});

test('the already-estimated phase is left exactly as it was', () => {
  const c = showJson('phase-3-c');
  assert.equal(c.difficulty, 'hard', 'its difficulty is untouched');
  assert.equal(c.model, 'large', 'its core-derived tier is untouched');
  assert.ok(!(c.body || '').includes('## Estimate'), 'no audit note was appended to it');
  assert.ok((c.body || '').includes('ORIGINAL BODY C'), 'its body is untouched');
});

test('a second pass over the now-written repo has nothing left to estimate', async () => {
  const { summary, rated } = await runPipeline();

  assert.deepEqual(rated, [], 'no phase is rated a second time — nothing to rate means no fan-out at all');
  assert.deepEqual(summary.estimated, [], 'no writeback commands are returned on a re-run');
  assert.deepEqual(
    summary.skipped,
    ['phase-1-a', 'phase-2-b', 'phase-3-c'],
    'all three phases now read back as estimated, so the run is idempotent'
  );
});
