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
import { execFileSync, spawnSync, spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import {
  hasBlocking,
  refutePrompt,
  formatCommentBody,
  parseCommentHeader,
  buildReviewPipeline,
  classifyOutcome,
  persistAnchorFor,
  persistDegradationNoteBody,
  persistDegradationGateLines,
} from '../../.claude/workflows/lib/review.mjs';

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

/**
 * A plain shell run with EXTRA environment variables layered over
 * `SHELL_ENV`, returning the full `{ status, stdout, stderr }` rather than
 * just the exit code — used by the RDM_PERSIST_ANCHORS_DEGRADED gate test
 * below, which has to assert both the exit status AND that the refusal
 * actually lands on stderr, for a variable no other test in this file ever
 * sets (tests-1).
 */
function shPlainWithEnv(script, extraEnv, cwd) {
  return spawnSync('/bin/bash', ['-c', script], {
    encoding: 'utf8',
    cwd: cwd || PLAN_ROOT,
    env: { ...SHELL_ENV, ...extraEnv },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

/**
 * Runs `script` in a plain shell the way an orchestrator's Bash tool does —
 * stdin a pipe that is never written to and never closed — and resolves once
 * the process exits, or rejects if it doesn't within `timeoutMs`.
 *
 * This is deliberately `child_process.spawn`, not `spawnSync`/`execFileSync`:
 * confirmed by experiment that `spawnSync` with `stdio: 'pipe'` and no
 * `input` closes the child's stdin immediately (EOF) and does NOT reproduce
 * the hang this regression targets — only `spawn`, with the stdin stream
 * left open and never `.end()`ed, does. Held-open stdin is exactly the shape
 * of an agent's Bash tool: the emitted ladder must run to completion under
 * it with NO redirect added by the caller (the ladder's own `< /dev/null`
 * lines, per line, are what save it — see persistReviewCommands).
 */
function runWithOpenStdinPipe(script, timeoutMs = 8000) {
  return new Promise((resolve, reject) => {
    const child = spawn('/bin/bash', ['-c', script], {
      cwd: PLAN_ROOT,
      env: SHELL_ENV,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    // Held open deliberately: never written to, never `.end()`ed.
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => {
      stdout += d;
    });
    child.stderr.on('data', (d) => {
      stderr += d;
    });
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error('script did not exit within ' + timeoutMs + 'ms with stdin held open:\n' + stdout + stderr));
    }, timeoutMs);
    child.on('close', (code) => {
      clearTimeout(timer);
      resolve({ code, stdout, stderr });
    });
    child.on('error', (err) => {
      clearTimeout(timer);
      reject(err);
    });
  });
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

/**
 * One real commit on a worktree's branch, so the reviewed range is non-empty.
 * `filenames` is a single path or an array of paths, all committed together —
 * a subdirectory path (e.g. `'dir/roadmap-work.txt'`) is created on demand,
 * so a test needing a file OUTSIDE the worktree root (the suffixed-path
 * regression below) doesn't need its own separate pinned commit.
 */
function pin(worktree, filenames) {
  const files = Array.isArray(filenames) ? filenames : [filenames];
  for (const f of files) {
    const full = path.join(worktree, f);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, 'shipped\n');
    git(['add', f], worktree);
  }
  git(['commit', '--quiet', '-m', 'feat: ' + files.join(', ')], worktree);
  return {
    source: worktree,
    base: git(['rev-parse', 'main'], worktree),
    expectedHead: git(['rev-parse', 'HEAD'], worktree),
    expectedBranch: git(['rev-parse', '--abbrev-ref', 'HEAD'], worktree),
  };
}

const ROADMAP_PIN = pin(addWorktree(ROADMAP), ['roadmap-work.txt', 'dir/roadmap-work.txt']);
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
  assert.equal(review.verdict, 'request-changes', 'a rework outcome persists as request-changes');
  assert.equal(review.target.kind, 'change', 'the reviewed artifact is the pinned change, not the phase document');
  // Each survivor lands its own comment PLUS the ladder's own degradation
  // note comment (correctness-1: this run degrades one of two requested
  // anchors, so RDM_PERSIST_TOTAL_DEGRADED > 0 and the note is appended).
  assert.equal(review.comments.length, 4, 'each survivor is persisted exactly once, plus one degradation note');

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

  // correctness-1: the persisted review itself now carries a whole-document
  // NOTE comment naming the REAL (build-time-plus-run-time) degraded total —
  // not just the build-time-only clause folded into `review.body` below —
  // so a review whose anchors degraded is never a single stdout line away
  // from being mistaken for clean persistence. Exactly this note's text,
  // produced by the same `persistDegradationNoteBody` the emitted ladder's
  // `printf` uses, with the run-time-only-known total (1) filled in.
  const note = review.comments.find((c) => c.body.startsWith('persist-note: anchors-degraded'));
  assert.ok(note, 'a nonzero degraded total must append exactly one whole-document note comment');
  assert.equal(note.anchor, undefined, 'the note is whole-document, never a file anchor');
  assert.equal(note.body, persistDegradationNoteBody(1, 2), 'the note names the real total against what was requested');

  // AC4's other half: the review's OWN summary states the build-time
  // degradation — a review carrying a dropped anchor cannot read as clean
  // persistence, even on a run where every anchor did not fail (one of the two
  // requested anchors landed here).
  assert.match(
    review.body,
    /1 of 2 requested anchor\(s\) could not be placed at persist time \(path-missing x1\); see the `anchor` header on each comment\./,
    "the review's own summary names the build-time degradation"
  );

  // AC4's machine-readable half, over a PARTIALLY-degraded run: one of the two
  // requested anchors landed, so `all` must stay false even though one did
  // degrade — a caller must be able to tell "some" from "every" without
  // parsing the prose clause above.
  assert.deepEqual(
    result.persistDegraded,
    { requested: 2, degraded: 1, all: false },
    'a partially-degraded review must not set `all`'
  );
  assert.match(out, /^anchorsDegraded=partial$/m, 'the ladder itself prints the partial disposition');
});

test('a finder-declared `path` carrying a `:line` suffix still lands a real anchor, and the ladder completes', async () => {
  // A finder prompt-matching the adjacent `location: <path>:<line>` line
  // sometimes tacks a line suffix onto the STRUCTURED `path` field too, even
  // though the prompt asks for a bare path there. Regression coverage: the
  // fixture MUST be a `:line`-suffixed path that CONTAINS A SLASH
  // (`dir/roadmap-work.txt:1`), with a prose-only `location` that no fallback
  // can rescue it through. A slash-free suffixed path like
  // `roadmap-work.txt:1` does NOT exercise the fix at all — `isRepoRelativePath`
  // already REJECTS it outright (no `/`, and `.txt:1` does not match the
  // extension regex), so if the `stripPathLineSuffix` call in
  // `persistAnchorFor` were deleted, the declared path would be rejected and
  // the code would fall back to `pathFromLocation('roadmap-work.txt:1')`,
  // landing the SAME anchor either way — a test built on that fixture cannot
  // fail no matter which behavior runs. `dir/roadmap-work.txt:1` DOES contain
  // a slash, so `isRepoRelativePath` accepts it whole, suffix and all,
  // without the strip: the real binary would then be handed a literal
  // `--path 'dir/roadmap-work.txt:1'`, which names no file, and (now that a
  // refused path-anchored comment retries whole-document rather than
  // aborting the ladder — see the runtime-degradation test below) the
  // comment would land `anchor: degraded` instead of `anchor: path`. Either
  // way this test's assertions below only pass when the strip is applied.
  const { result } = await drive(
    { ...COMMON, gate: false, persist: true, implements: 'plan/' + PLAN, roadmap: ROADMAP, phase: 'phase-2-dirty', ...ROADMAP_PIN },
    {
      id: 'suffixed-path-bug',
      concern: 'correctness',
      severity: 'blocking',
      confidence: 90,
      what_fails: 'it drops a write',
      // Prose-only, deliberately not itself a derivable repo-relative path —
      // so `pathFromLocation` cannot rescue a broken strip either.
      location: 'see the gate step',
      // The finder-declared path carries both a subdirectory AND the `:1`
      // line suffix this finding's fix strips before validating.
      path: 'dir/roadmap-work.txt:1',
      quote: 'shipped',
    }
  );

  assert.ok(result.persistScript, 'a persist:true run emits a ladder');
  const out = sh(result.persistScript);
  const id = /reviewId=(\S+)/.exec(out);
  assert.ok(id, 'the ladder completes and prints the id it created: ' + out);
  assert.match(out, /^anchorsDegraded=none$/m, 'the suffixed path still lands, so nothing degraded');

  const review = JSON.parse(rdm(['review', 'show', id[1], '--project', PROJECT, '--format', 'json']));
  assert.equal(review.comments.length, 1);
  assert.equal(review.comments[0].anchor.anchor_type, 'file-quote', 'the suffixed path must still land a real file anchor');
  assert.equal(
    review.comments[0].anchor.path,
    'dir/roadmap-work.txt',
    'the `:1` line suffix must be stripped from the declared path before it is used'
  );
  assert.equal(
    parseCommentHeader(review.comments[0].body).anchor,
    'path',
    'a landed anchor, never `degraded` — the suffix is tolerated, not treated as an invalid path'
  );
  assert.deepEqual(result.persistDegraded, { requested: 1, degraded: 0, all: false });
});

test('a path-anchored comment refused at RUN TIME (quote outside a touched hunk) is retried whole-document by the ladder itself, and anchorsDegraded reports it', async () => {
  // Every prior test in this file covers BUILD-TIME degradation (no derivable
  // path at all). This is the run-time half (ac-1/correctness-1/ac-2/arch-1):
  // a finding whose declared `path` passes every build-time check — a real,
  // in-range file — but whose `quote` sits OUTSIDE every hunk the reviewed
  // range actually touches. `persistAnchorFor` has no way to see this ahead
  // of time; only the real binary, at `rdm review comment` time, refuses it
  // (`Error::QuoteOutsideChangedHunks`, "is not touched by <base>..<head>").
  //
  // rdm generates its hunks with `--unified=0` (see rdm-core/src/change.rs),
  // so a change touching only ONE line of a two-line file leaves the OTHER
  // line entirely outside any hunk — genuinely "outside a touched hunk", not
  // an approximation. Two real commits on the roadmap worktree's branch
  // construct exactly that: the first adds `outside-hunk.txt` with a line
  // that will stay untouched (`keepme`) and one that will change; BASE is
  // pinned there. The second changes only the second line; HEAD is pinned
  // there. The finding quotes the UNTOUCHED first line.
  const src = ROADMAP_PIN.source;
  fs.writeFileSync(path.join(src, 'outside-hunk.txt'), 'keepme\nchangeme\n');
  git(['add', 'outside-hunk.txt'], src);
  git(['commit', '--quiet', '-m', 'feat: add outside-hunk.txt'], src);
  const base = git(['rev-parse', 'HEAD'], src);
  fs.writeFileSync(path.join(src, 'outside-hunk.txt'), 'keepme\nchanged\n');
  git(['add', 'outside-hunk.txt'], src);
  git(['commit', '--quiet', '-m', 'feat: change only the second line'], src);
  const head = git(['rev-parse', 'HEAD'], src);
  const branch = git(['rev-parse', '--abbrev-ref', 'HEAD'], src);

  const { result } = await drive(
    {
      ...COMMON,
      gate: false,
      persist: true,
      implements: 'plan/' + PLAN,
      roadmap: ROADMAP,
      phase: 'phase-2-dirty',
      source: src,
      base,
      expectedHead: head,
      expectedBranch: branch,
    },
    {
      id: 'outside-hunk-bug',
      concern: 'correctness',
      severity: 'blocking',
      confidence: 90,
      what_fails: 'looks fine, but is not',
      location: 'see the gate step',
      path: 'outside-hunk.txt',
      quote: 'keepme',
    }
  );

  assert.ok(result.persistScript, 'a persist:true run emits a ladder');
  const out = sh(result.persistScript);
  const id = /reviewId=(\S+)/.exec(out);
  assert.ok(id, 'the ladder completes (exit 0) even though the anchor was refused at run time: ' + out);
  assert.match(out, /^anchorsDegraded=all$/m, 'the ladder run-time-degraded its only anchored finding, so `all` fires');

  const review = JSON.parse(rdm(['review', 'show', id[1], '--project', PROJECT, '--format', 'json']));
  // The one finding's comment, plus the ladder's own degradation note —
  // correctness-1: the note is what makes a run-time-only-degraded review
  // ever readable as anything other than clean persistence, since this
  // finding's own comment carries no build-time trace at all.
  assert.equal(review.comments.length, 2);
  const finding = review.comments.find((c) => c.body.includes('outside-hunk-bug'));
  assert.ok(finding, 'the finding comment must still be present');
  assert.equal(finding.anchor, undefined, 'the run-time-refused anchor must land whole-document, not as a file anchor');
  assert.equal(
    parseCommentHeader(finding.body).anchor,
    'degraded',
    'a run-time-refused anchor is header-marked `degraded`, exactly like a build-time one'
  );
  // correctness-1: the note names the REAL total (1 of 1) — a total the
  // build-time-only `persistDegradedSummary` computation below cannot see at
  // all, since nothing had degraded yet when it ran.
  const note = review.comments.find((c) => c.body.startsWith('persist-note: anchors-degraded'));
  assert.ok(note, 'a run-time-only degradation must still append the note comment');
  assert.equal(note.body, persistDegradationNoteBody(1, 1), 'the note reports the run-time result, not a build-time zero');
  // AC4's build-time-only preview cannot see this: nothing degraded until the
  // ladder actually ran. This is exactly why a caller must key its park
  // decision off the ladder's own printed `anchorsDegraded=` line, never off
  // `result.persistDegraded` alone (correctness-1/arch-1).
  assert.deepEqual(
    result.persistDegraded,
    { requested: 1, degraded: 0, all: false },
    'the build-time-only preview must not see a run-time refusal'
  );

  // Restore the shared worktree's branch so later tests reusing
  // `...ROADMAP_PIN` (whose `expectedHead` was captured once, at seed time)
  // still see the head they were pinned against.
  git(['reset', '--quiet', '--hard', ROADMAP_PIN.expectedHead], src);
});

test('a mixed run — one path-anchored comment lands, another is refused at run time — prints anchorsDegraded=partial', async () => {
  // Same run-time mechanism as the previous test, but alongside a finding
  // that lands cleanly, proving the tally distinguishes "some" from "every".
  const src = ROADMAP_PIN.source;
  // `untouched.txt` is committed BEFORE `base`, so it exists unchanged at
  // both ends of the reviewed range — not part of the diff at all, which the
  // real binary refuses the same way as a quote outside a touched hunk
  // ("is not touched by <base>..<head>").
  fs.writeFileSync(path.join(src, 'untouched.txt'), 'never touched\n');
  git(['add', 'untouched.txt'], src);
  git(['commit', '--quiet', '-m', 'feat: seed a file the next commit will not touch'], src);
  const base = git(['rev-parse', 'HEAD'], src);
  fs.writeFileSync(path.join(src, 'landed.txt'), 'shipped-anchor\n');
  git(['add', 'landed.txt'], src);
  git(['commit', '--quiet', '-m', 'feat: add landed.txt'], src);
  const head = git(['rev-parse', 'HEAD'], src);
  const branch = git(['rev-parse', '--abbrev-ref', 'HEAD'], src);

  const { result } = await drive(
    {
      ...COMMON,
      gate: false,
      persist: true,
      implements: 'plan/' + PLAN,
      roadmap: ROADMAP,
      phase: 'phase-2-dirty',
      source: src,
      base,
      expectedHead: head,
      expectedBranch: branch,
    },
    [
      {
        id: 'lands-fine',
        concern: 'correctness',
        severity: 'blocking',
        confidence: 90,
        what_fails: 'a real bug, correctly anchored',
        location: 'see the gate step',
        path: 'landed.txt',
        quote: 'shipped-anchor',
      },
      {
        id: 'refused-at-runtime',
        concern: 'tests',
        severity: 'concern',
        confidence: 85,
        what_fails: 'coverage gap, quote names an untouched file',
        location: 'see the gate step',
        path: 'untouched.txt',
        quote: 'never touched',
      },
    ]
  );

  assert.ok(result.persistScript, 'a persist:true run emits a ladder');
  const out = sh(result.persistScript);
  const id = /reviewId=(\S+)/.exec(out);
  assert.ok(id, 'the ladder completes: ' + out);
  assert.match(out, /^anchorsDegraded=partial$/m, 'one anchor landed and one was refused at run time — never `all`, never `none`');

  const review = JSON.parse(rdm(['review', 'show', id[1], '--project', PROJECT, '--format', 'json']));
  // Two findings, plus the ladder's own degradation note — correctness-1: a
  // PARTIALLY-degraded run still gets the note, not just an all-degraded one.
  assert.equal(review.comments.length, 3);
  const landed = review.comments.find((c) => c.body.includes('lands-fine'));
  assert.ok(landed, 'the landed-anchor comment must be present');
  assert.equal(landed.anchor.anchor_type, 'file-quote', 'the landed comment carries a real file anchor');
  assert.equal(parseCommentHeader(landed.body).anchor, 'path');

  const refused = review.comments.find((c) => c.body.includes('refused-at-runtime'));
  assert.ok(refused, 'the run-time-refused comment must still be present, whole-document');
  assert.equal(refused.anchor, undefined, 'the run-time-refused anchor must land whole-document');
  assert.equal(parseCommentHeader(refused.body).anchor, 'degraded');

  const note = review.comments.find((c) => c.body.startsWith('persist-note: anchors-degraded'));
  assert.ok(note, 'a partially-degraded run must still append the note comment');
  assert.equal(note.body, persistDegradationNoteBody(1, 2), 'the note reports 1 of 2, not the build-time-only 0');

  assert.deepEqual(
    result.persistDegraded,
    { requested: 2, degraded: 0, all: false },
    'the build-time-only preview sees neither run-time outcome'
  );

  // Restore the shared worktree's branch so later tests reusing
  // `...ROADMAP_PIN` (whose `expectedHead` was captured once, at seed time)
  // still see the head they were pinned against.
  git(['reset', '--quiet', '--hard', ROADMAP_PIN.expectedHead], src);
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
  assert.deepEqual(
    result.persistDegraded,
    { requested: 0, degraded: 0, all: false },
    'a run that requested zero anchors must never report `all: true`'
  );
  assert.match(out, /^anchorsDegraded=none$/m, 'the ladder prints `none` when nothing was ever requested');
});

test('AC4: every requested anchor degrading trips `persistDegraded.all` and the ladder prints anchorsDegraded=all', async () => {
  // Two findings, BOTH carrying a `quote` and BOTH with no derivable path — no
  // `path` field, and `location` is free-form prose that `pathFromLocation`
  // cannot parse. Every requested anchor therefore degrades to whole-document
  // at build time, which is the stronger, ALL-degraded case distinct from the
  // partially-degraded run covered above.
  const { result } = await drive(
    { ...COMMON, gate: false, persist: true, implements: 'plan/' + PLAN, roadmap: ROADMAP, phase: 'phase-2-dirty', ...ROADMAP_PIN },
    [
      {
        id: 'first-degraded',
        concern: 'correctness',
        severity: 'blocking',
        confidence: 90,
        what_fails: 'no derivable path at all',
        location: 'throughout the gate step',
        quote: 'shipped',
      },
      {
        id: 'second-degraded',
        concern: 'tests',
        severity: 'concern',
        confidence: 85,
        what_fails: 'still no derivable path',
        location: 'elsewhere, also prose-only',
        quote: 'shipped',
      },
    ]
  );

  assert.ok(result.persistScript, 'a persist:true run emits a ladder');
  const out = sh(result.persistScript);
  const id = /reviewId=(\S+)/.exec(out);
  assert.ok(id, 'the ladder prints the id it created: ' + out);

  assert.deepEqual(
    result.persistDegraded,
    { requested: 2, degraded: 2, all: true },
    'AC4: every requested anchor degrading must set `all: true`'
  );
  assert.match(out, /^anchorsDegraded=all$/m, 'the ladder itself prints the all-degraded disposition');

  const review = JSON.parse(rdm(['review', 'show', id[1], '--project', PROJECT, '--format', 'json']));
  // Both findings, plus the ladder's own degradation note — correctness-1:
  // an ALL-build-time-degraded run gets the note too, not just a run-time one.
  assert.equal(review.comments.length, 3, 'both findings still land, whole-document, plus one degradation note');
  const findingComments = review.comments.filter((c) => !c.body.startsWith('persist-note: anchors-degraded'));
  assert.equal(findingComments.length, 2, 'both findings still land, whole-document');
  for (const c of findingComments) {
    assert.equal(parseCommentHeader(c.body).anchor, 'degraded', c.body + ': every comment here must be header-marked degraded');
  }
  const note = review.comments.find((c) => c.body.startsWith('persist-note: anchors-degraded'));
  assert.equal(note.body, persistDegradationNoteBody(2, 2), 'the note reports the all-degraded total, 2 of 2');
});

test('tests-1: the gateScript refuses to write reviewed when RDM_PERSIST_ANCHORS_DEGRADED=all, and writes normally for partial/none', async () => {
  // Nothing in the suite before this test ever EXECUTES the gate script's
  // `RDM_PERSIST_ANCHORS_DEGRADED=all` branch — every other gate test either
  // never sets the variable (so only the `:-none` default runs) or never
  // builds `persistCommands` at all, so the guard is never even emitted. A
  // clean review (no findings) against the task target, with BOTH `persist:
  // true` and `gate: true`, is enough: `persistCommands` existing is what
  // makes the driver emit the guard in the first place (arch-1's
  // `persistDegradationGateLines()`), and this test never actually needs to
  // run `persistScript` — it drives `gateScript` directly, standing in for
  // the persist ladder's own run-time result by setting the environment
  // variable the two ladders are threaded through.
  const { result } = await drive({ ...COMMON, persist: true, task: TASK, ...TASK_PIN });

  assert.equal(result.outcome, 'reviewed');
  assert.ok(result.persistCommands, 'persist:true must build a persist ladder — that is what makes the gate guard exist at all');
  assert.ok(result.gateScript, 'a reviewed outcome with gate:true emits a ladder');
  assert.match(
    result.gateScript,
    /RDM_PERSIST_ANCHORS_DEGRADED/,
    'the emitted gate script must actually carry the run-time degradation guard'
  );

  // RDM_PERSIST_ANCHORS_DEGRADED=all: every requested anchor degraded to
  // whole-document, per the persist ladder's own printed line — the gate
  // must refuse to write `reviewed`, on stderr, leaving the item at whatever
  // `needs-review` write already landed just before the guard.
  const refused = shPlainWithEnv(result.gateScript, { RDM_PERSIST_ANCHORS_DEGRADED: 'all' }, TASK_PIN.source);
  assert.notEqual(refused.status, 0, 'the gate must exit non-zero when every anchor degraded');
  assert.match(refused.stderr, /RDM_PERSIST_ANCHORS_DEGRADED=all/, 'the refusal must name its cause on stderr');
  assert.doesNotMatch(refused.stdout, /status: reviewed/, 'the refusal must land before any reviewed write is reported');
  assert.notEqual(taskJson(TASK).status, 'reviewed', 'the item must not reach reviewed on an all-degraded run');

  // Self-test: the SAME all-degraded run must succeed once the guard itself
  // is removed — proving the refusal above is caused by the guard, not by
  // something else in the ladder (e.g. a refused `--source` binding).
  const guard = persistDegradationGateLines();
  assert.ok(result.gateScript.includes(guard), 'the emitted script must contain the exact guard this test strips');
  const withoutGuard = result.gateScript.split(guard + '\n').join('');
  assert.notEqual(withoutGuard, result.gateScript, 'the mutant must actually remove the guard, or this self-test is vacuous');
  const mutant = shPlainWithEnv(withoutGuard, { RDM_PERSIST_ANCHORS_DEGRADED: 'all' }, TASK_PIN.source);
  assert.equal(mutant.status, 0, 'with the guard stripped, the same all-degraded run must reach reviewed');
  assert.equal(taskJson(TASK).status, 'reviewed', 'confirms the guard, not something else, was refusing the write above');

  // `partial` and `none` are ordinary persistence — the write must proceed
  // each time (the item was left at `reviewed` by the mutant run above, but
  // `needs-review` -> `reviewed` is a no-op transition here, not a skip).
  for (const value of ['partial', 'none']) {
    const ok = shPlainWithEnv(result.gateScript, { RDM_PERSIST_ANCHORS_DEGRADED: value }, TASK_PIN.source);
    assert.equal(ok.status, 0, 'RDM_PERSIST_ANCHORS_DEGRADED=' + value + ' must not be refused: ' + ok.stderr);
    assert.equal(taskJson(TASK).status, 'reviewed', 'the write must reach reviewed for anchorsDegraded=' + value);
  }
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

test('parseCommentHeader accepts a legacy SEVEN-key body with no trailing `anchor` line', () => {
  // The shape every comment carried before `anchor` was added as a trailing
  // eighth key: the same seven `key: value` lines, straight into the blank
  // separator and prose with NO `anchor: ...` line at all. This must still be
  // recognized as machine-written — never misread as an unheadered human
  // comment, which would silently defeat priorFindingsFromReviews's
  // repeat-finding detection (lib/plan-review.mjs) on every review persisted
  // before this change.
  const legacyBody = [
    'severity: blocking',
    'confidence: 90',
    'refuted: false',
    'unrefutedReason: none',
    'dimension: correctness',
    'finding-id: f1',
    'inScope: n/a',
    '',
    'correctness',
    'What fails: it drops a write',
  ].join('\n');

  const h = parseCommentHeader(legacyBody);
  assert.ok(h, 'a legacy seven-key body must still round-trip through the parser');
  assert.equal(h.severity, 'blocking');
  assert.equal(h.confidence, 90);
  assert.equal(h.refuted, false);
  assert.equal(h.unrefutedReason, 'none');
  assert.equal(h.dimension, 'correctness');
  assert.equal(h.findingId, 'f1');
  assert.equal(h.inScope, null);
  assert.equal(h.anchor, undefined, 'a legacy body carries no anchor line, so anchor must be unknown, never guessed');
  assert.equal(h.whatFails, 'it drops a write');

  // The CURRENT eight-key shape must still be preferred and parse unchanged —
  // this is a fallback, not a replacement.
  const currentBody = formatCommentBody({ id: 'f1', concern: 'correctness', severity: 'blocking', confidence: 90, what_fails: 'it drops a write' }, 'path');
  assert.equal(parseCommentHeader(currentBody).anchor, 'path');
});

test("an invalid finder `path` (absolute, `..`, or whitespace) falls back to pathFromLocation(location)", () => {
  const target = 'change/abc123';
  const opts = { pathAnchors: true, source: { noCode: false } };
  for (const badPath of ['/abs/path.rs', '../escape.rs', '   ']) {
    const finding = {
      id: 'f1',
      concern: 'correctness',
      quote: 'shipped',
      path: badPath,
      location: 'roadmap-work.txt:1',
    };
    const decision = persistAnchorFor(finding, target, opts);
    assert.equal(
      decision.path,
      'roadmap-work.txt',
      'declared path ' + JSON.stringify(badPath) + ' must be rejected and fall back to pathFromLocation(location)'
    );
    assert.equal(decision.quote, true, 'a usable fallback path still lands a real anchor');
    assert.equal(decision.reason, null, 'a successful fallback is not a degradation');
  }

  // Negative control: an invalid `path` with NO usable `location` fallback
  // either must still degrade, exactly like an absent `path`.
  const noFallback = persistAnchorFor(
    { id: 'f2', concern: 'correctness', quote: 'shipped', path: '/abs/path.rs', location: 'throughout the gate step' },
    target,
    opts
  );
  assert.equal(noFallback.path, null);
  assert.equal(noFallback.reason, 'path-missing');
});

test('a suffixed finder `path` that CONTAINS A SLASH is stripped before validating, not rejected outright', () => {
  // The regression case tests-1 named: `isRepoRelativePath` alone would
  // ACCEPT a slash-bearing suffixed path whole ('src/foo.rs:12-18' contains
  // a `/`), so a broken strip would hand the real binary a literal
  // `--path 'src/foo.rs:12-18'`, which names no file. `location` here is
  // free-form prose with no derivable path of its own, so nothing can rescue
  // a broken strip by falling back to it — the outcome depends entirely on
  // `stripPathLineSuffix` actually running inside `persistAnchorFor`.
  const decision = persistAnchorFor(
    { id: 'f1', concern: 'correctness', quote: 'shipped', path: 'src/foo.rs:12-18', location: 'prose' },
    'change/abc123',
    { pathAnchors: true, source: { noCode: false } }
  );
  assert.equal(decision.path, 'src/foo.rs', 'the `:12-18` suffix must be stripped from the declared path');
  assert.equal(decision.quote, true);
  assert.equal(decision.reason, null, 'a successfully stripped path is not a degradation');
});

test('AC4: a persist ladder runs to completion under an open stdin with no redirect added by the caller', async () => {
  // Regression for the review write commands (`rdm review start`/`comment`/
  // `submit`) hanging under an agent's Bash tool: before the fix, `rdm`
  // itself blocked reading stdin to EOF whenever it was piped and never
  // closed, so the FIRST `review submit` in every persist ladder deadlocked.
  // The CLI fix means `rdm` no longer blocks reading stdin at all, so THIS
  // case — run against the real, already-fixed binary — passes on the CLI
  // fix alone and proves nothing about the ladder's own `< /dev/null`
  // redirects; it would pass identically even with every one of them
  // deleted from `persistReviewCommands`. It stays as the end-to-end,
  // real-binary completion proof. The next test is what actually exercises
  // the redirects: it points the ladder at a stub `rdm` that blocks reading
  // stdin regardless of the CLI fix, so only the redirects can save it.
  const { result } = await drive({ ...COMMON, gate: false, persist: true, implements: 'plan/' + PLAN, task: TASK, ...TASK_PIN }, {
    id: 'stdin-hang-regression',
    concern: 'correctness',
    severity: 'concern',
    confidence: 90,
    what_fails: 'stub finding, just to produce a non-empty ladder',
    location: 'general',
  });

  assert.ok(result.persistScript, 'a persist:true run emits a ladder');

  const { code, stdout, stderr } = await runWithOpenStdinPipe(result.persistScript);
  assert.equal(code, 0, 'the ladder must exit 0 with stdin held open:\n' + stdout + stderr);
  assert.match(stdout, /reviewId=\S+/, 'the ladder ran to completion and printed the id it created');
});

test('AC4: the per-line `< /dev/null` redirects, not the CLI fix, are what save the ladder from a stdin-blocking rdm', async () => {
  // The case above proves completion against the real, already-fixed `rdm`
  // binary, so it cannot tell the redirects apart from the CLI fix — since
  // the fix, `rdm` never blocks on stdin at all, so that case would pass
  // exactly the same with every `< /dev/null` in `persistReviewCommands`
  // deleted. This case makes the redirect claim actually true: it points the
  // SAME ladder's `rdmBin` at a stub wrapper that deliberately blocks
  // reading stdin to EOF before delegating to the real binary — regardless
  // of what the real binary itself does — so the ladder can only complete if
  // its OWN per-line redirects feed that blocking read a closed stdin.
  //
  // Confirmed by hand while writing this test: regex-stripping every
  // ` < /dev/null` out of the emitted script and running it against this
  // same stub, with stdin held open the same way, times out every time — the
  // mutant self-test below reproduces that and asserts it, so this claim
  // cannot regress silently.
  const { result } = await drive({ ...COMMON, gate: false, persist: true, implements: 'plan/' + PLAN, task: TASK, ...TASK_PIN }, {
    id: 'stdin-hang-regression-stub',
    concern: 'correctness',
    severity: 'concern',
    confidence: 90,
    what_fails: 'stub finding, just to produce a non-empty ladder',
    location: 'general',
  });
  assert.ok(result.persistScript, 'a persist:true run emits a ladder');

  // A wrapper standing in for `rdm` on every line of the ladder: read stdin
  // to EOF first (which hangs forever on a never-closed pipe, unless the
  // caller redirected this invocation's stdin), THEN delegate to the real
  // binary with the same arguments.
  const stub = path.join(PLAN_ROOT, 'stdin-blocking-rdm');
  fs.writeFileSync(stub, '#!/bin/sh\ncat >/dev/null\nexec ' + RDM + ' "$@"\n', { mode: 0o755 });
  const blockingScript = result.persistScript.split(RDM).join(stub);
  assert.notEqual(blockingScript, result.persistScript, 'the stub substitution must actually replace every rdm invocation, or this test proves nothing');

  const { code, stdout, stderr } = await runWithOpenStdinPipe(blockingScript);
  assert.equal(code, 0, 'the ladder must exit 0 even when every invoked rdm blocks reading stdin, with the caller\'s stdin held open:\n' + stdout + stderr);
  assert.match(stdout, /reviewId=\S+/, 'the ladder ran to completion and printed the id it created');

  // Mutant self-test: with the redirects stripped, the exact same stub run
  // must hang — proving the assertions above are caused by the redirects,
  // not by something else (an already-closed stdin from the test runner, a
  // stub that doesn't really block, etc.).
  const stripped = blockingScript.split(' < /dev/null').join('');
  assert.notEqual(stripped, blockingScript, 'the mutant must actually remove every redirect, or this self-test is vacuous');
  await assert.rejects(
    runWithOpenStdinPipe(stripped, 1500),
    /did not exit within/,
    'stripping the per-line `< /dev/null` redirects must reproduce the hang under a stub that blocks reading stdin'
  );
});
