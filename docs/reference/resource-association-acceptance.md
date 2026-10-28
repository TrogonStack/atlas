# Resource association acceptance requirements

Status: proposed requirements for the
[resource association contracts](resource-association-contracts.md). None of the
proposed fields exists, so no executable coverage is linked and no requirement
below has run. Logical fixture names are not manifest syntax, submitted
occurrence IDs or predicted owner UIDs. Passing tests for the shipped
[communication intent](communication-intent-acceptance.md),
[Channel and DataStore](channel-and-datastore-acceptance.md),
[Specification](specification-and-api-acceptance.md) and
[DataProduct](data-product-acceptance.md) fixtures do not satisfy these
requirements.

All fixtures use native authoring only. No EventCatalog file, project or
provider is read, converted or synchronized.

## Fixture and evidence

Use fresh registered namespaces and exact version `1` unless stated otherwise.
Create targets first and retain their returned owner UIDs, member IDs, port IDs
and ConnectionIds. Select them in follow-up guarded edits. Do not use predicted
IDs to bypass staging.

| Logical entity | Required setup |
| --- | --- |
| Specification `shop/events@1` | MESSAGE members `order-placed` and `order-shipped` with equal payload definitions, a PAYLOAD SchemaBinding for each to a native Schema with a nested object field, a HEADERS binding, an artifact-backed binding, and a non-MESSAGE member for negative selection. |
| Specification `shop/events@2` and a second Specification | Distinct keys, UIDs and member identities, used as exact-version controls. |
| Channel `shop/orders@1` | Address with parameters `region` (allowed values `eu`, `us`) and `tenant` (unrestricted), with one observation binding. |
| Channel `shop/orders-archive@1`, `shop/orders-archive@2` and a Channel with no address | Link targets and negative controls. Each is anchored by its own observation binding. |
| Component `shop/checkout@1` | Owner of two equal SEND claims of `order-placed` and a RECEIVE claim of `order-shipped`, plus an existing peer-directed communication declaration of the same message to an independent peer. |
| ExternalSystem `shop/warehouse@1` | Owner of independent claims and storage claims. |
| DataStore `shop/ledger@1` and `shop/cache@1` | Different declared access modes. `ledger` has two ReadModel attachments. |
| DataProduct `shop/receipts@1` and `shop/receipts@2` | Ports of both directions, an optional output contract on one output and none on another. |
| Behavioral controls | A valid ReadModel, Processor and AutomationSlice that are not bound to any claim. Existing Channel observation bindings, ChannelOperationBinding and API providers remain unchanged. |
| Subjects | Team and Person assignments, and Decision and Document subjects, selecting individual routes, triggers, links and storage claims. |

Capture requests and responses, actual identities, checked etags and revisions,
branch, authenticated caller, authorized namespace scope and completeness.
Backend, authenticated transport, CLI and MCP, Studio component and deployed
browser evidence must be recorded separately and never substituted for one
another.

## Message claim assertions

| Case | Required public outcome |
| --- | --- |
| Create and read | All fields preserve presence, Unicode, order and exact selectors. Equal claims receive distinct IDs. Owner UID is server-owned. |
| Separation from declarations | A claim and a peer-directed declaration of the same message coexist, keep independent IDs, and neither satisfies, deduplicates or implies the other. Existing declarations are unchanged and keep their required peer. |
| Peerless | A claim with no peer is accepted and creates no peer reference. A shipped declaration without a peer is still rejected. |
| Direction | SEND and RECEIVE are accepted. Absence, zero and unknown numbers are rejected. |
| Message selector | Missing owner UID or member ID, a non-MESSAGE kind, a stale UID, an absent member, `expected_digest` and a nonexact Specification Id reject atomically. A different version never resolves. |
| Group label | Labels are display only. Equal labels render together within one owner and direction. Blank and 257-byte labels reject; 256 bytes succeed. No claim can select a group. |
| Route | Selects a local claim. Declared parameter names and allowed values are accepted, including an empty string value. Unknown parameter names, disallowed values, any parameter on a Channel with no address and a claim ID from another owner reject. Omitting a required parameter is accepted and reported as unbound. Delivery mode absence stays undeclared. A repeated route to the same Channel is a separate occurrence. |
| Trigger | Accepts a RECEIVE claim and rejects a SEND claim. Rejects the claim's own message selector and non-MESSAGE targets. Accepts a trigger cycle between two claims. Condition absence differs from present condition prose. No predicate is evaluated and no Processor, Command or slice is created or required. |
| Field selection | Accepts PAYLOAD bindings of the claim's own message in the same Specification incarnation. Rejects HEADERS, another message's binding and a binding from another incarnation. Valid nested native paths succeed. A nonexistent segment, a 33-segment path, a 257th selector and a 1,025-byte opaque value reject. Artifact and unresolved bindings accept opaque selectors only, labeled unverified. An empty selector list is valid and does not mean all fields. A second selection for the same claim and binding rejects. |
| No fan-out | A claim with several routes, triggers and selections reports each separately. No cross product appears anywhere in reads, references or export. |
| Behavioral separation | Claims, routes and triggers bind no slice, Processor, Channel observation or API provider, and change no Channel anchor, ChannelOperationBinding or runtime path. |
| Identity | Editing `group`, `doc`, `condition`, `parameter_values`, `delivery_mode` or selectors preserves IDs. Changing direction, message, `claim_id`, channel, triggered message or binding requires a fresh ID. Reordering preserves IDs. Equal siblings never substitute for a selected ID. |
| Reference multiplicity | Each occurrence returns outgoing references with its own ID, owner UID, exact typed targets and distinct current field paths. A route reports its own ID, not its claim's. Equal endpoints preserve multiplicity. |
| Claim removal | Rejected while any route, trigger or field selection selects it, unless the same guarded final graph removes or replaces each one. |
| Subjects | Assignment, Decision and Document queries select one route, trigger, selection or claim without matching equal siblings, the owner, the Channel or the message. Primary endpoint context is the Channel for a route and the Specification for the others. |
| Domain claims | A Domain independently declares SEND/RECEIVE messages with no member evidence. Values remain queryable and distinct from member-derived claims; existing domain-scoped listing semantics are unchanged. |

## Channel link assertions

| Case | Required public outcome |
| --- | --- |
| Create and read | COMPOSES and ROUTES_TO preserve kind, target and ID. Equal links with different parameter values coexist with distinct IDs. |
| Target | An unavailable or nonexistent target rejects. The same exact key rejects. A different version of the same slug is distinct and accepted. |
| Parameters | Declared names and allowed values are accepted. Unknown names, disallowed values and any value for a target with no address reject. Required parameters without defaults reject when omitted; authored defaults may remain omitted. The target is never modified. |
| Anchor | A Channel with a link and no observation binding still fails the anchor rule. A link never anchors a Channel. |
| Cycles | A link cycle between distinct Channels is accepted at write. Bounded queries report it with exact visited identities and mark incompleteness at their limits. |
| Identity | `parameter_values` and `doc` edits preserve the ID. Kind or target changes require a fresh ID. |
| References and deletion | `links[i].target` appears with the link's ID. Safe deletion of the target is refused, including hidden referrers. Force deletion leaves the link readable and unresolved with generic unavailable hydration and no cascade. |
| No stored message list | No field stores a channel's messages. Messages carried by a Channel are returned as the inverse of independent Specification declarations, claim routes and operation bindings, with the declaring authority, scope and completeness retained. |
| Bounds | 1,024 links per Channel succeed and 1,025 reject. The 1 MiB complete-Channel boundary and its overflow reject without partial publication. |

## Storage access claim assertions

| Case | Required public outcome |
| --- | --- |
| Directions | READ and WRITE are accepted on Component, ExternalSystem and DataProduct. Absence and unknown numbers reject. A read-write relationship is two occurrences. |
| Capability mismatch | A WRITE claim on a store declared with a read access mode is accepted. The declared mode is returned beside the claim and no mismatch status exists. |
| Store | Exact DataStore resolving in the final view. An unavailable target rejects without leaking existence. |
| Attachment | An attachment of a different DataStore rejects. An attachment of the claimed store resolves. Absence is a store-level claim. Claiming an attachment never marks the ReadModel as used by the owner. |
| Port | Accepted on a DataProduct for an existing local port of either direction. Rejected on Component and ExternalSystem. A stale or absent port rejects. Removing a port that a claim selects needs the same guarded edit to remove or replace the claim. |
| Independence | A DataProduct OUTPUT store access does not create a claim, and a claim does not create output access or lineage. A ReadModel attachment does not create a claim. |
| Identity | `doc` edits preserve the ID. Store, direction, attachment or port changes require a fresh ID. Equal claims receive distinct IDs. |
| References | Store and attachment owner references carry the claim's ID and distinct paths, and both must name the same store. Inverse listings of readers and writers are queries, not stored fields. |
| Subjects and deletion | A claim is individually selectable. Safe deletion of the store is refused. Force deletion leaves the claim unresolved. |
| Bounds | 1,024 claims per owner succeed and 1,025 reject. The shared 4,096 association ceiling and 1 MiB limit apply to DataProduct. |

## Data product target assertions

Run shipped behavioral ports, contracts and lineage controls unchanged. Then
create identified descriptive INPUT and OUTPUT associations to Event, Command,
Specification MESSAGE (including query intent), Component/ExternalSystem,
Channel and DataStore. Assert exact typed target/version, contract presence,
ordered multiplicity, distinct IDs, stable descriptive edits and renewed
retarget/direction/contract identity. No association changes producing paths,
verified access or the meaning of ReadModel output ports.

A Specification message-channel declaration with no boundary claim remains
persisted and discoverable in forward/reverse channel results. Preserve local
MESSAGE member identity, channel parameter values and optional delivery mode.
Stale member IDs, wrong member kinds, unauthorized/missing channels, duplicate
IDs and invalid bindings reject atomically. Display/exports distinguish this
owner's declaration from boundary claim routes and operation bindings.

## Schema variant assertions

| Case | Required public outcome |
| --- | --- |
| Environments | Several bindings of one message and role with disjoint environment sets are accepted. Overlapping names across bindings of the same message and role reject. Equal names across different messages or roles are accepted. |
| Environment value | Blank, control characters and 65 bytes reject. 64 bytes succeed. Case differences are distinct. The 33rd environment on one binding and duplicates within a binding reject. |
| Default | One default per message and role is accepted. A second default rejects. A default with environments rejects. `false` is preserved as an explicit negative, and absence is preserved as unknown. Neither order nor version designates a default. |
| Roles | PAYLOAD and HEADERS evaluate independently. Each may have its own default. |
| Identity | Changing environments or the default flag requires a fresh binding ID. Doc edits preserve it. Existing selectors to the old ID become unresolved and are not rebound. |
| No resolution | No call resolves a binding for an environment. Reads return declarations labeled as declared, not deployed or verified. No latest marker is stored. |
| Dangling | New or edited bindings that select an unresolved type reject. A binding whose type is removed later stays inspectable. |
| Regression | Existing bindings with no environment fields read, export and reapply unchanged. Message mapping and operation roles are unaffected. |
| Mixed targets | One message may carry native Schema, ContractTypeRef and TypeLibrary alternatives, each scoped independently. |

## Admission and lifecycle assertions

For every refusal, read back native content, etag, UID and the complete
occurrence inventory to prove no partial owner or branch publication.

| Input or operation | Required result |
| --- | --- |
| ID syntax and presence | Omitted IDs are assigned. Empty, malformed, nil and duplicate supplied IDs reject, including duplication across relationships, communications, claims, routes, triggers, selections, links and storage claims. |
| New-owner supplied IDs | Valid supplied IDs are accepted only on a new owner. Existing-owner edits reject unknown supplied IDs and ID stripping. |
| Existing-owner edit | Descriptive edits and reorder retain IDs. Fingerprint changes require fresh IDs. |
| Separate removal and re-add | The old selector becomes unresolved and re-added equal content receives a new ID. Equal siblings and their subjects are unchanged. |
| Exact-content id-less reapply | Preserves UID, IDs and etag. |
| Preview and idempotency | Preview mutates nothing and returned preview IDs are unreserved. Retry semantics use the original caller intent. |
| Clone and new version | New owner UID scopes retained local IDs. Local sibling selections are rewritten consistently. Incoming subjects stay on the old owner. External targets do not auto-advance. |
| Delete and recreate | Old ConnectionRefs fail against the new UID. Entity refs follow exact-key semantics. Member and port selectors do not follow a replacement UID. |
| Target retarget | Changing a Channel, store, peer or product key renews the affected occurrence IDs. Member and binding selectors require explicit selection. |
| Target force deletion | The declaration stays readable and identified with generic unavailable hydration. No cascade or rebinding. Unchanged dangling declarations never veto unrelated writes. |
| Safe deletion | Referrers in other namespaces and hidden referrers protect Channels, stores, Specifications and products, including final batch deletions and subject owners. |
| Stale and final-overlay writes | Validate against the final merged graph and tombstones. A stale preview cannot publish an invalid target or a partial batch. Stage a new target before selecting its returned identity. |
| Branch land and rebase | Cover ordinary edit, land and rebase preservation, opposing-side removal with no implicit ID revival, explicit resolution renewing only restored IDs, owner deletion and restoration, and independent same-key owner creation. Use recorded base and current views and deterministic gates for races. |
| Export and reapply | Same-store export, diff and no-op reapply preserve full content, UIDs, IDs and etags for every field. Clipped exports are reported incomplete and are never complete reapply artifacts. New-store rebinding is not claimed. |
| Writer floor | A writer below the new contract revision is refused before it can drop any proposed field. Both mutation floors move with the revision. |

## Admission and provenance regression

| Case | Required public outcome |
| --- | --- |
| Reference filter | Each proposed path on Component and ExternalSystem appears in admission and in the incoming-reference guard, not only in raw extraction. A fixture that adds only extraction and not the admission filter must fail. |
| Provenance | Each outgoing reference carries the occurrence ID of its own route, trigger, selection, claim, link or storage claim. A nested path never inherits the claim's ID. |
| Fingerprint regression | Existing messages whose fields share the names `doc`, `label` or `condition` keep byte-identical fingerprints. Descriptive fields of the new messages do not alter existing fingerprints. |
| Closed inventories | Every proposed identified occurrence is selectable in the subject inventory by full ConnectionRef, and nothing outside the closed set is. |

## Authorization, capacity and failure assertions

| Case | Required public outcome |
| --- | --- |
| Reader authoring | `PERMISSION_DENIED` before mutation. Directory assignment grants no write authority. |
| Owner authority | Writing requires write on the owner and read or reference access to every target. No target write is required or granted. A claim confers no permission to use a Channel, and RECEIVE confers no authority over the sender. |
| Hidden and missing targets | A new or edited owner refuses unavailable targets without hidden titles, counts or existence. Readable retained declarations stay identified with generic unavailable hydration. |
| Branch visibility | Foreign branch access fails before snapshot or target lookup. Baseline and owner-branch controls retain behavior. |
| Owner limits | Boundary and overflow tests for 1,024 claims, 4,096 owner-wide association occurrences, 256 selectors, link and storage-claim limits, and the 1 MiB prepared owner, including assigned IDs and existing relationships. Rejection leaves stored content unchanged. |
| Shared resource scans | Owners and targets count once per exact owner and view under the 5,000 row cap. Exact authorized hydration is independent of display truncation. |
| Examined occurrence limits | Mixed owners share the 10,000 examined-occurrence budget across nested subject resolution, with equal endpoints consuming work. Selected inventories overflow without pagination or nested-counter bypass. |
| Unrelated queries | More than 10,000 unselected new occurrences must not exhaust queries that never inspect them. |
| Graph bounds | Incoming and impact indexing charges actual work. A baseline or branch overflow and a smaller authorized-lens success distinguish bounded work from unrelated charging. |
| Native path validation | A request resolving many selections to the same Schema reads that row once and charges the row budget once. |
| Output and incompleteness | The 4,096 emitted-record cap and the incomplete report behavior remain observable. Clients never turn incomplete results into complete claims or reapply artifacts. |
| Operational faults | Store, decode, timeout and authorization-service errors remain errors, never absence, empty success or safe-delete authorization. Overflow is `RESOURCE_EXHAUSTED`. Complete deletion guard failures refuse publication. |

## Source isolation and connected clients

Exercise Component, ExternalSystem, Domain, Channel, DataProduct and Specification
through the public supplied-bundle Source APIs.

- Creates carrying any nonempty proposed collection reject under every ownership
  policy, including title-only, before native writes or mapping-base advancement.
- Specification creates and whole-owner writes with an environment or default on
  any binding or an independent message-channel declaration, and DataProduct creates and writes with a resource association,
  reject in the same way.
- Selection or adoption of any proposed occurrence, and field ownership of
  `message_claims`, `storage_claims` and `links`, reject.
- Whole-owner writes reject when either the current or the incoming entity has
  any proposed content, including an attempt to clear it.
- Existing-owner title-only reconciliation preserves every proposed selector and
  all unrelated fields byte for byte.
- Channel, DataProduct and Specification are tested explicitly because they have
  no current named Source guard; passing only for Component and ExternalSystem
  does not satisfy this section. Generic, reflection and descriptor-driven field
  copy paths are each exercised.
- Independent architectural-relationship and communication Source restrictions
  stay in force.
- The supplied bundle profile is unchanged and its opaque relations never
  resolve to proposed occurrences.

Real native CLI and MCP tests must retain exact large numeric versions, selector
alternatives, returned IDs, optional presence, parameter maps and enum values
through authoring, preview, inspection, export and reapply. Studio stays a
read-only inspection surface. It must not crash on partial raw subjects, must
not substitute an equal sibling for a selected ID, and any loaded-only incoming
view declares its scope. Authenticated browser evidence must identify the real
backend and fixture and is separate from component or mocked results.

## Executable coverage and limits

No executable coverage exists. When implemented, link the following boundaries
here and keep their evidence separate. Names are proposals for the parent's
queue, not existing files.

| Proposed fixture | Verification boundary |
| --- | --- |
| `trogon-atlas-core/tests/resource_associations.rs` | Typed values, descriptor-scoped fingerprints, occurrence lifecycle, retargeting, subject resolution and native-only ownership helpers. |
| `trogon-atlas-server/tests/resource_association_backend.rs` | Persistence, final-overlay admission, reference and provenance handling, subjects, bounds and deletion guards. |
| `trogon-atlas-server/tests/resource_association_acceptance.rs` | Authenticated gRPC with returned identities, multiplicity, exact subjects and failure behavior. |
| Source backend additions | Native-only isolation for each owner kind, including the generic copy paths. |
| `trogon-atlas-client/tests/resource_association_manifest.rs` | Typed selectors and malformed manifest rejection. |
| CLI and MCP integration additions | Native authoring, preview, inspection and same-store export and reapply. |
| Studio component additions | Read-only display, loaded-owner scope and partial raw subject handling. |

Record commands and results at the candidate SHA separately from this
inventory. This document does not establish production-scale or deployed
browser readiness, and no proposal here implies behavioral binding, predicate
evaluation, message-to-Event or Command equivalence, connectors or portable
migration.
