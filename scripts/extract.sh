#!/usr/bin/env bash
# Translate the verified crates (proofs/CRATES) to Lean with Charon and Aeneas.
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
while read -r crate lean; do
    llbc=$target/${crate//-/_}.llbc
    # Start clean, so a file from an older run cannot hide a problem.
    rm -rf "$root/proofs/$lean/Code"
    (cd "$root/crates/$crate" && PATH=$charon_dir/bin:$PATH charon cargo --preset=aeneas --dest-file="$llbc" < /dev/null)
    "$aeneas_dir/bin/aeneas" -backend lean "$llbc" -dest "$root/proofs" -subdir "/$lean/Code" -split-files < /dev/null

    # An axiom means Aeneas did not know a function. A sorry means it could not
    # translate a body. Either way the proofs would trust code that nobody checked.
    if grep -rnw 'axiom\|sorry' "$root/proofs/$lean/Code"; then
        echo "error: the generated code of $crate contains an axiom or a sorry" >&2
        exit 1
    fi

    # The hash of the Rust that the code came from, so a cheap check without the
    # tools can see a crate change with no new Lean (scripts/check-proof-source.sh).
    "$root/scripts/check-proof-source.sh" --print "$crate" > "$root/proofs/$lean/Code/Source.sha256"
done < <(grep -v '^#' "$root/proofs/CRATES")
