# Email catalog and sequence contracts

Status: proposed native contract. EmailTemplate resources, email step targets,
capture joins, catalog queries, inert previews and PDF export are not implemented
by this document. The [acceptance requirements](email-catalog-acceptance.md)
describe the evidence required before claiming them. Names below specify the
intended model, not existing protobuf fields, RPCs or manifest syntax.

## Scope and existing foundations

The catalog documents the product's emails, their declared triggers and timing,
their appearances, and their places in authored sequences. It does not send
mail, schedule campaigns, execute templates or establish delivery success.

This is an email-specific extension of the
[visual product Atlas proposal](../explanation/visual-product-atlas.md).
Its shared state, evidence and delivery dependencies follow the
[visual product roadmap](../explanation/visual-product-atlas-roadmap.md);
the email resource and sequence additions below remain proposed work.

Reuse the existing [DesignGraph](design-topology-contracts.md) for sequence
topology, [CatalogSubject](decision-and-document-contracts.md#typed-subjects-and-assignments)
for behavioral links and [connection identity](connection-identity.md) for
identified associations. A Channel describes transport; a Specification message
describes a contract. Neither is an email template. The existing Studio Sequence
projection describes Event Modeling moments and is not a campaign catalog.

The [visual evidence contract](visual-state-contracts.md) supplies the proposed
capture, comparison and baseline rules. Email captures require an explicit typed
email-variant subject extension to that evidence layer. They must not be
represented by a fabricated Screen, UI or VisualState. The implemented
[artifact service](../how-to/manage-artifacts.md) supplies verified byte
transfer and immutable artifact identity when enabled by the deployment.
Typed model attachments, image verification, capture acceptance and browser
delivery remain proposed integration work.
The [artifact integration requirements](visual-artifact-integration.md) define
typed ArtifactRef identity, image verification, browser delivery and retention.
Artifact metadata alone does not certify HTML safety, image format, dimensions,
rendering correctness or freshness.

## Native EmailTemplate resource

Select a new top-level `EmailTemplate` kind with an ordinary namespace, slug,
positive exact version, server-owned incarnation UID, etag, branch lifecycle,
registered metadata and optional same-kind `supersedes`. Follow existing native
preparation, authorization and guarded mutation rules. Coordinated schema work
must allocate available tags and update the complete closed kind inventories;
this proposal reserves no numeric tags.

| Value | Proposed rule |
| --- | --- |
| Title and documentation | Required CatalogTitle and optional CatalogText. Titles and subject lines do not establish identity. |
| Area | Optional authored catalog grouping. A URL, namespace, provider or bounded context does not implicitly supply it. |
| Variant lifecycle | Each variant declares `LIVE`, `RETIRED` or `UNKNOWN`; UNKNOWN includes a reason. Template summary is derived from its authorized variants and can be mixed. These are authored deployment assertions, not observed send statistics. The UI may label RETIRED as dead. |
| Sources | Nonempty identified source-binding collection. Unknown source is an explicit alternative with a reason, not an invented server source. |
| Variants | Nonempty identified collection of separately selectable copy/behavior variants. A single default variant remains an explicit record. |
| Subjects | Identified SubjectAssociation records selecting exact related behavioral and catalog subjects. Grouping is not a checked behavioral path. |

Sources and variants use server-prepared `LocalValueId` values unique within
their respective owner collections. Persisted selectors include the containing
EmailTemplate's exact identity, incarnation UID and local ID. Renaming,
reordering and editing copy preserve the variant ID. Retargeting a source binding
to a different provider/account/template or repository location requires a new
source ID. Removing and re-adding a source or variant requires a fresh ID.
Deleting and recreating a template cannot revive old selectors.

New owners follow existing explicit-ID creation rules. On existing owners, new
records omit IDs for server assignment; callers retain authoritative IDs before
a later guarded write selects them. Duplicate, malformed, stripped or retired
IDs reject rather than selecting by name, list index or content equality.
Retired local IDs mean removed occurrences. A present variant whose declared
lifecycle is RETIRED remains selectable and inspectable, including in sequences
and retained evidence.

### Source bindings

Source identity distinguishes a logical template from its supplying systems.
One template may document a migration between a server implementation and an
external provider. Matching upstream slugs never merges separate templates.

| Source alternative | Required declaration |
| --- | --- |
| Server | Logical repository, repository-relative location and template key; optional exact source revision. Use the existing code-reference conventions without accepting absolute local paths or credentials. A revisionless locator is not immutable source evidence. |
| External provider | Provider kind/key, opaque account or workspace key and upstream template key; optional upstream revision. These are non-secret logical locators. Different accounts remain distinct even when upstream keys match. |
| Unknown | Substantive reason why the source is not yet known. |

The source label is server or the configured provider display name. It conveys
provenance and filtering, not access rights or runtime availability. Source
locators are inert metadata. Reading the catalog never follows a URL, opens a
repository, calls a provider or sends a message. A future connector needs its own
explicit import, authorization and reconciliation contract.

Source bindings are inline metadata governed by their containing template's
visibility. They introduce no per-provider ACL or inferred provider access.
References from a template to another native owner or artifact still require
that dependency's current authority. A hidden-source test selects an unreadable
containing template or referenced native dependency, not an invented ACL on an
inline source record.

Each variant selects exactly one active source binding. Additional source
implementations use distinct variants, even when their copy is equal. This
prevents a provider filter from attributing one capture to several unverified
implementations. A source-only or undefined variant can still be cataloged.
Changing a source revision at the same logical locator preserves the source
ID; retargeting a variant to another source preserves the variant ID. Both
changes require a new evidence-applicability check. Earlier receipts retain
their original checked source and cannot inherit the replacement's provenance.

### Variants and copy

Each variant declares a name, selected source ID, lifecycle assertion, optional
locale/audience description, identified trigger declarations, and one copy
alternative. A role or recipient description never grants authorization.

| Copy alternative | Meaning |
| --- | --- |
| Retained | Retained UTF-8 subject expression plus optional preview text and at least one retained HTML or plain-text body. Expressions and variable names are opaque source text, never executed by Atlas. |
| Source only | Copy exists outside retained Atlas content; include a reason and select the known source. No preview or subject is invented. |
| Undefined | Copy has not been defined; include a reason. This is different from a known external copy that has not been captured. |

A retained subject explicitly selects literal text or a dynamic expression with
a description and an optional synthetic example. A source-only or undefined copy
may retain a subject description separately without claiming a known subject
value. Dynamic subject and body text do not add a template language,
expression evaluator or
model schema. Retained copy has a server-computed digest over exact original
bytes and media types in a versioned, unambiguous canonical envelope.
Sanitized HTML, rendered captures and PDF output are separate derived artifacts
with their own digests and provenance. Sanitization
does not silently rewrite retained source.

Variants are explicitly authored. The backend never expands condition prose,
locales, timing ranges or placeholder combinations into a guessed variant set.
Variant counts report authorized declared records. A job that sends reminders
at several times can reuse a variant in distinct sequence occurrences.

Copy alternatives describe retained text, not the availability of imported
imagery. Source-only or undefined copy may have a captured preview with explicit
provenance. An image does not promote either alternative to retained copy.

## Trigger and timing declarations

A variant's trigger records use persisted ConnectionId values. Each selects a
trigger alternative, documentation, optional exact CatalogSubject mapping, and a
timing alternative. The declared trigger kind does not itself add a Command,
Event, Processor, human-interaction path or execution guarantee.

Trigger IDs are unique across the complete containing EmailTemplate connection
inventory, including its SubjectAssociation records. An exact trigger selection
retains the template identity and incarnation, selected variant ID and trigger
ConnectionId. Resolution checks that the selected trigger still belongs to that
variant; the same ID cannot select a sibling variant's timing by accident.

| Trigger alternative | Declared meaning and model boundary |
| --- | --- |
| User action | A person takes an action. An optional exact UI issuance connection links the intended action; an endpoint pair or action label cannot certify issuance. |
| Webhook | An external notification is described. Optional links select the existing external-source ReadModel, observation connection or external boundary; Atlas does not create a webhook receiver. |
| Schedule | A recurring or scheduled job is described. Existing automation remains ReadModel observation through Processor and optional Command; a schedule label does not introduce a second trigger engine. |
| Administrative action | A described operator action, optionally linked to exact native subjects. Catalog responsibility or Team membership is not permission to perform it. |
| Provider signal | Provider tag/event kind and opaque signal key within the selected provider source. A provider signal is not implicitly a native Event or equivalent transport message. |
| Unknown | A substantive reason; the inspector exposes the missing behavioral mapping. |

Use identified SubjectAssociation records for additional contextual subjects.
Where a checked human or automation path is claimed, resolve the complete native
tuple using its existing inspection contract. A CatalogSubject association alone
claims relevance, not causal proof. Scenario context must use the exact containing
slice incarnation and a checked scenario selection; a title is insufficient.

| Timing alternative | Meaning |
| --- | --- |
| Immediate | Declared relative to the selected trigger, with any condition documented. It is not a latency guarantee. |
| Delay | Typed duration from zero through 365 days inclusive relative to the selected trigger, plus optional condition prose. Do not parse duration from a human phrase. |
| Schedule | Opaque schedule description and explicit UTC or IANA timezone. Atlas does not parse, validate as an executable cron dialect or run it. |
| Unknown | A substantive reason why timing is unspecified. |

Different trigger records can describe the same native subject with different
timing and retain distinct IDs. Reordering preserves IDs. Retargeting a trigger's
kind, selected native subject or provider signal requires a fresh occurrence ID.
Changing timing or explanatory text preserves its ID while invalidating captures
that pin the earlier required content. Invalid or absent required alternatives
reject atomically; a missing field never means immediate.

## Sequence topology and occurrence identity

Reuse a `FLOW` DesignGraph, with an explicit optional email-sequence
classification. Existing FLOW and DIAGRAM graphs retain their current meaning;
the presence of an email target alone does not classify the entire graph.
Classification is projection metadata and introduces no workflow execution.
No new EmailSequence entity is required.

Extend TopologyStepTarget with a typed email selection containing the exact
EmailTemplate ref, owner UID and variant ID, plus an optional exact trigger
ConnectionId within that selected variant. A removed local ID, missing,
wrong-owner or wrong-incarnation variant/trigger is unresolved or rejected under
native write rules. A present variant with RETIRED lifecycle still resolves.
Do not substitute a generic custom step URL, title, arbitrary property or latest
template version. Flow targets selecting VisualState remain a separate typed
alternative in the same graph.

Selecting a trigger identifies the intended timing declaration for that step.
Two reminders can select the same variant with different trigger IDs and retain
their distinct delay or schedule context. Omission explicitly leaves occurrence
timing unselected; it does not choose the first trigger, derive timing from list
position or mean immediate. Detail inspection can still list all variant
triggers. Retargeting the selected trigger preserves the step ID and requires a
fresh evidence-applicability check.

The existing graph owner UID, step ID and transition ConnectionId identify
occurrences. Two steps selecting the same email variant remain distinct, including
repeated reminders, branches, joins and loops. Step reorder preserves identity.
Changing a step target preserves step identity under the DesignGraph contract;
the occurrence must select evidence for its new variant and trigger context.
Its previous preview cannot follow the target change; that receipt remains
evidence for its original subject rather than becoming globally invalid.
Removing/re-adding the step or recreating its graph retires prior occurrence
selectors.

An explicitly classified email sequence uses the existing authored `steps[]`
order for its catalog listing. Branches and cycles remain visible topology; this
order is not a total execution order. A position badge such as `2/3` reports the
occurrence's position among eligible email steps in that declared listing and
states its selected scope. An exact complete variant can appear at several
positions or in several sequences. A badge opens the exact graph occurrence,
not just the email template. Incomplete or privacy-filtered membership does not
expose hidden positions, global denominators or hidden graph names; show an
unavailable position instead of renumbering a partial sequence as complete.

Sequence membership is derived from typed step targets. A general graph subject
association, shared title, trigger, source or provider campaign name does not
establish membership. Email-sequence filters select exact graph identity; an
optional title search can discover graphs without changing this identity.

## Captures and independent status dimensions

A capture selects the exact EmailTemplate incarnation, variant ID and required
source revision, retained-copy digest when present, trigger/timing bindings,
synthetic fixture and capture profile that were actually rendered. Store images
as immutable typed artifact references containing namespace, immutable ID and
required `expected_sha256`. Capture admission verifies static PNG, JPEG or WebP
bytes, dimensions and decoding bounds under the shared artifact contract.
Profiles distinguish locale, theme, viewport, rendering client and
fixture/audience context. Rendering in one mail client does not prove
parity in another. Redaction/thumbnail derivations preserve source-artifact links.

An optional exact trigger selection is part of the capture's typed email
context and candidate partition. A generic variant preview can remain useful
when that selection is absent, but it cannot claim to certify a sequence
occurrence's selected timing or trigger context. Comparison targets and profile
values cannot silently substitute a different reminder occurrence.
Within the exact subject/profile/trigger partition, use the shared contract's
backend acceptance ordering to choose the successful capture before computing
freshness. An older matching capture cannot hide a newer stale or unknown
capture, and caller-supplied capture times cannot reorder accepted evidence.

Apply the [visual evidence rules](visual-state-contracts.md) to email subjects:

| Dimension | Required interpretation |
| --- | --- |
| Lifecycle | Authored live, retired or unknown assertion, independent of capture availability. |
| Freshness | Current, stale, missing or unknown against an explicit authorized target, required model/source bindings and profile. A recent image with unspecified target source is unknown. |
| Latest attempt | No attempt, in progress, succeeded, failed or expired, separate from the last successful capture. An undefined template can have a failed capture attempt without inventing body content. |
| Visual baseline | No baseline or an exact selected baseline, independently selected under guards; comparison is Unreviewed, Unchanged, Changed or Unavailable under the shared evidence contract. A new successful capture does not automatically replace a selected baseline. Different digests establish different bytes, not a perceptual change verdict. |

The catalog's compact health badge may say fresh, stale, uncaptured or failed.
It must retain the underlying dimensions: fresh maps only to current; uncaptured
maps to missing; failed names the latest failed attempt and still exposes any
retained successful image and its freshness. Unknown provenance remains unknown.
The inspector always exposes lifecycle, freshness, attempt and baseline
separately. Filtering on health has explicit backend predicates rather than
reverse-engineering a display color.

## Catalog reads and Studio inspection

The backend provides bounded operations for listing templates/variants, exact
variant detail, source/trigger/timing inspection, capture history, and incoming
sequence occurrences. These are proposed operation responsibilities, not shipped
RPC names. Search includes template/variant titles, exact identifiers, declared
subject expressions and retained preview text. Server filtering supports area,
source binding, trigger kind, exact sequence membership, lifecycle, profile,
freshness and latest-attempt state.

Every result carries the authorized namespace/branch or historical scope,
snapshot/read-set identity, applied filters, comparison target/profile and
completeness. Cursors bind all of these plus caller visibility and cannot be
replayed under different filters or scope. Ordering uses exact stable identity
as a final tie-breaker. A page boundary does not silently drop variants or
repeated sequence occurrences.

Group headings may report template or variant counts only when the unit is
named. Counts and filter facets cover the same authorized filtered scope, not
the currently loaded browser page. Incomplete scans label counts unavailable;
missing/hidden dependencies cannot make a complete global total. Operational
errors stay errors, not empty or uncaptured success. Hidden source/capture or
sequence records must not leak through search matches, facets, badges, totals,
thumbnails or export.

Studio is read-only inspection with list/grid views, text search, filter controls,
area grouping, template/subject/source details, variant expansion, sequence
badges, safe thumbnails and detail links. Detail links preserve exact template,
variant, graph occurrence when selected, namespace, branch/historical scope and
profile. Keyboard access provides the same inspection path. Context actions can
communicate intent to an agent; they do not send email or mutate the model.

## Inert preview and PDF export

Prefer verified raster captures for previews. Retained HTML is source text until
an isolated renderer produces a separately identified sanitized derivative.
Source inspection escapes HTML. A rendered preview must block scripts, handlers,
forms, navigation, popups, downloads, frames, embeds, service workers, arbitrary
data/blob payloads, CSS URL loads, remote fonts, tracking pixels and all other
network requests. Do not depend on removing script elements alone. Links are
inert text in the preview. Secrets, actual recipient addresses and production
personal data do not belong in fixture profiles or sample substitutions.

No browser or PDF renderer may fetch a provider URL, source location, externally
hosted image or nested stylesheet because retained HTML references it. Supported
inline images must be separately admitted, size-bounded artifacts and referenced
through authorized immutable identity. Refused or unsupported content stays
inspectable as escaped source with an explicit preview-unavailable diagnostic.

PDF export is a bounded projection of an explicitly selected authorized result
set and capture profile, not an implicit all-data export. It carries scope,
comparison target, generated time, exact template/variant/occurrence identities,
source provenance, missing-copy/status annotations and completeness. Do not put
expiring artifact URLs, credentials or unredacted fixture data into the PDF.
Use inert raster captures and trusted layout primitives; never execute retained
email HTML in the export renderer. Record an output digest and use a sanitized
download filename. If an export artifact is persisted or shared, retain an
export receipt with the complete exact input owner/revision, variant, capture,
artifact and derivation identity set. Bind rendering to that frozen read set;
do not refresh same-key inputs partway through a render. If an input revision
cannot be retained or checked, refuse the export rather than mix revisions
under a complete claim.

Recheck current access to every required input before releasing the output and
on every later persisted-export read, in addition to the export artifact's own
namespace. A readable output namespace cannot flatten restrictions across input
namespaces. Apply the same transitive input authorization to derived thumbnails
and redactions. Source changes do not rewrite frozen export content; revocation
blocks later delivery. Previously downloaded bytes cannot be revoked. Retention
holds cover the complete input set for the supported export lifetime.

## Lifecycle, authorization and resource bounds

All template writes require current namespace write authority and read/reference
access to required selected subjects, sources and artifacts. Read access to a
template is insufficient to disclose hidden cross-namespace dependencies. Roles,
provider account keys and lifecycle labels grant no authority. Missing and hidden
dependencies share the existing opaque unresolved presentation; backend/storage
failures do not. Capture ingestion and baseline selection use their own current
authorization and mutation guards.

Extend entity codecs, preparation, exact resolution, reference discovery, inverse
queries, impact analysis, safe deletion, branching, merge preview/apply,
historical reads, manifest export/diff/reapply, CLI and MCP together. Safe delete
sees typed template/variant subjects and graph targets; forced removal leaves
exact selectors unresolved. Same-store reapply preserves IDs and original bytes.
New-store import requires explicit incarnation rebinding. Source reconciliation
must reject this new native kind/association inventory until a separately defined
supported profile exists, rather than silently claiming partial ownership.

Artifact holds must include templates, variants, captures, derivations, baselines,
supported branches and history before any collection of confirmed bytes is
enabled. A failed model attachment can leave a recoverable unattached Ready
artifact. It cannot leave a successful template capture selecting partial bytes.

The proposed initial ceilings are 64 source bindings, 256 variants and 1,024
combined identified subject/trigger associations per template; at most 32 triggers
per variant. Subject expressions are at most 4 KiB UTF-8, preview text 16 KiB and
each retained body 256 KiB. The fully prepared template, including metadata and
assigned IDs, remains within the existing 1 MiB encoded entity limit. Lower
transport/store limits win. Exceeding any bound rejects atomically, never clips.

Catalog pages default to 50 rows and accept at most 250; scans examine at most
10,000 records and emit at most 4,096 records per request, including sequence
occurrences. A result that exceeds the work budget reports exhaustion; counts
cannot claim completeness. Preserve native DesignGraph limits. PDF requests
allow at most 200 selected variants, 500 output pages, 64 MiB total input artifact
bytes and 32 MiB output, with a 60-second execution budget. Refuse overflow or
timeout with no successful partial download. These are proposed safety ceilings,
not measured scale or latency guarantees.

## Delivery boundary

The first native release requires complete entity/reference lifecycle, shared
artifact/capture evidence, exact sequence targets, bounded authorized catalog
queries, CLI/MCP authoring, inert Studio inspection and bounded PDF export, with
the [connected acceptance evidence](email-catalog-acceptance.md). A static gallery,
generic diagram or mocked provider response does not close this contract.
Sending, scheduling, live provider synchronization, deliverability analytics,
template execution and cross-client rendering parity remain outside this scope.
