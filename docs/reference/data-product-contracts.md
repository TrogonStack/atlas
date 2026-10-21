# DataProduct contracts

Status: native schema, backend admission and inspection, CLI/MCP and Studio
review support are implemented. The [acceptance reference](data-product-acceptance.md)
separates executable coverage from deployment and remaining acceptance limits.

## Scope and behavioral authority

A DataProduct describes a discoverable data offering through explicit input
and output ports. Every confirmed output selects exactly one ReadModel.
Inputs select ReadModels or confirmed local Events. External integration
information always uses ReadModels, including messages named events or commands
by an external system. Data-producing behavior remains in existing primitives
and slices; a product creates no transformation, observation or execution edge.

Contracts describe output shape. Access selections describe existing paths to
that same output. Neither input/output membership nor access establishes a
causal cross product. A ReadModel can be offered by several products, including
repeated ports within a product, without merging those occurrences.

This release is native first. Source overlays, source handles, source import,
sync, remote fetching, automatic format parsers, schema conversion, deployment
and runtime data delivery are excluded. Native manifest import/export means
round-tripping Atlas entities; it does not claim external-source fidelity.
Team/Person implementation is outside this slice. Ownership remains undeclared
until the pending responsibility layer supplies it; there is no duplicate owner
list or inferred owner from provider, namespace or producing slice.

## Entity and port values

Existing identity, concurrency, registered metadata and server-owned system
metadata rules apply to this entity kind.

| DataProduct field | Type and requiredness |
| --- | --- |
| `id` | Required existing `Id`: namespace, slug and exact positive breaking-change version. |
| `title` | Required nonblank display text. |
| `doc` | Optional Markdown; absent and empty mean no documentation. |
| `metadata` | Existing collection of registered typed annotations; no arbitrary nested reference discovery. |
| `supersedes` | Optional exact prior same-kind, same-namespace, same-slug version under existing terminating single-parent rules. |
| `purpose` | Optional nonblank prose describing the information offered and its intended use. |
| `inputs` | Collection of `DataInputPort`; empty is valid. |
| `outputs` | Nonempty collection of `DataOutputPort`; entries may explicitly remain undeclared as below. |

| Value | Contract |
| --- | --- |
| `DataProductRef` | Exact product `Id`; follows that key, without an incarnation or revision pin. |
| `DataPortId` | Non-nil lowercase hyphenated UUID, unique across input and output ports in one product incarnation. Not a `ConnectionId`. |
| `PortName` | Required nonblank display text; case-sensitive unique within its direction. Rename preserves identity. |
| `DataPortRef` | Required exact `DataProductRef`, `owner_uid`, direction `INPUT` or `OUTPUT`, and `port_id: DataPortId`. |

Each port contains required `port_id` in persisted state, required `name`, and
optional `doc`. Order has no behavioral meaning; native export preserves
authored order. Matching names, list indices, data targets or contract shapes
never substitute for a port ID. Unknown native enum numbers and supplied empty
oneofs are rejected. Optional records preserve presence.

| DataInputPort field | Type and requiredness |
| --- | --- |
| `data` | Optional `InputDataSelection`, containing persisted `connection_id` and required alternative: exact `ReadModelRef` or exact `EventRef`. |

An Event input is a confirmed local fact only when the authorized selected
model view contains an existing CommandSlice emission selecting that exact
Event. Admission must resolve that evidence; merely naming an integration
message as an Event is insufficient. The evidence is derived from existing
behavior, not a new product-owned behavioral edge. Queries identify the checked
slice and occurrence. Multiple valid emissions may confirm the same Event;
report the checked occurrences without inventing a unique emitter. Discovery
must include authorized emissions outside capped display projections. An
incomplete or failed search cannot confirm an input or prove no emission exists;
new admission fails explicitly when confirmation cannot finish within its budget.
Projection participation is separately reported and does
not prove an arbitrary output derives from this input.

| DataOutputPort field | Type and requiredness |
| --- | --- |
| `data` | Optional `OutputDataSelection`, containing persisted `connection_id` and required exact `read_model: ReadModelRef`. No Event, Command, store or operation alternative. |
| `contract` | Optional `OutputContract`, containing persisted `connection_id` and required type alternative defined below. |
| `access` | Collection of `OutputAccess` occurrences, each with persisted `connection_id`, required selection alternative and optional `doc`. Empty means access undeclared. |
| `derived_from` | Optional `DeclaredLineage` record, containing `inputs` and optional `doc`. |

Absent input or output `data` is permitted and classified as **undeclared**.
It is neither mapped nor a source-only/imported declaration. A supplied target
must resolve; unresolved targets cannot be newly admitted as undeclared ports.
An output without data may declare a shape contract and lineage, but must have
empty access. Its declarations establish no offered data or verified lineage.
A product whose outputs are all undeclared remains a draft offering, never a
confirmed mapped product. Presence of an unresolved retained reference differs
from absence of an authored selection.

## Output contracts and exact pins

`OutputContract.type` reuses exactly the existing native alternatives:

| Alternative | Required resolution |
| --- | --- |
| `SchemaRef` | Exact named native Schema `Id`; no inline anonymous schema or implicit field-shape inference. |
| `ContractTypeRef` | Existing SCHEMA `ContractMemberRef` with exact Specification key, incarnation UID and schema member ID; expected artifact member ID, SHA-256 digest and expected opaque selector, including selector absence, must match. |
| `TypeLibraryTypeRef` | Exact TypeLibrary reference and fully qualified protobuf message name; resolve through its existing compilation rules. |

See the [Specification type contract](specification-and-api-contracts.md#members-and-exact-selections).
No product-specific schema registry, format discriminator or duplicate artifact
bytes are introduced. An ARTIFACT member is not a type. Opaque artifact
selectors remain unverified; selection does not establish schema compatibility
with the ReadModel or semantic validity of retained format bytes.

An exact Atlas version selects a mutable breaking-change version. Nonbreaking
edits may change its contents under concurrency guards. Plain Schema,
ReadModel and TypeLibrary references do not freeze an incarnation. Member,
port and connection selections pin their owner incarnation and local identity,
not every property of the selected object. A ContractTypeRef additionally checks
its immutable artifact definition pin, not the entire offering or its behavior.
Historical conclusions require the checked snapshot/revision, branch context
and retained artifact bytes. Product versioning never advances its targets or
infers compatibility. Another version requires explicit selection.

## Access to the same output

Every access occurrence is contained in exactly one output port. Its selected
relationship must resolve to that port's exact ReadModel in the same authorized
branch/snapshot context. No match uses title, shape, version family or latest.
Repeated access selections are valid distinct occurrences with distinct IDs,
including repeated selection of the same path. They remain individually
inspectable and do not multiply unrelated providers or endpoints.

| Selection alternative | Fields and validation |
| --- | --- |
| `STORE` | Required exact `store: DataStoreRef` and required `attachment: ConnectionRef`. The connection owner must be that DataStore incarnation and its occurrence must be a `ReadModelAttachment` selecting the output ReadModel. |
| `CHANNEL` | Required exact `channel: ChannelRef`, `binding: ConnectionRef` and `observation: ConnectionRef`. Binding owner must be that Channel incarnation. Binding must select exactly this observation; observation must be a `ReadModelEdge` in `AutomationSlice.source_read_models` targeting the output ReadModel, with a valid paired Processor occurrence and target. |
| `OPERATION` | Required `operation: ContractMemberRef` of kind OPERATION, required `response: ConnectionRef`, and optional `provider_exposure: ConnectionRef`. Response must be an `OperationMessage` in that exact operation, on the same Specification incarnation, with role RESPONSE and a message mapped to the output ReadModel. |

For OPERATION access, provider selection is optional. When present it must
select a `ProviderExposure` on the same Specification incarnation and that same
operation. Its existing typed Component/ExternalSystem provider is derived,
not another independently editable product field. This selects only that
explicit provider and its operation binding. An absent provider selection means
contract-level access with provider undeclared, even when the Specification
declares several exposures. Contract-level access is allowed for a valid
operation-bearing Specification with no exposures; it does not itself make
that Specification an API. Omitting the product's provider selection never
changes the Specification's declaration-based API classification.

REQUEST and NOTIFICATION occurrences never satisfy output access. An operation
binding to a Channel alone does not establish the output observation. Neither a
DataStore's other attachments nor a Channel's other bindings count as selected
access. Access states no deployed endpoint, availability, runtime traffic,
credentials or new permission to data. Existing
[resource rules](channel-and-datastore-contracts.md) and
[Specification rules](specification-and-api-contracts.md#providers-and-api-usage)
remain authoritative for the selected declarations.

The explicit binding, attachment, response and optional exposure selectors are
candidate wire fields. They prevent occurrence ambiguity and stale access from
silently rebinding to an equivalent path. Reference discovery includes both
the exact selected owners and nested relationship selections.

## Declared and verified lineage

`DeclaredLineage.inputs` is a collection of `LineageInput` associations. Each
contains persisted `connection_id`, required local `input_port_id: DataPortId`
and optional `doc`. The ID must select an existing INPUT port in this same
product incarnation. Each input port may be selected at most once per output;
different input ports selecting the same ReadModel remain distinct.

Absent `derived_from` means lineage unknown. Present with empty `inputs` means
the author explicitly declares no input ports for this output; it does not
prove the absence of real dependencies. Present nonempty selections declare
per-output input membership. Input selection never defaults to every product
input. Documentary text is not executable evidence. An undeclared input may
be selected, but its data and verified relationship remain unknown.

Verified producing lineage is a separate query result supported by existing
primitives and slices in a retained checked context. It identifies the exact
input data, output ReadModel, traversed slices and occurrence identities, with
model/code evidence limits under the
[code-is-the-truth principle](../explanation/principles/code-is-the-truth.md).
A model path is model evidence, not runtime execution proof. A declared
selection without a checked producing path remains unverified. Conversely, a
verified producing path may reach an input that the product never declared;
report the difference without rewriting the declaration. Declaration validity
does not depend on proving producing behavior. Incomplete, unauthorized or
failed traversal cannot establish either verified lineage or no relationship.

## Port and association identity

Port IDs use the existing native member lifecycle:
omitted IDs on creation/addition receive server-assigned UUIDv7 values; valid
caller IDs may be supplied on new-owner creation. On an existing owner,
supplied port IDs must already be active in the same direction. New ports omit
IDs. Persisted IDs are authoritative, and previews do not reserve them. Local
lineage selections require known input IDs: supply valid IDs on new-owner
creation, or add inputs and select their returned IDs in a guarded follow-up.
There are no temporary handles or guessed owner UIDs.

All data selections, contracts, access occurrences and lineage input selections
carry their own `ConnectionId`, unique across the entire containing product.
They resolve through the product's exact owner, UID and ID under the existing
[connection identity contract](connection-identity.md). An absent lineage
record has no association; a present empty record has no input associations.
Port identity and association identity are separate: retargeting data preserves
the port ID and replaces the data-selection association ID.

| Edit | Required identity behavior |
| --- | --- |
| Rename, reorder or edit port documentation | Preserve port IDs and unchanged association IDs. |
| Repeat a data target on another port | Create a separate port and data association; never collapse by endpoint. |
| Change input/output direction | Remove and create a new port; the old ID cannot change direction. |
| Retarget data, contract, access or lineage input | Preserve the containing port, replace the changed association with a fresh ID, and revalidate the complete proposed product. |
| Remove and later re-add a port or association | Receive a fresh identity; old selectors do not revive. Remove and re-add in separate writes when unchanged endpoints would otherwise be indistinguishable. |
| Remove an INPUT port selected by lineage | Reject unless the guarded final graph removes or explicitly replaces every local lineage selection of that port. |
| Remove output data | Remove its access in the same guarded edit; optional contract and declared lineage may remain visibly undeclared as to data. |
| Delete and recreate a product at the same key | New UID invalidates old port and connection references; plain DataProductRef follows its exact key. |

Association endpoints include the containing port identity and their typed
selection. Data endpoints include target kind and exact key; contract endpoints
include the full type pin; access endpoints include its alternative and every
selected member/connection, including provider selection presence. Lineage
endpoints include output and input port IDs. Moving an association between
ports changes identity. Changes behind an unchanged external selector do not
renew the product association, but may invalidate resolution.

Retargeting an input's data preserves local lineage associations that still
select that port. Their declared meaning now uses the port's new data; former
verified conclusions do not follow the edit and require a new checked context.
Retargeting output data must explicitly prune or replace any access that no
longer matches. It does not retarget external access or type pins implicitly.

Cloning to another key/version creates a fresh owner UID. Local port and
association IDs may be retained; rewrite local selections consistently and
preserve external pins unless an explicit clone map changes them. Selectors
to the original product never follow the copy. Exact-content id-less reapply
may remain a no-op under existing identity rules; edited manifests cannot
strip IDs or match occurrences by position. No unbounded retired-ID ledger is
introduced.

Branches preserve owner incarnation and retained identities. Independent
creation of the same owner key yields distinct UIDs and requires explicit
conflict resolution. Merge, rebase and landing compare recorded base, current
baseline and selected branch identities. An identity removed by a present
opposing owner of the same incarnation cannot revive silently. Explicit
restoration assigns fresh IDs to restored ports/associations, consistently
rewrites local lineage selections or rejects ambiguity, and leaves external
selectors unresolved. Unaffected identities survive. Explicit keep-ours after
whole-owner baseline deletion restores the surviving branch incarnation under
existing connection rules. Stale branches must resolve recreated baseline
owners explicitly before landing.

## Admission, pruning and authorization

New or edited products validate their complete proposed content against the
authorized final graph, including batch, preview, conflict resolution and
landing. Require owner write permission and read/reference access to every
selected owner, member and derived endpoint. Cross-namespace references require
the same authorized model scope; they grant no target write or data access.
Exact targets outside capped projections must be resolved from the selected
branch view. Tombstones cannot fall back to baseline or another version.

Malformed/duplicate IDs, wrong kinds, nil UUIDs, blank required values, missing
oneofs, local selection failures, stale UIDs, invalid pins, access mismatches
and unauthorized selections reject admission atomically. Persisted content and
identities remain unchanged on failure. Internal diagnostics distinguish
missing, malformed, stale, forbidden and operational errors. Public diagnostics
must not disclose hidden namespaces, keys, titles, member names, bytes,
providers or graph paths. A readable offering grants no access to hidden data.

Accepted external edits or forced deletions may invalidate unchanged product
declarations. Preserve and diagnose them. Do not prune, cascade, match another
occurrence or silently drop access from export/query results. Report selected
access as stale/unresolved; it no longer qualifies as confirmed access. A new
or edited product with stale access fails admission until a guarded explicit
repair removes it or selects a valid replacement. Unrelated writes or branch
landing need not repair unchanged invalid declarations. Local pruning never
silently removes lineage; it must be an explicit part of the same guarded edit.

Safe deletion checks include product data, contract, access and every nested
external owner selection, even hidden inbound referrers. Safe-delete blocks
referenced entities, individually and in guarded batches, without leaking
hidden referrer details. Removing an externally selected port, member or
association through an otherwise accepted owner edit leaves its selectors
unresolved, consistently with existing member/connection lifecycle. Explicit
force-delete removes only the chosen entity. Deleting a product preserves its
referenced targets; deleting a selected target preserves referring products as
unresolved declarations. Generic entity-key
retargeting cannot guess replacement ports, members or connection occurrences;
these need explicit guarded selector repair and fresh changed association IDs.

## Query classification and bounds

`InspectDataProduct` accepts an exact `DataProductRef` and the existing branch
request metadata. The CLI exposes `inspect-data-product` and MCP exposes
`inspect_data_product`; Studio displays the same backend result. Per-port and
per-access states precede the product-level summary.

The response retains authored declarations, exact Event emission witnesses,
checked producing paths and checked scope. Scope includes the branch,
authorized namespaces, visited exact identities and a revision digest of the
retained checked content and etags. Reads form an assembled view under the
existing single-writer assumption; the digest is neither a distributed
transaction snapshot nor a retained historical replay token. Studio does not
attach current inspection evidence to historical declarations.

Producing traversal returns at most one checked path per input/output port
pair. Repeated ports remain independent pairs. Discovery, edge examinations,
traversal and combined returned connection references are bounded. A gap or
cycle reachable from a selected input suppresses that input's producing paths
and marks coverage incomplete. Disconnected defects do not invalidate its
checked traversal; complete input traversals retain their paths independently.
An output's incomplete flag describes aggregate input coverage and can coexist
with checked paths from complete inputs. Discovery, graph-construction or
combined returned-evidence exhaustion remains request-wide and suppresses all
producing-path claims. Event confirmation witnesses remain a separate result.

| Result dimension | Required distinction |
| --- | --- |
| Output data | Undeclared, confirmed exact ReadModel, unresolved/stale retained selection, or inaccessible with privacy-safe details. |
| Contract | Undeclared, resolved native type, opaque/unverified format semantics, or failed/stale selection. A resolved declaration is not compatibility proof. |
| Access | Undeclared when empty; each authored occurrence separately resolved or stale/inaccessible. OPERATION access separately reports selected provider or provider undeclared. |
| Lineage | Absent/unknown, explicit empty declaration, declared inputs, separately checked producing evidence, incomplete traversal or operational error. |
| Offering | Unresolved when any supplied data, contract, access or local lineage selection fails validation. Otherwise draft when any output data is undeclared; confirmed mapped only when every output selects resolving data. Undeclared input data remains visible and does not assert complete lineage. Empty optional access does not assert deployment. |
| Ownership | Undeclared pending the responsibility layer. |

These dimensions are derived from authored declarations and authorized checked
state, not editable status enums. Classification must not expose hidden target
details or turn inaccessible data into a public confirmation. Incoming/outgoing
references, impact, authorized closure, export and safe-delete must discover
every nested selection. Local lineage indexes the containing port identity,
not a fictitious external EntityRef. Descriptive associations introduce no
execution ordering or product-to-product composition. Existing supersession,
TypeLibrary import and behavioral cycle rules remain unchanged. Traversal uses
visited exact identities and preserves repeated occurrences without unbounded
expansion. A cycle is not evidence of producing behavior by itself.

Admission ceilings are 1,024 total ports, 4,096 total associations
across all nested collections, and 1 MiB for the complete protobuf-encoded
DataProduct. Each LineageInput counts as an association. Existing lower entity,
RPC and store limits still apply. Exceeding a ceiling rejects atomically, without
truncating ports or references. DataProduct uses entity kind `33`, entity union
field `34`, and wire contract revision `12` with matching mutation revision floors.

The required visible resource scan budget is 5,000 rows combined across
DataProducts, Channels, DataStores and Specifications in visible namespaces.
The scanner includes DataProducts. Exceeding it returns `RESOURCE_EXHAUSTED`; an empty
result is not a substitute. Bounded impact/lineage queries retain checked scope
and explicitly mark incomplete traversal. Operational failures remain errors,
not undeclared data or negative lineage findings.

Event-confirmation discovery uses a separate per-request budget: at most 5,000
distinct Event and CommandSlice rows combined, and at most 5,000 emission
occurrence examinations. These do not consume the visible resource-row budget.
Count each exact kind/key/version row once within the selected branch view,
including candidate CommandSlices that yield no matching emission. Count every
examined emission occurrence, including repeated occurrences targeting the same
Event. Reusing a loaded row does not add another row charge; examining its
emissions again still consumes occurrence work. Share these ceilings across all
products and Event inputs in one public request, including a guarded batch;
pagination and per-product loops do not reset them. Confirmation may stop once
every selected Event has a valid authorized witness. If completing required
confirmation would exceed either ceiling, return `RESOURCE_EXHAUSTED` and reject
the entire admission request atomically. A diagnostic query marks that evidence
incomplete, without asserting a missing emission or confirmed lineage. Backend
failures retain their operational error instead of becoming model findings.

Producing graph construction separately permits 5,000 source or emission
occurrence examinations. A CommandSlice examined in both phases incurs work
in each; reusing its fetched row does not exempt producing construction work.

Deletion reporting may use the existing capped-projection fallback only for
visible resource-budget exhaustion, with incomplete reporting explicit. Other
scanner errors remain errors. Safe-delete separately requires a complete
incoming-reference check across namespaces in the merged branch view, without
the visible row ceiling or hidden-referrer disclosure. Batches check all delete
targets together. Bounded concurrent reads and a total timeout must refuse
deletion on timeout, backend error or incomplete scan. No partial query result
can authorize deletion. This contract neither assumes a new persistent reverse
index nor changes the existing guard's authority.
