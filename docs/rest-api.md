# REST API

rdm includes a REST API server for programmatic integrations beyond the CLI.

## Starting the Server

```bash
rdm serve --port 8400
```

The server binds to `127.0.0.1` by default. Use `--host` to change the bind address.

## Endpoints

Endpoints mirror the CLI commands. All endpoints accept and return JSON by default.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/projects` | List all projects |
| `GET` | `/projects/:project/roadmaps` | List roadmaps with progress |
| `GET` | `/projects/:project/roadmaps/:roadmap` | Show roadmap details with phases |
| `POST` | `/projects/:project/roadmaps` | Create a new roadmap |
| `PATCH` | `/projects/:project/roadmaps/:roadmap/phases/:phase` | Update a phase |
| `GET` | `/projects/:project/tasks` | List tasks with optional filters |
| `GET` | `/projects/:project/tasks/:task` | Show task details |
| `POST` | `/projects/:project/tasks` | Create a new task |
| `PATCH` | `/projects/:project/tasks/:task` | Update a task |
| `GET` | `/projects/:project/annotations?target=…` | List what annotates a code location (see [Code-location annotations](#code-location-annotations)) |

## Review comments

`POST /projects/:project/reviews/:review_id/comments` adds a comment to a draft
review. Its JSON body takes `body` (required), `doc`, and one way of anchoring:
a text-quote `anchor` mapping, an anchorless `anchor_state` (plus
`intended_path` on a `change/` review), or `line`.

`line` is `"N"` or `"N-M"`, the grammar of `rdm review comment --line`: 1-based
lines of the document body as it was at the review's `created_commit` (see
[`file-formats.md`](file-formats.md#anchors)). rdm captures the lines' text as
the stored `line-range` anchor's `snippet`; the request never supplies it.

| Request | Status |
|---------|--------|
| Malformed `line` (`0`, `5-3`, non-numeric) | `422` |
| A range past the end of the document at `created_commit` | `422` |
| `line` with `anchor` or `anchor_state`, or on a `change/` review | `400` |
| A hand-built `line-range` `anchor` (here or on `PATCH …/comments/:id`) | `400`, pointing at `line` |

## Code-location annotations

`GET /projects/:project/annotations?target=<reference>` is the web UI's reverse
lookup from code: the documents and review comments that annotate a commit or a
source path. `target` takes exactly what `rdm backlinks` takes for a source-side
target — `rdm:src/<path>[@<rev>][#L<a>[-L<b>]]` or `rdm:src@<sha>`, the `rdm:`
prefix optional, percent-encoded in the query. The grammar and matching rules
are core's: see [`link-grammar.md` § "Reverse lookup from a code
location"](link-grammar.md#reverse-lookup-from-a-code-location).

**It returns annotations, never file contents.** No route serves a file, a blob
at a revision, blame or highlighting; each annotation links *out* to the code at
its pinned revision instead.

The HAL+JSON representation:

| Key | Meaning |
|-----|---------|
| `target` | the queried location (`src/<path>…` or `src@<sha>`) |
| `source_verification_skipped` | present only when no source repository was read: why |
| `_embedded.annotations` | the `rdm backlinks <target> --format json` entries, verbatim and in the same order; **absent** (not `[]`) when no source was read |
| `_links.rdm:backlink` | one link per distinct annotating document (its detail page; a change review's is the page of the item its plan implements) |
| `_links.rdm:code` | one link per entry with a navigable forge permalink, titled with the entry's `link` |

The HTML representation lists each annotation with its pinned permalink, its
level (`range`/`file`/`commit`), where it was found (a body link, a
`[head]`/`[base]`/`[applied_commit]` field, or a comment's anchor), and why a
comment is coarser than an intact anchor — a deliberate whole-document comment
is labelled differently from a lost anchor, and both differently from a legacy
comment whose reason was never recorded. A "Referenced by" section lists the
distinct annotating documents, and on a path query "any revision" / "whole file"
links widen a pinned query.

Every rendered `rdm:src/…` or `rdm:src@…` link on a detail page is followed by
an `annotations` link to this resource, pinned to the location the link
resolves to (its own `@rev`, else the containing phase's or task's stamped
commit). A change review comment's `applied_commit` renders the same way. Item
resources advertise the same entry points as `_links.rdm:annotations`, one per
distinct code or commit link in the body.

| Status | When |
|--------|------|
| `200` | the annotations, or — with no readable source repository — the fail-open note and no list |
| `400` | `target` missing or empty, not a source-side reference (e.g. `task/x`), or malformed |
| `404` | the revision does not resolve in the source repository, or the path does not exist there |
| `422` | the path is a directory or a submodule |

**No source repository fails open.** The server reads only a project's
configured local `source.repo` (never its own working directory or the plan
repo); without one the response is still `200`, carrying the note in
`source_verification_skipped` (HAL) or a notice (HTML).

**Beside a local checkout.** The web UI normally runs on the same machine as a
local checkout, and there the annotations page works fully: the server reads
annotations from the local `source.repo` and shows each entry's pinned
`path@rev`, which is enough to open the file locally (an editor integration is
the natural place to jump to the line). A permalink is navigable only when
`source.repo` is an `http(s)` URL; a local path renders as a non-navigable
`path@rev` span, and a project cannot yet name both a local checkout and a
forge URL. Clickable forge links matter only when the web UI is served away
from the code (e.g. a shared host); they are the follow-up task
`source-repo-web-url-separate-from-local-checkout`.

## Mutations Are Staged, Not Committed

A running server is **one session**, and every write it makes is attributed to
that session's changeset. By default the server **stages** mutations to disk and
commits nothing — a `POST` or `PATCH` that returns `201`/`200` has written the
markdown file, but has created no git commit. Landing them is a separate,
explicit step.

That default is deliberately loud on three surfaces rather than one, because a
boot-time warning on a server that has been up for a week is indistinguishable
from no warning at all:

- a `WARN` line on stderr at boot,
- a `WARN` line on stderr for **each** mutation, and
- response headers.

### Response headers

| Header | Set on | Meaning |
|--------|--------|---------|
| `x-rdm-changeset` | every response, when the server resolved a changeset id | The changeset this server's mutations belong to. Pass it to `rdm commit --changeset <id>` to land them. |
| `x-rdm-staged` | mutating responses only (anything other than `GET`/`HEAD`/`OPTIONS`), and only under the staging-only default | The plain-ASCII warning text: `WARN: mutation staged, NOT committed. Changeset '<id>'. Reconcile with: rdm commit --changeset <id>` |

Under `--autocommit` the write did reach a commit, so `x-rdm-staged` is absent —
its presence is exactly the signal that something is pending.

### Reconciling

The named recovery command is the one carried in `x-rdm-staged`:

```bash
rdm commit --changeset <id>     # land what the server staged
rdm session list                # find the id if you did not capture a header
```

### Committing on every mutation

Pass `--autocommit` to the `rdm-server` binary, or set
`RDM_SERVER_AUTOCOMMIT=1` (`true` is also accepted; any other value keeps the
staging-only default, so an unparseable value never silently opts in). Each
mutation then lands a scoped commit of its own. `--changeset <id>` (else
`RDM_SESSION`) pins which changeset the server attributes its writes to; a
trailing `--changeset` with no argument, or a blank id, falls back to the
ordinary session-resolution chain rather than naming a changeset `""`.

These two flags are read by the standalone `rdm-server` binary. The `rdm serve`
CLI subcommand starts the same router on the staging-only default but does not
currently parse them, and does not pre-resolve a changeset id — so it emits no
`x-rdm-changeset` header and its `x-rdm-staged` text renders the id as
`<unresolved>`. Use `rdm session list` to find the changeset in that case.

### Concurrent writers

Because scoping is per changeset, a server and a CLI session working in the same
plan repo do not sweep up each other's uncommitted work. Two writers racing on
the *same file* are a different matter, and are refused rather than silently
resolved — see the `409` rows below.

## Content Negotiation

The server uses the `Accept` header to determine the response format:

- **`application/hal+json`** — JSON with `_links` for discoverability and `_embedded` for related resources. API consumers should prefer this format.
- **`text/html`** — HTML rendered from Askama templates. Browsers get a human-readable view automatically.
- **`application/json`** — Plain JSON without HAL links.

## Error Responses

Errors follow [RFC 9457 Problem Details](https://www.rfc-editor.org/rfc/rfc9457):

```json
{
  "type": "urn:rdm:error:project-not-found",
  "title": "Project not found",
  "status": 404,
  "detail": "No project with slug 'foo' exists in this plan repo."
}
```

Each `rdm-core` error variant maps to a specific HTTP status code. The mapping is exhaustive — adding a new error variant to core forces the server to handle it at compile time.

### Lost-update refusals (`409 Conflict`)

Two writers racing on the same document are refused rather than silently
resolved. Both refusals carry the core error's own message verbatim as `detail`,
and both state that **nothing was written or committed** — so a `409` is always
safe to retry after re-reading:

| Core error | When | What to do |
|------------|------|------------|
| `StaleWrite` | The document changed on disk between this request reading it and writing it back — another session wrote it concurrently. | Re-issue the request so the change applies on top of the current content. |
| `ChangesetPathOverwritten` | At commit time, another session had overwritten a path this changeset wrote, so committing would land their content under your message. | Re-run the request that produced your change, then commit. |
