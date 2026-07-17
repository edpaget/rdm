// Pi extension: reprompt the agent to run rdm-plan-review while an item carries the
// `needs-plan-review` sentinel tag.
//
// This is the Pi analog of the Claude Code Stop hook (hook-plan-review-on-create.sh).
// Pi has no settings.json hooks; instead, an extension subscribes to the `agent_end`
// lifecycle event (the same mechanism Pi's shipped git-checkpoint.ts uses, and the same
// mechanism rdm-review.ts uses for the needs-review sentinel). Pi auto-discovers
// extensions from `.pi/extensions/*.ts` (project) and `~/.pi/agent/extensions/*.ts`
// (user).
//
// On every `agent_end`, the extension asks rdm whether any roadmap, phase, or task
// carries the `needs-plan-review` tag. If so, it injects a user message asking the
// agent to run the rdm-plan-review skill, then lets that injected turn settle before
// checking again. The tag itself is the sentinel — once rdm-plan-review clears it (on
// PASS or PASS WITH CONCERNS), the next `agent_end` finds nothing pending and the agent
// is allowed to finish.
//
// Loop prevention mirrors the shell hook's `stop_hook_active` guard, and rdm-review.ts's
// own `justInjected` pattern: a closure-scoped flag skips the `agent_end` that fires as
// a result of *our* injected turn, so we never wedge the agent in a loop.
//
// Divergence from rdm-review.ts (deliberate): this extension does NOT call `rdm review
// restamp`. Restamping exists to keep a branch/commit-scoped stamp from going stale
// across an amend or rebase. The `needs-plan-review` tag carries no branch/commit scope
// at all — it is a plain tag on the item — so there is nothing to go stale.
//
// `rdm` is invoked on PATH without an explicit project flag — project resolution
// follows the standard chain (`RDM_PROJECT` env var, then `default_project` in
// `rdm.toml`).
//
// Fail open: any error from `rdm.exec` or JSON parsing is swallowed and the agent is
// allowed to finish — a misconfigured plan repo must never block the agent.
//
// Manual test (from a directory with a configured plan repo that has `plan_review =
// true`):
//   1. Create an item (stamps the tag):  rdm task create demo-item --title "Demo" --no-edit
//   2. Run Pi with this extension loaded:  pi -e ./rdm-plan-review.ts
//   3. Ask the agent to finish — it should be re-prompted to run rdm-plan-review until
//      the tag is cleared.

export default function (pi) {
  // Loop prevention: set after we inject a plan-review prompt so the `agent_end`
  // triggered by our own injected turn is allowed through instead of re-injecting
  // forever.
  let justInjected = false;

  pi.on("agent_end", async (event, ctx) => {
    let pending = [];
    try {
      // `rdm search` with no --type spans roadmaps, phases, and tasks in one call. It is
      // the single shared source of truth for the hook, this extension, and the
      // rdm-plan-review skill.
      const result = await pi.exec(
        "rdm",
        ["search", "", "--tag", "needs-plan-review", "--format", "json"],
        { timeout: 30000 },
      );
      const items = JSON.parse(result.stdout);
      for (const item of items) {
        if (item && item.identifier) {
          pending.push(item.identifier);
        }
      }
    } catch (err) {
      // Fail open: never wedge the agent on a query or parse error.
      return;
    }

    if (pending.length === 0) {
      // Nothing pending — allow the agent to finish and clear any prior injection state.
      justInjected = false;
      return;
    }

    if (justInjected) {
      // This `agent_end` is the result of our own injected plan-review turn. Let it
      // settle exactly like the shell hook skips when `stop_hook_active` is true.
      justInjected = false;
      return;
    }

    // Pending items and we did not just inject: re-prompt the agent to plan-review them.
    await pi.sendUserMessage(
      "There are rdm item(s) tagged `needs-plan-review`. Before stopping, invoke the " +
        "rdm-plan-review skill on the pending item(s) to review the plan before " +
        "implementation begins. On PASS or PASS WITH CONCERNS it clears the tag; on " +
        "REWORK it reports what must change.",
    );
    justInjected = true;
  });
}
