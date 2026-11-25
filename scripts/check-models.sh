#!/usr/bin/env bash
# Check the Quint models: the permission flow (SPEC 25), review close (SPEC 26),
# the load cycle of a local review (SPEC 26.1) several windows (SPEC 29, 41.8) and pairing (SPEC 41.4).
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
# Discard, a checkout over kept work, a forced removal of a locked worktree, a
# local review cleaned like a PR review. If one passes, the model lost the bug
# it is for.
for old in stepNoGate stepBoolDiscard stepNoGuard stepForceLock stepNoLocalGuard; do
    if quint run review_close.qnt --step="$old" --invariants Always \
        --max-samples="$samples" --max-steps="$steps" --verbosity=0 > /dev/null 2>&1; then
        echo "error: $old passed: the model no longer sees that bug" >&2
        exit 1
    fi
    echo "== $old fails, as it must"
done

# The load cycle of a local review (SPEC 26.1): claims, loads, the poll, turns,
# close, a quit and the start. The fixed traces end in a bad state for the old
# code and a good one for the fix.
quint typecheck local_load.qnt
quint typecheck local_load_test.qnt
quint test local_load_test.qnt
for st in step stepPoll; do
    echo "== local_load $st: $samples runs of $steps steps"
    quint run local_load.qnt --step="$st" --invariants Always \
        --max-samples="$samples" --max-steps="$steps" --verbosity=1
done
local_must_fail() {
    if quint run local_load.qnt --step="$1" --invariant="$2" --max-samples="$samples" \
        --max-steps="$steps" --verbosity=0 > /dev/null 2>&1; then
        echo "error: local_load $1 $2 passed: the model lost that path" >&2
        exit 1
    fi
    echo "== local_load $1 $2 fails, as it must"
}
# The witnesses: the runs reach each path.
for w in NoKeptTurn NoMarkRef NoNewChanges NoSweepOfARef; do
    local_must_fail step "$w"
done
# The code before each fix: the old claim rule, the old poll, no sweep of the
# refs, no repair of the head ref, no look at start.
for old in stepOldClaim stepPollOldTick stepNoSweep stepNoRepair stepNoLook; do
    local_must_fail "$old" Always
done

# Several windows on one hub (SPEC 29, 41.8): screens per window, the screen
# lock, lost links, drops and fresh screens from the emulator. The witnesses must
# fail: else the runs never reach a fresh screen, a fresh screen of a shared pane,
# a quiet state with two windows on one pane, or a replay of missed events. The
# quiet state after a fresh screen of a shared pane is a fixed trace of the tests:
# random runs are too rarely there. The
# old code must fail too: commands in threads of their own, any expand answer that
# draws, a collapse with no screen lock, a fresh screen that pauses the pane, and
# a replay and a join with no log lock.
quint typecheck windows.qnt
quint typecheck windows_test.qnt
quint test windows_test.qnt
echo "== windows step: $samples runs of $steps steps"
quint run windows.qnt --step=step --invariants Always \
    --max-samples="$samples" --max-steps="$steps" --verbosity=1
must_fail() {
    if quint run windows.qnt "$@" --max-samples="$samples" --max-steps="$steps" \
        --verbosity=0 > /dev/null 2>&1; then
        echo "error: windows $* passed: the model lost that path" >&2
        exit 1
    fi
    echo "== windows $* fails, as it must"
}
for w in NoResync NoSharedResync NoQuietShare NoReplay; do
    must_fail --step=step --invariant="$w"
done
must_fail --step=stepAnyOrder --invariant=Always
must_fail --init=initAnyAnswer --step=step --invariant=Always
must_fail --step=stepNoScreenLock --invariant=Always
must_fail --step=stepPause --invariant=Always
must_fail --step=stepNoLogLock --invariant=Always

# Pairing, sessions and removal of devices (SPEC 41.4). Each old step has one
# check off, and must break the invariant of that check.
quint typecheck pairing.qnt
echo "== pairing step: $samples runs of $steps steps"
quint run pairing.qnt --step=step --invariants Always \
    --max-samples="$samples" --max-steps="$steps" --verbosity=1
pairing_must_fail() {
    if quint run pairing.qnt --step="$1" --invariant="$2" --max-samples="$samples" \
        --max-steps="$steps" --verbosity=0 > /dev/null 2>&1; then
        echo "error: pairing $1 $2 passed: the model lost that path" >&2
        exit 1
    fi
    echo "== pairing $1 $2 fails, as it must"
}
for w in NoRun NoRaceClosed NoReturn; do
    pairing_must_fail step "$w"
done
pairing_must_fail stepManyUses OneUse
pairing_must_fail stepNoMode OnlyInMode
pairing_must_fail stepPairAgain NeverAgain
pairing_must_fail stepNoCheck NothingLate
echo "models ok"
