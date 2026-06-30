# Landing reviewed work (`rdm-land`) & pruning worktrees

The autonomous-execution skills drive a roadmap (or task) to `reviewed` on its
own `roadmap/<slug>` / `task/<slug>` worktree branch, but they deliberately
**never touch `main`**. `rdm-land` is the landing tail: it integrates one
`reviewed` item into `main` with **linear history**, then cleans up after it.
`rdm worktree prune` is the batch broom that removes worktrees whose plan item is
already `done`.

## The land flow

`rdm-land` takes a single **item ref** (`<roadmap>/<phase>`, `task/<slug>`, or a
bare `<roadmap>`) and runs:

1. **Read item status** — confirm the item is `reviewed`.
2. **Update `main`** — refresh `main` in the **primary** worktree, but only if it
   tracks an upstream (`git -C <primary> pull --ff-only`). rdm's worktree model
   is local-only by default: branches are cut from the local `main`, so when
   there is no upstream this step is skipped — `main` is already the rebase base.
3. **Rebase the item's branch onto `main`** — from inside the item's worktree,
   `git rebase main` (no `git checkout main` — see the worktree note below).
4. **Re-run the CI-equivalent checks** on the rebased branch:
   ```bash
   cargo fmt --check
   cargo clippy -- -D warnings
   cargo nextest run
   ```
5. **Fast-forward `main`** — advance `main` from the primary worktree where it is
   checked out: `git -C <primary> merge --ff-only <branch>`.
6. **Confirm the item flipped `reviewed → done`** — the fast-forward onto the
   default branch fires `rdm hook post-commit`, which reads the `Done:` line and
   marks the item done. If hooks aren't installed, the idempotent fallback is
   `rdm phase update <phase> --status done --commit <sha>` (or the `task`
   variant).
7. **Clean up** — `rdm worktree remove <item> --delete-branch` for this item, or
   `rdm worktree prune` for batch cleanup of all already-`done` items.

> **Worktree topology.** rdm uses one worktree per roadmap: the item's branch
> lives in a *linked* worktree while `main` stays checked out in the **primary**
> worktree (the first entry in `git worktree list` — `<primary>` above). git
> refuses to check out a branch already checked out elsewhere, so landing never
> runs `git checkout main` from the item worktree — it rebases in place (`main`
> is a readable ref) and fast-forwards `main` via `git -C <primary>`.

### Preconditions (all must hold)

Landing checks these before touching `main`, and **aborts** if any fail:

- the item is `reviewed`;
- its branch carries the `Done: <item>` line (what the post-commit hook reads);
- the worktree is clean (no uncommitted changes);
- the CI-equivalent checks pass **on the rebased branch** (step 4).

## The linear-history guarantee

`main` only ever advances by a **fast-forward**: rebase onto `main`, then
`git merge --ff-only`. A fast-forward produces **no merge commit**, so history
stays linear. If a fast-forward is not possible, that is a signal to abort — never
fall back to a merge commit and never force.

The fast-forward is also exactly what triggers `rdm hook post-commit` on the
default branch to flip the item `reviewed → done`. The existing `Done:`-line
mechanism keeps working unchanged; `rdm-land` writes no `Done:` line by hand.

## Safety posture — explicit, opt-in only

Landing is the one step that **writes to `main`**, so it is gated:

- It runs **only on explicit invocation** — when you run `/rdm-land`, or when
  [`rdm-autopilot`](./autonomous-loop.md) is given its opt-in `--land` flag
  (default OFF).
- It **never auto-lands**. Nothing in the normal review flow reaches `main` on
  its own; reviewed work waits on its branch until a human (or an explicitly
  `--land`-enabled run) lands it.

## Abort and escalate (never force-merge)

On a **rebase conflict** or **failing checks**, `rdm-land`:

- runs `git rebase --abort` (or `git merge --abort`) to restore the branch's
  pre-landing state;
- leaves the worktree **intact** — never `git reset --hard`, force-push,
  force-merge, or discard work;
- surfaces an actionable escalation per
  [`docs/escalation-protocol.md`](./escalation-protocol.md): which precondition
  or step failed, the conflicting files or failing check, and that `main` was
  left untouched. The item stays `reviewed`, ready for a human to resolve and
  re-run landing.

## `rdm worktree prune`

Once items are `done`, their worktrees are dead weight. `prune` removes every
rdm-managed worktree whose plan item resolves to `done`, in one invocation:

```bash
rdm worktree prune                  # remove all done worktrees
rdm worktree prune --delete-branch  # also delete their (merged) branches
rdm worktree prune --dry-run        # report what would be removed, change nothing
rdm worktree prune --force          # prune even dirty done worktrees
```

A done item's status is resolved the same way `rdm next` computes it: a phase is
done when its status is `done`; a task when its status is `done`; a bare roadmap
when every one of its phases is terminal. Non-done worktrees are never
candidates. Dirty worktrees are skipped unless `--force`, so in-flight work is
never silently discarded. `--delete-branch` defaults **off**, matching
`rdm worktree remove`.

The single-item `rdm worktree remove <item> --delete-branch` (run by `rdm-land`
at the end of a successful landing) and the batch `rdm worktree prune` (for
end-of-run cleanup) are complementary: the former tidies the item you just
landed; the latter sweeps everything that has since reached `done`.

## MCP variant

The MCP build of `rdm-land` drives the status reads/updates and single-worktree
cleanup through the `mcp__rdm__*` tools, but **delegates the git landing
(steps 2–5) and the batch prune to a Bash-capable subagent** via the `Agent`
tool: rebase and `merge --ff-only` are inherently shell work, and
`rdm worktree prune` ships as a CLI command only this phase (the parallel
`rdm_worktree_prune` MCP tool is a deferred follow-up).

## See also

- [`docs/autonomous-loop.md`](./autonomous-loop.md) — the `rdm-autopilot` driver
  whose opt-in `--land` flag invokes this flow.
- [`docs/escalation-protocol.md`](./escalation-protocol.md) — the shared rule for
  what escalates and how parked escalations are recorded and resumed.
