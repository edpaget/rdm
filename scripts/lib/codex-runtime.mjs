/** Explicit Codex host integration. Canonical modules retain review/estimate policy. */
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {createRun, safeGit} from './codex-runtime-state.mjs';
import {runCodex, boundedParallel} from './codex-process.mjs';
import {runEstimate} from './codex-runtime-estimate.mjs';
import {buildReviewPipeline, classifyOutcome} from '../../.claude/workflows/lib/review.mjs';
import {runPlanReviewDriver} from '../../.claude/workflows/lib/plan-review.mjs';
const hash = text => createHash('sha256').update(text).digest('hex');
const tiers = ['small','medium','large'];

/** Resolve core policy first, then choose explicitly declared Codex capabilities. */
export async function resolveModels(ctx, host, steps, tier) {
  if (!host || typeof host !== 'object') throw new Error('Explicit host model configuration required');
  const result = {};
  for (const step of steps) {
    const resolved = await ctx.rdm(['model','resolve',step,...(tier ? ['--tier',tier] : []),'--format','json'],{json:true});
    if (resolved.step !== step || !tiers.includes(resolved.tier)) throw new Error('Invalid core model resolution');
    const override = host.steps?.[step];
    if (override && override.tier !== resolved.tier) throw new Error(`Step ${step} override must retain core tier ${resolved.tier}`);
    const binding = override ?? host.tiers?.[resolved.tier];
    if (!binding || typeof binding.model !== 'string' || /^(haiku|sonnet|opus)(-|$)/i.test(binding.model) ||
        !Array.isArray(host.capabilities?.[binding.model]) || !host.capabilities[binding.model].includes(binding.effort) ||
        !['minimal','low','medium','high','xhigh'].includes(binding.effort)) throw new Error(`Unsupported model/effort for ${step}/${resolved.tier}`);
    result[step] = {...binding,tier:resolved.tier};
    ctx.record('model-resolved',{step,core:resolved,host:result[step]});
  }
  return result;
}

/** Fresh read-only judgment only. Mechanical operations never enter this interface. */
export function createJudgmentAgent(ctx, models, spec) {
  let call = 0;
  return async (prompt, opts = {}) => {
    if (spec.signal?.aborted) throw new Error('Codex judgment cancelled');
    const label = opts.label ?? '';
    const role = opts.agentType ?? (label.startsWith('find:') ? 'finder' : label.startsWith('refute:') ? 'refuter' : undefined);
    if (!['finder','refuter','estimator'].includes(role)) throw new Error('Unsupported judgment role');
    const step = role === 'finder' ? 'review-find' : role === 'refuter' ? 'review-verify' : 'plan';
    const binding = models[step];
    if (!binding) throw new Error(`Missing model configuration for ${step}`);
    const id = ++call;
    const evidenceDir = path.join(ctx.runDir,`call-${id}`);
    fs.mkdirSync(evidenceDir,{mode:0o700});
    ctx.record('agent-started',{id,label,role,step,binding});
    try {
      const result = await runCodex({cwd:ctx.identity.sourceDir,prompt,schema:opts.schema,
        model:binding.model,effort:binding.effort,timeoutMs:spec.agentTimeoutMs ?? 180000,
        evidenceDir,label:`call-${id}`,signal:spec.signal});
      ctx.record('agent-completed',{id,label,threadId:result.threadId,usage:result.usage,elapsedMs:result.elapsedMs});
      return result.value;
    } catch (error) { ctx.record('agent-failed',{id,label,message:error.message}); throw error; }
  };
}
function requireComplete(result, code = false) {
  if (result.coverage?.complete !== true || result.budget?.passedThroughBudget > 0 || result.budget?.refuterErrors > 0 || (code && !result.acTable?.length)) throw new Error('Review incomplete: missing coverage, AC evidence, or independent refutation');
}

/** Review the exact arbitrary implementation-plan snapshot, report only. */
export async function reviewPlan(ctx, spec, deps, models) {
  if (typeof spec.planFile !== 'string' || !path.isAbsolute(spec.planFile)) throw new Error('Absolute implementation planFile required');
  const planText = fs.readFileSync(spec.planFile,'utf8');
  if (!planText.trim()) throw new Error('Implementation plan is empty');
  ctx.record('plan-snapshot',{path:spec.planFile,sha256:hash(planText),planText});
  const result = await runPlanReviewDriver({implementationPlan:true,planText,gateMode:'return',
    findModel:models['review-find']?.model,verifyModel:models['review-verify']?.model},
    {...deps,runPlanReview:buildReviewPipeline('plan',deps)});
  if (fs.readFileSync(spec.planFile,'utf8') !== planText) throw new Error('Implementation plan changed during review');
  requireComplete(result);
  return result;
}
function revision(root, value) {
  if (typeof value !== 'string' || !/^[a-f0-9]{40,64}$/.test(value)) throw new Error('Explicit full commit revision required');
  return safeGit(root,['rev-parse','--verify',`${value}^{commit}`]);
}
function clean(root) {if (safeGit(root,['status','--porcelain','--untracked-files=all'])) throw new Error('Code review requires a clean checkout');}
async function readItem(ctx, item) {
  if (!item || item.type !== 'phase' || !/^[a-z0-9][a-z0-9-]*$/.test(item.roadmap) || !/^[a-z0-9][a-z0-9-]*$/.test(String(item.phase))) throw new Error('Supported item is an explicit phase and roadmap');
  const data = await ctx.rdm(['phase','show',String(item.phase),'--roadmap',item.roadmap,'--project',ctx.identity.project,'--format','json'],{json:true});
  if (typeof data.body !== 'string' || !data.body.trim() || data.roadmap !== item.roadmap) throw new Error('Invalid phase identity/body');
  const worktrees = await ctx.rdm(['worktree','list','--format','json'],{json:true});
  if (!worktrees.some(w=>w.item===item.roadmap && fs.realpathSync(w.path)===ctx.identity.sourceDir && w.branch===safeGit(ctx.identity.sourceDir,['symbolic-ref','--short','HEAD']))) throw new Error('Phase worktree identity mismatch');
  return data;
}

/** Review a clean, pinned ancestor range and recheck all snapshots before reporting. */
export async function reviewCode(ctx, spec, deps, models, tier, initialItem) {
  const root=ctx.identity.sourceDir;
  clean(root);
  const base=revision(root,spec.base), head=revision(root,spec.head);
  if (head !== safeGit(root,['rev-parse','HEAD'])) throw new Error('Review head must equal checkout HEAD');
  safeGit(root,['merge-base','--is-ancestor',base,head]);
  const diff=safeGit(root,['diff','--no-ext-diff','--no-textconv',base,head,'--']);
  if (!diff) throw new Error('Review range is empty');
  const changedFiles=safeGit(root,['diff','--name-only',base,head,'--']).split('\n');
  const currentItem=spec.item ? await readItem(ctx,spec.item) : null;
  if (initialItem && JSON.stringify(currentItem) !== JSON.stringify(initialItem)) throw new Error('Review target changed during model resolution');
  const item=initialItem ?? currentItem;
  const body=item?.body ?? spec.target;
  if (typeof body !== 'string' || !body.trim()) throw new Error('Explicit target/acceptance criteria required');
  const target=`Review source ${root}, exact range ${base}..${head}. Read relevant files in this checkout.\n\n${body}\n\nDiff:\n${diff}`;
  ctx.record('code-snapshot',{base,head,changedFiles,targetHash:hash(target),item});
  const result=await buildReviewPipeline('code',deps)({target,reviewers:spec.reviewers ?? null,findModel:models['review-find']?.model,verifyModel:models['review-verify']?.model});
  clean(root);
  if (safeGit(root,['rev-parse','HEAD']) !== head || (item && JSON.stringify(await readItem(ctx,spec.item)) !== JSON.stringify(item))) throw new Error('Review target changed during review');
  requireComplete(result,true);
  return {...result,base,head,outcome:classifyOutcome({codeReviews:[result.survivors],acTable:result.acTable,tier})};
}

/** Run an explicitly authorized operation, writing durable evidence but no review gates. */
export async function runRuntime(spec) {
  if (!['plan-review','code-review','estimate'].includes(spec.operation)) throw new Error('Unsupported runtime operation');
  const concurrency=spec.concurrency ?? 3;
  if (!Number.isSafeInteger(concurrency) || concurrency < 1 || concurrency > 8) throw new Error('concurrency must be 1..8');
  if (spec.agentTimeoutMs !== undefined && (!Number.isSafeInteger(spec.agentTimeoutMs) || spec.agentTimeoutMs < 1 || spec.agentTimeoutMs > 3600000)) throw new Error('Invalid agentTimeoutMs');
  if (spec.signal !== undefined && !(spec.signal instanceof AbortSignal)) throw new Error('signal must be an AbortSignal');
  const controller = new AbortController();
  const interrupt = () => controller.abort();
  const signal = spec.signal ? AbortSignal.any([spec.signal, controller.signal]) : controller.signal;
  const ctx=createRun({...spec,signal});
  process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);
  try {
    const item=spec.operation==='code-review' && spec.item ? await readItem(ctx,spec.item) : null;
    const tier=item?.model ?? spec.tier;
    if (tier !== undefined && !tiers.includes(tier)) throw new Error('Invalid core tier hint');
    const models=await resolveModels(ctx,spec.host,spec.operation==='estimate'?['plan']:['review-find','review-verify'],tier);
    const rawAgent=createJudgmentAgent(ctx,models,{...spec,signal});
    // Every canonical parallel boundary shares the configured bound. Estimate uses
    // its own Promise.all, so the agent itself also uses this FIFO semaphore.
    let active=0; const queue=[];
    const agent=async (...args)=>{if(active>=concurrency)await new Promise(r=>queue.push(r));else active++;
      try{return await rawAgent(...args);}finally{const next=queue.shift();if(next)next();else active--;}};
    const parallel=ts=>boundedParallel(ts,concurrency);
    const deps={agent,parallel,log:message=>ctx.record('canonical-log',{message}),pipeline:async(xs,...stages)=>parallel(xs.map(x=>async()=>{for(const s of stages)x=await s(x);return x;}))};
    let result;
    if(spec.operation==='plan-review')result=await reviewPlan(ctx,spec,deps,models);
    if(spec.operation==='code-review')result=await reviewCode(ctx,spec,deps,models,tier ?? models['review-find'].tier,item);
    if(spec.operation==='estimate')result=await runEstimate({ctx,roadmap:spec.roadmap,apply:spec.apply===true,agent});
    if (signal.aborted) throw new Error('Codex runtime cancelled');
    return ctx.finish({operation:spec.operation,reportOnly:spec.operation!=='estimate'||spec.apply!==true,result});
  } catch(error) {ctx.fail(error);throw error;}
  finally {process.removeListener('SIGINT', interrupt);process.removeListener('SIGTERM', interrupt);}
}
