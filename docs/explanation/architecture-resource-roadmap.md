# Architecture resources

Status: native transport, storage, Specification, DataProduct, responsibility,
Decision and Document resources are implemented, alongside supplied-bundle
source reconciliation. The [coverage reference](../reference/architecture-resource-coverage.md)
links their field and connection contracts and separates implementation from
acceptance evidence. The remaining design scope is described below.

The [native coverage reference](../reference/eventcatalog-coverage.md) tracks
implemented Atlas objects, fields and relationships. Native product work comes
first. EventCatalog-specific integration is deferred to the final mile, after
the native capabilities and workflows are complete and verified. Readers,
importers, migration, sync, federation and format conversion are not active
roadmap items or prerequisites. Revisit them only in a separately scoped task.

## Start with the behavioral model

Architecture resources describe the same model's transport, persistence,
contracts and responsibility. They build on Event Modeling's existing causal
structure. Integration input is a ReadModel. An AutomationSlice observes that
ReadModel through a Processor, which can issue a local Command. A Channel
describes the selected observation. A DataStore describes storage attached to
an exact ReadModel.

Keeping those anchors explicit prevents a transport label from introducing an
additional behavioral grammar. Broker delivery claims do not establish
end-to-end processing guarantees, and a storage attachment does not establish
which component reads or writes the store.

## Connection identity is the implemented foundation

[Connection identity](../reference/connection-identity.md) already defines
persisted IDs on explicit typed edges and navigation/lifecycle occurrences.
An exact owner reference, its incarnation UID and an occurrence ID identify a
connection. Reorder preserves identity; retargeting creates a new occurrence.
Ordinary writes use bounded active state without a deleted-ID ledger.

That foundation does not identify every reference in the model. Raw reference
lists, repeated storyboard moments, additional architecture associations and
the future linter host's reference adapter still need their own supported
representations.
Implementing a new association requires extending reference discovery,
validation and lifecycle handling together.

## Implemented transport and storage resources

Channel and DataStore include typed values, identified associations and nested
reference handling. The acceptance fixture covers repeated observations, exact
versions, reference discovery, guarded mutations and branch behavior.

Runtime evidence comes from the linked executable tests. Component review
coverage does not establish deployed-browser or production readiness.

## Implemented Specification and API resources

The [Specification/API contract](../reference/specification-and-api-contracts.md)
defines native retained artifacts, selectable members and identified
associations. Backend admission and exact reference resolution preserve separate
request/response mappings and explicit provider usage. Native manifest, CLI/MCP
and Studio review surfaces preserve these selections. The
[acceptance reference](../reference/specification-and-api-acceptance.md) links
the executable fixture and identity, authorization and lifecycle regressions.

Retained opaque formats and selectors do not imply semantic format validation,
a source connector or another format compiler. Deployment and production-scale
evidence remain distinct from the linked local tests.

## DataProduct resource slice

The [DataProduct contract](../reference/data-product-contracts.md)
defines input and output ports, exact output contracts, selected access paths
and declared lineage. Its [acceptance criteria](../reference/data-product-acceptance.md)
connect those declarations to existing ReadModels, Channel observations,
DataStore attachments and Specification response messages.

Native schema, runtime admission, CLI/MCP authoring and inspection, and Studio
review are implemented. The linked acceptance reference identifies executable
fixtures and their limits. Port membership and access declarations do not
introduce producing behavior. Backend inspection reports checked model paths
separately from declared lineage and runtime execution evidence.

## Implemented responsibility resources

The [responsibility contract](../reference/responsibility-contracts.md)
defines Team and Person directory records, identified membership and explicit
responsibility for exact native subjects. Its
[acceptance criteria](../reference/responsibility-acceptance.md) connect these
declarations to entities, connection occurrences, Specification members and
DataProduct ports. Team membership and catalog responsibility grant no
authorization and introduce no producing behavior.

The first native scope uses exact versions and existing incarnation selectors;
it does not silently extend responsibility to a version family or every member
of a resource. Retained directory information and its history remain distinct
from authentication identities and external directory synchronization. Native
schema, backend admission and typed queries, CLI/MCP authoring and Studio review
preserve these exact selections. The acceptance reference links executable
coverage and records its deployment and acceptance limits.

## Decision and Document contracts

The [Decision and Document contract](../reference/decision-and-document-contracts.md)
defines how rationale and supporting material select exact model subjects.
Its [acceptance requirements](../reference/decision-and-document-acceptance.md)
extend the connected fixture with artifact, navigation, permission and lifecycle
cases. Native schema, exact subject admission, retained content and anchor
selection, typed queries, CLI/MCP authoring and Studio review are implemented.
The acceptance reference distinguishes executable coverage from remaining
exhaustive and deployed-browser evidence.

## Source integration and format support

Source integration uses supplied immutable bundles and explicit native
ownership. Native Channel and DataStore support remains independent of source
connectors and format-specific compatibility checking.

Native manifest export already exists. The
[supplied bundle profile](../reference/source-bundle-profile.md) implements strict
byte intake and deterministic digests in Rust core. The source control backend
implements retained snapshots, mappings, overlays, guarded reconciliation and
durable recovery. The acceptance reference records executable coverage and its
limits.

The [source integration contract](../reference/source-integration-contracts.md)
defines a native-first boundary for immutable supplied bundles, explicit
identity and version mappings, provenance and guarded reconciliation. Its
[acceptance requirements](../reference/source-integration-acceptance.md) distinguish
same-store reapply from migration into a new store, where incarnation-bound
selectors require explicit rebinding. The
[source control API](../reference/source-control-api.md) describes the concrete
typed boundary and read-only review surface.
Remote connectors and additional format compilers remain separate milestones.

## Native system relationship layer

The [system relationship contract](../reference/system-relationship-contracts.md)
builds on existing Component and ExternalSystem boundaries. Its
[acceptance requirements](../reference/system-relationship-acceptance.md)
keep authored declarations distinct from checked behavioral paths and runtime
evidence. Native schema, admission, exact occurrence identity, authoring and
inspection are implemented. The acceptance reference links executable coverage
without claiming exhaustive or deployed acceptance. Richer grouping and
actor-interaction boundaries remain explicit in the contract.

## Native communication intent layer

The [communication intent contract](../reference/communication-intent-contracts.md)
selects a declaration layer for exact Specification messages on existing
implementation/external boundaries. SEND and RECEIVE record the declaring
owner's intent; they do not assert the peer's agreement or create a behavioral
path. Optional condition prose records context without another trigger grammar.
Executable behavior still follows ReadModel observation through a Processor
and its optional Command.

The [acceptance requirements](../reference/communication-intent-acceptance.md)
cover exact member selection, occurrence lifecycle, owner-relative authority,
reference provenance, shared work bounds and native-only Source isolation.
Native schema, core/backend admission, CLI/MCP authoring and read-only Studio
inspection are implemented. The acceptance inventory separates executable
coverage from exhaustive or deployed evidence. Verified behavioral binding,
predicate execution and transport-message equivalence to native Commands/Events
require separate contracts.

## Native human interaction inspection

The [human interaction contract](../reference/human-interaction-contracts.md)
selects identified Component declarations that a UI is presented by that
implementation boundary. This is separate from raw membership and does not
claim exclusive runtime deployment or ownership of a Command handler.

A supplied exact hosting selection and same-slice edge tuple support a
checked human issuance or display path. Persona identity alone cannot select
the complete path. Display checks select an individual ReadModel observation,
preserving repeated equal observations. The
[acceptance requirements](../reference/human-interaction-acceptance.md) define
identity, role, visibility and lifecycle obligations.

Native hosting and checked interaction inspection have backend, connected-client
and authenticated browser acceptance under the linked contract. Exhaustive
actor discovery, persisted composite interaction subjects and ExternalSystem
hosting remain deferred. Checked per-owner revisions describe a read set, not
an atomic multi-owner snapshot or evidence of a human's runtime permissions.

## Atlas Linter comes after the architecture resources

The [remaining contract boundaries](../reference/architecture-resource-coverage.md#remaining-contract-boundaries)
distinguish existing strategic references and Component membership from richer
system/service relationships, behavioral communication binding, agent metadata,
business-data relationships and arbitrary design topology. These require
explicit scope decisions. Their absence does not make the implemented native
resource slices proposals again. Connected acceptance must record which
supported relationships and client workflows were actually exercised.

Atlas Linter is deferred until the remaining architecture resources and source
integration concerns are defined. Its rules can inspect model consistency,
ownership, documentation, naming, contract compatibility and architectural
constraints. It is not limited to architecture-specific checks.

The terminology is Lint Module for a WebAssembly component, Lint Rule for an
individual check, Finding for a reported issue and Lint Run for an execution.
Module remains the broader extension concept; future importers, exporters or
transformations would need their own contracts. No generic module registry or
module type is implemented by this naming decision.

Lint modules must use the WebAssembly Component Model and WIT. The detailed
host interface and runtime remain deferred, including authorized consistent
snapshot access, lossless entity and exact connection subjects, component and
configuration storage, registration, run/cancel/report operations, retention,
budget profiles and CI failure policy. The connection-ID foundation alone does
not provide that host or its adapter.
