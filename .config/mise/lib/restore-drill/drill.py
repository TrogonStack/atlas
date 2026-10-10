#!/usr/bin/env python3
"""gRPC side of the compose:restore-drill mise task: seeding calls and state capture.

Reads TROGON_ATLAS_DRILL_ADDR (host:port), TROGON_ATLAS_DRILL_PROTO_DIR and TROGON_ATLAS_DRILL_TOKENS (a JSON
file mapping principal name to bearer token). Shells out to grpcurl.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

SERVICE = "trogonatlas.api.eventmodel.v1alpha1.EventModelService"
PAGE = 500
BRANCH = "drill-pending"
SCOPED_PRINCIPALS = ("acme-agent", "globex-agent", "partner-reader")
PROBES = (
    ("ENTITY_KIND_PERSONA", "acme-reviews", "reviewer"),
    ("ENTITY_KIND_PERSONA", "acme-storefront", "shopper"),
    ("ENTITY_KIND_PERSONA", "globex-orders", "buyer"),
)


class RpcError(Exception):
    def __init__(self, method, output):
        super().__init__(f"{method} failed:\n{output}")
        self.output = output


def token(principal):
    return json.loads(Path(os.environ["TROGON_ATLAS_DRILL_TOKENS"]).read_text())[principal]


def call(method, principal, body=None):
    cmd = [
        "grpcurl",
        "-plaintext",
        "-max-msg-sz",
        str(64 * 1024 * 1024),
        "-import-path",
        os.environ["TROGON_ATLAS_DRILL_PROTO_DIR"],
        "-proto",
        "trogonatlas/api/eventmodel/v1alpha1/service.proto",
        "-H",
        f"authorization: Bearer {token(principal)}",
        "-d",
        "@",
        os.environ["TROGON_ATLAS_DRILL_ADDR"],
        f"{SERVICE}/{method}",
    ]
    done = subprocess.run(
        cmd, input=json.dumps(body or {}), capture_output=True, text=True
    )
    if done.returncode != 0:
        raise RpcError(method, done.stdout + done.stderr)
    return json.loads(done.stdout) if done.stdout.strip() else {}


def paged(method, principal, body, items, token_in="pageToken", token_out="nextPageToken"):
    out, page_token = [], ""
    while True:
        resp = call(method, principal, {**body, token_in: page_token})
        out.extend(resp.get(items, []))
        page_token = resp.get(token_out, "")
        if not page_token:
            return out


def canonical(value):
    return json.dumps(value, sort_keys=True, indent=1)


def sorted_by_json(values):
    return sorted(values, key=lambda v: json.dumps(v, sort_keys=True))


def entity_ref(kind, namespace, slug):
    return {"kind": kind, "id": {"namespace": namespace, "slug": slug, "version": 1}}


def change_feed(since):
    events, cursor = [], since
    while True:
        resp = call("ListChanges", "drill-admin", {"sinceToken": cursor, "pageSize": PAGE})
        page = resp.get("events", [])
        events.extend(page)
        cursor = resp.get("nextToken", cursor)
        if not page:
            return events, cursor


def capture(out_dir, cursor_file):
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)

    def write(name, value):
        (out / f"{name}.json").write_text(canonical(value) + "\n")

    write("snapshot", call("GetSnapshotId", "drill-admin", {"includeEntries": True}))
    write(
        "entities",
        sorted_by_json(paged("ListEntities", "drill-admin", {"pageSize": PAGE}, "entities")),
    )
    write(
        "namespaces",
        sorted(call("ListNamespaces", "drill-admin").get("namespaces", []), key=lambda n: n["id"]),
    )
    write(
        "etags",
        [
            call("GetEntity", "drill-admin", {"kind": k, "id": {"namespace": n, "slug": s, "version": 1}})
            for k, n, s in PROBES
        ],
    )
    for principal in SCOPED_PRINCIPALS:
        snap = call("GetSnapshotId", principal)
        namespaces = call("ListNamespaces", principal).get("namespaces", [])
        write(
            f"scope.{principal}",
            {
                "namespaces": sorted((n["name"], n["id"], n.get("parent", "")) for n in namespaces),
                "snapshot_id": snap.get("snapshotId"),
                "entity_count": snap.get("entityCount", 0),
            },
        )
    write("branches", call("ListBranches", "drill-admin"))
    write("branch_diff", call("DiffBranch", "drill-admin", {"name": BRANCH}))
    write(
        "changesets",
        paged("ListChangesets", "drill-admin", {"pageSize": PAGE}, "changesets"),
    )
    write(
        "history",
        paged(
            "GetEntityHistory",
            "drill-admin",
            {"ref": entity_ref("ENTITY_KIND_PERSONA", "acme-reviews", "reviewer"), "pageSize": PAGE},
            "revisions",
        ),
    )
    since = Path(cursor_file).read_text().strip()
    events, cursor = change_feed(since)
    write("changes", {"since": since, "events": events, "next": cursor})


def expect_denied(method, principal, body):
    try:
        call(method, principal, body)
    except RpcError as err:
        if "PermissionDenied" in err.output:
            return
        raise
    raise SystemExit(f"{method} as {principal} was allowed; expected PermissionDenied")


def main(argv):
    cmd, *rest = argv
    if cmd == "call":
        method, principal, *body = rest
        print(canonical(call(method, principal, json.loads(body[0]) if body else {})))
    elif cmd == "capture":
        capture(*rest)
    elif cmd == "cursor":
        print(call("ListChanges", "drill-admin", {}).get("nextToken", ""))
    elif cmd == "expect-denied":
        method, principal, body = rest
        expect_denied(method, principal, json.loads(body))
    elif cmd == "feed-since":
        events, cursor = change_feed(Path(rest[0]).read_text().strip())
        print(canonical({"events": events, "next": cursor}))
    else:
        raise SystemExit(f"unknown command {cmd}")


if __name__ == "__main__":
    main(sys.argv[1:])
