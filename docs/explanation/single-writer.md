# Single-writer fencing

Status: implemented for the NATS store (`rsworkspace/crates/trogon-atlas-store/src/nats.rs`,
`rsworkspace/crates/trogon-atlas-store/src/writer_lease.rs`). Advertised on the wire as
`features.single_writer_fencing` (`GetServerInfo`).

Only one server process may mutate state at a time. Before this existed, the
mutation lock the old `validateReplicas` Helm check warned about was
process-local: running a second replica (or leaving an old one up during a
rolling deploy) silently broke `BatchMutate`/`MergeBranch` atomicity, because
nothing coordinated the two processes. The writer lease replaces that
process-local lock with a lease row in NATS KV, so correctness no longer
depends on exactly one process existing; it depends on exactly one process
holding the lease at a time, which NATS now arbitrates.

## Roles

Every server process runs with one of three roles
(`--role` / `TROGON_ATLAS_ROLE`, default `writer`):

- **Writer** competes for the lease and serves mutations while it holds it.
- **Standby** also competes for the lease (so it can take over) but is not
  presumed to hold it; it serves reads and refuses mutations until it wins
  the lease.
- **Reader** never competes for the lease. It only ever serves reads and
  always refuses mutations.

Role is static configuration; which process actually holds the lease at any
given moment is runtime state. A `Standby` becomes the live writer the moment
it takes over an expired lease, with no restart and no reconfiguration.

## The lease

The lease is a single row in a dedicated NATS KV bucket, holding the current
holder's id and a monotonically increasing epoch. `Writer` and `Standby`
processes run a background loop that tries, once every few seconds, to
create the row (if absent), renew it (if they hold it), or take it over (if
it expired). `Reader` processes only read the row to report status.

Lease creation, renewal, takeover and expiry checks use the lease stream
leader's `STREAM.INFO` timestamp. Receipt timestamps come from the same NATS
clock, so Atlas hosts do not need to share its wall clock. NATS nodes still
need synchronized clocks across leader changes. Missing or invalid server time
refuses a lease decision; Atlas does not substitute its local wall clock.

A process believes it holds the lease only while both are true: no attempt
since its last successful renewal proved the lease lost, and that success
happened recently enough that the lease could not have expired out from
under it. The local monotonic freshness interval starts before requesting
server time and writing the lease, so network latency cannot extend that belief. Only a lost compare-and-swap proves the loss. A renewal that times
out or loses its connection might still have landed, so it leaves the
belief to the second check instead of refusing writes until the next
attempt. That second check is a
local defense against a stalled renewal loop (for example, after a long GC
pause): without it, a process that silently stopped renewing would keep
believing it was the writer indefinitely, even after another process took
over.

## Fencing

Every mutating `Store` method checks the lease first and refuses with
`StoreError::NotWriter { role, epoch }` if this process does not currently
hold it. On the wire this becomes an `UNAVAILABLE` status carrying the
`NOT_WRITER` `ErrorInfo` reason, with `role` and `epoch` metadata (see
`docs/reference/failure-reasons.md`). A caller load-balancing across
replicas should retry against a different one; retrying the same process
will keep failing until it becomes the writer.

Automatic batch recovery (startup recovery and the periodic sweep) is gated
the same way: it only runs on the lease holder. During a rolling deploy, the
old writer and the new process can briefly overlap, and only the lease
holder may safely repair a batch it might still be mid-write on. Manual
recovery (`trogon-atlas-server recover-batches`) is deliberately not gated;
an operator invoking it has already decided it is safe to run.

The batch journal header carries the epoch the writer held when it started
the batch (`writer_epoch`), so a recovered batch is attributable to the
lease holder that wrote it, even across a takeover.

## Cross-process glossary reads

Enable these readers after all writers and recovery tools use the current
journal implementation, with earlier processes quiesced, drained and stopped.

A Reader or Standby does not share the writer's mutation lock or cache
generations. Glossary reads therefore bracket their work with durable Store
markers. The marker observes the native model, branch, namespace, source and
journal streams through the stream leader. It also pins the writer holder and
epoch; a renewal by the same holder does not change the model being read.
The payload reads use the leader-backed API too, so unchanged markers cannot
certify values obtained from a lagging follower.

Revisions alone cannot identify a read that starts and finishes while a writer
has paused between updates. Native batches and branch maintenance retain a
journal before their first write and through publication and cleanup. Branch
records preserve exact raw prior deltas, including their baseline snapshots and
tombstones. Recovery rolls back uncommitted work or completes committed history,
changesets and cleanup in their recorded scope. A branch write does not
publish a baseline change notification.

Marker acquisition refuses active source publication and unresolved journals,
and checks for publication changes while sampling the stream revisions. Before
returning a glossary result, the server compares the marker again and rechecks
the branch. A changed marker aborts the read. An unavailable marker remains an
operational error; it cannot establish that vocabulary is absent.

Namespace ownership needs its own cache refresh when the durable namespace
revision changes. That refresh and the glossary scan run outside the global
mutation lock. This lets the writer make progress while the reader returns a
stable result or an explicit refusal.

Extended branch and maintenance journals use phase codes that older recovery
tools report as unsupported before reading prior images. Current recovery also
accepts the previous baseline journal format. Upgrade recovery tools with the
server and retain the existing writer-lease and manual-recovery boundary above.

## Artifact publication

File verification can outlast a lease or overlap a writer takeover. A local
writer check before provider I/O cannot authorize the eventual publication.
Artifact completion therefore captures the original holder and epoch, then
conditionally publishes an immutable acceptance receipt in the same KV stream
as lease changes. The publication checks both the stream sequence and the
unused receipt subject. A concurrent renewal or takeover requires a fresh
snapshot, and a different holder or epoch refuses publication.

Lease snapshots use the leader-backed stream API because direct KV reads can
lag on a follower. The receipt's server publication timestamp must also precede
the captured lease expiry; a delayed message arriving afterward remains
rejected even when no takeover occurred. Renewals by the same holder and epoch
can extend that expiry while verification runs.

The receipt binds the artifact metadata, creator, backend and final object key. A
catalog `Ready` row is a projection of that acceptance, and every metadata,
content or download-grant read requires a matching valid receipt. Completion
can recover an already accepted result after session expiry or an interrupted
catalog update. It cannot turn a missing or rejected receipt into acceptance
without a new authorized publication.

The lease bucket (`trogon-atlas-writer-lease` by default) now also retains
artifact receipts. To take a consistent backup, quiesce upload transfers and
completion/publication on every writer, then wait for in-flight provider writes
and receipt acceptance to settle. Keep publication paused while snapshotting
object storage, the artifact catalog and the lease/receipt bucket together.
Restore that coordinated set; independently timed snapshots can retain a receipt
without its final bytes or lose acceptance for a retained object. Do not purge
the bucket as disposable lease state, expire its messages or configure eviction
of old records. Capacity refusal is
safe for retained receipts, but operators must leave headroom for further
publications. Artifact storage refuses a store without this publication
authority.

The store schema marker includes `artifact-receipts-v1`. Stores with the prior
marker are refused even when artifact storage is disabled. Follow the
[greenfield development policy](wire-compatibility.md#current-development-policy):
recreate development stores or provide a separately reviewed migration before
upgrading retained data. Stop older replicas before starting the new version;
a startup marker check cannot stop a process that is already running. Existing
artifact rows without verified receipts retain their bytes and require
reupload rather than invented historical acceptance.

## What this does not do

Fencing makes it safe for more than one process to exist; it does not load
balance writes across them. Exactly one process (the current lease holder)
accepts mutations at any moment, by design. Backends without a writer lease
(anything that does not implement one) report every process as the sole
writer, the behavior every caller saw before this fencing existed.

Namespace registry mutations and their authorization callbacks are serialized
within one server. The writer lease does not fence an already queued external
SpiceDB callback across processes. A delayed release from a former writer can
remove a grant recreated by its successor for the same namespace and owner.
An idempotent `RegisterNamespace` retry or authorization reconciliation repairs
that missing grant. Preventing it across takeover requires conditional external
updates tied to a namespace incarnation; local locking alone does not provide
that guarantee.

## Deployment

The server's Helm deployment uses the `Recreate` strategy so a rolling
update stops the old pod before starting the new one, keeping the window
where two processes could hold conflicting leases as small as possible. The
lease still protects correctness if that window is ever nonzero; `Recreate`
is belt-and-suspenders, not the mechanism that makes concurrent processes
safe.
