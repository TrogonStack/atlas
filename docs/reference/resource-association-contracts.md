# Resource association contracts

Status: native contract with implemented MessageClaims schema, core, backend
and connected authoring and inspection surfaces. The
[acceptance requirements](resource-association-acceptance.md#messageclaims-executable-evidence)
link the MessageClaims verification boundaries. Other resource-association
extensions have independent implementation and acceptance ownership.

The scope is Atlas native declarations only. EventCatalog-specific integration
is deferred to the final mile under the [native-first scope](eventcatalog-coverage.md#native-first-scope);
it is not active work or a dependency of these declarations. Source overlays
stay unable to select, own or write the proposed fields. Historical comparison
names do not establish source-format compatibility requirements.

Historical private checklists do not decide what exists. Every statement of
shipped support below was checked against the current protobuf schema, reference
extraction and admission code named in the evidence table.

## Disposition by concept

The table separates native support, behavioral limits and independent extension
work. The adopted declarations are integrated. Their executable acceptance boundaries
remain distinct from runtime behavior and from final checklist closure.

| Concept | Supported now | Behavioral boundary | Native disposition |
| --- | --- | --- | --- |
| Communication claims | Peer-directed CommunicationDeclaration and independent MessageClaims on Component and ExternalSystem; independent MessageClaims on Domain. | No implicit peer or executable behavior. | [Message claims](#message-claims) is implemented independently of peer-directed declarations. |
| Domain-level claims | Authored Domain.message_claims, independent of derived member views. | No member roll-up is inferred. | Existing domain-scoped listing semantics remain separate. |
| Triggers | ClaimTrigger names an exact message from a local RECEIVE claim, with optional condition prose. | No executable trigger. | No Processor or slice is inferred. |
| Field groups | ClaimFieldSelection selects exact PAYLOAD bindings and field selectors; group is descriptive. | Opaque selectors remain unverified. | Native inline paths resolve against exact Schema state. |
| Channel bindings on a claim | ClaimRoute selects an exact Channel with parameter values and optional delivery mode. | No transport is instantiated. | Required parameters without authored defaults must be supplied. |
| Channel composition and routes | Identified Channel links select exact Channels with authored target parameter values. | No routing or delivery execution is implied. | [Channel links](#channel-links) are implemented; a different exact version of the same slug is selectable. |
| Channel message list | Native inverse discovery joins Specification declarations, claim routes and operation bindings. | Each authority and occurrence remains distinct; no stored inverse list is inferred. | `ListChannelMessages`, CLI and MCP retain exact provenance and completeness. |
| Service and product storage access | Component, ExternalSystem and DataProduct retain explicit READ/WRITE storage claims and optional exact attachments. | A claim grants no runtime access and is distinct from a ReadModel attachment. | [Storage access claims](#storage-access-claims) are implemented. |
| Data product input targets | Identified descriptive resource associations coexist with behavioral ReadModel/Event input ports. | An association does not establish behavioral input. | [Resource associations](#data-product-targets) retain exact typed selections and direction. |
| Data product output targets | Descriptive output associations coexist with ReadModel output ports and optional contracts. | An association does not establish a producing path. | [Resource associations](#data-product-targets) retain independent identities. |
| Optional output contract | Contract absence is valid and reported as undeclared. | None. | None. |
| Schema variants, default, environment | SchemaBinding retains environment names and optional default designation with presence. | No deployment or environment resolution is inferred. | [Schema variants](#schema-variants) preserve role/message scopes and identity. |
| Unresolved schema references | New or edited declarations must resolve; unchanged dangling selectors remain inspectable. | None. A placeholder for unresolved references is a deliberate non-goal. | None. Recorded as an exclusion. |

## Evidence

| Claim | Location |
| --- | --- |
| Communication declaration fields and required peer | [infrastructure.proto](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto) |
| Channel has `bindings` and `broker` only; no link or route field | [architecture.proto](../../proto/trogonatlas/eventmodel/v1alpha1/architecture.proto) |
| DataStore capability fields and `attachments` only | [architecture.proto](../../proto/trogonatlas/eventmodel/v1alpha1/architecture.proto) |
| SchemaBinding has role, type and doc; no environment or default | [specification.proto](../../proto/trogonatlas/eventmodel/v1alpha1/specification.proto) |
| Input selection alternatives are ReadModel and Event; output selection is ReadModel only | [data_product.proto](../../proto/trogonatlas/eventmodel/v1alpha1/data_product.proto) |
| Domain carries no message, channel or storage field | [strategic.proto](../../proto/trogonatlas/eventmodel/v1alpha1/strategic.proto) |
| Channel outbound references cover `broker` and `bindings[i].observation.owner` only | [refs.rs](../../rsworkspace/crates/trogon-atlas-core/src/refs.rs) |
| DataProduct references cover inputs, outputs, contract and access paths | [data_product_refs.rs](../../rsworkspace/crates/trogon-atlas-core/src/data_product_refs.rs) |
| Admission filters Component and ExternalSystem references to the `relationships[` and `communications[` prefixes | [architecture_admission.rs](../../rsworkspace/crates/trogon-atlas-server/src/architecture_admission.rs) |
| Reference provenance attaches the occurrence ID by path prefix for `relationships[` and `communications[` only | [graph.rs](../../rsworkspace/crates/trogon-atlas-server/src/graph.rs) |
| Occurrence fingerprint clears the field names `connection_id`, `doc`, `label` and `condition` for every identified message, and a closed table gives wrapper messages a single target | [connections.rs](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs) |
| Source rejects relationships and communications by explicit name for Component and ExternalSystem; other kinds fall through to a generic whole-owner field copy | [source_integration.rs](../../rsworkspace/crates/trogon-atlas-core/src/source_integration.rs), [server Source guard](../../rsworkspace/crates/trogon-atlas-server/src/source_integration.rs) |
| Contract revision and both mutation floors at the pre-implementation baseline were 17 | [lib.rs](../../rsworkspace/crates/trogon-atlas-proto/src/lib.rs) |

These consequences recur below. A new collection is invisible to
admission and to reference provenance until those prefix checks are extended.
The fingerprint rule is global by field name, so a new message inherits it
unless a descriptor-scoped rule says otherwise. Source guards for new
collections on Channel, DataProduct and Specification do not exist; the generic
field copy would carry them.

## Shared rules

Every proposed association reuses the existing rules unless a section says
otherwise.

| Rule | Requirement |
| --- | --- |
| Identity | Each identified occurrence carries a `ConnectionId` from [connection identity](connection-identity.md). IDs are unique across the owner's complete uniqueness domain, which includes relationships, communications and any other identified collection. Omitted IDs request server assignment. Selection pins owner, server-owned incarnation UID and local ID. |
| Exact references | Entity targets use existing typed refs with exact namespace, slug and positive numeric version. No latest, range, family, title, name or position selector. Message and member targets use `ContractMemberRef` with returned owner UID and member ID. |
| Local selection | An association selecting a sibling in the same owner uses a local `ConnectionId` or `DataPortId`, as `ApiUsage.exposure_id` does. New-owner creation may supply IDs. An existing owner adds the target first and selects its returned ID in a guarded follow-up. No temporary handle or predicted ID is introduced. |
| Retarget and renewal | Changing any field that is part of the fingerprint listed per association requires a fresh ID. Descriptive fields preserve it. A selector never follows an equal-looking sibling. |
| Presence | Optional scalars and enums preserve presence. Absence means undeclared, never a synthesized default. |
| Descriptive only | No association introduces execution, delivery, ordering, subscription, transport availability, schema compatibility or authorization. Reachability through them is not runtime dependency. |
| Cycles | Descriptive graphs may contain cycles between distinct exact identities. Self-selection of the same exact key is rejected where stated. Traversal uses visited exact identities and reports incompleteness at its limits. |
| Dangling | New or edited declarations must resolve every selection in the final merged branch view. Unchanged dangling declarations remain inspectable and do not veto unrelated writes. No cascade, rebinding or garbage collection. |
| Fingerprint scope | Fingerprint contents are specified per association as behavior. The mechanism must be descriptor-scoped, not an extension of the global field-name list, so unrelated messages that happen to share a field name keep their current fingerprints. |

## Message claims

A message claim is an independently authored owner-local statement that the owner sends or receives an
exact Specification message, with no peer named. It is a sibling of the shipped
peer-directed [communication declaration](communication-intent-contracts.md),
not a replacement. The two never merge, satisfy each other or imply each other,
and neither is derived from the other. Existing declarations keep every current
meaning, including the required peer.

A message claim is not a native Event or Command, does not establish that a
Processor fires, and does not prove delivery. Query-kind and command-kind
source messages have no special native form: they are Specification messages
until separate message-to-behavior equivalence is designed.

### Wire contract

`Component.message_claims` uses tag 11, `ExternalSystem.message_claims` tag 9
and `Domain.message_claims` tag 7. Component tag 10 remains available for UI
hosting and Domain tag 8 remains reserved for authored members. MessageClaims uses flat
identified lists; it does not add an entity kind or union arm. Shared
`ClaimDeliveryMode` lives in `resource_association_values.proto` with
UNSPECIFIED 0, PUSH 1, PULL 2 and PUSH_PULL 3. Optional absence stays absent.

```proto
message MessageClaims {
  repeated MessageClaim claims = 1;
  repeated ClaimRoute routes = 2;
  repeated ClaimTrigger triggers = 3;
  repeated ClaimFieldSelection field_selections = 4;
}

message MessageClaim {
  ConnectionId connection_id = 1;
  CommunicationDirection direction = 2;   // SEND or RECEIVE, reuses the shipped enum
  ContractMemberRef message = 3;          // MESSAGE kind, no expected_digest
  optional CatalogTitle group = 4;        // descriptive display label
  optional CatalogText doc = 5;
}

enum ClaimDeliveryMode {
  CLAIM_DELIVERY_MODE_UNSPECIFIED = 0;
  CLAIM_DELIVERY_MODE_PUSH = 1;
  CLAIM_DELIVERY_MODE_PULL = 2;
  CLAIM_DELIVERY_MODE_PUSH_PULL = 3;
}

message ClaimRoute {
  ConnectionId connection_id = 1;
  ConnectionId claim_id = 2;              // local MessageClaim
  ChannelRef channel = 3;
  map<string, string> parameter_values = 4;
  optional ClaimDeliveryMode delivery_mode = 5;
  optional CatalogText doc = 6;
}

message ClaimTrigger {
  ConnectionId connection_id = 1;
  ConnectionId claim_id = 2;              // local MessageClaim, RECEIVE only
  ContractMemberRef message = 3;          // MESSAGE kind, the message that may follow
  CatalogText condition = 4;              // optional prose, never evaluated
  optional CatalogText doc = 5;
}

message FieldPath { repeated string segments = 1; }

message ClaimFieldSelector {
  oneof selector {
    FieldPath path = 1;                   // validated against a native Schema
    OpaqueTypeSelector opaque = 2;        // retained, unverified
  }
}

message ClaimFieldSelection {
  ConnectionId connection_id = 1;
  ConnectionId claim_id = 2;              // local MessageClaim
  ConnectionRef binding = 3;              // SchemaBinding occurrence, role PAYLOAD
  repeated ClaimFieldSelector selectors = 4;
  optional CatalogText doc = 5;
}
```

Identified occurrences are flat leaves under one container so that every
existing identity walker, which treats an identified message as a leaf, can
discover them. An occurrence nested inside another occurrence would be a new
shape; this proposal avoids it.

### Fingerprints and descriptive fields

| Association | Fingerprint | Descriptive, preserved on edit |
| --- | --- | --- |
| MessageClaim | Direction and the full message selector, including Specification key, owner UID, kind and member ID. | `group`, `doc`. |
| ClaimRoute | `claim_id` and the exact Channel Id. | `parameter_values`, `delivery_mode`, `doc`. |
| ClaimTrigger | `claim_id` and the full triggered message selector. | `condition`, `doc`. |
| ClaimFieldSelection | `claim_id` and the full binding ConnectionRef. | `selectors`, `doc`. |

The existing Channel binding already treats parameter-value edits as
descriptive. These rules continue that precedent. A claim keeps its identity
through added routes, triggers and field selections, so assignments and rationale
that selected the claim survive refinement.

### Validation

| Rule | Requirement |
| --- | --- |
| Direction | Require SEND or RECEIVE. Reject absence, zero and unknown numbers. |
| Message | Exactly one MESSAGE member in an existing Specification incarnation. Reject other kinds, absent members, stale UIDs and `expected_digest`. Resolve in the final merged view. |
| Equal claims | Equal direction and message may repeat with distinct IDs and distinct occurrence order. Order has no causal meaning. |
| `group` | A display label only. Equal labels within one owner and direction render together. It has no identity, membership record or selector, and no claim can reference a group. Nonblank, at most 256 UTF-8 bytes when present. |
| ClaimRoute | `claim_id` selects a claim in the same owner. The Channel must resolve exactly. Each parameter name must be declared in the Channel's address, each value must belong to its allowed set when one is declared, and an empty string is a valid value. A Channel with no address accepts no parameter values. Required parameters without defaults must be supplied. Omitted parameters with authored defaults retain those defaults; the route does not instantiate transport. `delivery_mode` absence means undeclared; no `push` default is synthesized. Repeated routes to the same Channel are separate occurrences. |
| ClaimTrigger | `claim_id` must select a RECEIVE claim. Reject on SEND. The triggered message must differ from the claim's own message selector. It need not equal any SEND claim of the same owner, and no corroboration status is reported. Cycles among triggers are legal and descriptive. Condition absence means none documented, not unconditional. |
| ClaimFieldSelection | `binding` must be a SchemaBinding on the same Specification incarnation as the claim's message, on that exact message, with role PAYLOAD. Reject HEADERS, another message's binding and a binding from another incarnation. At most one selection per claim and binding pair. Native `path` selectors are valid only when the binding resolves, within one hop and without expanding dependencies, to an exact native Schema, and every segment must exist in the resolved fields, nested through object fields. Bindings that resolve to an artifact, a TypeLibrary type or an opaque selector accept only `opaque` selectors, which stay labeled unverified. New or edited selections of a native Schema require that exact Schema to be authorized and available even when the selectors are opaque. Unchanged dangling declarations remain inspectable and do not veto unrelated edits. An empty selector list is valid and means no subset declared, not all fields selected. |
| Fan-out | A claim never expands into a cross product of routes, triggers and field selections. Each association is independent. |

### Bounds

At most 1,024 claims per owner. At most 4,096 identified occurrences in the
complete owner inventory, including the MessageClaims lists, existing
relationships, communications and identified ResourceGroup metadata. At most 256 field selectors per selection, each opaque
value at most 1,024 UTF-8 bytes and each native path at most 32 segments. At most
1 MiB for the complete prepared owner, including communications, relationships
and metadata. Lower existing limits win. Exceeding any ceiling rejects the whole
write without truncation.

### Authorization

Writing requires write permission on the owning Component, ExternalSystem or Domain and
authorized read or reference access to the Specification, each Channel, each
triggered message's Specification, the binding's Specification and, for native
path validation, the resolved Schema. No target write is required or granted.
RECEIVE confers no authority over the sender, a claim confers no permission to
use a Channel, and a route to a readable Channel reveals nothing hidden. Missing
and hidden targets share generic unavailable status without titles, counts or
existence leaks.

### Native integration

| Surface | Obligation |
| --- | --- |
| Reference extraction | Emit outgoing references with full `from`, `from_uid`, the exact occurrence `connection_id` and diagnostic paths `message_claims.claims[i].message.specification`, `message_claims.routes[i].channel`, `message_claims.triggers[i].message.specification` and `message_claims.field_selections[i].binding.owner`. The occurrence ID on a route, trigger or selection is its own ID, not its claim's. Raw path text is diagnostic, never identity. |
| Admission and provenance | Extend reference admission and occurrence provenance for Component, ExternalSystem and Domain for the new paths. Without both, new references are silently ignored. |
| Subjects | Extend the closed responsibility and CatalogSubject connection inventories so each identified occurrence is selectable by full ConnectionRef. Primary endpoint context: claim and trigger, the message's Specification; route, the Channel; field selection, the binding's Specification. Subjects never propagate to the Channel, message, owner or an equal sibling. |
| Impact and deletion | Typed references with occurrence provenance join bounded owner-level traversal. Safe deletion sees Channel, Specification and subject-owner referrers, including hidden ones, with the existing complete guard. Force deletion leaves claims unresolved. Retargeting a Channel key renews affected route IDs. Member and binding selectors require explicit selection, never owner-key rewriting alone. |
| Authoring surfaces | Get, put, batch, guarded preview, branch operations, export, diff, same-store reapply and snapshot recovery preserve every field, selector and ID. Writers below the new revision floor are refused before they can drop the container. |
| Removal of a claim | Reject unless the same guarded final graph removes or explicitly replaces every local route, trigger and field selection that selects it. |
| Ordering | Preserve authored order in each list without causal meaning. |

### Existing communication fields stay separate

Peer-directed declarations keep their required peer and every shipped rule. A
peerless claim is deliberately not a relaxation of that rule: relaxing it would
alter the fingerprint, admission and acceptance of a shipped contract for no
gain. A claim and a declaration about the same message are independent
occurrences. Authors who want both author both.

## Channel links

A Channel link is a descriptive Channel-to-Channel relation owned by the source
Channel. It covers source nested or composed channel references and routes. It
does not move messages, instantiate the target, relax the behavioral anchor or
merge Channels.

```proto
enum ChannelLinkKind {
  CHANNEL_LINK_KIND_UNSPECIFIED = 0;
  CHANNEL_LINK_KIND_COMPOSES = 1;    // this Channel is composed of the target
  CHANNEL_LINK_KIND_ROUTES_TO = 2;   // this Channel declares onward routing to the target
}

message ChannelLink {
  ConnectionId connection_id = 1;
  ChannelLinkKind kind = 2;
  ChannelRef target = 3;
  map<string, string> parameter_values = 4;   // binds the target's address parameters
  optional CatalogText doc = 5;
}
```

Channel gains `repeated ChannelLink links`, tag allocated at implementation
(the current last tag is 11).

| Rule | Requirement |
| --- | --- |
| Fingerprint | Kind and exact target Channel Id. `parameter_values` and `doc` are descriptive. |
| Target | Exact ChannelRef resolving in the final merged view. Reject a link whose target has the same exact namespace, slug and version as its owner. A different version is a distinct Channel. |
| Parameter values | Names must be declared by the target's address, values must belong to its allowed set when declared. A target with no address accepts none. Required parameters without defaults must be supplied; omission is allowed only with an authored default. The link never changes the target. |
| Equal links | The same kind and target may repeat with distinct IDs, for example with different parameter values. |
| Anchor | Unchanged. Every Channel still needs at least one observation binding, so a composing or routing Channel is not a way to create an unanchored Channel. |
| Cycles | Not checked at admission. Cross-owner cycle checks would require unbounded traversal at write time. Queries report cycles with visited identities and mark incompleteness at their limits. |
| Bounds | At most 1,024 links per Channel. The complete Channel stays within 1 MiB and the existing limits. |
| Authorization | Write on the source Channel and read or reference access to the target. No write to the target. |
| References | Emit `links[i].target` with the occurrence ID. The Channel arm of reference extraction currently covers only `broker` and `bindings[i].observation.owner`. Safe deletion, impact, retargeting and force deletion follow the shared rules; deleting a target leaves the link unresolved. |
| Excluded | An inverse channel message list is queried from independent Specification message declarations, boundary/Domain claim routes and operation bindings. Each authority remains distinguishable; no declaration is discarded because another authority is absent. |

## Storage access claims

A storage access claim states that a Component, ExternalSystem or DataProduct
reads or writes an exact DataStore. It is distinct from a ReadModel attachment,
which anchors a store to behavior, and from a DataProduct OUTPUT store access,
which selects a path to an offered ReadModel.

```proto
enum StorageDirection {
  STORAGE_DIRECTION_UNSPECIFIED = 0;
  STORAGE_DIRECTION_READ = 1;
  STORAGE_DIRECTION_WRITE = 2;
}

message StorageAccessClaim {
  ConnectionId connection_id = 1;
  DataStoreRef store = 2;
  StorageDirection direction = 3;
  ConnectionRef attachment = 4;   // optional ReadModelAttachment narrowing
  DataPortId port_id = 5;         // optional, DataProduct owners only
  optional CatalogText doc = 6;
}
```

Component, ExternalSystem and DataProduct each gain `repeated StorageAccessClaim
storage_claims`, tags allocated at implementation. The existing `StoreAccess`
message in the DataProduct schema is unrelated and keeps its meaning; this name
differs deliberately.

| Rule | Requirement |
| --- | --- |
| Direction | READ or WRITE. A read-write relationship is two occurrences. There is no combined value. |
| Fingerprint | Exact store Id, direction, the full `attachment` selector when present, and `port_id` when present. `doc` is descriptive. |
| Store | Exact DataStoreRef resolving in the final merged view. |
| Attachment | Optional. When present it must be a ReadModelAttachment occurrence whose owner is that exact DataStore incarnation. Absence means the claim concerns the store as a whole. Selecting an attachment does not imply the claimant uses that ReadModel. |
| Port | Valid only on DataProduct. Must select an existing local port of either direction in the same product incarnation, using the current port lifecycle. Component and ExternalSystem owners reject it. Absence means a product-level claim. |
| Independence from access | Neither a claim nor an OUTPUT store access implies the other. Output access states a path to offered data; a claim states a use of the store. |
| Capability | The store's declared `access_mode` is displayed beside the claim and never compared. No mismatch status is invented, and a WRITE claim on a READ-declared store is admitted. |
| Equal claims | Equal content may repeat with distinct IDs. |
| Bounds | At most 1,024 claims per owner. DataProduct claims count as associations under the existing 4,096 association ceiling and 1 MiB entity limit. |
| Authorization | Write on the owner and read or reference access to the store and attachment owner. No store write or data access is granted. |
| References | Emit `storage_claims[i].store` and `storage_claims[i].attachment.owner` with the occurrence ID. Both must name the same DataStore. Inverse listings of which owners read or write a store are queries over references, not stored fields. |
| Product port removal | Removing a port selected by a claim requires the same guarded edit to remove or replace the claim, as for lineage selections. |

## Data product targets

Keep existing behavioral input/output ports and verified access semantics.
Add a separate identified descriptive association collection on DataProduct;
these declarations never create producing behavior or change port validation.

Each `ProductResourceAssociation` has a ConnectionId, required INPUT or OUTPUT
direction, an exact typed target, optional exact output contract selection and
optional doc. Targets include native Event/Command, Specification MESSAGE member
(including query messages), Component/ExternalSystem, Channel and DataStore.
Native messages are selected explicitly, not classified from titles or labels.
An optional output contract uses existing OutputContract selectors and pins;
it does not convert a generic resource association into a verified data offering.

Preserve distinct occurrences, target kinds/versions, contract absence and
ordering. Direction, full target and contract selection are identity-bearing;
doc and wording are descriptive. Full owner ID inventory, admission against final
overlay, authorization, references/provenance, retargeting, deletion, branch
lifecycle and Source isolation apply as elsewhere in this contract.

Declaration queries/inspection label these as authored resource associations,
separate from checked data access and producing paths. A command/query request
association is permitted as descriptive intent without asserting it produces
data. Missing/hidden targets share generic unavailable diagnostics. Unrelated
field updates preserve declarations; their owner edits validate retained targets.

A product-to-product output-port input alternative is a separate Atlas design
choice, not a requirement from this comparison. It is not introduced here.

## Independent message-to-channel declarations

Specification MESSAGE members can declare channel carriage without a Component,
ExternalSystem or Domain claim. Add identified message/channel associations on
the Specification, selecting its local returned MESSAGE member ID, exact ChannelRef,
parameter bindings and optional delivery mode. Create/read the message member
before adding an association on an existing owner; no predicted IDs. Member and
channel/bindings/mode determine identity; descriptive prose does not.

Inverse channel message discovery includes these declarations, claim routes and
operation bindings with their distinct declaring owners and occurrence selectors.
Never require a boundary claim to retain an independent message declaration, or
silently treat an operation binding as an identical message association.

## Schema variants

A message may already hold several SchemaBinding occurrences for the same role.
What is missing is a way to say which applies in which environment and which is
the default. There is no environment entity, endpoint or deployment in Atlas, and
none is introduced.

```proto
message EnvironmentName { string value = 1; }

message SchemaBinding {
  // existing fields unchanged
  repeated EnvironmentName environments = 8;   // proposed, tag rechecked
  optional bool is_default = 9;                // proposed, tag rechecked
}
```

| Rule | Requirement |
| --- | --- |
| `EnvironmentName` | Nonblank, at most 64 UTF-8 bytes, no control characters, case-sensitive, retained verbatim. It names no entity, endpoint, credential or deployment. |
| Scope | Within one message and role, no environment name may appear on two bindings. A binding with no environments is unscoped. At most 32 environments per binding, unique within it. |
| Default | Within one message and role at most one binding may set `is_default` true. A default binding must have no environments. Absence means unknown and is never inferred from order, version or recency. `false` is an explicit negative claim. |
| Fingerprint | Environments and the default flag are part of the binding's identity, so editing either requires a fresh ID. A binding's scope is a meaningful part of the occurrence, and this needs no change to the fingerprint mechanism. |
| Resolution | Exact reads return declarations. No resolution RPC selects a variant for an environment; clients choose. Reads label the result as declared, never as deployed or verified. |
| Header and payload | The uniqueness and default rules apply separately per role. |
| Latest | EventCatalog's derived latest marker is excluded. Order and version never designate a default. |
| Variant names, formats and files | Excluded as duplicates: member names, `ContractFormat` and retained artifact paths already hold them. |
| Unresolved references | Excluded. New or edited bindings must resolve their type. A dangling selector may remain only after a target is removed, as the shipped contract allows. |
| Mapping | Message ReadModel mapping and operation roles are unchanged. Variants do not alter them. |
| Bounds | Existing 4,096 association ceiling and 1 MiB Specification limit apply. Environments add bytes, not associations. |
| Authorization | Unchanged. Environment names are descriptive data on a writable Specification. |

## Shared lifecycle

| Event | Required behavior |
| --- | --- |
| Create | Assign a fresh owner UID and omitted IDs. Accept valid supplied IDs on a new owner only. Reject duplicate IDs across every identified collection of the owner. |
| Existing-owner edit | Preserve active IDs with unchanged fingerprints. Reject unknown supplied IDs, changed fingerprints that retain an ID and identity stripping. |
| Reorder and descriptive edits | Preserve IDs. A selector identifies a mutable declaration, not its wording; historical assertions need checked content, etag and branch. |
| Remove and re-add | The old selector stops resolving; the new occurrence has a fresh ID. Equal endpoints do not revive identity. |
| Exact-content id-less reapply | Preserve stored UID, IDs and etag under the existing no-op exception. |
| Preview and apply | Preview IDs are unreserved. Admission is enforced again at guarded publication. |
| Clone and new version | New owner incarnation; local IDs may survive in their scope. External targets stay exact unless explicitly changed. Local selections between siblings are rewritten consistently. |
| Delete and recreate owner | Old ConnectionRefs fail against the new UID. Plain entity refs follow exact-key semantics; member and port selectors do not follow a replacement UID. |
| Target force deletion | Retain the declaration and report unresolved. No cascade or rebinding. Unchanged dangling declarations do not veto unrelated writes. |
| Branch, land and rebase | Preserve active same-incarnation IDs against the recorded base and both sides. Ordinary landing refuses deleted-ID revival. Explicit restoration renews only restored IDs and rewrites local sibling selections consistently or rejects the ambiguity. |

## Authorization, budgets and failure

The existing read, write and budget rules apply and are not widened.

| Boundary | Requirement |
| --- | --- |
| Reads and branches | Authenticated caller visibility and owned-branch checks occur before any snapshot or target hydration. Exact lookup bypasses display caps, never authorization or size bounds. |
| Visible rows | The shared 5,000 visible resource-row ceiling applies where these owners and targets are scanned. Each exact owner and view counts once across nested work. Channel link targets and storage stores are already scanned kinds. |
| Examined occurrences | Every claim, route, trigger, field selection, storage claim and link examined while resolving subjects, references or impact counts toward the shared 10,000 examined-occurrence budget. Equal endpoints each consume work. Continuation never resets required full-scan bounds. |
| Emitted records | The existing 4,096 emitted-record cap and explicit incompleteness reporting apply. A clipped export is never a complete reapply artifact. |
| Overflow | Typed budget overflow is `RESOURCE_EXHAUSTED`. Store, decode, timeout and authorization-service failures stay errors and never become absence, empty success or deletion authority. |
| Deletion guards | The complete incoming-reference guard sees every new reference path, across namespaces and hidden referrers, with bounded concurrency and a total timeout. Capped reporting never authorizes deletion. |
| Native field validation | Resolving a native `path` selector reads one Schema row per distinct exact Schema once per request, charged to the row budget. |

## Source isolation

All proposed fields are native authoring only. Initial implementation rejects
every Source path that could select, own, create or overwrite them:

- Selection or adoption of any proposed occurrence by a Source mapping.
- Field-level ownership of message claims, storage claims, links, product resource associations and message-channel declarations.
- Source creates carrying a nonempty proposed collection, under every ownership
  policy including title-only, because creation copies the incoming entity.
- Whole-owner Source writes when either the current or the incoming entity has
  a nonempty proposed collection, including an attempt to clear it silently.
- Source creates and whole-owner writes whose Specification has a SchemaBinding
  with environments or a default flag or message-channel declarations, and whose
  DataProduct has resource associations, in either current or incoming state.
- Every generic, reflection or descriptor-driven fallback path.

Supported unrelated field updates on existing owners preserve the proposed
fields byte for byte. Channel, DataProduct and Specification currently have no
named Source guard and would otherwise copy these fields on a whole-owner write,
so the guard must be added for them, not only extended for Component and
ExternalSystem. The supplied bundle profile is unchanged: its opaque relations
never resolve to these occurrences. Rejection is atomic, with no native or
mapping-base publication.

## Compatibility

The contract revision and both mutation floors are 19. Native MessageClaims
writers require that floor so an older client cannot silently drop the claim
container or its identified lists. The maintained manifest schema matches the
current descriptors. Further breaking writer extensions require a coordinated
revision and floor update, recorded in the wire compatibility document.
Development data may be recreated under the current greenfield policy.

## Design decisions and remaining objections

| Concern | Design context | Disposition |
| --- | --- | --- |
| Domain-authored claims | Authored claims could disagree with derived member views. | Resolved: Domain.message_claims is independently authored with admission, subject inventories, budgets and Source guards. Derived member views remain separate. |
| Peerless claim versus optional peer | One collection with an optional peer would be smaller, but it changes shipped admission, fingerprint and acceptance. | Separate `MessageClaims`. |
| Group as label | A label has no identity, so a group cannot be selected or assigned. | Label only, matching how the source uses it. |
| Event as DataProduct output | Verified ports remain read data, while an authored association is descriptive. | Required in the separate ProductResourceAssociation layer; no producing behavior is inferred. |
| Product-to-product input | The pinned schema does not show that products are accepted input targets, and adoption narrows a shipped sentence. | Not adopted until a decision. All other proposals do not depend on it. |
| Unanchored composing Channel | Allowing a Channel without an observation would relax a core Channel rule. | Anchor rule unchanged. |
| Partial parameter binding | Omitted required parameters are reported rather than rejected. A stricter rule would block routes that only narrow some parameters. | Required parameters without defaults must be supplied. Omitted parameters with defaults retain their authored default. |
| Composition cycle admission | Rejecting composition cycles needs cross-owner traversal at write time. | Cycles allowed and reported by bounded queries. |
| Field selection depth | Native paths resolve one hop to a Schema. Following TypeLibrary or nested dependencies needs recursion limits. | At most 32 path segments through inline object fields; no recursive TypeRef expansion. |
| Environment as a value object | A free value object permits typos where an entity would not. An entity would add a kind and lifecycle with no deployment meaning. | Value object, no entity. |
| Tag collisions | Component tag 10 is already proposed for UI hosting. Other pending proposals may claim neighbors. | Recheck at implementation. |

## Dependencies

Implementation depends on the following existing sites. None of them is changed
by this document.

| Site | Change needed |
| --- | --- |
| Connection fingerprint and occurrence walking in `connections.rs` | A descriptor-scoped way to name descriptive fields and a two-field local-selection signature for `ClaimRoute`, `ClaimTrigger` and `ClaimFieldSelection`, whose wrapper-table entries have only one target. |
| Reference extraction in `refs.rs`, `data_product_refs.rs` and the Specification walker | New outbound and rewrite paths for every field above, including guarded retargeting that renews affected IDs. |
| `architecture_admission.rs` reference filter | Admit the new Component and ExternalSystem prefixes. |
| `graph.rs` provenance decoration | Resolve the occurrence ID for each new nested path, not the claim's. |
| `architecture_resources.rs` and the Specification and DataProduct admission paths | Final-view validation for every rule above, including parameter values, native paths and variant uniqueness. |
| `responsibility_resources.rs` budget accounting and the subject inventories | Count the new occurrence lists and extend the closed inventories. |
| Source guards in the core and server `source_integration` modules | Named rejection for Channel, DataProduct and Specification, extended rejection for Component, ExternalSystem and Domain. |
| Protobuf generation and contract revision | Parent-owned. This task generates nothing. |
| Native manifest parser and CLI and MCP authoring | Typed fields with exact selectors, full ConnectionRefs and returned IDs. |

## Shipped versus proposed

Shipped: peer-directed communication declarations, Channel observation bindings,
Channel broker and address parameters, DataStore attachments and capability
fields, Specification SchemaBinding occurrences with PAYLOAD and HEADERS roles,
DataProduct ReadModel and Event inputs, ReadModel outputs, exact contract pins
and local declared lineage.

Implemented MessageClaims: independent claims, routes, descriptive triggers
and exact PAYLOAD field selections on Component, ExternalSystem and Domain.
Native authoring, exact occurrence references, subjects, Source isolation and
connected inspection are covered by the linked acceptance fixtures.

Channel links, storage claims, product associations, independent Specification
message-channel declarations and SchemaBinding variants have separate acceptance
ownership; MessageClaims evidence does not establish their coverage.

No proposal makes a trigger executable, widens the runtime grammar, establishes
message-to-Event or Command equivalence, infers delivery, or reads, migrates or
synchronizes any external catalog.

## Accepted implementation clarifications

Authored Domain claims are required and remain independent of derived member
views. A stored inverse channel message list is unnecessary only when all forward
authorities, including independent Specification message/channel declarations,
remain expressible and queryable. Source-derived resource target varieties are
represented by descriptive ProductResourceAssociation values rather than excluded
because verified behavioral ports are narrower. These decisions supersede
contrary defaults in the original open-objection table.

Channel partial parameter bindings are allowed only for omitted parameters with
an authored default; required parameters without defaults must be supplied.
Descriptive composition cycles between distinct exact channels are allowed;
self-links reject. Field-path depth is capped at 32 and traversal cannot follow
arbitrary TypeRef recursively. Limits and role-specific fingerprint validation
must be enforced before publication.
