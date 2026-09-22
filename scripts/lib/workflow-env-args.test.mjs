// workflow-env-args.test.mjs — the ENVIRONMENT-ARG contract of the backlog and
// document engines, decided by EXECUTING their own builders.
//
// Both engines used to name this repo's dogfood build path (`./target/debug/rdm`)
// and this repo's project (`rdm`) in every command they build or hand to an
// agent, which made them unshippable: a downstream consumer would have been
// handed commands naming a binary absent from its tree. They now take the pair
// as runtime arguments, per docs/workflow-schemas.md § "Environment args:
// `rdmBin` and `project`".
//
// Nothing here greps a source file for a string it hopes is absent. Every
// assertion runs a real builder and inspects what it produced.
//
// Run by rdm-core/tests/workflow_env_args.rs, so `cargo nextest run` is the gate.
// The plan-review engine's half of the same contract lives in Suite E of
// scripts/lib/plan-review-hoist.test.mjs; the estimate engine already carried
// the contract and is covered by scripts/lib/estimate-writeback.test.mjs.

import test from 'node:test';
import assert from 'node:assert/strict';

import * as backlog from '../../.claude/workflows/lib/backlog.mjs';
import * as document from '../../.claude/workflows/lib/document.mjs';

const BIN = '/opt/tools/rdm';

// An rdm invocation recognized STRUCTURALLY: a token that IS `rdm` or ends in
// `/rdm` (so any executable path, including the `./target/debug/rdm` this phase
// removed, is caught and then compared against the expected one), followed by a
// known rdm subcommand. Matched anywhere in a line, since a prompt names
// commands inline as well as on their own indented line.
const RDM_SUBCOMMANDS =
  'review|commit|plan|roadmap|phase|task|search|backlog|promote|next|model|config|link|verify|merge|archive';
const INVOCATION = new RegExp(
  '(?:^|[\\s`(\\[])((?:[\\w./~-]*/)?rdm) (' + RDM_SUBCOMMANDS + ')\\b',
  'g'
);

// `rdm commit` refuses a project flag; every other subcommand these two engines
// emit is project-scoped.
const UNSCOPED = new Set(['commit']);

// Every rdm invocation anywhere in a multi-line prompt or command string, with
// the remainder of its line so the project flag can be checked in place.
function invocations(text) {
  const out = [];
  for (const line of String(text).split('\n')) {
    INVOCATION.lastIndex = 0;
    let m;
    while ((m = INVOCATION.exec(line)) !== null) {
      out.push({ line: line, bin: m[1], sub: m[2], tail: line.slice(m.index) });
    }
  }
  return out;
}

// assertExecutable — EVERY rdm invocation anywhere in the text, whether it is a
// standalone command line or an example quoted inline in prose, must invoke the
// caller's executable. Returns how many it checked, so a caller can floor it.
function assertExecutable(text, bin, label) {
  const found = invocations(text);
  assert.ok(found.length > 0, `${label}: no rdm invocation at all`);
  for (const inv of found) {
    assert.equal(inv.bin, bin, `${label}: wrong executable — ${inv.line}`);
  }
  return found.length;
}

// assertProjectFlag — the project flag is checked on the COMMAND LINES a
// consumer copies and runs (a line that is nothing but a command), not on the
// prose-embedded read-only lookup examples, which deliberately carry none. An
// UNSCOPED subcommand must carry none either way.
function commandLines(text) {
  return invocations(text).filter((inv) => inv.line.trim().indexOf(inv.bin + ' ') === 0);
}

function assertProjectFlag(text, project, label) {
  const lines = commandLines(text);
  assert.ok(lines.length > 0, `${label}: no standalone rdm command line at all`);
  for (const inv of lines) {
    if (UNSCOPED.has(inv.sub) || !project) {
      assert.ok(
        !inv.tail.includes('--project'),
        `${label}: emitted a project flag it must not carry — ${inv.line}`
      );
    } else {
      assert.ok(inv.tail.includes(' --project ' + project), `${label}: lost its project flag — ${inv.line}`);
    }
  }
  return lines.length;
}

// ================================================================ backlog
//
// Everything this engine builds: the one read command the orchestrator runs,
// and the four analyzer prompts — whose read-only lookup examples AND ten
// proposal templates are the pass's actual product, handed to a human to run.

const REPORT = {
  stale_tasks: [{ slug: 'old-thing' }],
  duplicate_clusters: [{ slugs: ['a', 'b'] }],
  tag_clusters: [{ tag: 'auth', slugs: ['a', 'b'] }],
  archivable_roadmaps: [{ slug: 'done-thing' }],
};

test('backlog: a caller-supplied rdmBin/project reaches the report command and every analyzer prompt', () => {
  const cfg = backlog.parseBacklogArgs({ rdmBin: BIN, project: 'demo' });
  assert.equal(cfg.rdmBin, BIN);
  assert.equal(cfg.project, 'demo');

  assertExecutable(backlog.backlogReportCommand(cfg), BIN, 'report command');
  assertProjectFlag(backlog.backlogReportCommand(cfg), 'demo', 'report command');

  let total = 0;
  let scoped = 0;
  for (const cat of backlog.CATEGORY) {
    const prompt = cat.analyzerPrompt(REPORT[cat.arrayField], cfg);
    total += assertExecutable(prompt, BIN, `${cat.key} prompt`);
    scoped += assertProjectFlag(prompt, 'demo', `${cat.key} prompt`);
  }
  // Floors over the real template set, so a prompt builder that stopped
  // emitting commands entirely could not make this pass vacuously: ten
  // proposal templates, plus the two read-only lookup examples the preamble
  // quotes inline (which carry the executable but deliberately no flag).
  assert.ok(total >= 12, `expected every proposal template and lookup example, saw ${total}`);
  assert.ok(scoped >= 10, `expected every proposal template to be project-scoped, saw ${scoped}`);
});

test('backlog: an omitted project emits no flag, and an omitted rdmBin yields a plain `rdm`', () => {
  const cfg = backlog.parseBacklogArgs({});
  assert.equal(cfg.rdmBin, 'rdm');
  assert.equal(cfg.project, null);

  assertExecutable(backlog.backlogReportCommand(cfg), 'rdm', 'report command');
  assertProjectFlag(backlog.backlogReportCommand(cfg), null, 'report command');
  for (const cat of backlog.CATEGORY) {
    const prompt = cat.analyzerPrompt(REPORT[cat.arrayField], cfg);
    assertExecutable(prompt, 'rdm', cat.key);
    assertProjectFlag(prompt, null, cat.key);
  }
});

test('backlog: an invalid value of either axis is refused at parse time', () => {
  assert.throws(() => backlog.parseBacklogArgs({ rdmBin: 42 }), /rdmBin must be a string path/);
  assert.throws(() => backlog.parseBacklogArgs({ rdmBin: {} }), /rdmBin must be a string path/);
  assert.throws(() => backlog.parseBacklogArgs({ project: 'a b' }), /plain project name/);
  assert.throws(() => backlog.parseBacklogArgs({ project: 'a;rm -rf /' }), /plain project name/);
  assert.equal(backlog.resolveRdmBin(undefined), 'rdm');
  assert.equal(backlog.parseProjectArg(undefined), '');
});

// ================================================================ document
//
// Both of this engine's read commands: the roadmap payload the orchestrator is
// told to run, and the per-phase command named in both agents' prompts.

test('document: a caller-supplied rdmBin/project reaches both read commands', () => {
  const cfg = document.parseDocumentArgs({ roadmap: 'r', rdmBin: BIN, project: 'demo' });
  assert.equal(cfg.rdmBin, BIN);
  assert.equal(cfg.project, 'demo');

  for (const [label, cmd] of [
    ['roadmap command', document.documentRoadmapCommand('r', cfg)],
    ['phase command', document.documentPhaseCommand('r', 'phase-1-a', cfg)],
  ]) {
    assertExecutable(cmd, BIN, label);
    assertProjectFlag(cmd, 'demo', label);
  }
});

test('document: an omitted project emits no flag, and an omitted rdmBin yields a plain `rdm`', () => {
  const cfg = document.parseDocumentArgs({ roadmap: 'r' });
  assert.equal(cfg.rdmBin, 'rdm');
  assert.equal(cfg.project, '');

  for (const [label, cmd] of [
    ['roadmap command', document.documentRoadmapCommand('r', cfg)],
    ['phase command', document.documentPhaseCommand('r', 'phase-1-a', cfg)],
  ]) {
    assertExecutable(cmd, 'rdm', label);
    assertProjectFlag(cmd, null, label);
  }
});

test('document: an invalid value of either axis is refused at parse time', () => {
  assert.throws(() => document.parseDocumentArgs({ roadmap: 'r', rdmBin: 42 }), /rdmBin must be a string path/);
  assert.throws(() => document.parseDocumentArgs({ roadmap: 'r', project: 'a b' }), /plain project name/);
  assert.throws(
    () => document.parseDocumentArgs({ roadmap: 'r', project: 'a;rm -rf /' }),
    /plain project name/
  );
  assert.equal(document.resolveRdmBin(undefined), 'rdm');
  assert.equal(document.parseProjectArg(undefined), '');
  assert.equal(document.projectFlag({ project: 'demo' }), ' --project demo');
  assert.equal(document.projectFlag({}), '');
});
