#!/usr/bin/env bash
# Check the Quint models: the permission flow (SPEC 25) and review close (SPEC 26).
# SAMPLES sets the number of random runs for each step profile (default 100000).
# STEPS sets the length of each run (default 50).
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root/models"

if ! command -v quint > /dev/null; then
    PATH="$HOME/.local/opt/node/bin:$PATH"
fi
if ! command -v quint > /dev/null; then
    echo "error: quint is not installed (npm i -g @informalsystems/quint)" >&2
    exit 1
fi

samples=${SAMPLES:-100000}
steps=${STEPS:-50}

quint typecheck permit.qnt
quint typecheck permit_test.qnt

# The fixed traces of the findings. The traces of fixed findings end in a good
# state. The traces of open findings (F3, F4) still end in a bad state; when one
# is fixed, its test fails: then change the model, the test and SPEC 25.
quint test permit_test.qnt

# Random runs. `step` has crashes, resumes and closes; `stepNoCrash` has none,
# so more of its runs reach a tile answer. The `Prompt` steps read each
# `Permit` line before its request ends (see `stepWith`): the tile checks need it.
check() {
    echo "== $2, $1: $samples runs of $steps steps"
    quint run permit.qnt --step="$1" --invariants $2 \
        --max-samples="$samples" --max-steps="$steps" --verbosity=1
}
check step Always
check stepNoCrash Always
check stepPrompt "Always Tiles"
check stepPromptNoCrash "Always Tiles"

# Review close and worktree cleanup (SPEC 26). The fixed traces include the old
# order, which must leak; the random runs then check the code's order.
quint typecheck review_close.qnt
quint typecheck review_close_test.qnt
quint test review_close_test.qnt
echo "== review_close step: $samples runs of $steps steps"
quint run review_close.qnt --step=step --invariants Always \
    --max-samples="$samples" --max-steps="$steps" --verbosity=1
# The code before each fix must fail: no open check at the checkout, a bool
# Discard, a checkout over kept work, a forced removal of a locked worktree. If
# one passes, the model lost the bug it is for.
for old in stepNoGate stepBoolDiscard stepNoGuard stepForceLock; do
    if quint run review_close.qnt --step="$old" --invariants Always \
        --max-samples="$samples" --max-steps="$steps" --verbosity=0 > /dev/null 2>&1; then
        echo "error: $old passed: the model no longer sees that bug" >&2
        exit 1
    fi
    echo "== $old fails, as it must"
done
echo "models ok"
