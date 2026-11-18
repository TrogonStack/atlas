# Artifact lifecycle and cleanup contract

Status: proposed cleanup contract. Upload, verification and retrieval exist;
automatic garbage collection, provider lifecycle tags, complete provider write
reservations and model-reference retention do not. See the implemented
[artifact workflow](../how-to/manage-artifacts.md).

## Confirmation and retention

`complete_artifact_upload` confirms that the declared bytes were verified,
persisted under a separate immutable key, read back, accepted by a fenced
publication receipt and materialized as `Ready` in the artifact catalog.
Confirmation does not mean attachment to an Event Model. A retry after
successful confirmation returns the accepted artifact.

Artifact retention is independent of upload-session expiry. A confirmed
file can be retained by its namespace and artifact ID for later association
with documents, exact connection occurrences, branches and later edits. Typed
model attachments are not implemented yet. Failure to attach a file
immediately must not turn a successful upload into an expired file.

| Condition | Cleanup policy |
| --- | --- |
| Upload never confirmed | Expire the session, fence completion and reclaim its temporary bytes and unaccepted promotion attempts. |
| Upload confirmed | Preserve the selected final object; reclaim temporary copies and failed promotion attempts. |
| Confirmed file has no known attachment | Retain by default. No recorded attachment does not prove that a file is unused. |
| Model or branch removed | Do not cascade-delete files. Other branches, documents or retained history can still need them. |
| Last reference removed under a future retention policy | Start a recovery window only after authoritative reference accounting proves eligibility. |

The first cleanup implementation must cover expired uploads and temporary
copies. Collection of confirmed files remains disabled until the reference
contract below is implemented. No blanket final-object TTL is permitted.

## Temporary markers and provider tags

The NATS KV catalog tracks upload lifecycle and ownership. Immutable
publication receipts in the model store's writer-lease bucket authorize
acceptance, and a matching receipt protects its selected final object even
before the catalog materializes `Ready`. Both stores must survive independently
of upload grants. The proposed cleanup eligibility checks must consult that
authority; a storage tag or prefix alone never authorizes deletion.

Temporary bytes use `staging/`; final candidates use fresh `artifacts/` keys.
Neither name is reused by a later upload or promotion attempt. Confirmation
selects one final candidate rather than making the client's staging key
permanent. A late PUT can therefore never replace an accepted file.

On S3 providers supporting object tags, use `atlas-object-role=staging` on
temporary objects and `atlas-object-role=artifact` on final candidates. Bind
the staging tag in the signed PUT's `x-amz-tagging` header and return it in
the existing required-header map. The signing identity needs the provider's
tagging permission. Clients must not be able to select a permanent role.
These tags describe object roles; the catalog determines whether a final
candidate was accepted. A failed tag update must not expose a Ready file to
automatic provider deletion.

S3-compatible deployments without tagging retain the same catalog and prefix
contract. Advertise the selected capability; do not return an unsupported
required header. Use NATS Object Store metadata for diagnostic role and
attempt markers where available. Neither NATS metadata nor S3 tags replace
the write receipts needed to reclaim interrupted writes.

## Durable upload and write records

Extend the current upload record rather than using a second uncoordinated
cleanup queue. All decisions affecting completion and reclamation use the
same record's compare-and-swap revision. Any separate scheduling index is
only a projection and must be repairable from the catalog.

The service already records its named final candidate before writing bytes and
its receipt ID before publication. The provider cleanup handles, operation
leases and cleanup claims below remain proposed; a named key does not identify
unnamed NATS chunks left by an interrupted write.

| Record data | Required purpose |
| --- | --- |
| Namespace, upload ID, creator, backend instance | Preserve ownership and bind cleanup to the exact configured bucket/store. A backend name alone is insufficient after configuration changes. |
| Session expiry and latest issued upload-grant expiry | Bound new upload admission and establish when staging cleanup may begin. |
| Phase | Retain `Pending`, `Receiving`, `Uploaded`, `Verifying`, `Ready`; add a terminal expired state that refuses further upload or confirmation. |
| Operation attempt, generation, lease deadline and maximum operation deadline | Distinguish live work from abandoned work and prevent indefinite renewal. |
| Prepared writes | Record each attempt's role, immutable key, backend identity and provider-specific cleanup handle before sending bytes. |
| Selected final attempt and confirmation time | Identify the only final object protected by successful completion. |
| Cleanup claim, not-before time, retry time, attempts and outcome | Support bounded workers, takeover after worker failure and retryable deletion. |

A prepared write is provider-owned data returned by a reservation operation.
Its schema must be validated against the backend and never accepted from an
upload client. Persist it before executing the write. S3 direct upload must
also have its durable record before its signed URL is returned.

Every failed or retried promotion retains its previous write receipt. Never
overwrite the only pointer to an earlier candidate. Bound attempts per
session; exhausting the bound requires a new upload rather than dropping
cleanup history. Ambiguous timeouts leave receipts eligible for inspection
and retry, rather than being treated as proof that no bytes were written.

## Completion and cleanup coordination

The catalog and acceptance receipts live in different streams. Catalog
compare-and-swap alone cannot exclude a concurrent receipt publication. Before
enabling deletion, the cleaner needs a shared fenced authority or a durable
coordination protocol that prevents acceptance while a cleanup claim is held.
Until that protocol is implemented and verified, retain uncertain candidates
and keep automatic deletion disabled.

1. Begin creates the durable session and staging reservation, then returns
   the upload instructions. A failed catalog write returns no usable grant.
1. An Atlas-mediated transfer claims a receiving lease before provider I/O.
   The request deadline includes reading the body. S3 direct transfer has no
   Atlas operation lease; its grant and staging reservation remain tracked.
1. Completion claims verification through compare-and-swap. It checks
   staged bytes, reserves and persists a fresh final-write plan, then writes
   and reads back the candidate. Renewals and phase changes require the
   current operation generation and an unexpired lease.
1. Completion publishes a writer-fenced acceptance receipt, then materializes
   `Ready` and selects the candidate in a conditional catalog update. The
   proposed operation and cleanup claims must also fence publication. A lost
   writer lease prevents new acceptance; a late catalog update cannot create
   authority. A lost response is recovered by retrying completion.
1. Cleanup conditionally expires abandoned sessions after session and
   operation deadlines permit. This transition competes with verification
   on the same revision. Before expiring a session, the worker must recover any
   valid acceptance receipt and protect its selected final object. A worker
   must reload after a conflict, not delete based on its old observation.
1. For a Ready record, cleanup claims only staging and unselected attempts.
   It does not change Ready into an unavailable phase or collect the
   selected final object.
1. The worker deletes exact provider receipts, records results and retries
   transient failures. It never removes the durable cleanup record first.

Workers run only on the current Atlas writer and also use per-record claims.
Writer failover and stale operation generations must be tested together;
the process-wide writer lease does not replace conditional upload updates.
All server-controlled writers recheck their generation before subsequent
provider operations and before publishing acceptance.

Catalog fencing prevents stale acceptance, but cannot undo provider writes
already admitted. A timed-out writer may still leave bytes after a deletion.
Retain cleanup tombstones and revisit receipts until provider quiescence is
proved or an enforced provider expiry bounds their survival. One successful
delete or an absent object is not sufficient proof of quiescence. The
initial implementation must not age out tombstones on a guessed deadline.

## S3 cleanup and late uploads

A presigned PUT can begin before expiry and finish afterward. Waiting for
URL expiry plus a grace period does not prove the object cannot reappear.
Expired and Ready staging reservations therefore remain scheduled for
repeated cleanup. Every additional grant, if grant renewal is later added,
must extend the recorded cleanup deadline before the URL is issued.

Use a provider lifecycle rule restricted to the Atlas staging prefix as a
backstop where supported. Choose its age longer than maximum upload and
verification lifetimes plus the cleanup grace. Configure noncurrent-version
expiry and delete-marker cleanup when bucket versioning is enabled. A
normal delete can leave historical versions; report this limitation rather
than claiming that all stored bytes were reclaimed.

Never apply a lifecycle rule to final candidates based on a mutable
unconfirmed tag. Catalog publication and provider tag updates are not an
atomic transaction. Unselected candidates are reclaimed by their durable
receipts; selected Ready candidates remain protected.

Current transfers use a single PUT. If multipart upload is added, persist
the provider upload ID and abort abandoned multipart uploads separately.
The control-plane upload ID is not an S3 multipart upload ID.

## NATS cleanup, including partial chunks

In pinned `async-nats` 0.51.0, `ObjectStore::put` creates an internal chunk
NUID, publishes chunks, and publishes object-name metadata afterward.
`list()` enumerates metadata, and named deletion requires that metadata.
A crash before metadata publication can therefore leave bytes that neither
operation finds. Recording only an object key does not close this gap.

The proposed provider reservation must include the object name and reserved
chunk NUID/subject before any chunk publication. The current SDK `put` API
cannot accept that reserved NUID. Implement a maintained, tested adapter for
the JetStream Object Store write protocol, or obtain an SDK extension with
the same reservation contract, before claiming complete NATS reclamation.
Do not describe metadata tagging as recovery for unnamed chunks.

Cleanup purges only that attempt's exact chunk subject and handles any
published name metadata after checking its identity. It must be repeatable
when metadata is absent, publication was acknowledged ambiguously, or late
chunks arrive. Shared wildcard purges are prohibited. Preserve standard
Object Store digest, chunk ordering and metadata semantics, with SDK reads
used to verify interoperability.

A separate expiring staging bucket is an optional backstop. It does not
solve interrupted writes into the durable final bucket. The current shared
bucket correctly refuses a nonzero TTL because it also holds Ready bytes.
Never enable bucket-wide expiry to solve temporary garbage.

## Retention after model removal

Future automatic final-object collection requires typed artifact references
and durable reference accounting. The artifact service currently has no
typed model attachment inventory. Free-form notes or external links do not
provide complete reference accounting. Until a typed contract and
reconciliation establish coverage, confirmed artifacts remain retained.

Reference identity must include the artifact namespace and immutable ID,
and the referring entity incarnation, pinned version, branch or retained
snapshot. Under that future contract, a Document attachment remains a
reference even if one of its subjects no longer resolves. Exact connection
selectors identify what it documents; they do not make its attachment
exclusive to that connection.

Use the complete authoritative reference traversal, never a capped impact
report or search result. Track references from all retained branches and
history, plus explicit retention holds. Removing a model removes only its
eligible references, after its history retention obligations end. Namespace
teardown requires a separate authorized retention procedure.

New reference registration must compete with a final-deletion claim at the
artifact authority. Reserve a durable reference hold before committing a
model attachment through the coordinated mutation path; a crash may leave
a conservative hold, never a committed reference without protection.
Release only after the authoritative model/history change is durable.
Reconciliation needs a consistent snapshot or mutation watermark and must
fail closed on incomplete coverage or read errors.

When no references or holds remain, mark the artifact unreferenced and start
a configurable recovery period. A proposed default is 30 days, enabled only
with complete reference accounting. Reattachment cancels eligibility before
deletion is claimed. After a deletion claim, a new attachment must fail
explicitly rather than point at bytes being removed. Operator pins can
retain reusable assets independently of any particular model.

## Worker configuration and operations

Proposed worker defaults are a five-minute sweep, a one-hour cleanup grace,
a batch of at most 100 records and at most four concurrent deletions. Persist
scan progress, bound each sweep's execution time and resume fairly after
restart so early retrying records cannot starve later records. These
are scheduling defaults, not proof that an upload has stopped. Lease and
maximum operation deadlines must be explicit and sized for the configured
provider and upload bound. All settings belong in server deployment and
Helm configuration; CLI and MCP use the same server policy.

Expose a read-only cleanup preview and structured upload/cleanup status
through CLI and MCP before adding a manually invoked sweep. Preview is an
observation, not later deletion authority; execution rechecks every record.
Report expired sessions, pending deletions, retried failures, retained Ready
objects and incomplete provider reclamation separately. Never log signed
URLs, bearer credentials or raw provider error payloads.

Do not TTL the catalog itself while any accepted artifact or cleanup receipt
depends on it. Preserve catalog and blobs together in backups. Unknown
objects and missing catalog records need reconciliation or an operator
quarantine procedure; absence from a failed or stale lookup never authorizes
deletion.

## Acceptance requirements

- Upload without completion expires and is reclaimed after its allowed
  activity ends; a late completion cannot revive it.
- Confirmed bytes survive session expiry, failed model attachment, branch
  removal and cleanup-worker restart.
- Concurrent completion and cleanup produce one catalog winner; cleanup
  never deletes that winner's selected final receipt.
- Crash injection before and after each provider write and catalog update
  leaves a discoverable receipt and supports idempotent recovery.
- Lost Ready responses return the same artifact on retry, without creating
  another accepted object.
- Expired leases, delayed acknowledgements, late S3 PUTs and stale writers
  cannot publish Ready or permanently evade repeated cleanup.
- NATS crashes before object metadata publication leave reclaimable chunks;
  cleanup cannot affect another attempt or SDK-readable Ready content.
- S3 tag headers are signed and enforced where supported; compatible
  providers without tags preserve equivalent catalog-based safety.
- Versioned S3 tests distinguish current-key absence from historical-byte
  reclamation. Provider failures preserve retry receipts.
- Multiple worker instances and writer failover preserve conditional claims,
  bounded work and eventual retries.
- Future final retention tests cover cross-branch and historical references,
  deletion/reattachment races, failed model writes, holds and recovery grace.

Use the public upload, completion and download seams for behavior acceptance,
with ephemeral NATS and an S3-compatible service plus provider-specific
coverage where semantics differ. Static review alone does not establish
crash recovery or storage reclamation.
