#!/usr/bin/env bash
# Install Charon and Aeneas at the commits in proofs/TOOLS, into ~/verif (or
# $VERIF_DIR), for scripts/extract.sh and scripts/check-proofs.sh.
# Needs: git, rustup, opam, GNU make, and libgmp. On macOS: brew install opam make gmp.
# On Debian or Ubuntu: apt-get install opam libgmp-dev pkg-config.
# Lean comes from elan: the version is in proofs/lean-toolchain.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
dir=${VERIF_DIR:-$HOME/verif}
charon_rev=$(awk '$1 == "charon" {print $2}' "$root/proofs/TOOLS")
aeneas_rev=$(awk '$1 == "aeneas" {print $2}' "$root/proofs/TOOLS")
switch=${OPAM_SWITCH:-aeneas}
make=make
if command -v gmake > /dev/null; then make=gmake; fi

mkdir -p "$dir"
fetch() {
    if [ ! -d "$dir/$1" ]; then
        git clone -q "https://github.com/AeneasVerif/$1" "$dir/$1"
    fi
    git -C "$dir/$1" fetch -q origin
    git -C "$dir/$1" checkout -q "$2"
}
fetch charon "$charon_rev"
fetch aeneas "$aeneas_rev"
if [ "$(cat "$dir/aeneas/charon-pin" | grep -v '^#')" != "$charon_rev" ]; then
    echo "error: aeneas $aeneas_rev wants another charon (see its charon-pin)" >&2
    exit 1
fi

# Charon: a Rust program on a pinned nightly (charon/rust-toolchain). Its build runs
# cargo fmt, and charon-driver links that nightly's rustc.
tc=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$dir/charon/rust-toolchain")
rustup toolchain install "$tc" --profile minimal --component rustc-dev,llvm-tools,rust-src,miri,rustfmt
(cd "$dir/charon" && $make build-charon-rust)

# Aeneas: OCaml 5, with the Charon OCaml library.
if [ ! -d "$HOME/.opam" ]; then
    opam init -y --bare --disable-sandboxing
fi
if ! opam switch list --short | grep -qx "$switch"; then
    opam switch create "$switch" 5.3.0 -y
fi
opam install --switch="$switch" -y calendar core_unix domainslib easy_logging menhir \
    ocamlformat.0.27.0 ocamlgraph odoc ppx_deriving ppx_deriving_yojson progress \
    unionFind visitors yojson zarith
ln -sfn "$dir/charon" "$dir/aeneas/charon"
(cd "$dir/charon" && opam exec --switch="$switch" -- $make build-charon-ml)
(cd "$dir/aeneas" && opam exec --switch="$switch" -- $make build-bin-dir)
echo "charon $charon_rev and aeneas $aeneas_rev are in $dir"
