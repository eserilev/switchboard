"""A plain line tracker: map lines of a file from one commit to another.

It reads `git diff -U0 --diff-algorithm=histogram -M50%` and walks the hunks.
Old line L maps to None when a hunk removed or changed it, else it shifts by
the lines added minus removed in the hunks before it.
"""
import re

from common import git

HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")
_names = {}


def new_path(a, b, path):
    """The path of `path` at commit b, following a rename. None when deleted."""
    key = (a, b)
    if key not in _names:
        out = git("diff", "--name-status", "-M50%", a, b)
        m = {}
        for line in out.splitlines():
            parts = line.split("\t")
            st = parts[0]
            if st.startswith("R"):
                m[parts[1]] = parts[2]
            elif st == "D":
                m[parts[1]] = None
        _names[key] = m
    return _names[key].get(path, path)


def hunks(a, b, path, path_b=None):
    """List of (old_start, old_count, new_start, new_count) for one file."""
    path_b = path_b or path
    args = ["diff", "-U0", "--diff-algorithm=histogram", "-M50%", a, b, "--", path]
    if path_b != path:
        args.append(path_b)
    out = git(*args)
    hs = []
    for line in out.splitlines():
        m = HUNK.match(line)
        if m:
            oc = 1 if m.group(2) is None else int(m.group(2))
            nc = 1 if m.group(4) is None else int(m.group(4))
            hs.append((int(m.group(1)), oc, int(m.group(3)), nc))
    return hs


def map_line(hs, line):
    """New line number of old `line`, or None when it was removed or changed."""
    shift = 0
    for os_, oc, ns, nc in hs:
        if oc == 0:
            # A pure insert after old line os_.
            if os_ < line:
                shift += nc
            continue
        if line < os_:
            break
        if line < os_ + oc:
            return None
        shift += nc - oc
    return line + shift


def window_changed(hs, lo, hi):
    """True when any old line in [lo, hi] was removed or changed, or lines were added inside it."""
    for os_, oc, ns, nc in hs:
        if oc == 0:
            if lo <= os_ < hi:
                return True
            continue
        if os_ <= hi and os_ + oc - 1 >= lo:
            return True
    return False
