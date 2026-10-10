# trogon-atlas-server Helm chart

Deploys the `trogon-atlas-server` gRPC service (port 50069) with an optional
Prometheus metrics endpoint (port 9100).

NATS JetStream is a required external dependency and is not installed by
this chart. Point `config.natsUrl` at an existing NATS installation with
JetStream enabled.

Installed on its own, every value below drops the `server.` prefix it would
carry under the `trogon-atlas` umbrella chart; `server.auth.tokensFile` here
is just `auth.tokensFile`.

## Quick start

```bash
helm install trogon-atlas-server devops/helm/charts/trogon-atlas-server \
  --set config.natsUrl=nats://my-nats:4222 \
  --set auth.token.value=change-me
```

## Authentication

The server refuses to start without authentication, so the chart fails at template time unless one of these is configured:

- `auth.tokensFile.content`: inline TOML token registry (see `rsworkspace/crates/trogon-atlas-server/examples/tokens.toml`); the chart stores it in a Secret and mounts it.
- `auth.tokensFile.existingSecret`: a pre-existing Secret containing the registry under `auth.tokensFile.key`.
- `auth.token.value` or `auth.token.existingSecret`: a single shared bearer token (deprecated upstream).
- `auth.insecureAllowAnonymous=true`: no authentication; only for isolated clusters.

A caller that needs a bearer token (such as trogon-atlas-studio) should get a principal with `role = "reader"` from the token registry rather than an unrestricted key.

## Who may write, and where

A token registry grants each principal a role (`reader`, `writer`, `admin`) and, optionally, a `namespaces` allow-list bounding *which* namespaces it may write. Omitting the list leaves the principal unrestricted, so registries written before the field existed are unaffected. See `docs/explanation/authorization.md`.

Independently, `auth.protectBaseline=true` refuses direct writes to baseline so changes arrive by branch and merge. `auth.protectBaselineNamespaces={orders,shop-*}` narrows that to named namespaces: with a non-empty list, only those are protected and the rest keep accepting direct writes.

## The store is the other half of the boundary

**An authenticated server needs an authenticated store.** The chart refuses to render otherwise.

Ownership is not held in the server. The namespace registry that decides which owner a namespace belongs to is a KV bucket in NATS, alongside the entities themselves, so anything that can reach the store unauthenticated can read every tenant's data without an RPC and reassign ownership without one either. The server goes on enforcing against a registry that anyone may edit, and nothing about that looks wrong from the outside.

Not publishing the port is not the control. So when `auth` is configured, `config.natsUrl` must carry userinfo or `config.natsUrlSecret.name` must be set. `config.acknowledgeUnauthenticatedStore=true` overrides it, for deployments that close the store some other way (mesh mTLS, a NetworkPolicy that isolates the bucket).

## Who may see a namespace

Left alone, the server answers that from the registry's own `parent` column: a namespace belongs to exactly one owner, and only that owner sees it. That is the whole answer for a deployment where each namespace has one tenant, and it needs no configuration. What it cannot express is a namespace shared between two owners.

`spicedb.endpoint` moves the permission question to SpiceDB. The registry stays authoritative for whether a namespace exists and what its id is; only "may this principal see it" moves. `NOTES.txt` prints which of the two is deciding after every install, so the fallback is never silent.

SpiceDB is all-or-nothing, and the chart refuses to render the half-configured cases rather than let them become a CrashLoopBackOff or, worse, a quiet answer of "no" to everything:

- `presharedKey` without `endpoint`. The server refuses to start on exactly this combination; the point of failing here is that the answer arrives at `helm install`.
- `endpoint` without `presharedKey`. The same mistake pointing the other way.
- a `freshness` outside `minimize-latency` and `fully-consistent`.
- `skipStartupSync` with no sync CronJob (below).
- `sync.enabled` without an `endpoint`.

`spicedb.freshness` decides how each check reads. `minimize-latency` (the default) takes whichever replica answers first, floored by this process's own writes. `fully-consistent` quorum-reads every check, which is what makes a grant revoked through another process stop working immediately rather than at the end of SpiceDB's quantization window.

**`sync` (`spicedb.sync`, default off).** Runs `sync-spicedb`, which installs the schema and re-derives every grant from the registry and the token file. Idempotent. Off by default because the server already does this at startup and again on every token-file reload, so the usual deployment needs nothing. Turn it on as a repair loop where a registry write can land while the matching SpiceDB write fails, or when `skipStartupSync` is set.

The Job mounts the same tokens Secret as the server and sets `TROGON_ATLAS_AUTH_TOKENS_FILE` when one exists. Membership is derived from that file, so a sync without it publishes namespace grants and zero principals: every check would then answer no.

**`skipStartupSync` is not just "skip a startup step".** Startup sync and the token-file reload are one code path, so setting it means no principal ever reaches SpiceDB from the server process, and an edit to the token file never reaches it either. Something else has to run `sync-spicedb`; the CronJob above is the only such thing the chart can see, which is why the pair is required together. Until that Job next runs, a principal added to the token file stays invisible.

## The realtime feed follows the credential

trogon-atlas-studio can watch the entity bucket over a NATS WebSocket, and that listener admits every browser on one identical grant covering every namespace. See the trogon-atlas-studio chart's README for how it decides whether to use it.

## Probes

Readiness and liveness use the standard gRPC health-checking protocol, which the server exposes without auth. The server reports NOT_SERVING until NATS is reachable and during shutdown drain (`config.shutdownDrainSecs`, default 30s; `terminationGracePeriodSeconds` defaults to 40s to cover it). When `tls.existingSecret` is set, probes fall back to tcpSocket because the kubelet gRPC probe does not support TLS.

## Scaling: the server is single-writer

**`replicaCount` must stay `1`.** The chart refuses to render otherwise.

The server's mutation lock is a process-local `tokio::sync::Mutex`. With two writer replicas:

- `BatchMutate` and `MergeBranch` lose their cross-key atomicity, and the compensating rollback is best-effort.
- A mutation on replica A does not invalidate replica B's in-process snapshot cache, so B serves stale reads until its own next local mutation.

Neither failure surfaces an error, which is why this is a render-time refusal rather than a documented convention. Set `acknowledgeSingleWriter=true` to override, and only when every replica is effectively read-only and mutations are routed elsewhere.

Scaling the write path needs a leader lease or per-scope locks in KV, which does not exist yet.

## Git mirror operations

With `gitMirror.enabled=true` the chart provisions the mirror PVC and two CronJobs.

**`reconcile` (`gitMirror.reconcile`, default on, `17 3 * * *`).** A mirror write can fail while the store commit succeeds: the server logs a warning, increments `git_mirror_failures_total`, and lets the RPC succeed, so the two diverge silently. Since the mirror is the documented restore path for entities (`import-git`), an unwatched divergence is discovered on restore day. This job compares store against mirror and **exits non-zero on drift**, so alert on the resulting failed Job.

**`gc` (`gitMirror.gc`, default on, `43 4 * * 0`).** The server only ever commits; it never repacks. Every mutation is a commit and every entity a blob, so without this the object store grows unbounded against a fixed PVC.

**A live-mirrored tree does not carry ownership.** It holds entities; the namespace registry that says which owner each namespace belongs to is a separate KV bucket no mirroring commit touches. A store restored with `import-git` from such a tree comes back with every entity and no owner, which every caller bound to a parent reads as empty and the first writer claims. `import-git` names the unowned namespaces when it finishes.

A tree written by `export-git` does carry it, in `namespaces.json` at the mirror root, and `import-git --restore-namespaces` puts those owners back with their original ids and timestamps. That flag is off by default because it decides what every caller can see, and it skips rather than re-owns any id the live registry already holds. `export-git` also rebuilds a mirror from scratch, which live mirroring cannot do. Backing the registry up with the store (`nats stream backup`) remains the primary path.

Both jobs mount the mirror PVC, which is `ReadWriteOnce` by default, so both default to `colocateWithServer: true` (pod affinity on the server pod's node). Set that to `false` only with a `ReadWriteMany` volume.

### Alerts to configure

| Metric | Meaning |
|---|---|
| `git_mirror_failures_total` | a store commit succeeded but its mirror write did not; the mirror is now behind |
| `trogon_atlas_store_change_events_lost_total` | a KV write is durable but its change event was never published; every change-feed consumer now has a hole. Not mirror-specific, and worth alerting on even with the mirror off |
| `trogon_atlas_changesets_lost_total` | writes committed but their changeset never landed, so those changes have no durable attribution. Not mirror-specific, and worth alerting on even with the mirror off |

### Pushing the mirror off-box

Not done by the server, deliberately: a daemon holding push credentials is a different security posture, and a push that can fail is a push that could eventually fail a write. The supported pattern is a sidecar or your own CronJob mounting the same PVC with its own credentials. Note that the audit log dies with the volume if you do not.

## Notable values

| Value | Default | Purpose |
|---|---|---|
| `global.imageRegistry` | `ghcr.io/trogonstack` | prepended to image repositories |
| `image.repository` | `atlas/trogon-atlas-server` | server image |
| `replicaCount` | `1` | must stay 1; the server is single-writer |
| `acknowledgeSingleWriter` | `false` | override the `replicaCount > 1` refusal |
| `config.natsUrl` | `nats://nats:4222` | NATS JetStream URL. Credentials may be embedded as userinfo, but land in the Deployment env in plaintext |
| `config.natsUrlSecret.name` | `""` | Read the whole URL from this Secret instead; wins over `natsUrl` |
| `config.natsUrlSecret.key` | `url` | Key within that Secret |
| `config.acknowledgeUnauthenticatedStore` | `false` | override the credential-less `natsUrl` refusal |
| `config.changeCursorKey.existingSecret` | `""` | seal `ListChanges` cursors with a Secret-held key instead of a per-process random one; required once more than one replica serves the same poll |
| `config.changeCursorKey.key` | `change-cursor-key` | Key within that Secret |
| `spicedb.endpoint` | `""` | delegate namespace visibility to SpiceDB; empty means the registry's `parent` column decides |
| `spicedb.presharedKey.value` | `""` | key SpiceDB was started with; required whenever `endpoint` is set, refused when it is not |
| `spicedb.presharedKey.existingSecret` | `""` | read that key from an existing Secret instead |
| `spicedb.freshness` | `minimize-latency` | `fully-consistent` quorum-reads every check so revocations take effect immediately |
| `spicedb.skipStartupSync` | `false` | stop the server publishing schema and grants; requires `sync.enabled` |
| `spicedb.sync.enabled` | `false` | CronJob running `sync-spicedb` to re-derive every grant (requires `endpoint`) |
| `spicedb.sync.schedule` | `*/15 * * * *` | schedule for that job |
| `metrics.enabled` | `true` | expose `/metrics` on port 9100 |
| `metrics.serviceMonitor.enabled` | `false` | create a Prometheus Operator ServiceMonitor |
| `gitMirror.enabled` | `false` | mount a PVC at `/var/lib/trogon-atlas/mirror` and enable the git mirror |
| `gitMirror.reconcile.enabled` | `true` | CronJob detecting store/mirror divergence (mirror must be enabled) |
| `gitMirror.gc.enabled` | `true` | CronJob repacking the mirror repository (mirror must be enabled) |
| `llm.provider` | `disabled` | `anthropic` enables LLM features; API key via `llm.apiKey.existingSecret` |

See `values.yaml` for the full list.
