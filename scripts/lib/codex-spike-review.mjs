// Experimental host wiring only: all findings, refutation and verdict policy
// remain in the canonical workflow modules. No real item gate is written here.
import {buildReviewPipeline, classifyOutcome} from '../../.claude/workflows/lib/review.mjs';
import {runPlanReviewDriver} from '../../.claude/workflows/lib/plan-review.mjs';

export function requireCompleteReview(result, code = false) {
  if (result.coverage?.complete !== true || result.budget?.passedThroughBudget > 0 ||
      result.budget?.refuterErrors > 0 || (code && !result.acTable?.length)) {
    throw new Error('Spike evidence incomplete: missing review coverage, AC table, or independent refutation');
  }
}

export async function runReviewExperiment({agent, target, model, parallel, log = () => {}}) {
  const deps = {
    agent, log,
    parallel: parallel ?? (async thunks => Promise.all(thunks.map(async thunk => {
      try { return await thunk(); } catch { return null; }
    }))),
    pipeline: async (items, ...stages) => Promise.all(items.map(async item => {
      for (const stage of stages) item = await stage(item);
      return item;
    })),
  };
  const code = await buildReviewPipeline('code', deps)({
    target, findModel: model, verifyModel: model,
    signals: {changesLogic: true, missingTests: true},
  });
  requireCompleteReview(code, true);
  const codeOutcome = classifyOutcome({codeReviews: [code.survivors], acTable: code.acTable, tier: 'medium'});
  const plan = await runPlanReviewDriver({
    implementationPlan: true,
    planText: 'Implement sum.mjs add(a,b) using addition; first add a failing test asserting add(2,3) === 5, then implement and run node --test. Keep the existing API and add no dependencies.',
    findModel: model, verifyModel: model, gateMode: 'return',
  }, {...deps, runPlanReview: buildReviewPipeline('plan', deps)});
  requireCompleteReview(plan);
  return {code, codeOutcome, plan};
}
