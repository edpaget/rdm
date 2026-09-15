---
name: rdm-revise
description: Address requested changes on rdm document reviews with per-comment commit provenance and explicit unresolved feedback.
---

# Revise document feedback

This edits roadmap, phase, or task documents, not implementation code review. Use the configured `RDM_BIN` (the rebuilt development binary in rdm's own repo) or installed `rdm`. Carry the same `RDM_ROOT`, `RDM_PROJECT`, and stable `RDM_SESSION` into each shell call.

```bash
"${RDM_BIN:-rdm}" review requests --format json --project rdm
"${RDM_BIN:-rdm}" review show <review-id> --format json --project rdm
```

Work only the requested review(s). Read the summary first, then each open comment and the complete target document. For roadmap comments, honor a comment's `doc` phase. Interpret anchors as follows:

- Resolved text quote: use the body named by `resolution.body`; `original` refers to `created_commit`, readable with the document's `show --at <sha>`.
- Drifted text quote: ranges index the original body, not current text. Map the intent only when unambiguous.
- Missing, unresolved, or unknown anchor: treat as whole-document feedback.

Apply each requested edit through `roadmap update`, `phase update`, or `task update`, using the full preserved body via `--body` and `--no-edit`; update does not read stdin. Preserve unrelated content and tags. Check `rdm:` links. Commit each comment's document edit separately with `"${RDM_BIN:-rdm}" commit -m "docs(plan): address review comment"`, then capture that plan-repo commit SHA immediately, before updating the review. Inspect `rdm status` first to avoid mixing other changes from your session.

```bash
"${RDM_BIN:-rdm}" review update <review-id> --comment <n> --status addressed --applied-commit <plan-sha> --reply "What changed" --project rdm
```

Every addressed comment needs an actual change and its explicit applied commit. Pinned `rdm:src/` links instead use source-repo SHAs. Commit review bookkeeping separately before starting the next comment, maintaining one document-edit commit per comment.

If intent or drift is ambiguous, post a clarification reply without a status change; leave the comment open. If a change should not be made, use `--status wont-fix --reply "Reason"`, within the user's scope. Close with `review update <review-id> --state addressed` only when no comments remain open, then commit the bookkeeping. Otherwise leave the review submitted and report outstanding questions. These are document resolution states, not independent implementation-review approval.


## Principles

Read `docs/principles.md` before starting. It contains project conventions that should guide your work.
