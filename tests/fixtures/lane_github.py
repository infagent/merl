#!/usr/bin/env python3
"""Metadata-only fake for the fixed GitHub GraphQL operations used by lanes."""
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parent
state = json.loads((root / "provider.json").read_text())
request = json.load(sys.stdin)
assert sys.argv[1:] == ["api", "--hostname", "github.com", "--include", "graphql", "--input", "-"]
with (root / "requests.jsonl").open("a") as log:
    log.write(json.dumps(request) + "\n")
variables = request["variables"]
name = request["operationName"]
size = state.get("page_size", 1)
print("HTTP/2.0 503 Service Unavailable\r" if state.get("error")=="unavailable" else "HTTP/2.0 200 OK\r")
print("X-RateLimit-Remaining: 1000\r")
if state.get("error") == "rate":
    if state.get("advance_clock"):
        (root / "response-clock").touch()
    print("Retry-After: 600\r")
    print("X-RateLimit-Reset: 601\r")
    print("\r")
    print(json.dumps({"errors": [{"type": "RATE_LIMITED", "message": "synthetic rate limit"}]}))
    sys.exit(0)
print("\r")
if state.get("error") == "unavailable":
    print(json.dumps({"message":"synthetic service unavailable"}))
    sys.exit(1)
if state.get("error") == "cursor":
    print(json.dumps({"errors": [{"type": "INVALID_CURSOR_ARGUMENTS", "message": "synthetic invalid cursor"}]}))
    sys.exit(0)


def connection(items, prefix, after=None):
    index = int(after.split(":")[1]) if after else 0
    selected = items[index:index + size]
    edges = [{"cursor": f"{prefix}:{index + n + 1}", "node": node} for n, node in enumerate(selected)]
    return {"edges": edges, "pageInfo": {"hasNextPage": index + len(selected) < len(items),
                                        "endCursor": edges[-1]["cursor"] if edges else None}}


def snapshot(pr):
    events = pr.get("events", [])
    tail = {"edges": [{"cursor": f"t:{len(events)}", "node": {"__typename": events[-1]}}] if events else [],
            "pageInfo": {"endCursor": f"t:{len(events)}" if events else None, "hasNextPage": False}}
    return {"number": pr["number"], "headRefOid": pr.get("head", "a" * 40), "state": pr.get("state", "OPEN"),
            "isDraft": pr.get("draft", False), "updatedAt": "2026-01-01T00:00:00Z",
            "labels": {"nodes": [{"name": label} for label in pr.get("labels", [])],
                       "pageInfo": {"hasNextPage": pr.get("truncated_labels", False)}}, "timelineItems": tail}


if name == "Discover":
    assert "pullRequests" in request["query"] and "timelineItems" in request["query"]
    prs = [pr for pr in state["prs"] if pr.get("state", "OPEN") == "OPEN"
           and all(label in pr.get("labels", []) for label in variables["labels"])]
    result = {"pullRequests": connection([snapshot(pr) for pr in prs], "d", variables.get("after"))}
elif name in {"Observe", "Resync"}:
    assert "reviewThreads" in request["query"] and "totalCount" in request["query"]
    pr = next(pr for pr in state["prs"] if pr["number"] == variables["number"])
    result_pr = snapshot(pr)
    if name == "Observe":
        result_pr["timelineItems"] = connection([{"__typename": kind} for kind in pr.get("events", [])], "t", variables.get("after"))
    threads = [{"id": t["id"], "isResolved": t.get("resolved", False),
                "comments": {"totalCount": t.get("count", 1),
                             "nodes": [{"id": t.get("comment", "comment-1"), "updatedAt": t.get("updated", "2025-01-01T00:00:00Z"),
                                        "url": "https://github.com/example/widgets/pull/1#discussion_r1"}]}}
               for t in pr.get("threads", [])]
    result_pr["reviewThreads"] = connection(threads, "r", variables.get("threadsAfter"))
    result = {"pullRequest": result_pr}
else:
    raise AssertionError(name)
print(json.dumps({"data": {"repository": result}}))
