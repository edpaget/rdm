#!/bin/sh
# Sourced helper library shared by the scripts/gen-workflow-*.sh generators, so
# the marker-block-stamping and embedded-copy-syncing logic lives in one place
# instead of being duplicated (byte-for-byte, awk state machine and all) in
# every generator.
#
# This file is SOURCED, never executed directly:
#
#   . "$REPO_ROOT/scripts/lib/gen-workflow-block.sh"
#   stamp_block "$SOURCE" "$BEGIN" "$END" "$CHECK" "$consumer1" "$consumer2"
#   sync_full_copy "$src" "$dst" "$CHECK"
#
# Public functions:
#
#   stamp_block SOURCE BEGIN END CHECK CONSUMER...
#       Extracts the region strictly between the `// >>> $BEGIN <<<` /
#       `// >>> $END <<<` marker lines in SOURCE, then, for each CONSUMER,
#       replaces the same marked region in that file with the extracted block
#       (the consumer keeps its OWN marker lines; only the region between them
#       is replaced). Markers are matched only after a `>>> ` comment prefix,
#       so an incidental in-block mention of the marker token can't be
#       mistaken for a real marker line and silently truncate extraction.
#       When CHECK is "1", nothing is written: a drifted consumer prints a
#       `DRIFT:` line (plus a unified diff) to stderr and the function returns
#       non-zero. Otherwise each consumer is rewritten in place when it
#       differs, printing `regenerated: <path>` or `unchanged: <path>`.
#
#   sync_full_copy SRC DST CHECK
#       Whole-file copy-or-diff: makes DST byte-identical to SRC. When CHECK
#       is "1", nothing is written: a drifted DST prints a `DRIFT:` line (plus
#       a unified diff) to stderr and the function returns non-zero.
#       Otherwise DST is overwritten when it differs, printing the same
#       `regenerated: <path>` / `unchanged: <path>` style stamp_block uses, so
#       `--check` output stays uniform across every generator that calls
#       either function.
#
# Both functions return 0 when nothing drifted (CHECK mode) or after
# successfully syncing (write mode), and non-zero when CHECK mode finds
# drift. Neither function exits the calling script directly — the caller is
# expected to accumulate a status and exit with it, exactly as each
# generator already does.

stamp_block() {
    _swb_source="$1"
    _swb_begin="$2"
    _swb_end="$3"
    _swb_check="$4"
    shift 4

    if [ ! -f "$_swb_source" ]; then
        echo "error: source of truth not found: $_swb_source" >&2
        return 1
    fi

    _swb_blockfile=$(mktemp)
    trap 'rm -f "$_swb_blockfile"' EXIT INT HUP TERM

    awk -v b=">>> $_swb_begin" -v e=">>> $_swb_end" '
        index($0, b) { infence = 1; next }
        index($0, e) { infence = 0 }
        infence { print }
    ' "$_swb_source" >"$_swb_blockfile"

    if [ ! -s "$_swb_blockfile" ]; then
        echo "error: no block found between '$_swb_begin' / '$_swb_end' markers in $_swb_source" >&2
        rm -f "$_swb_blockfile"
        trap - EXIT INT HUP TERM
        return 1
    fi

    _swb_status=0
    for _swb_consumer in "$@"; do
        if [ ! -f "$_swb_consumer" ]; then
            echo "error: consumer not found: $_swb_consumer" >&2
            _swb_status=1
            continue
        fi
        if ! grep -q ">>> $_swb_begin" "$_swb_consumer" || ! grep -q ">>> $_swb_end" "$_swb_consumer"; then
            echo "error: consumer $_swb_consumer is missing the '>>> $_swb_begin'/'>>> $_swb_end' markers" >&2
            _swb_status=1
            continue
        fi

        _swb_out=$(mktemp)
        awk -v b=">>> $_swb_begin" -v e=">>> $_swb_end" -v bf="$_swb_blockfile" '
            BEGIN { state = 0 }
            state == 0 {
                print
                if (index($0, b)) {
                    while ((getline line < bf) > 0) print line
                    close(bf)
                    state = 1
                }
                next
            }
            state == 1 {
                if (index($0, e)) { print; state = 2 }
                next
            }
            state == 2 { print }
        ' "$_swb_consumer" >"$_swb_out"

        if [ "$_swb_check" -eq 1 ]; then
            if ! diff -u "$_swb_consumer" "$_swb_out" >/dev/null 2>&1; then
                echo "DRIFT: $_swb_consumer is out of sync with $_swb_source — run the generator that stamps it" >&2
                diff -u "$_swb_consumer" "$_swb_out" >&2 || true
                _swb_status=1
            fi
        else
            if diff -q "$_swb_consumer" "$_swb_out" >/dev/null 2>&1; then
                echo "unchanged: $_swb_consumer"
            else
                cp "$_swb_out" "$_swb_consumer"
                echo "regenerated: $_swb_consumer"
            fi
        fi
        rm -f "$_swb_out"
    done

    rm -f "$_swb_blockfile"
    trap - EXIT INT HUP TERM
    return "$_swb_status"
}

sync_full_copy() {
    _sfc_src="$1"
    _sfc_dst="$2"
    _sfc_check="$3"

    if [ ! -f "$_sfc_src" ]; then
        echo "error: source not found: $_sfc_src" >&2
        return 1
    fi
    if [ ! -f "$_sfc_dst" ]; then
        echo "error: destination not found: $_sfc_dst" >&2
        return 1
    fi

    if [ "$_sfc_check" -eq 1 ]; then
        if diff -u "$_sfc_dst" "$_sfc_src" >/dev/null 2>&1; then
            return 0
        fi
        echo "DRIFT: $_sfc_dst is out of sync with $_sfc_src — run the generator that syncs it" >&2
        diff -u "$_sfc_dst" "$_sfc_src" >&2 || true
        return 1
    fi

    if diff -q "$_sfc_dst" "$_sfc_src" >/dev/null 2>&1; then
        echo "unchanged: $_sfc_dst"
    else
        cp "$_sfc_src" "$_sfc_dst"
        echo "regenerated: $_sfc_dst"
    fi
    return 0
}
