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
| Events, commands and queries | **Native analogue.** Native behavioral primitives and selectable Specification messages exist. | **Contract pending.** A message kind alone does not establish a domain Event, local Command or ReadModel. Query messages are requests, not implicit response schemas. |
| Services | **Partial.** Component represents a deployable implementation unit, curated members, directed relationships and communication intent. | Define authored entities, storage access, flows and domain/system memberships explicitly. Do not derive every source association from namespace placement. |
| Systems and containers of resources | **Partial.** Component membership and Component/ExternalSystem relationships exist. | Higher-level system grouping, source system membership, internal/external scope mapping and system-level actor declarations need explicit disposition. ExternalSystem has no members field. |
| Agents | **Native analogue.** Component-only AgentAnnotation retains independently optional model provider/name/version and ordered descriptive tools; presence marks a declared agent. | [Native acceptance](agent-metadata-acceptance.md) covers admission, reference authorization, Source isolation and connected clients. The annotation grants no execution privileges or provider connectivity. |
| Domains and subdomains | **Partial.** Domain, Subdomain, BoundedContext realizations and exact domain-scoped queries exist. | Arbitrary authored domain nesting and shared versioned resource memberships are not equivalent to the native strategic hierarchy or namespace-derived membership. |
| Sends, receives and triggers | **Partial.** Owner-local SEND/RECEIVE intent selects exact Specification messages and descriptive conditions. | Source domain-level claims, field selections, groups, channel bindings and trigger targets still require native contracts. Native declarations do not establish trigger execution or message-to-behavior equivalence. |
| Channels | **Native analogue.** Channel has protocols, address parameters, delivery declarations and bindings to actual observations. | **Contract pending.** Account for source nested/composed channel references, routes and message-pointer parameter bindings. Do not silently substitute observation bindings for every source relationship. |
| Storage containers | **Native analogue.** DataStore retains class, technology, purpose, authority, access mode, classification, residency and retention. | **Contract pending.** Authored service/product read and write claims require explicit native relationships; a ReadModel attachment is not the same claim. |
| Schemas and specifications | **Native analogue.** Specification retains artifacts and selectable schema/message/operation members with explicit mappings and provider usage. | **Contract pending.** Disposition schema variants, defaults, environments and unresolved references in native contracts. |
| Data products | **Native analogue.** DataProduct has ports, contracts, selected access and declared lineage. | **Contract pending.** Disposition input/output target varieties and optional contracts; native ports do not establish identical semantics. |
| Users, teams and ownership | **Native analogue.** Person, Team, membership and exact-subject responsibility exist. | [Directory authority is explicitly bounded](responsibility-contracts.md#scope-and-authority): profiles and pointers are unverified; no principal binding, automatic directory lookup or identity verification is implied. Catalog ownership grants no runtime authorization. |
| ADRs | **Native analogue.** Decision retains subjects, native status, rationale, options, consequences, dates, makers, amendments, replacement and supporting documents. | [Catalog context](catalog-context-contracts.md) preserves historical participation separately from responsibility. Integrated acceptance remains open; native status vocabulary does not imply source status conversion. |
| Business entities and properties | **Partial.** Schema fields, nested shapes, type references and identity annotations exist. | Business identifier roles, optional aggregate-root intent, foreign entity/property selections and business relationship labels need dedicated contracts. Type reuse does not supply those semantics. |
| Ubiquitous language | **Native analogue.** Term and Ambiguity represent scoped vocabulary, definitions and interpretation; Term has optional exact domain ownership. | [Vocabulary ownership](catalog-context-contracts.md#vocabulary-ownership) remains independent of namespace and strategic realization. Complete cross-feature acceptance remains open. |
| Flows | **Partial.** Storyboards retain ordered slices and traces. | Generic typed steps, branching and subflows need topology contracts. Preserve repeated step occurrences, transition labels and scalar/plural next-step forms without inventing executable behavior. |
| Designs and diagrams | **Partial.** Document and retained artifacts can preserve content. | Typed traversal of authored design nodes/edges and `.ecstudio` topology is absent. Rendering heterogeneous diagram formats and preserving layout are separate contract decisions. |
| Pages and resource documentation | **Partial.** Document supports retained text or links, subjects, anchors and navigation. | **Contract pending.** Legacy pages, custom pages, resource docs, categories, sidebars, attachments and authored changelogs have distinct source schemas and associations. Native revision history does not replace authored changelog prose. |
| Examples | **Partial.** Retained artifacts and Documents provide content primitives. | **Contract pending.** Preserve raw examples, usage prose, filenames and nested associations. Arbitrary text is not a typed Event/Command payload or an executed example. |

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
- [ ] Define remaining ADR, vocabulary, documentation, diagram and example fields.

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
| Business data | [Contract](business-data-contracts.md), [acceptance](business-data-acceptance.md) | A local implementation checkpoint covers Schema declarations, property paths, references and Source preservation. Integration, writer-floor verification and complete authenticated lifecycle acceptance remain pending. |
| Flow and design topology | [Contract](design-topology-contracts.md), [acceptance](design-topology-acceptance.md) | An isolated implementation covers schema, core, server and client surfaces. Independent identity/admission review, integration and the remaining Source, restoration and budget acceptance remain pending. |
| Communication and resource associations | [Contract](resource-association-contracts.md), [acceptance](resource-association-acceptance.md) | Isolated implementation is active for message claims, channel/storage relationships and product/Specification associations. Authored Domain claims and independent declarations must not be replaced by derived views. Integration and complete acceptance remain pending. |
| Catalog context | [Contract](catalog-context-contracts.md), [acceptance](catalog-context-acceptance.md) | Calendar-date value implemented in [core](../../rsworkspace/crates/trogon-atlas-core/src/authored_date.rs) with [public tests](../../rsworkspace/crates/trogon-atlas-core/tests/authored_date.rs). Decision dates, makers and amendments, Term domain ownership and Document content/associations are integrated with the metadata backend. The [composed acceptance fixture](../../rsworkspace/crates/trogon-atlas-server/tests/catalog_context_composed.rs) and binary CLI/MCP round trips pass on the integration revision. Independent Decision amendment safety, provenance and branch-landing checks pass, and composed acceptance also passes in the parent checkout. Studio inert text presentation and exact-byte downloads are implemented with component and generated-codec coverage. Remaining Document restoration/pagination cases and broader integrated acceptance are still pending. |
| System structure | [Contract](system-structure-contracts.md), [acceptance](system-structure-acceptance.md) | Isolated implementation is active for abstract groups, authored memberships and typed actor declarations. Integration and full native acceptance remain pending. |

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
