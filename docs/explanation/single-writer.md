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

A process believes it holds the lease only while both are true: its last
renewal attempt succeeded, and that success happened recently enough that
the lease could not have expired out from under it. That second check is a
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

## What this does not do

Fencing makes it safe for more than one process to exist; it does not load
balance writes across them. Exactly one process (the current lease holder)
accepts mutations at any moment, by design. Backends without a writer lease
(anything that does not implement one) report every process as the sole
writer, the behavior every caller saw before this fencing existed.

## Deployment

The server's Helm deployment uses the `Recreate` strategy so a rolling
update stops the old pod before starting the new one, keeping the window
where two processes could hold conflicting leases as small as possible. The
lease still protects correctness if that window is ever nonzero; `Recreate`
is belt-and-suspenders, not the mechanism that makes concurrent processes
safe.
