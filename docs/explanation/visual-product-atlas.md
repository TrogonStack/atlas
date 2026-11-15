# Visual product Atlas

Status: proposed product capability and architecture. The page/state catalog,
capture records, visual flow explorer and email catalog described here require
implementation.
The existing artifact service supplies byte storage and transfer, not these
visual capabilities. The [delivery roadmap](visual-product-atlas-roadmap.md)
defines contribution slices and completion evidence.

The proposed [visual-state contract](../reference/visual-state-contracts.md),
[artifact integration requirements](../reference/visual-artifact-integration.md)
and [email catalog contract](../reference/email-catalog-contracts.md) refine the
native boundaries. Their acceptance requirements describe future verification;
they do not establish implemented schemas, APIs or Studio support.

## Product intent

Atlas should provide a navigable inventory of a product's pages, visual states
and user flows. A person can move from a product overview to an exact state,
inspect a capture, understand the action leading to another state and discover
the Event Model behind it.

The same inspection should cover a product's emails: declared sources, copy
variants, trigger and timing context, captured renderings and occurrences in
authored sequences. Email documentation shares evidence and graph identities
with the visual catalog while retaining its own template and variant model.

The target experience combines searchable screenshot catalogs with grouped,
branching flow maps. It includes thumbnails, named states, labeled transitions,
alternate outcomes, filters, scoped counts, capture freshness, pan and zoom,
and direct inspection of a selected occurrence.

A signup/login flow is a useful acceptance fixture for this documentation
capability. Building Atlas's own signup or identity-provider integration is a
separate authentication requirement.

## Existing foundation

| Existing capability | Reuse and boundary |
| --- | --- |
| [Screen and UI](../../proto/trogonatlas/eventmodel/v1alpha1/moments.proto) | Screen describes page composition and a route; UI is a behavioral moment. Neither establishes a captured state catalog. |
| [Connection identity](../reference/connection-identity.md) | Exact owner, owner incarnation and occurrence ID select the interaction being documented. Equal endpoints do not select the same occurrence. |
| [Catalog subjects](../../proto/trogonatlas/eventmodel/v1alpha1/decision_document.proto) | Typed selections connect documentation to entities, connections, contract members and ports. |
| [DesignGraph](../reference/design-topology-contracts.md) | Authored steps, labeled transitions, branches, subflows and repeated occurrences provide flow topology. A visual-state target still needs implementation. |
| [Studio Screens](../../tsworkspace/webapps/studio/src/components/canvas/ScreensBoard.tsx) | Existing Journey and Sitemap projections provide schematic inspection. They do not establish screenshot-backed page/state coverage. |
| [Artifact service](../how-to/manage-artifacts.md) | NATS Object Store or S3-compatible storage, verified byte transfer and immutable artifact identities are implemented. Typed attachments, raster verification, capture freshness and Studio image delivery remain separate work. |

The backend remains authoritative under the
[Atlas principles](principles/README.md). Studio presents read-only inspection;
agents author model changes through the native guarded backend workflow.

## Proposed concepts and identity

| Concept | Responsibility |
| --- | --- |
| Page | The catalog presentation of an existing Screen. A route helps discovery but does not establish identity. |
| VisualState | A proposed native entity declaring a named visible state, optional exact Screen/UI context and exact behavioral subject associations. |
| Capture profile | Explicit viewport, device scale, platform, theme, locale and relevant fixture or role context. |
| Capture record | Immutable evidence binding accepted image bytes to an exact visual-state or email-variant subject, profile, model scope and source target. |
| Flow | An existing DesignGraph, extended with typed visual-state selections. |
| Flow step | A distinct graph occurrence. Several steps can select the same VisualState without collapsing their identities. |
| EmailTemplate and variant | A proposed native template with owner-bound source and variant selections. Subject text, provider keys and filenames are not identity. |
| Email sequence | An explicitly classified DesignGraph FLOW whose steps select exact email variants and optional trigger occurrences. It documents topology without sending or scheduling mail. |

Reuse Screen and DesignGraph rather than introducing another page identity or
graph store. VisualState needs the ordinary native entity identity, version,
incarnation and lifecycle. The exact schema and wire allocations remain design
work; these names do not announce existing API fields.

A state can represent a default page, pending submission, empty result,
validation error, rejected command, success, modal or behaviorally distinct
role-dependent view. A connection-attached state can exist without a Screen or
route. It remains discoverable without inventing a page for it.

Responsive size, locale and theme usually belong to the capture profile.
Different behavior or a meaningfully different visible condition warrants a
distinct state. Declared roles and fixtures describe the capture; they do not
grant Atlas permissions.

## Pages catalog and state inspection

Add Pages and Flows alongside the existing Journey and Sitemap views. Keep
those perspectives separately identifiable.

The Pages catalog should provide:

- List and grid discovery for every authorized Screen, including Screens outside
  Storyboards.
- Title, route when declared, authored area, representative capture, named
  states and evidence status.
- Search across titles, routes, state names and exact identifiers.
- Filters for authored area, flow membership, persona or fixture context,
  platform/profile and capture status.
- Authorized counts for pages, states, captured states, current captures and
  flows within the selected scope.
- Explicit incomplete, unavailable and truncated results, distinct from a
  complete empty result.

Area is authored catalog grouping. It must not silently equate a URL prefix
with a bounded context or namespace. Namespace scope remains an independent
filter and authorization boundary.

The [state contract](../reference/visual-state-contracts.md#page-grouping)
proposes authored Screen catalog areas independently from state-area labels.
Pages without states or Storyboards still appear, including an explicit
Unassigned group when no page area is authored.

Selecting a state opens its image, profile, evidence status, source revision,
Screen composition, related UIs, behavioral subjects and containing flows.
Deep links preserve exact resource selection, branch or historical scope and
flow occurrence where applicable.

## Visual flow explorer

The overview groups authored flows by name and description. A focused flow
shows state thumbnails joined by labeled actions and outcomes, including
branches, joins, cycles, repeated states and exact subflow references.

Provide pan and zoom, fit-all and fit-selection controls, keyboard selection,
incoming/outgoing navigation and a readable non-canvas listing. Missing images,
unresolved targets and generic steps stay inspectable.

Graph step IDs remain stable when titles, labels or selected targets change
under the existing topology contract. Reusing a state creates another step
occurrence, not another state definition. A DesignGraph cycle describes authored
topology; it must not invent a backward causal edge or collapse repeated
moments in a Storyboard.

## Email catalog and sequences

The proposed [email contract](../reference/email-catalog-contracts.md) adds
native templates with identified source bindings and variants. A source can
describe server code, an external provider or an explicitly unknown origin.
Provider/account keys are inert provenance, not access credentials or evidence
that Atlas contacted the provider. Reading the catalog does not send mail.

Each variant declares its copy state, lifecycle, triggers and timing. Retained
copy, source-only copy and undefined copy have distinct meanings. A dynamic
subject can have a description and synthetic example without claiming the
example is production copy or introducing an expression evaluator.

The catalog should provide list/grid discovery, authored area groups, subject
and source details, variant expansion and exact sequence links. Backend search
and filters cover source, trigger kind, sequence, lifecycle and evidence within
the authorized scope. Template counts, variant counts and repeated sequence
occurrences remain distinct units.

Reuse DesignGraph FLOW for explicitly classified email sequences. Repeated
reminders can select the same variant at distinct steps and select different
exact trigger occurrences for their delay or schedule context. Listing position
does not establish execution order or timing. Present RETIRED variants remain
inspectable; removing a local variant ID makes old selectors unresolved.

Email captures use a typed email-variant subject, sharing capture profiles,
immutable artifacts, freshness, attempt history and guarded baseline selection.
They need no fabricated Screen or VisualState. A declared live/dead/unknown
lifecycle stays separate from evidence status; a failed capture attempt can
coexist with a retained successful image.

Previews use verified inert imagery or escaped source. Retained HTML cannot
execute scripts, follow links, load tracking pixels or initiate arbitrary
network requests. PDF export freezes an explicitly selected authorized input
set, records exact variants/profile/target and preserves missing-copy evidence.
Persisted exports and derivatives recheck every required transitive input's
authority on later delivery. The
[email acceptance requirements](../reference/email-catalog-acceptance.md) cover
these public boundaries, hostile preview inputs and actual generated PDF bytes.

## Connections to Event Modeling

A UI-to-command connection is a first-class attachment point. The same exact
interaction can expose ready, submitting, validation-feedback, command-rejection
and accepted-result states, each with its own capture evidence and optional
scenario/outcome context.

An association must select the exact containing owner/version, its incarnation
and the stored connection occurrence. For a composite selector spanning several
connections, retain the complete selection. Endpoint equality, array position,
diagram edge IDs and matching titles do not establish an association.

Inspection should work in both directions:

- From a state to its Screen, UIs, command slice, commands, events, read models
  and declared scenario or Storyboard context.
- From an exact connection or scenario to its documented visual states.
- From a flow occurrence to the selected state and the evidence applicable to
  that occurrence's context.

Input validation before command issuance differs from rejection after
submission. An error image alone cannot establish that a command was sent or
that an event occurred. Those behavioral facts require explicit model and
scenario associations. Missing mappings remain visible.

## Capture evidence and freshness

Separate a VisualState declaration from evidence about it. A state may have no
capture, and imported design imagery may describe intended behavior rather
than a running implementation.

An accepted capture record should retain:

- Exact state, context and behavioral selections, with checked incarnations,
  revisions and composition/connection/scenario bindings where applicable.
- Artifact namespace, immutable artifact ID and expected SHA-256 digest.
- Verified image format, dimensions and byte size.
- Capture profile, source build or commit when known, environment and fixture
  identity, producer and capture-time provenance.
- Backend acceptance ordering and verification results, kept separate from
  caller-supplied claims about when or how a capture was produced.
- Exact source relationships for derived thumbnails or sanitized copies.

Original artifact bytes remain immutable. Baseline selection and attempt history
have their own lifecycle. A successful capture does not automatically become
the accepted baseline, and a failed attempt does not erase prior evidence.

Freshness compares evidence with an explicit target:

| Status | Meaning |
| --- | --- |
| Missing | No eligible capture exists for the selected state and profile. |
| Current | Required checked bindings, source target and profile match under a declared policy. |
| Stale | A required binding differs or a declared age policy has expired. |
| Unknown | Required provenance or comparison target is unavailable. |

Choose the newest successful receipt by backend acceptance order within its
exact subject, scope, profile and capture context, then compare it to the target.
An older matching capture cannot hide a newer stale or unknown capture.
Producer timestamps do not establish this ordering.

Show the latest attempt separately as no attempt, in progress, succeeded, failed
or expired. Keep visual comparison and reviewed-baseline status separate from
freshness. A changed image can be current; a reviewed historical baseline can be
stale.

Counts bind to the selected namespace, model scope, profile and comparison
target. Desktop evidence does not establish mobile coverage. Restricted evidence
must not leak through aggregates. Historical inspection uses an explicit
historical comparison target when supported; otherwise that comparison is
unavailable rather than replaced with a live result.

## Artifact integration and image delivery

Reuse the implemented [artifact workflow](../how-to/manage-artifacts.md):
discover limits, begin an upload, transfer bytes, complete verified publication,
then submit a separate guarded model attachment. Atlas-mediated transfers and
control operations use gRPC. Direct S3 byte transfers use presigned HTTP requests.
CLI/MCP clients discover the server's backend configuration.

The proposed typed model reference binds artifact namespace and immutable ID,
with an expected digest for capture integrity. It does not persist provider
credentials, bucket keys or temporary delivery URLs. Equal digests do not merge
artifact identities or grant access to another namespace.

Upload completion establishes stored-byte integrity and publication. It does
not establish raster format, dimensions, capture meaning, sanitization or a model
attachment. The visual ingestion layer must verify actual image bytes before
accepting inline evidence. The initial inline profile should cover static PNG,
JPEG and WebP with bounded input size, dimensions, decoded memory, processing
time and concurrency. Other retained files do not acquire inline-rendering
authority from their declared media type.

Verification and rendering preserve the original. Redaction, re-encoding and
thumbnail generation produce separate derivatives with their own identities
and exact source relationships. Capture producers remove credentials and
private user data before upload; image decoding alone does not establish that
an image is safe to share.

Browser delivery is an implementation decision still to settle. Studio needs
an authenticated browser-readable path that checks current model and artifact
access, returns verified inert imagery and respects bounded response capacity.
An authenticated Studio bridge can use existing gRPC reads; temporary S3 grants
remain delivery details. No new raw artifact HTTP route is implemented by this
proposal, and a signed URL must not become persistent model identity.

Cross-namespace attachments require authority for both the model mutation and
the referenced artifact. Thumbnail caches, exports and persisted derivatives
must preserve these checks for every transitive input, including on later reads.

Artifact publication and model attachment are separate operations. A failed
attachment can leave a recoverable accepted file. Future collection must
coordinate attachment holds and deletion claims rather than assume an atomic
transaction across model writes and object storage. Retain confirmed bytes
while reference accounting is incomplete. The
[artifact lifecycle proposal](../reference/artifact-lifecycle.md) defines the
separate cleanup work.

## Native operations and Studio boundary

The backend owns state identity, associations, reference/impact discovery,
admission, capture acceptance, baseline changes and authorized projections.
These operations must participate in native lifecycle rules: branch guards,
preview/apply, safe deletion, explicit retargeting and manifest round trips.

CLI/MCP surfaces need discovery, authoring and inspection support for states,
typed flow targets, capture ingestion and baseline selection. A caller-local
file path does not give a remote MCP server access to that file. File-capable
clients handle the transfer through the artifact workflow.

Studio consumes backend projections for review, navigation and comparison. A
catalog tile must not execute an imported page, crawl arbitrary URLs or turn
the browser into another authoritative editor. Capture execution belongs in a
separately authorized producer or CI workflow.

## Rendering and representative scale

Reuse the existing React Flow foundation with screenshot nodes, labeled
connectors, flow grouping and level-of-detail presentation. Load topology and
status before full-resolution imagery; fetch visible thumbnails progressively
and virtualize the catalog. Zooming out must preserve identifiable flow groups
and selection.

Use a representative fixture with hundreds of pages/states and dozens of flows.
Measure loading, layout, decoding, interaction latency and memory before
committing performance budgets. The roadmap defines connected acceptance;
renderer-only or schema-only checks cannot establish the complete capability.

## Design references

These products provide interaction patterns; Atlas retains its own identities,
backend authority and Event Modeling semantics:

- [Overflow](https://overflow.io/): focused screen-based flows and labeled
  connections.
- [Mobbin](https://mobbin.com/): dense, searchable screenshot discovery.
- [Storybook stories](https://storybook.js.org/docs/writing-stories): explicit,
  reproducible UI states.
- [Chromatic visual tests](https://www.chromatic.com/docs/visual): separate
  captured evidence, comparison and reviewed baseline.
- [React Flow viewport](https://reactflow.dev/learn/concepts/the-viewport): canvas
  pan, zoom and navigation.

Pixel diffing, scheduled recapture, design-tool synchronization, route discovery
and live replay can build on this foundation as separately scoped extensions.
