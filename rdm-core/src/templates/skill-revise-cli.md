---
name: rdm-revise
description: Act on document reviews requesting changes — work each comment and drive the review to addressed
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
---

Act on submitted document reviews with verdict `request-changes`: work through each comment, apply the requested edits through `rdm` update commands, and drive the review to `addressed`. This is not `rdm-review` (which reviews *implementations*) — this skill acts on *document* reviews of roadmaps, phases, and tasks.

`$ARGUMENTS` may name a specific review id; when empty, work every review in the queue.
{principles}
## Steps

### 1. Discover the queue

```bash
rdm review requests --format json {proj_flag}
```

This lists submitted reviews with verdict `request-changes`. If `$ARGUMENTS` names a review id, work only that review; otherwise work each review in turn.

### 2. Read the review — summary first

```bash
rdm review show <review-id> --format json {proj_flag}
```

Read the review's summary (`body`) **before** touching any comment: it carries the reviewer's overall intent, and each comment is an instance of that intent, not an isolated request. The JSON also carries, per comment, the stored `anchor` (a tagged union on `anchor_type`) and a computed `resolution`.

### 3. Dispatch each open comment on its anchor

For each comment with `status: open`, branch on the anchor:

- **`anchor_type: text-quote`, `resolution.state: resolved`** — the quoted span was located. `resolution.range_start`/`range_end` index the body version named by `resolution.body`: `original` means the document as of the review's `created_commit` (re-read it with `--at <created_commit>` if needed), `current` means the document at HEAD. Make the targeted edit at that span.
- **`anchor_type: text-quote`, `resolution.state: drifted`** — the document changed since the review; the range indexes the `created_commit` body, and `resolution.quote` is the text the reviewer saw. If you can confidently map the intent onto the current text, edit it and note the drift in your reply. If not, do **not** guess — go to step 6.
- **No anchor, `resolution.state: unresolved`, or an unrecognized `anchor_type`** — treat as **whole-document** feedback: read the target document (or the comment's `doc` phase, for roadmap reviews) in full, apply the change in context, and note in your reply that no anchor was resolved.

### 4. Apply the edit and capture the commit

Apply the change through the matching update command (never edit plan files directly). Pass the full revised body with `--body` — `update` does **not** read stdin, so a heredoc is silently ignored. Capture the multiline body into a shell variable with a quoted heredoc (which keeps backticks, `$`, and punctuation literal), then pass `--body "$body"`:

```bash
body=$(cat <<'EOF'
...full revised body...
EOF
)
rdm phase update <stem-or-number> --roadmap <slug> --body "$body" --no-edit {proj_flag}
# or: rdm task update <slug> --body "$body" --no-edit {proj_flag}
# or: rdm roadmap update <slug> --body "$body" --no-edit {proj_flag}
```

`--body` is authoritative: rdm takes the value verbatim and never touches stdin.

Each mutation only **stages** the change — it does not commit. Land it immediately, before moving to the next comment:

```bash
rdm commit -m "docs(plan): address review comment"
```

Then capture the SHA of that commit **immediately, before any review update** (which creates its own commit):

```bash
git -C "$RDM_ROOT" rev-parse HEAD
```

**Commit per comment, not per batch:** `rdm commit` lands *every* currently staged change as one commit, so if you edited more than one comment's target before committing, that single SHA would no longer identify which comment it resolved. Commit after **each** comment's edit, before starting the next, to keep a 1:1 comment→commit provenance trail — do not batch multiple comments' edits for efficiency here.

### 5. Record the resolution

```bash
rdm review update <review-id> --comment <n> --status addressed --applied-commit <sha> --reply "What changed, and whether the anchor resolved (note drift if any)." {proj_flag}
```

Always pass `--applied-commit` explicitly with the SHA captured in step 4 — never a later HEAD, which may be a review-file-only commit.

### 6. Ambiguous comment or anchor drifted beyond recovery

Set a reply asking for clarification, leave the comment **open** (no `--status`), and continue with the remaining comments:

```bash
rdm review update <review-id> --comment <n> --reply "The quoted text changed since the review — did you mean X or Y?" {proj_flag}
```

### 7. Won't-fix escape hatch

When a comment should not be acted on, say why and close it without a commit:

```bash
rdm review update <review-id> --comment <n> --status wont-fix --reply "Reasoning for not making this change." {proj_flag}
```

### 8. Close the review

When **no comments remain open**:

```bash
rdm review update <review-id> --state addressed {proj_flag}
```

If any comment was left open for clarification, **leave the review submitted** (the transition would be refused anyway) and report the open comment ids and the questions you asked.

## Guidelines

- The summary is the spec; comments are its instances. Resolve conflicts between a comment and the summary in favor of the reviewer's overall intent, and say so in the reply.
- One comment, one reply: every terminal status (`addressed` or `wont-fix`) must carry a reply explaining the decision.
- Never mark a comment `addressed` without an actual applied change and its commit SHA.
- Unknown `anchor_type` values are expected (written by newer rdm versions) — they are whole-document comments, not errors.
