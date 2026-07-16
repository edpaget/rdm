# Subagent dispatch enforcement

## The observed failure

Phase 1 of the `roadmap-autopilot-dispatch-hardening` roadmap fixed *how*
subagents are dispatched (synchronously, blocking, no `SendMessage` back to
the orchestrator). This phase addresses a distinct, observed failure: older
or less capable models frequently do not dispatch subagents at all. Instead
of spawning a planner, plan reviewer, implementer, and code reviewer as
separate `Agent` calls, they collapse all of those roles into inline work in
a single context — reading the phase, then drafting a plan, writing code, and
"reviewing" it themselves without ever invoking the `Agent` tool. This
defeats the context isolation and independent-review guarantees `rdm-autopilot`
and `rdm-dispatch-phase` are built on: a planner that also reviews its own
plan, or an implementer that also code-reviews its own diff, provides no
independent check at all.

The skill prose before this phase *described* the dispatch flow ("dispatch a
planning subagent with the `Agent` tool...") but did not *compel* it. To a
capable model, descriptive prose reads as the obvious next action. To a
weaker model under context pressure, it can read as optional-sounding
narration — something to summarize rather than an action to take.

This note evaluates prompt-engineering techniques analytically (the phase
permits this without web research) to decide which ones to encode as hard,
non-skippable requirements in `rdm-autopilot` and `rdm-dispatch-phase`.
Version/capability framing ("older models", "weaker models") is deliberately
confined to this note and the changelog — it never appears in the skill
files themselves, which must read the same regardless of which model is
executing them.

## Techniques evaluated

All seven were adopted. They are cheap to include, complementary (each
targets a different point in the failure chain — skipping the instructions
entirely, drifting mid-step, or having no way to self-catch a drift), and
none conflicts with another.

1. **Imperative MUST/MUST NOT framing.** Descriptive prose ("dispatch a
   planning subagent") reads as optional to a weaker model; imperative modal
   verbs remove that ambiguity. Applied throughout the new "Mandatory
   dispatch" sections and every step restatement.
2. **Explicit prohibition on inline implementation.** Naming the exact thing
   not to do is more reliably followed than an instruction to do the right
   thing alone. Applied as the "You (the loop/dispatcher) MUST NOT..."
   sentence that opens each "Mandatory dispatch" section.
3. **Pre-action declaration.** A stated declaration of which subagent/role is
   about to be dispatched makes a skipped dispatch visibly absent from the
   transcript rather than silently missing. Checklist item 1 in each
   "Mandatory dispatch" section.
4. **Self-verification checkpoint / refusal-to-proceed gate.** A model that
   must affirmatively confirm a subagent returned before moving on is less
   likely to silently substitute its own inline output for that subagent's.
   Applied as "Self-check before proceeding" paragraphs at each point of
   action (autopilot step 2; dispatch-phase steps 4, 5, 6, and 7's rework
   bullet), plus checklist item 4 in each "Mandatory dispatch" section.
5. **Restating the requirement at the point of action.** A preamble stated
   once at the top of a skill is diluted by the time the model reaches the
   actual dispatch step several paragraphs or tool calls later; restating the
   constraint inline keeps it present exactly when the model decides whether
   to call the `Agent` tool.
6. **Concrete negative example naming the failure mode.** A model
   pattern-matches against a named failure ("inline-collapse") more
   reliably than an abstract prohibition. One canonical paragraph per skill
   (in the "Mandatory dispatch" section), referenced — not re-narrated — at
   each restatement point, keeping the addition non-repetitive.
7. **Structural cues (numbered checklist).** A numbered declare → call →
   block → verify checklist is easier to mechanically follow than prose, and
   easier for a reviewer (human or subagent) to audit against. The 4-item
   checklist in each "Mandatory dispatch" section.

## Where they land

- `.claude/skills/rdm-autopilot/SKILL.md` and its CLI/MCP templates
  (`rdm-core/src/templates/skill-autopilot-{cli,mcp}.md`): a new `### Mandatory
  dispatch — no inline work` section inside `## Loop`, after the
  bounded-state paragraph and before step 1; a "Self-check before proceeding"
  bullet appended to step 2's dispatch bullet list.
- `.claude/skills/rdm-dispatch-phase/SKILL.md` and its CLI/MCP templates
  (`rdm-core/src/templates/skill-dispatch-phase-{cli,mcp}.md`): a new `##
  Mandatory dispatch — no inline work` section between `## Dispatch contract`
  and `## Steps`; "Self-check before proceeding" paragraphs at the end of
  steps 4, 5, and 6, and a one-sentence self-check appended to step 7's
  fail-fixable rework bullet; a mandatory-isolation lead-in prepended to `##
  Context isolation`.
- The CLI and MCP templates under `rdm-core/src/templates/` were also
  brought into sync with phase 1's synchronous-dispatch wording
  (`dispatched synchronously`, `never resume ... by message`, the
  `SendMessage`-to-orchestrator prohibition), which the two dogfood
  `SKILL.md` files already had but the four templates were missing.

## Model-agnostic by design

None of the added prose names a model, vendor, or capability tier — that
framing belongs only in this note and `CHANGELOG.md`. The new language is
explicitly framed as layering **on top of**, not replacing, phase 1's
synchronous-dispatch contract: "dispatched synchronously" describes *how* a
dispatch happens once it happens; "Mandatory dispatch — no inline work"
guarantees the dispatch *happens at all*. The two are consistent and
reference the same key terms (`MUST NOT`, `inline-collapse`, `Self-check
before proceeding`, `Mandatory dispatch`) across both skills.
