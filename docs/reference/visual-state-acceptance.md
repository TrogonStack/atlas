# Visual state and capture acceptance requirements

Status: proposed acceptance requirements for the
[visual state contract](visual-state-contracts.md), refining the
[visual product proposal](../explanation/visual-product-atlas.md) and
[delivery roadmap](../explanation/visual-product-atlas-roadmap.md). No VisualState
implementation, executed capture test coverage or runtime acceptance is claimed.
A passing artifact upload or schematic Screens renderer does not satisfy these
requirements.

## Native fixture

Create the fixture through authenticated native tools against real persistence.
Logical names are not manifest syntax, predicted UIDs or supplied occurrence
IDs. Retain actual returned identities and etags before dependent writes.

| Fixture | Required content |
| --- | --- |
| Visual contexts | Screens for sign-in, email entry, account creation, account destination, team access and password reset; an additional UI-only modal state and a connection-attached state with absent Screen/UI context. Include a Screen outside every Storyboard and an Unassigned state with no associations. |
| Behavioral model | Real UI, Persona, Command, Event and ReadModel primitives, CommandSlice and UiSlice wiring, exact UI hosting and complete human interaction selections where claimed. |
| Equal endpoints | Distinct slice occurrences connecting the same UI and Command, with separate owner/connection selectors. |
| States | Ready, submitting, invalid input before issuance, rejected after submission, accepted result, loading, empty and modal variants. Include an intentionally uncaptured state. |
| Scenario context | Valid emitted-result and rejection scenarios, a same-named scenario under another slice, a duplicate-ID negative case and a scenario changed after capture. |
| Flow | Authentication DesignGraph with labeled email signup/login alternatives, team-domain and SSO branches, role-dependent destinations, wrong-password feedback and password reset. Include joins, a loop, a generic step and an exact subflow. |
| Repeated target | Same VisualState in distinct steps of one graph and in another graph, with separate persisted step IDs. |
| Profiles | Desktop and mobile, different locale/theme, role and fixture values. Include an explicitly unspecified comparison-relevant value. |
| Captures | Imported design imagery and runtime imagery with explicit producer assertions, successful and failed attempts, verified thumbnail/redacted derivatives and an older selected baseline. |
| Visibility | Separate visible and hidden namespaces with states, captures, flows and artifact records. Use Reader, Writer and restricted principals. |
| Scope | Baseline, an edited branch, retained historical model and same-key recreated owners with different UIDs. |

Use synthetic image fixtures and non-sensitive source/profile identifiers.
Capture producers sanitize bytes before upload. Store credentials only in the
test harness's established secret mechanism; never include them in receipts,
screenshots, profiles or public fixtures.

## State identity and behavioral associations

| Case | Required public outcome |
| --- | --- |
| State create/read | Exact ID, context, title, optional presence, areas and association order round-trip with authoritative UID and etag. |
| Context alternatives | Absent context admits explicitly. Supplied exact Screen or UI context with matching UID admits; an empty supplied selection, multiple alternatives, wrong kind/version or missing UID rejects atomically. |
| UI-only state | State remains discoverable without a Screen, route or Storyboard. |
| Standalone state | Connection-attached and Unassigned states remain discoverable without supplied Screen/UI context. No title, route, imagery or endpoint match synthesizes context or page membership. |
| Page grouping | Screen catalog areas round-trip and filter even without states or Storyboards. Empty areas remain in Unassigned; state areas do not silently change the Screen's page areas. |
| Title or route edit | State remains the same entity; route/title are not used as reference fallback. |
| Equal association targets | Distinct associations retain distinct returned IDs and authored multiplicity through reorder, export and inspection. |
| Exact interaction | States on equal UI/Command endpoint pairs remain attached to their selected slice occurrences. Removing or recreating an occurrence leaves old selectors unresolved. |
| Full human path | A claimed issuance/display relationship retains and validates all hosting, Persona, UI and Command/ReadModel selectors required by native inspection. Matching endpoint labels cannot substitute. |
| Consistency | Conflicting state context, interaction UI, slice or selected scenario outcome rejects. An unavailable dependency is not guessed from imagery. |
| Scenario selection | Exact owner/version/UID, scenario kind/ID and checked content digest resolve. Missing, duplicate, wrong-owner, wrong-kind and altered-content selections do not match. |
| Pre-issuance feedback | Invalid-input state can be documented without an invented Command issuance, rejection Event or lifecycle transition. |
| Limits | Boundary values admit for 128 associations, 32 areas and 1 MiB prepared owner; the next item or byte rejects atomically, including assigned IDs and system metadata. Lower storage/transport limits still win. |
| Unknown alternatives | Unknown or conflicting JSON/protobuf union alternatives reject without being dropped or coerced into a supported state. |

## Artifact and capture ingestion

Use the implemented [artifact service](../how-to/manage-artifacts.md) as the
byte-storage foundation. Its declared media metadata is not proof that visual
admission decoded an image. Raster verification and typed attachment remain
proposed integration work.

| Case | Required public outcome |
| --- | --- |
| Real upload path | A local client transfers a synthetic image through native upload/finalize controls and real object storage, then attaches the immutable returned artifact identity. Verify NATS gRPC transfer and direct S3 presigned HTTP separately; no NATS provider HTTP image URL is invented. A remote MCP server cannot read a caller-local path. |
| Actual image verification | Valid PNG, JPEG or WebP bytes produce checked dimensions, media type, size and digest matching required `expected_sha256`. Declared image media type with nonimage, malformed or unsupported bytes rejects. |
| Bounded decode | Oversized dimensions, decoded pixels, work/memory budget exhaustion and decompression attacks reject before unsafe allocation; no capture receipt is published. |
| Unsupported active content | Script-capable or nonallowlisted content never enters the image rendering path or becomes an executable preview. |
| Immutable object | Delayed writes, retries and duplicate finalize requests cannot overwrite another attempt or an accepted object. Same digest does not bypass object identity or authorization. |
| Interrupted upload | Partial or unaccepted bytes cannot attach successfully; expired transfer authority cannot continue an unfinished upload. No Ready or success receipt is fabricated. |
| Accepted object after expiry | An already accepted Ready artifact remains attachable after its original upload session or grant expires, subject to current authority, digest/image verification and model guards. Transfer expiry cannot become final-object expiry. |
| Attachment guards | State/context/subject change between attempt and attachment refuses stale publication. Ready bytes remain recoverable under artifact policy without retagging old evidence as new. |
| Attachment versus collection | Coordinate attachment holds with deletion claims. A collector cannot remove bytes accepted by an in-flight or newly committed attachment. Exercise the actual storage/receipt seam. |
| Replay | Exact repeat returns the accepted receipt. Changed input under the same operation identity refuses; concurrent accepted receipts cannot overwrite each other. |
| Derivative | Verified thumbnail or redacted copy retains exact parent reference and its own immutable identity; original bytes/digest are unchanged. |
| Producer provenance | Source build/time assertions stay labeled as assertions unless separately verified. Imported design imagery is never reported as runtime execution. |
| Profile/provenance ceilings | Reject more than 128 checked bindings or more than 16 KiB of profile/provenance without dropping fields. Discovery reports actual image/decode admission bounds. |

## Evidence, attempts and baseline

| Case | Required public outcome |
| --- | --- |
| No capture | Eligible state/profile reports Missing, with no fabricated successful attempt or image. |
| Exact match | Known matching binding set, source target, profile and age policy produce Current with explainable comparison facts. |
| Candidate ordering | A durable server acceptance sequence selects the newest success in the exact subject/scope/profile/capture-context partition. Forged producer timestamps and changed storage iteration order cannot change the selection. |
| No freshness fallback | A newer mismatching success remains selected and Stale even when an older successful capture matches the target. Explicit inspection can select that older receipt. |
| Binding changed | Context/subject incarnation or etag change, altered scenario digest or composition/path change produces Stale under the selected policy. |
| Profile differs | Mobile, locale, theme, role, fixture and device-scale differences cannot reuse a desktop or other-profile Current result. |
| Unknown target | Absent exact source build or required provenance produces Unknown, even for recent bytes. |
| Status precedence | Known mismatch plus unknown provenance is Stale; otherwise unknown required facts are Unknown. Required verified provenance cannot be satisfied by a producer assertion. |
| Age policy | Explicit expiry changes status to Stale; policy identity is part of the comparison and pagination context. |
| New failed attempt | Retain older successful image and its independent freshness; show failed latest attempt and diagnostic. |
| Concurrent attempt order | Start A, start B, accept B, then fail A. Latest attempt remains B succeeded, selected success remains B and A's terminal failure stays in history. Server creation sequences survive replay; producer time and terminal arrival order cannot make A latest. |
| Abandoned attempt | Bounded lease expiry becomes an explicit terminal outcome rather than permanent in-progress status. |
| Baseline selection | Guarded selection pins exact successful receipt/profile, actor and reason. New success does not implicitly accept itself. |
| Baseline comparison | No selected baseline is Unreviewed; comparable equal/different bytes are Unchanged/Changed; unavailable inputs are Unavailable. Selection and comparison remain distinct. |
| Changed bytes | Different digest reports changed bytes relative to selected baseline without claiming perceptual quality or runtime correctness. |
| Independent axes | A Current changed capture and a Stale accepted historical baseline remain expressible without a combined misleading health flag. |
| Operational failure | Store, timeout, decode and authorization-service failures remain explicit errors, never Missing, empty success or freshness. |

## Flow projection and occurrence identity

| Case | Required public outcome |
| --- | --- |
| Typed target | New exact VisualState target resolves through backend detail/traversal and codecs. A slug in custom properties is not accepted as equivalent. |
| Repeated state | Distinct graph step IDs remain distinct nodes and deep links, even with equal state targets and image bytes. |
| Branches and joins | Authored labels, outgoing multiplicity, joins, step order and transition IDs survive projection. |
| Step and label edits | Existing step-edit and transition occurrence rules apply; a changed target cannot display the previous target's image. |
| Loop and subflow | Preserve existing depth, cycle and unavailable markers. A cycle is authored topology and does not become a backward Storyboard execution edge. |
| Generic or unresolved step | Generic step, unavailable state and missing capture remain navigable and distinguishable without fabricated imagery. |
| Reverse discovery | State, context, connection and containing graph inspection expose the same explicit associations with returned owner/occurrence identities. |
| Storyboard control | Storyboard exact order and repeated slice/trace occurrences remain unchanged. No graph image sequence establishes execution. |

## Authorization, pagination and lifecycle

| Case | Required public outcome |
| --- | --- |
| Reader/Writer boundary | Readers inspect authorized data; mutation refuses without required write authority. Profile role labels do not grant rights. |
| Hidden parity | Hidden and missing selections share generic unavailable resolution. No hidden title, UID, dimensions, thumbnail, digest or existence leaks. |
| Delivery authority | Original, thumbnail, download and export check current authority; Studio uses authenticated same-origin delivery. Prior read or captured receipt cannot grant permanent access. |
| Scoped counts | Counts include only the same authorized filtered scope/profile. A hidden namespace cannot affect exposed totals or freshness counts. |
| Catalog completeness | Include Screens outside Storyboards, UI-only states and standalone connection-attached or Unassigned states. Complete empty, partial, unavailable and failed results remain different. |
| Pagination | Default 50 and maximum 250 records; larger requests reject. Repeated requests under unchanged revision preserve order with no omissions or duplicates. |
| Bound tokens | Changed principal scope, branch/history target, filter, profile, source target, capture context, policy or revision refuses a prior token. Expired or malformed tokens cannot restart silently. |
| Query bounds | Exercise 10,000 examined and 4,096 emitted ceilings, as well as lower applicable limits. Unsupported partial output returns an explicit error. |
| Partial totals | Exhausted count evaluation cannot label a partial total as exact. No client fetch cap is reported as a product total. |
| Safe deletion | Complete reference inventory blocks deletion of referenced state/context or required artifact. Force deletion preserves unresolved retained selections. |
| Owner recreation | Same key with a new UID cannot inherit UID-bound context/scenario/capture/flow selections. Plain CatalogSubject entity refs retain native exact-key resolution; replacement changes checked UID/etag and cannot make old evidence Current. |
| Branch preview/apply/land | Preserve IDs under existing branch rules; recheck endpoint UIDs, artifacts and guards. Baseline does not accept a branch capture merely because the branch lands. |
| Branch removal and retention | Historical/baseline pins and other branches keep required artifacts retained; deletion does not collect them early. |
| Export/reapply | Same-store native cycle preserves IDs, optional presence, order, digests, immutable artifact selectors and capture/baseline records. Incomplete or unavailable material cannot pose as a complete reapply bundle. |
| Supplied-source isolation | Source create/whole-owner/field/clear selections of VisualState or new grouping/target inventories reject until explicitly supported. Unrelated allowed Screen field updates preserve catalog areas; current or incoming native grouping prevents Source whole-owner replacement. Capture/attempt/baseline records never become inferred Source-owned fields. Native export/diff/reapply still round-trips the supported records. |
| Visual snapshot | Page/flow export preserves frozen exact input/capture/step identities, scope/profile/target and visible missing-evidence markers. A same-key live change cannot replace an input mid-render. |
| Derived export authority | Revoke one required cross-namespace input before release or a later persisted snapshot/thumbnail read. Access to the output namespace alone cannot deliver those derived bytes. |
| Historical view | Compare only to selected historical target when supported; otherwise expose retained declarations and unavailable comparison without live substitution. |
| Recovery window | Verify explicit pins, last-reference timing, recovery window and cleanup coordination before enabling collection of confirmed artifacts. |

## Reference-sized Studio fixture

Maintain a generated native fixture with 227 Screens, 471 VisualStates and 23
DesignGraphs, reflecting the scale of the supplied product references. Include
all states from the branching authentication fixture, repeated state steps,
uncaptured states, unavailable targets and mixed evidence statuses. Record exact
fixture generator version and returned identities; totals apply to an
unrestricted fixture principal and change under restricted scopes.

Verify through authenticated Studio connected to the real backend and storage:

- Pages search, explicit area/flow/context/profile/status filters, pagination,
  scoped counts, state inspection and exact branch/history deep links.
- Focused and overview Flows with actual verified thumbnails, branch labels,
  fit all/selection, pan, pinch/control-scroll zoom and incoming/outgoing
  navigation that retains persisted step identity.
- Full keyboard inspection and a readable non-canvas flow listing; visible
  loading, error, unavailable, missing-capture and incomplete states.
- Metadata loading before imagery, bounded thumbnail fetching near the viewport
  and on-demand full-resolution retrieval. Record request count, load latency
  and peak memory on a named machine/browser with a cold and warm cache.
- Freshness, latest attempt and baseline review displayed as separate facts.
  Switching branch/profile/scope discards stale requests and image selections.
- Model inspection and intent communication remain read-only; capture execution
  and authoritative mutations use authenticated native tools.

Performance thresholds must be stated before the acceptance run and evaluated
against measured receipts. Fixture size, a screenshot or successful initial
render alone is not a performance claim. Email trigger/source/timing/variant
and sequence browsing require separate native contracts and acceptance.

## Required evidence and current boundary

| Layer | Receipt required to close it |
| --- | --- |
| Contract and schema | Checked field/union allocation, complete reference inventory and generated codecs with unknown/presence behavior. |
| Core and persistence | Public contract regressions and real Store lifecycle, image admission, attachment and race coverage. |
| Connected tools | Authenticated client, CLI and MCP upload/author/inspect/preview/apply/export paths through production transport. |
| Browser | Authenticated Studio against the native fixture and real image delivery, with accessible navigation and reference-sized measurements. |
| Retention | Demonstrated attachment/deletion coordination and branch/history holds before collection is enabled. |

Retain exact source revision, commands, test exits, fixture and artifact digests,
authorization scope and query completeness with each receipt. A discovered bug
needs a meaningful failing public-seam regression before its repair. Static
review, schema lint, mocked component tests and queued checks remain their own
evidence categories.

All rows are currently requirements. Main has Screen/UI, ConnectionRef,
DesignGraph and checked human interaction inspection, but not the native
VisualState/capture surfaces described here. Artifact infrastructure is
implemented on main; it does not by itself provide raster verification, typed
model attachment accounting, authenticated Studio image delivery or this
acceptance.
