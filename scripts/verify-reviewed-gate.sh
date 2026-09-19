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
#   D  the gate's worktree probe is built in ONE feature-split place, so the
#      `phase update` / `task update` arms compile with `git` disabled
#   E  every gated update wrapper's `# Errors` block enumerates every `Gate*`
#      variant `rdm-core/src/error.rs` declares, and none of them states a
#      hard-coded variant COUNT instead
#
# Sections A, B, D and E each carry a planted-mutation self-test proving the
# check can fail, and a restatement that the real, unmutated tree passes.
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
    'GateStaleChangeReview|rdm review start --on change/' \
    'GateStaleChangeReview|--override-gate' \
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
ok "each of the seven gate refusals names its remediation"

# The two worktree refusals must state that an override will not help, or an
# operator reads the refusal as a bug in the override.
grep -qF -e 'waives' "$ERR" ||
    fail "no refusal explains what --override-gate actually waives"
ok "the override's (a)/(b)-only scope is stated in the refusals"

# ---------------------------------------------------------------------------
# Section D — the worktree probe is built in one feature-split place
# ---------------------------------------------------------------------------
say "Section D — the gate's probe construction stays feature-agnostic at the call sites"

# `git` is an OPTIONAL feature of rdm-cli, so a build with it off is a
# configuration the crate supports — but `cargo clippy` / `cargo nextest run`
# only ever exercise the default feature set, so a `rdm_git::` call reached
# from a feature-agnostic call site compiles fine in CI and breaks every
# no-default-features consumer. `commands::build_gate_probe` is the single
# place that split is spelled out; this section keeps it single.
#
# (CI's feature-matrix step is the dynamic half. This static half is what runs
# in the harness loop and names the rule when it is broken.)

MODRS="$REPO_ROOT/rdm-cli/src/commands/mod.rs"

grep -q 'cfg(feature = "git")' "$MODRS" ||
    fail "rdm-cli/src/commands/mod.rs has no git-enabled arm at all"
awk '/fn build_gate_probe\(/ { found++ } END { exit(found == 2 ? 0 : 1) }' "$MODRS" ||
    fail "rdm-cli/src/commands/mod.rs must declare exactly two build_gate_probe arms (git and not(git)); found $(grep -c 'fn build_gate_probe(' "$MODRS")"
grep -B2 'fn build_gate_probe(' "$MODRS" | grep -q 'cfg(not(feature = "git"))' ||
    fail "build_gate_probe has no not(feature = \"git\") arm — the non-git build has no probe to fall back to"
ok "build_gate_probe carries both feature arms"

# scan_direct_probe <root> — print "<file>" for each update arm that builds a
# probe itself instead of going through the shared helper.
scan_direct_probe() {
    for f in rdm-cli/src/commands/phase.rs rdm-cli/src/commands/task.rs; do
        [ -f "$1/$f" ] || continue
        if grep -q 'discover_distinct_project_repo\|GateProbe::new' "$1/$f"; then
            printf '%s\n' "$f"
        fi
    done
}

DIRECT=$(scan_direct_probe "$REPO_ROOT")
if [ -n "$DIRECT" ]; then
    fail "update arm(s) constructing the gate probe directly:
$DIRECT

These are compiled in BOTH the git and non-git builds, so naming rdm_git:: or
GateProbe::new here breaks \`cargo check -p rdm-cli --no-default-features\`.
Call commands::build_gate_probe(gate_enabled, root) instead."
fi
ok "neither update arm builds the probe itself"

for f in rdm-cli/src/commands/phase.rs rdm-cli/src/commands/task.rs; do
    grep -q 'build_gate_probe(' "$REPO_ROOT/$f" ||
        fail "$f never calls build_gate_probe — the gate's worktree precondition is not wired there"
done
ok "both update arms route through build_gate_probe"

# Self-test D1: restoring the inlined, git-only construction IS caught.
mkdir -p "$TMP/d-scratch/rdm-cli/src/commands"
sed 's/commands::build_gate_probe(gate_enabled, root)/std::env::current_dir().ok().and_then(|cwd| rdm_git::worktree::discover_distinct_project_repo(\&cwd, root).ok()).map(commands::GateProbe::new)/' \
    "$REPO_ROOT/rdm-cli/src/commands/phase.rs" \
    >"$TMP/d-scratch/rdm-cli/src/commands/phase.rs"
cp "$REPO_ROOT/rdm-cli/src/commands/task.rs" "$TMP/d-scratch/rdm-cli/src/commands/task.rs"
if [ "$(scan_direct_probe "$TMP/d-scratch")" != "rdm-cli/src/commands/phase.rs" ]; then
    fail "self-test failed: an inlined git-only probe construction is NOT caught, so section D is vacuous"
fi
ok "self-test: an inlined git-only probe construction IS caught"

# Self-test D2: and the real, unmutated tree still passes the same scan.
[ -z "$(scan_direct_probe "$REPO_ROOT")" ] ||
    fail "self-test failed: the real tree trips section D"
ok "self-test: the real tree passes section D"

# ---------------------------------------------------------------------------
# Section E — every gated wrapper documents every gate refusal
# ---------------------------------------------------------------------------
say "Section E — the gated wrappers' \`# Errors\` blocks enumerate every Gate* variant"

# `rdm-core` has `#![warn(missing_docs)]`, so every gated wrapper HAS an
# `# Errors` block — but nothing makes it keep up with `error.rs`. The
# omission this section exists for was real: two wrappers left
# `GateOverrideGateDisabled` out entirely, and the third documented its
# refusals by intra-doc pointer plus a hard-coded count ("the five gate
# variants listed on [`update_phase_gated`]") that a later commit made false.
# A count-bearing pointer carries no literal `Error::Gate` text at all, so a
# grep for the variants alone would have reported that wrapper green.
#
# Wrappers are discovered BY SIGNATURE, never by name, and the discovered
# count is pinned: a fourth `pub fn *_gated` must turn this section red rather
# than be silently skipped.

# gated_wrappers <root> — print "<file>:<fn name>" per gated wrapper.
gated_wrappers() {
    for f in "$1"/rdm-core/src/ops/*.rs; do
        [ -f "$f" ] || continue
        rel=${f#"$1"/}
        sed -n 's/^pub fn \([a-z_]*_gated\)(.*/\1/p' "$f" |
            while read -r name; do printf '%s:%s\n' "$rel" "$name"; done
    done
}

# errors_block <file> <fn name> — print the `# Errors` section of that
# function's doc comment (the `///` run immediately above its `pub fn` line).
errors_block() {
    awk -v want="$2" '
        /^\/\/\// { buf = buf $0 "\n"; next }
        /^#\[/    { next }
        {
            if ($0 ~ "^pub fn " want "\\(") { print buf; exit }
            buf = ""
        }
    ' "$1" | awk '/^\/\/\/ # Errors/ { on = 1; next } on { print }'
}

# Every `Gate*` variant declared in error.rs, one per line.
GATE_VARIANTS=$(sed -n 's/^    \(Gate[A-Za-z]*\)[ ,{].*/\1/p' "$ERR" | sort -u)
[ -n "$GATE_VARIANTS" ] || fail "no Gate* variants found in $ERR — the scan is broken"

# check_section_e <root> — print one diagnostic line per problem found.
# Always exits 0: the findings are the output, not the status.
check_section_e() {
    root=$1
    found=$(gated_wrappers "$root")
    count=$(printf '%s' "$found" | grep -c . || true)
    if [ "$count" -ne 3 ]; then
        printf 'WRAPPER-COUNT %s (expected 3)\n' "$count"
    fi
    printf '%s\n' "$found" | while IFS=: read -r rel name; do
        if [ -z "$name" ]; then
            continue
        fi
        block=$(errors_block "$root/$rel" "$name")
        if [ -z "$block" ]; then
            printf 'NO-ERRORS-BLOCK %s::%s\n' "$rel" "$name"
            continue
        fi
        for variant in $GATE_VARIANTS; do
            if ! printf '%s\n' "$block" | grep -q "Error::$variant"; then
                printf 'MISSING %s::%s %s\n' "$rel" "$name" "$variant"
            fi
        done
        # A pointered or prose variant COUNT is what let "the five gate
        # variants" go stale; the enumeration must stand on its own.
        if printf '%s\n' "$block" |
            grep -Eq '(three|four|five|six|seven|eight|[0-9]+) gate variants'; then
            printf 'HARD-CODED-COUNT %s::%s\n' "$rel" "$name"
        fi
    done
    return 0
}

PROBLEMS=$(check_section_e "$REPO_ROOT")
if [ -n "$PROBLEMS" ]; then
    fail "gated wrappers with an incomplete or stale \`# Errors\` block:
$PROBLEMS

Every \`pub fn *_gated\` must enumerate EVERY Gate* variant declared in
rdm-core/src/error.rs, by name, in its own \`# Errors\` block — no intra-doc
pointer, and never a hard-coded count of how many there are.
A WRAPPER-COUNT line means a gated wrapper was added or removed: extend this
section (and the three wrappers' docs) rather than loosening the assertion."
fi
ok "all 3 gated wrappers enumerate every Gate* variant, with no hard-coded counts"

# Self-test E1/E2/E3: the check fails for each way a block can go stale —
# a deleted variant in the plain wrapper, a deleted variant in the PREVIOUSLY
# POINTERED wrapper (the sub-case a two-wrapper scope would pass vacuously),
# and a reinstated hard-coded count.
mkdir -p "$TMP/e-scratch/rdm-core/src/ops"
cp "$REPO_ROOT"/rdm-core/src/ops/*.rs "$TMP/e-scratch/rdm-core/src/ops/"
[ -z "$(check_section_e "$TMP/e-scratch")" ] ||
    fail "self-test setup failed: an unmutated copy already trips section E"

mutate_e() {
    cp "$REPO_ROOT"/rdm-core/src/ops/*.rs "$TMP/e-scratch/rdm-core/src/ops/"
    "$@"
}

drop_variant_from() {
    # $1 = file, $2 = fn name, $3 = variant: blank out the variant's mention
    # inside that function's `# Errors` block only.
    python3 - "$1" "$2" "$3" <<'PYE'
import re, sys
path, name, variant = sys.argv[1], sys.argv[2], sys.argv[3]
src = open(path).read()
idx = src.index("\npub fn %s(" % name)
head = src[:idx]
start = head.rindex("/// # Errors")
block = head[start:]
block = block.replace("[`Error::%s`]" % variant, "[`Error::Placeholder`]", 1)
open(path, "w").write(head[:start] + block + src[idx:])
PYE
}

mutate_e drop_variant_from \
    "$TMP/e-scratch/rdm-core/src/ops/phase.rs" update_phase_gated GateOverrideGateDisabled
printf '%s\n' "$(check_section_e "$TMP/e-scratch")" |
    grep -q 'MISSING rdm-core/src/ops/phase.rs::update_phase_gated GateOverrideGateDisabled' ||
    fail "self-test failed: a variant deleted from update_phase_gated's block is NOT caught"
ok "self-test: a variant missing from update_phase_gated IS caught"

mutate_e drop_variant_from \
    "$TMP/e-scratch/rdm-core/src/ops/phase.rs" update_phase_with_estimate_gated GateStaleChangeReview
printf '%s\n' "$(check_section_e "$TMP/e-scratch")" |
    grep -q 'MISSING rdm-core/src/ops/phase.rs::update_phase_with_estimate_gated GateStaleChangeReview' ||
    fail "self-test failed: a variant deleted from update_phase_with_estimate_gated's block is NOT caught — the previously pointered wrapper is out of scope again"
ok "self-test: a variant missing from update_phase_with_estimate_gated IS caught"

# shellcheck disable=SC2016  # the backticks are literal rustdoc, not a subshell
mutate_e sed -i.bak \
    's|/// Everything \[`update_phase_with_estimate`\] returns, plus — only when `status` is|/// Everything [`update_phase_with_estimate`] returns, plus the five gate variants listed on [`update_phase_gated`], and — only when `status` is|' \
    "$TMP/e-scratch/rdm-core/src/ops/phase.rs"
rm -f "$TMP/e-scratch/rdm-core/src/ops/phase.rs.bak"
printf '%s\n' "$(check_section_e "$TMP/e-scratch")" |
    grep -q 'HARD-CODED-COUNT rdm-core/src/ops/phase.rs::update_phase_with_estimate_gated' ||
    fail "self-test failed: a reinstated hard-coded variant count is NOT caught"
ok "self-test: a reinstated hard-coded variant count IS caught"

# Self-test E4: a FOURTH gated wrapper reddens the count check rather than
# being silently skipped.
mutate_e true
cat >>"$TMP/e-scratch/rdm-core/src/ops/task.rs" <<'EOS'

/// A fourth gated wrapper, planted by the harness self-test.
///
/// # Errors
///
/// Nothing documented on purpose.
pub fn update_something_else_gated() {}
EOS
printf '%s\n' "$(check_section_e "$TMP/e-scratch")" | grep -q 'WRAPPER-COUNT 4' ||
    fail "self-test failed: a fourth gated wrapper does not trip the count assertion, so section E can be silently outgrown"
ok "self-test: a fourth gated wrapper trips the count assertion"

# Self-test E5: and the real, unmutated tree still passes the same check.
[ -z "$(check_section_e "$REPO_ROOT")" ] ||
    fail "self-test failed: the real tree trips section E"
ok "self-test: the real tree passes section E"

say "All sections passed."
