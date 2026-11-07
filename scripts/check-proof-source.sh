#!/usr/bin/env bash
# A cheap check, with no proof tools: the Lean code of each verified crate
# (proofs/*/Code) came from the Rust that is in the tree now. scripts/extract.sh
# writes the hash of the Rust next to the code. If a crate changed and nobody ran
# scripts/check-proofs.sh, the hashes differ.
# `--print <crate>` prints the hash lines of one crate.
set -euo pipefail

root=$(git rev-parse --show-toplevel)

# Every Rust file that Charon translates: the crate source without its tests.
hashes() {
    (cd "$root/crates/$1" && find src -name '*.rs' ! -name 'tests.rs' | LC_ALL=C sort | xargs shasum -a 256)
}

if [ "${1:-}" = "--print" ]; then
    hashes "$2"
    exit 0
fi

bad=0
while read -r crate lean; do
    if ! diff <(hashes "$crate") "$root/proofs/$lean/Code/Source.sha256" > /dev/null 2>&1; then
        echo "error: crates/$crate changed, but proofs/$lean/Code did not. Run scripts/check-proofs.sh and commit." >&2
        bad=1
    fi
done < <(grep -v '^#' "$root/proofs/CRATES")
exit $bad
