"""Shared bits for the repo knowledge spike (SPEC 35, phase 0)."""
import json
import os
import subprocess
import threading
import time

REPO = "sigp/lighthouse"
DATA = os.path.expanduser("~/.cache/sb-know-spike")
PRS = os.path.join(DATA, "prs")
BARE = os.path.join(DATA, "lighthouse.git")
COUNT = os.path.join(DATA, "claude_runs.txt")


def gql(query, variables=None):
    """Run one read-only GraphQL query. Waits on the rate limit and retries on errors."""
    args = ["gh", "api", "graphql", "-f", "query=" + query]
    for k, v in (variables or {}).items():
        if isinstance(v, int):
            args += ["-F", "%s=%d" % (k, v)]
        else:
            args += ["-f", "%s=%s" % (k, v)]
    for attempt in range(4):
        p = subprocess.run(args, capture_output=True, text=True)
        if p.returncode == 0:
            out = json.loads(p.stdout)
            rl = out.get("data", {}).get("rateLimit")
            if rl and rl["remaining"] < 200:
                print("rate limit low, waiting for", rl["resetAt"])
                time.sleep(60)
            return out["data"]
        print("gql error, retry", attempt, p.stderr[:300])
        time.sleep(5 * (attempt + 1))
    raise SystemExit("gql failed")


def git(*args, cwd=BARE, check=True):
    p = subprocess.run(["git"] + list(args), cwd=cwd, capture_output=True, text=True)
    if check and p.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), p.stderr))
    return p.stdout


def load_prs():
    out = []
    for name in os.listdir(PRS):
        if name.endswith(".json"):
            with open(os.path.join(PRS, name)) as f:
                out.append(json.load(f))
    out.sort(key=lambda p: p["mergedAt"], reverse=True)
    return out


_lock = threading.Lock()


def claude_count(add=0):
    with _lock:
        return _count(add)


def _count(add):
    n = 0
    if os.path.exists(COUNT):
        with open(COUNT) as f:
            n = int(f.read().strip() or 0)
    n += add
    with open(COUNT, "w") as f:
        f.write(str(n))
    return n


def claude(prompt, cwd, model="sonnet", extra=None, cap=60):
    """One `claude -p` run. The prompt goes first. Counts every run."""
    n = claude_count()
    if n >= cap:
        raise SystemExit("Claude run cap reached: %d" % n)
    args = ["claude", "-p", prompt, "--model", model, "--output-format", "json", "--no-session-persistence"] + (extra or [])
    p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=1800)
    n = claude_count(1)
    print("  [claude runs: %d]" % n, flush=True)
    try:
        out = json.loads(p.stdout)
    except ValueError:
        return {"result": p.stdout, "error": p.stderr[:500]}
    return out


def parse_json_block(text, start="[", end="]"):
    """Take the first JSON array or object from a model's answer."""
    i = text.find(start)
    j = text.rfind(end)
    if i < 0 or j < i:
        return None
    try:
        return json.loads(text[i:j + 1])
    except ValueError:
        return None
