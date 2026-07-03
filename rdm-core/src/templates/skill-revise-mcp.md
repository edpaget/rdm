---
name: rdm-revise
description: Act on document reviews requesting changes — work each comment and drive the review to addressed
allowed-tools:
  - Read
  - Glob
  - Grep
  - {t_review_requests}
  - {t_review_show}
  - {t_review_address_comment}
  - {t_review_complete}
  - {t_phase_update}
  - {t_task_update}
  - {t_roadmap_update}
---

Act on submitted document reviews with verdict `request-changes`: work through each comment, apply the requested edits through the rdm update tools, and drive the review to `addressed`. This is not `rdm-review` (which reviews *implementations*) — this skill acts on *document* reviews of roadmaps, phases, and tasks.

`$ARGUMENTS` may name a specific review id; when empty, work every review in the queue.
{principles}
## Steps

### 1. Discover the queue

Call `rdm_review_requests` with `project: {proj_param}`. It returns the submitted reviews with verdict `request-changes`, each with `id`, `target`, `author`, `summary`, `created_commit`, and `open_comment_count`. If `$ARGUMENTS` names a review id, work only that review; otherwise work each review in turn.

### 2. Read the review — summary first

Call `rdm_review_show` with `project: {proj_param}, review_id: "<id>"`. Read the review's summary (`review.body`) **before** touching any comment: it carries the reviewer's overall intent, and each comment is an instance of that intent, not an isolated request.

The response needs no follow-up calls: `review.comments[]` carries each comment's stored `anchor` (a tagged union on `anchor_type`) and computed `resolution`, and `documents[]` carries the rendered body of every referenced document — both `body_at_created_commit` (what the reviewer saw) and `current_body` (HEAD).

### 3. Dispatch each open comment on its anchor

For each comment with `status: "open"`, branch on the anchor:

- **`anchor_type: text-quote`, `resolution.state: resolved`** — the quoted span was located. `resolution.range_start`/`range_end` index the body named by `resolution.body`: `original` means the matching document's `body_at_created_commit`, `current` means its `current_body`. Make the targeted edit at that span.
- **`anchor_type: text-quote`, `resolution.state: drifted`** — the document changed since the review; the range indexes `body_at_created_commit`, and `resolution.quote` is the text the reviewer saw. If you can confidently map the intent onto the current text, edit it and note the drift in your reply. If not, do **not** guess — go to step 6.
- **No anchor, `resolution.state: unresolved`, or an unrecognized `anchor_type`** — treat as **whole-document** feedback: read the target document's `current_body` (or the comment's `doc` phase, for roadmap reviews) in full, apply the change in context, and note in your reply that no anchor was resolved.

### 4. Apply the edit and capture the commit

Apply the change through the matching update tool (never edit plan files directly): `rdm_phase_update` with `project: {proj_param}, roadmap: "<slug>", phase: "<stem>", body: "..."`, `rdm_task_update` with `project: {proj_param}, task: "<slug>", body: "..."`, or `rdm_roadmap_update` with `project: {proj_param}, roadmap: "<slug>", body: "..."`.

Each mutation auto-commits to the plan repo, and the tool's response ends with a `Commit: <sha>` line naming that commit. **Capture that SHA from the response** — it is the provenance record for the next step.

### 5. Record the resolution

Call `rdm_review_address_comment` with:

```
project: {proj_param}, review_id: "<id>", comment_id: <n>,
status: "addressed", applied_commit: "<sha from step 4>",
reply: "What changed, and whether the anchor resolved (note drift if any)."
```

Always thread `applied_commit` explicitly from the `Commit:` value the update tool reported. If omitted with `status: "addressed"`, the tool falls back to the plan-repo HEAD before the call — best-effort only, and wrong whenever an unrelated commit landed in between. It never defaults for `wont-fix`.

### 6. Ambiguous comment or anchor drifted beyond recovery

Call `rdm_review_address_comment` with **no `status`** and a `reply` asking for clarification — the comment stays open — and continue with the remaining comments:

```
project: {proj_param}, review_id: "<id>", comment_id: <n>,
reply: "The quoted text changed since the review — did you mean X or Y?"
```

### 7. Won't-fix escape hatch

When a comment should not be acted on, say why and close it without a commit: `rdm_review_address_comment` with `status: "wont-fix"` and a `reply` giving the reasoning.

### 8. Close the review

When **no comments remain open**, call `rdm_review_complete` with `project: {proj_param}, review_id: "<id>"`.

If any comment was left open for clarification, **leave the review submitted** (`rdm_review_complete` refuses and lists the open comment ids — expected) and report the open comment ids and the questions you asked.

## Guidelines

- The summary is the spec; comments are its instances. Resolve conflicts between a comment and the summary in favor of the reviewer's overall intent, and say so in the reply.
- One comment, one reply: every terminal status (`addressed` or `wont-fix`) must carry a reply explaining the decision.
- Never mark a comment `addressed` without an actual applied change and its commit SHA.
- Unknown `anchor_type` values are expected (written by newer rdm versions) — they are whole-document comments, not errors.
