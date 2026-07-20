#!/bin/sh
# Render the canonical review spec into the shipped review skill templates.
#
# `.claude/workflows/lib/review.mjs` is the ONE source of truth for the whole
# review — find → refute → filter → verdict → gate. Its `//| ` literate comment
# lines inside the `review-spec` and `review-gate-spec` regions are the
# skill-renderable prose; this script strips the `//| ` prefix and stamps the
# result between the markers
#
# Mode tags: a prose line may carry an optional per-line mode tag immediately
# after the `//|` prefix.
#
#   //| ...      shared    — rendered in EVERY mode
#   //|code| ... code-only — rendered by --mode code
#   //|plan| ... plan-only — rendered by --mode plan
#
# The tag is recognized only as the literal `code|` or `plan|` immediately after
# `//|`, so shared prose must never begin with that text. One region pair, one
# emitter, two skills — there is no second generator.
#
#   <!-- rdm:review-spec:begin ... -->
#   <!-- rdm:review-spec:end -->
#
# in each consumer template. Everything outside those markers — the cli/mcp tool
# narrative, the `{proj_flag}` / `{proj_param}` / `{t_*}` placeholders, model
# resolution prose — stays hand-authored.
#
# Its sibling `scripts/gen-workflow-review.sh` stamps the JS block from the same
# source into the workflow-script consumers.
#
# Usage:
#   scripts/gen-skill-review.sh                    # rewrite consumers in place
#   scripts/gen-skill-review.sh --check            # exit non-zero on drift
#   scripts/gen-skill-review.sh --mode plan        # target the plan-review skills
#
# Modes:
#   code (default) — consumers: skill-review-{cli,mcp}.md
#   plan           — consumers: skill-plan-review-{cli,mcp}.md
#
# `--check` is what scripts/verify-workflow-review.sh and CI use to prove no
# template was hand-edited out of sync with the source of truth. BOTH modes are
# `--check`-gated there.

set -eu

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

SOURCE="$REPO_ROOT/.claude/workflows/lib/review.mjs"
TEMPLATES="$REPO_ROOT/rdm-core/src/templates"
MARKER_BEGIN='<!-- rdm:review-spec:begin'
MARKER_END='<!-- rdm:review-spec:end -->'

CHECK=0
MODE=code
while [ $# -gt 0 ]; do
    case "$1" in
        --check) CHECK=1 ;;
        --mode)
            shift
            MODE="${1:-}"
            ;;
        --mode=*) MODE="${1#--mode=}" ;;
        *)
            echo "error: unknown argument: $1" >&2
            echo "usage: gen-skill-review.sh [--check] [--mode code|plan]" >&2
            exit 2
            ;;
    esac
    shift
done

case "$MODE" in
    code) set -- "$TEMPLATES/skill-review-cli.md" "$TEMPLATES/skill-review-mcp.md" ;;
    plan) set -- "$TEMPLATES/skill-plan-review-cli.md" "$TEMPLATES/skill-plan-review-mcp.md" ;;
    *)
        echo "error: unknown mode: $MODE (expected 'code' or 'plan')" >&2
        exit 2
        ;;
esac

if [ ! -f "$SOURCE" ]; then
    echo "error: canonical review source not found: $SOURCE" >&2
    exit 1
fi

specfile=$(mktemp)
trap 'rm -f "$specfile"' EXIT INT HUP TERM

# Extract the `//| ` prose from the review-spec region, then the review-gate-spec
# region, in that order. Marker lines are matched only after the "// >>> "
# comment prefix, so an incidental in-region mention cannot truncate extraction.
extract_region() {
    awk -v b=">>> $1:begin" -v e=">>> $1:end" -v mode="$MODE" '
        index($0, b) { inregion = 1; next }
        index($0, e) { inregion = 0 }
        inregion && /^[[:space:]]*\/\/\|/ {
            # Prose lines may be indented (they sit inside object literals).
            # Strip the indent AND the "//|" prefix; what remains may open with a
            # mode tag ("code|" / "plan|"), otherwise the line is shared.
            line = $0
            sub(/^[[:space:]]*\/\/\|/, "", line)
            tag = ""
            if (substr(line, 1, 5) == "code|") { tag = "code"; line = substr(line, 6) }
            else if (substr(line, 1, 5) == "plan|") { tag = "plan"; line = substr(line, 6) }
            if (tag != "" && tag != mode) next
            # Drop the single separating space after the prefix/tag; anything
            # further in is the markdown authors intentional indentation.
            sub(/^ /, "", line)
            print line
        }
    ' "$SOURCE"
}

{
    extract_region review-spec
    extract_region review-gate-spec
} >"$specfile"

if [ ! -s "$specfile" ]; then
    echo "error: no '//|' spec prose found in the review-spec / review-gate-spec regions of $SOURCE" >&2
    exit 1
fi

# The rendered region is shared byte-for-byte across the cli and mcp consumers,
# so it must be free of surface-specific template placeholders. Reject rather
# than ship an unrendered `{proj_flag}` into a generated skill.
if grep -nE '\{proj_flag\}|\{proj_param\}|\{t_[a-z_]+\}|\{principles\}' "$specfile" >&2; then
    echo "error: the shared review spec must not contain template placeholders — they cannot render in both cli and mcp" >&2
    exit 1
fi

status=0
for consumer in "$@"; do
    if [ ! -f "$consumer" ]; then
        echo "error: consumer not found: $consumer" >&2
        exit 1
    fi
    if ! grep -q "$MARKER_BEGIN" "$consumer" || ! grep -q "$MARKER_END" "$consumer"; then
        echo "error: consumer $consumer is missing the rdm:review-spec begin/end markers" >&2
        exit 1
    fi

    # Consumer head (through its begin marker) + spec + tail (from end marker on).
    out=$(mktemp)
    awk -v b="$MARKER_BEGIN" -v e="$MARKER_END" -v sf="$specfile" '
        BEGIN { state = 0 }
        state == 0 {
            print
            if (index($0, b)) {
                print ""
                while ((getline line < sf) > 0) print line
                close(sf)
                print ""
                state = 1
            }
            next
        }
        state == 1 {
            if (index($0, e)) { print; state = 2 }
            next
        }
        state == 2 { print }
    ' "$consumer" >"$out"

    if [ "$CHECK" -eq 1 ]; then
        if ! diff -u "$consumer" "$out" >/dev/null 2>&1; then
            echo "DRIFT: $consumer is out of sync with $SOURCE — run scripts/gen-skill-review.sh --mode $MODE" >&2
            diff -u "$consumer" "$out" >&2 || true
            status=1
        fi
    else
        if diff -q "$consumer" "$out" >/dev/null 2>&1; then
            echo "unchanged: $consumer"
        else
            cp "$out" "$consumer"
            echo "regenerated: $consumer"
        fi
    fi
    rm -f "$out"
done

exit "$status"
