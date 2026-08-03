//! document — pure decision logic for the headless documentation-draft workflow.
//!
//! This is the **single source of truth** for the deterministic decision core of
//! the `rdm-wf-document` workflow: argument parsing/defaulting, the all-done validation,
//! and the has-SHA-vs-body-only fallback check that decides whether a phase's
//! per-phase gather step runs `git log`/`git diff --stat` or falls back to
//! phase-body-only. Because the Claude Code Workflow runtime cannot
//! `import`/`require` (see docs/workflow-schemas.md § "Import spike"), the marked
//! block below is copied BYTE-IDENTICAL into `.claude/workflows/rdm-wf-document.js`.
//! Unlike the review-refute-fix block — which is stamped by
//! `scripts/gen-workflow-review.sh` — this block is NOT run through a generator
//! (it is unique to the one `document` consumer, mirroring the `dispatch-outcome`
//! block's precedent in `lib/dispatch-phase.mjs`); instead
//! `scripts/verify-workflow-document.sh` gates the two copies for byte-equality.
//!
//! Everything the block needs is self-contained (no imports, pure array/string
//! ops, no `Date.now` / `Math.random`). The `export { … }` at the bottom lives
//! OUTSIDE the markers so it is never copied into the workflow script (whose only
//! permitted export is `meta`). The verify harness imports this module and unit-
//! tests the pure logic with fabricated args/phase arrays — zero LLM calls.

// >>> document-core:begin <<<
// Pure, deterministic decision logic for the document workflow.
//
// This block is the single source of truth in
// .claude/workflows/lib/document.mjs and is copied BYTE-IDENTICAL into
// .claude/workflows/rdm-wf-document.js (the Workflow runtime cannot load modules at run
// time). scripts/verify-workflow-document.sh gates the two copies for drift.
// No Date.now / Math.random — pure array/string ops only.

// parseDocumentArgs(args) — coerce and default the whole args payload.
//
// The Workflow tool contract forbids stringified args, but LLM callers (the
// rdm-document skill shim, or a hand-run invocation) may still deliver a JSON
// string; coerce once, mirroring parseDispatchArgs in lib/dispatch-phase.mjs.
function parseDocumentArgs(args) {
  let documentArgs = args || {};
  if (typeof documentArgs === 'string') {
    try {
      documentArgs = JSON.parse(documentArgs) || {};
    } catch (e) {
      documentArgs = {};
    }
  }
  if (!documentArgs || typeof documentArgs !== 'object') documentArgs = {};
  return {
    roadmap: documentArgs.roadmap || '',
    out: documentArgs.out || '',
  };
}

// defaultOutPath(slug) — the default write location when no --out is given.
function defaultOutPath(slug) {
  return 'docs/' + slug + '.md';
}

// resolveOutPath(args) — an explicit `out` always wins over the default.
function resolveOutPath(args) {
  const a = args || {};
  return a.out || defaultOutPath(a.roadmap);
}

// computeIncompletePhases(phases) — every phase whose status is not `done`.
// A roadmap with zero phases is vacuously all-done (returns []), so an empty
// roadmap proceeds to a (contentless) draft rather than short-circuiting —
// this is a deliberate, documented choice (see rdm-wf-document.js's driver), not an
// oversight.
function computeIncompletePhases(phases) {
  const list = Array.isArray(phases) ? phases : [];
  return list.filter((p) => !p || p.status !== 'done');
}

// buildGitRangeCommands(sha) — the git commands a per-phase gather agent runs
// to collect what actually shipped for one phase's commit SHA. Mirrors the
// single-commit range convention: a phase is completed by exactly one commit,
// so the range is always `<sha>~1..<sha>` (degenerates identically whether the
// roadmap has one phase or many, since this operates per-phase, not
// per-roadmap-range). `hasSha` is false for any non-string or empty SHA, which
// is also the fallback-to-body-only signal the gather prompt keys off.
function buildGitRangeCommands(sha) {
  if (typeof sha !== 'string' || sha === '') {
    return { hasSha: false, log: null, diffStat: null };
  }
  return {
    hasSha: true,
    log: 'git log --oneline ' + sha + '~1..' + sha,
    diffStat: 'git diff --stat ' + sha + '~1..' + sha,
  };
}
// >>> document-core:end <<<

// Node-only exports for the verify harness. NOT part of the copied block — the
// marker END is above this line, so a copy never carries these.
export { parseDocumentArgs, defaultOutPath, resolveOutPath, computeIncompletePhases, buildGitRangeCommands };
