---
name: rdm-estimate
description: Rate each phase's difficulty and assign a model tier from its body
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
---

Estimate the difficulty of one or more rdm phases. `$ARGUMENTS` is either a roadmap slug (estimate every phase in the roadmap) or `<roadmap-slug> <phase-number>` (estimate a single phase).
{principles}
## Steps

1. **Parse arguments**: read `$ARGUMENTS`. If only a roadmap slug is given, estimate every phase in it; if a phase number follows the slug, estimate just that one phase.
2. **List the phases**: `rdm phase list --roadmap <slug> {proj_flag}`. The output shows each phase's number, stem, status, and current difficulty.
3. **Select phases to estimate**: keep only phases whose **difficulty is unset**. Skip any phase that already has a difficulty — it was either estimated on an earlier run or set deliberately by a human, and must be left untouched.
4. **For each selected phase**, in order:
   1. Read the body: `rdm phase show <stem-or-number> --roadmap <slug> {proj_flag}`.
   2. Rate its difficulty as one of `trivial`, `easy`, `moderate`, or `hard`, based on the scope, risk, and breadth of the work the body describes. Write a one-line justification for the rating.
   3. Record the justification by appending a short `## Estimate` note to the phase body. Take the existing body from step 4.1, append a section like:
      ```
      ## Estimate

      <difficulty> — <one-line justification>
      ```
      then write the difficulty and the updated body in a single update. `update` does **not** read stdin, so pass the body with `--body`: capture the full body into a shell variable with a quoted heredoc (which keeps backticks, `$`, and punctuation literal), then pass `--body "$body"`:
      ```bash
      body=$(cat <<'EOF'
      <full phase body, with the appended ## Estimate section>
      EOF
      )
      rdm phase update <stem-or-number> --difficulty <difficulty> --body "$body" --no-edit --roadmap <slug> {proj_flag}
      ```
      Do **not** pass `--model`: the model tier is derived automatically from the difficulty (`trivial`/`easy` → small, `moderate` → medium, `hard` → large).
5. **Report** what was estimated: list each phase with its assigned difficulty, the derived model tier, and the one-line justification. Note any phases that were skipped because they already had a difficulty.

## Guidelines

- **Skipping is the override mechanism.** A phase that already has a difficulty is never re-rated, so re-running this skill is idempotent and never overwrites a human-set (or previously estimated) value.
- To force a re-estimate of a phase, a human first clears its difficulty (`rdm phase update <phase> --clear-difficulty --no-edit --roadmap <slug> {proj_flag}`); the next run will then pick it up.
- Rate from the body's described scope, not from how the work feels to implement: a one-line change is `trivial`/`easy`; a self-contained feature is `moderate`; cross-cutting or high-risk work is `hard`.
- Keep the justification to a single line — it explains the rating, it is not a plan.
- Never set `--model` by hand here; the difficulty→tier mapping is authoritative.
