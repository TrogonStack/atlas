# Source control API

Status: implemented with connected acceptance. The
[source integration contract](source-integration-contracts.md) remains the
required behavior; this page records the concrete API boundary.

Source records live outside the native entity graph. Retaining a selection is
provenance, not a native incoming reference, ownership grant or authorization.
The input profile remains
[`supplied-native-v1`](source-bundle-profile.md).

## Capabilities and clients

| Operation | Minimum role |
| --- | --- |
| Registration creation, update and deletion | Admin |
| Snapshot submission and mapping changes | Writer |
| Run preview, apply, cancellation and recovery | Writer |
| Inspection and report pages | Reader |

Roles are necessary but do not replace namespace, branch or native-reference
authorization. Destination namespaces use the IDs returned by
`RegisterNamespace`. A display name is not an adoption decision or a substitute
for a checked destination ID. Apply and recovery remain bound to the preview's
principal and current permissions.

The CLI exposes each RPC as a kebab-case command with `--request` containing its
typed protobuf JSON request. MCP exposes the corresponding source tools through
the same backend. Unknown fields, invalid scalar types, unknown enum values and
ambiguous selector alternatives reject before serialization. Native `Any` values
retain their declared type context.

`upload-source-snapshot` accepts an explicit inventory file and repeated
`--file logical-name=media-type=path` arguments. It reads only those supplied
files, with the profile's input bounds. It does not walk directories or follow
inventory pointers. `preview-source-run --native-owner
category=source-key=manifest-path` supplies a complete native owner from each
explicit manifest, using the connected tenant type context.

Studio is a read-only review surface. Its source queries inspect current
branch-scoped control state; a historical model view does not hydrate current
source records as historical evidence. Inspecting a run summary does not fetch
all operation or fidelity pages automatically.

## Registration and immutable snapshots

`CreateSourceRegistration`, `UpdateSourceRegistration`, `GetSourceRegistration`,
`ListSourceRegistrations` and `DeleteSourceRegistration` manage a registration's
administration namespace, existing destination namespaces, ownership policy,
deletion policy, budgets and retention. Configuration revisions are opaque;
guard updates and deletion with `if_match`.

`AcceptSourceSnapshot` takes the exact inventory bytes and named file payloads.
It is an intake operation, not native reconciliation. Source versions retain
their exact spelling. An identical version and bundle is idempotent; changed
bytes under the same version conflict. `GetSourceSnapshot` accepts an explicit
`include_bytes` flag. `ListSourceSnapshots` supplies bounded metadata pages.
Byte availability and native acceptance are distinct from a stored snapshot's
declared `COMPLETE` or `PARTIAL` coverage.

Caller timestamps, retained bases and commit markers cannot establish backend
history. Creation, acceptance, expiry and settlement evidence are backend-owned.
Source registration deletion never deletes native owners and cannot discard
unsettled publication evidence.

## Mappings and ownership

Mapping operations are `CreateSourceMapping`, `UpdateSourceMapping`,
`GetSourceMapping`, `ListSourceMappings` and `DeleteSourceMapping`.
`SourceResourceKey` combines registration identity, category and source-local
key. A mapping is branch-scoped and selects an existing typed alternative:

| Selection | Exact native value |
| --- | --- |
| Entity | `EntityRef` |
| Connection | `ConnectionRef` |
| Specification member | `ContractMemberRef` |
| Schema type | `ContractTypeRef` |
| DataProduct port | `DataPortRef` |
| Document or anchor | `DocumentSelection` |

Writable owner mappings use an entity selection and an explicit ownership
policy: none, whole owner, or a closed list of `SourceOwnedField` values. Fields
must exist on the selected native kind. Identity and system fields cannot be
source-owned. Ordered collections remain whole units. A real protobuf oneof
uses its group ownership unit, such as Document `BODY`, rather than separately
claiming its alternatives. Local connection/member/port/anchor selections are
descriptive and require ownership none in this release.

Existing-owner adoption requires explicit authorization and the observed owner
UID and etag. The backend retains the accepted source value and corresponding
native value separately. Local drift is reported without overwriting it or
silently adopting it as the new base. Conflict resolution requires a fresh
preview against current guards.

`GetSourceOverlay` returns ordered resource and relation evidence, raw pointers,
checked mapping status and authorized candidates. Unknown pointers remain inert.
MISSING requires a definitive exact owner read in an authorized namespace.
Hidden or incomplete lookup does not establish verified absence. This profile
accepts one active mapping per source key and branch; it does not enumerate
candidate correspondences or emit the reserved AMBIGUOUS status.

## Preview, publication and recovery

`PreviewSourceRun` binds an immutable snapshot, configuration revision, run
identity, complete native owner payloads and any explicit completeness decision.
Each payload names its source resource. Only owned units can change native
content; unowned fields remain local. Preview does not mutate native owners or
advance accepted mappings. Complete-inventory deletion proposals also require
the same partition's accepted base, explicit ownership and deletion policy.

`ApplySourceRun` requires the preview digest and run revision. It rechecks
authorization, native guards, mapping/base revisions, branch context and complete
final-view admission. A partial snapshot or source omission alone cannot delete
native content. A run is never silently split into separate native commits.

The native publication protocol uses a durable read/write fence while its
outcome is unsettled. Ordinary access must not expose provisional source writes
as committed state. The canonical commit decision retains exact postimages,
assigned identities and resulting revisions. Evidence finalization uses those
recorded values; a later native edit cannot become the accepted source base.

`RecoverSourceRun` resolves the same run's durable evidence. It does not replay
a committed batch. Missing acknowledgments or unavailable evidence remain
outcome-unknown until a definitive decision is available. `CancelSourceRun`
can prevent publication before commit; after commit may have started, it records
a request whose disposition follows recovery. Cancellation cannot undo a native
commit. Compensation requires a new guarded preview.

## Inspection and fidelity pages

`GetSourceRun` and `ListSourceRuns` return bounded run summaries. A summary with
`details_omitted` is not a complete plan or loss report. Counts and preview digest
identify the retained evidence.

`ListSourceRunOperations` pages the proposed operations and guards.
`ListSourceRunResults` pages dispositions, fidelity records and checked owners.
Pages bind the run revision and preview digest; a continuation must not mix
different revisions. A page shares its record and encoded-byte budgets across
all included record types. Follow continuations before claiming a complete
review. An incomplete or truncated result never establishes absence of conflicts
or losslessness.

Native same-store reapply and exact supplied-byte recovery are different fidelity
claims. Reports retain that distinction, authorized scope, source/native
identities, exact occurrence selections and unsupported or overlay-only content.
Remote source connectors, schema compilers, portable incarnation rebinding and
Atlas Linter remain outside this source profile.
