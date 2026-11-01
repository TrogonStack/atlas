# Native Atlas coverage

This reference tracks native Atlas objects, fields, relationships and their
acceptance evidence. Native contracts determine the implementation scope.

## Native-first scope

Build and verify Atlas independently before considering external catalog
integration. EventCatalog-specific work is deferred to the final mile, after
the native capabilities and workflows are complete and verified. It is not
active backlog, a completion requirement or a prerequisite for native work.

This deferral includes readers, importers, migration, synchronization, federation,
source-format conversion, `.ecstudio` parsing and EventCatalog-specific selector
rebinding. Any future integration needs a separately scoped task. Do not report
these deferred items as missing native capabilities or next steps.

The historical comparison at the end records research provenance only. It does
not define an integration contract or require source-format parity.

Atlas status was reconciled on 2026-10-29 against the native schemas and linked
contracts. [Architecture resource coverage](architecture-resource-coverage.md)
remains authoritative for native implementation and acceptance boundaries.

Status terms:

- **Native analogue**: implemented Atlas representation, which may have different
  semantics from superficially similar external catalog concepts.
- **Partial**: relevant native support exists, but the stated source relationship
  or field semantics remain uncovered.
- **Gap**: a dedicated native contract is absent.
- **Contract pending**: the corresponding concept needs an explicit native
  representation or exclusion; file conversion is not required.

None of these statuses promises full product parity or authorizes a new entity
kind. Existing representations and proposed extensions remain distinct.

## Native resource and relationship inventory

| Concept | Atlas status and existing support | Native boundary |
| --- | --- | --- |
| Events, commands and queries | **Native analogue.** Native behavioral primitives and selectable Specification messages exist. | Exact Specification MESSAGE selectors preserve catalog messages independently of behavioral primitives. A message kind does not establish a domain Event, local Command or ReadModel. Query messages remain requests, not implicit response schemas. |
| Services | **Native analogue.** Component represents a deployable implementation unit, curated members, directed relationships, communication intent and storage-access claims. | Storage-access and authored domain/system membership pass integrated acceptance under the adopted native contracts. Namespace placement does not supply authored membership or flow behavior. |
| Systems and containers of resources | **Native analogue.** SystemGroup retains abstract grouping, descriptive scope, typed members and actors alongside Component membership and boundary relationships. | The [SystemGroup contract](system-structure-contracts.md) is integrated with typed DesignGraph targets. [Integrated acceptance](#integrated-verification) passes. ExternalSystem retains no members field. |
| Agents | **Native analogue.** Component-only AgentAnnotation retains independently optional model provider/name/version and ordered descriptive tools; presence marks a declared agent. | [Native acceptance](agent-metadata-acceptance.md) covers admission, reference authorization, Source isolation and connected clients. The annotation grants no execution privileges or provider connectivity. |
| Domains and subdomains | **Native analogue.** Domain, Subdomain, BoundedContext realizations, exact domain-scoped queries and authored Domain memberships exist. | Explicit domain nesting and shared versioned membership remain separate from strategic realization and namespace scope. Their [SystemGroup acceptance](system-structure-acceptance.md) passes in the integrated candidate. |
| Sends, receives and triggers | **Native analogue.** Component, ExternalSystem and Domain carry independent SEND/RECEIVE claims, routes, triggers and field selections alongside peer-directed communication intent. | The [resource association contract](resource-association-contracts.md) preserves separate authorities and descriptive grouping. Integrated identity, boundary and connected-client acceptance passes; declarations establish no trigger execution or message-to-behavior equivalence. |
| Channels | **Native analogue.** Channel has protocols, address parameters, delivery declarations and bindings to actual observations. | Claim routes and parameter values are integrated separately from observation and operation bindings. Channel composition is adopted in the [resource association contract](resource-association-contracts.md), with its implementation, corrections and authenticated browser acceptance verified in the integrated candidate. |
| Storage containers | **Native analogue.** DataStore retains class, technology, purpose, authority, access mode, classification, residency and retention. | The [resource association contract](resource-association-contracts.md) adopts explicit boundary/product storage-access claims; its implementation and corrections are integrated. A ReadModel attachment remains distinct from an authored access claim. |
| Schemas and specifications | **Native analogue.** Specification retains artifacts and selectable schema/message/operation members with explicit mappings and provider usage. | SchemaBinding environments and optional default declarations are integrated. Exact variant boundaries, identity and Source acceptance pass in the integrated candidate. Unavailable declarations stay distinct from environment resolution or deployment verification. |
| Data products | **Native analogue.** DataProduct has ports, contracts, selected access and declared lineage. | Native resource associations retain target varieties, direction and optional contract selectors independently of behavioral ports. Direction, identity and connected-client acceptance passes in the integrated candidate; no producing path is inferred. |
| Users, teams and ownership | **Native analogue.** Person, Team, membership and exact-subject responsibility exist. | [Directory authority is explicitly bounded](responsibility-contracts.md#scope-and-authority): profiles and pointers are unverified; no principal binding, automatic directory lookup or identity verification is implied. Catalog ownership grants no runtime authorization. |
| ADRs | **Native analogue.** Decision retains subjects, native status, rationale, options, consequences, dates, makers, amendments, replacement and supporting documents. | [Catalog context](catalog-context-contracts.md) preserves historical participation separately from responsibility. [Integrated native acceptance](catalog-context-acceptance.md#integrated-verification-receipt) passes; native status vocabulary does not imply source status conversion. |
| Business entities and properties | **Native analogue.** Addressable Schema carries optional aggregate-root intent, identified property-path declarations and exact entity/property relationships. | The [business data contract](business-data-contracts.md) separates declarations from type reuse, payload schemas and stream behavior. The executable assertion map and integrated acceptance cover identifier, relationship and client boundaries. |
| Ubiquitous language | **Native analogue.** Term and Ambiguity represent scoped vocabulary, definitions and interpretation; Term has optional exact domain ownership. | [Vocabulary ownership](catalog-context-contracts.md#vocabulary-ownership) remains independent of namespace and strategic realization. [Integrated native acceptance](catalog-context-acceptance.md#integrated-verification-receipt) passes for this ownership boundary. |
| Flows | **Native analogue.** DesignGraph retains identified typed steps, labeled transitions, branching, cycles and exact subflow references. Storyboards retain their ordered slices and traces. | The [topology contract](design-topology-contracts.md) keeps authored graph traversal separate from execution. SystemGroup targets are composed. Target, lifecycle, traversal and connected-client acceptance passes in the integrated candidate. |
| Designs and diagrams | **Native analogue.** DesignGraph provides authored topology and an optional Document layout reference; Document retains exact content. | Native topology and authenticated browser acceptance passes. Heterogeneous rendering and layout interpretation remain separate native product decisions. External format support is deferred under the native-first scope. |
| Pages and resource documentation | **Native analogue.** Document retains text, links or exact bytes, purpose, subjects, categories, anchors, ordered navigation and typed related content. | [Catalog context](catalog-context-contracts.md) preserves native associations without filesystem inference. Integrated acceptance covers authored changelog revision independence, exact retained bytes, collection limits and typed associations. Revision history does not replace prose. |
| Examples | **Native analogue.** EXAMPLE Documents retain raw bytes, media type, digest, filename, usage prose and exact subjects. | The composed raw-JSON example, selected MESSAGE and containing-Document case passes authenticated native acceptance. Arbitrary text is not a validated Event/Command payload or an executed example. |

## Remaining model checklist

These native requirements are complete under the linked contracts. Closure
requires an explicit native contract or exclusion, implementation and executable
acceptance for each adopted concept.

- [x] Define higher-level system grouping, authored domain/system/service
  memberships and system-level actor relationships.
- [x] Complete agent model and tool metadata across native admission, Source
  isolation and connected-client acceptance.
- [x] Define business identifiers, aggregate intent and entity/property
  relationships.
- [x] Define flow step and transition topology, branching and subflows; disposition
  authored design topology separately from visual layout.
- [x] Define communication relationships for domain claims, triggers, fields, groups
  and channel bindings without inferring execution.
- [x] Complete channel composition, storage access and DataProduct relationships.
- [x] Define remaining shared metadata, including lifecycle dates, draft messages,
  badges, resource groups and reference semantics.
- [x] Define remaining ADR, vocabulary, documentation, diagram and example fields.

The tables retain already implemented analogues so these items are not
mistaken for missing Channel, DataStore, Specification, DataProduct, Team,
Person, Decision or Document kinds.

## Delivery status

The adopted native workstreams are complete within their contracts. The
[integrated verification](#integrated-verification) combines executable acceptance
with the requirement audits. Separate proposals and exclusions remain outside
this completion boundary.

| Workstream | Contract and acceptance | Verified native boundary |
| --- | --- | --- |
| Agents and shared metadata | [Contract](agent-metadata-contracts.md), [acceptance](agent-metadata-acceptance.md) | **Complete within the native contract.** Parent verification and requirement review cover schema/core admission, nested scope, exact limits, references and provenance, identity and branch lifecycle, authorization, Source isolation, tenant descriptors, CLI/MCP roundtrips and Studio codec/component inspection. The public aggregate boundary includes stored occurrence IDs and verifies atomic refusal with unchanged reference indexes. No deployed-browser or execution capability is claimed. |
| Business data | [Contract](business-data-contracts.md), [acceptance](business-data-acceptance.md) | Schema declarations, property paths, checked references, Source preservation and writer revision 19 are integrated. The [authenticated order fixture](../../rsworkspace/crates/trogon-atlas-server/tests/business_data.rs) verifies explicit aggregate intent and payload isolation. **Complete within the native contract.** Identifier, relationship and connected-client assertions pass in the integrated verification. |
| Flow and design topology | [Contract](design-topology-contracts.md), [acceptance](design-topology-acceptance.md) | **Complete within the native contract.** DesignGraph target, traversal, occurrence identity, reference, incarnation, Source, branch and capacity assertions pass. Typed SystemGroup targets, actual Storyboard traces, native CLI/MCP export and reapply, codec inspection and authenticated Graph browser acceptance pass in the integrated candidate. Authored topology remains independent of execution and layout interpretation. |
| Communication and resource associations | [Contract](resource-association-contracts.md), [acceptance](resource-association-acceptance.md) | Domain, Component and ExternalSystem message claims and product/Specification associations are integrated. [Channel discovery](../../rsworkspace/crates/trogon-atlas-server/src/channel_messages.rs) retains independent Specification, claim-route and operation-binding authorities. [Source variant acceptance](../../rsworkspace/crates/trogon-atlas-server/tests/schema_variant_source_backend.rs) protects native environments and default presence from enclosing-field ownership. Channel/storage corrections, exact subjects, connected clients and complete authority joins are integrated. **Complete within the native contract.** Integrated acceptance includes the retained-claim correction, complete prepared capacity and authenticated Channel/storage browser proof. |
| Catalog context | [Contract](catalog-context-contracts.md), [acceptance](catalog-context-acceptance.md#integrated-verification-receipt) | **Complete within the native contract.** Decision dates/makers/amendments, Term ownership and Document purpose/categories/content/navigation/related associations have integrated identity, reference, authorization, lifecycle, Source, budget and connected-client acceptance. The authenticated boundary fixtures prove changelog independence, retained-byte limits, exact nested MESSAGE examples and atomic shared-write refusal. Fresh parent execution passed 155 native tests and 99 Studio codec/component tests. Typed DesignGraph topology remains a separate workstream; no parser, active rendering or execution is claimed. |
| System structure | [Contract](system-structure-contracts.md), [acceptance](system-structure-acceptance.md) | Abstract groups, authored memberships, typed actors and real DesignGraph targets are integrated. Retained-dangling and authorization-error repairs, Source/branch/budget fixtures and composed checks pass in the frozen integration receipt. **Complete within the native contract.** Concurrency, privacy, branch, client-diff, exact-identity expansion and composed capacity assertions pass in the integrated candidate. |

Native value tests, Studio component tests, generated codec tests and authenticated
backend fixtures prove different boundaries. Completion requires evidence for
each requirement's actual schema, mutation, reference and client behavior.

## Integrated verification

The verified code candidate is `8c7462c79c9036bf97457e867727e6bc693d3d5b`,
which combines the audited native candidate
`c8f24e15a41887a4c7fabe750ccef648f608b42c` with frozen upstream
`bf2c4759e45a0a9d9358567bc4434a61d948bc13`. Validation coverage preserves
requested and effective scope, snapshot incompleteness and suite results.
Stale `PutEntity` dry runs retain the same guarded precondition behavior as
writes. The native contract and both writer compatibility floors remain 19.

Verification on 2026-10-29 passed:

- Full Rust workspace: 2,832 passed, zero failed, four ignored, using
  `mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml --workspace --no-fail-fast -- --test-threads=1`.
- Strict workspace Clippy with all targets and `-D warnings`, Rust formatting,
  maintained protobuf generation equality and lint, default breaking checks
  against the frozen upstream baseline, and committed manifest schema equality.
- Studio: 1,574 tests, typecheck and lint with the frozen upstream lockfile.
- Authenticated native Channel/storage and DesignGraph Playwright acceptance
  against disposable real Stores through the production bridge, with
  candidate-SHA receipts and released fixture listeners.

The ignored historical revision-18 compatibility case has a separate explicit
runtime receipt under the [historical-client procedure](../how-to/test-historical-native-client.md).
The ignored browser-serving fixtures were explicitly executed by the native
Playwright acceptance. The credential-dependent live Jev test remains unexecuted
and is outside this native coverage scope. Browser proof covers the authenticated
fixture roles; it does not establish a restricted-reader browser matrix or a
production deployment. Backend and MCP authorization fixtures cover the stated
privacy and no-grant requirements.

## Atlas adaptations and separate roadmap

These native Atlas proposals are separate from the completed coverage scope:

| Atlas choice | Native boundary |
| --- | --- |
| Exact numeric identities, owner incarnation UIDs and occurrence selectors | Native foundations for checked references and mutation safety. |
| [Component UI hosting and InspectHumanInteraction](human-interaction-contracts.md) | Proposed Atlas representation of human interaction paths; not implemented. |
| Typed behavioral binding and executable communication predicates | Separate behavioral work. Source trigger declarations and condition prose do not prove runtime execution. |
| WASM/WIT linter host | Atlas extension architecture, separate from native resource coverage. |

The [architecture roadmap](../explanation/architecture-resource-roadmap.md)
tracks these choices. Completing native coverage does not automatically adopt
them.

## Updating the checklist

Close an item only with a linked disposition and evidence for the claimed
boundary: native schema, validation, reference discovery and query support.
Excluded concepts must remain visible in this inventory. A passing resource
fixture does not establish every proposed relationship.

Update native statuses from native contracts and executable evidence. External
comparison research does not add integration work to this checklist.

## Historical comparison baseline

EventCatalog source inventory: commit
[`2736a2272207cf1b1ee09e6a440401d2215c6380`](https://github.com/event-catalog/eventcatalog/tree/2736a2272207cf1b1ee09e6a440401d2215c6380),
audited on 2026-10-16. The
[collection definitions](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts)
anchor the resource and field inventory. This is a pinned comparison, not a
claim about all features in the latest EventCatalog release.
