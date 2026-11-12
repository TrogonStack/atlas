# Visual product Atlas delivery roadmap

Status: proposed implementation work. This roadmap accompanies the
[visual product Atlas proposal](visual-product-atlas.md). Publishing these
documents does not implement the listed capabilities or settle their wire
shapes. Each unchecked item needs connected verification before completion.

The proposed [state contract](../reference/visual-state-contracts.md),
[artifact integration contract](../reference/visual-artifact-integration.md)
and [email contract](../reference/email-catalog-contracts.md) define semantic
shapes and bounded behavior for review before implementation. The
[visual acceptance requirements](../reference/visual-state-acceptance.md) and
[email acceptance requirements](../reference/email-catalog-acceptance.md)
remain open verification work.

## Start a contribution

Choose a delivery slice below, describe its assumptions and dependencies in the
implementation PR, and keep its scope reviewable. Link the relevant requirement
and the tests that establish its behavior. Coordinate schema and API decisions
before dependent client work; mock data can support design feedback but does
not establish backend support.

Use the [Atlas principles](principles/README.md) and existing native lifecycle
and identity contracts. Public configuration, examples and captures must be
safe to share. Application screenshots used for acceptance should come from
controlled fixtures without credentials or private user data.

The [local Atlas tutorial](../tutorials/run-atlas-locally.md) provides a running
baseline for inspecting the current product before extending it.

## Artifact prerequisite verification

The byte-storage foundation from [PR #344](https://github.com/TrogonStack/atlas/pull/344)
is on main. Visual work should consume it rather than duplicate its upload or
publication logic. Before relying on retained capture evidence:

- [ ] Require file-backed acceptance-receipt storage at artifact startup,
  preserving the existing receipt retention and writer-fencing checks. Add a
  real-NATS regression for a pre-existing memory-backed receipt stream.
- [ ] Make shared backup and retention guidance include object storage, the
  artifact catalog and writer-lease receipt bucket for either byte backend.
  Preserve quiescing and in-flight publication requirements.
- [ ] Verify the artifact workflow through connected CLI/MCP clients, including
  expired grants, lost completion replies, size/digest mismatch and namespace
  authorization. Keep direct S3 HTTP and Atlas gRPC transfer paths explicit.

These tasks close integration prerequisites; they do not establish image safety,
typed attachments or capture freshness. Automatic garbage collection can remain
separate while confirmed files are retained under the
[lifecycle contract](../reference/artifact-lifecycle.md).

## Delivery slices

| Slice | Depends on | Main contribution surface | Completion evidence |
| --- | --- | --- | --- |
| Native visual state | Agreed identity and schema shape | Shared schema/core, backend admission, reference/impact discovery, lifecycle and manifests | Exact connection/scenario selections survive branch and manifest operations; stale or recreated owners cannot inherit attachments. |
| Typed attachment and image verification | Native state and artifact prerequisite | Model reference admission, bounded raster verification and derivative records | Guarded attachment checks both namespaces and expected digest; malformed or oversized images cannot become inline evidence. |
| Capture evidence and baseline | State, attachment and explicit source/profile contracts | Immutable capture/attempt records, acceptance ordering, comparison and guarded baseline operations | Failed attempts preserve earlier captures; freshness, latest attempt, comparison availability and reviewed baseline remain distinct. |
| Visual flow targets and projections | State identity | DesignGraph target union, traversal, exact step resolution and backend projections | Repeated states, equal-endpoint interactions, cycles, branches and subflows retain distinct occurrence identity. |
| Discovery and native clients | Stable state/capture/flow contracts | Backend catalogs and inverse associations, pagination, CLI/MCP | Authorized scoped counts and stable pages preserve completeness and comparison target; public authoring/inspection round trips pass. |
| Studio review surfaces | Backend projections and settled browser delivery | Pages list/grid, Flows map, evidence inspector and accessible navigation | A connected authenticated browser can inspect exact states and occurrences, including missing/unresolved evidence, without authoring model changes directly. |
| Email definitions | Agreed native template and occurrence identity | EmailTemplate variants, source/copy/lifecycle and trigger/timing declarations; core, backend, lifecycle and native clients | Exact variants and trigger selectors survive guarded writes, branches, references and manifest round trips without sending email or evaluating copy. |
| Email sequences and discovery | Email definitions and shared capture contracts | Typed DesignGraph email targets, explicit sequence classification, catalog and inverse occurrence queries | Repeated variants retain distinct step and timing context; authorized search, pagination, counts and sequence badges expose complete scope or explicit incompleteness. |
| Email review and PDF | Email queries, verified imagery and settled delivery | Studio email list/grid, inert previews, sequence inspection and bounded PDF generation | Authenticated inspection and actual PDF bytes preserve frozen input identity, independent statuses and every required input's authority; hostile HTML cannot execute or load remote content. |
| Connected acceptance and scale | Implemented delivery slices | Native fixtures, browser workflows, measured loading/layout/decoding | The complete fixture and failure matrix below pass with retained receipts and explicit unverified limits. |

Presentation prototypes can proceed alongside contract discussion. A prototype
must retain its provisional status until it consumes the accepted backend
contract and passes connected acceptance.

## Decisions to settle before dependent implementation

- [ ] Review the proposed VisualState context, subject, capture and typed
  artifact-reference shapes, then allocate schema fields against current main.
- [ ] Adopt or refine the proposed capture profile, source/build identity,
  required bindings, candidate acceptance ordering and comparison precedence.
  Preserve Missing, Stale, Unknown and latest-attempt distinctions.
- [ ] Implement the proposed guarded baseline selection separately from
  successful capture ingestion, including concurrent edits and replay behavior.
- [ ] Verify the proposed authenticated browser bridge and caching rules,
  including current authority and historical limits for every transitive input.
- [ ] Set byte, dimension, decoded-memory, processing and concurrency limits for
  image verification and thumbnails. Measure rather than borrow limits from
  unrelated workloads.
- [ ] Define query filters, pagination scope and authorized aggregate behavior,
  with explicit incomplete and unavailable results.
- [ ] Review EmailTemplate source/variant/trigger identity and copy/lifecycle
  semantics, then allocate native fields and typed DesignGraph email targets.
- [ ] Settle email-sequence classification and exact trigger selection without
  inferring scheduling from step order or provider metadata.
- [ ] Validate inert preview and frozen-input PDF limits against a measured
  prototype before accepting export performance or safety claims.

Implementers should turn settled decisions into maintained schema/core contracts
and public-seam tests. Keep unresolved decisions visible in review rather than
inventing client-side defaults.

## Acceptance fixture

Use a controlled branching authentication example containing signup,
existing-account login, team-domain and SSO alternatives, wrong-password
feedback, password reset and role-dependent destinations. These are modeled
application states, not a requirement to build Atlas authentication in this work.

- [ ] Browse authorized Screens outside Storyboards and standalone
  connection-attached states, with explicit missing captures and behavioral links.
- [ ] Attach default, submitting, validation-error, rejection and success states
  to one exact UI-to-command occurrence. Preserve separate attachments for an
  equal-endpoint occurrence and after owner/connection recreation.
- [ ] Reuse one state in multiple flows and multiple steps of a flow, preserving
  exact deep links, repeated occurrences, subflows and unresolved targets.
- [ ] Ingest design and runtime evidence with explicit provenance and profiles.
  Check source/model edits, unavailable build targets, profile mismatch, failed
  attempts and concurrent baseline changes independently.
- [ ] Exercise upload, verified completion, image verification, guarded
  attachment and authorized image delivery through real public seams. Cover
  interrupted transfers, delayed retries, failed attachments and retained finals.
- [ ] Prove safe deletion, explicit retargeting, branch preview/landing and
  manifest round trips preserve exact references and refusal behavior.
- [ ] Prove restricted users cannot infer hidden states, flows, captures or
  counts. Recheck artifact access and all transitive input permissions for
  derivatives, caches, exports and later reads.
- [ ] Verify complete empty results differ from truncation, unavailable storage,
  denied access and unknown comparison targets. A loaded viewport is not a
  product-wide count.
- [ ] Verify list/grid and non-canvas navigation with a keyboard and assistive
  technology, and preserve selection when changing filters or layout.
- [ ] Measure the representative hundreds-of-pages/states and dozens-of-flows
  fixture, including incremental loading, layout, image decoding, pan/zoom and
  inspection. Record budgets and observed limits with the implementation.

The [email acceptance fixture](../reference/email-catalog-acceptance.md) extends
this evidence discipline with retained/source-only/undefined copy, dynamic
subjects, provider-account key collisions, repeated reminders with distinct
trigger timing, retired-but-inspectable variants and independent lifecycle and
capture status. It requires actual inert preview/PDF paths, frozen input
revisions and current cross-namespace input authority. Native declarations can
be supplied without a provider connector; delivery, scheduling and live provider
synchronization remain separately scoped capabilities.

## Reporting completion

An implementation PR should identify the behavior now available, its public
entry points, exact verification commands/results and remaining limitations.
Link accepted schema and implementation changes from this roadmap as slices
land. Preserve the proposed label for capabilities without connected acceptance.

Local mocks, generated types, a rendered screenshot or an uploaded file alone
do not establish the complete page/state/flow experience. Backend authoring,
capture acceptance and authenticated Studio inspection need their own evidence.
