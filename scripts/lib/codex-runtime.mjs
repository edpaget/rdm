/** Explicit Codex host integration. Canonical modules retain review/estimate policy. */
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {createRun, safeGit, rdmEnvironment} from './codex-runtime-state.mjs';
import {runCodex, boundedParallel} from './codex-process.mjs';
import {runEstimate} from './codex-runtime-estimate.mjs';
import {buildReviewPipeline, classifyOutcome, shellQuote, resolveReviewers} from '../../.claude/workflows/lib/review.mjs';
import {runPlanReviewDriver} from '../../.claude/workflows/lib/plan-review.mjs';
const hash = text => createHash('sha256').update(text).digest('hex');
const tiers = ['small','medium','large','frontier'];
// The efforts the Codex process guard (codex-process.mjs) will run. Core never
// resolves `max` for the codex host; this list re-checks the boundary anyway.
const codexEfforts = ['low','medium','high','xhigh'];

/**
 * Resolve each step's model AND reasoning effort from core — `rdm model resolve
 * <step> --host codex --format json` — and bind the profile directly. The host
 * spec no longer binds models: `host.tiers` / `host.steps` are refused so a
 * stale spec cannot silently override the operator's `[models]` policy.
 * `host.capabilities`, when present, stays a guard: a resolved model/effort it
 * does not list is refused.
 */
export async function resolveModels(ctx, host, steps, tier) {
  if (host !== undefined && host !== null && typeof host !== 'object') throw new Error('host must be an object when supplied');
  for (const key of ['tiers','steps']) {
    if (host?.[key] !== undefined) throw new Error(`host.${key} is no longer supported: model/effort now come from \`rdm model resolve --host codex\`; configure [models.profiles.codex.<tier>] / [models.steps] instead`);
  }
  const result = {};
  for (const step of steps) {
    const resolved = await ctx.rdm(['model','resolve',step,...(tier ? ['--tier',tier] : []),'--host','codex','--format','json'],{json:true});
    if (resolved.step !== step || !tiers.includes(resolved.tier) || (resolved.host !== undefined && resolved.host !== 'codex')) throw new Error('Invalid core model resolution');
    const {model, effort} = resolved;
    if (typeof model !== 'string' || !model || /^(haiku|sonnet|opus|fable)(-|$)/i.test(model) || !codexEfforts.includes(effort) ||
        (host?.capabilities !== undefined && !(Array.isArray(host.capabilities[model]) && host.capabilities[model].includes(effort))))
      throw new Error(`Unsupported model/effort for ${step}/${resolved.tier}`);
    result[step] = {model,effort,tier:resolved.tier};
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
    const role = opts.agentType ?? (label.startsWith('find:') ? 'finder' : label.startsWith('refute:') ? 'refuter' : label.startsWith('consolidate:') ? 'consolidator' : undefined);
    if (!['finder','refuter','consolidator','estimator'].includes(role)) throw new Error('Unsupported judgment role');
    const step = role === 'finder' ? 'review-find' : role === 'refuter' ? 'review-verify' : role === 'consolidator' ? 'review-consolidate' : 'plan';
    const binding = models[step];
    if (!binding) throw new Error(`Missing model configuration for ${step}`);
    const id = ++call;
    const evidenceDir = path.join(ctx.runDir,`call-${id}`);
    fs.mkdirSync(evidenceDir,{mode:0o700});
    ctx.record('agent-started',{id,label,role,step,binding});
    try {
      const result = await runCodex({cwd:ctx.identity.sourceDir,prompt,schema:opts.schema,
        env:rdmEnvironment({...ctx.identity, session:ctx.session ?? ctx.identity.session}),
        model:binding.model,effort:binding.effort,timeoutMs:spec.agentTimeoutMs ?? 180000,
        evidenceDir,label:`call-${id}`,signal:spec.signal});
      ctx.record('agent-completed',{id,label,threadId:result.threadId,usage:result.usage,elapsedMs:result.elapsedMs});
      return result.value;
    } catch (error) { ctx.record('agent-failed',{id,label,message:error.message}); throw error; }
  };
}
function requireComplete(result, code = false) {
  const acSelected = code && result.coverage?.selected?.includes('ac');
  if (result.coverage?.complete !== true || result.budget?.passedThroughBudget > 0 || result.budget?.refuterErrors > 0 || (acSelected && !result.acTable?.length)) throw new Error('Review incomplete: missing coverage, AC evidence, or independent refutation');
}

/**
 * Resolve the per-run refutation budget for a review operation. The spec's
 * `maxRefutations` is this runtime's payload layer and wins outright. Otherwise
 * ONE `rdm config get max_refutations --raw` applies env > repo > global:
 * every rdm child's env is rebuilt without the host's RDM_* variables, so a
 * non-blank host RDM_MAX_REFUTATIONS is forwarded into that one child
 * explicitly (a blank one is unset and falls through to config). The value is
 * passed down unparsed; the engine's resolveRefutationBudget is the single
 * validator and throws on a malformed one before any agent runs. Returns
 * undefined when nothing is set, so the engine default applies.
 */
async function resolveRefutationBudgetLayer(ctx, spec) {
  if (spec.maxRefutations !== undefined && spec.maxRefutations !== null) {
    ctx.record('refutation-budget',{layer:'payload',value:spec.maxRefutations});
    return spec.maxRefutations;
  }
  const hostValue = process.env.RDM_MAX_REFUTATIONS;
  const envForwarded = typeof hostValue === 'string' && hostValue.trim() !== '';
  const raw = (await ctx.rdm(['config','get','max_refutations','--raw'],{env:envForwarded ? {RDM_MAX_REFUTATIONS:hostValue} : {}})).trim();
  const value = raw === '' ? undefined : raw;
  ctx.record('refutation-budget',{layer:value === undefined ? 'default' : 'config-get',envForwarded,value:value ?? null});
  return value;
}

function reviewerSelection(mode, reviewers) {
  if (reviewers != null && (!Array.isArray(reviewers) || reviewers.some(value => typeof value !== 'string'))) throw new Error('reviewers must be an array of reviewer names');
  resolveReviewers(mode, reviewers);
  return reviewers ?? null;
}

function slug(value, name) {
  if (typeof value !== 'string' || !/^[a-z0-9][a-z0-9-]*$/.test(value)) throw new Error(`Invalid ${name}`);
  return value;
}

/** Review the exact arbitrary implementation-plan snapshot, report only. */
export async function reviewPlan(ctx, spec, deps, models, maxRefutations) {
  const reviewers = reviewerSelection('plan', spec.reviewers);
  if (typeof spec.planFile !== 'string' || !path.isAbsolute(spec.planFile)) throw new Error('Absolute implementation planFile required');
  const planText = fs.readFileSync(spec.planFile,'utf8');
  if (!planText.trim()) throw new Error('Implementation plan is empty');
  ctx.record('plan-snapshot',{path:spec.planFile,sha256:hash(planText),planText});
  const item = spec.item ? await readItem(ctx, spec.item) : null;
  const roadmap = spec.item?.roadmap ?? spec.roadmap;
  if (roadmap !== undefined) slug(roadmap, 'roadmap');
  if (spec.roadmap && spec.item && spec.roadmap !== spec.item.roadmap) throw new Error('Parent roadmap disagrees with review item');
  let pin = {};
  if (item) {
    const source = ctx.identity.sourceDir;
    const head = revision(source, spec.head ?? safeGit(source, ['rev-parse', 'HEAD']));
    const base = revision(source, spec.base ?? head);
    if (head !== safeGit(source, ['rev-parse', 'HEAD'])) throw new Error('Plan review head must equal checkout HEAD');
    safeGit(source, ['merge-base', '--is-ancestor', base, head]);
    pin = {source, base, expectedHead:head, expectedBranch:safeGit(source, ['symbolic-ref', '--short', 'HEAD']), phase:String(spec.item.phase)};
  } else if (spec.base !== undefined || spec.head !== undefined) {
    throw new Error('Plan source revisions require an explicit item');
  }
  // The PATH, not the text. An identifier crosses the engine boundary and each
  // reviewer reads the file itself from the `cat` its prompt names; the snapshot
  // hash above and the re-read below still pin the exact bytes that were graded.
  const result = await runPlanReviewDriver({implementationPlan:true,planFile:spec.planFile,
    reviewers, roadmap, ...pin, rdmBin:shellQuote(ctx.identity.rdmBin ?? 'rdm'), project:ctx.identity.project,
    findModel:models['review-find']?.model,verifyModel:models['review-verify']?.model,
    consolidateModel:models['review-consolidate']?.model,...(maxRefutations === undefined ? {} : {maxRefutations})},
    {...deps,runPlanReview:buildReviewPipeline('plan',deps)});
  if (fs.readFileSync(spec.planFile,'utf8') !== planText) throw new Error('Implementation plan changed during review');
  if (item && (safeGit(pin.source,['rev-parse','HEAD']) !== pin.expectedHead ||
      safeGit(pin.source,['symbolic-ref','--short','HEAD']) !== pin.expectedBranch ||
      JSON.stringify(await readItem(ctx,spec.item)) !== JSON.stringify(item))) throw new Error('Plan review source or item changed during review');
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

async function readApprovedPlan(ctx, planSlug, item, itemData) {
  slug(planSlug, 'planSlug');
  const plan = await ctx.rdm(['plan','show',planSlug,'--project',ctx.identity.project,'--format','json'],{json:true});
  if (plan?.slug !== planSlug || plan.project !== ctx.identity.project || plan.status !== 'approved' ||
      typeof plan.body !== 'string' || !plan.body.trim()) throw new Error('Review requires an approved plan in the selected project');
  if (item && plan.implements !== `rdm:phase/${item.roadmap}/${itemData.stem ?? item.phase}`) throw new Error('Approved plan does not implement the review item');
  return plan;
}

/** Review a clean, pinned ancestor range and recheck all snapshots before reporting. */
export async function reviewCode(ctx, spec, deps, models, tier, initialItem, maxRefutations) {
  const reviewers = reviewerSelection('code', spec.reviewers);
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
  const plan=spec.planSlug === undefined ? null : await readApprovedPlan(ctx,spec.planSlug,spec.item,item);
  const body=item?.body ?? spec.target;
  if (typeof body !== 'string' || !body.trim()) throw new Error('Explicit target/acceptance criteria required');
  const target=`Review source ${root}, exact range ${base}..${head}. Read relevant files in this checkout.\n\n${body}\n\nDiff:\n${diff}`;
  ctx.record('code-snapshot',{base,head,changedFiles,targetHash:hash(target),item,plan});
  const planCommand = plan ? `${shellQuote(ctx.identity.rdmBin ?? 'rdm')} plan show ${shellQuote(spec.planSlug)} --project ${shellQuote(ctx.identity.project)} --format json` : null;
  const result=await buildReviewPipeline('code',deps)({target,reviewers,planCommand,findModel:models['review-find']?.model,verifyModel:models['review-verify']?.model,consolidateModel:models['review-consolidate']?.model,...(maxRefutations === undefined ? {} : {maxRefutations})});
  clean(root);
  if (safeGit(root,['rev-parse','HEAD']) !== head || (item && JSON.stringify(await readItem(ctx,spec.item)) !== JSON.stringify(item))) throw new Error('Review target changed during review');
  if (plan && JSON.stringify(await readApprovedPlan(ctx,spec.planSlug,spec.item,item)) !== JSON.stringify(plan)) throw new Error('Approved plan changed during review');
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
    const models=await resolveModels(ctx,spec.host,spec.operation==='estimate'?['plan']:['review-find','review-verify','review-consolidate'],tier);
    const rawAgent=createJudgmentAgent(ctx,models,{...spec,signal});
    // Every canonical parallel boundary shares the configured bound. Estimate uses
    // its own Promise.all, so the agent itself also uses this FIFO semaphore.
    let active=0; const queue=[];
    const agent=async (...args)=>{if(active>=concurrency)await new Promise(r=>queue.push(r));else active++;
      try{return await rawAgent(...args);}finally{const next=queue.shift();if(next)next();else active--;}};
    const parallel=ts=>boundedParallel(ts,concurrency);
    const deps={agent,parallel,log:message=>ctx.record('canonical-log',{message}),pipeline:async(xs,...stages)=>parallel(xs.map(x=>async()=>{for(const s of stages)x=await s(x);return x;}))};
    const maxRefutations=spec.operation==='estimate' ? undefined : await resolveRefutationBudgetLayer(ctx,spec);
    let result;
    if(spec.operation==='plan-review')result=await reviewPlan(ctx,spec,deps,models,maxRefutations);
    if(spec.operation==='code-review')result=await reviewCode(ctx,spec,deps,models,tier ?? models['review-find'].tier,item,maxRefutations);
    if(spec.operation==='estimate')result=await runEstimate({ctx,roadmap:spec.roadmap,apply:spec.apply===true,agent});
    if (signal.aborted) throw new Error('Codex runtime cancelled');
    return ctx.finish({operation:spec.operation,reportOnly:spec.operation!=='estimate'||spec.apply!==true,result});
  } catch(error) {ctx.fail(error);throw error;}
  finally {process.removeListener('SIGINT', interrupt);process.removeListener('SIGTERM', interrupt);}
}
