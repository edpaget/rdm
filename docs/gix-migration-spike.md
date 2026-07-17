# Spike: Evaluate consolidating git operations on `gix` only

**Status:** no-go on full consolidation, as of 2026-07-16. Local object-database reads/writes stay on `gix`; clone/fetch/push/merge/worktree-admin stay on the `git` CLI for now.

## Summary / Recommendation

**No-go on full CLI-to-`gix` consolidation at this time.** `gix` 0.85 (the version pinned in this workspace) does not yet implement `push`, merge workflow orchestration, `MERGE_HEAD`-compatible conflict-state persistence, or `git worktree` create/remove. Those are exactly the operations `rdm-store-git`'s remote/merge porcelain and `rdm-git`'s worktree lifecycle depend on, so a full switch is not achievable — let alone "clean," which is the bar the originating phase set.

`clone` and `fetch` are comparatively mature in `gix` and could plausibly be migrated in isolation, but this document recommends against doing that alone: `git` would still need to be on `PATH` for push, merge, and worktree admin, so motivation #2 (the implicit runtime dependency on `git`) stays completely unsolved, while motivation #1 (a dual mental model) gets *worse* — contributors would now need to know which of the two libraries handles which network operation, for no reduction in the CLI dependency. See [Recommendation](#recommendation--revisit-trigger) below for the full rationale and a concrete revisit trigger.

**No code changes are required or recommended in `rdm-git` or `rdm-store-git` as an outcome of this phase.**

## Background

`rdm-store-git` (the plan-repo `GitStore` backend) and `rdm-git` (repo-agnostic git primitives, shared with the project-repo worktree lifecycle) already use `gix` for local object-database work — tree/blob/commit writes and HEAD reads — but shell out to the `git` CLI for clone, fetch, push, merge, and `git worktree` administration. Per the originating phase body (`roadmap-git-storage-backend-consolidation`, phase 1), this dual approach creates three problems:

1. **A dual mental model** (library vs. CLI) for contributors — some git operations are gix calls, others are `Command::new("git")` invocations, with no obvious rule for which is which without reading the source.
2. **An implicit runtime dependency on `git` being on `PATH`** — despite rdm's "zero-dependency" positioning (users need only the compiled binary), remote and merge operations quietly require a system `git` install.
3. **Subtle invariants around index sync** — because `GitRepo::create_git_commit` (`rdm-store-git/src/commit.rs:166`) builds tree objects and writes commits directly via gix, bypassing the git index entirely, the index can drift out of sync with `HEAD`. `GitRepo::sync_index_to_head` (`rdm-store-git/src/commit.rs:376-383`, `git reset` at line 377) exists solely to reset the index back to `HEAD` before any operation that consults it (`git merge`, `git status` as run by a human outside rdm). This is a consequence of the dual gix/CLI design, not a limitation of `gix` itself — worth naming precisely so it isn't miscounted as evidence against `gix`.

The shared non-interactive spawner for every CLI git call is `git_command()` (`rdm-git/src/process.rs:48`): it forces `GIT_EDITOR=true`, `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=true`, and tags the child with `RDM_GIT_SUBPROCESS=1` for hook re-entrancy detection. A `gix`-only implementation would preserve these guarantees natively — `gix` never launches an external editor or credential prompt — which is a real (if minor) point in `gix`'s favor for whichever operations do end up migrated.

## Capability Survey (gix 0.85, re-verified 2026-07-16)

Verified against the `gitoxide` [`crate-status.md`](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md) (fetched 2026-07-16) and [docs.rs `gix` 0.85.0](https://docs.rs/gix/0.85.0/gix/) feature-flag documentation. `Cargo.lock` resolves `gix 0.85.0` (bumped from 0.84.0 in commit `b9f5d4b`, "build(deps): bump gix from 0.84.0 to 0.85.0 (#32)"); both `rdm-git/Cargo.toml:11` and `rdm-store-git/Cargo.toml:13` pin `gix = { version = "0.85", default-features = false, features = ["basic", "index", "revision", "parallel", "sha1"] }` — neither the network transport features nor `worktree-mutation` are currently enabled.

| Operation | `crate-status.md` state (verbatim) | Assessment |
|---|---|---|
| Clone / fetch / ls-refs / shallow variants | `[x] clone, fetch, ls-refs and shallow variants` | Done. |
| Push | `[ ] push and self-contained clone/fetch over file:// and ssh://` | **Not implemented.** Note the bullet bundles push together with *self-contained* (no external transport binary) clone/fetch over `file://`/`ssh://` — the clone/fetch that *is* done above still relies on the `blocking-http-transport-{curl,reqwest}` features for HTTP(S), which are themselves optional add-ons, not a from-scratch reimplementation of every transport. |
| Three-way blob content-merge | `[x] three-way content-merge analysis of blobs` | Done — low-level diff/merge-of-blob-content primitive exists. |
| Merge workflow orchestration | `[ ] merge workflow orchestration` | **Not implemented.** No porcelain equivalent of `git merge` (branch resolution, fast-forward detection, commit creation, conflict staging) exists yet — only the blob-level primitive above. |
| Persist merge-in-progress state (`MERGE_HEAD` etc.) | `[ ] persist and resume conflicted merges with MERGE_HEAD, MERGE_MSG and MERGE_MODE compatible state` | **Not implemented.** `rdm-store-git`'s `git_is_merge_in_progress` (`rdm-store-git/src/merge.rs:40-43`) currently just checks for the `MERGE_HEAD` file directly — there is no gix API to produce or consume that state. |
| Worktree create / move / remove / repair | `[ ] create, move, remove, and repair` | **Not implemented.** (All four checkboxes share this single bullet in the source doc.) The crate does support *opening* repositories that already have worktrees and handling their refs correctly — it just cannot create or tear one down. |

Feature-flag notes (docs.rs, `gix` 0.85.0):
- `blocking-network-client` — "Make `gix-protocol` available along with a blocking client"; required for any clone/fetch.
- `blocking-http-transport-curl` / `blocking-http-transport-reqwest` — HTTP(S) transport add-ons layered on top of `blocking-network-client`.
- `worktree-mutation` — "various ways to alter the worktree makeup by checkout and reset." This is **working-tree file checkout/reset only** — it has nothing to do with `git worktree` (multiple linked working trees), which is the separate, unimplemented capability in the table above. This is an easy naming collision to trip over when reading `gix`'s docs.
- `gix::prepare_clone(url, path)` returns a `clone::PrepareFetch` configured with the local git installation's config for auth. Its `fetch_then_checkout(progress, &should_interrupt)` method (verified against `docs.rs/gix/0.85.0/gix/clone/struct.PrepareFetch.html`) returns `Result<(PrepareCheckout, Outcome), Error>` and itself requires **both** `blocking-network-client` *and* `worktree-mutation` — i.e. even the mature clone/fetch path needs the checkout-flavored feature, not just the network one. The returned `PrepareCheckout` is then driven to `main_worktree(...)` to materialize the working tree. This confirms the API shape cited in the originating plan.

## Call-Site Inventory

Every row below was verified by opening the cited file in this worktree at commit `HEAD` (branch `roadmap/git-storage-backend-consolidation`) on 2026-07-16; line numbers reflect the file as it stands today.

### `rdm-store-git` — headline network/merge sites (the phase's actual subject)

| File:line | Call | Function | `gix` 0.85 capable? |
|---|---|---|---|
| `rdm-store-git/src/lib.rs:214-252` (clone args at 232-239, `gix::open` at 245) | `git clone [--branch <b>] <url> <root>` | `clone_remote` | **Partial.** `gix::prepare_clone` + `fetch_then_checkout` covers this, but needs `blocking-network-client` + a transport feature + `worktree-mutation` added to `rdm-store-git/Cargo.toml`, none of which are enabled today. |
| `rdm-store-git/src/remote.rs:156` | `git fetch <remote>` | `git_fetch` | **Partial.** Same gap as clone — `gix`'s fetch path is mature, but the crate features aren't enabled. |
| `rdm-store-git/src/remote.rs:196-202` | `git push <remote> <branch> [--force]` | `git_push` | **No.** `push` is explicitly unimplemented in `gix` 0.85 (`crate-status.md`: `[ ] push and self-contained clone/fetch`). |
| `rdm-store-git/src/remote.rs:311` | `git merge --no-edit <tracking_ref>` (diverged branches) | `git_pull` | **No.** No merge-workflow orchestration API; also depends on `sync_index_to_head` (see Background #3). |
| `rdm-store-git/src/remote.rs:346` | `git merge --ff-only <tracking_ref>` | `git_pull` (fast-forward case) | **Partial, but not worth isolating.** A fast-forward-only "merge" is really just a ref update to a known-ancestor commit, which `gix` can do natively (`gix_ref` transaction) without needing the full merge-workflow API. Isolating just this branch of `git_pull` would still leave the diverged-branch case (line 311) on the CLI, so it doesn't change the overall recommendation. |
| `rdm-store-git/src/remote.rs:390` | `git rev-parse --verify --quiet <tracking_ref>` | `git_sync_status` (tracking-ref check) | **Yes** (read-only ref resolution — `gix` can do this natively). Currently CLI; see "cheap partial win" below. |
| `rdm-store-git/src/remote.rs:397` | `git rev-list --left-right --count HEAD...<tracking_ref>` | `git_sync_status` (ahead/behind count) | **Yes** (read-only graph walk over the ODB — `gix`'s revision-walk API can compute this). Currently CLI; see "cheap partial win" below. |
| `rdm-store-git/src/merge.rs:20` | `git diff --name-only --diff-filter=U` | `git_list_unmerged` | **Yes**, in principle (index-state read), but only meaningful in the context of an in-progress merge, which `gix` cannot itself create or complete — low value in isolation. |
| `rdm-store-git/src/merge.rs:55` | `git merge --abort` | `git_merge_abort` | **No.** No merge-state API to abort against (`MERGE_HEAD` persistence unimplemented). |
| `rdm-store-git/src/merge.rs:85` | `git add <path>` | `git_resolve_conflict` (stage resolution) | **Partial** (index write is technically reachable via `gix`'s `index` feature already enabled) but only useful alongside the merge-completion step below, which is not. |
| `rdm-store-git/src/merge.rs:98` | `git commit --no-edit` | `git_resolve_conflict` (complete merge) | **No** in the merge context — completing a merge commit requires reading `MERGE_HEAD` as a second parent, which has no `gix` equivalent yet. (Note: ordinary, non-merge commits already go through `gix` directly via `create_git_commit` — see the "already gix" row below.) |
| `rdm-store-git/src/commit.rs:59` | `git symbolic-ref refs/remotes/origin/HEAD` | `default_branch_name` | **Yes** (symbolic-ref read is a native `gix` operation). Currently CLI; see "cheap partial win" below. |
| `rdm-store-git/src/commit.rs:377` | `git reset` | `sync_index_to_head` | N/A — this exists *because* of the dual design (Background #3), not as a capability gap; a full `gix`-only implementation with no CLI-built index divergence wouldn't need it at all. |
| `rdm-store-git/src/commit.rs:554` | `git rev-parse --verify --quiet <sha>^{commit}` | `fetch_body_at` (SHA validation) | **Yes** (native `gix` object lookup). Currently CLI; see "cheap partial win" below. |
| `rdm-store-git/src/commit.rs:567` | `git show <sha>:<path>` | `fetch_body_at` (historical file read) | **Yes** (native `gix` tree/blob lookup at a historical commit, same technique as `walk_tree_blobs` below). Currently CLI; see "cheap partial win" below. |

### `rdm-store-git` — already `gix` (strongest evidence of what it already does well)

`rdm-store-git/src/commit.rs:75-200` (`build_tree_from_dir`, `create_git_commit`) and `:282-541` (`git_status`, `git_discard`, `collect_head_tree`/`collect_head_blobs`, `walk_tree`/`walk_tree_blobs`) — all local object-database reads and writes (blob/tree/commit construction, HEAD tree walks, working-tree-vs-HEAD diffing) already run entirely on `gix`'s ODB and ref APIs, with zero CLI shell-outs. `rdm-store-git/src/repo.rs:1-58` — the `GitRepo` struct owns a `gix::ThreadSafeRepository` directly (`root`/`repo` fields, lines 22-25), and `reopen()` (line 47) re-opens it via `gix::open` after every CLI mutation to refresh cached refs/config — itself a symptom of the dual design (Background #1): every CLI shell-out needs an explicit "now go re-read what git just changed behind gix's back" step.

`rdm-store-git/src/error.rs` — the `GitError` enum's `BranchesDiverged` (doc comment, lines 22-27) and `MergeConflict` (lines 28-33) variants are documented as "not currently constructed" — divergent-pull conflicts already surface through `PullOutcome::Conflict` rather than an error. A hypothetical `gix`-based merge implementation would need to define its own new failure modes (e.g. a typed error for "no merge-workflow API available" or partial-merge-state corruption) rather than simply reusing these.

`rdm-store-git/src/remote.rs:28-67` (`git_remote_list`), `:79-99` (`git_remote_add`), `:101-139` (`git_remote_remove`) — **not applicable** to this survey: these are hand-rolled `.git/config` INI parsing/writing, not `git`-CLI shell-outs at all. `gix::config` could plausibly replace the hand-rolled parser, but that is a separate, unrelated cleanup with no bearing on the clone/fetch/push/merge/worktree recommendation below — noted here only so it isn't mistaken for an overlooked CLI call site.

### `rdm-git` — read-only queries (cheap partial win, out of this phase's scope)

The phase body's premise — that `rdm-git`/`rdm-store-git` split cleanly into "gix" vs. "CLI" — is not quite accurate for these four functions in `rdm-git/src/lib.rs`: they shell out today but are all **read-only** queries that `gix` could plausibly serve natively, independent of the no-go conclusion on network/merge/worktree operations above:

| File:line | Call | Function |
|---|---|---|
| `rdm-git/src/lib.rs:178` | `git log --format=%H%n%B%n<END> HEAD --not <anchor>` | `commit_messages_since_at` |
| `rdm-git/src/lib.rs:208` | `git symbolic-ref --quiet HEAD` | `current_branch_at` |
| `rdm-git/src/lib.rs:229` | `git merge-base --is-ancestor <sha> HEAD` | `is_ancestor_of_head_at` |
| `rdm-git/src/lib.rs:263` | `git merge-base --is-ancestor <sha> <branch>` | `is_ancestor_of_branch_at` |

(`rdm-git/src/lib.rs:296` `is_ancestor_at` — comparing two arbitrary commits directly — shells out via `git merge-base --is-ancestor` the same way and belongs in this same read-only bucket, though it postdates the phase body's own list.)

Already-`gix` for comparison: `rdm-git/src/lib.rs:110` (`discover_git_dir`), `:124` (`discover_hooks_dir`), `:148` (`head_commit_info_at`) all call `gix::discover`/`repo.head()` directly.

This is flagged as an independent, low-risk future migration candidate — **explicitly out of scope to implement in this phase.** It does not change the headline no-go recommendation, since it touches none of push/merge/worktree-admin.

### `rdm-git` — bonus finding: `git worktree` admin (project-repo lifecycle)

`rdm-git/src/worktree.rs` manages linked worktrees for the *project* repo (distinct from the plan-repo `GitStore`), and depends on the CLI at every mutation point:

| File:line | Call | Function |
|---|---|---|
| `rdm-git/src/worktree.rs:449` | `git rev-parse --show-toplevel` | `discover_project_repo` |
| `rdm-git/src/worktree.rs:463` | `git rev-parse --is-bare-repository` | `discover_project_repo` (bare-repo fallback) |
| `rdm-git/src/worktree.rs:465` | `git rev-parse --absolute-git-dir` | `discover_project_repo` (bare-repo fallback) |
| `rdm-git/src/worktree.rs:482` | `git worktree list --porcelain` | `main_worktree` |
| `rdm-git/src/worktree.rs:578` | `git worktree add <path> <branch>` (reuse existing branch) | `add` |
| `rdm-git/src/worktree.rs:583` | `git worktree add -b <branch> <path> <base>` | `add` (new branch) |
| `rdm-git/src/worktree.rs:615` | `git worktree list --porcelain` | `list` |
| `rdm-git/src/worktree.rs:673` | `git rev-parse --show-toplevel` | `current` |
| `rdm-git/src/worktree.rs:784` | `git worktree remove [--force]` | `remove_info` |
| `rdm-git/src/worktree.rs:798` | `git branch -d\|-D <branch>` | `remove_info` (branch cleanup) |
| `rdm-git/src/worktree.rs:859` | `git status --porcelain` | `is_dirty` |

This is a git-CLI dependency of exactly the same shape as clone/fetch/push/merge, and `gix` lists worktree create/move/remove/repair as unimplemented (see the capability table above) — this **strengthens** the no-go case. This document does not expand scope into a worktree-specific recommendation; it's noted purely as corroborating evidence that the CLI dependency runs wider than just `rdm-store-git`'s remote/merge porcelain.

### Test-only CLI usage — not in scope, not counted

`rdm-store-git/src/lib.rs:646-647` (`git_cmd` test helper), `:1548-1556` (`make_bare_plan_repo`), `:1603-1607`, and numerous `.args(["clone" | "push" | ...])` calls in the roughly `660`-`1400` range are all inside `#[cfg(test)]` modules. Likewise `rdm-git/src/lib.rs:317-460` and `rdm-git/src/worktree.rs:1000-1764` are test fixtures. None of these are production call sites and none are counted in the inventory above.

## Recommendation & Revisit Trigger

**No-go on full CLI-to-`gix` consolidation**, scoped precisely to: `push`, merge workflow orchestration (both the diverged-merge and conflict-resolution/abort paths), and `git worktree` admin (create/remove/branch-cleanup). Existing local-object-write `gix` usage (`build_tree_from_dir`, `create_git_commit`, `git_status`, `git_discard`, the tree-walk helpers) is unaffected and should stay exactly as-is — none of this spike's findings bear on it.

Rationale, tied directly to the inventory above:
- `git_push` has no `gix` equivalent at all (`push` unimplemented).
- `git_pull`'s diverged-branch path, `git_merge_abort`, and `git_resolve_conflict`'s merge-completion step all depend on merge-workflow orchestration and `MERGE_HEAD`-compatible state persistence, both unimplemented.
- `worktree::add`/`remove_info` in `rdm-git` depend on `git worktree`, unimplemented.
- `clone`/`fetch` (`clone_remote`, `git_fetch`) are the one genuinely mature area (`crate-status.md`: `[x] clone, fetch, ls-refs and shallow variants`; `gix::prepare_clone`/`PrepareFetch::fetch_then_checkout` verified to exist with the expected signature) — but migrating only those two is explicitly **not recommended**. `git` would still be required on `PATH` for push/merge/worktree, so motivation #2 (implicit runtime dependency) stays unsolved, while motivation #1 (dual mental model) gets worse: contributors would now need to remember that clone/fetch is `gix` but push/merge/worktree is CLI, a finer-grained split than today's coarser "reads are `gix`, mutations that touch a remote or another working tree are CLI" line.

**Revisit trigger:** re-run this survey when `gitoxide`'s `crate-status.md` marks **both** `push` and `merge workflow orchestration` as done (`[x]`) — `git worktree` create/remove is not on the critical path for `rdm-store-git`'s remote/merge porcelain and can lag, though it would need to land separately before `rdm-git/src/worktree.rs` could also drop its CLI dependency. Alternatively, revisit if a maintained third-party crate providing push/merge on top of `gix`'s primitives reaches a stable (1.0+) release. Independently of `gix`'s progress, also revisit — per the originating phase body's own condition — **if the `git`-CLI runtime dependency becomes an actual reported problem** (e.g. a user environment without `git` on `PATH`), which would change the cost/benefit calculus even for a partial migration.

## Sources

- [gitoxide `crate-status.md`](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md) — re-fetched 2026-07-16; clone/fetch/ls-refs/shallow `[x]` done, push `[ ] push and self-contained clone/fetch over file:// and ssh://`, three-way blob content-merge `[x]` done, merge workflow orchestration `[ ]`, MERGE_HEAD-compatible state persistence `[ ]`, worktree create/move/remove/repair `[ ]` (all four share one bullet).
- [docs.rs `gix` 0.85.0](https://docs.rs/gix/0.85.0/gix/) — feature-flag documentation for `blocking-network-client`, `blocking-http-transport-curl`, `blocking-http-transport-reqwest`, `async-network-client`, `worktree-mutation`; checked 2026-07-16.
- [docs.rs `gix::clone::PrepareFetch`](https://docs.rs/gix/0.85.0/gix/clone/struct.PrepareFetch.html) — `fetch_then_checkout` signature, return type (`Result<(PrepareCheckout, Outcome), Error>`), and its `blocking-network-client` + `worktree-mutation` feature requirement; checked 2026-07-16.
- `Cargo.lock` (this repo) — resolves `gix 0.85.0`; `rdm-git/Cargo.toml:11`, `rdm-store-git/Cargo.toml:13` pin the same version with `default-features = false`.
- Commit `b9f5d4b` ("build(deps): bump gix from 0.84.0 to 0.85.0 (#32)") — the version-bump commit for the currently pinned `gix` release.
- `rdm-git/src/process.rs:48` (`git_command`) — the shared non-interactive git subprocess spawner and its module doc comment.
