#!/bin/sh
# Copy the Claude Code web sandbox template into a target source repo.
#
# Usage: scripts/install-claude-code-web-template.sh <target-repo-dir> [--force] [--yes]
#
#   --yes    Overwrite differing files without prompting.
#   --force  Same as --yes but also suppresses the diff preview.
#
# Safe to run repeatedly: identical files are skipped silently; differing
# files prompt before overwrite unless --yes/--force is passed.

set -eu

usage() {
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
}

TARGET=""
FORCE=0
YES=0

while [ $# -gt 0 ]; do
    case "$1" in
        --force) FORCE=1; YES=1; shift ;;
        --yes)   YES=1; shift ;;
        -h|--help) usage; exit 0 ;;
        --*) echo "error: unknown option: $1" >&2; usage >&2; exit 2 ;;
        *)
            if [ -n "$TARGET" ]; then
                echo "error: unexpected extra argument: $1" >&2
                usage >&2
                exit 2
            fi
            TARGET="$1"
            shift
            ;;
    esac
done

if [ -z "$TARGET" ]; then
    echo "error: target directory is required" >&2
    usage >&2
    exit 2
fi

if [ ! -d "$TARGET" ]; then
    echo "error: target is not a directory: $TARGET" >&2
    exit 1
fi

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
SRC="$REPO_ROOT/templates/claude-code-web"

if [ ! -d "$SRC" ]; then
    echo "error: template dir not found at $SRC" >&2
    echo "       run this script from a checkout of the rdm repo." >&2
    exit 1
fi

TARGET=$(cd "$TARGET" && pwd)

COPIED=0
SKIPPED_IDENTICAL=0
SKIPPED_USER=0
OVERWROTE=0

install_one() {
    src="$1"
    rel="$2"
    dst="$TARGET/$rel"

    dst_dir=$(dirname "$dst")
    mkdir -p "$dst_dir"

    if [ -e "$dst" ]; then
        if cmp -s "$src" "$dst"; then
            SKIPPED_IDENTICAL=$((SKIPPED_IDENTICAL + 1))
            return 0
        fi
        if [ "$YES" -eq 0 ]; then
            echo
            echo "  $rel (differs)"
            if command -v diff >/dev/null 2>&1; then
                diff -u "$dst" "$src" || true
            fi
            printf "  overwrite %s? [y/N] " "$rel"
            read -r answer
            case "$answer" in
                y|Y|yes|YES)
                    : ;;
                *)
                    SKIPPED_USER=$((SKIPPED_USER + 1))
                    return 0
                    ;;
            esac
        elif [ "$FORCE" -eq 0 ]; then
            echo "  overwriting $rel (differs)"
        fi
        cp "$src" "$dst"
        case "$rel" in *.sh) chmod +x "$dst" ;; esac
        OVERWROTE=$((OVERWROTE + 1))
    else
        cp "$src" "$dst"
        case "$rel" in *.sh) chmod +x "$dst" ;; esac
        COPIED=$((COPIED + 1))
        echo "  + $rel"
    fi
}

echo "Installing template into $TARGET"

# Collect relative paths into a tempfile so the while loop runs in the main
# shell (pipelines spawn subshells, which would drop our counter mutations).
tmpfile=$(mktemp)
trap 'rm -f "$tmpfile"' EXIT INT HUP TERM
( cd "$SRC" && find . -type f ) > "$tmpfile"

while IFS= read -r rel <&3; do
    rel=${rel#./}
    install_one "$SRC/$rel" "$rel"
done 3< "$tmpfile"

echo
echo "Summary: $COPIED copied, $OVERWROTE overwritten, $SKIPPED_IDENTICAL identical, $SKIPPED_USER skipped by user"
echo
echo "Next steps:"
echo "  - Set RDM_PLAN_REPO in your Claude Code sandbox env."
echo "  - Merge .claude/settings.claude-code-web.json.example into .claude/settings.json."
echo "  - See docs/claude-code-web.md in the rdm repo for details."
