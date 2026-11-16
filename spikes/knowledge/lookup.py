#!/usr/bin/env python3
"""Phase 0 lookup test: can a lookup keyed on the lines under review find the past thread
that raised the same concern? Retrieval only, no Claude.

Ground truth: the ceiling judge of deep.py (deep/ceiling.json). For each of the 30 replay2 PRs,
the lookup runs once per changed hunk (and once per changed file) of base..snapshot. A target
counts as found at k when one of its matching past threads is in the top k for the hunk that
holds the target's line, or the nearest hunk in that file.

  lookup.py run      compute and save deep/lookup.json, print the report
  lookup.py report   print the report from deep/lookup.json
  lookup.py judge    sonnet judges the top-10 candidates the ceiling never saw (at most 10 runs),
                     then `report` also scores against the old and new matches together

Methods, each a ranked list per hunk:
  L1   same file, by weight
  L2   same file, line mapped to the PR base, within 20 lines of the hunk; by distance, weight
  L2s  as L2, but a line that changed goes to the start of the code that replaced it
  L3   same file, same function (hunk header or enclosing fn); by weight
  L4f  BM25, hunk text against thread text, same file only
  L4r  BM25, whole repo
  L5   L2s, then L3, then L4f, then L4r, cut at k
Only past threads whose first comment is before the PR was created. Weight is as of that time.
"""
import collections
import concurrent.futures
import datetime
import json
import math
import os
import re
import sys

from common import BARE, DATA, git, load_prs
from track import hunks as line_hunks, map_line
from replay2 import load_pick, rows_by_pr, ts

DEEP = os.path.join(DATA, "deep")
OUT = os.path.join(DEEP, "lookup.json")
NEAR = 20
KS = [5, 10, 30]
METHODS = ["L1", "L2", "L2s", "L3", "L4f", "L4r", "L5"]
HDR = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@ ?(.*)$")
FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
FN_DEF = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]*\"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)")
TOK = re.compile(r"[A-Za-z][A-Za-z0-9_]*")
STOP = set("""the and for this that with from not are but was you can have has its let mut pub self
use impl return true false none some into new let type where crate super struct enum match else
then than also just should would could will been being what when which there their they them
our out all any one get set more less only like here very much what how why does did doing done
str string usize u64 u8 u32 i64 vec option result ok err clone ref""".split())


# ---------------------------------------------------------------- text

def toks(s):
    out = []
    for w in TOK.findall(s or ""):
        parts = [w] + [x for x in re.split(r"_|(?<=[a-z0-9])(?=[A-Z])", w) if x]
        for x in (parts if len(parts) > 2 else [w]):
            x = x.lower()
            if len(x) >= 3 and x not in STOP:
                out.append(x)
    return out


class BM25:
    def __init__(self, docs, k1=1.2, b=0.75):
        self.tf = {k: collections.Counter(v) for k, v in docs.items()}
        self.len = {k: len(v) for k, v in docs.items()}
        self.avg = (sum(self.len.values()) / len(docs)) if docs else 1.0
        df = collections.Counter()
        for c in self.tf.values():
            df.update(c.keys())
        n = len(docs)
        self.idf = {w: math.log(1 + (n - d + 0.5) / (d + 0.5)) for w, d in df.items()}
        self.k1, self.b = k1, b

    def score(self, q, key):
        tf, ln = self.tf[key], self.len[key]
        s = 0.0
        for w in set(q):
            f = tf.get(w)
            if f:
                s += self.idf[w] * f * (self.k1 + 1) / (f + self.k1 * (1 - self.b + self.b * ln / self.avg))
        return s


# ---------------------------------------------------------------- git helpers

_show = {}


def show(commit, path):
    key = (commit, path)
    if key not in _show:
        try:
            _show[key] = git("show", "%s:%s" % (commit, path)).splitlines()
        except RuntimeError:
            _show[key] = None
    return _show[key]


def enclosing_fn(lines, line):
    """The name of the nearest `fn` at or above line (1-based). Naive, Rust only."""
    if not lines:
        return None
    i = min(line, len(lines)) - 1
    while i >= 0:
        m = FN_DEF.match(lines[i])
        if m:
            return m.group(1)
        i -= 1
    return None


def pr_diff(base, snap):
    """[{old, new, hunks: [{os, oc, ns, nc, ctx, text, fns}]}] from git diff -U3."""
    out = git("diff", "-U3", "--diff-algorithm=histogram", "-M50%", base, snap)
    files, cur, h = [], None, None
    for line in out.splitlines():
        if line.startswith("diff --git "):
            cur = {"old": None, "new": None, "hunks": []}
            files.append(cur)
            h = None
        elif cur is not None and h is None and line.startswith("--- "):
            cur["old"] = None if line[4:] == "/dev/null" else line[6:]
        elif cur is not None and h is None and line.startswith("+++ "):
            cur["new"] = None if line[4:] == "/dev/null" else line[6:]
        elif line.startswith("@@"):
            m = HDR.match(line)
            h = {"os": int(m.group(1)), "oc": 1 if m.group(2) is None else int(m.group(2)),
                 "ns": int(m.group(3)), "nc": 1 if m.group(4) is None else int(m.group(4)),
                 "ctx": m.group(5), "lines": []}
            cur["hunks"].append(h)
        elif h is not None and line[:1] in "+- ":
            h["lines"].append(line)
    return [f for f in files if f["new"] and f["hunks"]]


# ---------------------------------------------------------------- the pool

def weight_at(r, when, cut):
    now_age = (datetime.datetime(2026, 11, 16) - when).days / 365.25
    years = (cut - when).days / 365.25
    return r["weight"] / (0.5 ** max(0.0, now_age - 1)) * (0.5 ** max(0.0, years - 1))


def thread_info(prs, by):
    """Every weighted thread: fixed facts that do not depend on the PR under review."""
    out = {}
    for q in prs:
        for r in by.get(q["number"], []):
            if r["weight"] <= 0:
                continue
            t = q["reviewThreads"][r["i"]]
            c0 = t["comments"][0]
            m = FN.search(c0["diffHunk"].splitlines()[0] if c0["diffHunk"] else "")
            out[(q["number"], r["i"])] = {
                "pr": q, "t": t, "r": r, "path": t["path"], "first": ts(c0["createdAt"]),
                "merged": ts(q["mergedAt"]), "merge": (q.get("mergeCommit") or {}).get("oid") if isinstance(q.get("mergeCommit"), dict) else q.get("mergeCommit"),
                "hdr_fn": m.group(1) if m else None, "oc": c0["originalCommit"],
                "line": t["originalLine"], "lo": t["originalStartLine"] or t["originalLine"],
                "side": t["diffSide"]}
    return out


def card(info, cut):
    """The text the agent would see for one past thread, as in replay2's block."""
    t = info["t"]
    cs = [x for x in t["comments"] if ts(x["createdAt"]) < cut]
    hunk = "\n".join(t["comments"][0]["diffHunk"].splitlines()[-8:])
    conv = "\n".join("%s: %s" % (x["author"], " ".join(x["body"].split())[:500]) for x in cs[:3])
    return hunk, conv


_fnc = {}


def thread_fns(key, info):
    """Function names for a past thread: its hunk header and the enclosing fn at its commit."""
    if key not in _fnc:
        s = set()
        if info["hdr_fn"]:
            s.add(info["hdr_fn"])
        if info["oc"] and info["line"] and info["side"] == "RIGHT":
            f = enclosing_fn(show(info["oc"], info["path"]), info["line"])
            if f:
                s.add(f)
        _fnc[key] = s
    return _fnc[key]


_maps = {}


def soft_line(hs, line):
    """Like map_line, but a removed or changed line goes to the start of the code that replaced it."""
    shift = 0
    for os_, oc, ns, nc in hs:
        if oc == 0:
            if os_ < line:
                shift += nc
            continue
        if line < os_:
            break
        if line < os_ + oc:
            return ns
        shift += nc - oc
    return line + shift


def map_range(a, b, pa, pb, lo, hi, soft=False):
    key = (a, b, pa, pb)
    if key not in _maps:
        try:
            _maps[key] = line_hunks(a, b, pa, pb) if a != b else []
        except RuntimeError:
            _maps[key] = None
    hs = _maps[key]
    if hs is None:
        return None
    f = soft_line if soft else map_line
    x, y = f(hs, lo), f(hs, hi)
    if x is None and y is None:
        return None
    return (x if x is not None else y, y if y is not None else x)


_names = {}


def renames(a, b):
    key = (a, b)
    if key not in _names:
        m = {}
        try:
            for line in git("diff", "--name-status", "-M50%", a, b).splitlines():
                parts = line.split("\t")
                if parts[0].startswith("R"):
                    m[parts[1]] = parts[2]
                elif parts[0] == "D":
                    m[parts[1]] = None
        except RuntimeError:
            pass
        _names[key] = m
    return _names[key]


def at_base(info, base, base_files, cut, soft=False):
    """(path at base, (lo, hi) at base or None, why). Maps original commit -> merge commit
    (when merged before the cut) -> base. A rename is looked for only when the path is gone."""
    path = info["path"]
    steps = []
    if info["merge"] and info["merged"] < cut:
        steps.append(info["merge"])
    steps.append(base)
    if info["side"] != "RIGHT" or not info["oc"] or not info["line"]:
        p = path if path in base_files else renames(info["oc"] or steps[0], base).get(path, path)
        return p, None, "left side or no line"
    a, rng, p = info["oc"], (info["lo"], info["line"]), path
    for b in steps:
        nb = p
        if b == base and p not in base_files:
            nb = renames(a, b).get(p, p)
            if nb is None:
                return None, None, "file deleted"
        elif b != base:
            nb = renames(a, b).get(p, p) if a != b else p
            if nb is None:
                return None, None, "file deleted"
        if rng is not None:
            rng = map_range(a, b, p, nb, rng[0], rng[1], soft)
        a, p = b, nb
        if rng is None:
            return p, None, "lines gone"
    return p, rng, "mapped"


# ---------------------------------------------------------------- one PR

def hunk_query(f, h, snap_lines):
    fns = set()
    m = FN.search(h["ctx"] or "")
    if m:
        fns.add(m.group(1))
    for l in h["lines"]:
        m = FN_DEF.match(l[1:])
        if m:
            fns.add(m.group(1))
    e = enclosing_fn(snap_lines, h["ns"] + 3 if h["nc"] > 3 else h["ns"])
    if e:
        fns.add(e)
    text = " ".join(l[1:] for l in h["lines"] if l[:1] in "+-")
    return fns, toks(text + " " + " ".join(fns) + " " + (h["ctx"] or ""))


def one_pr(c, p, info, base_files):
    cut = ts(p["createdAt"])
    base, snap = c["base"], c["snap"]
    pool = [k for k, v in info.items() if v["first"] < cut and k[0] != p["number"]]
    files = pr_diff(base, snap)
    changed_old = {f["old"]: f for f in files if f["old"]}
    changed_new = {f["new"]: f for f in files}
    # Map each pool thread on a changed file to the base.
    where = {}
    for k in pool:
        v = info[k]
        maybe = v["path"] in changed_old or v["path"] in changed_new or v["path"] not in base_files
        if not maybe:
            continue
        pb, rng, why = at_base(v, base, base_files, cut)
        f = changed_old.get(pb) if pb else None
        if f is None and pb in changed_new:
            f = changed_new[pb]
        if f is None and v["path"] in changed_new:
            f = changed_new[v["path"]]
        if f is not None:
            soft = at_base(v, base, base_files, cut, True)[1]
            where[k] = {"file": f["new"], "rng": rng, "why": why, "soft": soft}
    # BM25 over the whole pool.
    docs = {}
    for k in pool:
        t = info[k]["t"]
        cs = [x for x in t["comments"] if ts(x["createdAt"]) < cut]
        docs[k] = toks(" ".join(x["body"] for x in cs[:3]) + " " + t["comments"][0]["diffHunk"][-1500:])
    bm = BM25(docs)
    w = {k: weight_at(info[k]["r"], info[k]["first"], cut) for k in pool}
    by_file = collections.defaultdict(list)
    for k, v in where.items():
        by_file[v["file"]].append(k)
    units = []  # one per hunk, then one per file
    for f in files:
        snap_lines = show(snap, f["new"])
        same = sorted(by_file.get(f["new"], []), key=lambda k: (-w[k], -info[k]["first"].timestamp()))
        hs = []
        for h in f["hunks"]:
            fns, q = hunk_query(f, h, snap_lines)
            hs.append({"h": h, "fns": fns, "q": q})
        for gran, groups in (("hunk", [[x] for x in hs]), ("file", [hs])):
            for g in groups:
                fns = set().union(*[x["fns"] for x in g])
                q = [t for x in g for t in x["q"]]
                lists = {"L1": same}
                for m, field in (("L2", "rng"), ("L2s", "soft")):
                    near = []
                    for k in same:
                        rng = where[k][field]
                        if rng is None:
                            continue
                        d = min(max(0, x["h"]["os"] - rng[1], rng[0] - (x["h"]["os"] + max(x["h"]["oc"], 1) - 1)) for x in g)
                        if d <= NEAR:
                            near.append((d, -w[k], k))
                    lists[m] = [k for _, _, k in sorted(near)]
                lists["L3"] = [k for k in same if thread_fns(k, info[k]) & fns]
                sf = [(-bm.score(q, k), k) for k in same]
                lists["L4f"] = [k for s, k in sorted(sf) if s < 0]
                sr = sorted(((-bm.score(q, k), k) for k in pool), key=lambda x: x[0])
                lists["L4r"] = [k for s, k in sr[:200] if s < 0]
                l5 = []
                for m in ("L2s", "L3", "L4f", "L4r"):
                    for k in lists[m]:
                        if k not in l5:
                            l5.append(k)
                lists["L5"] = l5[:200]
                units.append({"gran": gran, "file": f["new"],
                              "ranges": [(x["h"]["ns"], x["h"]["ns"] + max(x["h"]["nc"], 1) - 1) for x in g],
                              "lists": {m: [list(k) for k in v[:200]] for m, v in lists.items()}})
    words = {}
    for k in {tuple(k) for u in units for v in u["lists"].values() for k in v[:60]}:
        hunk, conv = card(info[k], cut)
        words["%d_%d" % k] = len((hunk + " " + conv).split())
    wh = {"%d_%d" % k: v for k, v in where.items()}
    return {"units": units, "words": words, "where": wh,
            "pool": len(pool), "hunks": sum(len(f["hunks"]) for f in files), "files": len(files)}


def truth(extended=False):
    """{pr: {target index: [keys]}} from the ceiling judge, as deep.py counts it.
    extended: add the matches of `lookup.py judge`."""
    extra = {}
    jp = os.path.join(DEEP, "lookup_judge.json")
    if extended and os.path.exists(jp):
        with open(jp) as f:
            extra = json.load(f)["answers"]
    with open(os.path.join(DEEP, "ceiling.json")) as f:
        st = json.load(f)
    out = {}
    for c in load_pick():
        a = st["answers"].get(str(c["number"]), {})
        cands = {q["id"]: q for q in st["prep"][str(c["number"])]["cands"]}
        out[c["number"]] = {}
        for ti, _ in enumerate(c["targets"]):
            same = ["%d_%d" % tuple(cands[x]["key"]) for x in a.get("t%d_%d" % (c["number"], ti), []) if x in cands]
            for x in extra.get("t%d_%d" % (c["number"], ti), []):
                if x[1:] not in same:
                    same.append(x[1:])
            if same:
                out[c["number"]][ti] = same
    return out


def run():
    prs = load_prs()
    by_num = {p["number"]: p for p in prs}
    info = thread_info(prs, rows_by_pr())
    res = {}

    def job(c):
        base_files = set(git("ls-tree", "-r", "--name-only", c["base"]).splitlines())
        return c["number"], one_pr(c, by_num[c["number"]], info, base_files)

    with concurrent.futures.ThreadPoolExecutor(6) as ex:
        for n, r in ex.map(job, load_pick()):
            res[str(n)] = r
            print("PR %d: files %d, hunks %d, pool %d, mapped on PR files %d" % (n, r["files"], r["hunks"], r["pool"], len(r["where"])), flush=True)
    with open(OUT, "w") as f:
        json.dump(res, f)
    report()


# ---------------------------------------------------------------- scoring

def unit_for(units, gran, path, line):
    us = [u for u in units if u["gran"] == gran and u["file"] == path]
    if not us:
        return None
    return min(us, key=lambda u: min(0 if a <= line <= b else min(abs(line - a), abs(line - b)) for a, b in u["ranges"]))


def report():
    with open(OUT) as f:
        res = json.load(f)
    chosen = load_pick()
    for ext in (False, True):
        if ext and not os.path.exists(os.path.join(DEEP, "lookup_judge.json")):
            break
        gt = truth(ext)
        nt = sum(len(v) for v in gt.values())
        print("\n==== ground truth: %s, %d targets with a match, %d pairs" % (
            "ceiling + lookup judge" if ext else "ceiling judge only", nt, sum(len(x) for v in gt.values() for x in v.values())))
        report_one(res, gt, chosen)
    misses()


def report_one(res, gt, chosen):
    rows = []
    for gran in ("hunk", "file"):
        for m in METHODS:
            found = {k: 0 for k in KS}
            pairs = {k: 0 for k in KS}
            anyw = {k: 0 for k in KS}
            n_t = n_p = 0
            size = {5: [0, 0], 10: [0, 0]}
            for c in chosen:
                r = res[str(c["number"])]
                units = r["units"]
                for ti, keys in gt[c["number"]].items():
                    t = c["targets"][ti]
                    u = unit_for(units, gran, t["path"], t["hi"])
                    n_t += 1
                    n_p += len(keys)
                    lst = ["%d_%d" % tuple(k) for k in u["lists"][m]] if u else []
                    for k in KS:
                        top = set(lst[:k])
                        if any(x in top for x in keys):
                            found[k] += 1
                        pairs[k] += sum(1 for x in keys if x in top)
                        allk = {"%d_%d" % tuple(y) for v in units if v["gran"] == gran for y in v["lists"][m][:k]}
                        if any(x in allk for x in keys):
                            anyw[k] += 1
                for k in (5, 10):
                    sel = {"%d_%d" % tuple(x) for u in units if u["gran"] == gran for x in u["lists"][m][:k]}
                    size[k][0] += len(sel)
                    size[k][1] += sum(r["words"][x] for x in sel)
            np_ = len(chosen)
            rows.append((gran, m, n_t, n_p, found, pairs, anyw, {k: (v[0] / np_, v[1] / np_) for k, v in size.items()}))
    print("granularity | method | found@5 | @10 | @30 | of 143 @5 | @10 | @30 | pairs @10 | anywhere in PR @5 | @10 | threads/PR k=5 | words/PR k=5 | threads/PR k=10 | words/PR k=10")
    for gran, m, n_t, n_p, found, pairs, anyw, size in rows:
        print("%s | %s | %d/%d | %d | %d | %.1f%% | %.1f%% | %.1f%% | %d/%d | %d | %d | %.0f | %.0f | %.0f | %.0f" % (
            gran, m, found[5], n_t, found[10], found[30], 100 * found[5] / 143, 100 * found[10] / 143, 100 * found[30] / 143,
            pairs[10], n_p, anyw[5], anyw[10], size[5][0], size[5][1], size[10][0], size[10][1]))
    sizes = [(res[str(c["number"])]["files"], res[str(c["number"])]["hunks"]) for c in chosen]
    print("files per PR mean %.1f, hunks per PR mean %.1f, max hunks %d" % (
        sum(a for a, _ in sizes) / len(sizes), sum(b for _, b in sizes) / len(sizes), max(b for _, b in sizes)))
    budget(res, gt, chosen)


def budget(res, gt, chosen):
    """A fixed budget per PR: take threads round robin over the units' lists until n are taken.
    Found when a match of the target is anywhere in the PR's set (the agent sees all of it)."""
    print("\nfixed budget per PR, round robin over units; found = a match anywhere in the PR's set")
    print("granularity | method | budget | found | of 143 | words/PR")
    for gran in ("hunk", "file"):
        for m in METHODS:
            for n in (15, 30, 45):
                found = words = 0
                for c in chosen:
                    r = res[str(c["number"])]
                    lists = [["%d_%d" % tuple(k) for k in u["lists"][m]] for u in r["units"] if u["gran"] == gran]
                    sel, i = [], 0
                    while len(sel) < n and any(i < len(x) for x in lists):
                        for x in lists:
                            if i < len(x) and x[i] not in sel and len(sel) < n:
                                sel.append(x[i])
                        i += 1
                    words += sum(r["words"].get(x, 0) for x in sel)
                    found += sum(1 for keys in gt[c["number"]].values() if any(x in sel for x in keys))
                print("%s | %s | %d | %d | %.1f%% | %.0f" % (gran, m, n, found, 100 * found / 143, words / len(chosen)))


def misses():
    """Why each (target, past thread) pair is missed by L5 at k = 10, per hunk."""
    with open(OUT) as f:
        res = json.load(f)
    gt = truth()
    prs = {p["number"]: p for p in load_prs()}
    why = collections.Counter()
    detail = []
    for c in load_pick():
        r = res[str(c["number"])]
        for ti, keys in gt[c["number"]].items():
            t = c["targets"][ti]
            u = unit_for(r["units"], "hunk", t["path"], t["hi"])
            ranks = {}
            for m in METHODS:
                lst = ["%d_%d" % tuple(k) for k in u["lists"][m]] if u else []
                ranks[m] = {x: (lst.index(x) + 1 if x in lst else None) for x in keys}
            hit = any(ranks["L5"][x] and ranks["L5"][x] <= 10 for x in keys)
            for x in keys:
                q, i = map(int, x.split("_"))
                pth = prs[q]["reviewThreads"][i]["path"]
                wh = r["where"].get(x)
                if u is None:
                    reason = "target file not in the snapshot diff"
                elif ranks["L5"][x] and ranks["L5"][x] <= 10:
                    reason = "found"
                elif wh is None:
                    if pth == t["path"]:
                        reason = "same path, not mapped to a changed file"
                    else:
                        reason = "other file (%s)" % ("L4r rank %s" % ranks["L4r"][x] if ranks["L4r"][x] else "L4r rank >200")
                elif wh["file"] != t["path"]:
                    reason = "other changed file of the PR"
                elif wh["soft"] is None:
                    reason = "same file, no line (%s)" % wh["why"]
                elif ranks["L2s"][x] is None:
                    reason = "same file, over 20 lines from the hunk"
                else:
                    reason = "same file, near, ranked below 10"
                detail.append((c["number"], ti, x, "target-hit" if hit else "target-miss", reason, pth, ranks))
                why[(reason.split(" (")[0], hit)] += 1
    print("\nper pair, L5 at k=10, per hunk (target hit = another match of the same target was found):")
    for (reason, hit), n in sorted(why.items(), key=lambda x: -x[1]):
        print("  %3d  %s%s" % (n, reason, "  [target found by another pair]" if hit and reason != "found" else ""))
    with open(os.path.join(DEEP, "lookup_misses.json"), "w") as f:
        json.dump(detail, f, indent=1, default=str)


JUDGE_CAP = 10


def judge():
    """The ceiling judge only saw past threads picked by word overlap with the target. Show sonnet
    every top-10 candidate of every method for the target's hunk and file that it never saw."""
    import common
    from common import claude, parse_json_block
    from deep import CEIL, SONNET
    common.COUNT = os.path.join(DATA, "claude_runs_lookup.txt")
    with open(OUT) as f:
        res = json.load(f)
    with open(os.path.join(DEEP, "ceiling.json")) as f:
        st = json.load(f)
    prs = {p["number"]: p for p in load_prs()}
    path = os.path.join(DEEP, "lookup_judge.json")
    state = {"answers": {}, "asked": {}}
    if os.path.exists(path):
        with open(path) as f:
            state = json.load(f)
    cases = []
    for c in load_pick():
        if str(c["number"]) in state["asked"]:
            continue
        r = res[str(c["number"])]
        cut = ts(prs[c["number"]]["createdAt"])
        seen = {"%d_%d" % tuple(q["key"]) for q in st["prep"][str(c["number"])]["cands"]}
        new = []
        for ti, t in enumerate(c["targets"]):
            for g in ("hunk", "file"):
                u = unit_for(r["units"], g, t["path"], t["hi"])
                for m in (METHODS if u else []):
                    for k in u["lists"][m][:10]:
                        x = "%d_%d" % tuple(k)
                        if x not in seen and x not in new:
                            new.append(x)
        tx = []
        for ti, t in enumerate(c["targets"]):
            conv = " | ".join("%s: %s" % (x["author"], " ".join(x["body"].split())[:500]) for x in t["conv"][:3])
            tx.append("[t%d_%d] %s line %d: %s" % (c["number"], ti, t["path"], t["hi"], conv))
        cx = []
        for x in new:
            q, i = map(int, x.split("_"))
            th = prs[q]["reviewThreads"][i]
            cs = [y for y in th["comments"] if ts(y["createdAt"]) < cut]
            text = " ".join(" ".join(y["body"].split()) for y in cs[:2])[:450]
            cx.append("[q%s] %s: %s" % (x, th["path"], text))
        cases.append((c["number"], new, "=== PR %d\n--- target threads\n%s\n--- past threads for this PR\n%s\n" % (
            c["number"], "\n".join(tx), "\n".join(cx))))
    batches, b, size = [], [], 0
    for case in cases:
        if b and size + len(case[2]) > 170000:
            batches.append(b)
            b, size = [], 0
        b.append(case)
        size += len(case[2])
    if b:
        batches.append(b)
    print("judge batches:", len(batches), [sum(len(x[2]) for x in b) for b in batches], "pairs to judge:",
          sum(len(x[1]) for x in cases), flush=True)
    if os.environ.get("DRY"):
        return

    def one(b):
        out = claude(CEIL.format(cases="\n".join(x[2] for x in b)), cwd=DATA, model="sonnet", extra=SONNET, cap=JUDGE_CAP)
        ans = parse_json_block(out.get("result", "")) or []
        return b, ans

    with concurrent.futures.ThreadPoolExecutor(4) as ex:
        for b, ans in ex.map(one, batches):
            ok = {"q" + x for case in b for x in case[1]}
            for a in ans:
                try:
                    tid = str(a["target"])
                    same = [str(x) for x in a.get("same") or [] if str(x) in ok]
                except (KeyError, TypeError):
                    continue
                if same:
                    state["answers"][tid] = same
            if ans:
                for case in b:
                    state["asked"][str(case[0])] = len(case[1])
            with open(path, "w") as f:
                json.dump(state, f, indent=1)


if __name__ == "__main__":
    {"run": run, "report": report, "judge": judge}[sys.argv[1]]()
