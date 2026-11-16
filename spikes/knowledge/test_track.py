#!/usr/bin/env python3
"""Random check of the line tracker against real git diffs of random files.

A mapped line must hold the same text, the mapped lines must keep their order,
and the count of mapped lines must be the old count minus the removed lines.
"""
import os
import random
import subprocess
import tempfile

from track import HUNK, map_line, window_changed


def parse(out):
    hs = []
    for line in out.splitlines():
        m = HUNK.match(line)
        if m:
            hs.append((int(m.group(1)), 1 if m.group(2) is None else int(m.group(2)),
                       int(m.group(3)), 1 if m.group(4) is None else int(m.group(4))))
    return hs


def main():
    d = tempfile.mkdtemp()
    a, b = os.path.join(d, "a"), os.path.join(d, "b")
    random.seed(1)
    bad = 0
    for _ in range(400):
        old = [random.choice("abcdefgh") for _ in range(random.randint(0, 30))]
        new = list(old)
        for _ in range(random.randint(0, 6)):
            op = random.random()
            if op < 0.4 and new:
                del new[random.randrange(len(new))]
            elif op < 0.7:
                new.insert(random.randint(0, len(new)), random.choice("xyz"))
            elif new:
                new[random.randrange(len(new))] = random.choice("xyz")
        with open(a, "w") as f:
            f.write("".join(c + "\n" for c in old))
        with open(b, "w") as f:
            f.write("".join(c + "\n" for c in new))
        out = subprocess.run(["git", "diff", "--no-index", "-U0", "--diff-algorithm=histogram", a, b],
                             capture_output=True, text=True).stdout
        hs = parse(out)
        removed = sum(oc for _, oc, _, _ in hs)
        added = sum(nc for _, _, _, nc in hs)
        mapped = [(n, map_line(hs, n)) for n in range(1, len(old) + 1)]
        got = [m for _, m in mapped if m is not None]
        ok = len(got) == len(old) - removed and len(new) == len(old) - removed + added
        ok = ok and all(old[n - 1] == new[m - 1] for n, m in mapped if m is not None)
        ok = ok and got == sorted(got)
        # A window over the whole file changed exactly when the diff is not empty.
        if old:
            ok = ok and window_changed(hs, 1, len(old)) == (bool(hs) and not all(
                oc == 0 and (os_ == 0 or os_ == len(old)) for os_, oc, _, _ in hs))
        bad += 0 if ok else 1
    print("tracker random check: %d bad of 400" % bad)


if __name__ == "__main__":
    main()
