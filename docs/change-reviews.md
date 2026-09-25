# Change reviews (`change/<sha>`)

A design record for rdm's fourth review target: a review of the **code**, not
of a plan-repo document.

A `change/<head-sha>` review lives in the plan repo like every other review,
but its comment anchors point into the project's **source** repository. That
is the whole point — the artifact history then covers the change itself, and
drift detection works on code the way it already works on documents.

## The target

```
rdm review start --on change/<sha>   --project <p>   # an explicit revision
rdm review start --on change/HEAD    --project <p>   # from inside a checkout
```

`ReviewTarget::Change { head, base }`:

- **`head`** is always a full 40-character commit SHA. Whatever the operator
  types — `HEAD`, a branch name, an abbreviated sha — is rev-parsed against
  the source repository *before* anything is stored. Storing the abbreviation
  would make two reviews of the same commit look like different targets.
- **`base`** is the other end of the reviewed range: `--base <rev>` when
  given, otherwise the merge-base of `head` with the project's default branch
  (`project.source.default_branch` → `default_branch` resolved for the
  project: `RDM_DEFAULT_BRANCH` → `[projects.<p>]` → `rdm.toml` → global
  → `main`). **No merge-base is an error naming `--base`**,
  never a silent fallback to the root commit or to `head` itself — either
  would produce an empty diff and make every quote out-of-hunk.
- **`base` is provenance, not identity.** The reference grammar is
  `change/<head>` only, and `ReviewTarget::same_item` compares `head` alone,
  so `rdm review list --on change/<sha>` matches a stored target that also
  carries a base. Derived `PartialEq` would not.
- The review additionally records `change_branch`: the source-repo branch the
  checkout was on, used only to pick the tip drift is measured against.

This `base` default (merge-base with the default branch, absent an explicit
`--base`) is specific to the `change/<sha>` target. A **`phase/<roadmap>/<stem>`
or `task/<slug>`** target — resolved through `rdm review source --on …`, and
through the same code path the gated `reviewed` write's `--source` binding
uses — has a different default: it prefers the item's recorded `started_head`
(the commit the roadmap or task worktree was at when the item's own work
began, recorded explicitly by `phase update`/`task update --start-commit
<sha>` and exposed by `phase show`/`task show --format json`) over the
merge-base, so a phase implemented in a shared roadmap worktree is reviewed as
its own diff rather than as every earlier phase's changes too. `--base` still
overrides, and with no `started_head` recorded — because nothing has ever
recorded one for this item — it falls back to the merge-base exactly as
`change/<sha>` does, reporting that fallback in the response's `baseNote`
field.

A recorded `started_head` is rev-parsed/verified exactly like an explicit
`--base`, so an unreachable value (e.g. garbage-collected) still fails
closed. But a `started_head` that *does* resolve is additionally checked with
`git merge-base --is-ancestor` against `head`: if the roadmap or task branch
was later rebased onto an advanced default branch, or an earlier commit
amended or rebased, the recorded SHA can still exist as an object while no
longer being an ancestor of the current checkout's HEAD. Reviewing that range
directly (`git diff started_head head`) would silently mix in every change
the default branch picked up, plus reverse diffs of the rewritten commits.
Rather than fail closed there, `review source` falls back to the merge-base
with the default branch — a strict superset of the intended range, safe if
noisier — and names the stale `started_head` and the reason in `baseNote`. An
explicit `--base` is exempt from this check: it was named deliberately, and
is the escape hatch an operator has after history changes.

**Who writes `started_head`, and when.** No status transition records it as a
side effect. `phase update`/`task update --start-commit <sha>` is an
explicit, write-once command, independent of `--status` — it may be passed
alone or combined with any status transition in the same call. A second
`--start-commit` against an item that already has a recorded value is
refused, naming the existing value; the original is left untouched. In
practice, the `rdm-dispatch-phase` skill is the one that records it: from the
head it has already pinned for the item, immediately before that item's first
implementer dispatch, skipping when a value is already recorded (a resumed
dispatch, or a phase predating this field). A malformed value (not a full
40-lowercase-hex-character commit SHA) or one that doesn't resolve to a
commit in the item's worktree is refused before any write, naming the
rejected value and — for the existence check — the repository it was checked
against.

`change` is deliberately **not** a link kind. `rdm:change/<sha>` is rejected by
`rdm_core::link::parse` with a message pointing at `rdm:src/<path>@<sha>` —
there is no plan-repo document at the other end of a change reference, so
linking one would be a dangling link by construction. This is the single
divergence between `ItemRef`'s link grammar and `ReviewTarget`'s reference
grammar.

## Anchoring

```
rdm review comment <id> --path src/lib.rs --quote "<exact text>" --body "…"
```

The quote is located in that file's content **at the reviewed `head`**, read
from the object database (`git show <rev>:<path>`) rather than from the
working tree — so a dirty checkout, `core.autocrlf`, or a different trailing
newline can never change what an anchor resolves against.

The quote must **overlap at least one hunk** the change touches in that file.
Overlap, not containment: a finding about an edited line *and* its unchanged
neighbour is legitimate and anchors. A quote outside every touched hunk is an
error naming the nearest one; a path the change does not touch at all gets its
own distinct message ("`<path>` is not touched by `<base>..<head>`"), because
a "nearest hunk" that does not exist reads nonsensically.

Duplicate occurrences reuse the existing `--occurrence <n>` vocabulary, not a
second one.

A `--path` containing `@` or `#` is **refused**. Those are the two characters
the `rdm:src/<path>@<rev>#L<n>` permalink grammar reserves for itself, and
nothing escapes them, so an anchor stored on such a path would render a
permalink that parses back as a shorter path plus a nonsense revision. The
refusal names the offending character. The rule is deliberately minimal:
characters the grammar does not reserve (`?`, `%`, spaces, multi-byte names)
are still accepted. The guarantee it buys is forward-looking — every path the
validator accepts round-trips through `Link::Code` → `to_string()` →
`link::parse` — and it does not retract anchors stored before the check
existed, which still render rather than failing.

The stored anchor is:

```yaml
anchor_type: file-quote
path: src/lib.rs
quote: "fn two_renamed() {}"
occurrence: 1
start_line: 2
end_line: 2
```

**Deliberately no `prefix`/`suffix`.** A review file lives in the plan repo,
and "nothing beyond the quote the reviewer chose is embedded" is only
*literally* true if no surrounding context is copied in. Disambiguation
therefore uses `occurrence` plus the recorded line range instead. The line
range also makes the permalink derivable with no checkout present.

## Resolution: resolved / drifted / unresolved

`rdm review show` resolves each anchor against a **tip**, chosen by
`rdm_core::change::resolve_drift_tip` — the single implementation of that
policy, which every consumer delegates to rather than re-deriving:

| `TipOrigin` | when | tip |
|---|---|---|
| `StampedBranch` | the review's stamped `change_branch` still resolves | that branch's **current** sha |
| `BranchGone` | a branch was stamped but no longer resolves (renamed, deleted, never pushed) | the repository's `HEAD` |
| `NoBranchStamp` | the review was started on a detached HEAD, so nothing was stamped | the repository's `HEAD` |

`StampedBranch` deliberately takes the branch's *current* sha rather than
the review's recorded `head`: drift is measured against where the branch is
now, so a force-moved branch re-points the tip. Two failure modes are
distinct, and neither falls back silently:

- Nothing resolvable at all — no branch, and an unborn `HEAD` — is
  `Error::ChangeTipUnresolvable`.
- A **source-repository failure** (git missing, or the adapter's
  option-shaped-operand guard `Error::InvalidChangeRevisionInput` firing
  before any subprocess) is *propagated*. A source that cannot answer must
  not silently change what drift is measured against; the caller degrades
  explicitly (see "The degrade rule") instead of quietly measuring against
  a different revision.

The resolution states:

| state | meaning |
|---|---|
| `resolved` | the path exists at the tip, it holds at least as many occurrences of the quote as `head` did, **and** one of them still sits between the same neighbouring lines the anchored occurrence sat between at `head` |
| `drifted` | the path exists at the tip but one of those two conditions fails |
| `unresolved` | the path is gone at the tip (or the comment has no anchor) |

A path lookup at the review's `head` has **three** possible outcomes, not two,
and they are reported apart:

| lookup | reported as |
|---|---|
| the reviewed commit is not in this checkout | a **source note** (`source_verification_skipped`); no per-comment reason |
| the commit is here but the path is not | a per-comment `unresolved_reason`: "`<path>` no longer exists at `<head>`" |
| the path names a directory or a submodule | a per-comment `unresolved_reason` naming which |

The first row is the third cause of a skip, alongside "no checkout reachable"
and "no resolvable HEAD". It exists because reporting an unreachable commit as
a missing *path* was a confidently false statement: the file is present in
every checkout that has the commit, and the remedy is environmental (fetch the
branch, or point `source.repo` at the right checkout) rather than anything
about the path. `rdm_core::change::change_anchor_ineligibility` therefore maps
that case to `None`, widening the same carve-out it already made for a
source-repository failure.

The test is *occurrence identity* — neither a plain substring search nor a
bare count. It is a two-clause disjunction, evaluated in this order:

1. **Count.** When a file holds the same text twice and the author edits
   exactly the occurrence the reviewer anchored to, the surviving other copy
   would satisfy a `contains` check and the comment would read "still true"
   although the line it named is gone. Requiring the tip to retain at least
   as many occurrences as `head` had reports that as drift.
2. **Line window.** A count alone is still fooled by a single commit that
   edits the anchored occurrence away *and* adds a fresh copy of the same
   text elsewhere: the count is unchanged, yet the anchor is genuinely
   drifted. So the tip must also hold an occurrence whose immediately
   neighbouring lines are the ones the anchored occurrence sat between at
   `head`.

The window is neighbour-based rather than positional, so code that merely
*moved with its neighbours* still reads as resolved — the reviewer's words
are still true of it. Neighbours that do not exist head-side (the quote is at
the start or end of the file) are not compared, and a quote whose
line-extended span is the whole file degenerates to the count clause alone
rather than reporting permanent drift. The anchored occurrence's own line is
not compared byte for byte either: it already carries the quote, so an edit
*beside* the quote on the same line — a trailing comment, say — is not drift.

Because clause 1 is the first term and is unchanged, clause 2 can only ever
*add* drift detections: no anchor that a pure count reports as drifted can
start reporting resolved.

The reported byte range always indexes the **head-side** content, matching
`Resolution::Original`'s "the body the reviewer saw" contract.

## Permalinks

Each anchored comment carries a head-pinned permalink:

```
rdm:src/<path>@<head>#L<start>[-L<end>]
```

emitted as `source_link` in `rdm review show --format json` and under each
comment in the human/markdown renderers. It is derived from the **anchor
alone**, so it renders with no source checkout present, and it is exactly the
grammar `rdm link resolve` accepts. A single-line span emits `#L7`, not
`#L7-L7`.

## The degrade rule

The **read** path degrades, the **write** path does not.

With no source repository reachable (running from an unrelated cwd,
`source.repo` unset or gone, a checkout that does not contain the reviewed
commit, a build without the `git` feature), `rdm review show` still prints the
review: every change comment comes back `unresolved` and a
`source_verification_skipped` note says why — the precedent `rdm link check`'s
`path_verification_skipped` established. Permalinks still render, because they
need no checkout.

**A skip never sets a per-comment `unresolved_reason`.** The two channels answer
different questions: the source note says verification could not run, while
`unresolved_reason` says *this* anchor is itself invalid (its path is a
directory, a submodule, or absent at a head the checkout does have). Putting an
environmental cause in the per-comment channel would dress a skip up as a
verdict about the reviewer's quote, so the one is never used for the other, and
a comment is never `unresolved` with **both** channels empty.

The rule holds on **every** read surface, not just the CLI. `GET
/projects/<p>/reviews/<id>` and the HTML review section both resolve through the
same `rdm_core::change::resolve_change_review` pass, against the project's
configured local `source.repo`; when that does not resolve they set
`source_verification_skipped` to a note naming the server and the cause. They
previously reported every change-review comment `unresolved` with both channels
null — indistinguishable from anchors that genuinely no longer resolve, and
contradicting what `rdm review show --format json` said about the same file.

`rdm review comment --path` fails loudly in the same situations. An anchor
that was never checked against real content would silently mislead every
later reader — and a checkout lacking the reviewed commit is reported as
exactly that (`Error::ChangeHeadNotInSource`), never as a missing path.

## `implements`: linking the change to its plan

A change review records `implements: rdm:plan/<slug>`. Explicitly:

```
rdm review start --on change/HEAD --implements rdm:plan/<slug>
```

or inferred, when omitted, from the worktree the command runs in: the item
that checkout maps to must have **exactly one** `approved` plan. Zero and
many are both actionable errors naming `--implements` (many lists every
candidate). A roadmap-level worktree has no single item to look plans up for,
so it errors rather than guessing across the roadmap's phases.

Inference is restricted to `approved` plans; an **explicit** `--implements` is
accepted whatever the named plan's status, because the operator asked for it.
The two rules are different on purpose.

Surfaced two ways:

- `rdm plan show --format json` gains `change_reviews[]`, kept **distinct**
  from the existing `reviews[]` (reviews *on* the plan document). Merging them
  would make the plan's own verdict summary meaningless.
- `rdm backlinks phase/<roadmap>/<stem>` reaches the change reviews through
  the plan, as structural `implements` backlinks tagged with `via`. Depth is
  fixed at **one hop** and stays that way: following a second level would make
  output depend on chain length and could revisit a document. Duplicates
  (two plans implementing one phase, a superseding plan) are emitted once.

## The never-write-the-source-repo invariant

Every source-repo interaction routes through `rdm_core::source::SourceRepo`, a
port with **reads only** — `rev_parse`, `merge_base`, `file_at`,
`unified_diff`, `head`, `current_branch`. There is no write method, and there
never should be: the invariant is enforced by the port's shape, not by
discipline at each call site.

`rdm-git`'s `GitSourceRepo` is the production implementation and spawns only
`rev-parse`, `merge-base`, `show` and `diff`; that allow-list is asserted
directly over the argv each method builds. `rdm-core`'s own
`MemorySourceRepo` is the in-memory double the pure anchoring logic in
`rdm_core::change` is unit-tested against, so every correctness-critical rule
is testable with no process spawned.

## Source discovery

Two commands ask the same question — "is the checkout I am standing in the
project's configured source repository?" — and they used to answer it with
two hand-written ladders in `rdm-cli`. An audit confirmed the two genuinely
**disagree**, in exactly three of the eight environments below (E2, E4, E6a),
and that the disagreement is intentional rather than drift:

- `rdm review --on change/…` pins content. It must name a source or refuse,
  so it may leave the cwd for a configured local directory.
- `rdm link check` is a lint that fails open. It never verifies against a
  repository the operator is not standing in; every other environment is a
  documented skip, reported as `path_verification_skipped`.

So `rdm_core::source_select::select_source` owns the **classification** of
the environment, while its `SourceFallback` parameter
(`ConfiguredLocal` / `CwdOnly`) carries exactly that difference in
**disposition**. Collapsing the two into one ladder — the fix the original
allegation implied — would erase link check's fail-open skip, so it was
explicitly refused.

| Environment | `ConfiguredLocal` (change review) | `CwdOnly` (link check) |
|---|---|---|
| E1 matching checkout | `ReadCwdCheckout` | `ReadCwdCheckout` |
| E2 mismatching checkout + configured LOCAL source | `ReadConfiguredLocal` | `Unavailable(CheckoutIsNotConfiguredSource)` |
| E3 mismatching checkout + configured REMOTE source | `Unavailable(CheckoutIsNotConfiguredSource)` | `Unavailable(CheckoutIsNotConfiguredSource)` |
| E4 no checkout + configured LOCAL source | `ReadConfiguredLocal` | `Unavailable(NotInCheckout)` |
| E5 no checkout + configured REMOTE source | `Unavailable(ConfiguredSourceNotLocal)` | `Unavailable(NotInCheckout)` |
| E6a in a checkout that is NOT the plan repo, NO source configured | `ReadCwdCheckout` | `Unavailable(NoSourceConfigured)` |
| E6b in the PLAN REPO itself, NO source configured | `Unavailable(CheckoutIsPlanRepo)` | `Unavailable(NoSourceConfigured)` |
| E7 no checkout, NO source configured | `Unavailable(NoCheckoutAndNoSource)` | `Unavailable(NoCheckoutAndNoSource)` |

E6a, E6b and E7 are distinct environments with distinct causes and are never
collapsed into one row, even where two causes happen to share a message.
The same eight row labels appear in `select_source`'s rustdoc and in its
table-driven unit test, which pins the row count at sixteen cells and
asserts that E2, E4 and E6a *differ* between the two consumers — so a later
attempt to unify the ladders fails loudly instead of silently erasing the
skip.

**E6b** is the one environment where change review refuses a checkout it is
standing in. Standing in the plan repo with no `source.repo` configured used to
read the *plan repo* as the source: `change/HEAD` pinned the plan repo's own
HEAD, the merge base of that HEAD against the plan repo's own default branch was
that same HEAD, and rdm silently recorded a review of plan data as though it
were the project's code. The refusal names both ways out:

```
the current directory is the plan repo, not a source checkout — run this from
the project's source checkout, or set `source.repo` in its project.md
```

The guard is deliberately scoped to `configured: None`. An operator who
explicitly points a project's `source.repo` *at* the plan repo has named that
choice, and E1/E2 keep working unchanged. Identity is compared through
`discover_project_repo` on **both** sides, canonicalized — never by matching the
raw `RDM_ROOT` string — so a plan root nested inside its git repository, a
symlinked root, and a plan repo opened through a linked worktree all compare
correctly.

A related refusal lives one step later, in `resolve_change_target`: a **derived**
base that resolves to the same commit as the head is an empty reviewed range,
refused as `Error::ChangeEmptyReviewedRange`. Only a derived base — an explicit
`--base <head>` is a supported shape (an intentionally code-free review, and the
root commit of an orphan branch that has no merge base at all), so the guard
never fires on one.

### Identity root vs. read root

The classifier is handed only booleans, never a path, and that is
load-bearing. `rdm_git::worktree::discover_project_repo` deliberately
answers with the repository's **main working tree**, which is the right
answer for *identity* (is this checkout the project's source?) and the wrong
one to *read* through: for a linked worktree, reading it would make
`change/HEAD` pin main's HEAD and stamp `main` as the change branch instead
of the worktree's own. The adapter therefore keeps two differently named
values — the **identity root** (only ever the left-hand side of
`repo_matches_source`) and the **read root** (the cwd) — and
`SourceSelection::ReadCwdCheckout` names the cwd and nothing else.
`rdm link check` reads through the cwd for the same reason; that is
behavior-preserving, because every revision it resolves is a shared ref or
SHA rather than a per-worktree HEAD.

Regressions: `cli_review_change.rs::change_review_in_a_linked_worktree_pins_that_worktrees_head_not_main`,
`::change_review_falls_back_to_the_configured_local_source_while_link_check_skips`,
and `cli_link.rs::check_from_a_linked_worktree_of_the_configured_source_still_verifies`.

## Where the code lives

Every rule below — target pinning, base derivation, anchor derivation, anchor
resolution, `--implements` resolution — is a **domain** rule and lives in
core, taking `&impl SourceRepo` so it is unit-testable against
`MemorySourceRepo` with no process spawned and no temp repository on disk.
The CLI's share is only what is genuinely environmental: deciding *which*
checkout to read, asking the cwd which worktree it is, and formatting output.

| concern | module |
|---|---|
| the read-only port + test double | `rdm-core/src/source.rs` |
| hunk parsing, anchoring, resolution, permalinks | `rdm-core/src/change.rs` |
| target pinning (`resolve_change_target`) and anchor derivation (`derive_change_anchor`) | `rdm-core/src/change.rs` |
| the default branch the merge base is taken against (`source_default_branch`) | `rdm-core/src/ops/reviews.rs` |
| `--implements` parsing and single-approved-plan inference | `rdm-core/src/ops/plan.rs` |
| the git implementation | `rdm-git/src/source.rs` |
| the drift-tip ladder (`resolve_drift_tip`) | `rdm-core/src/change.rs` |
| which repository to read, as a pure decision (`select_source`) + locator identity | `rdm-core/src/source_select.rs` |
| the environmental half of discovery: cwd, checkout lookup, `is_dir`, `origin` remote, plan-repo identity | `rdm-cli/src/source_repo.rs` |
| the whole read-path degrade ladder and its note strings (`resolve_change_review`) | `rdm-core/src/change.rs` |
| CLI wiring: discovery + core calls + formatting | `rdm-cli/src/commands/review.rs` |
| the server's discovery: **configured-local only**, structurally cwd-blind | `rdm-server/src/source_repo.rs` |
| the REST + HTML read surfaces, both routed through `resolve_change_review` | `rdm-server/src/handlers/reviews.rs`, `rdm-server/src/review_views.rs` |

Every failure mode above is a matchable `rdm_core::Error` variant rather than
a formatted string, so the server layer maps them onto its own status codes
without re-deriving the rule (`rdm-server/src/problem.rs`).

The degrade ladder lives in core rather than in `rdm-cli` for a concrete
reason: while it was ~90 lines of CLI-side policy, `rdm-server` could not reach
it, and so reported every change-review comment `unresolved` with no note.
Anything that is *policy about how to degrade* belongs beside the rules it
degrades; only checkout discovery differs between the surfaces, and each keeps
its own (the CLI reads the cwd, the server reads only a configured local
directory).

Integration coverage: `rdm-cli/tests/cli_review_change.rs` (real temp git
source repo + real temp plan repo), `rdm-server/tests/change_reviews.rs` (the
REST and HTML surfaces against a real temp source repo, including the
never-`unresolved`-with-a-null-note invariant) and `rdm-cli/tests/workflow_review/persist.rs`
(the review core's emitted persist ladder run under a real shell against the
real binary, for document targets; the change-target ladder is exercised by
`rdm-cli/tests/workflow_review/driver.rs`, e.g.
`driver::persist_ladder_records_change_review_anchors`).
