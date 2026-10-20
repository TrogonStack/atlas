# Specification and API contracts

Status: native first-release contract; schema, backend, CLI/MCP and Studio
implementation is pending. Names below describe the required model, not
existing protobuf messages or allocated field numbers. The
[acceptance contract](specification-and-api-acceptance.md) defines the connected
fixture and failure cases required before implementation can be called complete.

## Behavioral scope

A Specification describes a versioned interface or schema contract. An API is
a Specification with operations and explicit provider exposures; it is not a
competing top-level behavioral primitive. Integration messages map individually
to ReadModels. Request, response and notification roles belong to an operation's
use of a message, independently of the message's ReadModel mapping.

A request does not become a Command because it carries intent, and a
notification does not become an Event because it announces something. Existing
ReadModel-to-Processor observations, Processor-to-Command issuance, Command
handling and Event projections remain the behavioral paths. A Specification
creates none of those paths. Behavioral impact follows the mapped ReadModels
and existing slices. Editable operation-to-Command/Event intent shortcuts are
outside this release.

Standalone schema contracts need no invented operation, provider or behavioral
anchor. A message may have no confirmed ReadModel mapping; queries report that
absence explicitly. A declared operation must select at least one message, but
does not prove a complete behavioral implementation. Source overlays,
connectors, source handles, directory ownership and data offerings are outside
this contract.

## Entity and value rules

| Specification field | Contract |
| --- | --- |
| `id` | Existing exact `Id`: namespace, slug and positive breaking-change version. |
| `title` | Required nonblank display text. |
| `doc` | Optional Markdown; empty and absent mean no documentation. |
| `metadata` | Existing registered typed annotations; unsupported arbitrary nested values do not become graph references. |
| `supersedes` | Optional exact prior same-kind, same-namespace, same-slug version; existing terminating single-parent rules apply. |
| `artifacts` | Collection of retained `ContractArtifact` members. |
| `schemas` | Collection of `ContractSchema` members. |
| `messages` | Collection of `ContractMessage` members. |
| `operations` | Collection of `ContractOperation` members. |
| `exposures` | Explicit `ProviderExposure` associations. |
| `usages` | Explicit `ApiUsage` associations. |
| `dependencies` | Explicit `ContractDependency` associations. |

A Specification contains at most 1,024 members and 4,096 identified associations
across all nested collections. Exceeding either ceiling rejects admission.
A Specification must contain at least one artifact, schema or message. Member
and association collections may otherwise be empty unless a rule below requires
them. Collection order has no execution meaning; export preserves authored order.
Optional values preserve presence. Unknown enum numbers and missing required
oneof alternatives are rejected on native writes.

| Value object | Contract |
| --- | --- |
| `ContractMemberId` | Non-nil lowercase hyphenated UUID, unique across all members in one Specification incarnation. Distinct from `ConnectionId`. |
| `MemberName` | Nonblank display name, unique within its member kind using case-sensitive exact comparison. Renaming preserves member identity. |
| `ContentDigest` | Algorithm fixed to SHA-256 with lowercase hexadecimal digest of the exact retained artifact bytes. No text, newline or encoding normalization. |
| `ArtifactPath` | Unique case-sensitive relative POSIX path within the Specification; no empty segment, absolute path, backslash, `.` or `..` segment. A label, never a host filesystem instruction. |
| `MediaType` | Nonblank ASCII media type declaration; no inferred validation behavior. |
| `ContractFormat` | Alternative `OPENAPI`, `ASYNCAPI`, `GRAPHQL`, `JSON_SCHEMA`, `AVRO`, `PROTOBUF`, or `OTHER` with a required nonblank name. Known alternatives reject custom names. |
| `FormatVersion` | Optional nonblank declared version text, preserved verbatim; no compatibility ordering inferred. |
| `ProtocolName` | Lowercase ASCII token matching `[a-z][a-z0-9.+-]*`, as in the Channel contract. |
| `QualifiedProtoName` | Fully qualified protobuf message name, without leading dot; resolves through the selected TypeLibrary's compiled descriptors. |

## Members and exact selections

Every artifact, schema, message and operation has required `member_id` and
`name`, plus optional `doc`. A member is contained data, not a top-level entity
with an independently assigned Atlas version.

`SpecificationRef` contains an exact Specification `Id`. Like existing typed
entity references, it follows that key and does not freeze later nonbreaking
edits or pin an incarnation.

`ContractMemberRef` contains required `specification: SpecificationRef`,
`owner_uid`, `kind: ARTIFACT | SCHEMA | MESSAGE | OPERATION`, and `member_id`.
Resolution checks the exact owner key, incarnation, kind and member identity.
No selection falls back to name, position, path or a matching digest. Local
member selections inside a Specification use typed member IDs rather than
self-referencing external entity references.

Only artifact selections may additionally contain `expected_digest`. A matching
pin guarantees the retained bytes, not the rest of the Specification or its
mutable documentation. For a schema artifact, `ContractTypeRef` contains a
SCHEMA `ContractMemberRef`, required expected local artifact member ID and
artifact digest, and optional expected opaque selector. It accepts only an
artifact-backed schema. Resolution checks the artifact identity, digest and
selector including absence against the current schema definition. Changing any
of them invalidates that pin even when the schema member ID remains unchanged. An ARTIFACT
selection alone denotes a document, not a schema type.

An ordinary exact version remains editable under Atlas concurrency guards.
Changing a breaking-change version is explicit; no semantic format parser
automatically determines compatibility. A historical proof must retain its
checked snapshot or revision and bytes. Neither an entity version nor a digest
alone freezes message mappings, provider declarations or behavioral edges.

## Retained artifacts and schemas

| ContractArtifact field | Contract |
| --- | --- |
| `path` | Required `ArtifactPath`. |
| `format` | Required `ContractFormat`. |
| `format_version` | Optional `FormatVersion`. |
| `media_type` | Required `MediaType`. |
| `content` | Required nonempty UTF-8 source text; its exact UTF-8 bytes are retained. |
| `digest` | Required `ContentDigest`; backend verifies against `content`. |

Artifact content, format, format version and media type are immutable within a
member identity. Replacing any of them requires a new artifact member ID. Path,
name and documentation remain editable labels. Existing schemas stay on the old
artifact until explicitly retargeted; old byte pins remain valid while that
artifact is retained. Removing it leaves external selections unresolved. Binary
artifacts are not accepted in this release.

Artifacts live inside the entity and its authorized export. No remote-only
artifact, automatic fetch, external credentials, executable documentation or
mutable download URL substitutes for retained bytes. Native manifest export
must retain every byte and digest. Limits are 256 KiB of UTF-8 bytes per artifact, 512 KiB of total artifact bytes,
and 1 MiB for the complete protobuf-encoded Specification. Existing lower
entity/RPC/storage limits still apply. Over-limit admission fails atomically,
without truncation or a reference-only fallback. External blob
storage and streaming large contracts require a separate release contract.

| ContractSchema field | Contract |
| --- | --- |
| `definition` | Required `SchemaDefinition` association selecting a local artifact or exact native `SchemaRef`. |
| `definition.connection_id` | Required persisted association occurrence ID, selectable through `ConnectionRef`. |
| `definition.artifact` | Local ARTIFACT member ID and required expected digest. |
| `definition.selector` | Optional `OpaqueTypeSelector` only with the artifact alternative. |
| `definition.schema` | Exact named Atlas Schema identity; anonymous inline schemas are not a reference target. |

`SchemaRef` is a typed exact reference introduced with this feature, not the
existing `FieldType.ref` union. An `OpaqueTypeSelector` contains required
nonblank `syntax` and `value` text. Native admission validates presence and
retains both strings; it does not claim that a JSON pointer, GraphQL type, Avro
name or protocol operation exists inside artifact bytes. Query results label
these selectors unverified. A whole-artifact schema omits the selector and is
also a declaration, not semantic proof that the bytes define a valid schema.

A `SchemaBinding` contains `connection_id`, required role `PAYLOAD | HEADERS`,
required `type`, and optional `doc`. Its `type` is a typed alternative:
local SCHEMA member ID, external `ContractTypeRef`, exact `SchemaRef`, or
`TypeLibraryTypeRef`. The latter contains exact `TypeLibraryRef` and
`QualifiedProtoName`; existing compilation validates that the message exists.
This selection does not freeze future nonbreaking library edits. Native Schema
and TypeLibrary alternatives do not introduce artifact digests they cannot
verify. Repeated bindings to the same type are allowed as distinct declared
occurrences with distinct connection IDs.

## Messages and operation roles

| ContractMessage field | Contract |
| --- | --- |
| `schemas` | Collection of `SchemaBinding` occurrences. Empty means undeclared shape. |
| `mapping` | Optional `MessageReadModelMapping`: its own `connection_id` and required exact `ReadModelRef`. |

Each message independently selects at most one exact ReadModel. Several
messages may map to the same ReadModel, and several Specifications may describe
it. Another ReadModel version is a distinct selection. Mapping does not assert
schema equality or infer a native Schema from an external artifact. Message
examples, schema compatibility checks and automatic model generation are
outside this release.

| ContractOperation field | Contract |
| --- | --- |
| `binding` | Optional typed `OperationBinding` declaration. |
| `messages` | Nonempty collection of `OperationMessage` occurrences. |

An `OperationMessage` contains `connection_id`, required local MESSAGE
`message_id`, required role `REQUEST | RESPONSE | NOTIFICATION`, and optional
`doc`. A message can be used repeatedly in one operation, including with
different roles. Each occurrence has its own connection ID. Unknown roles are
not accepted in native authoring. A notification-only operation is valid;
absence of request or response does not fabricate one.

Request and response shapes come from the selected messages; there is no
competing operation-level request/response schema list. Response ReadModels
come only from RESPONSE message occurrences. REQUEST or NOTIFICATION mappings
do not silently count as response data. This release records no request-to-
response correlation or execution ordering among occurrences.

| OperationBinding alternative | Required declaration |
| --- | --- |
| `HTTP` | Uppercase ASCII method token and nonblank path beginning `/`; optional distinct integer response status codes in `100..599`. Path text is retained without template interpretation. |
| `RPC` | Nonblank service and method names; no automatic protobuf service lookup. |
| `CHANNEL` | Association `connection_id` plus exact `ChannelRef`. |
| `GRAPHQL` | Kind `QUERY`, `MUTATION` or `SUBSCRIPTION` and nonblank field name; no document parsing or field lookup. |
| `OTHER` | Required `ProtocolName` and nonblank binding description. |

An absent binding means undeclared transport. HTTP status declarations do not
pair particular message occurrences. A CHANNEL binding references descriptive
transport; it does not assert that the Channel observes every mapped message,
select a different Processor, or create a behavioral observation. Existing
Channel bindings remain the authority for those observations.

## Providers and API usage

`ProviderTarget` is a typed alternative containing exact `ComponentRef` or
`ExternalSystemRef`. It does not accept a Domain, Swimlane, directory entry,
URL or arbitrary entity kind.

| ProviderExposure field | Contract |
| --- | --- |
| `connection_id` | Required persisted association occurrence ID. |
| `provider` | Required `ProviderTarget`. |
| `operation_id` | Required local OPERATION member ID. |
| `doc` | Optional declaration-specific Markdown. |

An exposure declares that exactly that provider offers exactly that operation
with the operation's binding. The same provider-operation pair may appear only
once. Different providers can expose one operation; each declaration remains
separate. This release has no deployment endpoint or environment overrides.
A provider list never expands into an operation cross-product.

| ApiUsage field | Contract |
| --- | --- |
| `connection_id` | Required persisted association occurrence ID. |
| `caller` | Required `ProviderTarget`; caller and provider may be the same entity. |
| `operation_id` | Required local OPERATION member ID. |
| `exposure_id` | Optional local ConnectionId selecting one ProviderExposure on this Specification. |
| `doc` | Optional declaration-specific Markdown. |

A selected exposure must exist and select the same operation. Absence means
the caller uses the contract with provider undeclared. It does not select all
providers or allow a guessed provider. Usage and exposure are descriptive
dependencies, not proof of runtime traffic or Processor calls. Repeated usages
are valid separate declarations; callers, exposures and messages never form
an implicit Cartesian product.

## Dependency graph

A `ContractDependency` contains `connection_id`, required `target` alternative
of exact `SpecificationRef` or `TypeLibraryRef`, and optional `doc`. Duplicate
same-kind exact targets in this list are rejected. Dependencies describe
whole-contract prerequisites; external member/type references independently
declare their own dependencies and need no redundant entry here.

Member resolution never follows dependency lists looking for a matching name.
Descriptive Specification dependencies and external schema bindings may form
cycles. Resolving a ContractTypeRef checks its selected schema and local artifact
without recursively expanding dependencies or interpreting opaque schemas.
A self-dependency in the explicit dependencies list is redundant and rejected;
mutual dependencies are allowed. Local selections remain typed: operations
select messages, messages select schemas, and schemas select artifacts or
native Schemas. Existing TypeLibrary import and supersession cycle rules still
apply independently. Caller/provider cycles are also descriptive and create no
behavioral execution cycles. Bounded graph queries use visited identities and
report incomplete traversal when their limits are reached; partial traversal
cannot authorize deletion.

Reference enumeration, reverse lookup, impact, authorized closure, retargeting
and deletion protection must include every nested external reference above.
Local member/association selections are checked and reported with precise
member IDs; they do not create fictitious top-level EntityRef targets. Member
references index the Specification owner as well as their exact selection.
Incomplete diagnostics cannot authorize deletion; global safety checks must
remain complete even when visible reporting is capped.

## Identity and lifecycle

New member IDs are server-assigned when omitted, using UUIDv7. A new owner may
accept caller-supplied valid member IDs. On an existing owner, supplied IDs
must already be active in the same member kind. Additions omit their IDs;
removal followed by re-addition receives a fresh ID. No retired-member ledger
is accumulated. Returned persisted state is authoritative, and subsequent
edits preserve its member and association IDs. The same exact-content no-op
exception as [connection identity](connection-identity.md) applies to unchanged
id-less manifests; an edited manifest cannot strip retained identities.

Local selections require known IDs. On new-owner creation, clients may supply
the member and association IDs needed for same-owner selections. On an existing
owner, first add id-less members or exposures, then use their authoritative
returned IDs in a guarded follow-up edit. This release introduces no temporary
member handles or client-selected IDs for additions to an existing owner.
Preview-assigned IDs do not become active or reserved.

Members retain identity across rename, reorder, documentation, schema-definition,
mapping and transport-declaration edits. Immutable artifact content is the
exception described above. Changing member kind requires a new
member. Association IDs follow existing connection admission rules: changing
their role or selected endpoint requires a new connection ID. The member ID
is not that connection ID. A message mapping can therefore change while its
message member remains selected; the mapping occurrence changes identity.

Association endpoints include their containing member identity. For a
SchemaDefinition they include the schema member, target alternative, selected
artifact or native Schema, digest and opaque selector including absence. For
OperationMessage they include operation, selected message and role; for schema
bindings, containing message, role and selected type; for usage, caller,
operation and selected exposure or explicit absence. Exposures include provider
and operation. A Channel binding includes its containing operation and Channel.
Changing an opaque selector or digest in a selected external type counts as
retargeting that schema-binding association.

An OperationMessage selects the message member, not its derived ReadModel.
Changing that member's mapping renews only the mapping association. Likewise,
changing a schema member's definition preserves bindings that still select the
same local member. Immutable external type pins must still pass their explicit
definition checks.

All associations participate in `ConnectionRef` resolution using the exact
Specification owner, incarnation UID and local ConnectionId. Member selectors
use `ContractMemberRef` instead. Branch/conflict logic applies recorded
base/current/selected identity checks to members as well as associations;
it must not resurrect identities deleted by a present opposing same-incarnation
owner. Explicit restoration creates fresh IDs for those members/associations
and consistently rewrites same-owner local selections, or rejects an ambiguous
restoration. It never rewrites external references silently. Whole-owner
deletion with explicit keep-ours follows the existing surviving-incarnation
rule.

| Operation | Required behavior |
| --- | --- |
| Create/update Specification | Validate the complete proposed entity, member selections, external targets, digest checks, authorization and declared dependency rules in the operation's branch context. |
| Remove a locally selected member/exposure | Reject unless the same guarded final graph removes or explicitly replaces all local selections. No cascading deletion inferred from list order. |
| Safely delete a referenced Specification or other target | Refuse with authorized dependency diagnostics; inaccessible dependencies still protect deletion. |
| Force-delete an external target | Preserve declarations as inspectable unresolved references; no cascade or automatic replacement. |
| Replace artifact content | Add a fresh artifact member. Explicitly retarget local schemas and byte pins in the same guarded graph; retained external selections keep selecting the old member until its removal. |
| Remove a member selected by another Specification | Preserve the external declaration and diagnose a missing member, like removing a selected behavioral occurrence. |
| Delete/recreate a Specification at the same key | New UID invalidates old member and connection selections; plain SpecificationRef follows the key. |
| Clone/create another version | Fresh owner UID; local IDs may be retained. Rewrite same-owner local selections consistently. Preserve external exact targets unless an explicit mapping replaces them. |
| Guarded generic retarget | Rewrite supported exact entity targets and renew changed association IDs. ContractMemberRef and ContractTypeRef require explicit member selection; replacing only their owner key is not sufficient. |
| Preview, resolve and land a branch | Apply the same final-graph admission, concurrency, authorization and identity rules as writes. Preview identities are not reserved. |

Unchanged declarations may remain unresolved after accepted target edits or
forced deletion. Unrelated branch landing does not globally reject those
pre-existing diagnostics. New or edited Specification declarations must pass
their own admission rules. Backend reads must not expose hidden targets,
artifact bytes, member names or diagnostic details through partial access.

Cross-namespace references require authorized read/reference access to every
external target and write access to the owning Specification. A readable
Specification grants no additional access to referenced entities. Native
Schema and TypeLibrary targets may be selected across namespaces when the
caller has authorized reference access. A selected library compiles in its own
namespace, retaining the existing same-namespace TypeLibrary import rules.
This feature does not change entity Any validation, transcoding permissions or
library namespace ownership. Local member mutation requires owner write access,
not an independent member permission system.

## First-release support boundary

Native entity authoring, exact reads, reference/impact queries, validation,
guarded mutations, branch lifecycle and byte-preserving native manifest
import/export are required. CLI/MCP are authoring surfaces; Studio is a review
surface. Specification support must join existing connected graph and export
flows rather than rely on a separate registry.

All listed contract formats can be retained as UTF-8 source text with declared labels.
`OTHER` preserves an explicit format name. There is no first-release protocol
importer, source sync, remote fetch, schema compatibility checker, format
conversion, generated model, API execution or OpenAPI/AsyncAPI/GraphQL semantic
parser. Opaque selectors remain explicitly unverified. Existing TypeLibrary
compilation and typed native Schema resolution remain available only through
their explicit binding alternatives.

Malformed native members, wrong-kind selections, mismatched digests, duplicate
identities, missing required roles, stale UIDs, invalid local exposure selection,
invalid dependency declarations and unauthorized references fail admission atomically.
Unverified external document semantics do not masquerade as successful type
validation. Execution evidence belongs in the linked acceptance contract;
this reference alone does not establish implementation or release readiness.
