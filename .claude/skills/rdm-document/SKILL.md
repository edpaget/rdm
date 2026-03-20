---
name: rdm-document
description: Generate user documentation from a completed rdm roadmap using phase descriptions and commit SHAs
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Write
  - Edit
---

Generate user-facing documentation from a completed rdm roadmap. `$ARGUMENTS` should be `<roadmap-slug> [--out <path>]`.

**IMPORTANT: This is the rdm source repo. Always run `cargo build` first, then use `./target/debug/rdm` — never bare `rdm`.**

## Steps

1. Run `cargo build` to ensure the binary is up to date.
2. **Parse arguments**: extract the roadmap slug and optional `--out <path>` from `$ARGUMENTS`. Default output path is `docs/<slug>.md`.
3. **Read the roadmap**: `./target/debug/rdm roadmap show <slug> --project rdm` to get the overview and phase list.
4. **Validate completion**: all phases must be `done`. If any phase is not done, abort with a clear message listing which phases are incomplete and their statuses.
5. **Read each phase in order**: `./target/debug/rdm phase show <stem> --roadmap <slug> --project rdm --format json` for each phase. Collect titles, bodies, and commit SHAs.
6. **Gather code changes from commit SHAs**:
   - Extract the `commit` field from each phase's JSON output.
   - If SHAs are available, compute the git range in the **source repo** (the current working directory):
     - Multiple phases with SHAs: `git log --oneline <first_sha>~1..<last_sha>` and `git diff --stat <first_sha>~1..<last_sha>`
     - Single phase with SHA: `git log --oneline <sha>~1..<sha>` and `git diff --stat <sha>~1..<sha>`
     - For individual phase context: `git show --stat <sha>` per phase
   - **Missing SHAs**: if some or all phases lack commit SHAs, warn the user and fall back to phase descriptions only. Do not abort — the documentation can still be generated from phase content alone.
7. **Cross-reference**: compare phase descriptions with actual code changes (diff stats, file lists) to ensure the documentation accurately reflects what was built. Note any discrepancies.
8. **Draft documentation** with this structure:

   ```markdown
   # <Feature Title>

   ## Overview
   What the feature is — one or two paragraphs.

   ## Motivation
   Why it was built — the problem it solves.

   ## Usage
   Concrete examples: CLI commands, config options, API calls.
   Use fenced code blocks for commands and examples.

   ## How it works
   (Include only for complex features)
   Architecture, key modules, data flow.

   ## Limitations
   (Include only if applicable)
   Known gaps, unsupported scenarios, planned future work.
   ```

   Guidelines for drafting:
   - **Write for users**, not developers — focus on what they can do, not internal implementation details.
   - **Usage section is the most important** — include real, working examples.
   - **Internal/refactoring phases** (phases that only restructure code without user-visible changes): mention briefly in "How it works" if relevant, but omit from the Usage section.
   - **Derive content from both sources**: phase descriptions explain intent, code diffs show what actually shipped.

9. **Write the file** to the output path.
10. **Present the draft to the user** for review. Summarize what was generated and note any gaps (e.g., phases without SHAs, internal-only phases). Do not consider the task done until the user has reviewed and approved the documentation.

## Edge cases

- **Phases without commit SHAs**: warn once and continue. Use phase descriptions and body content as the sole source. Note in the output to the user which phases lacked SHAs.
- **Single-phase roadmaps**: the diff range is just that one commit. Handle the `~1..` range correctly.
- **Internal refactoring phases**: if a phase description indicates pure refactoring or internal restructuring with no user-visible changes, note it as an internal change in the "How it works" section and omit it from Usage.
- **Roadmap not found**: relay the error from rdm and abort.
- **Empty phase bodies**: some phases may have minimal descriptions. Use the commit diff to fill in details.
