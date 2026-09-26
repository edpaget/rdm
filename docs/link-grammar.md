# `rdm:` link grammar

Document bodies (roadmaps, phases, tasks, plans, review summaries and
comments) can carry `rdm:` links, written as ordinary Markdown links:
`[the auth roadmap](rdm:roadmap/auth)`. The grammar is implemented once, by
`rdm_core::link::parse`, and every surface (`rdm link check` / `list` /
`resolve`, `rdm backlinks`, rdm-server's renderer) goes through it.

## Forms

| Form | Names | Example |
|------|-------|---------|
| `rdm:roadmap/<slug>` | a roadmap | `rdm:roadmap/auth` |
| `rdm:phase/<roadmap-slug>/<stem-or-number>` | a phase | `rdm:phase/auth/phase-1-design` |
| `rdm:task/<slug>` | a task | `rdm:task/fix-login` |
| `rdm:plan/<slug>` | an implementation plan | `rdm:plan/impl-auth-v2` |
| `rdm:src/<path>[@<rev>][#L<start>[-L<end>]]` | a file (optionally at a revision, optionally a line range) in the project's source repo | `rdm:src/rdm-core/src/link.rs@a1b2c3d#L42-L58` |
| `rdm:src@<sha>` | a whole commit in the project's source repo | `rdm:src@a1b2c3d` |

The item forms share their identifiers with `Done:` lines and
`rdm review --on`.

### The `src` ladder

The source forms make one coarse-to-fine ladder over one repository:

```
rdm:src@<sha>                        the commit
rdm:src/<path>@<sha>                 a file at that commit
rdm:src/<path>@<sha>#L<n>-L<m>       lines of that file at that commit
```

- `rdm:src@<sha>` takes a 7–64 character ASCII hex SHA (7 is git's default
  abbreviation, 64 allows SHA-256 repositories). The SHA is written back
  exactly as authored and never expanded.
- A branch or tag name is a moving target, not a commit, so `rdm:src@main` is
  refused (`LinkParseError::InvalidCommitRev`). To link a branch or tag tip,
  link a file: `rdm:src/<path>@main`.
- A line range needs a file, so any `#` fragment on the commit form is
  refused with its own error (`LinkParseError::CommitLineRange`), even when
  the fragment is a well-formed `L<n>`.
- `rdm:src` and `rdm:src/` with no path remain `LinkParseError::EmptyPath`.

### Not a link: `rdm:change/<sha>`

`change/<sha>` is a valid **review target** (`rdm review --on change/<sha>`)
but never a link. A review target is not a commit: it names the diff under
review, and the review itself is already reachable as a document. The parser
rejects `rdm:change/<sha>` (`LinkParseError::NotLinkable`) with a message
naming both alternatives: `rdm:src@<sha>` for the commit, and
`rdm:src/<path>@<sha>` for code at it.

## Resolution

`rdm link resolve rdm:src@<sha>` reports:

```json
{ "kind": "commit", "rev": "<sha>", "web_url": "<repo>/commit/<sha>" }
```

`web_url` is present only when the project has a `source` configured. Unlike
`rdm:src/<path>`, a commit link never falls back to the containing phase or
task's stamped `commit`: the SHA is part of the link. Resolution reads no
repository.

## `rdm link check`

`link check` resolves every link and exits nonzero on anything broken: a
dangling item link, a parse error (including a malformed commit link), or a
code link whose path does not exist at its pinned revision.

Commit links are verified only when the command runs inside the project's
configured source checkout (the same condition as code-link path
verification; elsewhere the `path_verification_skipped` note covers both).
A commit link whose SHA does not resolve there is **reported but not
failing**:

- JSON: an entry under `unresolved_commits` (document, byte range, `uri`,
  `rev`); the array is always present. `commit_links_checked` counts every
  commit link found.
- Text: `unresolved commit link in <doc>: <uri> — commit <sha> not found in
  this checkout (reported, not failing)`.

A dangling SHA and an inconclusive one (a shallow clone lacking the commit)
are the same observation, so neither fails the check. A link that fails to
resolve degrades to a coarser target rather than disappearing.

A project-wide check also covers each `change/<sha>` review's **structural**
commit references — the target's `head` and `base` and each comment's
`applied_commit` — as `rdm:src@<sha>` links derived from the stored SHAs (see
[`change-reviews.md`](change-reviews.md#commit-references-are-links)). They
are counted in `links_checked` and `commit_links_checked` and verified the
same way. An unresolved one carries `field` (`head`, `base`,
`applied_commit`) in place of `range_start`/`range_end` in JSON, and the text
line names it after the document: `unresolved commit link in
review/<id>#comment-2 [applied_commit]: …`. A roadmap, phase, task or plan
review's `applied_commit` is a plan-repo SHA and is never checked.

## Backlinks

`rdm backlinks <reference>` lists everything that references a target. The
reference takes one of three forms (an `rdm:` prefix is optional on each):

- a plan item: `roadmap/<slug>`, `phase/<roadmap-slug>/<stem-or-number>`,
  `task/<slug>` or `plan/<slug>`;
- a source-repo commit: `src@<sha>`, abbreviated or full;
- a source path: `src/<path>[@<rev>][#L<a>[-L<b>]]`, the same grammar as a
  code link. The path is normalized the way `--path` is (a leading `./` is
  dropped; `..`, an absolute path, and an `@` or `#` in the path are
  refused), and an empty `@` is refused.

A plan item is answered from the plan repo alone (`ops::links::backlinks`).
A commit or a path is a **code location**, answered by
`ops::links::code_annotations`, which also reads the project's source
checkout — see [Reverse lookup from a code location](#reverse-lookup-from-a-code-location).

A commit target finds three kinds of reference:

1. `rdm:src@<sha>` links written in any body (body entries, with a byte
   range);
2. change reviews whose `head`, `base` or `applied_commit` name the commit
   (structural entries tagged `[head]`, `[base]` or `[applied_commit]`, with
   `field` in JSON);
3. the commit-level comments of a change review whose head is that commit
   (entries tagged `[anchor]`).

The queried SHA is first resolved in the source checkout, so an abbreviation
is expanded exactly, and a SHA the checkout lacks is an error rather than an
empty answer. Stored and written revisions are then matched against the full
SHA case-insensitively by prefix, so an abbreviated `rdm:src@<sha>` in a body
or a legacy abbreviated `applied_commit` still matches.

### Reverse lookup from a code location

This is the question an editor or a web view asks: starting from a file (and
maybe a revision and some lines), what annotates it? The answer lists two
kinds of annotation:

- **documents** — every `rdm:src/<path>…` link on that path, in any
  roadmap, phase, task, plan, review summary or review comment body;
- **review comments** — each `change/` review comment, described by its
  coarsest true target (see
  [`change-reviews.md`](change-reviews.md#the-coarsest-true-target)), so a
  comment whose line anchor was lost still counts when its file exists at the
  review's head.

The matching rules:

- A path query matches **range-level and file-level** comments on that path.
- A **commit-level** comment (no recorded path, or a path that is absent, a
  directory or a submodule at the review's head) matches only a commit query.
- A match is **never inferred from a change's diff.** A whole-document
  comment on a change that touches `src/lib.rs` is not an annotation of
  `src/lib.rs`: nothing in the comment says so.
- A plan-document review's comments are never code annotations.

Each entry carries a link `rdm link resolve` accepts, its level (`range`,
`file` or `commit`), its line range when it has one, its revision when it is
pinned to one, and — for a comment — why its target is coarser than an
intact anchor and whether that was verified.

#### Decision: revision semantics

1. **A path query without a revision means *any revision*** — not the source's
   `HEAD`, and not an item's stamped commit.
   - Annotations are pinned to many commits: each review's head, and each
     document link's `@rev` or its item's stamped `commit`.
   - A `HEAD` default would silently drop every annotation recorded at an
     earlier commit, even where the file is byte-identical. The answer would
     be quietly incomplete rather than visibly wrong.
   - A query has no item, so "the item's stamped commit" cannot apply.
   - Every entry carries its pinned `rev`, so the consumer decides staleness
     and nothing is hidden on its behalf.
   - This differs on purpose from a revision-less **link**, which resolves
     against a default (see [Resolution](#resolution)): a link names one
     place to go, a query asks what exists anywhere.
2. **`@<rev>` narrows to annotations pinned at that exact commit.**
   - Both sides go through the source checkout's `rev-parse`, so a branch or
     tag means the commit it names *at query time*.
   - A document link's revision is its own `@rev`, else its document's
     stamped `commit`. A **floating** link (neither) is pinned to nothing: it
     is excluded from revision queries and returned by revision-less ones.
3. **Line ranges are compared by number, as recorded, and never translated
   across revisions.**
   - Translating would need a diff-based line remap, which this lookup does
     not do and which sits next to the "never inferred from a diff" rule.
   - Under `@<rev>` every match is pinned at that commit, so the comparison
     is exact. That is the precise form, and the one to use when lines
     matter.
   - Without a revision, the overlap is approximate for annotations recorded
     at other revisions. Each entry carries its `rev` and `lines`, so a
     consumer can tell.
4. **A commit query resolves its SHA in the source checkout before
   matching.** A commit query returns commit-level comments, and whether a
   comment is commit-level depends on which paths exist at its head, so
   answering without a repository would change the result set with the
   environment.

#### Line-range narrowing

With `#L<a>[-L<b>]`, a range-level entry is kept when its inclusive range
overlaps the query's (a single line `L<n>` is `L<n>-L<n>`). A file-level entry
is always kept: it annotates every line.

#### Empty, missing, unresolvable, unreadable

| Outcome | Exit | Output |
|---------|------|--------|
| The path is a file (at `@<rev>`, else at the checkout's `HEAD`) and nothing annotates it | 0 | `[]`, or `<target>: no annotations found.` |
| The path does not exist there | non-zero | `'<path>' does not exist at <rev> in the source repository — …` |
| The path is a directory or a submodule | non-zero | `'<path>' is a directory at <rev> — name a file` |
| The revision (or an unborn `HEAD`) does not resolve | non-zero | `'<rev>' does not name a commit in the source repository — …` |
| The source repository cannot be read | non-zero | the git error |

Existence is checked only when nothing matched, so a path deleted since it
was annotated still returns its annotations.

#### Source discovery

A code location needs the project's source checkout, found the way
`rdm review --on change/…` finds it (see
[`change-reviews.md`](change-reviews.md#source-discovery)): the checkout the
command runs in when it is the configured source, else a local
`source.repo`. With neither, or standing in the plan repo, the command
refuses — "empty" means nothing without a repository to check against. A
build without git support refuses every source-side target.

#### JSON

`rdm backlinks` on a commit or a path prints an array of entries, sorted by
document then position (body links, then fields, then the anchor):

```json
{
  "kind": "review", "id": "<id>", "comment": 2,
  "found_in": "anchor",
  "link": "rdm:src/src/lib.rs@<sha>",
  "path": "src/lib.rs",
  "rev": "<sha>",
  "level": "file",
  "reason": "outside-hunk",
  "unverified": false
}
```

| Key | Present |
|-----|---------|
| `kind` plus `id`/`comment`, `slug`, or `roadmap`/`stem` | always — the annotating document |
| `found_in` | always: `body`, `field` or `anchor` |
| `range_start`, `range_end` | `body` only: the link's byte range |
| `field` | `field` only: `head`, `base` or `applied_commit` |
| `link` | always: an `rdm:src/…` or `rdm:src@…` link `rdm link resolve` accepts |
| `path` | unless the entry is commit-level |
| `rev` | when pinned; absent for a floating document link |
| `lines` | exactly when the entry has a range: `{ "start", "end" }`, `end == start` for one line |
| `level` | always: `range`, `file` or `commit` |
| `reason` | for a comment coarser than an intact anchor: its `anchor_state`, `not-recorded`, or `ineligible` |
| `reason_note` | with `ineligible` only: why the path is not a file at the head |
| `unverified` | always: `true` when the checkout could not confirm the path at the review's head |

Item backlinks keep their own shape.

## Decision: commit rung in `src`, not `rdm:commit/`

Two surface forms were considered for linking a commit: a path-less rung
inside the existing `src` namespace, `rdm:src@<sha>`, and a sibling
namespace, `rdm:commit/<sha>`. The `src` rung was chosen:

1. **No new reserved slug.** Every link namespace is also a reserved roadmap
   slug (`RESERVED_ROADMAP_SLUGS` in `rdm-core/src/link.rs`: `task`, `plan`,
   `src`, `change`). A `commit` namespace would need `commit` reserved too,
   retroactively invalidating any existing roadmap named `commit` along with
   its `rdm:roadmap/commit` links and `Done: commit/...` lines. The `src`
   rung adds nothing to the list.
2. **One ladder, one namespace.** `rdm:src@<sha>` → `rdm:src/<path>@<sha>` →
   `rdm:src/<path>@<sha>#L<n>-L<m>` is a single coarse-to-fine ladder over
   one source repository. When a link cannot resolve it degrades to a coarser
   target; within one namespace that means dropping a suffix, while two
   namespaces would mean switching grammars.
3. **Unambiguous.** Every existing `src` form has `/` immediately after
   `src`, so `src@` cannot collide with any of them, nor with an item kind
   (kinds are split on `/`).
4. **`change/<sha>` stays rejected.** A review target is not a commit; the
   rejection message now names both correct alternatives.
