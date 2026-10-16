#!/usr/bin/env bash
# Translate the verified crates (guide-check, outq) to Lean with Charon and Aeneas.
# Set CHARON_DIR and AENEAS_DIR if the tools are not in ~/verif.
# The commits that the proofs expect are in proofs/TOOLS.
set -euo pipefail

charon_dir=${CHARON_DIR:-$HOME/verif/charon}
aeneas_dir=${AENEAS_DIR:-$HOME/verif/aeneas}
root=$(git rev-parse --show-toplevel)
target=${CARGO_TARGET_DIR:-$root/target}

# Charon gets a sysroot and a cache of its own, so another nightly cannot confuse it.
export MIRI_SYSROOT=$HOME/.cache/charon-guide-check/miri
export CHARON_CACHE_DIR=$HOME/.cache/charon-guide-check

mkdir -p "$target"
# Each verified crate and the Lean folder of its code.
for pair in "guide-check:GuideCheck" "outq:OutQ"; do
    crate=${pair%%:*}
    lean=${pair##*:}
    llbc=$target/${crate//-/_}.llbc
    # Start clean, so a file from an older run cannot hide a problem.
    rm -rf "$root/proofs/$lean/Code"
    (cd "$root/crates/$crate" && PATH=$charon_dir/bin:$PATH charon cargo --preset=aeneas --dest-file="$llbc")
    "$aeneas_dir/bin/aeneas" -backend lean "$llbc" -dest "$root/proofs" -subdir "/$lean/Code" -split-files

    # An axiom means Aeneas did not know a function. A sorry means it could not
    # translate a body. Either way the proofs would trust code that nobody checked.
    if grep -rnw 'axiom\|sorry' "$root/proofs/$lean/Code"; then
        echo "error: the generated code of $crate contains an axiom or a sorry" >&2
        exit 1
    fi
done
