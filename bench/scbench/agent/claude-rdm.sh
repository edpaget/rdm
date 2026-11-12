#!/bin/sh
# claude-rdm: the agent binary SlopCodeBench runs in the rdm-equipped image
# (installed as /opt/rdm/bin/claude-rdm). On the first call in a container it
# initializes the plan repo at $RDM_ROOT, so each problem (one container)
# starts with a fresh plan repo that its later checkpoints reuse. Then it runs
# claude with SlopCodeBench's arguments. SlopCodeBench parses stdout as
# stream-json, so rdm init's output goes to stderr.
#
# After claude exits (every invocation, so a --continue retry exports too) it
# tars the plan repo, working tree and .git, to a new, counting-up name under
# $HOME/.claude/$RDM_PLAN_EXPORT_SUBDIR. That directory lies under the Claude
# trace directory SlopCodeBench copies new files from after each checkpoint,
# so each export lands in the checkpoint it was written during; the runner
# reads the greatest name back (README.md § "Full-lane arm"). An export
# failure is reported on stderr and never changes claude's exit status.
set -eu

if [ ! -d "$RDM_ROOT/.git" ]; then
    rdm --root "$RDM_ROOT" init --default-project "$RDM_PROJECT" >&2
fi
status=0
claude "$@" || status=$?
dir="$HOME/.claude/$RDM_PLAN_EXPORT_SUBDIR"
mkdir -p "$dir" &&
    n=$(find "$dir" -name '*.tar' | wc -l) &&
    tar -C "$RDM_ROOT" -cf "$dir/$(printf '%06d' $((n + 1))).tar" . >&2 ||
    echo "claude-rdm: plan repo export failed" >&2
exit "$status"
