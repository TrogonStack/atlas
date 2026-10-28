# Catalog context contracts

Status: native extensions are implemented in the current checkout; complete
acceptance remains open. Support below is reconciled against checked-in schemas
and contracts, not historical checklists.
This reference does not close the master coverage checklist. No importer,
migration, provider synchronization or federation is proposed.

## Disposition and evidence

| Concept | Disposition | Evidence and remaining boundary |
| --- | --- | --- |
| ADR status, rationale, options and replacement | Supported | [Decision schema](../../proto/trogonatlas/eventmodel/v1alpha1/decision_document.proto) and [contract](decision-and-document-contracts.md). PROPOSED, ACCEPTED, REJECTED, WITHDRAWN, SUPERSEDED are native states. Version ancestry is distinct from substantive replacement. |
| ADR dates, decision makers and amendments | Implemented | Decision retains authored/decided dates, exact Person/Team makers and AMENDS occurrences. Responsibility remains a current catalog assignment rather than historical participation. |
| Vocabulary entry identity and definition | Supported | [Term and Ambiguity](../../proto/trogonatlas/eventmodel/v1alpha1/strategic.proto): exact Term Id, title, doc definition, embodied_by and version ancestry; namespace denotes bounded context. Ambiguity records synonym/homonym rulings. |
| Explicit vocabulary owning domain | Implemented | Term has optional exact `owning_domain`; it remains independent of the bounded-context namespace. No separate dictionary-entry entity is needed. |
| Resource docs, pages, anchors and navigation | Supported | Document subjects, retained text or inert HTTPS link, anchors and navigation in the Decision/Document contract. Page filesystem ancestry and sidebar presentation fidelity are excluded non-goals. Native navigation order already carries authored link order. |
| Categories, attachments and authored changelog | Implemented, client acceptance incomplete | Document collection annotations, purpose, retained bytes and role-qualified related content preserve these declarations. Safe Studio presentation/download and remaining lifecycle/pagination acceptance are open. |
| Diagram and example raw text | Implemented retention | Document retains exact bytes, media type, digest and optional filename. Specification artifacts keep their separate contract-source semantics. Retention does not validate a diagram format or execute an example. |
| Diagram/example associations | Implemented | `related_content` distinguishes EXAMPLE, DIAGRAM, ATTACHMENT and CHANGELOG through exact Document selections. Subjects separately select exact native resources or Specification members. |
| Metadata and ownership | Supported within existing semantics | [Registered annotations](../../proto/trogonatlas/eventmodel/v1alpha1/annotations.proto) and [responsibility](responsibility-contracts.md). Arbitrary metadata payloads are not references. Directory Person/Team records are not authenticated identities or approval permissions. |
| Source layout and EventCatalog fidelity | Excluded | Original filesystem hierarchy, parser conversion, visual layout fidelity and provider/source synchronization are outside the requested native scope. Native raw content and associations remain in scope. |
| Diagram topology | Separate dependency | Typed nodes/edges and flow topology remain genuine gaps in eventcatalog-coverage.md. Retaining content does not implement topology or execution. |

## Decision additions

Names here are design names, not allocated protobuf fields or RPCs.
`AuthoredDate` is an ISO Gregorian calendar date YYYY-MM-DD with a valid day;
it has no timezone or implied midnight instant. Optional `authored_on` and
`decided_on` preserve absence; decided_on cannot precede authored_on when both
exist. Future dates remain authored assertions. They never replace server audit
timestamps. Status does not fabricate or require dates, preserving existing
valid Decisions. No workflow approval is inferred.

`decision_makers` contains at most 128 identified associations, each with
ConnectionId, exact PersonRef or TeamRef and optional CatalogText participation
prose. Duplicate full assignee alternatives reject. A Team declaration does not
expand current membership into historical makers. This collection is independent
of responsibility assignments; changing either never changes the other.

`amends` contains at most 128 identified exact DecisionRef associations with
optional CatalogText rationale. Direction is from amendment to amended decision.
Self-links and AMENDS cycles reject in the final authorized graph. AMENDS does
not change either status, selected option, version ancestry or replacement.
A decision cannot both amend and replace the same exact target: a target whose
replaced_by points to the amendment conflicts with that amendment's amends edge.
Check this constraint on edits to either side, including a batch. RELATED may
coexist and retains its weaker meaning. Existing replacement termination and
status invariants remain unchanged. Mixed AMENDS/RELATED/replacement cycles do
not imply execution; only each explicitly constrained graph must be acyclic.

Unknown source status labels are not accepted native enums or guessed mappings.
They may be quoted in explanatory text but cannot establish native disposition.

## Vocabulary ownership

Add optional `Term.owning_domain: DomainRef`, an exact positive-version native
reference. Absence means undeclared ownership; it does not derive from namespace.
The existing Term key remains entry identity and doc remains definition. Different
Term versions or namespaces remain different entries even with equal titles.
Owning domain grants neither authorization nor inherited assignments. No new
DomainRef is fabricated from a dictionary name. Bounded-context scope and domain
ownership can coexist; this declaration does not change strategic realization
rules or enforce a new namespace hierarchy. A Term without doc remains legal
under existing admission; completeness of its definition is a separate finding.

## Document classification and content

Add proto3 `optional purpose` enum GENERAL, EXAMPLE, DIAGRAM, ATTACHMENT, CHANGELOG. Preserve absent separately from present GENERAL.
Absence retains current generic Document semantics. Purpose is descriptive,
not a renderer, payload validator or execution claim. Existing typed subjects
provide exact resource/member associations. Do not widen CatalogSubject to
arbitrary paths or infer nested message associations from filenames.

Add optional `filename: LogicalFilename`, nonblank UTF-8 up to 256 bytes,
without slash, backslash, control characters, dot or dot-dot as complete names.
It is a display/download name, never a path or identity. Existing doc holds usage
prose; body holds raw content, keeping commentary distinct from example bytes.
For text fixtures with nested associations, use the exact member subject plus
an explicit Document association; raw text never becomes a typed payload.

Add retained-body alternative `RetainedContent` with required MediaType,
exact `bytes`, and verified ContentDigest. Introduce a shared byte-oriented digest validator; the existing Specification string digest helper cannot validate arbitrary binary content. Content must be nonempty, at most
256 KiB; media type is a syntactically valid lowercase type/subtype without
parameters, at most 127 ASCII bytes. Enforce this narrower retained-content profile explicitly; do not assume the existing MediaType wrapper has the same grammar. No decoding, newline conversion, fetch,
script execution or format validation. Binary bytes remain downloadable only;
clients use safe text presentation for recognized textual media and never render
HTML/SVG actively. Existing retained text stays unchanged. Anchors are allowed
only on existing retained text, avoiding invented byte offsets or binary anchors.
Raw text diagrams, JSON examples and binary attachments therefore have honest
retention semantics. Specification artifacts remain authoritative for contract
schema selection, not a general-purpose attachment store.

Add `DocumentCollectionAnnotation`, a registered typed annotation on Document,
with required CatalogTitle `category` and optional CatalogText `description`.
At most one per Document. It marks a navigation document as a category; existing
ordered navigation selects whole documents or anchors and supplies sidebar labels.
A category has no separate identity, inherited membership or recursive access.
Changelog documents use CHANGELOG purpose and authored prose; they do not derive
from revisions. Attachments/examples/diagrams/changelogs attach to a containing
Document through proposed `related_content` associations: ConnectionId, required
DocumentSelection, required role EXAMPLE/DIAGRAM/ATTACHMENT/CHANGELOG and optional
CatalogTitle label. Equal role/full-target duplicates and self-links reject;
other cycles are descriptive and permitted. Target purpose, if present, must
match role; a later purpose edit may leave an inspectable invalid incoming
assertion without cascading mutation. Repeated use under different roles remains
distinct. Category metadata does not contain references.

## Identity, admission and lifecycle

Reuse [connection identity](connection-identity.md) and existing Decision/Document
admission. All new associations join the owner's complete ConnectionId inventory.
Full role and target including pins determine identity; labels, prose, dates and
ordering do not. New additions omit IDs on existing owners; server returns active
UUIDv7 IDs. Target or role change requires fresh ID. Removal/re-addition never
revives an occurrence. Preview reserves no identity. Exact native refs include
kind and namespace/slug/positive numeric version, never latest or display names.
Document anchor and Specification member selections additionally require returned
owner UID and local ID, with digest presence preserved exactly.

Native guarded entity writes and batch preview/apply validate complete edited
owners in the final branch view. New dates and content obey ordinary etag guards.
Same-key recreation invalidates contained selections; whole refs follow the exact
key. Clone/new version gets fresh owner UID; local IDs may survive only in that
new identity scope. Branch merge preserves unchanged identities and never revives
opposing same-incarnation deletions. Explicit restoration renews affected local
IDs. Retargeting contained selectors requires explicit new selectors. New fields
join manifest round trips, diff, export, reference discovery, reverse lookup,
impact, complete safe-delete protection and force-delete unresolved diagnostics.
Unchanged dangling declarations remain inspectable; unrelated edits do not heal
or globally reject them. Editing their owner requires valid complete state.

## APIs, authorization and budgets

Extend native authoring and GetDecisionDetail/GetDocumentDetail outputs with the
new fields. Add `ListDecisionAmendments(exact DecisionRef, direction)` and
`ListTermDomainDeclarations(exact DomainRef)`; return declaring owner ref/UID,
association identity where present and original exact selector. Extend incoming
Document declarations with related_content role filters. Direct queries preserve
all distinct occurrences and do not recursively follow navigation. Binary detail
returns original bytes/media/digest only to authorized readers. No fetch API.

The new listing requests include page_size (default 100, maximum 500) and
opaque page_token; responses include next_page_token and incomplete. Order by
full declaring owner kind/key and local occurrence ID, with exact selectors
retained. Bind tokens to caller visibility, branch, query filter and snapshot
revision; stale or mismatched tokens reject rather than silently skip records.
No next token means the bounded snapshot scan exhausted, not whole-store
completeness when its snapshot is capped. Existing unpaginated queries are not
implicitly upgraded by this proposal.

Use current branch/scope, etags, completeness and resolution diagnostics.
Namespace write plus authorized read/reference access to all selected targets
and necessary native context is mandatory. Hidden direct reads return NOT_FOUND;
Reader mutations PERMISSION_DENIED; readable declarations hydrate hidden/missing
targets as UNAVAILABLE without titles, bytes, counts or existence leakage.
Operational failure is not a dangling reference or a verified empty result.

All extensions share existing limits: 1 MiB encoded entity; 2,048 identified
associations across owner, 128 related_content associations; 5,000 visible
resource rows across native discovery, 10,000 examined occurrences/anchors/hops,
and 4,096 emitted records per request. One request budget spans nested reads, batches and recursive calls. Each
separate pagination request has its own bounded budget; page tokens do not
claim a retained cross-request computation budget. Date fields and annotation add no independent scan.
Term-domain discovery counts Terms against the same shared row budget. Retained
binary content obeys the encoded entity ceiling, including transport expansion.
Write cycle/conflict checks share request work ceilings and fail atomically with
RESOURCE_EXHAUSTED if proof cannot complete. Lower Store/staged-write/RPC limits
prevail. Safe deletion uses the independent complete incoming guard, including
hidden referrers, never capped discovery output as proof. Timeouts and backend
failures retain operational errors and refuse mutation.

## Source isolation and implementation dependencies

[Source contracts](source-integration-contracts.md) remain a separate control
plane. Source pointer, snapshot, provenance or directory authority cannot assign
native IDs, overwrite local fields, grant permissions or create amendments.
No Source schema/profile changes, automatic adoption, remote fetching or original
file hierarchy fidelity are required here. New fields are native-only. Reject Source creates containing any of these new
extensions under every ownership policy, including TITLE-only: the create path
copies the whole incoming entity. Reject WholeOwner updates whenever current
or proposed new extensions are present, including clears. Reject new contained
selectors and field paths through the closed ownership inventory. Supported
unrelated field-specific updates preserve the local extensions unchanged.
Native content digests attest stored bytes, not original source recovery.

Implementation requires coordinated protobuf revision, core admission and
reference inventory, server guarded graph checks, typed queries, Store routing,
manifest/CLI/MCP round trips and Studio safe review. See the
[acceptance specification](catalog-context-acceptance.md). No schema allocation,
generation or executable test was performed for this proposal.

Open objections: confirm whether calendar dates suffice for authored decisions
or a separate precise instant is required; confirm public API naming and binary
transport support; review the cross-owner AMENDS/replacement conflict check cost;
confirm category-as-Document navigation meets native product needs. These require
explicit implementation decisions, not silent exclusions. Typed design topology
and richer nested example selection depend on separate topology/member contracts;
this proposal does not claim to supply them.
