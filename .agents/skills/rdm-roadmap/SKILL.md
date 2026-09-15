---
name: rdm-roadmap
description: Create or refine an rdm roadmap with ordered, independently reviewable phases before implementation.
---

# Roadmap authoring

Use the shell and ordinary file-reading tools. Commands below use the configured `RDM_BIN` or an installed `rdm`; follow repository bootstrap instructions when developing rdm itself. Carry the same `RDM_ROOT`, `RDM_PROJECT`, and stable `RDM_SESSION` into every shell call. Replace placeholders before execution.

Read related work before writing:

```bash
"${RDM_BIN:-rdm}" roadmap list --project rdm
"${RDM_BIN:-rdm}" search <topic> --project rdm
"${RDM_BIN:-rdm}" tag list --project rdm
```

Capture the user's intended outcome, boundaries, dependencies, and observable acceptance criteria. Order phases to deliver a usable vertical slice early. Keep unrelated work separate. Discuss material scope decisions before creating them.

Create through the CLI, with meaningful tags and `needs-plan-review` on the roadmap and each phase:

```bash
"${RDM_BIN:-rdm}" roadmap create <slug> --title "Title" --body "Intent and scope" --tags <topic>,needs-plan-review --no-edit --project rdm
"${RDM_BIN:-rdm}" phase create <bare-slug> --number 1 --roadmap <slug> --title "Title" --body "Work and acceptance criteria" --tags <topic>,needs-plan-review --no-edit --project rdm
```

Use bare phase slugs: rdm adds `phase-N-`. On update, `--body` replaces the whole body and stdin is ignored; `--tags` replaces all tags, so preserve existing ones. Never edit plan files directly. Validate any `rdm:` links with `rdm link check --on <ref>`, using the same binary and project.

Inspect `"${RDM_BIN:-rdm}" status`, then `"${RDM_BIN:-rdm}" commit -m "docs(plan): describe roadmap changes"`. These commands take no project flag and commit only this session's changes. Do not use `--all` to work around a missing session identity.

Hand the plan to an independent human or a working plan-review host. Codex phase-one support does not provide `rdm-plan-review`; author self-checks do not clear `needs-plan-review` or authorize implementation. Report exact roadmap/phase references and remaining decisions.


## Principles

Read `docs/principles.md` before starting. It contains project conventions that should guide your work.
