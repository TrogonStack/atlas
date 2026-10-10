# Event Modeling Stack Runbook

## Stack lifecycle

```sh
# Start
mise run compose:up

# Stop (preserves volumes)
mise run compose:down

# Upgrade (rebuild images, rolling restart)
mise run compose:build -- --no-cache --pull
mise run compose:up
```

## Backup and restore

A full store backup is NATS JetStream plus SpiceDB. Backing up only one of
them, or only the change feed, does not bring the deployment back.

### Inventory

| Data set | Where | Authoritative? |
| --- | --- | --- |
| `KV_trogon-atlas-entities` | NATS (`nats-data` volume) | Yes. Current entity bodies; the KV revision is the etag clients send back. |
| `KV_trogon-atlas-revisions` | NATS | Yes. Per-entity revision history. |
| `KV_trogon-atlas-changesets` | NATS | Yes. Changeset log that revert and history read. |
| `KV_trogon-atlas-branches` | NATS | Yes. Branches and their pending changes. |
| `KV_trogon-atlas-namespaces` | NATS | Yes. Namespace registry: names, minted `ns_...` ids, owning parent. |
| `KV_trogon-atlas-batches` | NATS | Yes. Journal of baseline batches that have not finished. Empty when no batch is running and none needs recovery; an entry in it is what tells the server how to finish or undo an interrupted batch. |
| `KV_trogon-atlas-operations` | NATS | No. Idempotency claims for `operation_id` retries. The changeset, not the receipt, is the source of truth; losing this bucket only costs a future retry a fresh claim instead of a replay. |
| `KV_trogon-atlas-writer-lease` | NATS | No. Single row naming the current writer lease holder and its epoch (see `docs/explanation/single-writer.md`). Losing it only costs a fresh lease race on restore; no entity data lives here. |
| `TROGON_ATLAS_CHANGES` | NATS | Yes. Change feed; resume cursors are its sequence numbers. |
| SpiceDB `namespace#viewer` / `namespace#editor` shares, `organization#parent` | SpiceDB (`spicedb-data` volume) | Yes. Nothing else records them. |
| SpiceDB `namespace#parent`, `organization#member` | SpiceDB | No. The server re-writes them at startup from the namespace registry and the token file. |
| Git mirror | `trogon-atlas-mirror` volume | No. Rebuilt from the store with `export-git`; see "Git mirror" below. |

The server creates no durable JetStream consumers, so there is no consumer
state to back up. A client's change-feed cursor stays valid across a restore
because `nats stream restore` keeps sequence numbers.

Not in the backup: `.env`, the token file and TLS material. Keep them with
your secrets. The token file must be restored with the same principal names
and parents, since SpiceDB memberships and namespace owners refer to them by
name; the token values and the NATS, Postgres and SpiceDB passwords can be
rotated freely.

### Consistency

There is no point-in-time snapshot across NATS and SpiceDB, so take the
backup with every `trogon-atlas-server` replica stopped. With no writer,
neither side moves while the other is copied.

```sh
docker compose -f atlas/devops/docker/compose/dev/compose.yaml stop trogon-atlas-server

export NATS_URL=nats://service:${TROGON_ATLAS_NATS_PASSWORD:-localdevpass}@nats.trogon-atlas.orb.local:4222
backup=./backup-$(date +%Y%m%d%H%M)
nats stream ls --names   # must list exactly the streams in the table above
for s in TROGON_ATLAS_CHANGES KV_trogon-atlas-entities KV_trogon-atlas-revisions \
         KV_trogon-atlas-changesets KV_trogon-atlas-branches KV_trogon-atlas-namespaces \
         KV_trogon-atlas-batches KV_trogon-atlas-operations KV_trogon-atlas-writer-lease; do
  nats stream backup "$s" "$backup/nats/$s"
done

mkdir -p "$backup/spicedb"
docker run --rm --network trogon-atlas_trogon-atlas -v "$PWD/$backup/spicedb:/backup" \
  authzed/zed:v1.2.1 --endpoint spicedb:50051 --insecure \
  --token "${TROGON_ATLAS_SPICEDB_PRESHARED_KEY:-localdevkey}" \
  backup create /backup/spicedb.zedbackup

docker compose -f atlas/devops/docker/compose/dev/compose.yaml start trogon-atlas-server
```

If `nats stream ls` shows a stream not in the table, stop and find out what
owns it before trusting the backup.

### Restore order

Restore into empty services, never on top of live data.

1. SpiceDB: start an empty, migrated SpiceDB and run
   `zed backup restore /backup/spicedb.zedbackup`. It writes the schema and
   every relationship, shares included.
2. NATS: start an empty JetStream and run
   `nats stream restore <dir>` for each stream in the inventory. Do this before
   the server starts, or it creates empty buckets that the restore then
   refuses to overwrite.
3. Put `.env`, the token file and TLS material back.
4. Start `trogon-atlas-server` with its default role (`writer`), so it wins
   the (now-empty) writer lease and startup batch recovery actually runs;
   see `docs/explanation/single-writer.md`. On startup it finishes or undoes
   any batch the journal in `KV_trogon-atlas-batches` says was interrupted at
   least a minute ago (younger entries are left to the background sweep),
   then reconciles the derived SpiceDB grants against the restored registry
   and token file. A backup taken with the server stopped normally has an
   empty journal; one that does not is still consistent, because the
   entries and the writes they describe were copied together.
5. Rebuild the git mirror with `export-git` into an empty volume, then check
   it with `reconcile --mirror`. Do not restore an old mirror volume; it is
   older than the store you just restored.

### Data-loss window

Everything written after the last completed backup is lost; there is no
point-in-time recovery. Writes are refused while the server is stopped for
the backup, so the cost of a backup is that downtime, not lost data.
`mise run compose:restore-drill` (see "Backup verification") prints how long a
real backup and restore took.

### Offline volume snapshot

The same consistency rule applies: both volumes, taken with the whole stack
down. A NATS snapshot without the matching SpiceDB snapshot loses every
share.

```sh
docker compose -f atlas/devops/docker/compose/dev/compose.yaml down
for v in nats-data spicedb-data; do
  docker run --rm -v "trogon-atlas_$v:/data" -v "$(pwd)/backups":/backup \
    alpine tar czf "/backup/$v-$(date +%Y%m%d).tar.gz" -C /data .
done
```

Restore both, into empty volumes, before starting the stack:

```sh
for v in nats-data spicedb-data; do
  docker run --rm -v "trogon-atlas_$v:/data" -v "$(pwd)/backups":/backup \
    alpine sh -c "tar xzf /backup/$v-<DATE>.tar.gz -C /data"
done
```

## Git mirror: `trogon-atlas-mirror` volume

The git mirror is stored in the `trogon-atlas-mirror` Docker volume at `/var/lib/trogon-atlas/mirror`.

If the volume is lost, rebuild it from the store with `export-git`. Live
mirroring only ever appends as writes arrive, so a fresh mirror otherwise
starts empty and fills in from the next mutation onward:

```sh
docker compose -f atlas/devops/docker/compose/dev/compose.yaml exec trogon-atlas-server \
  trogon-atlas-server --store=nats --nats-url=nats://nats:4222 \
    export-git /var/lib/trogon-atlas/mirror
```

`export-git` writes a snapshot, not an accumulation: an entity the store no
longer has is deleted from the mirror in the same commit. It is the only
command that produces a mirror without having run a server with
`--git-mirror`, and unlike live mirroring it also writes `namespaces.json`
at the mirror root, which is what makes the ownership restore below possible.

Recovering from a pushed remote (see below) works too, and is the only option
when the store is gone as well.

`import-git` runs the other direction. It reads a mirror tree and writes those
entities into the store, which is the restore path when the *store* is the
thing that was lost:

```sh
docker compose -f atlas/devops/docker/compose/dev/compose.yaml exec trogon-atlas-server \
  trogon-atlas-server import-git /var/lib/trogon-atlas/mirror
```

**On its own it does not restore ownership.** The namespace registry that says
which owner each namespace belongs to lives in its own KV bucket, which no
mirror commit written by live mirroring has ever touched. A store restored
from such a mirror comes back with every entity and no owner, and a namespace
with no registry row is invisible to every caller bound to a parent and
claimed by the first one that writes into it. `import-git` names the affected
namespaces at the end of its run.

A mirror produced by `export-git` carries the owners too, in
`namespaces.json`. Add `--restore-namespaces` to put them back:

```sh
docker compose -f atlas/devops/docker/compose/dev/compose.yaml exec trogon-atlas-server \
  trogon-atlas-server import-git /var/lib/trogon-atlas/mirror --restore-namespaces
```

Off by default because it decides what every caller can see. It restores each
row exactly as recorded, ids and timestamps included, so entities keyed by a
minted `ns_...` id find their namespace again. Rows the live registry already
holds are left alone: an id the snapshot and the registry disagree about is
reported as a conflict and skipped rather than re-owned, so this never takes a
namespace away from its current parent. Run it against an empty registry.

Without such a snapshot, restore the registry from a stream or volume backup
above; `backfill-namespaces` is the last fallback, and it assigns everything
to a single owner, so on a multi-tenant deployment it is a starting point for
`MoveNamespace` rather than a restore.

### Drift detection

After any direct NATS KV surgery (rare; only the FieldType wire-format
migration has needed it) the mirror is stale until every modified entity
is no-op read-write through the server. To verify, run:

```sh
docker compose -f atlas/devops/docker/compose/dev/compose.yaml exec trogon-atlas-server \
  trogon-atlas-server --store=nats --nats-url=nats://nats:4222 \
    reconcile --mirror /var/lib/trogon-atlas/mirror
```

The command walks both sides, compares them by `(kind, ns, slug,
version)`, and reports:

- Entities present in the store but missing from the mirror.
- Entities present in the mirror but missing from the store.
- Byte-level differences between the two.

Exits non-zero if any drift exists. Wire this into CI on the mirror
branch to catch silent divergence.

### Volume snapshot

```sh
docker compose -f atlas/devops/docker/compose/dev/compose.yaml down
docker run --rm \
  -v trogon-atlas_trogon-atlas-mirror:/data \
  -v "$(pwd)/backups":/backup \
  alpine tar czf /backup/trogon-atlas-mirror-$(date +%Y%m%d).tar.gz -C /data .
```

## Secrets

Copy `.env.example` to `.env` and fill in values before starting the stack:

```sh
cp atlas/devops/docker/compose/dev/.env.example atlas/devops/docker/compose/dev/.env
```

The `.env` file is gitignored and never committed.

## Health checks

- NATS monitoring: `docker compose exec nats wget -qO- http://127.0.0.1:8222/healthz`

  Not published to the host. The endpoint takes no credentials and describes
  the whole deployment to whoever reaches it, so the listener binds the
  container's loopback; a published port would not have confined it, because
  OrbStack resolves every container by DNS regardless.
- trogon-atlas-server metrics: http://127.0.0.1:9100/metrics

## Rolling deploys and batch recovery

Batch recovery is gated on the writer lease (see
`docs/explanation/single-writer.md`): both startup recovery and the
periodic sweep only run on the process currently holding the lease, so an
old and a new server process overlapping during a rolling deploy cannot
both repair the same batch. Only the lease holder may ever roll back or
complete a batch, so the recovered journal entry is attributable to the
writer that was actually applying it. The Helm chart's `Recreate`
deployment strategy keeps that overlap at zero in the common case; the
lease is what makes correctness not depend on it staying that way.

Manual recovery is not gated the same way: `trogon-atlas-server
recover-batches` runs regardless of lease state, because an operator
invoking it directly has already decided it is safe. Check what is
pending with
`trogon-atlas-server recover-batches --report-only --min-age-secs 60`.

## Store backend

NATS JetStream is the only storage backend: entities live in a KV
bucket and the change feed is a JetStream stream. The reference compose
stack runs it by default; backup and restore are covered above. For local
dev and fixture replay, point the server at the compose NATS (or any
JetStream instance); integration tests provision their own ephemeral
JetStream via testcontainers.

## Authentication

The gRPC service uses a single shared bearer token configured via
`--auth-token` / `TROGON_ATLAS_AUTH_TOKEN`. The MCP adapter and the studio
bridge present the same token. Operating points:

- The server refuses to start with `--auth-token` unset unless
  `--insecure-allow-anonymous` (or
  `TROGON_ATLAS_INSECURE_ALLOW_ANONYMOUS=true`) is also passed. The
  compose stack sets the flag because the listener is bound to
  127.0.0.1 inside the orb network; remove it for any deployment that
  exposes the listener beyond a trusted boundary.
- Rotate the token by writing a new value into `.env`, restarting the
  server, and updating the studio + MCP environments in the same
  window. Clients see a single transient `UNAUTHENTICATED` and recover.
- For multi-tenant or per-client tokens, front the server with a
  reverse proxy that rewrites the `authorization` header per client;
  the in-process interceptor is single-token by design.

## TLS in production

The server can serve TLS directly via `--tls-cert` / `--tls-key`. The
reference compose stack ships plaintext on `127.0.0.1:50069` and
expects the operator to terminate TLS in front of it (Envoy, Caddy, or
the cloud LB); that path is simpler to operate and gives you HTTP/2
ALPN negotiation for free. Two supported postures:

### Posture A: terminate at a reverse proxy (recommended)

1. Front the server with Envoy / Caddy / nginx / cloud LB.
2. Terminate TLS at the proxy; forward to the server over the
   orb-internal network.
3. Configure the proxy to require the bearer token on a header it
   rewrites into `authorization` so the server never sees external
   credentials.

### Posture B: direct tonic-rustls

1. Provision a PEM certificate chain and private key for the listen
   host.
2. Set `TROGON_ATLAS_TLS_CERT=/path/cert.pem` and
   `TROGON_ATLAS_TLS_KEY=/path/key.pem`.
3. Point clients at `https://...:50069`. The studio bridge accepts
   `https://` URLs for `TROGON_ATLAS_GRPC` without configuration; the
   MCP adapter accepts the same.

Either posture: never serve plaintext gRPC across an untrusted network.
The server still emits a startup warning when TLS is not configured.

## Backup verification

A backup is only as good as its last restore. Once per release branch, and
after any change to the buckets, streams or SpiceDB schema, run the drill:

```sh
mise run compose:restore-drill
```

It never touches the `trogon-atlas` stack. It brings up a source stack under a
generated `trogon-atlas-drill-<id>-src` compose project with its own volumes and
throwaway credentials, refuses to run if that project or any of its volumes
already exists, and removes everything it created on exit.

Against the source it seeds the Acme manifests, a second tenant with a minted
namespace id, a SpiceDB share, a reverted changeset and a branch with a
pending change. It then takes the backup above, deletes the source stack,
restores into a second, empty `trogon-atlas-drill-<id>-dst` project in the documented
order, and fails unless the restored stack matches: snapshot id, every
entity and etag, namespace ids and owners, each principal's scoped view,
cross-tenant writes still refused, changesets, history, branch diff, a
branch merge, and a change-feed cursor taken before the backup resuming
without gaps or replays. Finally it rebuilds the git mirror and reconciles
it. It prints the backup size, writer downtime and time to serving.

Set `TROGON_ATLAS_DRILL_SERVER_IMAGE` to test a published image instead of building one
from the working tree, and `TROGON_ATLAS_DRILL_KEEP=1` to keep the captured state.
