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
  (`project.source.default_branch` → the plan repo's `rdm.toml`
  `default_branch` → `main`). **No merge-base is an error naming `--base`**,
  never a silent fallback to the root commit or to `head` itself — either
  would produce an empty diff and make every quote out-of-hunk.
- **`base` is provenance, not identity.** The reference grammar is
  `change/<head>` only, and `ReviewTarget::same_item` compares `head` alone,
  so `rdm review list --on change/<sha>` matches a stored target that also
  carries a base. Derived `PartialEq` would not.
- The review additionally records `change_branch`: the source-repo branch the
  checkout was on, used only to pick the tip drift is measured against.

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

`rdm review show` resolves each anchor against a **tip** — the review's
stamped `change_branch` when it still resolves, otherwise the repository's
HEAD:

| state | meaning |
|---|---|
| `resolved` | the path exists at the tip and every occurrence the quote had at `head` survives byte-for-byte |
| `drifted` | the path exists at the tip but holds fewer occurrences of the quoted text than `head` did |
| `unresolved` | the path is gone at the tip (or the comment has no anchor) |

The test is *occurrence count*, not position: code that merely moved within
the file keeps its count and still reads as resolved, because the reviewer's
words are still true of it. Counting rather than a plain substring search is
what keeps a duplicated quote honest — when a file holds the same text twice
and the author edits exactly the occurrence the reviewer anchored to, the
surviving other copy would satisfy a `contains` check and the comment would
read "still true" although the line it named is gone.

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
`source.repo` unset or gone, a build without the `git` feature),
`rdm review show` still prints the review: every change comment comes back
`unresolved` and a `source_verification_skipped` note says why — the
precedent `rdm link check`'s `path_verification_skipped` established.
Permalinks still render, because they need no checkout.

`rdm review comment --path` fails loudly in the same situation. An anchor
that was never checked against real content would silently mislead every
later reader.

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
| source-repo discovery from the cwd | `rdm-cli/src/source_repo.rs` |
| CLI wiring: discovery + core calls + formatting | `rdm-cli/src/commands/review.rs` |

Every failure mode above is a matchable `rdm_core::Error` variant rather than
a formatted string, so the server layer maps them onto its own status codes
without re-deriving the rule (`rdm-server/src/problem.rs`).

Integration coverage: `rdm-cli/tests/cli_review_change.rs` (real temp git
source repo + real temp plan repo) and § 7 of
`scripts/verify-workflow-review-outcome.sh` (the workflow lane's persist path
executed verbatim against the real binary).
