#!/usr/bin/env python3
"""Mine merged PRs and their review threads from GitHub. Read only.

Walks merge-date slices newest first, keeps PRs with at least one review
thread, and stops at the target count. One JSON file per PR under
~/.cache/sb-know-spike/prs. Field names were checked by schema introspection.
GitHub has no merge method field on PullRequest: actedon.py infers it from git.
"""
import collections
import datetime
import json
import os
import sys

from common import PRS, REPO, gql, load_prs

TARGET = int(sys.argv[1]) if len(sys.argv) > 1 else 200
SLICE_DAYS = 14
PAGE = 10

COMMENT = """
  author { login __typename } authorAssociation createdAt body isMinimized
  originalCommit { oid } diffHunk
"""
THREAD = """
  id path diffSide line startLine originalLine originalStartLine subjectType
  isResolved isOutdated
  comments(first: 20) { totalCount pageInfo { hasNextPage endCursor } nodes { %s } }
""" % COMMENT

SEARCH = """
query($q: String!, $after: String, $n: Int!) {
  rateLimit { remaining resetAt }
  search(type: ISSUE, query: $q, first: $n, after: $after) {
    issueCount pageInfo { hasNextPage endCursor }
    nodes { ... on PullRequest {
      id number title body url author { login __typename } createdAt mergedAt
      baseRefName baseRefOid headRefOid mergeCommit { oid }
      commits(first: 100) { totalCount pageInfo { hasNextPage endCursor }
        nodes { commit { oid message committedDate } } }
      reviewThreads(first: 50) { totalCount pageInfo { hasNextPage endCursor }
        nodes { %s } }
    } }
  }
}
""" % THREAD

MORE_THREADS = """
query($id: ID!, $after: String) {
  rateLimit { remaining resetAt }
  node(id: $id) { ... on PullRequest {
    reviewThreads(first: 50, after: $after) { pageInfo { hasNextPage endCursor } nodes { %s } } } }
}
""" % THREAD

MORE_COMMENTS = """
query($id: ID!, $after: String) {
  rateLimit { remaining resetAt }
  node(id: $id) { ... on PullRequestReviewThread {
    comments(first: 20, after: $after) { pageInfo { hasNextPage endCursor } nodes { %s } } } }
}
""" % COMMENT

MORE_COMMITS = """
query($id: ID!, $after: String) {
  rateLimit { remaining resetAt }
  node(id: $id) { ... on PullRequest {
    commits(first: 100, after: $after) { pageInfo { hasNextPage endCursor }
      nodes { commit { oid message committedDate } } } } }
}
"""


def fill_comments(t):
    c = t["comments"]
    while c["pageInfo"]["hasNextPage"]:
        d = gql(MORE_COMMENTS, {"id": t["id"], "after": c["pageInfo"]["endCursor"]})
        more = d["node"]["comments"]
        c["nodes"] += more["nodes"]
        c["pageInfo"] = more["pageInfo"]


def fill(pr):
    rt = pr["reviewThreads"]
    while rt["pageInfo"]["hasNextPage"]:
        d = gql(MORE_THREADS, {"id": pr["id"], "after": rt["pageInfo"]["endCursor"]})
        more = d["node"]["reviewThreads"]
        rt["nodes"] += more["nodes"]
        rt["pageInfo"] = more["pageInfo"]
    for t in rt["nodes"]:
        fill_comments(t)
    cm = pr["commits"]
    while cm["pageInfo"]["hasNextPage"]:
        d = gql(MORE_COMMITS, {"id": pr["id"], "after": cm["pageInfo"]["endCursor"]})
        more = d["node"]["commits"]
        cm["nodes"] += more["nodes"]
        cm["pageInfo"] = more["pageInfo"]


def flatten(pr):
    threads = []
    for t in pr["reviewThreads"]["nodes"]:
        comments = []
        for c in t["comments"]["nodes"]:
            a = c.get("author") or {}
            comments.append({
                "author": a.get("login"), "authorType": a.get("__typename"),
                "authorAssociation": c["authorAssociation"], "createdAt": c["createdAt"],
                "body": c["body"], "isMinimized": c["isMinimized"],
                "originalCommit": (c.get("originalCommit") or {}).get("oid"),
                "diffHunk": c["diffHunk"],
            })
        threads.append({k: t[k] for k in (
            "id", "path", "diffSide", "line", "startLine", "originalLine",
            "originalStartLine", "subjectType", "isResolved", "isOutdated")})
        threads[-1]["comments"] = comments
    a = pr.get("author") or {}
    return {
        "number": pr["number"], "url": pr["url"], "title": pr["title"], "body": pr["body"],
        "author": a.get("login"), "authorType": a.get("__typename"),
        "createdAt": pr["createdAt"], "mergedAt": pr["mergedAt"],
        "base": pr["baseRefName"], "baseRefOid": pr["baseRefOid"], "headRefOid": pr["headRefOid"],
        "mergeCommit": (pr.get("mergeCommit") or {}).get("oid"),
        "mergeMethod": None,
        "commitCount": pr["commits"]["totalCount"],
        "commits": [n["commit"] for n in pr["commits"]["nodes"]],
        "reviewThreads": threads,
    }


def main():
    os.makedirs(PRS, exist_ok=True)
    have = len([n for n in os.listdir(PRS) if n.endswith(".json")])
    end = datetime.date.today() + datetime.timedelta(days=1)
    queries = 0
    while have < TARGET:
        start = end - datetime.timedelta(days=SLICE_DAYS)
        q = "repo:%s is:pr is:merged merged:%s..%s" % (REPO, start.isoformat(), end.isoformat())
        after = None
        found = []
        while True:
            v = {"q": q, "n": PAGE}
            if after:
                v["after"] = after
            d = gql(SEARCH, v)
            queries += 1
            s = d["search"]
            found += [n for n in s["nodes"] if n]
            if not s["pageInfo"]["hasNextPage"]:
                break
            after = s["pageInfo"]["endCursor"]
        found.sort(key=lambda p: p["mergedAt"], reverse=True)
        kept = 0
        for pr in found:
            if have >= TARGET:
                break
            if pr["reviewThreads"]["totalCount"] < 1:
                continue
            path = os.path.join(PRS, "%d.json" % pr["number"])
            if os.path.exists(path):
                continue
            fill(pr)
            with open(path, "w") as f:
                json.dump(flatten(pr), f)
            have += 1
            kept += 1
        print("slice %s..%s: %d merged, %d kept, total %d" % (start, end, len(found), kept, have), flush=True)
        end = start
    report()


def report():
    prs = load_prs()
    threads = [t for p in prs for t in p["reviewThreads"]]
    comments = [c for t in threads for c in t["comments"]]
    roles = collections.Counter(t["comments"][0]["authorAssociation"] if t["comments"] else "NONE_LEFT"
                                for t in threads)
    kinds = collections.Counter(t["subjectType"] for t in threads)
    print("PRs %d, threads %d, comments %d" % (len(prs), len(threads), len(comments)))
    print("first-comment role:", dict(roles))
    print("thread kind:", dict(kinds))
    print("range: %s .. %s" % (prs[-1]["mergedAt"], prs[0]["mergedAt"]))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "report":
        report()
    else:
        main()
