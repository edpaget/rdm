#!/bin/sh
# claude-rdm: the agent binary SlopCodeBench runs in the rdm-equipped image
# (installed as /opt/rdm/bin/claude-rdm). On the first call in a container it
# initializes the plan repo at $RDM_ROOT, so each problem (one container)
# starts with a fresh plan repo that its later checkpoints reuse. Then it runs
# claude with SlopCodeBench's arguments. SlopCodeBench parses stdout as
# stream-json, so rdm init's output goes to stderr.
set -eu

if [ ! -d "$RDM_ROOT/.git" ]; then
    rdm --root "$RDM_ROOT" init --default-project "$RDM_PROJECT" >&2
fi
exec claude "$@"
