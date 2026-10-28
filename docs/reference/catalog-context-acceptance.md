# Catalog context acceptance specification

Status: acceptance requirements for the implemented native extensions. The
[contract](catalog-context-contracts.md) and this matrix do not close the master
coverage checklist.

Executable coverage includes [core values](../../rsworkspace/crates/trogon-atlas-core/tests/decision_document.rs),
[Document content](../../rsworkspace/crates/trogon-atlas-core/tests/document_catalog_context.rs),
[Decision backend](../../rsworkspace/crates/trogon-atlas-server/tests/decision_document_backend.rs),
[Document authorization/lifecycle](../../rsworkspace/crates/trogon-atlas-server/tests/document_catalog_auth_lifecycle_backend.rs)
and [Term ownership](../../rsworkspace/crates/trogon-atlas-server/tests/term_domain_backend.rs).
The [composed fixture](../../rsworkspace/crates/trogon-atlas-server/tests/catalog_context_composed.rs)
passes in the parent checkout. Decision transitive safety, exact provenance and
branch-landing controls also have independent passing runs. These checks prove
their explicit assertions, not every requirement below. Safe Studio retained-content
presentation/download and remaining Document restoration/pagination cases remain
open in the [coverage status](eventcatalog-coverage.md#delivery-status).

## Fixtures and executable acceptance targets

Use isolated native fixtures in an authorized branch: exact-version Decisions A,
B and C with valid existing options/consequences; Person P and Team T; Domain D;
Term W scoped to a bounded context; Documents X and Y with returned incarnation
and anchor IDs; Specification S with returned MESSAGE and ARTIFACT member IDs.
Create visible and hidden namespace variants and capture etags. Never identify
fixtures by title, array position or latest-version lookup.

| Acceptance case | Setup/action | Required assertion |
| --- | --- | --- |
| Calendar dates | Write leap-day date, absent dates and invalid Gregorian dates; decided_on before authored_on | Valid values round trip exactly; invalid values reject atomically; no inferred audit time or status transition. |
| Makers | Add P and T as makers, then edit responsibility and Team membership | Returned maker occurrences remain unchanged; no implied approvals or member expansion. Duplicate exact makers and wrong-kind refs reject. |
| Native status | Exercise all existing statuses and an unknown enum | Existing selected-option/consequence/replacement invariants remain; unknown value rejects. Dates never synthesize a status. |
| Amendment | A amends B; B amends C; attempt C amends A | Cycle rejects, including one guarded batch; RELATED descriptive cycles remain legal. |
| Replacement conflict | A amends B, then set B.replaced_by=A; repeat in opposite write order and batch | Both orderings and batch reject the same conflicting final graph. Unrelated RELATED or version ancestry never becomes AMENDS. |
| Vocabulary | Two Terms with equal title, different exact keys; W selects D | Entries remain distinct; definition and explicit domain round trip; namespace does not infer domain. Missing or hidden D rejects new selection safely. |
| Categories and sidebar | Mark X category; order navigation to Y and an anchor | Native label/order/pin survive export and reapply; no inferred filesystem or inherited membership. Duplicate category annotations reject. |
| Authored changelog | CHANGELOG document with prose; mutate resource revisions | Authored prose stays unchanged. Revisions never create or overwrite it. |
| Raw retention | JSON example, SVG diagram and non-UTF-8 attachment with digest | Exact original bytes/media/digest survive native round trip; no parser, active renderer, network or execution invoked. Mismatched digest rejects. |
| Content limits | Empty bytes, content at/over 256 KiB, encoded entity at/over 1 MiB | Accepted boundary obeys all lower transport limits; overflow rejects whole write without truncation. |
| Filename | Valid logical name; slash/backslash/control/dot-dot names | Valid name round trips as label; prohibited names reject without filesystem access. |
| Nested example | EXAMPLE Document subjects select exact S MESSAGE; X relates to example | Forward and reverse results retain member kind, UID, ID and association identity. Raw JSON is not a validated native payload. |
| Typed content roles | Same target under different roles; duplicate role/selection; conflicting declared purpose | Distinct roles preserve occurrences; duplicate/conflicting new declarations reject; generic absent purpose remains allowed. |
| Binary anchor | Attempt anchor on RetainedContent; select existing retained-text anchor | Binary anchor rejects. Existing text anchor and optional digest use existing exact resolution. |
| Identity | Reorder/rename, retarget, remove/re-add, clone, recreate same key | Stable edits retain IDs; retarget/re-add renew IDs; clone UID changes; stale contained selectors fail while plain refs follow key. |
| Branch conflicts | Opposing same-incarnation removal and explicit restoration | Ordinary merge never resurrects removed IDs; explicit restoration uses fresh IDs and no silent external rebinding. |
| Reference completeness | Enumerate new maker/amendment/domain/content references | Generic references, reverse queries and impact retain typed owners, fields and occurrence selectors. Domain is a plain ref without invented local occurrence. |
| Deletion | Safe-delete targets including hidden incoming owners; force-delete separately | Complete guard blocks safely without hidden details; force-delete preserves exact unresolved declarations without cascade. |
| Authorization | Reader write, hidden direct read, readable owner with hidden target | PERMISSION_DENIED, NOT_FOUND and generic UNAVAILABLE respectively; no hidden title/bytes/count leakage. |
| Budget | Cross 5,000 rows, 10,000 work items, 4,096 outputs and owner association limits | RESOURCE_EXHAUSTED; no nested reset within one request/batch/traversal; each pagination request has its own bounded budget; writes atomic. Incomplete reporting never authorizes delete. |
| Pagination | Traverse new listing pages with small page_size; change caller, branch/filter or snapshot between requests | Deterministic order, exact distinct occurrences, bounded-snapshot completeness and explicit stale/mismatched token rejection. |
| Purpose presence | Absent purpose and explicit GENERAL | Values remain distinct through binary/JSON/native manifest roundtrip. |
| Operational failure | Inject timeout/backend/auth-service failure during cycle or incoming guard | Operational error retained; no model-error substitution, empty success or partial mutation. |
| Source isolation | Native edits without registration; existing Source-owned unrelated field | Native operation needs no Source. Source creates containing new extensions reject even under TITLE-only ownership; WholeOwner current/incoming nonempty extensions reject including clears. Unrelated field-specific updates preserve local extensions exactly; new selectors reject. |

## Evidence required before reporting implementation

Implement value/admission tests in core, graph/reference tests in core and server,
guarded final-view and authorization tests in backend, routing/round-trip tests
in Store/client, and transport tests for CLI/MCP typed results and binary bytes.
Use existing test suites as placement guidance, not evidence that the new cases
already pass. Record actual commands, revisions and results at implementation
time. Parent owns protobuf generation and compile queues; this task ran neither.

Require explicit rejection evidence for conflicting multi-owner batches and
incomplete safety checks. Verify safe review rendering without executing retained
HTML/SVG/scripts. Inspect manifest and export byte equality and presence of pins,
IDs and ordered associations. No EventCatalog fixture or importer is required.

Unresolved design decisions are the contract's calendar-date precision, binary
transport, category representation and amendment-check cost objections. Typed
diagram topology and richer selectors require their separate contracts. No test
here proves topology, source-file fidelity, approval authority or execution.
