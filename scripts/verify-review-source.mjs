// Real source-bound standalone regression; no live LLM calls.
import assert from 'node:assert/strict';
import * as review from '../.claude/workflows/lib/review.mjs';
const complete = { complete: true, selected: ['ac', 'correctness'], ran: ['ac', 'correctness'], failed: [], acDimensionRan: true };
const ac = [{ criterion: 'AC1: works', status: 'PASS', evidence: 'test' }];
assert.equal(review.classifyOutcome({ planFindings: [], codeReviews: [[]], evidence: { coverage: { ...complete, complete: false, failed: ['ac'], acDimensionRan: false }, acTable: null } }), 'escalated');
for (const evidence of [
  { criteria: ac.map(row => row.criterion), coverage: complete, acTable: null },
  { criteria: ac.map(row => row.criterion), coverage: complete, acTable: [] },
  { criteria: ac.map(row => row.criterion), coverage: complete, acTable: [{ status: 'INVALID' }] },
  { coverage: { ...complete, complete: false, failed: ['correctness'] }, acTable: ac },
  { criteria: ac.map(row => row.criterion), coverage: complete, acTable: ac, budget: { passedThroughBudget: 1 } },
  { criteria: ac.map(row => row.criterion), coverage: complete, acTable: ac, survivors: [{ severity: 'concern', refuterError: true }] },
]) assert.equal(review.classifyOutcome({ codeReviews: [[]], evidence }), 'escalated');
assert.equal(review.classifyOutcome({ codeReviews: [[]], evidence: { criteria: ac.map(row => row.criterion), coverage: complete, acTable: ac, survivors: [{ severity: 'suggestion', unrefuted: true, unrefutedReason: 'non-gating' }] } }), 'reviewed');
assert.equal(review.classifyOutcome({ codeReviews: [[]], evidence: { criteria: ac.map(row => row.criterion), coverage: { complete: false, last: complete }, acTable: ac } }), 'reviewed');
// Authoritative identities retain wrapped and nested continuation text.
assert.deepEqual(review.acceptanceCriteria('## Acceptance Criteria\n- [ ] first\n  continued\n  - nested detail\n- second\n## Verification\nignored'), ['AC1: first continued - nested detail', 'AC2: second']);
assert.deepEqual(review.acceptanceCriteria('## Acceptance Criteria\n1. first\n2. second'), ['AC1: first', 'AC2: second']);
assert.deepEqual(review.acceptanceCriteria('## Acceptance\nFirst paragraph\ncontinues.\n\nSecond paragraph.'), ['AC1: First paragraph continues.', 'AC2: Second paragraph.']);
for (const body of ['', 'no criteria heading', '## Acceptance Criteria\n', '## Acceptance Criteria\n| table |', '## Acceptance Criteria\n- same\n- same', '## Acceptance Criteria\n### uncertain']) assert.deepEqual(review.acceptanceCriteria(body), []);
console.log('automatic completeness regressions passed');

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
const root = path.resolve(import.meta.dirname, '..');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'rdm-review-source-'));
const binary = path.join(root, 'target/debug/rdm');
const sourceRepo = path.join(temp, 'source repo');
const planRepo = path.join(temp, 'plans');
const env = { ...process.env, RDM_ROOT: planRepo, RDM_PROJECT: 'verify', RDM_SESSION: 'verify-review-source',
  GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1', GIT_AUTHOR_NAME: 'Test', GIT_AUTHOR_EMAIL: 'test@example.invalid', GIT_COMMITTER_NAME: 'Test', GIT_COMMITTER_EMAIL: 'test@example.invalid' };
for (const key of Object.keys(env)) if (/^RDM_(STAGE|FORMAT|PLAN_REPO|SOURCE_REPO)/.test(key) || /^GIT_(DIR|WORK_TREE|INDEX_FILE|CONFIG_COUNT|CONFIG_KEY_|CONFIG_VALUE_)/.test(key)) delete env[key];
const run = (bin, args, cwd = sourceRepo) => execFileSync(bin, args, { cwd, env, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim();
const rdm = (args, cwd = sourceRepo) => run(binary, args, cwd);
const git = (args, cwd = sourceRepo) => run('git', args, cwd);
const shell = (command, cwd = sourceRepo) => run('/bin/sh', ['-eu', '-c', command], cwd);
const json = (args, cwd) => JSON.parse(rdm([...args, '--format', 'json'], cwd));
const srcPath = path.join(root, '.claude/workflows/rdm-wf-review-refute-fix.js');
const raw = fs.readFileSync(srcPath, 'utf8');
const code = raw.replace(/export const meta\s*=\s*\{[\s\S]*?\n\}/, '');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const driver = new AsyncFunction('args', 'agent', 'pipeline', 'parallel', 'log', code);
const parallel = async (thunks) => Promise.all(thunks.map(async (fn) => { try { return await fn(); } catch { return null; } }));
const pipeline = async (items, ...stages) => { let out = items; for (const stage of stages) out = await stage(out); return out; };
// Retain the survivors-only invocation shapes and dispatch hygiene contracts.
const legacyAgent = async (_prompt, options) => options.label === 'find:code:ac' ? { ac, findings: [] } : { findings: [] };
for (const mode of ['code', 'plan']) {
  const legacy = await driver({ mode, context: { target: 'legacy' } }, legacyAgent, pipeline, parallel, () => {});
  assert.equal(legacy.mode, mode); assert.ok(legacy.survivors.every((f) => f.severity === 'suggestion')); 
  assert.equal(legacy.outcome, undefined, 'legacy report does not approve');
}
await assert.rejects(() => driver({ mode: 'code', task: 'repair', roadmap: 'alpha', phase: 'phase-1-work' }, legacyAgent, pipeline, parallel, () => {}));
const region = raw.slice(raw.indexOf('// >>> review-refute-fix:end'));
assert.equal((region.match(/classifyOutcome\(/g) || []).length, 1);
assert.equal((region.match(/= *buildReviewPipeline\('code'\)/g) || []).length, 1);
assert.doesNotMatch(raw, /Done:|Date\.now\(|Math\.random\(/);
assert.equal(raw, fs.readFileSync(path.join(root, 'rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js'), 'utf8'));
try {
  fs.mkdirSync(sourceRepo);
  git(['init', '-b', 'main']);
  git(['commit', '--allow-empty', '-m', 'initial']);
  const base = git(['rev-parse', 'HEAD']);
  rdm(['init', '--default-project', 'verify']);
  rdm(['roadmap', 'create', 'alpha', '--title', 'Alpha', '--body', 'Goal: correct source', '--no-edit']);
  rdm(['phase', 'create', 'work', '--roadmap', 'alpha', '--number', '1', '--title', 'Work', '--body', '## Acceptance Criteria\n- works', '--no-edit']);
  rdm(['task', 'create', 'repair', '--title', 'Repair', '--body', '## Acceptance Criteria\n- works', '--no-edit', '--no-plan-review']);
  rdm(['plan', 'create', 'implementation', '--implements', 'phase/alpha/phase-1-work', '--title', 'Implementation', '--body', 'Implement works.', '--no-edit']);
  const planReview = json(['review', 'start', '--on', 'plan/implementation', '--author', 'independent', '--body', 'Independently reviewed.', '--no-edit']);
  rdm(['review', 'submit', planReview.id, '--verdict', 'approve', '--no-edit']);
  rdm(['plan', 'create', 'task-implementation', '--implements', 'task/repair', '--title', 'Task implementation', '--body', 'Implement repair.', '--no-edit']);
  const taskPlanReview = json(['review', 'start', '--on', 'plan/task-implementation', '--author', 'independent', '--body', 'Independently reviewed.', '--no-edit']);
  rdm(['review', 'submit', taskPlanReview.id, '--verdict', 'approve', '--no-edit']);
  rdm(['task', 'create', 'unrelated', '--title', 'Unrelated', '--body', '## Acceptance Criteria\n- unrelated behavior', '--no-edit', '--no-plan-review']);
  rdm(['plan', 'create', 'unrelated-implementation', '--implements', 'task/unrelated', '--title', 'Other plan', '--body', 'Other behavior.', '--no-edit']);
  const otherReview = json(['review', 'start', '--on', 'plan/unrelated-implementation', '--author', 'independent', '--body', 'Reviewed.', '--no-edit']);
  rdm(['review', 'submit', otherReview.id, '--verdict', 'approve', '--no-edit']);
  rdm(['commit', '-m', 'chore(plan): fixture']);
  const shared = fs.realpathSync(rdm(['worktree', 'add', 'alpha']));
  const stale = fs.realpathSync(rdm(['worktree', 'add', 'alpha/phase-1-work']));
  fs.writeFileSync(path.join(shared, 'file with spaces.txt'), 'implemented\n');
  fs.mkdirSync(path.join(shared, 'src'));
  fs.writeFileSync(path.join(shared, 'src/lib.rs'), 'fn implemented() {}\n');
  git(['add', '.'], shared); git(['commit', '-m', 'feat: implementation'], shared);
  const head = git(['rev-parse', 'HEAD'], shared);
  rdm(['config', 'set', 'gates.reviewed', 'true']);
  rdm(['commit', '-m', 'chore(plan): enable gate']);
  const defaultArgs = { mode: 'code', roadmap: 'alpha', phase: 'phase-1-work', rdmBin: binary, project: 'verify', base, implements: 'plan/implementation' };
  const finding = { id: 'issue', concern: 'correctness', severity: 'concern', confidence: 90, location: 'file with spaces.txt:1', what_fails: 'example', why: 'example', recommendation: 'fix' };
  async function execute(args = {}, options = {}) {
    const calls = [];
    let resolved;
    const agent = async (prompt, opts) => {
      calls.push({ prompt, ...opts });
      if (opts.label.startsWith('plan:')) {
        const command = prompt.split('\n').find(line => line.includes(' plan '));
        const result = JSON.parse(shell(command));
        return command.includes(' plan list ') ? { plans: result } : result;
      }
      if (opts.label.startsWith('source:') && opts.label !== 'source:acceptance') {
        if (options.moveBeforeRevalidate && opts.label === 'source:revalidate') {
          git(['commit', '--allow-empty', '-m', 'external movement'], shared); options.moveBeforeRevalidate = false;
        }
        const command = prompt.split('\n').find((line) => line.includes(' review source --on '));
        resolved = JSON.parse(shell(command)); // ACTUAL resolver; no fabricated correct identity.
        if (options.wrongReturnedItem) resolved.item = 'task/other';
        return resolved;
      }
      if (opts.label === 'source:acceptance') {
        const command = prompt.split('\n').find((line) => line.includes(' show '));
        return { acceptance: JSON.parse(shell(command)).body };
      }
      if (opts.label.startsWith('find:')) {
        assert.ok(prompt.includes(shared), 'finder reads the resolved shared checkout');
        assert.ok(prompt.includes(head), 'finder reads pinned head');
        assert.ok(prompt.includes('Acceptance criteria:'), 'criteria threaded');
        if (options.failDimension && opts.label.startsWith('find:code:' + options.failDimension)) return null;
        if (opts.label === 'find:code:ac') return options.invalidAc ? { ac: [{ status: 'INVALID' }] } : { ac: options.acRows || [{ ...ac[0], status: options.acFail ? 'FAIL' : 'PASS' }], findings: [] };
        return { findings: opts.label === 'find:code:correctness' ? (options.findings || []) : [] };
      }
      if (opts.label.startsWith('refute:')) {
        assert.ok(prompt.includes(shared) && prompt.includes(head), 'refuter pinned too');
        if (options.refuterCrash) throw new Error('injected refuter crash');
        return { refuted: false, confidence: 90, reason: 'verified' };
      }
      if (opts.label === 'persist:review') {
        if (options.persistFailure) return { ok: false };
        assert.ok(!prompt.includes('worktree add'), 'persistence never reselects or creates checkout');
        assert.ok(!prompt.includes('--on phase/'), 'no item approval fallback');
        assert.ok(!prompt.includes('FALLBACK COMMAND LADDER'), 'a source-bound persist emits no document fallback ladder');
        const start = prompt.indexOf('Run these commands IN ORDER');
        const emitted = prompt.slice(prompt.indexOf('\n', start) + 1, prompt.indexOf('\nANCHORING FALLBACK'));
        let command = emitted;
        // Counted from the EMITTED list, before any planted mutation: this is
        // how many findings ASKED for an anchor, which is what the accounting
        // reconciles against.
        const asked = (emitted.match(/--quote "\$RDM_PERSIST_QUOTE"/g) || []).length;
        const comments = (emitted.match(/ review comment /g) || []).length;
        if (options.missingPath) {
          command = command.replaceAll(' --path "$RDM_PERSIST_PATH"', '');
          assert.notEqual(command, emitted, 'missing-path mutation must be planted');
        }
        if (options.degradeAnchors) {
          // The documented whole-document rung, applied by the agent: drop the
          // path AND the quote, so the finding is persisted once, unanchored.
          command = command.replaceAll(' --path "$RDM_PERSIST_PATH"', '').replaceAll(' --quote "$RDM_PERSIST_QUOTE"', '');
          assert.notEqual(command, emitted, 'degrade-anchors mutation must be planted');
        }
        const output = shell(command + '\nprintf "\\n%s\\n" "$RDM_REVIEW_ID"');
        const reviewId = output.split('\n').at(-1);
        const record = JSON.parse(rdm(['review', 'show', reviewId, '--format', 'json'], shared));
        const anchored = record.comments.filter((comment) => comment.anchor).length;
        const degraded = options.lyingAck ? 0 : Math.max(0, asked - anchored);
        return {
          ok: true,
          reviewId,
          targetUsed: 'change/' + head,
          attempted: comments,
          // A legitimate retry inflates ONLY commandsRun; nothing reconciles
          // against it, so this must not perturb the verdict.
          commandsRun: comments + (options.retryOnce ? 1 : 0),
          anchored: options.lyingAck ? asked : anchored,
          wholeDocumentIntended: comments - asked,
          degraded,
          degradedReasons: Array.from({ length: degraded }, (_, i) => ({ findingId: 'degraded-' + i, reason: 'outside-hunk' })),
        };
      }
      if (opts.label === 'gate:persist') {
        if (options.gateFailure) return { ok: false, head, branch: resolved.branch, status: 'needs-review' };
        const lines = prompt.split('\n').filter((line) => line.startsWith(binary + ' '));
        let stamped = false;
        let status;
        for (const line of lines) {
          const output = shell(line, shared);
          if (line.includes(' review pending ')) {
            const pending = JSON.parse(output);
            assert.ok(pending.some((p) => p.review_sha === head && p.branch === resolved.branch)); stamped = true;
          }
          if (line.includes(' show ')) status = JSON.parse(output).status;
        }
        assert.ok(stamped);
        return { ok: true, head: resolved.head, branch: resolved.branch, status };
      }
      throw new Error('unexpected agent ' + opts.label);
    };
    const result = await driver({ ...defaultArgs, ...args }, agent, pipeline, parallel, () => {});
    return { result, calls };
  }
  // Wrong/empty caller diff cannot bypass the real resolver.
  const clean = await execute({ diff: { changedFiles: [], diffText: 'fake' }, persist: true, gate: true });
  assert.equal(clean.result.outcome, 'reviewed', JSON.stringify(clean.result));
  assert.equal(clean.result.source.path, shared);
  assert.equal(clean.result.source.head, head);
  assert.equal(clean.result.source.base, base);
  assert.ok(clean.calls.some((call) => call.label === 'source:resolve'));
  const persisted = json(['review', 'show', clean.result.reviewId]);
  assert.equal(persisted.target.head, head);
  assert.equal(persisted.target.base, base);
  assert.equal(persisted.implements, 'rdm:plan/implementation');
  assert.equal(json(['phase', 'show', 'phase-1-work', '--roadmap', 'alpha']).status, 'reviewed');
  const quoted = { ...finding, id: 'quoted', location: 'src/lib.rs:1', quote: 'fn implemented() {}' };
  const unlocated = { ...finding, id: 'unlocated', location: 'throughout the gate step' };
  async function checkWriter(missingPath = false, extra = {}) {
    const written = await execute({ persist: true, tier: 'large' }, { findings: [quoted, unlocated], missingPath, ...extra });
    assert.equal(written.result.outcome, 'rework', 'generated code writer must persist rework: ' + written.result.summary);
    const record = json(['review', 'show', written.result.reviewId], shared);
    const located = record.comments.find(comment => comment.anchor);
    assert.ok(located, 'quoted finding must have a resolved code path anchor');
    assert.equal(located.anchor.anchor_type, 'file-quote');
    assert.equal(located.anchor.path, 'src/lib.rs');
    assert.equal(located.resolution.state, 'resolved');
    assert.equal(located.resolution.quote, quoted.quote);
    assert.equal(located.source_link, 'rdm:src/src/lib.rs@' + head + '#L1');
    const link = json(['link', 'resolve', located.source_link], shared);
    assert.equal(link.kind, 'code'); assert.equal(link.path, 'src/lib.rs');
    assert.equal(link.rev, head); assert.equal(link.line, 1);
    assert.equal(record.comments.filter(comment => !comment.anchor).length, 1, 'unlocated finding stays whole-document');
  }
  await checkWriter();
  await assert.rejects(() => checkWriter(true), /generated code writer must persist rework|quoted finding must have a resolved code path anchor/);
  console.log('generated writer control passed; planted missing-path mutation failed as required');

  // --- Anchor-degradation accounting, end to end through the real driver ----
  // A mixed review: one finding that ASKED for an anchor, one that never did.
  // Both non-gating, so the review is otherwise clean and the only thing that
  // can move the outcome is how the anchors landed.
  const quotedSuggestion = { ...quoted, id: 'quoted-suggestion', severity: 'suggestion' };
  const unlocatedSuggestion = { ...unlocated, id: 'unlocated-suggestion', severity: 'suggestion' };
  const mixed = [quotedSuggestion, unlocatedSuggestion];
  const mixedClean = await execute({ persist: true }, { findings: mixed });
  assert.equal(mixedClean.result.outcome, 'reviewed', 'a clean mixed review still reports reviewed: ' + mixedClean.result.summary);
  const cleanAccounting = mixedClean.result.reviewPersistence;
  assert.ok(cleanAccounting, 'the OUTCOME carries reviewPersistence whenever the persist ran');
  assert.equal(cleanAccounting.expectedTotal, 2);
  assert.equal(cleanAccounting.expectedAnchorable, 1, 'only the quoted finding asked for an anchor');
  assert.equal(cleanAccounting.anchored, 1);
  assert.equal(cleanAccounting.wholeDocumentIntended, 1, 'a finding that never carried a quote is INTENDED, not degraded');
  assert.equal(cleanAccounting.degraded, 0);
  assert.equal(cleanAccounting.reconciled, true);
  assert.equal(cleanAccounting.unresolvedDegradation, false);
  assert.equal(cleanAccounting.targetFellBack, false);
  assert.ok(!mixedClean.result.summary.includes('degraded'), 'a clean run names no degradation');

  // Same review, every attempted anchor lost: it must NOT be indistinguishable
  // from the clean run above.
  const mixedDegraded = await execute({ persist: true }, { findings: mixed, degradeAnchors: true });
  assert.equal(mixedDegraded.result.outcome, 'escalated', 'a 100-percent degraded review cannot report reviewed');
  assert.equal(mixedDegraded.result.status, 'blocked');
  assert.equal(mixedDegraded.result.writesCompletion, false);
  assert.equal(mixedDegraded.result.reviewPersistence.degraded, 1);
  assert.equal(mixedDegraded.result.reviewPersistence.anchored, 0);
  assert.equal(mixedDegraded.result.reviewPersistence.wholeDocumentIntended, 1, 'the intentional whole-document finding is still counted apart from the failed anchor');
  assert.equal(mixedDegraded.result.reviewPersistence.unresolvedDegradation, true);
  assert.match(mixedDegraded.result.summary, /anchors: 0 landed, 1 intentionally whole-document, 1 degraded/);
  // The persisted artifact still carries BOTH findings exactly once.
  assert.equal(JSON.parse(rdm(['review', 'show', mixedDegraded.result.reviewId, '--format', 'json'], shared)).comments.length, 2);

  // A legitimate retry is NOT degradation: commandsRun exceeds the per-finding
  // attempted count and the verdict is unmoved.
  const retried = await execute({ persist: true }, { findings: mixed, retryOnce: true });
  assert.equal(retried.result.outcome, 'reviewed', 'an anchor that landed on a retry keeps the review clean');
  assert.equal(retried.result.reviewPersistence.attempted, 2, 'attempted is denominated in FINDINGS');
  assert.equal(retried.result.reviewPersistence.commandsRun, 3, 'commandsRun is denominated in INVOCATIONS');
  assert.equal(retried.result.reviewPersistence.unresolvedDegradation, false);
  assert.match(retried.result.summary, /\[anchors: 1 retried\]/);

  // An EMPTY committed range carrying a quoted finding: no hunk can ever hold
  // the anchor, so the run must not read clean.
  // It must escalate for the RIGHT reason: the review is still RECORDED, with
  // the unanchorable quote dropped at build time and counted. `--quote` without
  // `--path` is refused outright on a change target, so before the writer
  // downgraded it the whole persist aborted and this leg passed on a total
  // failure rather than on graceful degradation.
  const emptyRange = await execute({ base: head, noCode: true, persist: true }, { findings: [quotedSuggestion] });
  assert.equal(emptyRange.result.outcome, 'escalated', 'a quoted finding against an empty committed range cannot report reviewed');
  assert.ok(emptyRange.result.reviewId, 'the review is still recorded — the persist degrades, it does not abort');
  assert.equal(emptyRange.result.reviewPersistence.preDegraded, 1, 'the unanchorable quote is counted at build time');
  assert.equal(emptyRange.result.reviewPersistence.unresolvedDegradation, true);
  assert.match(emptyRange.result.summary, /unanchorable before any command ran/);
  const emptyRecord = JSON.parse(rdm(['review', 'show', emptyRange.result.reviewId, '--format', 'json'], shared));
  assert.equal(emptyRecord.comments.length, 1, 'the finding is persisted exactly once, whole-document');
  assert.equal(emptyRecord.comments.filter((c) => c.anchor).length, 0, 'and nothing anchored, because nothing could');

  // The INVERSE of the missing-path mutation: an ack that CLAIMS a clean
  // anchoring while the script it ran dropped every --path. The accounting
  // believes the self-report, so the read-back is what must catch it — proving
  // the anchor assertions are not satisfied by the ack alone.
  await assert.rejects(
    () => checkWriter(true, { lyingAck: true }),
    /generated code writer must persist rework|quoted finding must have a resolved code path anchor/,
    'a lying ack must not buy a clean anchored review'
  );
  console.log('anchor-degradation accounting regressions passed');

  assert.equal(git(['rev-parse', 'HEAD'], stale), base, 'stale branch unchanged');
  assert.equal(git(['status', '--porcelain'], stale), '', 'stale checkout unchanged');
  assert.equal(json(['worktree', 'list']).length, 2, 'no task checkout created');
  const unrelated = await execute({ roadmap: undefined, phase: undefined, task: 'repair', source: shared, persist: true, gate: false, diff: { diffText: 'fake', changedFiles: [] }, implements: 'plan/unrelated-implementation' });
  assert.equal(unrelated.result.outcome, 'escalated', 'unrelated plan must not approve');
  assert.ok(!unrelated.calls.some(c => c.label === 'persist:review' || c.label.startsWith('find:')), 'unrelated plan refused before review or persistence');
  assert.equal((json(['plan', 'show', 'unrelated-implementation']).change_reviews || []).length, 0);
  assert.equal((await execute({ implements: undefined })).result.outcome, 'reviewed', 'single approved intended plan resolves automatically');
  assert.equal((await execute({ implements: 'plan/missing', persist: true })).result.outcome, 'escalated');
  const task = await execute({ roadmap: undefined, phase: undefined, task: 'repair', source: shared, expectedHead: head, persist: true, gate: true, implements: 'plan/task-implementation' });
  assert.equal(task.result.outcome, 'reviewed', JSON.stringify(task.result));
  assert.equal(json(['worktree', 'list']).length, 2);
  for (const options of [{ failDimension: 'ac' }, { failDimension: 'correctness' }, { invalidAc: true }, { findings: [finding], refuterCrash: true }]) {
    const { result, calls } = await execute({ persist: true }, options);
    assert.equal(result.outcome, 'escalated'); assert.equal(result.writesCompletion, false);
    assert.ok(!calls.filter((c) => c.label === 'persist:review').some((c) => c.prompt.includes('--verdict approve')));
  }
  rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--body', '## Acceptance Criteria\n- works\n- second behavior', '--no-edit']);
  const missing = await execute({ persist: true });
  assert.equal(missing.result.outcome, 'escalated', 'omitted intended criterion must not approve');
  assert.ok(!missing.calls.some(c => c.label === 'persist:review' && c.prompt.includes('--verdict approve')));
  for (const acRows of [
    [ac[0], ac[0]],
    [ac[0], { ...ac[0], criterion: 'AC2: unknown' }],
  ]) {
    const invalid = await execute({ persist: true }, { acRows });
    assert.equal(invalid.result.outcome, 'escalated');
    assert.ok(!invalid.calls.some(c => c.label === 'persist:review' && c.prompt.includes('--verdict approve')));
  }
  const allRows = [ac[0], { ...ac[0], criterion: 'AC2: second behavior' }];
  assert.equal((await execute({}, { acRows: allRows })).result.outcome, 'reviewed');
  rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--body', '## Acceptance Criteria\nworks\n\nsecond behavior', '--no-edit']);
  assert.equal((await execute({}, { acRows: allRows })).result.outcome, 'reviewed', 'prose criteria remain supported');
  rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--body', 'No criteria supplied.', '--no-edit']);
  assert.equal((await execute({ persist: true })).result.outcome, 'escalated');
  rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--body', '## Acceptance Criteria\n- works', '--no-edit']);
  const overflow = await execute({ maxRefutations: 0, tier: 'small' }, { findings: [finding] });
  assert.equal(overflow.result.outcome, 'escalated', 'ungraded concern cannot approve even at small tier');
  assert.equal(overflow.result.reviewBudget.passedThroughBudget, 1);
  const suggestion = await execute({ findModel: 'finder-model', verifyModel: 'refuter-model' }, { findings: [{ ...finding, severity: 'suggestion' }] });
  assert.equal(suggestion.result.outcome, 'reviewed', 'intentional non-gating suggestion remains non-gating');
  assert.ok(suggestion.calls.filter((c) => c.label.startsWith('find:')).every((c) => c.model === 'finder-model'));
  const modeled = await execute({ findModel: 'finder-model', verifyModel: 'refuter-model' }, { findings: [finding] });
  assert.ok(modeled.calls.filter((c) => c.label.startsWith('refute:')).every((c) => c.model === 'refuter-model'));
  assert.ok(!modeled.calls.some((c) => c.label === 'persist:review' || c.label === 'gate:persist'), 'mutations default off');
  for (const project of ['bad name', 'bad;command', 42]) await assert.rejects(() => execute({ project }));
  await assert.rejects(() => execute({ rdmBin: 42 }));
  assert.equal((await execute({ persist: { on: 'plan/implementation' } })).result.outcome, 'escalated', 'explicit wrong persistence target cannot approve');
  const rework = await execute({ gate: true }, { acFail: true });
  assert.equal(rework.result.outcome, 'rework', JSON.stringify(rework.result));
  assert.equal(json(['phase', 'show', 'phase-1-work', '--roadmap', 'alpha']).status, 'in-progress');

  assert.equal((await execute({}, { acFail: true })).result.outcome, 'rework');
  assert.equal((await execute({ gate: true }, { gateFailure: true })).result.outcome, 'escalated');
  assert.equal((await execute({ persist: true }, { persistFailure: true })).result.outcome, 'escalated');
  assert.equal((await execute({}, { wrongReturnedItem: true })).result.outcome, 'escalated');
  assert.equal((await execute({ base: head })).result.outcome, 'escalated');
  assert.equal((await execute({ base: head, noCode: true })).result.outcome, 'reviewed');
  assert.equal((await execute({ expectedHead: base })).result.outcome, 'escalated');
  assert.equal((await execute({ source: stale })).result.outcome, 'escalated');
  assert.equal((await execute({ source: temp })).result.outcome, 'escalated');
  assert.equal((await execute({}, { moveBeforeRevalidate: true })).result.outcome, 'escalated');
  const moved = git(['rev-parse', 'HEAD'], shared);
  assert.throws(() => rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--status', 'reviewed', '--source', shared, '--base', base, '--expected-head', head, '--no-edit'], shared), /HEAD moved/);
  // Old approval cannot release the newly observed head (even without expected-head).
  assert.throws(() => rdm(['phase', 'update', 'phase-1-work', '--roadmap', 'alpha', '--status', 'reviewed', '--source', shared, '--base', base, '--expected-head', moved, '--no-edit'], shared));
  console.log('real standalone source, persistence, status and incomplete evidence regressions passed');
} finally { fs.rmSync(temp, { recursive: true, force: true }); }
