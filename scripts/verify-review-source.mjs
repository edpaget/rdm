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
// DELETED (no-mechanical-agents-in-workflows phase 34, commit 2): the
// acceptanceCriteria() parser tests. The function is gone — the `ac` reviewer
// reads the item document itself and takes the criteria from it.
console.log('automatic completeness regressions passed');

import fs from 'node:fs';
import path from 'node:path';
const root = path.resolve(import.meta.dirname, '..');
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
// DELETED (no-mechanical-agents-in-workflows phase 34, commit 2): the whole
// real-binary section below this point. Every one of its assertions drove the
// engine's mechanical agents — `source:resolve` / `source:revalidate`,
// `plan:resolve` / `plan:revalidate`, `source:acceptance`, `persist:review` and
// `gate:persist` — by intercepting their prompts and running the commands they
// named. None of those agents exists: the engine now dispatches finders and
// refuters only, takes the pinned identity as caller arguments, and RETURNS the
// persist and gate ladders as command text instead of running them. Deleted and
// named, never re-pointed at the returned strings — the persist ladder's
// behavior against the real binary is covered by verify-workflow-review.sh
// §15b/§15d, which executes exactly the bytes `persistReviewCommands` emits.

console.log('legacy-shape, structural and completeness regressions passed');
