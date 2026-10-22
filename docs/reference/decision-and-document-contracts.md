# Decision and Document contracts

Status: proposed native resource contracts. Decision and Document have no
implemented schema, admission, storage, queries or client surfaces. Names below
are conceptual fields and operations, not allocated protobuf tags, entity-kind
numbers or RPCs. The [acceptance reference](decision-and-document-acceptance.md)
specifies required evidence; existing fixtures do not prove this proposal works.

## Scope

A Decision records a catalog choice and its rationale against selected native
subjects. A Document retains authored text or an explicitly unverified external
link, with typed model associations and optional declared navigation anchors.
Neither resource creates commands, events, projections, execution order, approval
authority, provider exposure or implementation evidence. A decision's acceptance
is a catalog assertion, not proof that its selected behavior is implemented.

Existing entity concurrency, registered typed metadata, server-owned system
metadata, namespace authorization and greenfield version policy apply. There is
no source-overlay compatibility layer, external synchronization, remote fetch,
format compiler, executable content or filesystem access in this proposal.

## Shared fields and values

| Field | Proposed contract |
| --- | --- |
| `id` | Required existing Id: namespace, slug and exact positive breaking-change version. |
| `title` | Required `CatalogTitle`. |
| `doc` | Optional Markdown; empty and absent mean no documentation. |
| `metadata` | Existing registered typed annotations. Embedded prose and arbitrary payload values do not become references. |
| `supersedes` | Optional exact prior version of the same kind, namespace and slug; existing terminating single-parent version rules apply. |
| `subjects` | Identified `SubjectAssociation` collection, required nonempty on each resource. |

| Value | Validation |
| --- | --- |
| `CatalogTitle` | Nonblank UTF-8, at most 256 encoded bytes. No identity normalization. |
| `CatalogText` | Nonblank UTF-8, at most 16 KiB encoded bytes per value. |
| `LocalValueId` | Non-nil lowercase hyphenated UUID, unique across options or anchors within the owner incarnation; separate from ConnectionId. |
| `DecisionRef`, `DocumentRef` | Required exact native Id, without a UID/revision pin, consistent with existing typed entity refs. |
| `ExternalDocumentLink` | Absolute HTTPS URL, host required, no user information or control characters, at most 2,048 UTF-8 bytes. Stored inertly; reachability and content are unverified. |
| `AnchorFragment` | Case-sensitive ASCII `[A-Za-z][A-Za-z0-9._:-]*`, at most 128 bytes, without a leading `#`. |

Unknown enum values, missing required alternatives and present empty optional
records reject writes. Authored collection order is retained without execution
meaning. Nonblank means at least one non-whitespace character. Entire entities
are limited to 1 MiB encoded, subjects to 1,024 per owner and identified
associations to 2,048 across an owner. Lower entity, transport or storage limits
still apply. Exceeding any limit rejects the whole write, without truncation.

## Decision values and state

| Field | Proposed contract |
| --- | --- |
| `status` | Required PROPOSED, ACCEPTED, REJECTED, WITHDRAWN or SUPERSEDED. |
| `context` | Required CatalogText describing the question and constraints. |
| `rationale` | Required CatalogText explaining the current disposition. |
| `options` | Nonempty collection, at most 64 `DecisionOption` values. |
| `selected_option` | Optional local option ID, required exactly for ACCEPTED and SUPERSEDED. Must select a retained option. |
| `consequences` | Collection of at most 64 `DecisionConsequence` values; nonempty for ACCEPTED and SUPERSEDED. |
| `replaced_by` | Optional identified `DecisionReplacement`, required exactly for SUPERSEDED. |
| `related_decisions` | Identified RELATED associations to exact DecisionRefs, at most 128. |
| `documents` | Identified supporting Document selections, at most 128. |

A DecisionOption has required `option_id: LocalValueId`, `title: CatalogTitle`
and `description: CatalogText`. IDs persist through wording edits/reordering.
Removing an option makes its ID inactive; re-adding requires a fresh ID. A
DecisionConsequence has required `kind` BENEFIT, COST or RISK and `text:
CatalogText`; it is an ordered value, not an independently addressable occurrence.
Exact duplicate consequences reject; equal text with different kinds is allowed.

PROPOSED records deliberation; ACCEPTED records adoption of the selected option;
REJECTED declines the question without a selected option; WITHDRAWN closes it
without a decision. SUPERSEDED retains the previously adopted choice and records
its replacement. A new entity may start in any status satisfying these rules;
this contract imports assertions, not a historical approval workflow.

Guarded edits may change status freely while satisfying the entire resulting
state. Reopening an adopted decision removes `selected_option` and
`replaced_by` where forbidden; retained revision history records earlier state.
There are no inferred timestamps, signatories, votes or approval permissions.
An adopted choice may be edited at the same version under normal guards; the
status does not make the entity immutable or freeze its targets.

`DecisionReplacement` contains ConnectionId and required exact `decision:
DecisionRef`. Its target must be ACCEPTED or SUPERSEDED in the authorized final
view. Self-selection and replacement cycles reject, including cross-namespace
and cross-version cycles. Every new/edited replacement chain must terminate at
an ACCEPTED decision. Missing or hidden links and budget/operational failures
cannot establish termination. Retained chains can become unresolved after
force deletion or subsequent target edits; queries report that state separately.
Changing a target's status does not cascade changes into incoming decisions.

`supersedes` is version ancestry within one resource family. `replaced_by` is a
substantive replacement between decisions, including different slugs/namespaces;
it does not create a version or modify the replacing decision. Merely creating
a new version, setting version ancestry or adding RELATED never changes status,
creates replacement edges or transfers subjects. Document has no decision state
and no semantic replacement field; its `supersedes` remains version ancestry.
A related-decision association contains `connection_id`, required `decision:
DecisionRef` and optional `doc: CatalogText`. RELATED is directed, deduplicated
by exact target and rejects self-selection; descriptive cycles between different
decisions are allowed. A supporting-document association contains
`connection_id`, required `document: DocumentSelection` and optional `doc:
CatalogText`. Duplicate full DocumentSelections, including digest pin presence
and value, reject within a Decision. Queries do not recursively expand it by default.

## Document body, identity and navigation

| Field | Proposed contract |
| --- | --- |
| `body` | Required oneof `retained: RetainedDocumentText` or `link: ExternalDocumentLink`. |
| `anchors` | At most 256 declared `DocumentAnchor` values; allowed only with retained body. |
| `navigation` | At most 128 identified associations to Document selections, each with optional CatalogTitle label. |

RetainedDocumentText contains required `media_type`, `content` and `digest`.
Media type is exactly `text/plain` or `text/markdown`, without parameters.
Content is nonempty UTF-8, at most 256 KiB encoded. Digest reuses
[ContentDigest](specification-and-api-contracts.md#entity-and-value-rules): SHA-256,
64 lowercase hexadecimal digits over the exact bytes, without newline or Unicode
normalization. The server verifies the supplied digest; missing/mismatched
values reject. No binary codec, HTML renderer, attachment path or ContractFormat
is introduced. Document text is not a Specification artifact or a type contract.

Body edits are mutable entity edits; digest pins detect byte changes. Media type
changes are also allowed under normal guards and do not assert byte changes.
`DocumentAnchor` contains required `anchor_id: LocalValueId`, `fragment:
AnchorFragment` and `label: CatalogTitle`. Fragments are unique per owner. These
are authored navigation declarations, not parsed headings, byte offsets or proof
that a renderer exposes an HTML element. No parser derives or verifies them.
Label edits and reordering preserve IDs; changing a fragment requires a fresh
anchor ID. Changing content preserves retained anchor IDs as authored navigation
intent, while any old expected digest becomes stale. Switching to link body
requires removing all anchors. Removed anchor IDs cannot be revived.

A `DocumentSelection` is exactly one of a whole `DocumentRef` or a
`DocumentAnchorRef`. The latter contains required exact DocumentRef,
`owner_uid`, `anchor_id` and optional `expected_digest: ContentDigest`. It
resolves only against a retained body and the complete owner's anchor inventory;
a pin must match current exact bytes. UID and local ID follow the existing
non-nil UUID identity conventions. Whole selection permits link-only documents;
it carries no digest or anchor pin. Neither selection silently matches by title,
fragment or URL. Whole-document selection follows same-key recreation; anchor
selection cannot enter a recreated owner. A matching digest pins bytes, not
media type, rationale, navigation, subjects or implementation behavior.

Each navigation association contains `connection_id`, required `document:
DocumentSelection` and optional `label: CatalogTitle`. Navigation associations
are directed and unique by full DocumentSelection,
including digest presence/value. Self-navigation rejects; cycles between distinct
documents are allowed. Rendering a link requires an explicit user navigation;
admission, query, export and preview never fetch it. Stored Markdown is inert
content, rendered only through the client's safe text/Markdown boundary. URLs
inside prose create neither discovered references nor permission to fetch.

## Typed subjects and assignments

A SubjectAssociation contains persisted ConnectionId, required `subject:
CatalogSubject` and optional `doc: CatalogText`. No relation-kind discriminator
or transitive coverage is inferred. Duplicate complete subjects reject within
one owner, including optional pin presence/value in equality.

CatalogSubject is exactly one existing selector: EntityRef, ConnectionRef,
ContractMemberRef or DataPortRef. Its entity and connection inventories are the
closed [responsibility subject inventories](responsibility-contracts.md#subject-inventory-and-exact-resolution)
as of this proposal. Decision and Document are excluded as entity subjects;
their associations, anchors, options, Team membership and responsibility
associations are excluded as connection subjects. Separate typed Decision and
Document relationships above do not extend this inventory recursively.

| Selector | Required exact resolution |
| --- | --- |
| EntityRef | Exact kind, namespace, slug and version; unknown kinds reject. No latest, family or contained-member expansion. |
| ConnectionRef | Exact owner EntityRef, owner_uid and ConnectionId. Validate complete persisted identity inventory and the selected occurrence's native context. Repeated equal endpoints remain distinct occurrences. |
| ContractMemberRef | Exact SpecificationRef, owner_uid, member kind and member_id. ARTIFACT, SCHEMA, MESSAGE and OPERATION allowed; optional expected_digest only for ARTIFACT. A member is not ContractTypeRef or format/type validity proof. |
| DataPortRef | Exact DataProductRef, owner_uid, INPUT/OUTPUT direction and port_id. Validate the complete owner's port identities; no offered-data or lineage inference. |

Reuse native resolution, [connection identity](connection-identity.md),
[member selection](specification-and-api-contracts.md#members-and-exact-selections)
and [port identity](data-product-contracts.md#port-and-association-identity).
No list indices, arbitrary document spans, trace-step occurrences, screen slots,
metadata paths or filesystem paths become selectors. Storyboard slice lists and
trace order retain repeated occurrences but do not acquire invented IDs.

Plain entity refs follow mutable exact keys, including same-key recreation.
Incarnation/local selectors fail on recreation, removal, malformed/duplicate
identity or mismatched kind/direction. No selection freezes mappings or behavior.
A historical conclusion requires checked bytes, branch, revision/etag and
authorization context; current hydration is not historical evidence.

Team/Person responsibility entity subjects must explicitly admit the proposed
Decision and Document kinds at implementation. Existing assignment kinds,
assignee authority, direct/reverse queries and no-inheritance rules remain.
No new assignments live on Decision or Document, and no Person becomes an
authenticated approver. Responsibility subjects exclude these new associations,
options and anchors. Owner/member/port responsibility never implies
responsibility for a connected Decision or Document.

## Association lifecycle and graph mutation

ConnectionIds share one uniqueness domain across each Decision/Document owner
incarnation. A fingerprint comprises association role and full typed target,
including pins; documentation, display labels and list order are excluded.
Changing role, target or pin requires fresh association identity. All
association and anchor inventories are validated before local selection.

| Operation | Proposed requirement |
| --- | --- |
| Create/add | Omitted local/association IDs request server UUIDv7. Valid supplied IDs only on new-owner creation; existing-owner additions omit IDs. Persisted returned UID/IDs are authoritative. Preview reserves none. |
| Edit/reorder | Preserve unchanged IDs; reject unknown IDs, changed fingerprints retaining IDs and stripping retained identities. |
| Remove/re-add | New IDs required. Indistinguishable same-target remove/add requires separate guarded writes; never resurrect by endpoint or position. |
| Native unchanged reapply | Reuse existing exact-content id-less no-op exception; changed manifests cannot strip identities. |
| Clone/new version | Fresh owner UID, scoped local IDs may be retained. Exact external targets stay unchanged absent explicit mappings; incoming refs stay on originals. |
| Branch land/rebase | Preserve incarnation and unchanged IDs. Compare base, baseline and branch; ordinary merge cannot revive IDs deleted by an opposing same-incarnation owner. |
| Explicit conflict resolution | Preserve selected-side incarnation and renew restored local/association IDs deleted on the opposing same-incarnation owner. Whole-owner deletion uses existing explicit keep-side restoration rules. |
| Independent same-key creation | Distinct UIDs conflict; matching titles, URLs, subjects or content do not merge identities. |
| Retarget | Guarded exact entity-key retarget may update supported plain refs with fresh association IDs. Member/port/connection/anchor targets require explicit replacement selectors, not owner-key substitution. |
| Safe delete | Complete incoming-reference guard protects subjects, selected local owners, related/replacing decisions, supporting documents, navigation targets, ancestry and responsibility targets. |
| Force delete | Leave incoming declarations unresolved with unchanged exact selectors; no cascade, repair, reassignment or replacement lookup. |

Removing an anchor or other selected local occurrence may leave incoming refs
unresolved under existing local-removal semantics. Safe deletion of its containing
entity remains protected. Deleting a Decision/Document removes its outgoing
associations only, without deleting selected resources. Deletion and text edits
promise no purge of revisions, exports, branches, backups or historical copies.

New/edited owners validate the complete proposed state in the authorized merged
branch view, including batch overlays and tombstones. Unchanged dangling owners
remain inspectable and do not veto unrelated writes/landings. Editing such an
owner requires repair of every invalid selection on it. Resolution validates
selected identity and required native context, not recursively every unrelated
outgoing declaration on the selected entity. Batch preview/apply checks all
requested delete targets against the final graph with concurrency guards; stale
preflight cannot authorize publication. Create targets first and use returned
identities, or reference known same-incarnation targets in a guarded batch.

## Authorization and diagnostics

Writes require namespace write access and authorized read/reference access to
selected owners and the native context needed to resolve/disclose selections.
Cross-namespace reference access grants no write access. Resource readability
does not reveal hidden target titles, member names, content, contacts, anchors,
referrers, paths, counts or existence. Already authored selectors on a readable
owner remain visible without adding hidden hydration.

Direct hidden-target reads preserve NOT_FOUND; Reader mutations return
PERMISSION_DENIED. A query for an unreadable subject fails without disclosing its
existence. Hidden hydration of readable declarations uses a generic unavailable
state, indistinguishable from missing to that caller. Authorized exact missing
and stale-identity diagnostics may be more specific where existence is safe to
disclose. Diagnostics identify visible field/association identity, never hidden
lookup payloads. Hidden referrers still block deletion with a generic protected
dependency result. Totals and pagination describe authorized results only.

Invalid model writes reject atomically. Budget exhaustion is RESOURCE_EXHAUSTED;
timeouts, decoding, backend and authorization-service errors retain operational
failure semantics. Neither hidden, failed nor incomplete lookup is a verified
empty result. Validation must not convert operational failures into model errors.

## Proposed operations, queries and bounds

Implementation must integrate ordinary native entity authoring, manifests,
validation, branch operations, diff, preview, safe/force deletion, snapshot/export
and exact reads. No executable command/RPC spelling is allocated here.

| Conceptual query | Input and required result |
| --- | --- |
| Decisions/documents for subject | Exact CatalogSubject and optional typed Decision status filter. Return authorized owners, owner UID, association ID and unchanged full selector; no owner/endpoint/family expansion. |
| Decision detail | Exact DecisionRef. Return authored state, selected option and resolved/unresolved supporting/replacement/subject records; following replacement is explicit, bounded and cycle-aware. |
| Document detail | Exact DocumentRef or DocumentAnchorRef. Return authorized retained bytes/media/digest or unverified inert link and declared anchor metadata. |
| Incoming related/navigation/supporting declarations | Exact DecisionRef or DocumentSelection. Match full selector, including pin presence/value, without promoting anchor lookup to whole-document lookup. |
| Generic references/impact | Preserve containing from EntityRef/from_uid, field, ConnectionId, full typed declaration and selected owner as to. Owner-level impact finds nested selectors without replacing their provenance. |

Queries default to direct declarations, not recursive relationship expansion.
Explicit traversal uses visited typed identities and preserves every distinct
association occurrence, including multiple references to one behavioral endpoint.
Storyboard displays preserve authored causal order and recurrence. Typed detail
results carry branch, checked scope, owner etags/revision digest and completeness;
assembled reads are not a distributed snapshot. Link-only content remains
unverified even when lookup itself completes successfully.

Visible resource discovery extends the existing shared 5,000-row ceiling across
Channels, DataStores, Specifications, DataProducts, Teams, Persons, Decisions and
Documents. Count each exact kind/key/version once per selected view/request.
Examine at most 10,000 identified occurrences, anchors or replacement hops
combined and emit at most 4,096 records per request. Repeated equal endpoints
consume occurrence work; pagination, batch members and traversal loops never
reset budgets. Exact owner hydration bypasses display projection caps while
retaining entity/RPC limits. Replacement admission shares these work ceilings;
it fails atomically if chain termination cannot be established within them.

Typed discovery exceeding a ceiling returns RESOURCE_EXHAUSTED. Existing bounded
impact/closure/export surfaces may explicitly report incomplete/truncated
coverage; incomplete output proves neither absence nor deletion safety. Native
export must preserve retained bytes, media types, links, digest/pin presence,
local IDs and association order; a truncated export is not a complete reapply
artifact. Clients must retain incompleteness and structured failure metadata.

Safe delete uses the existing independent complete incoming-reference guard
across namespaces and all requested targets, with bounded concurrent reads and
a total timeout. Its safety decision does not use visible reporting caps. Any
incomplete guard or operational failure refuses deletion. Only visible resource
budget exhaustion permits the existing explicitly incomplete delete-report
fallback; it never authorizes deletion. No persistent reverse index is promised.
