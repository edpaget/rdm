#!/bin/sh
# Hermetic, static gate over the `reviewed` transition gate's THREADING.
#
# The runtime behavior of the gate is covered by `cargo nextest run`, on every
# surface it is wired into rather than only the CLI: `rdm-core/tests/gate.rs`
# (the rule), `rdm-cli/tests/cli_gate.rs` (the binary),
# `rdm-server/tests/reviewed_gate.rs` (the 409 an HTTP PATCH gets), and
# `rdm-git/src/worktree.rs`'s tests (the production worktree probe). A static
# check like this one cannot see a gate that is *wired but not enforcing* — a
# wrong config key or an error variant mapped to the wrong status code would
# leave every grep below green — which is why those exist alongside it.
#
# What tests in turn cannot see is the shape of the call graph: `rdm-core`'s `update_phase`,
# `update_phase_with_estimate` and `update_task` are deliberately left
# UNCHECKED, and the gate only bites because every user-facing status-write
# surface routes through their `_gated` siblings instead. That boundary is a
# convention the compiler cannot enforce, so it is enforced here — exactly the
# way `scripts/verify-scoped-commit.sh` § D bounds the commit primitives.
#
#   A  every non-test call to an ungated update primitive is on an explicit
#      allowlist, with the status it writes recorded
#   B  every user-facing status-write surface uses a `_gated` entry
#   C  each of the gate's refusals names a remediation command
#
# Sections A and B each carry a planted-mutation self-test proving the check
# can fail, and a restatement that the real, unmutated tree passes.
#
# Run after touching `rdm-core/src/ops/gate.rs`, the gated/ungated split in
# `rdm-core/src/ops/{phase,task}.rs`, or any caller of those primitives.
# Design record: `docs/core-enforced-gates.md`.
#
# Requires: nothing but POSIX sh and grep. No cargo build, no network.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT HUP TERM

say() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\n\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2
    exit 1
}
ok() { printf '\033[1;32m[ OK ]\033[0m %s\n' "$*"; }

# ---------------------------------------------------------------------------
# Section A — every ungated-primitive caller is on the allowlist
# ---------------------------------------------------------------------------
say "Section A — ungated update-primitive callers are allowlisted"

# The sanctioned sites, one `<file>:<line-context>` per line, each with the
# status it writes. Adding a caller is a deliberate edit to this list, which is
# printed on failure so the reason is obvious.
#
#   rdm-cli/src/commands/review.rs   x2  NeedsReview  (`rdm review restamp`;
#       gating it would be inert and would put
#       scripts/verify-worktree-review-loop.sh at risk)
#   rdm-cli/src/commands/mod.rs      x2  Done         (the `Done:` post-merge /
#       post-commit hook path, which is contractually exit-0 and must never
#       acquire a failure mode)
#   rdm-core/src/ops/task.rs         x3  Done / WontFix / status:None
#       (consolidate_task_into_roadmap and merge_tasks — core-internal, and
#       none can pass `Reviewed`)
ALLOWLIST='rdm-cli/src/commands/mod.rs
rdm-cli/src/commands/review.rs
rdm-core/src/ops/task.rs'

EXPECTED_HITS=7

# scan_ungated <root> — print "<file>:<line>" for every call to an ungated
# update primitive that is neither in a `tests/` directory nor below the file's
# own `#[cfg(test)]` marker. Definitions (`fn update_phase(`) and commented-out
# lines are excluded so a mere mention is never mistaken for a call.
scan_ungated() {
    (
        cd "$1" || exit 1
        find . -name '*.rs' -not -path './target/*' -not -path '*/tests/*' |
            sed 's|^\./||' | sort
    ) | while read -r f; do
        [ -f "$1/$f" ] || continue
        cut=$(grep -n '^#\[cfg(test)\]' "$1/$f" | head -1 | cut -d: -f1)
        [ -n "$cut" ] || cut=999999
        grep -n 'update_phase(\|update_phase_with_estimate(\|update_task(' "$1/$f" |
            grep -v 'fn [a-z_]*update_\|fn update_' |
            grep -v '^[0-9]*: *//' |
            while IFS=: read -r n _; do
                if [ "$n" -lt "$cut" ]; then
                    printf '%s:%s\n' "$f" "$n"
                fi
            done
    done
}

scan_ungated "$REPO_ROOT" >"$TMP/a.hits"
[ -s "$TMP/a.hits" ] ||
    fail "the ungated-primitive grep matched nothing at all — it proves nothing"

HIT_COUNT=$(wc -l <"$TMP/a.hits" | tr -d ' ')
[ "$HIT_COUNT" -eq "$EXPECTED_HITS" ] || fail "expected exactly $EXPECTED_HITS ungated-primitive call sites, found $HIT_COUNT:
$(cat "$TMP/a.hits")

Each ungated caller must be a deliberate, documented addition: update the
allowlist AND the count above, and record the status the new site writes in
docs/core-enforced-gates.md. A site that can ever write \`reviewed\` belongs on
a \`_gated\` entry instead."

cut -d: -f1 "$TMP/a.hits" | sort -u >"$TMP/a.files"
printf '%s\n' "$ALLOWLIST" | sort -u >"$TMP/a.allow"
UNSANCTIONED=$(comm -23 "$TMP/a.files" "$TMP/a.allow")
if [ -n "$UNSANCTIONED" ]; then
    fail "unsanctioned ungated update-primitive caller(s):
$UNSANCTIONED

The sanctioned files are:
$ALLOWLIST

A user-facing status write must use update_phase_gated /
update_phase_with_estimate_gated / update_task_gated so the \`reviewed\`
transition gate can refuse it."
fi
ok "every ungated update-primitive caller is on the allowlist ($HIT_COUNT site(s))"

# Each allowlisted file must say, in a comment, that it is ungated on purpose —
# so a reader hitting the call site learns the reason without finding this file.
for f in $ALLOWLIST; do
    grep -q 'Deliberately UNGATED' "$REPO_ROOT/$f" ||
        fail "$f holds an allowlisted ungated call but carries no 'Deliberately UNGATED' comment explaining why"
done
ok "every allowlisted file explains its ungated call sites in place"

# Self-test A1: a planted new caller IS caught.
mkdir -p "$TMP/a-scratch/rdm-cli/src/commands"
cp "$REPO_ROOT/rdm-cli/src/commands/phase.rs" "$TMP/a-scratch/rdm-cli/src/commands/phase.rs"
printf '\nfn planted(s: &mut S) { let _ = rdm_core::ops::phase::update_phase(s); }\n' \
    >>"$TMP/a-scratch/rdm-cli/src/commands/phase.rs"
scan_ungated "$TMP/a-scratch" >"$TMP/a.scratch.hits"
grep -q '^rdm-cli/src/commands/phase.rs:' "$TMP/a.scratch.hits" ||
    fail "self-test failed: a planted ungated caller is NOT caught, so section A proves nothing"
ok "self-test: a planted ungated update-primitive caller IS caught"

# Self-test A2: the real tree passes its own gate (restated explicitly).
[ -z "$(comm -23 "$TMP/a.files" "$TMP/a.allow")" ] ||
    fail "self-test failed: the real, unmutated tree does not pass its own gate"
ok "self-test: the real, unmutated tree passes"

# ---------------------------------------------------------------------------
# Section B — every user-facing status-write surface is gated
# ---------------------------------------------------------------------------
say "Section B — user-facing status-write surfaces use a _gated entry"

# Every surface that can receive a CALLER-SUPPLIED status, and therefore reach
# `reviewed`. This half is the complement of section A: A bounds what stays
# ungated, B proves the rest really moved.
GATED_SURFACES='rdm-cli/src/commands/phase.rs:update_phase_with_estimate_gated
rdm-cli/src/commands/task.rs:update_task_gated
rdm-server/src/handlers/phases.rs:update_phase_gated
rdm-server/src/handlers/tasks.rs:update_task_gated'

# scan_gated <root> — print "<file>:<entry>" for each expected surface that
# really calls its gated entry.
scan_gated() {
    printf '%s\n' "$GATED_SURFACES" | while IFS=: read -r f entry; do
        [ -f "$1/$f" ] || continue
        if grep -q "$entry(" "$1/$f"; then
            printf '%s:%s\n' "$f" "$entry"
        fi
    done
}

scan_gated "$REPO_ROOT" >"$TMP/b.hits"
printf '%s\n' "$GATED_SURFACES" | sort >"$TMP/b.want"
sort "$TMP/b.hits" >"$TMP/b.got"
MISSING=$(comm -23 "$TMP/b.want" "$TMP/b.got")
if [ -n "$MISSING" ]; then
    fail "user-facing status-write surface(s) NOT using a gated entry:
$MISSING

Each of these accepts a caller-supplied status, so it can reach \`reviewed\` and
must route through the _gated sibling."
fi
ok "every user-facing status-write surface uses a _gated entry ($(wc -l <"$TMP/b.got" | tr -d ' ') site(s))"

# Self-test B1: a downgraded `_gated` call IS caught.
mkdir -p "$TMP/b-scratch/rdm-cli/src/commands"
sed 's/update_phase_with_estimate_gated(/update_phase_with_estimate(/g' \
    "$REPO_ROOT/rdm-cli/src/commands/phase.rs" \
    >"$TMP/b-scratch/rdm-cli/src/commands/phase.rs"
if grep -q 'update_phase_with_estimate_gated(' "$TMP/b-scratch/rdm-cli/src/commands/phase.rs"; then
    fail "self-test setup failed: the downgrade did not remove the gated call"
fi
ok "self-test: a downgraded _gated call is detectable (the gated name is gone)"

# Self-test B2: and the downgraded copy ALSO shows up in section A's scan as an
# unsanctioned ungated caller — the two halves interlock, so a downgrade cannot
# slip past by satisfying one and not the other.
scan_ungated "$TMP/b-scratch" >"$TMP/b.ungated"
grep -q '^rdm-cli/src/commands/phase.rs:' "$TMP/b.ungated" ||
    fail "self-test failed: a downgraded _gated call does not surface as an ungated caller, so the two sections do not interlock"
ok "self-test: a downgraded _gated call ALSO trips section A's allowlist"

# ---------------------------------------------------------------------------
# Section C — each refusal names a remediation command
# ---------------------------------------------------------------------------
say "Section C — every gate refusal is actionable"

ERR="$REPO_ROOT/rdm-core/src/error.rs"

# <variant>|<substring the message must contain>
for pair in \
    'GateNoApprovedPlan|rdm plan create' \
    'GateNoApprovedPlan|rdm review submit' \
    'GateNoApprovedChangeReview|rdm review start --on change/' \
    'GateWorktreeDirty|does NOT bypass this check' \
    'GateWorktreeUnobservable|does NOT bypass this check' \
    'GateOverrideEmptyReason|--override-gate requires a non-empty reason' \
    'GateOverrideGateDisabled|--override-gate has nothing to bypass' \
    'GateOverrideGateDisabled|rdm config set gates.reviewed true'; do
    variant=${pair%%|*}
    want=${pair#*|}
    grep -q "$variant" "$ERR" || fail "rdm-core/src/error.rs declares no $variant variant"
    grep -qF -e "$want" "$ERR" ||
        fail "the $variant refusal does not contain '$want' — every refusal must name the record that is missing AND the command that creates it"
done
ok "each of the six gate refusals names its remediation"

# The two worktree refusals must state that an override will not help, or an
# operator reads the refusal as a bug in the override.
grep -qF -e 'waives' "$ERR" ||
    fail "no refusal explains what --override-gate actually waives"
ok "the override's (a)/(b)-only scope is stated in the refusals"

say "All sections passed."
