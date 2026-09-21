// review-driver.test.mjs — what the code engine's standalone path DOES,
// decided by driving it and then running what it hands back.
//
// `.claude/workflows/rdm-wf-review-refute-fix.js` is the one engine in this lane
// that ships downstream (`rdm-core/src/templates/workflows/` and
// `plugins/rdm/workflows/` carry byte-identical copies). Its `mode: 'code'` +
// `{roadmap, phase}` / `{task}` branch was rewritten to read nothing and write
// nothing: the pinned source identity arrives as caller arguments, and the
// status write it used to perform through an agent comes back as `gateCommands`
// / `gateScript` for the orchestrator to run.
//
// That move took the only execution those writes ever got away from them. A
// dropped `--no-edit`, a missing `--expected-branch`, or an outcome mapped to
// the wrong status is cheap to emit, silent at emit time, and lands in the
// shipped template and the plugin tree byte for byte.
//
// So this file runs them. It seeds a real plan repo and a real source repo with
// the real `rdm` binary, registers a roadmap worktree and a task worktree the
// way `rdm worktree add` does, drives the real engine under a fake reviewer
// fleet, executes the `gateScript` it returns in a shell, and asserts on the
// plan state read back through the binary afterwards.
//
// Nothing here asserts on prompt text, and nothing greps the engine's source.
// The claims are about what the plan repo looks like once the emitted commands
// have run, and about what the driver returns for inputs that must not produce
// commands at all.
//
// Run by rdm-cli/tests/workflow_review_driver.rs, so `cargo nextest run` is the
// gate.

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// --------------------------------------------------------------- environment

const RDM = requiredEnv('RDM_REVIEW_TEST_BIN');
const PLAN_ROOT = requiredEnv('RDM_REVIEW_TEST_ROOT');
const SRC_ROOT = requiredEnv('RDM_REVIEW_TEST_SRC');

function requiredEnv(name) {
  const v = process.env[name];
  if (!v) {
    throw new Error(
      `${name} is unset. This file executes real rdm and git commands and is meant to be run by ` +
        'rdm-cli/tests/workflow_review_driver.rs under `cargo nextest run`, which supplies the ' +
        'freshly built binary and two empty temp directories.'
    );
  }
  return v;
}

const checkout = fileURLToPath(new URL('../../', import.meta.url));
const WORKFLOW = path.join(checkout, '.claude/workflows/rdm-wf-review-refute-fix.js');

const ROADMAP = 'rm-rev';
const TASK = 'standalone-task';
// Deliberately NOT the plan repo's default project (seeded as `decoy`): every
// emitted command must carry `--project`, or it resolves the wrong project and
// fails.
const PROJECT = 'rev-verify';

// `PATH` carries only the system directories, so a command naming a bare `rdm`
// — or any path other than the binary under test — cannot be satisfied by a
// developer's installed rdm. `RDM_ROOT` locates the plan repo, since the
// emitted commands (correctly) carry no `--root`.
const SHELL_ENV = {
  PATH: '/usr/bin:/bin',
  HOME: PLAN_ROOT,
  XDG_CONFIG_HOME: `${PLAN_ROOT}/nonexistent-config`,
  RDM_ROOT: PLAN_ROOT,
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_SYSTEM: '/dev/null',
  GIT_AUTHOR_NAME: 'Review Verify',
  GIT_AUTHOR_EMAIL: 'review@example.invalid',
  GIT_COMMITTER_NAME: 'Review Verify',
  GIT_COMMITTER_EMAIL: 'review@example.invalid',
};

// --------------------------------------------------------------- harness

/** Run a shell script the way an orchestrator would, returning its stdout. */
function sh(script, cwd) {
  return execFileSync('/bin/bash', ['-euo', 'pipefail', '-c', script], {
    encoding: 'utf8',
    cwd: cwd || PLAN_ROOT,
    env: SHELL_ENV,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

/**
 * The same script in a PLAIN shell — no `-e`, no `pipefail`. A caller pasting a
 * returned ladder into an existing session gets exactly this, so the ladder has
 * to carry its own failure handling. Returns the exit status rather than
 * throwing, because the status is the thing under test.
 */
function shPlainStatus(script, cwd) {
  return spawnSync('/bin/bash', ['-c', script], {
    encoding: 'utf8',
    cwd: cwd || PLAN_ROOT,
    env: SHELL_ENV,
    stdio: ['ignore', 'pipe', 'pipe'],
  }).status;
}

/** Invoke the binary directly — used only to seed and to read state back. */
function rdm(args, cwd) {
  return execFileSync(RDM, ['--root', PLAN_ROOT, ...args], {
    encoding: 'utf8',
    cwd: cwd || PLAN_ROOT,
    env: SHELL_ENV,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

function git(args, cwd) {
  return execFileSync('/usr/bin/git', args, {
    encoding: 'utf8',
    cwd,
    env: SHELL_ENV,
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

function phaseJson(stem) {
  return JSON.parse(rdm(['phase', 'show', stem, '--roadmap', ROADMAP, '--project', PROJECT, '--format', 'json']));
}

function taskJson(slug) {
  return JSON.parse(rdm(['task', 'show', slug, '--project', PROJECT, '--format', 'json']));
}

// The engine, loaded the way the Workflow runtime loads it: a top-level script
// with ambient `agent`/`pipeline`/`parallel`/`log`. `meta` is stripped because a
// function body cannot carry an export.
const rawWorkflow = fs.readFileSync(WORKFLOW, 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const driver = new AsyncFunction(
  'args',
  'agent',
  'pipeline',
  'parallel',
  'log',
  rawWorkflow.replace(/export const meta\s*=\s*\{[\s\S]*?\n\}/, '')
);
const referenceParallel = async (thunks) =>
  await Promise.all(
    thunks.map(async (fn) => {
      try {
        return await fn();
      } catch {
        return null;
      }
    })
  );
const referencePipeline = async (items, ...stages) => {
  let out = items;
  for (const stage of stages) out = await stage(out);
  return out;
};

// A clean AC table: every criterion PASSes, so `classifyOutcome` has complete
// evidence and nothing gating survives.
const CLEAN_AC = [{ criterion: 'AC1: it works', status: 'PASS', evidence: 'the tests cover it' }];

/**
 * A fake reviewer fleet. `finding` (optional) is planted by the `correctness`
 * finder and graded un-refuted, so it survives to the outcome — which is how a
 * `rework` run is produced without any judgment actually running.
 */
function makeFleet(finding) {
  const labels = [];
  return {
    labels,
    agent: async (_prompt, opts) => {
      const label = (opts && opts.label) || '';
      labels.push(label);
      if (label === 'find:code:ac') return { ac: CLEAN_AC, findings: [] };
      if (label === 'find:code:correctness' && finding) return { findings: [finding] };
      if (label.startsWith('refute:')) return { refuted: false, confidence: 95 };
      return { findings: [] };
    },
  };
}

async function drive(args, finding) {
  const fleet = makeFleet(finding);
  const logs = [];
  const result = await driver(args, fleet.agent, referencePipeline, referenceParallel, (m) => logs.push(String(m)));
  return { result, labels: fleet.labels, logs };
}

// --------------------------------------------------------------- seed

// A real plan repo. `decoy` is the repo default, so the `--project` flag the
// emitted commands carry is load-bearing.
rdm(['init', '--default-project', 'decoy']);
rdm(['project', 'create', PROJECT, '--title', 'Review Verify']);
rdm(['roadmap', 'create', ROADMAP, '--title', 'Review RM', '--body', 'seed', '--no-edit', '--project', PROJECT]);
for (const [slug, number] of [
  ['clean', '1'],
  ['dirty', '2'],
]) {
  rdm([
    'phase', 'create', slug,
    '--title', slug.toUpperCase(),
    '--number', number,
    '--body', '## Acceptance criteria\n\n- AC1: it works',
    '--no-edit',
    '--roadmap', ROADMAP,
    '--project', PROJECT,
  ]);
}
rdm(['task', 'create', TASK, '--title', 'Standalone', '--body', '## Acceptance criteria\n\n- AC1: it works', '--no-edit', '--project', PROJECT]);

// A real source repo, and the two registered checkouts the pinned identity has
// to name. `rdm worktree add`, run from inside the source repo, is what
// registers them — an unregistered path is refused by the real binary, which is
// precisely the kind of thing an emitted-text assertion cannot notice.
fs.mkdirSync(SRC_ROOT, { recursive: true });
git(['init', '--quiet', '-b', 'main', '.'], SRC_ROOT);
git(['commit', '--quiet', '--allow-empty', '-m', 'chore: initial'], SRC_ROOT);

function addWorktree(ref) {
  return execFileSync(RDM, ['--root', PLAN_ROOT, 'worktree', 'add', ref, '--project', PROJECT], {
    encoding: 'utf8',
    cwd: SRC_ROOT,
    env: SHELL_ENV,
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

/** One real commit on a worktree's branch, so the reviewed range is non-empty. */
function pin(worktree, filename) {
  fs.writeFileSync(path.join(worktree, filename), 'shipped\n');
  git(['add', filename], worktree);
  git(['commit', '--quiet', '-m', 'feat: ' + filename], worktree);
  return {
    source: worktree,
    base: git(['rev-parse', 'main'], worktree),
    expectedHead: git(['rev-parse', 'HEAD'], worktree),
    expectedBranch: git(['rev-parse', '--abbrev-ref', 'HEAD'], worktree),
  };
}

const ROADMAP_PIN = pin(addWorktree(ROADMAP), 'roadmap-work.txt');
const TASK_PIN = pin(addWorktree('task/' + TASK), 'task-work.txt');

const COMMON = { mode: 'code', rdmBin: RDM, project: PROJECT, gate: true };

// --------------------------------------------------------------- tests

test('a clean review gates the phase all the way to reviewed, by running the emitted commands', async () => {
  const { result } = await drive({ ...COMMON, roadmap: ROADMAP, phase: 'phase-1-clean', ...ROADMAP_PIN });

  assert.equal(result.outcome, 'reviewed');
  assert.equal(result.status, 'reviewed');
  assert.equal(result.writesCompletion, true, 'a reviewed outcome tells the lander it may write the trailer');
  assert.ok(result.gateScript, 'a reviewed outcome with gate:true emits a ladder');

  assert.equal(phaseJson('phase-1-clean').status, 'not-started', 'nothing was written before the ladder ran');
  sh(result.gateScript);
  assert.equal(
    phaseJson('phase-1-clean').status,
    'reviewed',
    'the emitted ladder really drove the phase to reviewed against the real binary'
  );
});

test('a rework outcome gates to in-progress, not reviewed — the same ladder, a different destination', async () => {
  const { result } = await drive(
    { ...COMMON, roadmap: ROADMAP, phase: 'phase-2-dirty', ...ROADMAP_PIN },
    { id: 'real-bug', concern: 'correctness', severity: 'blocking', confidence: 95, what_fails: 'it drops a write' }
  );

  assert.equal(result.outcome, 'rework');
  assert.equal(result.writesCompletion, false, 'a rework outcome must not authorise a completion trailer');
  assert.equal(result.findings.length, 1, 'the un-refuted blocking finding survived to the outcome');

  sh(result.gateScript);
  const after = phaseJson('phase-2-dirty');
  assert.equal(after.status, 'in-progress', 'the ladder parked the phase for rework');
  assert.notEqual(after.status, 'reviewed', 'a rework outcome can never land on reviewed');
});

test('a task target writes through `task update`, not `phase update`', async () => {
  const { result } = await drive({ ...COMMON, task: TASK, ...TASK_PIN });

  assert.equal(result.task, TASK);
  assert.equal(result.roadmap, undefined, 'a task result carries no roadmap/phase identity');
  assert.equal(result.outcome, 'reviewed');

  assert.equal(taskJson(TASK).status, 'open', 'nothing was written before the ladder ran');
  sh(result.gateScript);
  assert.equal(taskJson(TASK).status, 'reviewed', 'the emitted ladder drove the real task to reviewed');
});

test('a refused write fails the ladder, even in a plain shell with no set -e', async () => {
  const { result } = await drive({ ...COMMON, roadmap: ROADMAP, phase: 'phase-1-clean', ...ROADMAP_PIN });

  // The source binding is what makes the write refusable. Move the branch on,
  // and the same ladder must now fail rather than stamping a status against a
  // checkout that is no longer the one that was reviewed.
  fs.writeFileSync(path.join(ROADMAP_PIN.source, 'moved-on.txt'), 'later\n');
  git(['add', 'moved-on.txt'], ROADMAP_PIN.source);
  git(['commit', '--quiet', '-m', 'feat: moved on'], ROADMAP_PIN.source);

  // A plain shell, because that is what a caller pasting the ladder into an
  // existing session has. The ladder's last line is a read-back, so without
  // per-line failure handling the session would report the READ's success and
  // the refused write would vanish.
  assert.notEqual(
    shPlainStatus(result.gateScript),
    0,
    'a refused status write must fail the ladder, not be masked by the trailing read-back'
  );

  // Restore the pin so later tests see the head they were seeded with.
  git(['reset', '--quiet', '--hard', ROADMAP_PIN.expectedHead], ROADMAP_PIN.source);
});

test('a reviewer set with no `ac` escalates with a message naming what is missing', async () => {
  const { result } = await drive({
    ...COMMON,
    roadmap: ROADMAP,
    phase: 'phase-1-clean',
    ...ROADMAP_PIN,
    reviewers: ['correctness', 'tests'],
  });

  // Nothing REFUSES a thin set — the run happens, and the outcome is the honest
  // consequence of reviewing a diff with no acceptance-criteria evidence.
  assert.equal(result.outcome, 'escalated');
  assert.equal(result.gateCommands, undefined, 'an escalated, evidence-incomplete run emits no ladder');
  assert.match(result.summary, /ac/, 'the summary names the reviewer whose absence caused the park');
  assert.match(result.summary, /NO AC TABLE/, 'and the coverage channel says the table is ABSENT, not clean');
  assert.equal(
    result.reviewCoverage.acTableAbsent,
    true,
    'an UNSELECTED ac reviewer counts as absent, exactly like one that failed'
  );
});

test('an unresolvable source produces no commands at all and escalates', async () => {
  for (const [name, broken] of [
    ['a short head', { ...ROADMAP_PIN, expectedHead: 'abc123' }],
    ['a non-hex head', { ...ROADMAP_PIN, expectedHead: 'z'.repeat(40) }],
    ['a missing branch', { ...ROADMAP_PIN, expectedBranch: '' }],
    ['a missing base', { ...ROADMAP_PIN, base: undefined }],
    ['a missing source path', { ...ROADMAP_PIN, source: '   ' }],
  ]) {
    const { result, labels } = await drive({ ...COMMON, roadmap: ROADMAP, phase: 'phase-1-clean', ...broken, persist: true });
    assert.equal(result.outcome, 'escalated', `${name}: an unpinned review cannot report a verdict`);
    assert.equal(result.gateCommands, undefined, `${name}: no gate ladder is emitted`);
    assert.equal(result.gateScript, undefined, `${name}: and no gate script either`);
    assert.equal(result.persistCommands, undefined, `${name}: nor a persist ladder`);
    assert.deepEqual(labels, [], `${name}: the refusal precedes every agent dispatch`);
  }
});

test('gate: false returns the verdict and writes nothing — the interactive skills\' shape', async () => {
  const { result } = await drive({ ...COMMON, gate: false, roadmap: ROADMAP, phase: 'phase-1-clean', ...ROADMAP_PIN });
  assert.equal(result.outcome, 'reviewed');
  assert.equal(result.gateCommands, undefined, 'no ladder is offered when the caller keeps its own gate');
  assert.equal(
    phaseJson('phase-1-clean').status,
    'reviewed',
    'and the phase is exactly where the earlier ladder left it — this call wrote nothing'
  );
});

test('naming both a task and phase identifiers is refused before anything runs', async () => {
  await assert.rejects(
    () => drive({ ...COMMON, task: TASK, roadmap: ROADMAP, phase: 'phase-1-clean', ...ROADMAP_PIN }),
    /ambiguous review target/
  );
});

test('the two legacy survivors-only shapes still return their original report', async () => {
  for (const mode of ['code', 'plan']) {
    const { result } = await drive({ mode, context: { target: 'legacy' } });
    assert.equal(result.mode, mode);
    assert.ok(Array.isArray(result.survivors));
    assert.equal(result.outcome, undefined, 'a legacy report approves nothing');
    assert.equal(result.gateCommands, undefined, 'and emits no ladder, having no item to write to');
  }
});
