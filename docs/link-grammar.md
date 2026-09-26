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
