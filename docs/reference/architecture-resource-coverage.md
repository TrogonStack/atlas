# Architecture resource coverage

This reference separates native resource support from connection coverage and
acceptance evidence. A retained field or artifact does not establish a checked
relationship, executable behavior, or a passing client workflow. See the
[roadmap](../explanation/architecture-resource-roadmap.md) for design boundaries.

## Native resource contracts

| Resource | Supported fields and selections | Contract and executable coverage |
| --- | --- | --- |
| Strategic scope | Project, Domain, Subdomain and BoundedContext records; project/domain references, realized subdomains and directed context agreements. | [Strategic schema](../../proto/trogonatlas/eventmodel/v1alpha1/strategic.proto), [reference extraction](../../rsworkspace/crates/trogon-atlas-core/src/refs.rs). |
| Component and ExternalSystem | Component technology and curated exact entity membership; external boundary descriptions, ReadModel external-source links and Processor calls. | [Infrastructure schema](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto), [reference extraction](../../rsworkspace/crates/trogon-atlas-core/src/refs.rs). |
| Channel | Address parameters, protocols, delivery declarations, optional broker and bindings to exact AutomationSlice observation occurrences. | [Contract](channel-and-datastore-contracts.md), [acceptance](channel-and-datastore-acceptance.md). |
| DataStore | Store class, technology, purpose, authority, access mode, classification, residency, retention and identified ReadModel attachments. | [Contract](channel-and-datastore-contracts.md), [acceptance](channel-and-datastore-acceptance.md). |
| Specification | Retained artifacts, schema/message/operation members, individual message mappings, operation roles, provider exposures and usages. | [Contract](specification-and-api-contracts.md), [acceptance](specification-and-api-acceptance.md). |
| DataProduct | Input/output ports, output contracts, selected storage/channel/operation access and declared lineage; inspection reports checked model paths separately. | [Contract](data-product-contracts.md), [acceptance](data-product-acceptance.md). |
| Team and Person | Directory records, identified membership and explicit responsibility for an exact entity, supported connection occurrence, Specification member or DataProduct port. | [Contract](responsibility-contracts.md), [acceptance](responsibility-acceptance.md). |
| Decision | Subjects, disposition, options, selected option, rationale, consequences, replacement and supporting Document selections. | [Contract](decision-and-document-contracts.md), [acceptance](decision-and-document-acceptance.md). |
| Document | Subjects, retained content or link, content digest, identified anchors and navigation. | [Contract](decision-and-document-contracts.md), [acceptance](decision-and-document-acceptance.md). |
| Source control | Registrations, immutable supplied snapshots, explicit mappings/adoption, field ownership, overlays, guarded runs and durable publication/recovery. Source records are separate from native entity kinds. | [Contract](source-integration-contracts.md), [API](source-control-api.md), [acceptance](source-integration-acceptance.md). |

Shared native identity uses exact kind, namespace, slug and numeric version.
Contained selections additionally use the returned owner incarnation and local
identity required by their contract. Titles, names, list positions and source
version strings do not replace these selectors. See
[connection identity](connection-identity.md).

## Remaining contract boundaries

| Area | Existing representation | Remaining boundary |
| --- | --- | --- |
| Strategic membership | Domain project, Subdomain domain and context realizations are native references. | Arbitrary domain nesting and shared authored system/service membership have no dedicated fields. Domain listing derives membership from namespace and one realized subdomain; its maps omit versions. It does not prove exact-version or multi-domain membership. |
| System and service relationships | Component members can select other Components and ExternalSystems, with ordinary reference discovery. | Dedicated grouping levels, labeled directed system relationships and system-level actor interactions require a contract decision. Containment is Component-owned; ExternalSystem has no members field, so external-system-owned membership also remains a contract decision. |
| Communication | API roles/providers/usages, Processor calls and Channel bindings to actual observations are supported. | General authored send/receive and conditional-trigger declarations before behavioral mapping have no native relationship contract. Retained source declarations do not establish executable paths. |
| Metadata and agents | Registered annotations include links, code repository/path/symbol/ref provenance, tags and lifecycle status. | Lifecycle effective dates and agent model/tool declarations need dedicated contracts. Arbitrary metadata references are not recursively indexed. See [annotations](../../proto/trogonatlas/annotation/v1alpha1/annotations.proto). |
| Business data | Schema fields, nested shapes, schema type references and identity-derivation annotations are available. | Business identifier roles, foreign entity/property selections, relationship labels and general aggregate intent are not supplied by type reuse. See [field schema](../../proto/trogonatlas/eventmodel/v1alpha1/fields.proto). |
| Design graphs | Storyboards retain ordered slices and traces; Documents retain content, anchors and structured navigation. | Generic branches, loops and subflows remain document content without typed topology traversal. Parsing diagrams and validating executable examples require separate contracts. |
| Dependency explanations | References expose actual field paths; impact follows graph reachability; DataProduct inspection checks selected access. | These queries do not classify every runtime service dependency or recover relationships absent from native contracts. |

These boundaries are scoped design decisions, not implicit requirements to add
resource kinds. Supplied-bundle source reconciliation is implemented; remote
connectors, automatic synchronization, additional semantic format compilers,
portable selector rebinding and the WASM/WIT linter host remain separate work.

## Evidence boundaries

The linked acceptance references identify separate executable fixtures and
remaining requirements. The resource-specific fixtures use independent stores
and namespaces. The composed fixture below supplies an additional shared graph;
neither its existence nor separate passing tests prove every client workflow.

Backend service tests, authenticated transport tests, CLI/MCP tests, Studio
component tests and browser runs establish different boundaries. Browser
evidence must identify its backend and fixture; a fake gRPC backend cannot
establish native persistence or source publication. A native manifest roundtrip
in the same store does not establish portable migration of incarnation-bound
selectors into a new store.

## Composed graph

The [composed fixture](../../rsworkspace/crates/trogon-atlas-server/tests/connected_architecture_fixture.rs)
creates a registered namespace and retains the actual returned owner UIDs,
connection IDs, member IDs, port IDs and anchor IDs. Its logical identities use
version `1`; the generated namespace is reported by the live harness.

| Entry point | Connected declarations and checks |
| --- | --- |
| Component `receipt-service` | Exact membership of Processor `notifier`, ReadModel `receipt`, Specification `api` and DataProduct `receipts`. |
| AutomationSlice `observe` | Repeated observations of `receipt` have distinct identities. Channel `delivery` binds both; DataStore `warehouse` attaches to the same ReadModel. Reordering observations preserves their selectors. |
| Specification `api` | A response operation selects the returned Receipt message member mapped to `receipt`. Product output access selects that exact response, storage attachment and both channel observations. An undeclared provider remains undeclared. |
| Product `receipts` | Input/output ports retain identity. Declared lineage is reported separately from resolved access and data selections. |
| Person `maintainer` and Team `receipt-team` | Exact observation/member/port assignments and Team membership remain queryable. Assignment to one observation does not spread to its equal-endpoint sibling. |
| Decision `delivery-choice` and Document `delivery-note` | Accepted rationale, exact subjects, retained Markdown and a digest-pinned anchor remain connected through typed queries and reverse discovery. |
| Source registration `receipt-source` | A PARTIAL immutable bundle and explicit adoption own only the receipt title. Preview leaves native state unchanged; apply and replay preserve local documentation, all other owners and retained original bytes. |

The fixture asserts expected outgoing endpoint multiplicities, unique field
paths and matching incoming references. Native manifest export/reapply in the
same store preserves content, etags and returned identities. It does not assert
portable rebinding, all strategic-scope cases, every resource lifecycle case,
or the broader source capture matrix.

The ignored `serve_connected_architecture_fixture` test exposes that same
exercised graph over authenticated loopback gRPC and MCP for a bounded manual
run. See [connected acceptance](../how-to/verify-connected-architecture.md) for
CLI, MCP and Studio verification. Normal test execution does not start these
manual listeners.
