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
import { hasBlocking, refutePrompt, formatCommentBody, parseCommentHeader, buildReviewPipeline, classifyOutcome } from '../../.claude/workflows/lib/review.mjs';

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
  // The persist ladder runs `rdm review start`, which refuses rather than guess
  // an author; the hermetic environment above has no git identity to fall back on.
  RDM_REVIEW_AUTHOR: 'review-verify',
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
 * A fake reviewer fleet. `finding` (optional) is planted findings that survive
 * un-refuted, so an outcome is produced without any judgment actually running.
 * A bare object plants ONE finding on `correctness` (the pre-existing shape,
 * kept so every single-finding caller is unaffected); an ARRAY plants one
 * finding per entry, distributed in order across `correctness`, `tests`,
 * `architecture`, `api-docs`, `changelog`, `security` — up to six at once.
 */
const CODE_FINDING_DIMS = ['correctness', 'tests', 'architecture', 'api-docs', 'changelog', 'security'];
function makeFleet(finding) {
  const labels = [];
  const findings = Array.isArray(finding) ? finding : finding ? [finding] : [];
  return {
    labels,
    agent: async (_prompt, opts) => {
      const label = (opts && opts.label) || '';
      labels.push(label);
      if (label === 'find:code:ac') return { ac: CLEAN_AC, findings: [] };
      for (let i = 0; i < findings.length; i++) {
        if (label === 'find:code:' + CODE_FINDING_DIMS[i]) return { findings: [findings[i]] };
      }
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
// A real implementation plan. `rdm review start --on change/<sha>` records which
// plan the reviewed change implements, and a ROADMAP-wide worktree covers more
// than one phase, so the binary refuses to infer it — the engine's optional
// `implements` argument is the documented way to name it, and only a plan that
// really exists satisfies the write.
const PLAN = 'rev-verify-plan';
rdm([
  'plan', 'create', PLAN,
  '--title', 'Review Verify Plan',
  '--implements', 'phase/' + ROADMAP + '/phase-2-dirty',
  '--body', 'the approved plan',
  '--no-edit',
  '--project', PROJECT,
]);

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

test('the persist ladder records a real review: a real `--path` code anchor from the finder-supplied path field, a build-time-degraded anchor, and a whole-document-by-design finding', async () => {
  const { result } = await drive(
    { ...COMMON, gate: false, persist: true, implements: 'plan/' + PLAN, roadmap: ROADMAP, phase: 'phase-2-dirty', ...ROADMAP_PIN },
    [
      {
        id: 'anchored-bug',
        concern: 'correctness',
        severity: 'blocking',
        confidence: 95,
        what_fails: 'it drops a write',
        // A messy `location` shaped like the live evidence that motivated this
        // fix (a trailing "(mirrored at ...)" parenthetical defeats
        // pathFromLocation's end-anchored suffix strip) — the explicit `path`
        // field must be used directly, never parsed out of this prose.
        location: 'roadmap-work.txt:1 (mirrored at README.md:9-12)',
        path: 'roadmap-work.txt',
        quote: 'shipped',
      },
      {
        id: 'degraded-bug',
        concern: 'tests',
        severity: 'concern',
        confidence: 90,
        what_fails: 'coverage gap',
        // Prose-only location: no derivable repo-relative path, and no `path`
        // field either — the requested anchor is dropped at BUILD TIME.
        location: 'throughout the gate step',
        quote: 'shipped',
      },
      {
        id: 'whole-doc-note',
        concern: 'architecture',
        severity: 'suggestion',
        confidence: 80,
        what_fails: 'general note',
        location: 'general',
        // No `quote` at all: this finding never asked for an anchor.
      },
    ]
  );

  assert.ok(result.persistScript, 'a persist:true run emits a ladder');
  const out = sh(result.persistScript);
  const id = /reviewId=(\S+)/.exec(out);
  assert.ok(id, 'the ladder prints the id it created: ' + out);

  const review = JSON.parse(rdm(['review', 'show', id[1], '--project', PROJECT, '--format', 'json']));
  assert.equal(review.state, 'submitted');
  assert.equal(review.target.kind, 'change', 'the reviewed artifact is the pinned change, not the phase document');
  assert.equal(review.comments.length, 3, 'each survivor is persisted exactly once');

  const anchored = review.comments.find((c) => c.body.includes('anchored-bug'));
  assert.equal(anchored.anchor.anchor_type, 'file-quote', 'a change review anchors into the source file, not the document');
  assert.equal(
    anchored.anchor.path,
    'roadmap-work.txt',
    "the finder's explicit `path` field landed the anchor, not a parse of the messy `location`"
  );
  assert.equal(anchored.anchor.quote, 'shipped');
  assert.equal(
    parseCommentHeader(anchored.body).anchor,
    'path',
    'AC1/AC2: a real, structured path lands as an anchored comment, header-marked `anchor: path`, not `path-missing` degradation'
  );

  const degraded = review.comments.find((c) => c.body.includes('degraded-bug'));
  assert.equal(degraded.anchor, undefined, 'no repo-relative path could be derived from either field, so this landed whole-document');
  assert.equal(
    parseCommentHeader(degraded.body).anchor,
    'degraded',
    'AC4: a dropped anchor is header-marked `degraded`, distinct from a whole-document-by-design finding'
  );

  const wholeDoc = review.comments.find((c) => c.body.includes('whole-doc-note'));
  assert.equal(wholeDoc.anchor, undefined);
  assert.equal(
    parseCommentHeader(wholeDoc.body).anchor,
    'wholeDocumentIntended',
    'AC3: a finding that legitimately names no file is recorded as wholeDocumentIntended, never as a degraded anchor'
  );

  // AC4's other half: the review's OWN summary states the build-time
  // degradation — a review carrying a dropped anchor cannot read as clean
  // persistence, even on a run where every anchor did not fail (one of the two
  // requested anchors landed here).
  assert.match(
    review.body,
    /1 of 2 requested anchor\(s\) could not be placed at persist time \(path-missing x1\); see the `anchor` header on each comment\./,
    "the review's own summary names the build-time degradation"
  );
});

test('a whole-document-by-design finding, alone, never triggers the build-time degradation clause', async () => {
  // No `path`, no derivable `location`, and — critically — no `quote` at all:
  // this finding never ASKED for an anchor, so it must never read as one that
  // was dropped. gate: true here (unlike the mixed-findings test above), so
  // this also proves a lone whole-document finding still gates cleanly to
  // `reviewed`. `implements` is passed explicitly, same as the mixed-findings
  // test above: this worktree covers the whole roadmap, so the real binary has
  // no single plan to infer from the item alone.
  const { result } = await drive(
    { ...COMMON, persist: true, implements: 'plan/' + PLAN, roadmap: ROADMAP, phase: 'phase-1-clean', ...ROADMAP_PIN },
    [
      {
        id: 'note-only',
        concern: 'correctness',
        severity: 'suggestion',
        confidence: 80,
        what_fails: 'a general observation, not tied to any one file',
        location: 'general',
      },
    ]
  );
  assert.equal(result.outcome, 'reviewed');
  assert.ok(result.persistScript, 'a persist:true run emits a ladder');
  const out = sh(result.persistScript);
  const id = /reviewId=(\S+)/.exec(out);
  assert.ok(id, 'the ladder prints the id it created: ' + out);

  const review = JSON.parse(rdm(['review', 'show', id[1], '--project', PROJECT, '--format', 'json']));
  assert.equal(review.comments.length, 1);
  assert.equal(review.comments[0].anchor, undefined, 'a quote-less finding has no anchor to land at all');
  assert.equal(
    parseCommentHeader(review.comments[0].body).anchor,
    'wholeDocumentIntended',
    'AC3: never confused with a dropped anchor'
  );
  assert.doesNotMatch(
    review.body,
    /requested anchor\(s\) could not be placed at persist time/,
    'AC4 negative control: a whole-document-by-design finding, with nothing else degraded in the run, must not read as a build-time degradation'
  );
});

test('a refused `review start` fails the persist ladder, even in a plain shell with no set -e', async () => {
  const { result } = await drive({ ...COMMON, gate: false, persist: true, task: TASK, ...TASK_PIN });
  assert.ok(result.persistScript, 'a persist:true run emits a ladder');

  // Refuse `review start` and NOTHING ELSE — the ladder's opening `review
  // source` line already carried `|| exit 1`, so a stub that refused everything
  // would stop there and prove nothing about the rest. The ladder's last line is
  // a `printf`, so without per-line failure handling the shell reports THAT
  // command's status: the caller, whose skills make the exit status the whole
  // success signal, saw 0, recorded an empty `reviewId=`, and believed a review
  // existed that had never been created.
  const stub = path.join(PLAN_ROOT, 'refusing-rdm');
  fs.writeFileSync(stub, '#!/bin/sh\n[ "$2" = start ] || exit 0\necho "error: refused" >&2\nexit 1\n', { mode: 0o755 });
  const refusing = result.persistScript.split(RDM).join(stub);

  assert.notEqual(
    shPlainStatus(refusing),
    0,
    'a refused review start must fail the ladder, not be masked by the trailing printf'
  );
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

// ----------------------------------------------------------- scope grading
//
// `refuters-grade-finding-scope` (0129ddf) added a refuter-graded `inScope`
// verdict field with zero automated coverage — the accepted rework finding
// this section fixes. These are pure-function/driven-pipeline tests against
// the real exports of `.claude/workflows/lib/review.mjs` (imported directly,
// above); nothing here greps source text or asserts on a file's contents, and
// nothing re-verifies that some other surface still matches this module —
// each case exercises real behavior of the functions under test.

test('hasBlocking excludes an inScope:false finding at both gating tiers, and gates it when inScope is true or omitted', () => {
  const blocking = { severity: 'blocking', confidence: 90 };
  const concern = { severity: 'concern', confidence: 90 };

  // Default tier: only `blocking` gates; `concern` never does, with or without
  // a scope verdict — included as a control, not part of the matrix.
  assert.equal(hasBlocking([{ ...blocking, inScope: false }], undefined), false, 'default tier: inScope:false must not gate');
  assert.equal(hasBlocking([{ ...blocking, inScope: true }], undefined), true, 'default tier: inScope:true still gates');
  assert.equal(hasBlocking([{ ...blocking }], undefined), true, 'default tier: omitted inScope still gates (fail-safe default)');

  // `large` tier: a surviving `concern` gates too — the one-directional
  // tightening `hasBlocking`'s own comment describes.
  assert.equal(hasBlocking([{ ...concern, inScope: false }], 'large'), false, 'large tier, concern: inScope:false must not gate');
  assert.equal(hasBlocking([{ ...concern, inScope: true }], 'large'), true, 'large tier, concern: inScope:true still gates');
  assert.equal(hasBlocking([{ ...concern }], 'large'), true, 'large tier, concern: omitted inScope still gates (fail-safe default)');

  // And `blocking` keeps gating the same way at the large tier too.
  assert.equal(hasBlocking([{ ...blocking, inScope: false }], 'large'), false, 'large tier, blocking: inScope:false must not gate');
  assert.equal(hasBlocking([{ ...blocking, inScope: true }], 'large'), true, 'large tier, blocking: inScope:true still gates');
  assert.equal(hasBlocking([{ ...blocking }], 'large'), true, 'large tier, blocking: omitted inScope still gates (fail-safe default)');
});

test('refutePrompt appends the scope clause only for mode:"code" with a non-empty context.planCommand', () => {
  const dim = { key: 'correctness' };
  const finding = { id: 'f1', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'it drops a write' };
  const SCOPE_MARKER = /Grade whether this finding is IN SCOPE/;
  const PLAN_COMMAND = 'rdm plan show scope-flip --format json';

  const codeWithPlan = refutePrompt('code', dim, finding, { target: 'T', planCommand: PLAN_COMMAND });
  assert.match(codeWithPlan, SCOPE_MARKER, 'mode:code with a planCommand appends the scope clause');

  // The other three (mode, planCommand-presence) combinations must never carry
  // it — a plan-mode call, and a code-mode call with no associated plan.
  const codeNoKey = refutePrompt('code', dim, finding, { target: 'T' });
  const codeUndefined = refutePrompt('code', dim, finding, { target: 'T', planCommand: undefined });
  const codeEmpty = refutePrompt('code', dim, finding, { target: 'T', planCommand: '' });
  const planNoKey = refutePrompt('plan', dim, finding, { target: 'T' });
  const planWithPlan = refutePrompt('plan', dim, finding, { target: 'T', planCommand: PLAN_COMMAND });

  for (const [label, prompt] of [
    ['code, no planCommand key', codeNoKey],
    ['code, planCommand: undefined', codeUndefined],
    ['code, planCommand: ""', codeEmpty],
    ['plan, no planCommand key', planNoKey],
    ['plan, planCommand set', planWithPlan],
  ]) {
    assert.doesNotMatch(prompt, SCOPE_MARKER, label + ' must not carry the scope clause');
  }

  // Byte-identity: an absent key, an explicit `undefined`, and an explicit
  // empty string are three equivalent spellings of "no plan associated", and
  // in `code` mode they must produce the exact same prompt — the precise
  // non-empty-string guard the 56-item refuter-agreement corpus (whose
  // regenerated context is always `{ target: item.target }`, never carrying a
  // `planCommand` key) depends on never moving a byte for a corpus item.
  assert.equal(codeNoKey, codeUndefined, 'an absent key and an explicit undefined must be byte-identical');
  assert.equal(codeNoKey, codeEmpty, 'an absent key and an explicit empty string must be byte-identical');
});

test('a blocking finding graded inScope:false still survives but yields "reviewed"; inScope:true yields "rework" — driven end to end', async () => {
  const CLEAN_AC = [{ criterion: 'AC1: it works', status: 'PASS', evidence: 'covered' }];

  function scopeFleet(inScope) {
    return async (_prompt, opts) => {
      const label = (opts && opts.label) || '';
      if (label === 'find:code:ac') return { ac: CLEAN_AC, findings: [] };
      if (label === 'find:code:correctness') {
        return { findings: [{ id: 'scope-bug', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'it drops a write' }] };
      }
      if (label.startsWith('refute:')) return { refuted: false, confidence: 90, inScope };
      return { findings: [] };
    };
  }

  async function run(inScope) {
    const runReview = buildReviewPipeline('code', {
      agent: scopeFleet(inScope),
      pipeline: referencePipeline,
      parallel: referenceParallel,
      log: () => {},
    });
    return runReview({
      reviewers: ['ac', 'correctness'],
      planCommand: 'rdm plan show scope-flip --format json',
      target: 'the change under review',
    });
  }

  const outOfScope = await run(false);
  assert.equal(outOfScope.survivors.length, 1, 'the blocking finding survives refutation — it is never dropped');
  assert.equal(outOfScope.survivors[0].inScope, false);
  assert.equal(
    // `codeReviews: [survivors]` is the shape the real caller
    // (rdm-wf-review-refute-fix.js, `classifyOutcome({ ..., codeReviews: [survivors], ... })`)
    // passes for a single completed review round.
    classifyOutcome({ codeReviews: [outOfScope.survivors], acTable: outOfScope.acTable }),
    'reviewed',
    'an out-of-scope blocking survivor must not force rework'
  );

  const inScopeRun = await run(true);
  assert.equal(inScopeRun.survivors.length, 1, 'the identical finding still survives when graded in scope');
  assert.equal(inScopeRun.survivors[0].inScope, true);
  assert.equal(
    classifyOutcome({ codeReviews: [inScopeRun.survivors], acTable: inScopeRun.acTable }),
    'rework',
    'the same finding graded in scope forces rework — the outcome flips solely on the refuter verdict'
  );
});

test('formatCommentBody / parseCommentHeader round-trip inScope: true, inScope: false, and no inScope at all (the n/a sentinel)', () => {
  const base = { id: 'f1', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'it drops a write' };

  const trueBody = formatCommentBody({ ...base, inScope: true });
  assert.match(trueBody, /^inScope: true$/m);
  assert.equal(parseCommentHeader(trueBody).inScope, true);

  const falseBody = formatCommentBody({ ...base, inScope: false });
  assert.match(falseBody, /^inScope: false$/m);
  assert.equal(parseCommentHeader(falseBody).inScope, false);

  const ungradedBody = formatCommentBody({ ...base });
  assert.match(ungradedBody, /^inScope: n\/a$/m);
  assert.equal(parseCommentHeader(ungradedBody).inScope, null, 'never graded for scope parses back to null, not false');
});
