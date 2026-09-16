import { createHash } from 'node:crypto';
import { safeGit } from './codex-runtime-state.mjs';
import { buildEstimatePipeline, buildEstimatorPrompt } from '../../.claude/workflows/lib/estimate.mjs';

const ratingSchema = {
  type: 'object', additionalProperties: false,
  properties: { stem: { type: 'string' }, difficulty: { type: 'string', enum: ['trivial', 'easy', 'moderate', 'hard'] }, justification: { type: 'string' } },
  required: ['stem', 'difficulty', 'justification'],
};
const stable = value => Array.isArray(value) ? value.map(stable) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, stable(value[key])])) : value;
const hash = value => createHash('sha256').update(JSON.stringify(stable(value))).digest('hex');

/** Rate a real roadmap through canonical estimation. Preview is read-only.
 * Apply writes only unset difficulty and audit notes, then commits this run's
 * session. Throws on drift, invalid judgments or uncertain side effects; no
 * mutation is retried and no review status is advanced.
 */
export async function runEstimate({ ctx, roadmap, apply = false, agent }) {
  if (typeof roadmap !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(roadmap) || typeof apply !== 'boolean' || typeof agent !== 'function') {
    throw new Error('estimate requires a roadmap slug, boolean apply, and an agent');
  }
  const scope = ['--roadmap', roadmap, '--project', ctx.identity.project];
  const show = stem => ctx.rdm(['phase', 'show', stem, ...scope, '--format', 'json'], { json: true });
  const snapshots = new Map();
  const intended = new Map();
  const proposed = [];
  let failure;
  let attemptedWrite = false;
  // Canonical orchestration catches dependency failures. Capture and rethrow
  // outside it, and prevent later writes after the first failure.
  const guarded = fn => async (...args) => {
    if (failure) throw failure;
    try { return await fn(...args); } catch (error) { failure = error; throw error; }
  };
  const run = buildEstimatePipeline({
    list: guarded(async () => {
      const list = await ctx.rdm(['phase', 'list', ...scope, '--format', 'json'], { json: true });
      if (!Array.isArray(list) || list.some(p => !p || typeof p.stem !== 'string' || !/^phase-[A-Za-z0-9._-]+$/.test(p.stem)) || new Set(list.map(p => p.stem)).size !== list.length) throw new Error('invalid phase list');
      return list;
    }),
    parallelRate: guarded(async stems => {
      // Read every target before judgments; validate every response before the
      // canonical loop can perform its first write.
      for (const stem of stems) {
        const phase = await show(stem);
        if (!phase || typeof phase.body !== 'string' || phase.difficulty || phase.model) throw new Error(`estimate target changed or already set: ${stem}`);
        snapshots.set(stem, { phase, hash: hash({ roadmap, project: ctx.identity.project, stem, phase }) });
      }
      const ratings = await Promise.allSettled(stems.map(stem => agent(
        buildEstimatorPrompt(`Phase stem: ${stem}\n${snapshots.get(stem).phase.body}`),
        { label: `estimate:rate:${stem}`, agentType: 'estimator', step: 'estimate', schema: ratingSchema },
      )));
      for (let i = 0; i < stems.length; i++) {
        if (ratings[i].status === 'rejected') throw ratings[i].reason;
        const r = ratings[i].value;
        if (!r || Array.isArray(r) || Object.keys(r).sort().join(',') !== 'difficulty,justification,stem' || r.stem !== stems[i] || !ratingSchema.properties.difficulty.enum.includes(r.difficulty) || typeof r.justification !== 'string' || !r.justification.trim() || /[\r\n\u2028\u2029]/.test(r.justification)) throw new Error(`invalid estimate for ${stems[i]}`);
        proposed.push(r);
      }
      // Reject batch drift before any writes, then recheck each individual
      // document immediately before its update as well.
      for (const stem of stems) await unchanged(stem);
      await ctx.record('estimate-proposals', { roadmap, apply, proposed, snapshots: Object.fromEntries([...snapshots].map(([stem, value]) => [stem, value.hash])) });
      return proposed;
    }),
    writeback: guarded(async (stem, difficulty, justification) => {
      if (!apply) return { ok: true };
      const current = await unchanged(stem);
      if (typeof current.estimate_snapshot !== 'string' || !current.estimate_snapshot) throw new Error(`estimate snapshot missing: ${stem}; update the RDM binary before applying`);
      const body = `${current.body}\n\n## Estimate\n\n${difficulty} — ${justification}\n`;
      attemptedWrite = true;
      await ctx.rdm(['phase', 'update', stem, '--difficulty', difficulty, '--body', body, '--expected-estimate-snapshot', current.estimate_snapshot, '--no-edit', ...scope], { mutating: true });
      const after = await show(stem);
      if (after.difficulty !== difficulty || after.body.trimEnd() !== body.trimEnd() || hash(after.tags ?? []) !== hash(current.tags ?? [])) throw new Error(`estimate readback failed: ${stem}`);
      intended.set(stem, { body: body.trimEnd(), difficulty, tags: current.tags ?? [], model: after.model });
      return { ok: true };
    }),
    showTier: guarded(async stem => {
      if (!apply) return '';
      const tier = (await show(stem)).model;
      if (typeof tier !== 'string' || !tier) throw new Error(`missing core tier for ${stem}`);
      return tier;
    }),
  });
  async function unchanged(stem) {
    const phase = await show(stem);
    if (hash({ roadmap, project: ctx.identity.project, stem, phase }) !== snapshots.get(stem).hash) throw new Error(`estimate target changed: ${stem}`);
    return phase;
  }
  try {
    const summary = await run({ roadmap, project: ctx.identity.project, rdmBin: ctx.identity.rdmBin });
    if (failure) throw failure;
    let planCommit;
    if (apply && proposed.length) {
      await ctx.rdm(['commit', '-m', `chore(plan): estimate ${roadmap}`], { mutating: true });
      // A scoped commit may exit zero while skipping missing paths. Its human
      // stdout is not an acknowledgement that every intended estimate landed.
      planCommit = safeGit(ctx.identity.planRoot, ['rev-parse', '--verify', 'HEAD']);
      for (const [stem, expected] of intended) {
        const committed = await ctx.rdm(['phase', 'show', stem, ...scope, '--at', planCommit, '--format', 'json'], { json: true });
        if (!committed || hash({ body: committed.body?.trimEnd(), difficulty: committed.difficulty, tags: committed.tags ?? [], model: committed.model }) !== hash(expected)) throw new Error(`committed estimate verification failed: ${stem}`);
      }
      const journal = await ctx.rdm(['session', 'journal', '--format', 'json'], { json: true });
      if (journal?.id !== ctx.session || !Array.isArray(journal.paths) || journal.paths.length !== 0) throw new Error('owned estimate session journal is not settled after commit');
      if (safeGit(ctx.identity.planRoot, ['rev-parse', '--verify', 'HEAD']) !== planCommit) throw new Error('plan HEAD changed during committed estimate verification');
      await ctx.record('estimate-commit-verified', { planCommit, session: ctx.session, stems: [...intended.keys()] });
    }
    const result = { ...summary, estimated: apply ? summary.estimated : [], proposed, applied: apply, ...(planCommit ? { planCommit } : {}) };
    await ctx.record('estimate-result', result);
    return result;
  } catch (error) {
    if (attemptedWrite) {
      const uncertain = new Error(`estimate writes are uncertain; inspect run journal and session ${ctx.session} before restarting: ${error.message}`, { cause: error });
      uncertain.uncertainWrites = true;
      throw uncertain;
    }
    throw error;
  }
}
