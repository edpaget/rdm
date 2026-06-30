// Pi extension: reprompt the agent to run rdm-review while an item is in `needs-review`.
//
// This is the Pi analog of the Claude Code Stop hook (rdm-review-on-finalize.sh). Pi has
// no settings.json hooks; instead, an extension subscribes to the `agent_end` lifecycle
// event (the same mechanism Pi's shipped git-checkpoint.ts uses). Pi auto-discovers
// extensions from `.pi/extensions/*.ts` (project) and `~/.pi/agent/extensions/*.ts` (user).
//
// On every `agent_end`, the extension asks rdm whether any phase or task is in
// `needs-review`. If so, it injects a user message asking the agent to run the rdm-review
// skill, then lets that injected turn settle before checking again. The `needs-review`
// status itself is the sentinel — once rdm-review moves the item to `reviewed` (or any
// other status), the next `agent_end` finds nothing pending and the agent is allowed to
// finish.
//
// Loop prevention mirrors the shell hook's `stop_hook_active` guard: a closure-scoped
// `justInjected` flag. The `agent_end` that fires as a result of *our* injected turn is
// skipped (we only reset the flag and return), so we never wedge the agent in a loop.
// Caveat (shared with the shell hook): the guard is one-shot, keyed on "did we just
// inject" rather than on the pending set shrinking. If an injected review turn resolves
// only some of several pending items, the next `agent_end` is allowed through with items
// still in `needs-review`. The review prompt asks the agent to handle *all* pending
// item(s), which mitigates this; a later `agent_end` (after more work) re-checks and will
// re-prompt if any remain.
//
// One-worktree-per-roadmap model: Pi cannot move its own cwd, so the Pi session runs in
// place in the roadmap worktree (or on the `roadmap/<slug>` branch). `agent_end` therefore
// fires against the stamped roadmap branch, and the branch-scoped `rdm review pending`
// filter resolves exactly that roadmap's tree — no per-phase worktree, no nested move.
//
// `rdm` is invoked on PATH without an explicit project flag — project resolution follows
// the standard chain (`RDM_PROJECT` env var, then `default_project` in `rdm.toml`).
//
// Fail open: any error from `rdm.exec` or JSON parsing is swallowed and the agent is
// allowed to finish — a misconfigured plan repo must never block the agent.
//
// Manual test (from a directory with a configured plan repo):
//   1. Put an item in needs-review:  rdm task update <slug> --status needs-review --no-edit
//   2. Run Pi with this extension loaded:  pi -e ./rdm-review.ts
//   3. Ask the agent to finish — it should be re-prompted to run rdm-review until the
//      item leaves needs-review.

export default function (pi) {
  // Loop prevention: set after we inject a review prompt so the `agent_end` triggered by
  // our own injected turn is allowed through instead of re-injecting forever.
  let justInjected = false;

  pi.on("agent_end", async (event, ctx) => {
    let pending = [];
    try {
      // Refresh any stale stamp first (fail-open): re-point review_sha/review_branch at
      // the current HEAD/branch so a commit amended or rebased while an item is still
      // needs-review doesn't go stale and silently drop out of scope. Idempotent, so
      // running it on every agent_end is cheap. Mirrors the shell hook.
      try {
        await pi.exec("rdm", ["review", "restamp"], { timeout: 30000 });
      } catch (_) {
        // Fail open: a restamp error must never block the pending check below.
      }
      // `rdm review pending` returns the needs-review phases AND tasks in scope for the
      // current source-repo branch (branch-identity, with a SHA-reachability fallback for
      // legacy/unstamped items). It is the single shared source of truth for the hook,
      // this extension, and the rdm-review skill, so a session finishing one branch is
      // never reprompted to review work finalized on another. This mirrors the shell hook.
      const result = await pi.exec(
        "rdm",
        ["review", "pending", "--format", "json"],
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
      // This `agent_end` is the result of our own injected review turn. Let it settle
      // exactly like the shell hook skips when `stop_hook_active` is true.
      justInjected = false;
      return;
    }

    // Pending items and we did not just inject: re-prompt the agent to review them.
    await pi.sendUserMessage(
      "There are rdm item(s) in `needs-review`. Before stopping, invoke the rdm-review " +
        "skill on the needs-review item(s): categorize the findings — fix small issues " +
        "inline, and file large ones as rdm tasks. If review passes, set the item's status " +
        "to `reviewed` and write the `Done:` line in the commit message.",
    );
    justInjected = true;
  });
}
