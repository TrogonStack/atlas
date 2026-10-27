# System and service relationship contracts

Status: native relationship support is implemented in the schema, core, backend,
CLI/MCP authoring and Studio inspection. The
[infrastructure schema](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto)
is the wire contract. [Acceptance requirements and executable coverage](system-relationship-acceptance.md)
distinguish service, client and component fixtures from deployment/runtime evidence.

## Scope and existing representations

| Current representation | Evidence and boundary |
| --- | --- |
| Component | [Infrastructure schema](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto): a living deployable implementation unit, with technology and curated `members: repeated EntityRef`. Membership documents implementation intent and does not change behavioral validation. |
| Exact members | [Identity schema](../../proto/trogonatlas/eventmodel/v1alpha1/identity.proto) and [reference extraction/retargeting](../../rsworkspace/crates/trogon-atlas-core/src/refs.rs): members can select any known entity kind, including Component and ExternalSystem. A diagnostic `members[i]` path has no persisted occurrence ID. |
| ExternalSystem | Infrastructure schema: a living third-party boundary; no members field. Its existing inbound/outbound integration links include ReadModel ExternalSource and Processor calls in [moments](../../proto/trogonatlas/eventmodel/v1alpha1/moments.proto). |
| Human behavior | [Slices](../../proto/trogonatlas/eventmodel/v1alpha1/slices.proto): `CommandSlice.persona` field 4, `ui` field 5 and `command` field 6; `UiSlice.persona` field 4, `source_read_models` field 5 and `ui` field 6. UI itself has transitions and slot composition, no persona field. |
| Persona | Moments schema: always human; `role` is a descriptive string. It is neither a directory Person nor an authenticated principal. Automation uses Processor/AutomationSlice. |
| Existing API/data paths | [Specification](specification-and-api-contracts.md), [Channel and DataStore](channel-and-datastore-contracts.md) and [DataProduct](data-product-contracts.md) declarations have their own exact selectors and evidence boundaries. |

Use Component only where the deployable implementation meaning holds. A service
that is a deployable unit can use Component; a third-party boundary can use
ExternalSystem. Existing Component membership may curate hosted model elements
and other implementation boundaries, with shared exact members remaining shared.
It does not establish exclusive ownership, runtime hosting, arbitrary abstract
system containment, a business hierarchy or transitive membership. A reference
to an ExternalSystem does not mean the Component deploys that third party.
This layer does not change membership shape, admissible kinds, ordering,
duplicate handling or existing validation. It adds no membership ConnectionRef.

| Boundary | Disposition |
| --- | --- |
| Labeled directed declarations between implementation/external boundaries | Selected below, using existing owners and typed references. |
| Abstract systems above deployables, independent system/service levels, authored strategic nesting | Deferred. Existing source establishes implementation and strategic meanings but no defensible shared taxonomy for these additional levels. No System, Service or Actor entity kind is introduced. |
| External-system-owned membership | Deferred. ExternalSystem has no members field. A relationship labeled “contains” does not introduce membership; a Component proxy must not be invented for an external abstract group. |
| System-level human interactions | Deferred. Existing slice occurrences can ground human behavior, but raw membership cannot identify which hosting assertion supports an interaction. A new aggregation/interaction selector and its lifecycle need a separate contract. |
| General send/receive, conditional triggers and agent/tool communication | The separate native [communication intent contract](communication-intent-contracts.md) supports owner-local message declarations and condition prose, not executable triggers. Agent/tool communication and behavioral binding remain deferred; existing typed API/observation paths retain their own meanings. |

## Wire shape

Component carries `relationships` at field 8 and ExternalSystem at field 7.
Both are repeated
`ArchitecturalRelationship` values. Existing fields retain their meanings.

| Message/field | Type and value contract |
| --- | --- |
| `ArchitecturalRelationship.connection_id = 1` | Existing `ConnectionId`; required in persisted state, omitted for assignment as described below. |
| `ArchitecturalRelationship.target = 2` | Required `ArchitecturalEndpoint`. |
| `ArchitecturalRelationship.label = 3` | Required existing `CatalogTitle`, with nonblank UTF-8 `value`, at most 256 encoded bytes under its existing validation. Human-readable wording, no controlled runtime semantics. |
| `ArchitecturalRelationship.doc = 4` | Optional existing `CatalogText`, nonblank if present, at most 16 KiB encoded under its existing validation. Absence means no elaboration. |
| `ArchitecturalEndpoint.component = 1` | Oneof selection, existing `ComponentRef` wrapping exact native Id. Currently declared in [reliability.proto](../../proto/trogonatlas/eventmodel/v1alpha1/reliability.proto); reuse or relocate that definition, do not duplicate it. |
| `ArchitecturalEndpoint.external_system = 2` | Alternative in the same oneof, existing `ExternalSystemRef`. |

The containing exact Component or ExternalSystem is both the declaration owner
and implicit source endpoint. Direction is always source to target. There is no
separate owner ref, source field, bidirectional flag, inverse list or relation
entity kind. Either owner kind can target either endpoint kind. An inbound
claim is authored on its source, requiring write access there. Cross-namespace
reference permission alone cannot authorize it. A reverse arrow requires its
own declaration on the other source.

Endpoints require namespace, slug and positive numeric breaking-change version;
missing, unknown or multiple alternatives reject, as do latest/family/source
handles and self-targets with identical kind/key/version. Distinct versions
are distinct endpoints; descriptive cycles between distinct endpoints are legal.
There is no transitive dependency, deployment, execution or authorization rule.

Repeated equal target/label/doc values are permitted as separate occurrences.
Their ConnectionIds distinguish them; endpoint pairs and labels never deduplicate
or identify occurrences. Preserve list order for native export and display,
without interpreting it as causal order. An empty collection means no authored
outbound declarations in that checked owner, not no operational dependencies.
No per-relationship status, protocol, message, provider, behavior or metadata bag
is introduced. Decision/Document subjects can carry rationale and provenance.

## Identity, versions and branches

Reuse [connection identity](connection-identity.md), the
[connection inventory/resolver](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs)
and [persistence lifecycle helpers](../../rsworkspace/crates/trogon-atlas-core/src/connection_persistence.rs).
Adding protobuf fields alone does not integrate their semantic fingerprints,
resolved targets or graph discovery; these are explicit implementation obligations.

The fingerprint is relationship role plus full typed target. Source identity is
the containing exact owner/incarnation. Label, doc and order are descriptive and
excluded from the fingerprint; changing wording preserves identity. A target
kind/key/version change requires a fresh ID. All persisted connection occurrences
share one uniqueness domain within the containing owner incarnation.

| Operation | Required behavior |
| --- | --- |
| Create | Fresh server-owned UID. Omitted IDs receive UUIDv7 values; valid supplied IDs are allowed on a new owner only. Reject duplicate IDs across the entire owner and malformed/noncanonical/nil UUIDs. |
| Edit/add/reorder | Preserve unchanged IDs. Additions to existing owners omit IDs; reject unknown supplied IDs, stripping retained IDs in changed content and target changes retaining an ID. |
| Preview/retry | Preview IDs are not reserved and cannot be supplied as active IDs on apply. Preserve original request digest/idempotency semantics through assignment. |
| Remove/re-add | Removed IDs stop resolving. Re-add with fresh IDs; indistinguishable same-target replacement requires separate guarded remove/add writes. No endpoint/index matching or deletion ledger is introduced. |
| Exact-content id-less reapply | Existing whole-content no-op exception preserves stored UID/IDs. It does not permit edited content to strip identities. |
| Copy/new version | New owner incarnation; local IDs may be retained in their new scope. Incoming selectors remain on the original owner. Targets remain exact unless explicitly changed. |
| Branch edit/land/rebase | Preserve incarnation and active IDs; compare recorded base, baseline and branch. Ordinary landing refuses revival of an ID deleted on the opposing same-incarnation owner. |
| Explicit conflict resolution | Keep the selected incarnation; renew only restored IDs removed by the opposing same-incarnation owner. Whole-owner deletion uses existing explicit keep-side restoration behavior. Independent same-key creations with different UIDs require conflict resolution. |
| Delete/recreate | Old ConnectionRefs fail against the replacement UID. Plain endpoint refs follow the exact mutable key, including same-key recreation. |

A ConnectionRef is existing `owner: EntityRef`, `owner_uid` and `id`.
Resolution checks exact owner kind/key/version, UID, complete stored identity
inventory and the selected relationship. A readable stored declaration may
remain identifiable when its target has become unavailable; target resolution
is reported separately. No selector freezes mutable label/doc, endpoint content
or behavior. Historical findings require checked content and revision/etag,
branch and authorization context in addition to IDs.

## Subjects, references and native operations

Extend the closed responsibility and CatalogSubject connection inventories to
allow **only** `relationships[]` on Component and ExternalSystem. Reuse existing
ConnectionRef arms in [ResponsibilitySubject](../../proto/trogonatlas/eventmodel/v1alpha1/responsibility.proto)
and [CatalogSubject](../../proto/trogonatlas/eventmodel/v1alpha1/decision_document.proto).
There is no new selector union arm. Current
[responsibility admission](../../rsworkspace/crates/trogon-atlas-core/src/responsibility.rs)
excludes these owner kinds; implementation must extend validation, resolution,
authorized endpoint context and direct/reverse query discovery together. A raw
Component member, ancestry ref or arbitrary path remains an unsupported connection
subject even if its owner kind is admitted.

Team/Person can assign responsibility to that exact declaration. Decision and
Document can select it as a subject. Existing entity subjects for Component and
ExternalSystem remain unchanged. Assignment or rationale on the owner does not
spread to declarations, endpoints or members; assignment on a declaration does
not spread to an equal-endpoint sibling. Selecting a stored declaration asserts
neither valid endpoint hydration nor executed behavior. Target availability is
reported independently, with ordinary edited-owner reference admission retained.

| Surface | Required extension |
| --- | --- |
| Native authoring | Exact reads, guarded put/batch, validation, diff, branch operations, preview, native manifest/export and snapshot recovery preserve the fields and returned identities. Relationships were introduced at revision 16; current mutation floors are 17 with communication intent support. Older writers are refused before they can drop retained fields. |
| Outgoing/incoming references | Add explicit extraction for `relationships[i].target.component` or `.external_system`, and return existing [Reference](../../proto/trogonatlas/api/eventmodel/v1alpha1/graph.proto) fields `from`, `from_uid`, `connection_id`, `from_field`, `to`. Preserve parallel occurrences and multiplicity. A path is diagnostic only. |
| Subject discovery | ListSubjectAssignments, ListSubjectDecisions and ListSubjectDocuments match full ConnectionRef, preserving owner UID, occurrence ID and unchanged selector. Existing namespace visibility/budgets apply. New shapes must be explicitly included in scan/index provenance enrichment, not assumed indexed from protobuf reflection. |
| Impact | Include declared target references with full occurrence provenance on returned paths. Existing owner-level reachability/one-path-per-owner behavior is not exhaustive parallel-path enumeration or a runtime dependency classifier. |
| Retarget | Exact guarded entity-key retarget updates supported plain targets and renews affected relationship IDs. Existing ConnectionRefs selecting old declarations do not follow them; replacing these selectors requires explicit new selections. |
| Safe/force deletion | Complete incoming-reference guard sees relation targets and typed subject owners. Safe delete protects them even with hidden referrers. Force delete leaves incoming declarations/selectors unresolved without cascading or replacing them. Local occurrence removal follows existing local-removal semantics and can leave subjects unresolved. |

GetEntity and existing reference/subject queries suffice for direct inspection;
no relationship traversal RPC or new derived hierarchy is selected. Fetch the
readable source at its checked etag to obtain labels; Reference does not carry
a fabricated relationship payload. An etag change requires a refreshed read,
not merging old provenance with current wording.

Source integration is a separate boundary. Its existing
[SourceSelection and field enum](../../proto/trogonatlas/api/eventmodel/v1alpha1/source_integration.proto)
include ConnectionRef and RELATIONSHIPS for supported current uses; this does
not authorize these new occurrences automatically. This first release is native
only: source selection/adoption of architectural occurrences and source ownership
of their `relationships` field are unsupported and reject atomically. Source
whole-owner writes involving a nonempty current or proposed architectural
relationship collection must also reject, preventing silent deletion.
Source creates containing architectural relationships must reject regardless of
ownership policy, including title-only ownership: the
[create path](../../rsworkspace/crates/trogon-atlas-server/src/source_integration.rs)
copies the incoming entity, so field-specific ownership alone is not a filter
on initial content. Existing-owner supported field-specific writes to unrelated
fields preserve the local collection.
Remote connectors, format compilers, source selector rebinding and portable
migration of incarnation-bound subjects remain deferred. Native export/reapply
in the same store is not proof of portable migration.

## Admission, visibility and bounds

The following rules govern this native slice. Backend authority,
existing namespace access, write guards, metadata and entity size/RPC limits
remain applicable. No catalog label or assignment grants security authority.

| Boundary | Requirement |
| --- | --- |
| Changed owner | Validate complete resulting owner and exact targets in the authorized merged branch/final batch overlay, including tombstones. Target must exist and be readable/referenceable. Unchanged dangling owners remain inspectable and do not veto unrelated mutations. |
| Batch/branch concurrency | Validate and publish under existing guarded commit/writer coordination. Recheck relevant view/preconditions at publication; stale preview/read must fail or be revalidated against the guarded final graph. No partial owner, batch or landing publication. |
| Resource ceilings | At most 1,024 relationships per owner; entire protobuf entity at most 1 MiB. Lower existing limits win. Cap rejection never truncates a collection. |
| Discovery work | Extend the existing shared 5,000 visible resource-row ceiling to include Component and ExternalSystem, counted once per exact owner/view. Share 10,000 examined identified occurrences across nested subject work and 4,096 emitted records per request; repeated equal endpoints consume work. Internal scan pages and batch members do not reset a request's budgets. Independent requests have independent counters; a continuation token cannot bypass the complete-scan ceiling required by that request. |
| Exact hydration | Bypass display/discovery projection caps for exact authorized owner lookup, retaining entity/RPC limits. Validate the complete persisted identity inventory before occurrence selection. |
| Operational failure | Discovery/admission budget exhaustion is RESOURCE_EXHAUSTED. Backend, decode, timeout and authorization-service failures retain operational semantics; they cannot become a verified empty result or model absence finding. |
| Visibility | Source write plus authorized reference access to target required. Hidden direct reads preserve NOT_FOUND; Reader mutations return PERMISSION_DENIED. A query for an unreadable subject fails without disclosing existence. Readable authored selectors remain visible, without hidden target hydration. |
| Query hydration | For callers lacking target visibility, missing and hidden targets share generic unavailable status. Never reveal hidden titles, namespace inventory, referrer paths, counts or existence. Authorized stale UID/removed-ID diagnostics can be specific where safe. |
| Deletion safety | Use independent complete incoming-reference guard across namespaces and final requested deletions, with bounded concurrency/timeout. Reporting caps cannot authorize deletion. Operational/incomplete guard refuses deletion. Preserve existing `referrers_incomplete` reporting behavior and generic hidden-dependency protection. |

Typed discovery overflow fails with RESOURCE_EXHAUSTED. Existing bounded
impact/export/report surfaces may return explicitly incomplete output under
their existing contracts; clients must preserve incompleteness. A clipped export
is not a complete reapply artifact. A successful native write is evidence of
accepted declarations only. Authored grouping, architectural declaration,
checked behavioral paths and observed runtime behavior remain distinct layers.

## Deferred human interaction contract

A future system-level Persona view must ground issuance in the exact
CommandSlice persona/UI/command edges, and display in exact UiSlice persona/UI
and selected source ReadModelEdge occurrences. These paths are present in
[slice reference extraction](../../rsworkspace/crates/trogon-atlas-core/src/refs.rs)
and [connection context fingerprints](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs).
Repeated equal ReadModel edges must remain distinct; repeated storyboard moments
must retain authored order and recurrence without invented moment IDs.

Component membership alone cannot choose an interaction occurrence or prove
hosting, a human's permissions or runtime execution. This layer adds no
Persona-to-system link, interaction message, inferred actor or behavioral query.
The absent aggregation/hosting occurrence contract is the reason for deferral;
existing exact slice inspection remains available. UI navigation and Screen
slot composition do not supply a substitute Persona relation.
