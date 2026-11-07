#!/usr/bin/env bash
# Check that the proofs match the Rust code of the verified crates (proofs/CRATES) and hold.
set -euo pipefail

root=$(git rev-parse --show-toplevel)

# The generated Lean must match the current Rust.
"$root/scripts/extract.sh" > /dev/null
leans=$(grep -v '^#' "$root/proofs/CRATES" | awk '{print $2}')
for lean in $leans; do
    code=proofs/$lean/Code
    if ! git -C "$root" diff --quiet -- "$code" || [ -n "$(git -C "$root" ls-files --others --exclude-standard -- "$code")" ]; then
        echo "error: $code is out of date. Run scripts/extract.sh and commit." >&2
        exit 1
    fi
done

cd "$root/proofs"
files=Statements.lean
for lean in $leans; do files="$files $(ls "$lean"/*.lean)"; done
# shellcheck disable=SC2086
if grep -rnw 'sorry\|admit\|native_decide\|bv_decide' $files; then
    echo "error: a proof uses sorry, admit, or native code" >&2
    exit 1
fi
lake build

# Every approved statement needs its axiom check.
checks=$(grep -oE '^theorem check_[A-Za-z0-9_]+' Statements.lean | sed 's/theorem //' | sort)
printed=$(grep -oE 'Statements\.check_[A-Za-z0-9_]+' Axioms.lean | sed 's/Statements\.//' | sort)
if [ "$checks" != "$printed" ]; then
    echo "error: Axioms.lean must print the axioms of every check_ theorem:" >&2
    diff <(echo "$checks") <(echo "$printed") >&2 || true
    exit 1
fi

# Only the three standard axioms.
lake env lean Axioms.lean | python3 -c '
import re, sys
allowed = {"propext", "Classical.choice", "Quot.sound"}
text = sys.stdin.read()
found = re.findall(r"\x27(\S+)\x27 depends on axioms: \[([^\]]*)\]", text, re.S)
found += [(n, "") for n in re.findall(r"\x27(\S+)\x27 does not depend on any axioms", text)]
bad = not found
for name, axioms in found:
    extra = {a.strip() for a in axioms.split(",") if a.strip()} - allowed
    if extra:
        print(f"error: {name} depends on {sorted(extra)}", file=sys.stderr)
        bad = True
sys.exit(1 if bad else 0)
'
echo "proofs ok"
