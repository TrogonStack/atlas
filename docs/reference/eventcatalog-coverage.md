# EventCatalog coverage and gaps

This is the consolidated comparison of EventCatalog concepts with Atlas native
contracts. It replaces historical missing-resource checklists for status
tracking. The scope is missing Atlas objects, fields and relationships inspired
by EventCatalog. Reading or migrating EventCatalog projects is outside this scope.

## Comparison baseline

EventCatalog source inventory: commit
[`2736a2272207cf1b1ee09e6a440401d2215c6380`](https://github.com/event-catalog/eventcatalog/tree/2736a2272207cf1b1ee09e6a440401d2215c6380),
audited on 2026-10-16. The
[collection definitions](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts)
anchor the resource and field inventory. This is a pinned comparison, not a
claim about all features in the latest EventCatalog release.

Atlas status was reconciled on 2026-10-28 against the native schemas and linked
contracts. [Architecture resource coverage](architecture-resource-coverage.md)
remains authoritative for native implementation and acceptance boundaries.

Status terms:

- **Native analogue**: implemented Atlas representation, which may have different
  semantics from the corresponding EventCatalog concept.
- **Partial**: relevant native support exists, but the stated source relationship
  or field semantics remain uncovered.
- **Gap**: a dedicated native contract is absent.
- **Contract pending**: the corresponding concept needs an explicit native
  representation or exclusion; file conversion is not required.

None of these statuses promises full product parity or authorizes a new entity
kind. Existing representations and proposed extensions remain distinct.

## Resource and relationship inventory

| EventCatalog concept | Atlas status and existing support | Remaining EventCatalog coverage |
| --- | --- | --- |
| Events, commands and queries | **Native analogue.** Native behavioral primitives and selectable Specification messages exist. | Exact Specification MESSAGE selectors preserve catalog messages independently of behavioral primitives. A message kind does not establish a domain Event, local Command or ReadModel. Query messages remain requests, not implicit response schemas. |
| Services | **Partial.** Component represents a deployable implementation unit, curated members, directed relationships and communication intent. | Complete storage-access and authored domain/system membership integration and acceptance under the adopted native contracts. Namespace placement does not supply authored membership or flow behavior. |
| Systems and containers of resources | **Native analogue.** SystemGroup retains abstract grouping, descriptive scope, typed members and actors alongside Component membership and boundary relationships. | The [SystemGroup contract](system-structure-contracts.md) is integrated with typed DesignGraph targets. Final acceptance follow-ups remain under review. ExternalSystem retains no members field. |
| Agents | **Native analogue.** Component-only AgentAnnotation retains independently optional model provider/name/version and ordered descriptive tools; presence marks a declared agent. | [Native acceptance](agent-metadata-acceptance.md) covers admission, reference authorization, Source isolation and connected clients. The annotation grants no execution privileges or provider connectivity. |
| Domains and subdomains | **Native analogue.** Domain, Subdomain, BoundedContext realizations, exact domain-scoped queries and authored Domain memberships exist. | Explicit domain nesting and shared versioned membership remain separate from strategic realization and namespace scope. Their [SystemGroup acceptance](system-structure-acceptance.md) is under final integration review. |
| Sends, receives and triggers | **Native analogue.** Component, ExternalSystem and Domain carry independent SEND/RECEIVE claims, routes, triggers and field selections alongside peer-directed communication intent. | The [resource association contract](resource-association-contracts.md) preserves separate authorities and descriptive grouping. Complete identity, boundary and connected-client acceptance remains open; declarations establish no trigger execution or message-to-behavior equivalence. |
| Channels | **Native analogue.** Channel has protocols, address parameters, delivery declarations and bindings to actual observations. | Claim routes and parameter values are integrated separately from observation and operation bindings. Channel composition is adopted in the [resource association contract](resource-association-contracts.md), with its implementation and corrections still isolated. Complete acceptance remains open. |
| Storage containers | **Native analogue.** DataStore retains class, technology, purpose, authority, access mode, classification, residency and retention. | The [resource association contract](resource-association-contracts.md) adopts explicit boundary/product storage-access claims; its implementation and corrections remain isolated. A ReadModel attachment remains distinct from an authored access claim. |
| Schemas and specifications | **Native analogue.** Specification retains artifacts and selectable schema/message/operation members with explicit mappings and provider usage. | SchemaBinding environments and optional default declarations are integrated. Exact variant boundaries, identity and Source acceptance remain under review. Unavailable declarations stay distinct from environment resolution or deployment verification. |
| Data products | **Native analogue.** DataProduct has ports, contracts, selected access and declared lineage. | Native resource associations retain target varieties, direction and optional contract selectors independently of behavioral ports. Complete direction, identity and connected-client acceptance remains open; no producing path is inferred. |
| Users, teams and ownership | **Native analogue.** Person, Team, membership and exact-subject responsibility exist. | [Directory authority is explicitly bounded](responsibility-contracts.md#scope-and-authority): profiles and pointers are unverified; no principal binding, automatic directory lookup or identity verification is implied. Catalog ownership grants no runtime authorization. |
| ADRs | **Native analogue.** Decision retains subjects, native status, rationale, options, consequences, dates, makers, amendments, replacement and supporting documents. | [Catalog context](catalog-context-contracts.md) preserves historical participation separately from responsibility. [Integrated native acceptance](catalog-context-acceptance.md#integrated-verification-receipt) passes; native status vocabulary does not imply source status conversion. |
| Business entities and properties | **Native analogue.** Addressable Schema carries optional aggregate-root intent, identified property-path declarations and exact entity/property relationships. | The [business data contract](business-data-contracts.md) separates declarations from type reuse, payload schemas and stream behavior. Remaining identifier, relationship and client acceptance stays open. |
| Ubiquitous language | **Native analogue.** Term and Ambiguity represent scoped vocabulary, definitions and interpretation; Term has optional exact domain ownership. | [Vocabulary ownership](catalog-context-contracts.md#vocabulary-ownership) remains independent of namespace and strategic realization. [Integrated native acceptance](catalog-context-acceptance.md#integrated-verification-receipt) passes for this ownership boundary. |
| Flows | **Native analogue.** DesignGraph retains identified typed steps, labeled transitions, branching, cycles and exact subflow references. Storyboards retain their ordered slices and traces. | The [topology contract](design-topology-contracts.md) keeps authored graph traversal separate from execution. SystemGroup targets are composed. Remaining target, lifecycle, traversal and connected-client acceptance is under review. |
| Designs and diagrams | **Native analogue.** DesignGraph provides authored topology and an optional Document layout reference; Document retains exact content. | Native topology acceptance remains open. Heterogeneous rendering, layout interpretation and reading `.ecstudio` files are excluded from this native coverage goal. |
| Pages and resource documentation | **Native analogue.** Document retains text, links or exact bytes, purpose, subjects, categories, anchors, ordered navigation and typed related content. | [Catalog context](catalog-context-contracts.md) preserves native associations without filesystem inference. Integrated acceptance covers authored changelog revision independence, exact retained bytes, collection limits and typed associations. Revision history does not replace prose. |
| Examples | **Native analogue.** EXAMPLE Documents retain raw bytes, media type, digest, filename, usage prose and exact subjects. | The composed raw-JSON example, selected MESSAGE and containing-Document case passes authenticated native acceptance. Arbitrary text is not a validated Event/Command payload or an executed example. |

## Remaining model checklist

These are concept-level gaps, not a prescribed Atlas schema. Each item needs
an explicit native contract or exclusion. Adopted concepts also require the
implementation and executable acceptance described by that contract before
the item can be closed.

- [ ] Define higher-level system grouping, authored domain/system/service
  memberships and system-level actor relationships.
- [x] Complete agent model and tool metadata across native admission, Source
  isolation and connected-client acceptance.
- [ ] Define business identifiers, aggregate intent and entity/property
  relationships.
- [ ] Define flow step and transition topology, branching and subflows; disposition
  authored design topology separately from visual layout.
- [ ] Define communication relationships for domain claims, triggers, fields, groups
  and channel bindings without inferring execution.
- [ ] Complete channel composition, storage access and DataProduct relationships.
- [x] Define remaining shared metadata, including lifecycle dates, draft messages,
  badges, resource groups and reference semantics.
- [x] Define remaining ADR, vocabulary, documentation, diagram and example fields.

The tables retain already implemented analogues so these items are not
mistaken for missing Channel, DataStore, Specification, DataProduct, Team,
Person, Decision or Document kinds.

## Delivery status

Implementation work is active. Each completed entry is supported by its native
contract and acceptance evidence. Proposed or isolated work does not close the
remaining entries.

| Workstream | Contract and acceptance | Verified boundary and remaining work |
| --- | --- | --- |
| Agents and shared metadata | [Contract](agent-metadata-contracts.md), [acceptance](agent-metadata-acceptance.md) | **Complete within the native contract.** Parent verification and requirement review cover schema/core admission, nested scope, exact limits, references and provenance, identity and branch lifecycle, authorization, Source isolation, tenant descriptors, CLI/MCP roundtrips and Studio codec/component inspection. The public aggregate boundary includes stored occurrence IDs and verifies atomic refusal with unchanged reference indexes. No deployed-browser or execution capability is claimed. |
| Business data | [Contract](business-data-contracts.md), [acceptance](business-data-acceptance.md) | Schema declarations, property paths, checked references, Source preservation and writer revision 19 are integrated. The [authenticated order fixture](../../rsworkspace/crates/trogon-atlas-server/tests/business_data.rs) verifies explicit aggregate intent and payload isolation. The remaining identifier, relationship and connected-client acceptance matrix is under review; the checklist stays open. |
| Flow and design topology | [Contract](design-topology-contracts.md), [acceptance](design-topology-acceptance.md) | DesignGraph schema, core, server and connected-client surfaces are integrated, including reviewed occurrence identity and bounded incoming-subflow scans. Native Source and branch fixtures and the prepared-owner capacity check are implemented. Typed SystemGroup targets are composed. Public target, traversal, identity and codec assertions are implemented; review identified remaining reference, incarnation, Storyboard trace and native-client proof cases, now assigned for completion. |
| Communication and resource associations | [Contract](resource-association-contracts.md), [acceptance](resource-association-acceptance.md) | Domain, Component and ExternalSystem message claims and product/Specification associations are integrated. [Channel discovery](../../rsworkspace/crates/trogon-atlas-server/src/channel_messages.rs) retains independent Specification, claim-route and operation-binding authorities. [Source variant acceptance](../../rsworkspace/crates/trogon-atlas-server/tests/schema_variant_source_backend.rs) protects native environments and default presence from enclosing-field ownership. Channel/storage corrections and their connected-client acceptance remain in an isolated checkout; the requirement audit identified remaining identity, boundary and connected-client assertions, now assigned for acceptance. |
| Catalog context | [Contract](catalog-context-contracts.md), [acceptance](catalog-context-acceptance.md#integrated-verification-receipt) | **Complete within the native contract.** Decision dates/makers/amendments, Term ownership and Document purpose/categories/content/navigation/related associations have integrated identity, reference, authorization, lifecycle, Source, budget and connected-client acceptance. The authenticated boundary fixtures prove changelog independence, retained-byte limits, exact nested MESSAGE examples and atomic shared-write refusal. Fresh parent execution passed 155 native tests and 99 Studio codec/component tests. Typed DesignGraph topology remains a separate workstream; no parser, active rendering or execution is claimed. |
| System structure | [Contract](system-structure-contracts.md), [acceptance](system-structure-acceptance.md) | Abstract groups, authored memberships, typed actors and real DesignGraph targets are integrated. Retained-dangling and authorization-error repairs, Source/branch/budget fixtures and composed checks pass in the frozen integration receipt. Additional concurrency, privacy, branch and client-diff assertions returned from the owner and await final composed verification. |

Native value tests, Studio component tests, generated codec tests and authenticated
backend fixtures prove different boundaries. Completion requires evidence for
each requirement's actual schema, mutation, reference and client behavior.

## Atlas adaptations and separate roadmap

The following are Atlas choices, not direct EventCatalog schema requirements:

| Atlas choice | Relationship to the comparison |
| --- | --- |
| Exact numeric identities, owner incarnation UIDs and occurrence selectors | Native foundations for checked references and mutation safety. |
| [Component UI hosting and InspectHumanInteraction](human-interaction-contracts.md) | Proposed Atlas representation of human interaction paths; not implemented and not a direct copy of EventCatalog actor arrows. |
| Typed behavioral binding and executable communication predicates | Separate behavioral work. Source trigger declarations and condition prose do not prove runtime execution. |
| Portable selector rebinding | A migration concern for incarnation-bound native selections; not a source collection or a prerequisite for preserving original bytes. |
| WASM/WIT linter host | Atlas extension architecture, separate from EventCatalog resource coverage and EventCatalog's own lint tooling. |

The [architecture roadmap](../explanation/architecture-resource-roadmap.md)
tracks these choices. Completing this comparison does not automatically adopt
them.

## Updating the checklist

Close an item only with a linked disposition and evidence for the claimed
boundary: native schema, validation, reference discovery and query support.
Excluded concepts must remain visible in this inventory. A passing resource
fixture does not establish every proposed relationship.

When updating the EventCatalog baseline, compare the pinned source inventory
with the new revision before changing statuses. Keep source-derived gaps separate
from newly proposed Atlas capabilities.
