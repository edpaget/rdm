#!/bin/sh
# Shared helpers for asserting that a workflow script's mechanical
# (fetch/exec) agent() calls are pinned to the resolved mechanical/small
# tier rather than inheriting a judgment stage's model or the session model.
#
# Sourced by scripts/verify-workflow-{dispatch,backlog,document,estimate,review}.sh.
# Generalizes dispatch-phase's original AC-MODEL awk extractor
# (agent_option_blocks) plus a new assert_label_model primitive for asserting
# a SPECIFIC label's block carries a SPECIFIC model: expression. Every caller
# follows the same planted-mutation self-test convention: repoint (or strip)
# the real model: literal in a scratch copy, assert the check now fails,
# restore, assert it passes again.

# agent_option_blocks <file> — print every agent()/_agent() call's option
# object as a newline-delimited block terminated by a literal "---END---"
# line. Skips single-line `//` comments so prose mentioning agent()/_agent()
# never opens a false block. Tracks "saw agent(, still hunting for the
# opening {" ACROSS lines so a call whose options object opens on a later
# line is not silently invisible to callers.
agent_option_blocks() {
    awk '
      /^[[:space:]]*\/\// { next }
      !inblk && index($0, "agent(") { pending = 1 }
      pending && index($0, "{") { inblk = 1; pending = 0; buf = $0; next }
      inblk { buf = buf "\n" $0 }
      inblk && /^[[:space:]]*\}\)/ { print buf "\n---END---"; inblk = 0; buf = "" }
    ' "$1"
}

# assert_label_model <blocks-file> <label-regex> <expected-model-literal>
#
# For every block whose `label:` line matches <label-regex> (an unanchored
# ERE fragment — the caller supplies enough of the literal label to be
# unambiguous, e.g. "stamp:in-progress" or "gather:" as a prefix match),
# assert the block ALSO contains the literal substring
# "model: <expected-model-literal>". Fails (nonzero, message on stderr naming
# the first violating label) if a matching block lacks the expected model:
# expression, OR if no block matched the label at all — a label typo must not
# silently pass as "nothing to check".
assert_label_model() {
    blocks_file="$1"
    label_re="$2"
    expected="$3"
    awk -v label_re="$label_re" -v expected="$expected" '
      BEGIN { RS = "---END---"; matched = 0; bad = 0; violating = "" }
      $0 ~ ("label: .*" label_re) {
        matched++
        if (index($0, "model: " expected) == 0) {
          bad++
          if (violating == "") {
            n = split($0, lines, "\n")
            for (i = 1; i <= n; i++) {
              if (lines[i] ~ /label:/) { violating = lines[i]; break }
            }
          }
        }
      }
      END {
        if (matched == 0) {
          print "assert_label_model: no block matched label /" label_re "/" > "/dev/stderr"
          exit 1
        }
        if (bad > 0) {
          print "assert_label_model: missing \"model: " expected "\" on " violating > "/dev/stderr"
          exit 1
        }
        exit 0
      }
    ' "$blocks_file"
}

# assert_label_not_model <blocks-file> <label-regex> <forbidden-model-literal>
#
# The inverse of assert_label_model: for every block whose `label:` line
# matches <label-regex>, assert the block does NOT contain the literal
# substring "model: <forbidden-model-literal>" — guarding against a future
# over-eager repoint that accidentally pins a judgment stage to the
# mechanical tier. Also fails if no block matched the label (same reasoning
# as assert_label_model — a label typo must not silently pass).
assert_label_not_model() {
    blocks_file="$1"
    label_re="$2"
    forbidden="$3"
    awk -v label_re="$label_re" -v forbidden="$forbidden" '
      BEGIN { RS = "---END---"; matched = 0; bad = 0; violating = "" }
      $0 ~ ("label: .*" label_re) {
        matched++
        if (index($0, "model: " forbidden) != 0) {
          bad++
          if (violating == "") {
            n = split($0, lines, "\n")
            for (i = 1; i <= n; i++) {
              if (lines[i] ~ /label:/) { violating = lines[i]; break }
            }
          }
        }
      }
      END {
        if (matched == 0) {
          print "assert_label_not_model: no block matched label /" label_re "/" > "/dev/stderr"
          exit 1
        }
        if (bad > 0) {
          print "assert_label_not_model: unexpectedly found \"model: " forbidden "\" on " violating > "/dev/stderr"
          exit 1
        }
        exit 0
      }
    ' "$blocks_file"
}
