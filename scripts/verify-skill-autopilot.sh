#!/bin/sh
# Hermetic regression for the prose `rdm-autopilot` skill
# (.claude/skills/rdm-autopilot/SKILL.md), which replaced the JS
# `.claude/workflows/autopilot.js` / `.claude/workflows/lib/autopilot.mjs` loop
# (workflow-orchestration roadmap, phase 3). This file is modeled directly on
# scripts/verify-workflow-do-auto.sh — the closest existing precedent for
# gating a prose skill against a real binary rather than a Node-injected fake.
#
# *** Coverage-decision classification (written BEFORE the JS loop was deleted,
# per the phase's own "coverage decision before the deletion" instruction) ***
#
# The retired scripts/verify-workflow-autopilot.sh drove `lib/autopilot.mjs`'s
# pure functions under injected fakes in Node. A prose skill has no such
# module to import, so none of that machinery ports mechanically. Every
# section of the old harness is classified below as PORTABLE (a) — the
# promised behavior still exists and can be pinned as static text in SKILL.md,
# or as an exact rdm command shape driven against the real binary — E2E-ONLY
# (b) — same, but only checkable by actually running the documented commands,
# not by grepping text — or MOOT (c) — the assertion was about JS-only
# mechanics with no prose equivalent.
#
#   1. BEHAVIOR (old, Node/fakes) — MOSTLY MOOT, PARTLY PORTABLE.
#      - MOOT: buildMechanicalModelPrompt and other prompt-string builders,
#        describeRaw's truncation/cyclic-payload handling, and the triple-unwrap
#        defense in interpretNext for a JSON-string-re-encoded `result` field —
#        none of these functions exist once there's no schema-constrained
#        subagent to hand a prompt string to or receive a re-encoded payload
#        from. SKILL.md's own "Removed / changed" section states this
#        explicitly for the triple-unwrap defense and the `next` hoist.
#      - PORTABLE (static text): the malformed-vs-well-formed distinction
#        ("never treat a malformed payload as nothing" -> `unparseable`), the
#        known-good stop-reason allowlist (`nothing`, `blocked-on-dependencies`,
#        `budget`, `plan-only-exhausted`, `mechanical-model-unresolved`) vs. the
#        abnormal-termination banner for anything else, the summary template
#        shape, the `[fetch]`-tagged summary-only note, and the four guardrails
#        (single roadmap, no `main` mutation, no `Done:` trailer construction,
#        `--permission-mode auto`). All pinned in section 1 below.
#      - PORTABLE (real command shape): the advance/park write+read-back
#        contract's command shapes are exact `rdm phase update`/`rdm phase
#        show` invocations, driven for real in section 2 below (this is where
#        an old Node fake becomes a real binary call instead).
#
#   1b. DRIVEN LOOP (old, buildAutopilot + fakes) — MOSTLY MOOT, PARTLY
#      PORTABLE/E2E-ONLY.
#      - MOOT: mechanical-model threading into five dependency-injected
#        functions (fetchNext/estimateList/estimateWriteback/advance/park) and
#        the advance-null-as-failure / park-null-still-summarizing tests — all
#        of these existed because a mechanical subagent could return null/a
#        garbage ack; a direct Bash command instead returns a real process exit
#        code, so there is no "null ack" class of failure left to model.
#        Likewise the double/garbage-wrapped `fetch:next` payload tests (no
#        intermediate agent re-encodes JSON as a string anymore).
#      - PORTABLE (static text): drive-to-reviewed / rework-retry-once-then-park
#        / escalated / budget-stop / estimate-pre-pass-always-runs /
#        `--plan-only` dedup-via-in-context-set are all still real POLICY this
#        skill promises in prose — pinned as literal text assertions in
#        section 1.
#      - E2E-ONLY: the actual advance/park write, and its read-back retry
#        contract, is real and driven against the binary in section 2 — this
#        is the one piece of 1b that survives as an executable check rather
#        than a text check.
#
#   1c. HOIST self-tests — MOOT. These proved the OLD assertions weren't
#      vacuous; the assertions themselves are gone (see 1/1b), so their
#      self-tests have nothing left to guard.
#
#   2. BLOCK DRIFT (lib vs. workflow byte-identity) — MOOT. There is no
#      `lib/autopilot.mjs` and no stamped copy anymore; the skill is the one
#      and only source of the loop's prose.
#
#   3. STATIC INVARIANTS (JS greps: one nested `workflow()` dispatch call, no
#      import/require, both markers, no land/merge/main-mutation string,
#      no *_SCHEMA with a top-level `type:'array'`, meta.phases parity) —
#      MOOT, JS-runtime/JS-schema-specific. The two invariants with a real
#      prose equivalent — "the loop never touches `main`" and "the loop never
#      hand-builds a `Done:` trailer" — are re-pinned as literal-text
#      assertions in section 1 (they are exactly guardrails 2 and 3 in
#      SKILL.md's own "four guardrails" block).
#
#   3b. AC-MODEL (every agent() call in the five mechanical deps carries an
#      explicit model:) — MOOT. There are no agent() calls left in the loop
#      itself; the only remaining Workflow calls (`rdm-wf-estimate`, `rdm-wf-dispatch-phase`)
#      are unchanged callers already covered by their own harnesses.
#
#   4. MODULE PARSE (autopilot.js loads under module semantics) — MOOT. There
#      is no JS file to parse.
#
#   5. SIBLING GATE (verify-workflow-dispatch.sh stays green) — PORTABLE,
#      unchanged in spirit: the prose loop nests exactly the `rdm-wf-dispatch-phase`
#      and `rdm-wf-estimate` Workflows and no others, so both of their harnesses
#      staying green is still the right regression signal. Re-run in section 3
#      below (now naming both siblings, since `rdm-wf-estimate` is a genuinely new
#      call path per SKILL.md step 3).
#
#   6. LAND-TIME COMPLETION TRAILER — PORTABLE, workflow-agnostic; this section
#      never touched autopilot.js/mjs at all (rdm-land / `rdm hook done-line`
#      own it). Copied verbatim into section 4 below so this coverage is not
#      lost in the migration.
#
# *** Known, permanent coverage gap (see also SKILL.md's own note) ***
# The loop's POLICY decisions — the shared budget counter counting a rework
# re-dispatch of the SAME phase against `--max-phases`, rework-retry-exactly-
# once, `--plan-only` exhaustion dedup via an in-context set, and the estimate
# pre-pass actually firing on every run — are now made by an LLM reasoning in
# prose, not by callable JS. No deterministic harness (this one included) can
# drive that reasoning with injected fakes and assert on its branches the way
# scripts/verify-workflow-autopilot.sh's section 1b used to. This is an
# accepted, permanent trade-off of the prose migration (recorded in the
# roadmap phase body and in `docs/workflow-vs-prose-boundary.md`), not an
# oversight this harness fails to close.
#
# This harness covers what's left:
#
#   1. STATIC INVARIANTS   — grep-based assertions on SKILL.md: frontmatter,
#                             arg names/defaults, the two named Workflow calls,
#                             the four guardrails, the known-good stop-reason
#                             allowlist (with a planted-mutation self-test), the
#                             summary template, and the malformed-payload /
#                             rework-retry-once / [fetch]-tagged-note rules.
#   2. DYNAMIC OUTCOME CONTRACT — against the real binary in a hermetic temp
#                             plan repo: the exact `rdm phase update`/`rdm
#                             phase show --format json` command shapes SKILL.md
#                             documents for the advance and park steps land the
#                             status (+ blocked_reason) it promises, confirmed
#                             by a read-back.
#   3. SIBLING GATE         — verify-workflow-dispatch.sh and
#                             verify-workflow-estimate.sh (the two Workflows
#                             this skill nests) stay green.
#   4. LAND-TIME TRAILER    — copied verbatim from the old harness's section 6:
#                             a trailer-less autopilot-shaped branch commit
#                             gains its completion trailer from `rdm hook
#                             done-line` + `git commit --amend`, with no
#                             rebase, and `rdm hook post-commit` then completes
#                             the item.
#
# Requires: a cargo-built rdm at target/debug/rdm (from this repo).

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
RDM_BIN="$REPO_ROOT/target/debug/rdm"
SKILL="$REPO_ROOT/.claude/skills/rdm-autopilot/SKILL.md"

# Clear rdm-related env vars inherited from the caller's shell for hermeticity.
unset RDM_ROOT RDM_PROJECT RDM_STAGE RDM_FORMAT RDM_PLAN_REPO RDM_PLAN_REPO_TOKEN RDM_PLAN_REPO_PATH 2>/dev/null || true

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
pass() { printf '\033[1;32m[ok]\033[0m %s\n' "$*"; }

[ -x "$RDM_BIN" ] || fail "$RDM_BIN not found or not executable — run 'cargo build' first."
[ -f "$SKILL" ] || fail "skill file not found: $SKILL"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

# --- 1. STATIC INVARIANTS -----------------------------------------------------
say "1. Static invariants on .claude/skills/rdm-autopilot/SKILL.md"

# Frontmatter lists Bash and Workflow.
awk '/^---$/{n++; next} n==1' "$SKILL" >"$TMP/frontmatter"
grep -q -- '- Bash' "$TMP/frontmatter" || fail "SKILL.md frontmatter must list '- Bash' in allowed-tools"
grep -q -- '- Workflow' "$TMP/frontmatter" || fail "SKILL.md frontmatter must list '- Workflow' in allowed-tools"
pass "frontmatter lists Bash and Workflow"

# The five arg names and their documented default/omitted semantics.
grep -qF 'roadmap' "$SKILL" || fail "missing 'roadmap' arg"
grep -qF -- '--max-phases' "$SKILL" || fail "missing '--max-phases' arg"
grep -qF -- '--plan-only' "$SKILL" || fail "missing '--plan-only' arg"
grep -qF -- '--max-plan-revise' "$SKILL" || fail "missing '--max-plan-revise' arg"
grep -qF -- '--max-code-rework' "$SKILL" || fail "missing '--max-code-rework' arg"
grep -qF 'required roadmap slug' "$SKILL" || fail "SKILL.md must state the roadmap slug is required"
grep -qi 'unbounded by phase count' "$SKILL" || fail "SKILL.md must state --max-phases is omitted -> unbounded"
pass "all five arg names present, with required/omitted semantics documented"

# Exactly one dispatch-phase Workflow invocation and one estimate Workflow
# invocation, named as such.
# The backticks below are literal grep-pattern characters (SKILL.md quotes
# Workflow names in backticks), not shell command substitution.
# shellcheck disable=SC2016
grep -qF '**`rdm-wf-dispatch-phase`' "$SKILL" || fail "SKILL.md must name the 'dispatch-phase' Workflow"
# shellcheck disable=SC2016
grep -qF '**`rdm-wf-estimate`' "$SKILL" || fail "SKILL.md must name the 'estimate' Workflow"
pass "exactly one named dispatch-phase Workflow call and one named estimate Workflow call"

# --- 3a. THE rdm-wf- RENAME'S TWO HALVES -------------------------------------
# This is the single highest-risk file in the `rdm-wf-` engine rename: it is the
# one place where a token that MUST change (the engine names it invokes) and a
# token that MUST NOT (its own `rdm-autopilot` skill name) differ only by
# prefix. A sweep that gets one half and not the other is the expected failure
# mode, so both halves are asserted, each with its own planted-mutation
# self-test.
say "3a. rdm-wf- rename: engine invocations renamed, the skill's own name untouched"

# Half (a): every Workflow invocation names the PREFIXED engine, and no bold
# invocation names a bare engine.
# shellcheck disable=SC2016
if grep -qE '\*\*`(dispatch-phase|estimate)`' "$SKILL"; then
    fail "3a(a): SKILL.md still names a BARE engine in a bold Workflow invocation — the rename is half-completed"
fi
pass "3a(a): no bold invocation names a bare engine"

# Half (b): the skill's own identity is untouched.
[ -d "$REPO_ROOT/.claude/skills/rdm-autopilot" ] ||
    fail "3a(b): .claude/skills/rdm-autopilot/ no longer exists — the front-door skill was renamed, which this phase forbids"
grep -q '^name: rdm-autopilot$' "$SKILL" ||
    fail "3a(b): SKILL.md frontmatter no longer declares 'name: rdm-autopilot'"
NS=rdm-
[ "$(grep -c "$NS$NS" "$SKILL" || true)" -eq 0 ] ||
    fail "3a(b): SKILL.md contains a double-prefixed name — an unanchored substitution corrupted the skill name"
pass "3a(b): the skill directory, frontmatter name and every rdm-autopilot token are unchanged"

# Self-test for half (a): revert one invocation to its bare form and confirm
# the half-(a) check turns red.
AP_MUT_A="$TMP/autopilot-half-a.md"
# shellcheck disable=SC2016  # backticks are literal SKILL.md text, not substitution
sed 's/\*\*`rdm-wf-dispatch-phase`/**`dispatch-phase`/' "$SKILL" >"$AP_MUT_A"
# shellcheck disable=SC2016
grep -qF '**`dispatch-phase`' "$AP_MUT_A" ||
    fail "3a self-test A: could not plant the bare invocation name — the self-test is vacuous"
# shellcheck disable=SC2016
grep -qE '\*\*`(dispatch-phase|estimate)`' "$AP_MUT_A" ||
    fail "3a self-test A: a planted BARE engine invocation did NOT turn half (a) red"
pass "3a self-test A: a planted bare engine invocation correctly turns half (a) red"

# Self-test for half (b): rename the skill in a scratch tree and confirm the
# half-(b) checks turn red.
AP_MUT_B_DIR="$TMP/autopilot-half-b/.claude/skills/rdm-wf-autopilot"
mkdir -p "$AP_MUT_B_DIR"
sed 's/^name: rdm-autopilot$/name: rdm-wf-autopilot/' "$SKILL" >"$AP_MUT_B_DIR/SKILL.md"
[ -d "$TMP/autopilot-half-b/.claude/skills/rdm-autopilot" ] &&
    fail "3a self-test B: the scratch tree still has an rdm-autopilot directory — the self-test is vacuous"
grep -q '^name: rdm-autopilot$' "$AP_MUT_B_DIR/SKILL.md" &&
    fail "3a self-test B: a renamed frontmatter name did NOT turn half (b) red"
pass "3a self-test B: a renamed skill directory and frontmatter name correctly turn half (b) red"

# PER-CALLER rdmBin/project assertion (project-agnostic-lane, phase 10).
# dispatch-phase's `rdmBin` arg now DEFAULTS to a plain `rdm` on PATH, so a
# caller that passes none no longer throws — it silently runs whichever global
# rdm is first on PATH, which inside this repo is the stale build the
# development-build rule forbids. The assertion therefore survives the contract
# reversal unchanged; only the consequence it guards did. This skill IS such a
# caller, so its
# dispatch-phase invocation line must carry both keys. Phase 10 de-literalized
# the local loop entirely, so the payload now carries BARE `rdmBin`/`project`
# variable names (resolved from this skill's own `--rdm-bin`/`--project` args
# in step 1), never a literal path/project string — unlike the shipped
# templates below, which use the same bare-key style already. Line-scoped (not
# a whole-file grep), so a mention elsewhere cannot satisfy it.
assert_autopilot_dispatch_rdmbin() {
    grep -F 'dispatch-phase` Workflow**' "$1" >"$TMP/ap-dispatch-line" 2>/dev/null || return 1
    [ -s "$TMP/ap-dispatch-line" ] || return 1
    grep -qF 'rdmBin' "$TMP/ap-dispatch-line" || return 1
    grep -qF 'project' "$TMP/ap-dispatch-line" || return 1
    return 0
}
assert_autopilot_dispatch_rdmbin "$SKILL" ||
    fail "the dispatch-phase invocation line must pass bare rdmBin/project keys — omitting rdmBin silently falls back to a PATH-resolved rdm, which is the wrong binary in this repo"
pass "the dispatch-phase invocation line passes rdmBin and project"

# Self-test: prove the assertion is not vacuous.
sed 's/rdmBin/rdmBn/g' "$SKILL" >"$TMP/ap-rdmbin-mutant.md"
if assert_autopilot_dispatch_rdmbin "$TMP/ap-rdmbin-mutant.md"; then
    fail "the autopilot rdmBin detector missed a mangled key — the check is vacuous"
fi
pass "autopilot rdmBin detector fires on a mangled key"

# NEW estimate-payload assertion (project-agnostic-lane, phase 10): the
# `rdm-wf-estimate` Workflow invocation line must ALSO carry both rdmBin and project —
# rdm-wf-estimate.js's own parseEstimateArgs resolves them via
# resolveRdmBin/parseProjectArg, same defaulting contract as dispatch-phase.
# Line-scoped, mirroring the dispatch-phase assertion above.
assert_autopilot_estimate_rdmbin() {
    grep -F 'estimate` Workflow**' "$1" >"$TMP/ap-estimate-line" 2>/dev/null || return 1
    [ -s "$TMP/ap-estimate-line" ] || return 1
    grep -qF 'rdmBin' "$TMP/ap-estimate-line" || return 1
    grep -qF 'project' "$TMP/ap-estimate-line" || return 1
    return 0
}
assert_autopilot_estimate_rdmbin "$SKILL" ||
    fail "the estimate invocation line must pass rdmBin and project — omitting rdmBin silently falls back to a PATH-resolved rdm, which is the wrong binary in this repo"
pass "the estimate invocation line passes rdmBin and project"

# Self-test: prove the estimate assertion is not vacuous.
sed 's/rdmBin/rdmBn/g' "$SKILL" >"$TMP/ap-estimate-mutant.md"
if assert_autopilot_estimate_rdmbin "$TMP/ap-estimate-mutant.md"; then
    fail "the autopilot estimate rdmBin detector missed a mangled key — the check is vacuous"
fi
pass "autopilot estimate rdmBin detector fires on a mangled key"

# NO PRE-FLIGHT STOP ON A MISSING --rdm-bin (plugin-distribution, phase 6).
# `rdmBin` is optional now and defaults to a plain `rdm` on PATH, so the loop
# must NOT refuse to start when `--rdm-bin` is absent. The missing-ROADMAP-SLUG
# stop is unrelated and must survive — this pairs a negative with a positive so
# a blanket deletion of the whole pre-flight paragraph cannot go green.
assert_no_rdm_bin_preflight_stop() {
    grep -inE '(--)?rdm-?bin' "$1" |
        grep -iE 'missing.*stop|stop.*missing|Missing +→' >"$TMP/ap-preflight-hits" 2>/dev/null || true
    [ ! -s "$TMP/ap-preflight-hits" ]
}
if ! assert_no_rdm_bin_preflight_stop "$SKILL"; then
    cat "$TMP/ap-preflight-hits" >&2
    fail "SKILL.md still refuses to start on a missing --rdm-bin — the arg is optional now and defaults to a plain 'rdm'"
fi
grep -qF 'required roadmap slug' "$SKILL" ||
    fail "the missing-ROADMAP-SLUG pre-flight stop must SURVIVE — only the --rdm-bin stop was retired"
pass "no missing---rdm-bin pre-flight stop remains, and the roadmap-slug stop survives"

# Self-test: re-insert the retired stop and the detector must fire.
cp "$SKILL" "$TMP/ap-preflight-mutant.md"
# shellcheck disable=SC2016  # literal prose, deliberately unexpanded
printf '\nIf `--rdm-bin` is missing, stop before invoking anything and say so.\n' >>"$TMP/ap-preflight-mutant.md"
if cmp -s "$SKILL" "$TMP/ap-preflight-mutant.md"; then
    fail "the pre-flight-stop mutation did not apply — the self-test is not exercising anything"
fi
if assert_no_rdm_bin_preflight_stop "$TMP/ap-preflight-mutant.md"; then
    fail "the pre-flight-stop detector missed a re-inserted missing---rdm-bin stop — the gate is vacuous"
fi
pass "the pre-flight-stop detector fires on a re-inserted missing---rdm-bin stop"

# ZERO-LITERAL assertion (project-agnostic-lane, phase 10): the local drive
# loop's own Bash steps (`rdm next`, advance/park, model resolve, phase list,
# the printed-summary pointer) are now fully de-literalized behind
# `<rdmBin>`/`<proj-flag>` placeholders resolved from this skill's own
# `--rdm-bin`/`--project` args. An unmodified pre-phase-10 file carried 7 and 5
# occurrences respectively, so this is non-vacuous. This REPLACES phase 4's
# "must still carry its own literals" split-bound guard, which is now the
# opposite of what this file must do.
LITERAL_BIN_COUNT=$(grep -c 'target/debug/rdm' "$SKILL" || true)
[ "$LITERAL_BIN_COUNT" -eq 0 ] ||
    fail "SKILL.md must carry ZERO 'target/debug/rdm' literals — the drive loop must be fully de-literalized, found $LITERAL_BIN_COUNT"
LITERAL_PROJECT_COUNT=$(grep -c -- '--project rdm' "$SKILL" || true)
[ "$LITERAL_PROJECT_COUNT" -eq 0 ] ||
    fail "SKILL.md must carry ZERO '--project rdm' literals — the drive loop must be fully de-literalized, found $LITERAL_PROJECT_COUNT"
pass "the prose drive loop carries zero hardcoded binary/project literals"

# ALLOW-LIST assertion (project-agnostic-lane, phase 10): `model resolve`
# carries no project flag; `next` / `phase update` / `phase show` all do.
grep -F '<rdmBin> model resolve mechanical' "$SKILL" >"$TMP/ap-model-resolve-line" 2>/dev/null ||
    fail "missing the '<rdmBin> model resolve mechanical' line"
grep -qF '<proj-flag>' "$TMP/ap-model-resolve-line" &&
    fail "the model-resolve line must NOT carry <proj-flag> — it is allow-listed as project-free"
pass "model resolve carries no <proj-flag> (allow-listed)"

# Every matching LINE (not just "some" line in the file) must carry
# <proj-flag> — a per-line check, since phase update/show appear on more than
# one line.
assert_every_line_has_proj_flag() {
    file="$1"
    pattern="$2"
    grep -F "$pattern" "$file" >"$TMP/ap-allowlist-lines" 2>/dev/null || return 1
    [ -s "$TMP/ap-allowlist-lines" ] || return 1
    while IFS= read -r line; do
        printf '%s' "$line" | grep -qF '<proj-flag>' || return 1
    done <"$TMP/ap-allowlist-lines"
    return 0
}
for pattern in '<rdmBin> next' '<rdmBin> phase update' '<rdmBin> phase show'; do
    assert_every_line_has_proj_flag "$SKILL" "$pattern" ||
        fail "every '$pattern' line must carry <proj-flag>, found at least one line without it (or no matching line at all)"
done
pass "next/phase update/phase show all carry <proj-flag> on every occurrence"

# Self-test: prove the allow-list assertions are not vacuous — strip every
# <proj-flag> occurrence in a scratch copy (the phase-update/phase-show line
# carries TWO on one line, so a partial mutation could still pass vacuously —
# strip them all) and confirm detection on the phase-update pattern.
sed 's/<proj-flag>//g' "$SKILL" >"$TMP/ap-allowlist-mutant.md"
if assert_every_line_has_proj_flag "$TMP/ap-allowlist-mutant.md" '<rdmBin> phase update'; then
    fail "the allow-list self-test mutation did not remove <proj-flag> from the phase-update line — self-test is broken"
fi
pass "allow-list detector correctly rejects a scratch copy missing <proj-flag> on phase update (self-test)"

# Self-test the other direction: mutate model-resolve to ADD <proj-flag> and
# confirm the model-resolve check (above) would now fail on it too.
sed 's/<rdmBin> model resolve mechanical/<rdmBin> model resolve mechanical<proj-flag>/' \
    "$SKILL" >"$TMP/ap-model-resolve-mutant.md"
grep -F '<rdmBin> model resolve mechanical' "$TMP/ap-model-resolve-mutant.md" >"$TMP/ap-model-resolve-mutant-line"
if grep -qF '<proj-flag>' "$TMP/ap-model-resolve-mutant-line"; then
    pass "model-resolve allow-list self-test: a planted <proj-flag> is detectable (would fail the real check)"
else
    fail "the model-resolve self-test mutation did not add <proj-flag> — self-test is broken"
fi

# The four guardrails, present as literal text.
grep -qF 'Single roadmap' "$SKILL" || fail "missing guardrail 1: single roadmap"
grep -qF 'same fixed' "$SKILL" || fail "guardrail 1 must state every command uses the SAME fixed --roadmap"
# shellcheck disable=SC2016
grep -qi '`main` is never touched' "$SKILL" || fail "missing guardrail 2: main is never touched"
# shellcheck disable=SC2016
grep -qF 'No `Done:` trailer' "$SKILL" || fail "missing guardrail 3: no Done: trailer"
grep -qF -- '--permission-mode auto' "$SKILL" || fail "missing guardrail 4: --permission-mode auto"
pass "all four guardrails (single-roadmap, no-main-mutation, no-Done:-trailer, --permission-mode auto) present"

# Known-good stop-reason allowlist: exactly these five, no more, no fewer.
EXPECTED_REASONS="nothing blocked-on-dependencies budget plan-only-exhausted mechanical-model-unresolved"
grep -qF 'allowlist** is exactly' "$SKILL" || fail "SKILL.md must state the known-good stop-reason allowlist"
# Scope to just the clause between "is exactly:" and the following period, so
# the OTHER backtick-quoted tokens on the same line (`stop reason: <reason>`,
# `unparseable`, etc.) don't inflate the count.
ALLOWLIST_CLAUSE=$(grep -F 'allowlist** is exactly' "$SKILL" | sed -E 's/^.*allowlist\*\* is exactly: ([^.]*)\..*$/\1/')
[ -n "$ALLOWLIST_CLAUSE" ] || fail "could not extract the allowlist clause from SKILL.md"
for r in $EXPECTED_REASONS; do
    printf '%s' "$ALLOWLIST_CLAUSE" | grep -qF "\`$r\`" ||
        fail "known-good allowlist clause is missing stop reason: $r"
done
# shellcheck disable=SC2016
FOUND_COUNT=$(printf '%s' "$ALLOWLIST_CLAUSE" | grep -oE '`[a-z-]+`' | sort -u | wc -l | tr -d ' ')
[ "$FOUND_COUNT" -eq 5 ] || fail "known-good allowlist clause must name exactly 5 backtick-quoted reasons, found $FOUND_COUNT: $ALLOWLIST_CLAUSE"
pass "known-good stop-reason allowlist is exactly the 5 expected reasons"

# Self-test: prove the allowlist detector is not a tautological no-op — a
# scratch copy with one reason stripped from the clause must fail the same
# check this script just ran.
# shellcheck disable=SC2016
printf '%s\n' "$ALLOWLIST_CLAUSE" | sed 's/, `plan-only-exhausted`//' >"$TMP/allowlist.scratch"
# shellcheck disable=SC2016
SCRATCH_COUNT=$(grep -oE '`[a-z-]+`' "$TMP/allowlist.scratch" | sort -u | wc -l | tr -d ' ')
[ "$SCRATCH_COUNT" -eq 5 ] && fail "allowlist self-test broken — the scratch mutation did not remove a reason"
pass "allowlist detector correctly rejects a scratch copy missing one reason (self-test)"

# The abnormal-termination banner for anything outside the allowlist.
grep -qF 'ABNORMAL TERMINATION' "$SKILL" || fail "missing the abnormal-termination banner text"
grep -qi 'allowlist, not a denylist' "$SKILL" || fail "SKILL.md must state the allowlist (not denylist) framing"
pass "abnormal-termination banner and allowlist-not-denylist framing present"

# Malformed-vs-well-formed distinction: never fold a malformed payload into 'nothing'.
grep -qi 'never.*treat a malformed' "$SKILL" || fail "missing 'never treat a malformed payload as nothing' rule"
grep -qF 'unparseable' "$SKILL" || fail "missing the 'unparseable' stop reason"
pass "'malformed next payload -> unparseable, never nothing' rule present"

# Rework retry-once-then-park.
grep -qF 'DEFAULT_MAX_REWORK = 1' "$SKILL" || fail "missing literal 'DEFAULT_MAX_REWORK = 1' constant statement"
grep -qF 'DEFAULT_GLOBAL_BUDGET = 50' "$SKILL" || fail "missing literal 'DEFAULT_GLOBAL_BUDGET = 50' constant statement"
grep -qi 'rework budget exhausted' "$SKILL" || fail "missing the rework-budget-exhausted park reason"
pass "rework-retry-once-then-park rule present, with both budget constants stated literally"

# The [fetch]-tagged summary-only note.
grep -qF '[fetch]' "$SKILL" || fail "missing the [fetch] escalation tag"
grep -qF 'summary-only' "$SKILL" || fail "missing the [fetch]-tagged summary-only note"
grep -qF 'will NOT appear in the' "$SKILL" || fail "missing the 'will not appear in rdm review blocked' clause"
pass "[fetch]-tagged summary-only note present"

# The summary template shape.
grep -qF 'autopilot summary for roadmap/<slug>' "$SKILL" || fail "missing the summary template header line"
grep -qF 'phases completed (' "$SKILL" || fail "missing the 'phases completed (<n>)' summary line"
grep -qF 'escalations awaiting review (' "$SKILL" || fail "missing the 'escalations awaiting review (<n>)' summary line"
grep -qF 'reviewed work is left on the roadmap/<slug> branch; main is never touched.' "$SKILL" ||
    fail "missing the summary's closing 'main is never touched' line"
pass "summary template block present verbatim"

# escalated -> park mapping (one of the six loop policies the header claims
# are pinned as literal text in this section).
grep -qF 'outcome: "escalated"' "$SKILL" || fail "missing the outcome: \"escalated\" branch"
grep -qF '**park**' "$SKILL" || fail "missing the escalated -> park mapping"
pass "outcome: \"escalated\" -> park mapping present"

# estimate pre-pass always runs (unconditional, not gated on anything else).
grep -qF 'estimate pre-pass' "$SKILL" || fail "missing the estimate pre-pass section"
grep -qF 'one Workflow call, always' "$SKILL" || fail "missing the 'one Workflow call, always' unconditional framing"
pass "estimate pre-pass documented as unconditional (one Workflow call, always)"

# --plan-only dedup via an in-context set (planOnlySeen).
grep -qF 'planOnlySeen' "$SKILL" || fail "missing the planOnlySeen in-context dedup set"
grep -qF 'plan-only-exhausted' "$SKILL" || fail "missing the plan-only-exhausted stop reason"
pass "--plan-only dedup-via-in-context-set (planOnlySeen) present"

# --- 1b. SHIPPED skill-autopilot-{cli,mcp}.md static invariants ---------------
say "1b. Static invariants on the shipped skill-autopilot-{cli,mcp}.md templates"

# These are a SEPARATE surface from the local dogfood skill above: they were
# already fully threaded by phase 4's one-line carve-out (git diff main shows
# exactly one changed line per file), invoke dispatch-phase only, and never
# invoke estimate. Written as its own function/loop rather than reusing
# assert_autopilot_dispatch_rdmbin, per the plan's explicit "not copy-pasted"
# requirement.
SHIPPED_TEMPLATES="$REPO_ROOT/rdm-core/src/templates/skill-autopilot-cli.md $REPO_ROOT/rdm-core/src/templates/skill-autopilot-mcp.md"

assert_shipped_dispatch_line_has_keys() {
    file="$1"
    grep -F 'dispatch-phase` Workflow**' "$file" >"$TMP/shipped-dispatch-line" 2>/dev/null || return 1
    [ -s "$TMP/shipped-dispatch-line" ] || return 1
    grep -qF 'rdmBin' "$TMP/shipped-dispatch-line" || return 1
    grep -qF 'project' "$TMP/shipped-dispatch-line" || return 1
    return 0
}

for f in $SHIPPED_TEMPLATES; do
    [ -f "$f" ] || fail "shipped template not found: $f"
    assert_shipped_dispatch_line_has_keys "$f" ||
        fail "$f: the dispatch-phase invocation line must carry both rdmBin and project"
    pass "$(basename "$f"): dispatch-phase invocation line carries rdmBin and project"
done

# Self-test: prove the shipped-template assertion is not vacuous.
for f in $SHIPPED_TEMPLATES; do
    sed 's/rdmBin/rdmBn/g' "$f" >"$TMP/shipped-mutant.md"
    if assert_shipped_dispatch_line_has_keys "$TMP/shipped-mutant.md"; then
        fail "$(basename "$f"): the shipped-template rdmBin detector missed a mangled key — the check is vacuous"
    fi
done
pass "shipped-template rdmBin detector fires on a mangled key (both files)"

# NEGATIVE: neither shipped file invokes the `rdm-wf-estimate` Workflow. Scoped to the
# exact invocation phrase the LOCAL skill uses to name its real call, not the
# bare word "estimate" — both shipped files legitimately discuss "estimate" in
# their "Why no estimate pre-pass here" explanatory paragraph, and a bare
# substring-absence assertion would fail against correct, unmodified input.
for f in $SHIPPED_TEMPLATES; do
    # shellcheck disable=SC2016
    if grep -qF '**`rdm-wf-estimate`' "$f"; then
        fail "$(basename "$f"): must not invoke the estimate Workflow — this is a deliberate, permanent divergence from the local dogfood skill"
    fi
done
pass "neither shipped template invokes the estimate Workflow (scoped to the invocation phrase)"

# Self-test: prove the estimate-phrase check is non-vacuous — plant the
# invocation phrase into a scratch copy and confirm detection.
for f in $SHIPPED_TEMPLATES; do
    # shellcheck disable=SC2016
    printf '\ninvoke the **`rdm-wf-estimate` Workflow** via the Workflow tool\n' >>"$TMP/estimate-mutant.md"
    cat "$f" "$TMP/estimate-mutant.md" >"$TMP/estimate-mutant-full.md"
    # shellcheck disable=SC2016
    if ! grep -qF '**`rdm-wf-estimate`' "$TMP/estimate-mutant-full.md"; then
        fail "$(basename "$f"): estimate-phrase self-test mutation did not plant the phrase — self-test is broken"
    fi
    rm -f "$TMP/estimate-mutant.md" "$TMP/estimate-mutant-full.md"
done
pass "estimate-phrase detector fires when the phrase is planted (self-test, both files)"

# NEGATIVE: no `agentType` reference on the dispatch-phase payload/invocation
# line specifically (the same line the rdmBin/project check above greps).
# Scoped, not a whole-file substring search: both shipped files ALREADY,
# legitimately, contain the bare word "agentType" once each inside the
# pre-existing "Why no estimate pre-pass here" paragraph, so a whole-file
# assertion would fail against correct, unmodified input.
for f in $SHIPPED_TEMPLATES; do
    grep -F 'dispatch-phase` Workflow**' "$f" >"$TMP/shipped-dispatch-line-agentcheck" 2>/dev/null
    [ -s "$TMP/shipped-dispatch-line-agentcheck" ] || fail "$(basename "$f"): missing dispatch-phase invocation line"
    if grep -qF 'agentType' "$TMP/shipped-dispatch-line-agentcheck"; then
        fail "$(basename "$f"): the dispatch-phase payload/invocation line must not reference agentType"
    fi
done
pass "neither shipped template's dispatch-phase line references agentType (scoped check passes on unmodified input)"

# Self-test: prove the scoped agentType check fires when planted onto the
# dispatch-phase line of a scratch copy (and would NOT fire from the
# pre-existing explanatory-paragraph mention alone, already proven above).
for f in $SHIPPED_TEMPLATES; do
    awk '/dispatch-phase` Workflow\*\*/ { sub(/\}`/, ", agentType: \x27rdm-mechanical\x27 }`") } { print }' "$f" >"$TMP/shipped-agenttype-mutant.md"
    grep -F 'dispatch-phase` Workflow**' "$TMP/shipped-agenttype-mutant.md" >"$TMP/shipped-agenttype-mutant-line"
    grep -qF 'agentType' "$TMP/shipped-agenttype-mutant-line" ||
        fail "$(basename "$f"): agentType self-test mutation did not land on the dispatch-phase line — self-test is broken"
done
pass "agentType-on-dispatch-phase-line detector fires when planted (self-test, both files)"

# --- 2. DYNAMIC OUTCOME CONTRACT ----------------------------------------------
say "2. Dynamic advance/park write+read-back contract against the real binary"

# Hermetic HOME + XDG + git identity so neither rdm nor git touches the real
# developer/CI environment (matches the sibling worktree-review-loop harness).
export HOME="$TMP/home"
export XDG_CONFIG_HOME="$TMP/xdg-config"
export XDG_DATA_HOME="$TMP/xdg-data"
export XDG_STATE_HOME="$TMP/xdg-state"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"

export GIT_AUTHOR_NAME="verify-bot"
export GIT_AUTHOR_EMAIL="verify@example.invalid"
export GIT_COMMITTER_NAME="verify-bot"
export GIT_COMMITTER_EMAIL="verify@example.invalid"

PLAN="$TMP/plan"

rdm_plan() (
    RDM_ROOT="$PLAN" "$RDM_BIN" "$@"
)

say "2a. Seeding a hermetic plan repo (project 'verify') with roadmap 'rm' and 2 phases"
rdm_plan init --default-project verify >/dev/null
rdm_plan roadmap create rm --title "RM" --body "Autopilot advance/park contract regression roadmap." \
    --no-edit --project verify >/dev/null
rdm_plan phase create a --title "Phase A" --number 1 --body "Phase A." \
    --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase create b --title "Phase B" --number 2 --body "Phase B." \
    --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase update phase-1-a --status in-progress --no-edit --roadmap rm --project verify >/dev/null
rdm_plan phase update phase-2-b --status in-progress --no-edit --roadmap rm --project verify >/dev/null
rdm_plan commit -m "chore(plan): seed rm roadmap with 2 in-progress phases" >/dev/null
pass "seeded roadmap rm with phase-1-a/phase-2-b, both in-progress"

say "2b. advance -> rdm phase update --status reviewed, confirmed by a read-back"
rdm_plan phase update phase-1-a --status reviewed --no-edit --roadmap rm --project verify >/dev/null
OUT_A=$(rdm_plan phase show phase-1-a --roadmap rm --project verify --format json --no-body)
printf '%s' "$OUT_A" | grep -qF '"status": "reviewed"' || fail "phase-1-a expected status reviewed, got: $OUT_A"
pass "advance's write+read-back contract holds: --status reviewed lands and reads back"

say "2c. park -> rdm phase update --status blocked --reason '[code] rework budget exhausted', confirmed by a read-back"
rdm_plan phase update phase-2-b --status blocked --reason "[code] rework budget exhausted" \
    --no-edit --roadmap rm --project verify >/dev/null
OUT_B=$(rdm_plan phase show phase-2-b --roadmap rm --project verify --format json --no-body)
printf '%s' "$OUT_B" | grep -qF '"status": "blocked"' || fail "phase-2-b expected status blocked, got: $OUT_B"
printf '%s' "$OUT_B" | grep -qF '"blocked_reason": "[code] rework budget exhausted"' ||
    fail "phase-2-b blocked_reason must be the documented rework-exhausted park reason, got: $OUT_B"
pass "park's write+read-back contract holds: --status blocked with a [code]-tagged reason lands and reads back"

rdm_plan commit -m "chore(plan): land advance/park OUTCOME contract regression" >/dev/null

# --- 3. SIBLING GATE -----------------------------------------------------------
say "3. Sibling gate: the two Workflows this skill nests stay green"

if bash "$SCRIPT_DIR/verify-workflow-dispatch.sh" >/dev/null 2>&1; then
    pass "verify-workflow-dispatch.sh still green"
else
    bash "$SCRIPT_DIR/verify-workflow-dispatch.sh" >&2 || true
    fail "verify-workflow-dispatch.sh regressed"
fi

if bash "$SCRIPT_DIR/verify-workflow-estimate.sh" >/dev/null 2>&1; then
    pass "verify-workflow-estimate.sh still green"
else
    bash "$SCRIPT_DIR/verify-workflow-estimate.sh" >&2 || true
    fail "verify-workflow-estimate.sh regressed"
fi

# --- 4. LAND-TIME COMPLETION TRAILER -----------------------------------------
# Copied verbatim (in spirit) from the retired verify-workflow-autopilot.sh's
# section 6. Autopilot leaves a reviewed phase's branch commit WITHOUT a
# completion trailer; `rdm-land` is the land-time writer. This drives the exact
# documented rdm-land sequence against the REAL binary in a hermetic temp plan
# + source repo, and asserts the trailer arrives with NO rebase and no
# interactive step, and that the merge hook then completes the item. This
# section is workflow-agnostic and never touched autopilot.js/lib/autopilot.mjs
# even before their retirement, so it is unaffected by this migration.
say "4. Land-time completion trailer: a trailer-less autopilot branch gains it with no rebase"

SRC="$TMP/src"

rdm_plan roadmap create rm2 --title "RM2" --body "Land-time trailer regression roadmap." \
    --no-edit --project verify >/dev/null
rdm_plan phase create x --title "Phase X" --number 1 --body "Phase X." \
    --no-edit --roadmap rm2 --project verify >/dev/null
# Exactly the state autopilot leaves behind: the phase advanced to `reviewed`,
# the work committed on the roadmap branch, nothing landed.
rdm_plan phase update phase-1-x --status reviewed --no-edit --roadmap rm2 --project verify >/dev/null
rdm_plan commit -m "chore(plan): seed rm2/phase-1-x as reviewed" >/dev/null
pass "seeded hermetic plan repo: rm2/phase-1-x is reviewed"

# Source repo: a roadmap branch whose tip is the un-pushed reviewed commit with a
# message carrying NO trailer (exactly what an autopilot run leaves).
mkdir -p "$SRC"
git -C "$SRC" init -q -b main
printf 'seed\n' >"$SRC/README.md"
git -C "$SRC" add README.md
git -C "$SRC" commit -qm "chore: seed"
git -C "$SRC" checkout -q -b roadmap/rm2
printf 'work\n' >"$SRC/feature.txt"
git -C "$SRC" add feature.txt
git -C "$SRC" commit -qm "feat: implement phase X"
git -C "$SRC" log -1 --pretty=%B | grep -qF 'Done:' &&
    fail "setup is wrong: the autopilot-shaped commit must start WITHOUT a completion trailer"
pass "roadmap/rm2 tip is a reviewed, trailer-less, un-pushed commit"

# The documented rdm-land precondition-2 synthesis: ask rdm for the trailer (the
# format string has exactly one home) and amend it onto the branch tip. No
# rebase, no interactive editor.
DONE_LINE=$(rdm_plan hook done-line --roadmap rm2 --phase phase-1-x) ||
    fail "rdm hook done-line failed — the land path must abort rather than amend an empty trailer"
[ -n "$DONE_LINE" ] || fail "rdm hook done-line printed nothing"
ORIG_MSG=$(git -C "$SRC" log -1 --pretty=%B)
PRE_AMEND_BASE=$(git -C "$SRC" rev-parse HEAD~1)
printf '%s\n\n%s\n' "$ORIG_MSG" "$DONE_LINE" >"$TMP/amend-msg"
GIT_EDITOR=true git -C "$SRC" commit -q --amend -F "$TMP/amend-msg"

git -C "$SRC" log -1 --pretty=%B | grep -qF 'Done: rm2/phase-1-x' ||
    fail "the amended commit must carry 'Done: rm2/phase-1-x'; got: $(git -C "$SRC" log -1 --pretty=%B)"
pass "the branch tip now carries the completion trailer, synthesized by rdm hook done-line"

# No rebase was needed: the amend preserved the parent commit, and the branch
# still has exactly the same two-commit shape.
[ "$(git -C "$SRC" rev-parse HEAD~1)" = "$PRE_AMEND_BASE" ] ||
    fail "the amend must not have rewritten history below the tip — no rebase is permitted"
[ "$(git -C "$SRC" rev-list --count main..HEAD)" -eq 1 ] ||
    fail "roadmap/rm2 must still be exactly one commit ahead of main (no rebase, no extra commits)"
[ -z "$(git -C "$SRC" rev-parse -q --verify REBASE_HEAD 2>/dev/null || true)" ] ||
    fail "a rebase was started — the land-time trailer must need none"
[ ! -d "$SRC/.git/rebase-merge" ] && [ ! -d "$SRC/.git/rebase-apply" ] ||
    fail "a rebase directory exists — the land-time trailer must need no rebase"
pass "no rebase and no interactive step were required"

# Land it: fast-forward main, then run the merge-to-main hook. The trailer the
# lander synthesized is what flips the phase reviewed -> done.
git -C "$SRC" checkout -q main
git -C "$SRC" merge -q --ff-only roadmap/rm2
LANDED_SHA=$(git -C "$SRC" rev-parse HEAD)
(cd "$SRC" && "$RDM_BIN" --root "$PLAN" hook post-commit) ||
    fail "rdm hook post-commit failed on the landed commit"
PHASE_JSON=$(rdm_plan phase show phase-1-x --roadmap rm2 --project verify --format json --no-body)
printf '%s' "$PHASE_JSON" | grep -qF '"status": "done"' ||
    fail "the landed trailer must flip rm2/phase-1-x to done; got: $PHASE_JSON"
printf '%s' "$PHASE_JSON" | grep -qF "$LANDED_SHA" ||
    fail "the completed phase must record the landed commit SHA $LANDED_SHA; got: $PHASE_JSON"
pass "rdm hook post-commit flipped rm2/phase-1-x to done and recorded the landed SHA"

# Negative: `rdm hook done-line` rejects a malformed request, so the land path
# aborts instead of amending an empty trailer.
if rdm_plan hook done-line --roadmap rm2 >/dev/null 2>&1; then
    fail "rdm hook done-line must reject a request with neither --phase nor --task"
fi
if rdm_plan hook done-line --roadmap rm2 --phase phase-1-x --task t >/dev/null 2>&1; then
    fail "rdm hook done-line must reject both --phase and --task together"
fi
pass "rdm hook done-line rejects malformed requests, so the lander aborts rather than amending an empty trailer"

say "verify-skill-autopilot.sh: ALL GREEN"
