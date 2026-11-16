#!/usr/bin/env python3
"""A second acted_on rule (round 2), checked against the 50 labels of round 1.

  actedon2.py compute   work out the new rule for every line thread; write acted2.json
  actedon2.py check     compare old and new rules with labels.csv; write labels_round2.csv

New rule: a thread is acted on when
  - a PR commit after the thread's original commit (merge commits skipped, so code that came
    in from the base branch does not count) changed lines in the thread's function, or within
    15 lines when no function is found; or
  - the PR's author replied with "done", "fixed", "updated" or such words; or
  - a suggestion in the thread landed.
Variants are printed too, so each part can be seen on its own.
"""
import collections
import csv
import json
import os
import random
import re
import subprocess
import sys

from actedon import THREADS, have, last_commit, suggestion_landed
from common import DATA, git, load_prs
from track import hunks, new_path, window_changed

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(DATA, "acted2.json")
FN = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?((async|const|unsafe|extern\s+\"[^\"]*\")\s+)*fn\s+\w+")
DONE = re.compile(r"\b(done|fixed|updated|addressed|changed|applied|removed|renamed|good (catch|call|point)|"
                  r"fixed in|reverted|added)\b", re.I)
WIDE = 15
FN_MAX = 120

_parents = {}


def parents(oid):
    if oid not in _parents:
        _parents[oid] = git("rev-list", "--parents", "-n", "1", oid).split()[1:]
    return _parents[oid]


_files = {}


def file_at(oid, path):
    key = (oid, path)
    if key not in _files:
        _files[key] = git("show", "%s:%s" % (oid, path), check=False).splitlines()
    return _files[key]


def region(oid, path, lo, hi):
    """The thread's function at its original commit, as a line range; else lo-15..hi+15."""
    lines = file_at(oid, path)
    start = None
    for k in range(min(hi, len(lines)) - 1, -1, -1):
        if FN.match(lines[k]):
            start = k + 1
            break
    if start is None:
        return max(1, lo - WIDE), hi + WIDE, "window"
    end = len(lines)
    for k in range(max(start, hi), len(lines)):
        if FN.match(lines[k]):
            end = k
            break
    if end - start > FN_MAX:
        return max(1, lo - WIDE), hi + WIDE, "window (long fn)"
    return start, end, "function"


def shift_before(hs, x):
    s = 0
    for os_, oc, ns, nc in hs:
        end = os_ if oc == 0 else os_ + oc - 1
        if (oc == 0 and os_ < x) or (oc > 0 and end < x):
            s += nc - oc
    return s


def commits_after(p, oc):
    oids = [c["oid"] for c in p["commits"]]
    if oc not in oids:
        return None
    return oids[oids.index(oc) + 1:]


def touched(p, t, lo, hi):
    """Did a non-merge PR commit after the original commit change lines in [lo, hi]?
    Returns True, False, or None when the original commit is not in the PR's commit list."""
    oc = t["comments"][0]["originalCommit"]
    path = t["path"]
    after = commits_after(p, oc)
    if after is None:
        final = last_commit(p)
        pb = new_path(oc, final, path)
        if pb is None:
            return True
        return window_changed(hunks(oc, final, path, pb), lo, hi)
    when = {c["oid"]: c["committedDate"] for c in p["commits"]}
    said_at = t["comments"][0]["createdAt"]
    cur = oc
    for c in after:
        if not have(c):
            return None
        # Only a plain commit made after the comment counts; all commits move the lines.
        counts = len(parents(c)) == 1 and when.get(c, "") > said_at
        pb = new_path(cur, c, path)
        if pb is None:
            return counts
        hs = hunks(cur, c, path, pb)
        if counts and window_changed(hs, lo, hi):
            return True
        lo, hi = lo + shift_before(hs, lo), hi + shift_before(hs, hi + 1)
        cur, path = c, pb
    return False


def author_said_done(p, t):
    for c in t["comments"][1:]:
        if c["author"] == p["author"] and DONE.search(c["body"]):
            return True
    return False


def decide(p, t):
    """Returns a dict of the parts and the answers of each variant, or None for a thread with no line."""
    if t["subjectType"] != "LINE" or t["diffSide"] == "LEFT" or not t["comments"]:
        return None
    oc = t["comments"][0]["originalCommit"]
    final = last_commit(p)
    hi = t["originalLine"]
    if not oc or not final or hi is None or not have(oc) or not have(final):
        return None
    lo = t["originalStartLine"] or hi
    rlo, rhi, kind = region(oc, t["path"], lo, hi)
    fn = touched(p, t, rlo, rhi)
    near = touched(p, t, max(1, lo - WIDE), hi + WIDE)
    near3 = touched(p, t, max(1, lo - 3), hi + 3)
    said = author_said_done(p, t)
    pb = new_path(oc, final, t["path"])
    sugg = bool(pb) and suggestion_landed(t, final, pb)
    return {"fn": fn, "near15": near, "near3": near3, "said": said, "sugg": sugg, "region": kind,
            "acted": bool(fn or said or sugg)}


def compute():
    rows = []
    for p in load_prs():
        for i, t in enumerate(p["reviewThreads"]):
            d = decide(p, t)
            if d is None:
                rows.append({"pr": p["number"], "i": i, "acted": None})
                continue
            d.update({"pr": p["number"], "i": i})
            rows.append(d)
    with open(OUT, "w") as f:
        json.dump(rows, f)
    known = [r for r in rows if r["acted"] is not None]
    print("threads %d, with an answer %d, acted (new rule) %d" % (len(rows), len(known), sum(r["acted"] for r in known)))
    print("region kind:", dict(collections.Counter(r.get("region") for r in known)))


def find(prs, link, excerpt):
    m = re.match(r"https://github.com/sigp/lighthouse/pull/(\d+)/files (.*):(\d+)$", link)
    p = prs[int(m.group(1))]
    for i, t in enumerate(p["reviewThreads"]):
        if t["path"] == m.group(2) and str(t["originalLine"]) == m.group(3) and \
                " ".join(t["comments"][0]["body"].split())[:160] == excerpt:
            return p, i, t
    return p, None, None


def check():
    prs = {p["number"]: p for p in load_prs()}
    with open(os.path.join(HERE, "labels.csv")) as f:
        labels = list(csv.DictReader(f))
    variants = collections.OrderedDict([
        ("old: +-3 lines, net diff to final", lambda d, old: old),
        ("+-3 lines, PR commits only", lambda d, old: d["near3"]),
        ("+-15 lines, PR commits only", lambda d, old: d["near15"]),
        ("function, PR commits only", lambda d, old: d["fn"]),
        ("new: function or said done or suggestion", lambda d, old: d["acted"]),
        ("+-3 PR commits or said done or suggestion", lambda d, old: bool(d["near3"] or d["said"] or d["sugg"])),
        ("said done or suggestion only", lambda d, old: bool(d["said"] or d["sugg"])),
    ])
    cms = {k: collections.Counter() for k in variants}
    seen = set()
    for row in labels:
        p, i, t = find(prs, row["link"], row["comment"])
        if t is None:
            print("not found:", row["link"])
            continue
        seen.add((p["number"], i))
        d = decide(p, t)
        lab = row["label (model-made, sonnet)"] == "yes"
        old = row["heuristic"] == "yes"
        for k, f in variants.items():
            cms[k][(bool(f(d, old)), lab)] += 1
    print("| rule | yes/yes | yes/no | no/yes | no/no | accuracy |")
    print("|---|---|---|---|---|---|")
    for k, cm in cms.items():
        n = sum(cm.values())
        print("| %s | %d | %d | %d | %d | %.0f%% |" % (k, cm[(True, True)], cm[(True, False)], cm[(False, True)],
                                                     cm[(False, False)], 100.0 * (cm[(True, True)] + cm[(False, False)]) / n))
    # 20 new threads for a hand label, from all mined PRs, not among the 50.
    with open(OUT) as f:
        rows = [r for r in json.load(f) if r["acted"] is not None and (r["pr"], r["i"]) not in seen]
    rnd = random.Random(352)
    pick = rnd.sample(rows, 20)
    with open(os.path.join(HERE, "labels_round2.csv"), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["link", "comment excerpt", "heuristic answer (new rule)", "human"])
        for r in pick:
            p = prs[r["pr"]]
            t = p["reviewThreads"][r["i"]]
            w.writerow(["%s/files %s:%s" % (p["url"], t["path"], t["originalLine"]),
                        " ".join(t["comments"][0]["body"].split())[:200], "yes" if r["acted"] else "no", ""])
    print("wrote labels_round2.csv: 20 threads, %d yes" % sum(r["acted"] for r in pick))


if __name__ == "__main__":
    {"compute": compute, "check": check}[sys.argv[1]]()
