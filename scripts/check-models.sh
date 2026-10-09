#!/usr/bin/env bash
# Check the Quint model of the permission flow (SPEC 25).
# SAMPLES sets the number of random runs for each step profile (default 200000).
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

samples=${SAMPLES:-200000}
steps=${STEPS:-50}

quint typecheck permit.qnt
quint typecheck permit_test.qnt

# The fixed traces of the known findings. A test fails when a finding is fixed:
# then change the model, the test and SPEC 25.
quint test permit_test.qnt

# P1 to P6 in random runs. `step` has crashes, resumes and closes.
# `stepNoCrash` has none, so more of its runs reach a tile answer.
for step in step stepNoCrash; do
    echo "== Safety, $step: $samples runs of $steps steps"
    quint run permit.qnt --step="$step" --invariant=Safety \
        --max-samples="$samples" --max-steps="$steps" --verbosity=1
done
echo "models ok"
