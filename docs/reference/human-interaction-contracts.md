# UI hosting and human interaction contracts

Status: implemented native contract. Component UI-hosting assertions and the
read-only inspection API have maintained backend, connected-client and
authenticated browser acceptance. The
[acceptance requirements](human-interaction-acceptance.md) distinguish these
checked model paths from runtime execution or deployed hosting.

## Scope and existing representations

| Representation | Existing meaning and boundary |
| --- | --- |
| Component | A deployable implementation unit in the [infrastructure schema](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto). Its raw `members` list remains curated implementation intent, without occurrence IDs or exclusive ownership. |
| Persona | A human behavioral role in [moments](../../proto/trogonatlas/eventmodel/v1alpha1/moments.proto), distinct from a directory Person, Processor and authenticated principal. |
| Human issuance | [CommandSlice](../../proto/trogonatlas/eventmodel/v1alpha1/slices.proto) has separate persona, UI and command edges. An automation-issued command need not have the human edges. |
| Human display | UiSlice has persona, UI and repeated source ReadModel edges. Equal ReadModel targets can occur separately. UI itself has no persona field. |
| Connection selection | Existing [ConnectionRef](connection-identity.md) selects an exact containing owner, its incarnation UID and a local occurrence ID. The role must be resolved from that owner's complete inventory. |

A Component may assert that it presents a selected UI. The assertion does not
claim exclusive deployment, ownership of the UI or Command handler, production
hosting, a human's permission, or execution of a selected path. Other Components
may retain their own assertions for the same UI.

A separate inspection combines a supplied hosting occurrence with an explicitly
selected issuance or display path. It does not search for every interaction or
infer a path from Component membership, matching labels, communication intent,
UI navigation, Screen composition or storyboard position. No System, Service,
Actor or interaction entity kind is introduced. ExternalSystem hosting,
external-system-owned hosting and abstract system-level interaction inspection
remain deferred.

## Native hosting shape

`Component.ui_hostings = 10` contains repeated `UiHosting` values alongside
`communications = 9`, `message_claims = 11` and `storage_claims = 12`.

| UiHosting field | Type and rule |
| --- | --- |
| `connection_id = 1` | Existing ConnectionId; required in persisted state. Omission requests assignment. Explicit empty or malformed supplied IDs reject. |
| `ui = 2` | Required existing UIRef, with exact namespace, slug and positive numeric version. No latest/family selector or generic-reference alternative. |
| `label = 3` | Required existing CatalogTitle: nonblank UTF-8, at most 256 encoded bytes. |
| `doc = 4` | Optional existing CatalogText: nonblank if present, at most 16 KiB encoded. Absence means no elaboration. |

The containing Component is the declaration owner. Equal UI targets and wording
may repeat as distinct occurrences; retain order without causal meaning. Empty
means no hosting assertions in that checked owner, not no deployed interfaces.
No direct Persona field, reverse editable list or arbitrary hosting metadata is
added. Contract revision and both mutation compatibility floors are 20. The client
compatibility gate refuses older writers before a mutation can discard hosting
assertions.

Hosting value validation must inspect `connection_id` presence before generic
connection normalization or assignment. The current shared
[connection helper](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs)
treats a present empty value as omitted; reuse alone does not enforce this
contract. A hosting-specific check must reject present empty or malformed IDs
while leaving omission available for assignment, following the existing
[architectural value validator](../../rsworkspace/crates/trogon-atlas-core/src/architectural_relationships.rs).
This proposal does not change the shared helper's behavior for other roles.

Native manifests may use existing typed-field UIRef shorthand. Under the
[manifest parser](../../rsworkspace/crates/trogon-atlas-client/src/manifest.rs),
an omitted namespace uses the manifest namespace and an omitted version is
`1`, not the containing Component's version. Shorthand cannot reconstruct a
full ConnectionRef, owner UID or occurrence ID.

## Hosting identity and native integration

Reuse [connection persistence](../../rsworkspace/crates/trogon-atlas-core/src/connection_persistence.rs)
and its ordinary-write, branch and conflict-resolution lifecycle. Hosting,
relationship and communication IDs share the Component's complete uniqueness
domain. The hosting fingerprint contains its role and full typed UI target;
label, doc and list position are excluded.

| Operation | Required outcome |
| --- | --- |
| Create | Assign a fresh owner UID and omitted UUIDv7 occurrence IDs. Valid supplied IDs are accepted only under existing new-owner rules; duplicates across owner roles reject. |
| Existing-owner edit | Preserve active IDs with unchanged fingerprints. Reject unknown supplied IDs, identity stripping in changed content and a retarget retaining its old ID. New occurrences omit IDs. |
| Wording or reorder | Preserve IDs, including equal-target siblings. A selector identifies a mutable assertion; checked wording additionally needs its owner etag. |
| Remove/re-add | Removed selectors stop resolving; later additions receive fresh IDs. Endpoint or label equality does not revive identity. |
| Exact-content id-less reapply | Preserve stored owner UID, active IDs and etag under the existing no-op exception. This is same-store reapply, not portable rebinding. |
| Preview/apply | Preview IDs remain unreserved. Recheck original request guards and final-view admission at publication. |
| Clone, new version, owner recreation | New owner incarnation scopes its IDs. Old hosting ConnectionRefs do not follow a replacement owner UID. |
| Branch, land and conflict resolution | Preserve active same-incarnation identities against recorded base and both sides. Apply existing deleted-ID revival and explicit restored-ID renewal rules. |

Changed-owner admission validates the complete prepared entity and readable UI
target against the guarded final batch/branch view, including tombstones.
Hosting does not require a slice or interaction path to be supplied; inspection
is separate from authoring this assertion.
Existing unchanged dangling owners remain inspectable without vetoing unrelated
writes. No partial owner, batch or landing publication is permitted.

Extend outbound/incoming reference extraction for `ui_hostings[i].ui`, with
existing Reference owner, owner UID, connection ID, diagnostic field path and
exact target. Preserve parallel occurrences and multiplicity. Safe deletion
uses the independent complete incoming guard, including hidden dependencies;
reporting caps cannot authorize deletion. Force deletion leaves assertions
unresolved without cascading. Exact UI retarget renews affected hosting IDs.
Only Component-to-UI references are stored by hosting. An inspection request or
externally retained receipt creates no Component-to-Persona, Command or ReadModel
reference and adds no deletion guard for those selections.

Extend the closed responsibility and CatalogSubject connection inventories for
Component UI-hosting occurrences. Their primary context is the declared UI;
endpoint availability is checked separately. Existing exact hosting and slice
edge subjects remain separate. Selecting them together in a Decision, Document
or responsibility list does not create or certify a composite interaction
subject. No new composite subject ID or automatic assignment inheritance is
selected. Existing reference and impact results are not a human-path inspector.

## Checked interaction request

Read-only `InspectHumanInteraction` requires a hosting ConnectionRef
and exactly one typed path alternative:

| Alternative | Required full ConnectionRefs and roles |
| --- | --- |
| Human issuance | A CommandSlice's `persona`, `ui` and `command` occurrences. |
| Human display | A UiSlice's `persona`, `ui` and one selected `source_read_models` occurrence. |

Every path reference must identify the same exact slice kind/key/version and
owner UID. Resolve each supplied ID against that owner's complete persisted
inventory and require its named role. Matching endpoint keys, labels or indexes
do not substitute for an occurrence. Generic SliceRef, a naked PersonaRef, an
AutomationSlice or a partial tuple cannot stand in for these alternatives.
The hosting selector must resolve specifically to Component `ui_hostings`.
The UI selected by the slice must equal the hosting UI's exact key/version.
The selected Persona and Command or ReadModel endpoints must also resolve.
The operation checks one supplied tuple; it introduces no batch, pagination or
implicit expansion to other paths.

The wire API is [human_interactions.proto](../../proto/trogonatlas/api/eventmodel/v1alpha1/human_interactions.proto).
`InspectHumanInteractionRequest.hosting` is required. Its `path` oneof contains
`issuance` (`persona`, `ui`, `command`) or `display` (`persona`, `ui`,
`source_read_model`). Every field is a full ConnectionRef.

The response echoes `request`, reports `CatalogResolutionState`, and returns
role-specific `edges` only when the complete supplied tuple matches. Each edge
retains its selection, diagnostic field, authored target, resolution state and
optional checked endpoint receipt. `scope.visited` contains readable checked
rows with actual UIDs and etags. `consistency` is explicitly
`PER_OWNER_CHECKED_READ_SET`; the scope revision is an opaque digest of reported
inputs, not a snapshot guarantee.

The [existing fingerprints](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs)
do not make a Persona edge a whole-path selector: CommandSlice persona anchors
its UI, while its UI edge anchors persona and command. UiSlice persona and
source ReadModel edges anchor UI; its UI edge anchors persona. Preserve these
rules and check the complete tuple. Source ReadModel reorder preserves selected
IDs. Equal ReadModel targets remain individually selectable; inspection of one
never proves that every observation or display path was supplied.

This request is not stored as a native interaction declaration. Callers can
retain the full request and response as a receipt, but the backend introduces
no receipt registry, retention promise, portable rebinding or historical query.
There is no automatic Persona-to-Component discovery. The current
[architecture scan](../../rsworkspace/crates/trogon-atlas-server/src/architecture_admission.rs)
does not enumerate slices, UI or Persona; reusing it cannot prove interaction
completeness.

## Resolution, checked scope and evidence

| Boundary | Required response behavior |
| --- | --- |
| Resolved | Return the unchanged hosting/path selectors, role-specific checked model path and readable endpoint references. This establishes resolution only in the reported checked read set. |
| Unresolved selection | For a readable owner, stale owner UID, removed ID, role mismatch, missing companion edge or UI mismatch cannot become a match to a sibling or replacement occurrence. Report generic unresolved state without a fabricated path. Malformed request shape rejects as INVALID_ARGUMENT. |
| Unavailable endpoint | Missing and hidden endpoints share generic unavailable state. Retain readable authored selectors without disclosing hidden titles, UIDs, etags, counts or existence. |
| Unreadable containing owner | Use existing generic NOT_FOUND semantics before exposing its inventory. A query must not reveal whether an unreadable Component or slice exists. |
| Operational failure | Store/decode, timeout and authorization-service failures remain RPC errors. They cannot become unresolved, unavailable, empty or a successful checked path. |

Return checked row receipts for readable Component, slice and endpoints: exact
reference, actual owner UID and etag, with branch and authorization/visibility
scope context. Return an explicit per-owner checked-read-set consistency
boundary. Resolve all roles from one checked version of each participating owner
within the request; never splice versions of the same owner. Different owners
may be checked at different instants. Reads are not promised to describe one
atomic multi-owner snapshot;
a scope digest identifies reported inputs, not atomicity, unchanged current
state, historical truth or runtime execution. Concurrent mutation can make a
receipt stale immediately. Consumers must refresh rather than merge current
wording or targets into an old receipt. Native writes still require their own
current guards and final-view validation; this read-only receipt authorizes none.

UIRef, PersonaRef, CommandRef and ReadModelRef select mutable exact keys and
versions, not endpoint incarnations. Force delete/recreate at the same key may
resolve to the replacement endpoint; report its actual UID/etag. Hosting and
slice ConnectionRefs pin their containing owners' UIDs and occurrence IDs,
not the UIDs of all referenced endpoints. No stronger historical binding is
implied. A richer endpoint-incarnation selection would require another contract.

## Authorization, bounds and Source isolation

Hosting writes require Component write authority and authorized reference access
to UI, not writes to Persona or slice owners. Inspection requires authorized
reads of its containing owners and safe hydration of selected endpoints. A
Reader may inspect but cannot author hosting. Persona roles, catalog assignments
and hosting labels grant no authorization or principal identity.
Missing, deleted or foreign branches cannot fall back silently to the baseline
or another branch view.

At most 1,024 hostings per Component and 1 MiB for the entire prepared protobuf
owner apply, including assigned IDs and all existing collections. Lower limits
win; rejection never truncates. Extend existing shared 5,000 resource-row,
10,000 examined identified-occurrence and 4,096 emitted-record ceilings wherever
those surfaces apply. Charge nested complete Component/slice inventories once
per exact owner/view, including mixed occurrence roles; independent internal
pages or members do not reset a request's budget. Exact authorized lookup may
bypass display projection caps, never identity inventory, size or work bounds.
Budget exhaustion is RESOURCE_EXHAUSTED, not a successful incomplete path.
The new inspection explicitly charges each participating Component, slice, UI,
Persona, Command or ReadModel row once in its shared row budget. Existing
architecture resource classifiers do not include every such kind; their scan
or accounting helpers cannot be assumed to cover this query automatically.
Plain endpoint hydration charges rows, not unexamined contained occurrences
such as UI transitions. Complete identity-inventory charging applies to the
selected ConnectionRef containing owners, not every hydrated endpoint.

Hosting authoring is native-only. Extend Source's closed selection/ownership
inventories to reject hosting occurrences and collection ownership. Reject
Source creates carrying hostings under every ownership policy, including
TITLE-only: initial create copies incoming content. Reject WholeOwner writes
when either current or incoming hostings are nonempty, including attempts to
clear them. Supported unrelated field-specific updates preserve local hostings.
All such rejection is atomic, without native or mapping-base publication.
Remote connectors and portable selector rebinding remain deferred.

CLI/MCP authoring and exact inspection must preserve full selectors, uint64
versions, order and presence. Studio is a read-only review surface, using the
returned checked scope and displaying unavailable/stale state honestly. Loaded
model correlation is not this backend check; mocked gateways and component
fixtures are not native persistence or authenticated deployment evidence.
