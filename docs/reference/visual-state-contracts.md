# Visual state and capture contracts

Status: proposed native contract. No VisualState schema, capture API, gallery,
or visual-state flow target is implemented by this document. The
[acceptance requirements](visual-state-acceptance.md) describe the evidence
required before claiming connected support.

This semantic contract refines the merged
[visual product proposal](../explanation/visual-product-atlas.md) and its
[delivery roadmap](../explanation/visual-product-atlas-roadmap.md). It proposes
captured visual evidence connected to existing Screen, UI and DesignGraph
identities without adding another behavioral primitive or inferring commands,
events or execution from an image. Wire/API implementation remains separate.

## Current implementation and prerequisites

| Existing surface | Reuse and limit |
| --- | --- |
| [Screen and UI](../../proto/trogonatlas/eventmodel/v1alpha1/moments.proto) | Screen declares a route and composition slots; UI contributes behavioral interactions. Neither has native state or capture records. A catalog page is a Screen projection, not a new Page entity. |
| [ConnectionRef](../../proto/trogonatlas/eventmodel/v1alpha1/connections.proto) | Exact owner kind/version, owner incarnation UID and persisted connection ID identify an association. Endpoint pairs and diagnostic paths are insufficient. |
| [CatalogSubject and SubjectAssociation](../../proto/trogonatlas/eventmodel/v1alpha1/decision_document.proto) | Reuse typed entity, connection, member and port selections and identified associations where the existing inventory permits them. |
| [DesignGraph](../../proto/trogonatlas/eventmodel/v1alpha1/design_topology.proto) | Reuse authored steps, transition labels, subflows and persisted occurrence identities. Its target union currently has no VisualState alternative. |
| [Human interaction inspection](../../proto/trogonatlas/api/eventmodel/v1alpha1/human_interactions.proto) | Reuse complete checked issuance/display selections when documenting a human path. A selected connection alone does not establish the whole path. |
| [Studio Screens](../../tsworkspace/webapps/studio/src/components/canvas/ScreensBoard.tsx) | Existing Journey and Sitemap views are schematic projections, not captured imagery. The current Journey builder resolves loaded latest-by-slug data and suppresses repeated slice references; it cannot supply this contract's exact capture or flow occurrence projection. |
| [Artifact service](../how-to/manage-artifacts.md) | Main implements discovery, upload, stored-byte integrity verification, immutable identities and retrieval with NATS Object Store or S3-compatible storage. Typed model attachments, raster verification, branch/history reference accounting and Studio image delivery remain additional work. |

Artifact media declarations must not be described as image certification.
Visual capture admission requires new byte-level image checks even when an
artifact service accepts an upload with an image media type.

## VisualState

VisualState is a proposed top-level native entity with the ordinary exact
`Id`, server-owned incarnation UID, etag, branch lifecycle and version lineage.
No numerical Entity union tag, enum value or protobuf field number is reserved
here. Coordinated implementation must allocate them against the current schema.

| Proposed value | Rule |
| --- | --- |
| `id` | Required existing namespace, slug and exact nonzero breaking-change version. Route, title and image digest are not identity. |
| `title` | Required `CatalogTitle`. Examples include ready, submitting, empty basket and invalid password. These are authored names, not an exhaustive lifecycle enum. |
| `doc` | Optional `CatalogText`. |
| `context` | Optional typed selection of exactly one Screen or UI at an exact version, with its expected owner incarnation UID. When supplied, an empty or ambiguous selection rejects. Absence is explicit; a standalone connection-attached state needs no invented Screen, UI or route. |
| `subjects` | Identified `SubjectAssociation` collection selecting existing supported `CatalogSubject` alternatives. Preserve authored order and repeated equal selections with distinct association IDs. |
| `scenario` | Optional checked scenario selection described below. |
| `areas` | Explicit catalog grouping labels. A route prefix, namespace, role or bounded context does not implicitly create an area. |
| `metadata` | Existing registered typed annotations. |
| `supersedes` | Optional exact prior VisualState version under existing terminating lineage rules. |

A title, documentation or area edit preserves the state's identity. A new
behaviorally distinct visible condition is a separate VisualState. Viewport,
locale and theme normally vary the capture profile rather than create another
state. A role-dependent behavioral state may be distinct; its role label grants
no authorization.

This contract retains the proposal's optional context. A state without context
can document exact connection or scenario subjects and remains discoverable as
a standalone state. A draft with neither context nor subjects is visibly
Unassigned. Missing associations describe an incomplete model; the backend
does not infer a context from a title, route, image or selected endpoints.

The prepared entity is limited to 1 MiB encoded, including assigned IDs and
system metadata. The first contract allows at most 128 subject associations
and 32 area labels per state, with existing catalog text limits. Lower
transport and storage limits win. Reject excess input atomically; never clip
associations, synthesize an absent context or discard unknown union alternatives.

### Behavioral and scenario selections

Use the existing [connection identity](connection-identity.md) for an
interaction attachment. A state may associate with the exact CommandSlice UI
and Command edge occurrences, or another supported occurrence. A complete
human-issuance claim also retains the Persona and hosting selections checked by
human interaction inspection. A drawn UI-to-command connector may summarize
several occurrences; the drawing ID is not a replacement for those selections.

Different slices with equal UI and Command endpoints remain different subjects.
Ready, submitting, validation feedback, rejection and accepted-result states may
all document the same interaction without adding an Event or changing its
behavior. Context, subjects and scenarios must agree where they select the same
UI, slice or behavioral outcome. Contradictory supplied selections reject.

An optional scenario selection contains the exact owning slice, its incarnation
UID, scenario kind, existing scenario ID and a digest of the checked scenario
content. Scenario IDs are scoped to that owner; they are not connection IDs.
The backend must reject missing, duplicate or unsupported scenario selections.
Changing scenario content or removing and replacing it cannot make old capture
evidence current. No scenario reference is synthesized from a title, list index
or screenshot text.

Pure navigation keeps its existing UI navigation meaning. DesignGraph arrows
and authored subject associations do not establish an execution path, deployment
or runtime permission. Reference reads expose the same associations from state,
connection, context and containing flow inspection.

### Page grouping

Extend Screen with an optional typed `catalog_areas` collection, bounded to 32
catalog labels under the existing catalog text limits. This is a proposed
addition, not a field present on Screen today. It supplies page-level grouping
even when no VisualState or Storyboard references the Screen. Empty means
Unassigned, which remains a visible catalog group. Route, namespace and bounded
context never supply a fallback area.

Page-area filters use the Screen's authored labels. State-area filters use
VisualState `areas`, including standalone states. The request and response identify
which filter is applied; a state label does not silently assign its Screen to
that area. Reordering labels does not change entity identity. Their values
participate in ordinary entity guards, history and native manifest round trips.

## Capture records

Capture history is a backend-owned append-only record collection separate from
the mutable model entity. It must not make the entity grow with every capture.
Its subject is a closed typed selection bound to the exact owner's incarnation:
VisualState here, or an exact email variant under the separate
[email catalog contract](email-catalog-contracts.md). An email variant does not
require a dummy VisualState. The applicable subject contract defines its exact
selector and guards; unknown subject alternatives reject. A state may have no
evidence, and an imported design may be useful without being runtime evidence.

| Capture receipt value | Required meaning |
| --- | --- |
| Identity | Server-assigned immutable capture ID, authorized namespace and exact typed capture subject with its owner incarnation and etag checked for this attempt. VisualState selection includes its exact identity and expected UID. |
| Acceptance order | Durable backend-assigned acceptance sequence used to order successful receipts within the exact subject, scope, profile and capture-context partition. It is separate from producer time, storage order and capture ID. |
| Read context | Baseline or exact branch context, plus checked model bindings. A branch name by itself is not a revision. |
| Bindings | Exact context and subject/scenario selectors with actual owner UIDs, etags and scenario digest where applicable; include relevant Screen composition or checked human path when claimed. |
| Artifact | Typed immutable artifact reference containing namespace and artifact ID plus required `expected_sha256`, bound to verified byte size, media type and dimensions. Namespace and immutable ID establish identity; the digest is an integrity pin, not content-addressed identity. Never persist a presigned delivery URL as identity. |
| Profile | Complete structured capture profile and its backend-computed fingerprint. |
| Provenance | Explicit kind: imported design imagery, runtime capture or other imported evidence. Retain producer/tool identity, producer-asserted capture time and server receipt time separately. |
| Source target | Exact build/revision identity when known, environment and fixture identifier. Record absence explicitly; a branch label or mutable deployment URL alone is not an exact build. |
| Derivation | Optional exact parent capture/artifact reference for a thumbnail or redacted derivative, with the derivation kind. Original bytes and identity remain unchanged. |

Producer assertions such as a source commit or capture time remain assertions
unless separately attested and verified. The backend certifies only the checks
it actually performs. An image that depicts success does not prove a command
executed or a test passed.

### Capture profile

The profile is a typed value containing pixel viewport width and height, device
scale, platform/browser identity where applicable, theme, locale, and optional
Persona/role and fixture context. Required execution conditions must be
explicit; an unspecified value is not a wildcard match. Secrets, cookies,
passwords and real personal records are not profile fields.

The backend canonicalizes the complete value and computes its fingerprint.
Changing any comparison-relevant value produces a different profile identity.
Store the value as well as the fingerprint so clients can explain mismatches.
An unchanged friendly label cannot make mobile match desktop or one role match
another. Each receipt allows at most 128 checked bindings and 16 KiB of profile
and provenance data; exceedance rejects without truncation. Image byte,
dimension and decoded-pixel limits are deployment-configured admission bounds,
reported by capability discovery and enforced before allocation.

### Ingestion and attachment

1. An authorized producer starts an attempt against an exact subject, profile,
   target and checked binding set using current write guards.
1. Upload and finalize immutable artifact bytes through the implemented
   [artifact workflow](../how-to/manage-artifacts.md). Atlas-mediated transfer
   and control use gRPC; direct S3 transfer uses presigned HTTP. NATS supplies
   no provider HTTP image URL. A remote MCP service cannot read a caller's local
   file merely from its path.
1. Before accepting a successful capture, verify current artifact read authority,
   immutable Ready identity, digest and size. Decode an allowlisted inert raster
   format from PNG, JPEG or WebP with bounded dimensions, pixels, memory and
   work; compare actual media and dimensions with submitted metadata. Reject
   malformed, active or unsupported content. A thumbnail requires its own
   verified derived artifact.
1. Admit the typed attachment with current state/binding guards and artifact
   attachment coordination. A stale model refusal leaves an unattached Ready
   artifact recoverable; it does not publish a capture with rewritten provenance.
1. Return the immutable receipt. Repeat the same accepted operation idempotently;
   a changed request under the same operation identity rejects. Concurrent or
   delayed producers cannot overwrite a receipt or another attempt's bytes.

In-progress attempts have bounded leases. Completion, failure and expiry are
terminal outcomes with retained diagnostics safe for the caller to read. A
newer failed attempt never deletes the last successful receipt. Artifact
publication, capture attachment and accepted baseline selection are separate
operations; no cross-store atomicity is assumed.

Each attempt receives a durable server-assigned creation sequence within its
exact subject, scope, profile and capture-context partition before work starts.
Retain that sequence on every terminal record and successful receipt. Replaying
an accepted attempt operation returns its existing identity and sequence; it
does not create a newer attempt. Creation order and successful acceptance order
answer different questions and must not be conflated.

## Evidence status

Compute status in the backend for an explicit authorized scope, profile,
comparison target and policy revision. Clients display it rather than infer
freshness from timestamps or a nonempty image URL.

Select the default evidence candidate by exact typed subject and incarnation,
baseline/branch context, complete profile and optional subject-specific capture
context. For an email occurrence, this includes an explicitly selected trigger
context when one is claimed. Within that partition, select the successful
receipt with the greatest server-assigned acceptance sequence. Never use a
producer timestamp, storage iteration order or lexical capture ID as recency.
An explicit authorized capture selection can inspect an older receipt.

Binding or source-target equality is a comparison after candidate selection,
not a filter that hides stale evidence. A newer successful mismatching capture
must not disappear in favor of an older matching one. A newer failed attempt
is reported separately and cannot displace the successful candidate.

| Freshness | Meaning |
| --- | --- |
| Missing | No eligible successful capture exists for this state and selected profile. |
| Current | Required model bindings, source target, profile and configured age policy are all known and match. |
| Stale | A checked required binding differs, or a configured maximum age expired. |
| Unknown | A required target or provenance fact is absent or cannot be compared. |

Apply the statuses deterministically: an authorized subject/profile with no
candidate is Missing; a known required mismatch is Stale even when another
required comparison is unknown; otherwise a required unknown is Unknown; all
required known matches produce Current. Visibility and operational failures
remain opaque unavailability or errors before this evaluation, not freshness
values. Changing a source target is a mismatch, not a missing image.

The comparison policy identifies required bindings, source-target kind, maximum
age if any, its clock basis and provenance trust requirements. Unverified
producer assertions cannot satisfy a requirement for verified facts. Imported
design imagery can compare to an exact design revision under its own policy;
Current then describes that declared evidence scope, not runtime execution.

Latest attempt is a separate value: no attempt, in progress, succeeded, failed
or expired. A failed attempt with an older Current capture displays both facts.
Visibility or operational failure is not Missing or Unknown success.

Select latest attempt by the greatest durable attempt creation sequence in the
same partition. Delayed completion, failure or expiry of an older attempt cannot
replace the displayed status of a newer attempt. Producer timestamps, terminal
arrival order and storage iteration order do not define attempt recency.

Baseline selection is a separate optional record: no selected baseline, or an
exact successful capture and profile selected by a guarded mutation with actor
and reason. It does not rewrite the capture. Baseline comparison is separately
Unreviewed when no baseline is selected, Unchanged for equal comparable bytes,
Changed for different comparable bytes, or Unavailable when comparison cannot
be performed. An Accepted label means an explicitly selected baseline, never
automatic acceptance of a newer capture. Matching digests prove equal bytes;
different digests establish changed bytes, not a perceptual assessment.
Historical accepted baselines may be Stale, and changed imagery may be Current.

## Flow target and occurrence identity

Extend the existing `TopologyStepTarget` union with typed `visual_state`, using
an exact VisualState reference and expected incarnation UID. Include it in
admission, detail hydration, traversal, reference extraction, impact,
native export and every client codec. A custom property or URL containing a
state slug does not satisfy this contract.

Persisted DesignGraph owner identity, owner UID and `step_id` identify a flow
occurrence. Equal VisualState targets in the same graph or several graphs stay
distinct. Authored step and transition order survive projection. Transitions
keep their existing `connection_id` behavior, labels, branching, joins, loops
and subflow limits. A changed state target follows the existing step-edit rules;
capture selection must then be recomputed without reusing another target's
image.

Flow projection returns declared topology first and joins only authorized
state/capture data. Missing state, missing capture, generic step and unexpanded
subflow remain distinguishable. Hidden and missing target resolution share the
same public unavailable result. The projection is authored topology, not a
Storyboard execution trace.

## Lifecycle, references and retention

| Operation | Required behavior |
| --- | --- |
| Create, preview and apply | Use existing backend authorization, namespace, entity protection, concurrency guards and branch rules. Preview is not publication. |
| Reorder associations or flow steps | Preserve IDs, multiplicity and exact selectors. |
| Context, subject or scenario removal | Safe deletion uses a complete typed reference inventory. Force deletion leaves retained selections visibly unresolved. |
| Owner recreation | UID-bound context, scenario, capture and flow selections cannot bind to a same-key replacement without explicit guarded retargeting. Reused plain CatalogSubject entity references retain native exact-key semantics and may resolve to a replacement; capture read-set UID/etag comparison then detects the change. No new incarnation guarantee is implied for those plain refs. |
| Branch edit and landing | Preserve exact state/connection/step identities under existing same-incarnation rules. Recheck referenced UIDs, attachments and guards at landing. |
| Branch capture | Receipt stays bound to the branch/read-set actually captured. Landing does not relabel it as baseline evidence. Reuse requires an explicit checked comparison to the baseline target. |
| Export and reapply | Preserve exact state content, subject and step IDs, optional presence and artifact selectors. Export referenced capture/baseline records with explicit completeness and availability; no image bytes or provenance are regenerated. |
| Historical inspection | Use the selected historical model and retained evidence. If historical comparison is unsupported, show retained declarations and an unavailable comparison rather than live status. |
| State or branch deletion | Retain required historical capture/baseline references according to policy. Deletion cannot immediately collect bytes still held by another state, branch or supported history. |

Typed artifact attachment accounting must enumerate current model references,
supported branches, capture receipts, baseline pins and retained history.
Attachment holds and deletion claims must coordinate before any artifact
collection can be enabled. Define explicit retention, pins and a recovery
window after the last reference disappears. Ready artifacts are not expired
merely because they are unattached. Automatic cleanup is not provided by this
contract's documentation or by a catalog reference scan alone.

### Supplied-source isolation

VisualState, capture and baseline authoring and the new visual grouping/target
inventories are native-only until a separately defined supplied-source profile
supports them. Source reconciliation must reject unsupported VisualState
create/whole-owner mappings and field or occurrence selections, including
attempted clears. It must not claim partial ownership by ignoring unknown data.

For existing Screen owners, a Source selection of `catalog_areas` rejects, and
whole-owner updates reject when either current or incoming content contains
that native grouping. Supported unrelated field-specific Source writes preserve
the grouping. New visual-state graph targets follow the existing native-only
DesignGraph topology boundary. Capture, attempt and baseline records are not
silently admitted as Source-owned entity fields. Native manifest export,
diff and guarded reapply remain required under their explicit record and
artifact completeness rules.

### Capture-aware snapshot export

A page/flow snapshot export freezes its authorized model read set, exact state
and flow-step identities, selected captures/profile, comparison target and
completeness. Include visible missing/unavailable evidence and comparison reasons
rather than claiming full visual coverage. Use verified inert imagery and a
trusted snapshot layout; do not execute imported pages or follow source URLs.

Retain the complete input identity set for a persisted export and its derived
images. Generation must not replace frozen inputs with same-key live owners.
Recheck every required input's current authority before release and each later
persisted-export delivery, in addition to output-namespace access. See the
[artifact integration requirements](visual-artifact-integration.md) for shared
delivery, retention and transitive-input rules. A downloaded snapshot is a
retained export, not a live access-controlled model or proof of runtime behavior.

## Query and presentation contract

These are required semantic operations, not available RPC or command names.

| Operation | Required result |
| --- | --- |
| List page/state catalog | Authorized Screens including those outside Storyboards, standalone states in an explicit separate grouping, state summaries and scoped coverage. Filter by namespace, explicit page or state area, flow, context, profile and evidence status; search titles, routes and identifiers. |
| Inspect exact state | State, etag, checked context and subject resolution, eligible capture, latest attempt, baseline and outgoing/incoming reference links. |
| List capture history | Stable pagination of immutable receipts and terminal attempts for the exact state/profile; no client-side all-history fetch. |
| Project visual flows | Exact DesignGraphs and persisted step/transition identities joined with authorized thumbnails and explicit unresolved/incomplete markers. |

Catalog/history pages default to 50 records, permit at most 250 and reject
larger requests. Tokens bind authorization scope, branch or historical target,
filters, profile, source comparison target, subject-specific capture context,
comparison policy and a checked revision. Changing any bound
input, expiry or invalidation rejects the token; it must not silently resume a
different collection. Stable pagination cannot skip or duplicate occurrences
when storage iteration order changes.

All results carry checked scope/revision, comparison target, `incomplete` and
structured failures where partial results are supported. Complete empty results
are distinct from partial, unavailable or failed reads. Counts cover only the
same authorized filtered scope and selected profile. Exact totals require a
complete count evaluation; partial totals are labeled lower bounds, never
product-wide totals.

Reuse current catalog ceilings of 10,000 examined occurrences and 4,096 emitted
records per bounded query, with lower limits enforced first. Exhaustion that
cannot produce a truthful supported partial result returns an explicit error.
Do not fetch all pages in Studio to fabricate counts or hide budget exhaustion.

Studio is read-only for the model. It can browse Pages, select named states,
inspect captures and communicate intent. Flows support named groups, labeled
branches, pan/zoom, fit selection and an accessible non-canvas listing. Keep
existing behavioral Journey and Sitemap projections separately named.
Deep links retain exact resource, branch/history and flow-step selection.
Load metadata before imagery and fetch verified immutable thumbnails near the
viewport; full-resolution images are fetched only when needed.

Image delivery enforces current read authority independently for original,
thumbnail, download and export. This contract proposes a same-origin
authenticated Studio bridge using existing artifact content RPCs, refining the
proposal's unsettled browser-delivery choice. No such image route is implemented
by this document. Studio does not receive storage credentials or persist
expiring grants in model records. Direct S3 delivery is a separate opt-in policy
with explicit grant-expiry and revocation limits; it is not portable to NATS.
The [artifact integration contract](visual-artifact-integration.md#browser-delivery)
defines these boundaries. A successful receipt or expired signed URL is not
permanent read permission. Imported URLs are inert provenance, never instructions
for Studio or the backend to execute or crawl a live application.

## Delivery boundary

Native state lifecycle, verified artifact attachment, capture/status queries,
typed flow targets, CLI/MCP authoring and connected Studio inspection are all
required for the complete capability. Documentation, schema generation,
mocked renderers or a storage upload alone cannot close that acceptance.

Email-specific subjects, triggers, scheduling, source inventory, variants and
sequence membership need their own typed contract. An email rendering may reuse
this capture foundation, but the existing behavioral Sequence projection does
not establish an email sequence catalog. Visual diffing, scheduled recapture,
route crawling and live replay remain extensions.
