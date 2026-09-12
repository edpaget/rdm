# Index removal decision

**Roadmap**: `retire-generated-index`, phase 1
**Date**: 2026-09-11
**Decision**: Option A — delete `rdm index` and generated `INDEX.md` entirely.

## Executive summary

The roadmap set out to implement **Option C** — take `INDEX.md` off the write
path but keep `rdm index` around as explicit, opt-in porcelain that a human
without the rdm binary could still read. Phase 1's job was to settle the one
question separating C from A: does anyone actually browse the plan repo
without an rdm binary? The operator answered directly: **no**. Per the
roadmap's own framing, C "can still collapse to A later by deleting one
command" — the browse-without-rdm question was the only thing keeping that
command around, and it has now resolved against it.

**This document is that collapse.** The decision is Option A: delete
`rdm index`, `rdm-core/src/ops/index.rs`, `rdm-core/src/display/index.rs`, and
the generated `INDEX.md` / `projects/<p>/INDEX.md` files, in full. This
supersedes the roadmap's originally-chosen Option C.

## Problem statement

`INDEX.md` (root) and `projects/<p>/INDEX.md` are a generated, git-committed
convenience view of roadmaps, phases, tasks, and reviews. They are regenerated
inside `ops::mutate` / `ops::mutate_batch` on **every** mutation, and nothing
in rdm ever reads them back — the only reader is the `rdm-index` merge driver,
which reads its own regenerated output. Every live surface (`rdm list`,
`tree`, `roadmap list`, `task list`, `next`, `backlog report`, the REST/web UI)
already computes the same aggregates independently.

That per-mutation regeneration is the plan repo's central write-contention
object, and it is the root of an entire class of machinery this project has to
maintain purely to defend two files nobody reads:

- `rdm_core::paths::is_derived_path` and the session journal's derived-path
  class
- `ChangesetScope::derived`
- `GitRepo::reconcile_derived` and `drop_orphaned_project_subtrees`
  (`rdm-store-git/src/commit.rs`, ~200 loc of HEAD-projection and
  orphan-subtree pruning)
- the `tree ⊇ index` divergence carve-out in `docs/scoping-model-decision.md`
- the derived-index exemption in the lost-update guard
  (`docs/lost-update-evaluation.md`)
- the `[merge "rdm-index"]` git merge driver, its `.gitattributes` mapping,
  backfill-on-open, and discard-survival handling
- filtering at seven `git_status()` consumers, plus the `generated` field in
  MCP `rdm_status`

All of this exists in service of a file whose only justification, once the
write-path and merge-driver costs are already gone, is human browsability for
someone without the rdm binary.

## Options considered

These are the roadmap's own three shapes, reproduced verbatim from
`retire-generated-index`'s "Direction — option C" section — not
re-derived or redefined here.

**Option A — delete outright.** Remove `rdm index`
(`rdm-core/src/ops/index.rs`, 182 loc; `rdm-core/src/display/index.rs`, 760
loc; `rdm-cli/src/commands/index.rs`), the generated `INDEX.md` files, and
every piece of machinery that exists only to defend them. Cost: loses the
one-command human-browsable snapshot; anyone wanting an equivalent view runs
`rdm list --format markdown` instead. Benefit: the derived-path class, the
merge driver, and the per-mutation rescan all disappear completely, and there
is no leftover surface for a later session to wonder whether it's still load-
bearing.

**Option B — un-version it.** Keep generating `INDEX.md` on every write, but
stop git-tracking the file. Rejected by the roadmap's own text: this "keeps
the per-mutation rescan and... keeps every session writing the same two
files," so the derived-path class, the write contention, and the read
amplification all survive untouched. The Goal ("shrink the concurrency
model") goes unmet under B even though the git-conflict symptom disappears.

**Option C — take it off the write path, keep `rdm index` as opt-in
porcelain.** Delete `reconcile_derived`, the merge driver, and the
derived-path class, but keep an explicit `rdm index` command a user can run
by hand to produce a browsable snapshot on demand. This is the shape the
roadmap actually implements across phases 2–5, and it was originally chosen
over A because, in the roadmap's own words, "A was rejected as premature:
phase 1 settles whether anyone browses the plan repo without an rdm binary,
and C can still collapse to A later by deleting one command."

## The browse-without-rdm finding

**Does anyone browse the plan repo without an rdm binary? No — answered
directly by the operator on 2026-09-11, not inferred.**

This is the fact that separates finished-C from A. Once phases 2–4 have
already deleted the per-mutation rescan, the merge driver, and the
derived-path class, Option C's *remaining* justification — the only thing
distinguishing it from A — is preserving a human-browsable file for someone
without the rdm binary, produced by the opt-in `rdm index` command. With that
use case ruled out by the operator, nothing distinguishes finished-C from A
except one leftover command that nobody needs and nobody asked for.

## Decision

**Option A is chosen.** Per the roadmap's own framing, C can collapse to A by
deleting the one remaining opt-in command; the browse-without-rdm question —
the only thing keeping that command around — has now resolved against it.

Option C was the safe default while the browse question was open. The
question has resolved against it. The artifact — `rdm index`,
`INDEX.md`, and every mechanism that exists to defend it — is removed
because it no longer serves a purpose, not kept out of sentiment for having
been part of the original design.

## Consequences

1. **Phase 5 takes the delete branch, not the porcelain-reduction branch.**
   Its original framing ("reduce `rdm index` to opt-in porcelain and sweep the
   docs") is superseded by a straight delete: drop `rdm index`,
   `rdm-core/src/ops/index.rs`, and `rdm-core/src/display/index.rs` (~940
   loc total), then repoint the docs at `rdm list --format markdown`. Its
   original "or gains a `--project` scope" question is moot and needs no
   answer — there is no command left to scope.
2. **Phase 4's scope is unaffected by A vs. C.** The derived-path-class
   deletion in `rdm-store-git/src/commit.rs` goes either way — both A and
   finished-C remove `reconcile_derived` and the derived-path machinery. What
   changes is only what phase 5 does afterward: once `rdm index` itself is
   also gone, nothing in the codebase produces `INDEX.md` at all, under any
   command, opt-in or otherwise.

   *Landed.* Phase 4 (`phase-4-collapse-derived-path-class`) carried out
   exactly the deletion this list enumerates: `is_derived_path`, `index_path`,
   `project_index_path`, `is_project_manifest`, `ChangesetScope::derived`,
   `reconcile_derived`, `drop_orphaned_project_subtrees`, the
   `StatusReport::derived` bucket and the `verify_baselines` exemption are all
   gone. The `tree ⊇ index` divergence carve-out ceased to exist with them.

## Existing plan repos

Every plan repo in existence today has both `INDEX.md` and
`projects/<p>/INDEX.md` tracked, and after this roadmap lands nothing updates
them. Leaving them untouched means they go stale in git; that is worse than
either removing them cleanly or documenting them as orphaned. This section
answers what happens to them — this is settled input for phase 6, matching
phase 6's own already-authored body (`phase-6-existing-repo-migration`), not
an independently-derived answer.

**The answer: existing tracked index files are left in place by default,
never deleted automatically, on-open, or on detection.** Removal happens only
through an explicit, user-invoked command (phase 6: `rdm index --prune` or a
dedicated verb — the exact surface is phase 6's to choose) that deletes both
`INDEX.md` and every `projects/<p>/INDEX.md`, and stages those deletions into
the caller's changeset so removal lands through the normal `rdm commit` path
rather than a bare `rm` the session/discard model cannot see.

Two alternatives were considered and rejected:

- **Automatic deletion on detection** (e.g. the first `rdm` invocation against
  an existing repo silently removes the files). Rejected: silently deleting
  tracked files a user may have linked to from elsewhere is not a migration,
  it is data loss with no recovery path through rdm's own tooling. It also
  collides directly with the changeset/discard model —
  `plan-repo-concurrency/phase-2` had to design around exactly this class of
  hazard (an unprompted mutation a concurrent session's `rdm discard --force`
  cannot see coming), and reintroducing it here would undo that work.
- **Leave them and say nothing** (no prune command, no documentation, no
  signal). Rejected: stale generated content silently masquerading as current
  data is worse than either removing it via an explicit path or clearly
  documenting it as orphaned — a user or a future agent reading a
  months-stale `INDEX.md` has no way to know it stopped being maintained.

What remains genuinely open, and is correctly phase 6's decision rather than
this document's: the exact command surface (`rdm index --prune` vs. a
dedicated verb), and any detection/messaging niceties such as what
`rdm status` / `rdm commit` output looks like for a staged prune. The
automatic-vs-explicit axis itself is **not** open — it is decided above, and
matches phase 6's own body.

## Supersedes / annotates

This decision corrects or annotates the following existing documents in
place, each now pointing back here:

- `docs/principles.md` (line ~94) — the "regenerated on every write... this
  eliminates merge conflicts" causal claim is corrected: the elimination
  mechanism becomes "INDEX.md is out of the write path", not
  "regeneration on every write". *Corrected in this phase.*
- `docs/principles.md` (line ~99) — "the filesystem *is* the database...
  INDEX.md is a convenience view, not a source of truth" remains true and is
  the argument *for* this removal; annotated with a forward pointer, not
  altered. *Annotated in this phase.*
- `docs/scoping-model-decision.md` § "INDEX.md Consistency in Partial
  Commits" — marked historical in place (not deleted): it remains the binding
  record of why `plan-repo-concurrency`'s phases 5 and 8 (a separate,
  already-completed roadmap, not a phase of this one) shaped
  `reconcile_derived` and the orphaned-subtree projection the way they did.
  *Annotated in this phase.*
- `docs/lost-update-evaluation.md` (§ "Carve-outs", "Derived indexes are
  exempt", ~line 257) — the exemption becomes historical once INDEX.md
  generation is removed; the existing carve-out text is preserved as an
  accurate record of why it existed. *Annotated in this phase.*

A broader repo grep for the same "regenerated on every write / eliminates
merge conflicts" framing also turned up two additional locations that are
**not** edited by this phase, because they are the actual delete's job
(phase 5), not a design-doc correction:

- `docs/file-formats.md:219` — describes the `rdm-index` merge driver
  end-to-end; phase 5 removes this section entirely along with the driver.
- `docs/architecture.md:49` — states "`INDEX.md` is a derived view... and
  regenerated on every write operation"; phase 5 removes or rewrites this
  once the generator no longer exists.

Recording them here so the broader sweep is not silently dropped: phase 5's
docs sweep must cover both, in addition to whatever else its own audit turns
up.
