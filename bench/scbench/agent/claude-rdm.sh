#!/bin/sh
# claude-rdm: the agent binary SlopCodeBench runs in the rdm-equipped image
# (installed as /opt/rdm/bin/claude-rdm), once per checkpoint attempt.
#
# Init. On the first call in a container it initializes the plan repo at
# $RDM_ROOT, so each problem (one container) starts with a fresh plan repo
# that its later checkpoints reuse. When $RDM_SCBENCH_VERIFY is set (the
# manifest's [rdm] verify) it also sets that command as the plan repo's
# dispatch.verify, then commits init's files and the gate as the plan repo's
# first commit. Only the first call does this, so a later checkpoint or a
# resume never overwrites the gate.
#
# Ceiling. Claude Code's print mode stops background tasks (subagents,
# Workflows) 10 minutes after the last turn. Unless the caller set it, the
# wrapper raises CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS to 2 h for the rdm arms.
#
# Settle check. The checkpoint's phase is the highest-numbered phase of
# $RDM_ROADMAP, numbered above the highest phase before this checkpoint (the
# baseline, kept under $TMPDIR across SlopCodeBench's own --continue retry).
# It has settled when its status is done, blocked or wont-fix. The phase
# status, read through rdm, is the only thing that decides.
#
# Resume loop. It runs claude with SlopCodeBench's arguments. While the
# checkpoint has not settled, it resumes the same session up to
# $RDM_RESUME_MAX times (default 6; 0 disables): every argument before --,
# with --continue added unless present, then -- and a fixed continuation
# prompt. It exits with the last claude's status.
#
# Export. After every claude exit, resumes included, it tars the plan repo,
# working tree and .git, to a new, counting-up name under
# $HOME/.claude/$RDM_PLAN_EXPORT_SUBDIR. That directory lies under the Claude
# trace directory SlopCodeBench copies new files from after each checkpoint,
# so each export lands in the checkpoint it was written during; the runner
# reads the greatest name back (README.md § "Full-lane arm"). An export
# failure is reported on stderr and never changes claude's exit status.
#
# Stdout. SlopCodeBench parses stdout as stream-json, so stdout carries only
# claude's stream; everything the wrapper and rdm print goes to stderr.
set -eu

RDM_RESUME_MAX="${RDM_RESUME_MAX:-6}"
case "$RDM_RESUME_MAX" in
    '' | *[!0-9]*)
        echo "claude-rdm: RDM_RESUME_MAX=$RDM_RESUME_MAX is not a non-negative integer; not resuming" >&2
        RDM_RESUME_MAX=0
        ;;
esac
export CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS="${CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS:-7200000}"

if [ ! -d "$RDM_ROOT/.git" ]; then
    rdm --root "$RDM_ROOT" init --default-project "$RDM_PROJECT" >&2
    if [ -n "${RDM_SCBENCH_VERIFY:-}" ]; then
        rdm --root "$RDM_ROOT" config set dispatch.verify "$RDM_SCBENCH_VERIFY" >&2
    fi
    rdm --root "$RDM_ROOT" commit -m "chore(bench): initialize plan repo" >&2
fi

# Tars the plan repo to the next export name; never fails the wrapper.
export_plan() {
    dir="$HOME/.claude/$RDM_PLAN_EXPORT_SUBDIR"
    mkdir -p "$dir" &&
        n=$(find "$dir" -name '*.tar' | wc -l) &&
        tar -C "$RDM_ROOT" -cf "$dir/$(printf '%06d' $((n + 1))).tar" . >&2 ||
        echo "claude-rdm: plan repo export failed" >&2
}

# Prints "<number> <status>" of $RDM_ROADMAP's highest-numbered phase, or
# nothing when rdm fails (no roadmap yet) or there are no phases.
current_phase() {
    rdm --root "$RDM_ROOT" phase list --roadmap "$RDM_ROADMAP" \
        --project "$RDM_PROJECT" --format json | awk '
        {
            line = $0
            while (match(line, /"(number|status)"[[:space:]]*:[[:space:]]*("[^"]*"|[0-9]+)/)) {
                m = substr(line, RSTART, RLENGTH)
                line = substr(line, RSTART + RLENGTH)
                v = m
                sub(/^"[a-z]+"[[:space:]]*:[[:space:]]*/, "", v)
                gsub(/"/, "", v)
                if (m ~ /^"number"/) {
                    cur = v + 0
                    if (!seen || cur > max) max = cur
                    seen = 1
                } else if (seen) {
                    status[cur] = v
                }
            }
        }
        END { if (seen) print max, status[max] }'
}

# True when the checkpoint's phase has settled; sets phase_num and
# phase_status for the resume message either way.
settled() {
    cur=$(current_phase)
    phase_num=none
    phase_status=missing
    [ -n "$cur" ] || return 1
    [ "${cur%% *}" -gt "$baseline" ] || return 1
    phase_num=${cur%% *}
    phase_status=${cur#* }
    case "$phase_status" in
        done | blocked | wont-fix) return 0 ;;
    esac
    return 1
}

# Scan the incoming arguments before --: a --continue there is
# SlopCodeBench's own retry of this checkpoint attempt.
has_sep=0
has_continue=0
for a in "$@"; do
    if [ "$a" = -- ]; then
        has_sep=1
        break
    fi
    if [ "$a" = --continue ]; then
        has_continue=1
    fi
done

base_file="${TMPDIR:-/tmp}/claude-rdm.baseline"
if [ "$has_continue" -eq 0 ] || [ ! -f "$base_file" ]; then
    cur=$(current_phase)
    baseline=${cur%% *}
    printf '%s\n' "${baseline:-0}" >"$base_file"
fi
baseline=$(cat "$base_file")
case "$baseline" in
    '' | *[!0-9]*) baseline=0 ;;
esac

status=0
claude "$@" || status=$?
export_plan

# The resume arguments: every argument before --, --continue first unless
# present, then -- and the continuation prompt.
count=$#
seen_sep=0
for a in "$@"; do
    if [ "$a" = -- ]; then
        seen_sep=1
    fi
    if [ "$seen_sep" -eq 0 ]; then
        set -- "$@" "$a"
    fi
done
shift "$count"
if [ "$has_continue" -eq 0 ]; then
    set -- --continue "$@"
fi
prompt="Continue this SlopCodeBench checkpoint. Your previous claude process \
exited before the checkpoint settled: its phase of roadmap \`$RDM_ROADMAP\` has \
not reached \`done\`, \`blocked\` or \`wont-fix\`. Any background task, subagent \
or Workflow that process was running was stopped when it exited, and no result \
or notification from it will arrive. This is the same checkpoint: if you \
already created its phase, do not create another. Keep following every rule \
and step of this session's first prompt. Check where the checkpoint stands \
with \`rdm phase list --roadmap $RDM_ROADMAP --project $RDM_PROJECT --format \
json\` and the phase's plans and reviews, redo in the foreground any step whose \
result you never received, and carry on from there. Do not end a turn while a \
background task you started is still running; wait for its result first. If \
the first prompt's rules tell you to stop, end with the same one-line summary."
set -- "$@" -- "$prompt"

k=0
while ! settled; do
    if [ "$has_sep" -eq 0 ]; then
        echo "claude-rdm: phase $phase_num is $phase_status; no -- in the arguments, not resuming" >&2
        break
    fi
    if [ "$k" -ge "$RDM_RESUME_MAX" ]; then
        echo "claude-rdm: checkpoint not settled after $RDM_RESUME_MAX resume(s)" >&2
        break
    fi
    k=$((k + 1))
    echo "claude-rdm: phase $phase_num is $phase_status; resuming ($k/$RDM_RESUME_MAX)" >&2
    status=0
    claude "$@" || status=$?
    export_plan
done
exit "$status"
