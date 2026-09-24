# Plan Authoring: Grammar and Acceptance Criteria Rubric

This document specifies the canonical grammar for roadmap and phase body documents in an rdm plan repository, the sections appended by skills and workflows, and the rubric that makes acceptance criteria actionable by autonomously acting agents.

## Overview

Roadmaps and phases are authored in free-form Markdown, but the sections they contain follow a strict structure. This structure enables:
- **Consistency**: Plan documents have a predictable shape, making them easier to read and review
- **Machine automation**: Agents can locate specific sections and understand their purpose
- **Concrete acceptance criteria**: Authors know what makes a criterion actionable (observable, scoped, deferral-declared)
- **Section accountability**: Readers can distinguish author-written content from sections appended by tools

This document is the single source of truth for both the grammar and the rubric. When a skill or workflow creates or updates a plan document, it must conform to the grammar specified here.

## Roadmap Body Grammar

A roadmap body contains two sections with distinct purposes.

### `## Intent` Section (Required)

The Intent section describes why the roadmap exists, what it will achieve, and what it deliberately excludes. It has a fixed grammar with the following sub-sections:

- **`Goal.`** A single, observable statement of what the roadmap delivers. Not the mechanism (how), only the outcome (what) and why it matters.
- **`Non-goals.`** (Optional) Bullet list of things explicitly out of scope, preventing scope creep during implementation.
- **`Done looks like.`** Observable signals that the roadmap is complete. Starts with "WHEN <situation> THEN <observable outcome>." format.
- **`Interview.`** (Optional) Verbatim answers to discovery questions, captured as "Q: <question> → A: <answer>".
- **`Open.`** (Optional) Unresolved high-impact questions, to be answered before or during implementation.

The Intent grammar is fixed and enforced by the plan-review workflow — do not invent subsections or reorder these. See the rdm-roadmap skill documentation for the complete Intent template and examples.

### `## Context` Section (Optional)

Free-form narrative providing background, relevant history, or strategic context for the roadmap. This section is optional and authored at the implementer's discretion.

## Phase Body Grammar

A phase body contains three required sections and may contain additional sections appended by tools.

### `## Context` Section (Required)

Explains why this phase exists and what it builds on. Includes:
- The problem the phase solves or the capability it adds
- What earlier phases or external dependencies it depends on
- The scope of this phase relative to the full roadmap

### `## Approach` Section (Required)

The strategy or design principle guiding the implementation. Describes:
- The high-level strategy or design philosophy
- Key architectural decisions or principles
- Why this approach was chosen over alternatives
- Any technical constraints or context shaping the approach

This section replaces the deprecated `## Steps` section. It is a description of *how* the work will be done, not a detailed checklist of steps.

### `## Acceptance Criteria` Section (Required)

Three bullet-point statements of what success looks like. Each criterion must follow the AC rubric specified below.

## Sections Appended by Skills and Workflows

As a phase moves through implementation and review, other skills and workflows may append sections to the phase body. These sections are always appended *after* the Acceptance Criteria and are authored by their respective tools, not by the person implementing the phase.

| Section | Written By | Purpose |
|---------|-----------|---------|
| `## Intent` | rdm-roadmap skill (roadmaps only) | Captured during roadmap creation via interview |
| `## Key code` | rdm-do skill (optional) | Pinned `rdm:src/` links to touched files recorded at finalize; links resolve to a specific commit for permanence |
| `## Estimate` | Current versions write difficulty field; older phases may carry `## Estimate` note written by rdm-wf-estimate | Provides a brief justification of the difficulty rating. This section is optional and found only on phases worked on with earlier versions of the estimate workflow |
| Plan-review round block | rdm-wf-plan-review workflow | Recorded during the plan-review gate; shows reviewer verdict and any feedback on the plan before implementation began |
| Change-review records | rdm-wf-review-refute-fix workflow | Recorded in separate `change/<sha>` review documents, not appended to the phase body |

Readers can distinguish author-written content (Context, Approach, Acceptance Criteria) from machine-appended content by these boundaries.

## Acceptance Criteria Rubric

An acceptance criterion is a statement of what success looks like. For a criterion to be actionable by an autonomously acting agent, it must satisfy three properties:

### 1. Observable Outcome in Positive Form

**Rule:** Each criterion must name one concrete, observable outcome that success achieves — not what to avoid, not a prohibition.

**Why this matters:** An agent grading a criterion needs to know what to *build* or *verify*, not what to *prevent*. A prohibition ("don't return 5xx errors") tells an implementer nothing about what *should* happen. An observable outcome ("the API returns 200 OK with a valid response") tells the agent what to test for.

**Failure mode:** An implementer reads "avoid race conditions" and doesn't know whether the criterion is satisfied by adding locks, by using a language feature, by refactoring the architecture, or by accepting serialization costs. The agent marks the criterion incomplete because no observable state confirms success. This blocks landing.

**Good example:**  
"The search index contains all user-visible records and is updated within 5 seconds of any record mutation"  
- Observable: presence of all records in the index, latency bound
- Positive form: states what the system *does*, not what it *avoids*

**Problematic example:**  
"Don't introduce race conditions"  
- Prohibition form: names what to avoid, not what to achieve
- No observable outcome: a lock, a refactor, or a serialization choice all satisfy it differently

### 2. Scope Bound Carried by the Criterion Itself

**Rule:** The criterion itself must state where it ends. The bound must not depend on external context or future clarification.

**Why this matters:** An agent needs to know when a criterion is *done*, not when more context might appear. "The search works" could mean "returns results in 100ms" or "handles typos" or "works on mobile" — none of these are stated. An unbounded criterion cannot transition from "in-progress" to "done" without either guessing or asking for clarification. Under an agent workflow, this becomes a blocking finding.

**Failure mode:** An implementer reads "improve search performance" and ships a 50ms implementation. The agent or reviewer asks, "Is that fast enough?" — now it's a blocking question. The criterion, which should have been done, is reopened. The deferral should have been declared up front.

**Good example:**  
"The search index is queried in under 100ms for any query matching 10,000+ records"  
- Scoped: latency bound is explicit
- Bounded to an observable threshold: measurable, not subjective

**Problematic example:**  
"The search is fast"  
- Unbounded: "fast" is relative and requires interpretation
- No agent can mark it done without knowing the implicit threshold

### 3. Declared Deferral at Authoring Time

**Rule:** If the author knows a gap will exist when implementation is done, state it explicitly using language like "except for X" or "initial implementation omits Y".

**Why this matters:** An agent's code-review `ac` dimension treats any deferred or caveated criterion as a blocking finding if that deferral is not declared *when the phase is authored*. A deferral discovered during implementation becomes a hard blocker. A deferral declared up front is honored — the phase can land with a known limitation.

**Failure mode:** An author writes "The API supports all query operators" but doesn't mention that regex operators are deferred. At review time, the agent finds the regex support missing and marks it as an AC gap. The phase cannot land. If the deferral had been declared ("... except for regex operators, deferred to phase 2"), the agent honors it and the phase lands.

**Good example:**  
"The search API supports all query operators except regex, deferred to phase 2"  
- Observable: supported operators are listed
- Scoped: exception is named
- Deferral: explicit ("deferred to phase 2")

**Problematic example:**  
"The search API supports all query operators"  
- Incomplete: omits the known limitation
- At review: agent finds the missing regex support, blocks the phase
- The criterion was only true "in spirit" — a tacit deferral, not a declared one

## Example: A Complete Phase Body

```markdown
## Context

User onboarding currently requires manual email verification. This phase implements
automated SMS verification so new users can be active within seconds instead of waiting
for email delivery.

## Approach

Use the Twilio API to send SMS codes and verify ownership of a phone number. Each new user
receives a 6-digit code valid for 10 minutes. Verification is attempted up to 3 times per
session. This approach is faster than email, simpler than TOTP, and leverages an existing
Twilio contract already in use for password resets.

## Acceptance Criteria

- [ ] SMS codes are sent via Twilio within 100ms of signup form submission
- [ ] Verification codes are valid for exactly 10 minutes and are single-use, except SMS resend within the same session reissues a new code
- [ ] Code verification fails after 3 incorrect attempts in a single session; the user must restart signup, except existing verified users who fail phone verification are deferred to a manual support workflow in phase 2
```

Each criterion is observable (codes are sent, valid for a duration, fail after N attempts), scoped (timing bounds, attempt limits), and deferrals are declared (resend behavior is clear, manual workflow for existing users is deferred).

## Verification

This spec is a reference document. No automated gate verifies that phase bodies conform to the grammar — that is a human review responsibility. The `rdm link check` command validates only the `rdm:` links in plan documents, not the grammar itself. Future phases may wire the AC rubric into the code-review workflow to automatically grade criteria for observable outcomes and declared deferrals.
