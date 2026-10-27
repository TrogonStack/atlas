# Communication intent contracts

Status: native support is implemented in the schema, core, backend, CLI/MCP
authoring and Studio inspection. The
[infrastructure schema](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto)
is the wire contract. This layer retains send/receive intent before behavioral
mapping; it does not execute communication. The
[acceptance requirements and executable coverage](communication-intent-acceptance.md)
distinguish implemented fixtures from exhaustive and deployed evidence.

## Scope and existing representations

| Representation | Existing meaning and boundary |
| --- | --- |
| Component and ExternalSystem | Exact deployable and third-party boundaries in the [infrastructure schema](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto). Their [architectural relationships](system-relationship-contracts.md) carry directed labels, not message flow. |
| Specification message | A retained `ContractMessage` can exist without a ReadModel mapping. Its [member selector](specification-and-api-contracts.md#members-and-exact-selections) pins the exact Specification, owner UID, MESSAGE kind and local member ID. |
| Behavioral integration | [AutomationSlice](../../proto/trogonatlas/eventmodel/v1alpha1/slices.proto) observes ReadModels through a Processor and may issue a Command. External input uses `ReadModel.external_source`; `Processor.calls` describes an outbound external seam in [moments](../../proto/trogonatlas/eventmodel/v1alpha1/moments.proto). |
| Transport | [Channel](channel-and-datastore-contracts.md) binds an actual AutomationSlice observation. It requires a behavioral anchor and cannot hold an unbound transport declaration. |
| Conditions | Existing edge documentation and typed metadata retain conditions; `Processor.doc_trigger` describes firing intent. None introduces another executable trigger shape. See [Event Modeling principles](../explanation/principles/event-modeling.md). |

Select only owner-local communication declarations between existing Component
and ExternalSystem boundaries. Each declaration identifies a Specification
message and says whether its owner intends to send or receive it relative to
an exact peer. No Communication entity kind, System, Service, Actor, broker
binding or execution engine is introduced.

A transport payload is not automatically a native Event or Command. A
notification may require a subsequent read, and a message definition can lack
a behavioral mapping entirely. See [pub/sub messages and events](../explanation/pubsub-message-is-not-an-event.md).
This first scope uses only MESSAGE member selectors; it does not add a union
of EventRef, CommandRef, arbitrary names and transport strings. Unsupported
payloads require an explicit Specification message definition before selection.

## Wire shape

Component carries `communications` at field 9 and ExternalSystem at field 8,
both repeated `CommunicationDeclaration`. Existing `relationships` and
`members` retain their meanings.

| CommunicationDeclaration field | Type and rule |
| --- | --- |
| `connection_id = 1` | Existing ConnectionId, required in persisted state; omission requests assignment. Explicitly present empty or malformed values reject. |
| `direction = 2` | New `CommunicationDirection`: UNSPECIFIED = 0, SEND = 1, RECEIVE = 2. Require SEND or RECEIVE; reject absence, zero and unknown values. |
| `peer = 3` | Existing ArchitecturalEndpoint: exactly one ComponentRef or ExternalSystemRef with exact namespace, slug and positive version. No generic-reference or string fallback. |
| `message = 4` | Existing ContractMemberRef restricted to MESSAGE, with exact SpecificationRef, returned owner UID and member ID. Require an existing message member. Reject `expected_digest`, which is supported only for artifact selections. |
| `label = 5` | Existing required CatalogTitle: nonblank, at most 256 UTF-8 bytes. Wording has no runtime semantics. |
| `condition = 6` | Optional CatalogText, nonblank if present and at most 16 KiB UTF-8. Retained owner-local condition prose, never an evaluated predicate. |
| `doc = 7` | Optional CatalogText under the same existing presence and byte rules. |

Reuse the named rich types and their validators; do not duplicate member IDs,
endpoint refs, title/text values or timestamp semantics. Contract revision and
mutation floors are 17; older writers are refused before they can drop these
fields.

## Direction, conditions and evidence

| Direction | Authored assertion |
| --- | --- |
| SEND | The containing owner declares intent to send the selected message to the peer. |
| RECEIVE | The containing owner declares intent to receive the selected message from the peer. |

The owner is the only authoring authority for either assertion. RECEIVE on an
owner does not authorize a write to the peer or assert that the peer has
declared SEND. Independently authored matching declarations do not merge,
satisfy each other, prove agreement or imply delivery. Direction is relative to
the owner, not a reversal of reference provenance: a stored reference always
runs from the declaration owner to its selected peer or Specification.

Equal peer/message/direction values may repeat with distinct occurrence IDs.
Preserve authored order without treating it as causal order. An empty collection
means no declarations in that checked owner, not no operational communication.
Exact self-peers with the same kind/key/version reject; communication within
one deployable remains a behavioral modeling concern. Distinct versions are
distinct boundaries; descriptive cycles between distinct boundaries are legal.

Condition absence means no condition is documented, not unconditional or
always-on delivery. Present prose can describe intent such as a notification
threshold, but provides no condition language, timer, schedule, subscription,
retry, ordering, delivery guarantee or automatic Command/Event production.
Changing a condition does not create an executable branch or a new trigger.

| Evidence level | What it establishes |
| --- | --- |
| Stored declaration | Accepted owner-local intent with exact peer and message identity. |
| Message member mapping | Only the mapping already defined on that exact Specification member; it does not bind this declaration to an observation, provider, sender or consumer. |
| Existing behavioral slices | Their own explicit Event Modeling paths, independently checked under existing rules. A shared message, member mapping or Component membership does not connect them to this declaration. |
| Runtime observation | Requires separate runtime evidence; no successful native write or reference query supplies it. |

Executable conditional behavior continues to require the existing
ReadModel-to-Processor observation and optional emitted Command. A raw Event,
message or condition cannot connect directly to a Processor or Command here.
No Channel is synthesized from intent; any Channel still requires an actual
observation. API operation roles, provider exposures, usages and transport
guarantees retain their own contracts without inference from SEND/RECEIVE.

Verified binding from a communication declaration to a behavioral path is
deferred. Decision/Document subjects can retain rationale about exact declared
and behavioral subjects, but their co-occurrence does not certify a mapping.
This release offers no mapped/unmapped completion flag, automatic matching,
inverse reconciliation or runtime-dependency classification.

## Identity and lifecycle

Reuse [connection identity](connection-identity.md) and the existing persistence
lifecycle. The selector is full ConnectionRef: exact owner, server-owned
incarnation UID and local connection ID. Communication and architectural
relationship IDs share the owner's complete uniqueness domain.

The semantic fingerprint contains the communication role, direction, full typed
peer and full message member selector. Label, condition, doc and order are
descriptive and excluded. A ConnectionRef identifies a mutable declaration,
not immutable wording; historical assertions also need checked content/etag,
branch and authorization context. Direction, peer or message selector changes
require a fresh ID, including owner UID or member changes inside the message
selector. Do not match or preserve identity by label, endpoint equality or index.

| Operation | Required behavior |
| --- | --- |
| Create | Assign fresh owner UID and omitted UUIDv7 IDs. Accept valid supplied IDs only on a new owner; reject duplicate IDs across all owner roles, including `relationships`. |
| Existing-owner add/edit | New occurrences omit IDs. Preserve active unchanged fingerprints; reject unknown supplied IDs, changed fingerprints retaining IDs and identity stripping in changed content. |
| Wording, condition or reorder edit | Preserve IDs and exact subject resolution. No claim that an earlier checked condition remains current. |
| Remove/re-add | Removed selectors stop resolving; a later addition receives a fresh ID. Indistinguishable replacement requires separate guarded removal/addition, not endpoint matching or a deletion ledger. |
| Exact-content id-less reapply | Preserve stored UID, IDs and etag under the existing no-op exception. Never omit the selected message's UID/member ID. |
| Preview/apply | Preview IDs are unreserved; retain original request digest/idempotency behavior and enforce admission again at guarded publication. |
| Clone/new version | New owner incarnation; local IDs may survive in their new scope. Peers and message selectors remain exact unless explicitly changed. |
| Owner delete/recreate | Old ConnectionRefs fail against the replacement UID. Plain peer refs follow mutable exact-key semantics, including same-key recreation; message refs do not follow a replacement Specification UID. |
| Branch/land/rebase | Preserve active same-incarnation IDs against recorded base and both sides. Ordinary landing refuses deleted-ID revival; explicit conflict resolution renews only restored IDs under the existing lifecycle. |

Editing a selected message's retained content does not automatically change its
member identity or renew communication IDs. Existing Specification member
lifecycle and checked revisions govern what a selector resolves to. Removing
a message member or recreating its Specification makes the old member selector
unresolved; no equal-name sibling or latest member repairs it.

## Admission, subjects and discovery

New or changed declarations require an authorized exact peer and a resolvable
MESSAGE member in the operation's final merged branch/batch view. Validate the
complete resulting owner, stored identity inventory and local values before
publication. A new Specification UID must be obtained from an authoritative
create response before selecting its member; preview and predicted IDs do not
provide temporary references for same-batch creation.

Unchanged dangling declarations remain inspectable and do not veto unrelated
mutations. Editing their owner requires repairing or removing invalid retained
references under existing edited-owner rules. An accepted message-member
removal or explicit forced target deletion can leave declarations unresolved.
There is no automatic cascade, rebinding or garbage collection.

Extend the closed responsibility and CatalogSubject connection inventories for
only `communications[]` on Component/ExternalSystem. Existing supported
`relationships[]` stay supported; raw members remain unsupported as occurrence
subjects. Resolve full owner UID and complete identity inventory before selecting
the communication. Primary connection endpoint context is the exact peer;
the message is an additional required typed selection, checked separately.
Identifying the stored occurrence is distinct from authorized peer/member
hydration. Missing and hidden targets share generic unavailable status without
titles, counts or existence leaks. Team/Person assignment and Decision/Document
subjects never propagate to the peer, message, owner or equal sibling.

| Surface | Implementation obligation |
| --- | --- |
| Native authoring | Get/put/batch, guarded preview, validation, branch operations, export/diff/reapply and snapshot recovery preserve every field and returned selector. Old writers refuse before dropping communications. |
| Outgoing/incoming references | Extract peer and the message's Specification owner with full `from`, `from_uid`, declaration `connection_id`, exact `to` and diagnostic paths `communications[i].peer.component`/`.external_system` and `communications[i].message.specification`. Both hits belong to the same declaration; do not invent a message entity ref. |
| Member and subject lookup | Exact member inspection and reference validation must check MESSAGE kind, Specification UID and local member ID. Entity-level references to its Specification do not prove that member exists. Reverse assignment/Decision/Document queries match full communication ConnectionRef only. |
| Impact | Include supported typed references with occurrence provenance under existing bounded owner-level traversal. Reachability establishes neither physical direction of message delivery nor runtime execution. |
| Guarded retarget | Plain peer key retarget renews affected communication IDs. Specification member selectors require explicit member/UID selection; never repair them by rewriting only their owner's key. |
| Safe/force deletion | Complete cross-namespace guards see peers, Specification owners and subject owners. Existing local member-removal semantics can leave unresolved selectors. Reporting caps never authorize deletion. |

Use existing exact reads and reference/subject queries; no network send,
subscription, delivery matching or new traversal RPC is selected. A path is
diagnostic, never an identity. Read declaration wording at the checked owner
etag rather than combining stale provenance with refreshed text.

## Authorization, bounds and source isolation

| Boundary | Required behavior |
| --- | --- |
| Authoring | Write permission on the declaration owner plus read/reference access to peer and exact Specification member. RECEIVE does not require peer write access and grants no authority to author its intent. |
| Reads and branches | Use authenticated caller visibility and owned-branch checks before snapshots or target hydration. Exact lookup bypasses display caps but not authorization or entity limits. |
| Owner limits | At most 1,024 communications per owner and at most 1 MiB for the complete prepared protobuf entity, including relationships and metadata. Lower existing limits win; no truncating accepted writes. |
| Discovery work | Communications share the existing 5,000 visible resource-row, 10,000 examined occurrence and 4,096 emitted-record budgets where applicable. Count exact owner/view once across nested work. Charge inspected communication inventories, not unrelated directory scans. Reference/impact indexing that enumerates them does consume work; continuation does not reset required full-scan bounds. |
| Failures | Typed discovery overflow is RESOURCE_EXHAUSTED. Preserve backend/decode/timeout/authorization-service errors; never turn operational failure into absence. Existing incomplete report/export contracts retain explicit incompleteness. |
| Deletion | Independent complete guards retain bounded concurrency/timeout and refuse on operational/incomplete checks; capped referrer reporting does not establish safe deletion. |

Initial implementation is native only. Reject Source selection/adoption of
communication occurrences and every Source ownership path that could change
`communications`, including generic/reflection fallbacks. Reject Source creates
with nonempty communications regardless of ownership policy: title-only creates
still copy the complete incoming entity. Reject Source whole-owner writes when
either current or proposed communications are nonempty, including silent
clearing. Supported unrelated field-specific updates on existing owners must
preserve the collection. Existing restrictions for architectural relationships
continue independently.

## Deferred boundaries

Executable predicates/triggers, event/command equivalence, typed behavioral
binding, inferred matching between SEND and RECEIVE, unbound Channels, broker
and address catalogs, schema compatibility, remote connectors and portable
selector rebinding are separate contracts. Agent tools, autonomous actors,
abstract grouping and system-level Persona interactions are also outside this
scope. No source declaration or parsed protocol implies executable behavior.
