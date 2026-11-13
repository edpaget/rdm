#!/bin/sh
# Prepares rdm's plan root in the SlopCodeBench agent image. docker.j2 runs it
# as root at agent-image build time (installed as
# /opt/rdm/bin/rdm-plan-root-setup by the payload Dockerfile).
#
# SCBench runs the container as the host uid, which does not own a directory
# made at build time. chmod 1777 lets that uid write the plan root, but git
# refuses a worktree whose top-level the user does not own ("dubious
# ownership") unless safe.directory lists it. A --system entry covers every
# uid, needs no $HOME, and is read by both the git CLI and gix.
#
# /workspace__worktrees is where rdm puts the primary checkout's worktrees
# (rdm-git's worktree::worktree_path: <parent>/<repo-name>__worktrees/<item>);
# `/` is root-owned, so it is made here, beside the graded /workspace. Its
# worktrees need no safe.directory entry: each worktree dir is created by the
# running uid.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: rdm-plan-root-setup <plan-root>" >&2
    exit 2
fi
plan_root=$1

mkdir -p "$plan_root" /workspace__worktrees
chmod 1777 "$plan_root" /workspace__worktrees
git config --system --add safe.directory "$plan_root"
