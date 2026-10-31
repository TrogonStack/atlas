# Catalog context acceptance specification

Status: verified native acceptance at integration revision
`6e4859750e3b7d1db0b5df4f80cfdcf0db7e72ed`. The
[contract](catalog-context-contracts.md) and assertions below establish the native
Catalog-context boundary; they do not close unrelated coverage workstreams.

Executable coverage includes [core values](../../rsworkspace/crates/trogon-atlas-core/tests/decision_document.rs),
[Document content](../../rsworkspace/crates/trogon-atlas-core/tests/document_catalog_context.rs),
[Decision backend](../../rsworkspace/crates/trogon-atlas-server/tests/decision_document_backend.rs),
[Document authorization/lifecycle](../../rsworkspace/crates/trogon-atlas-server/tests/document_catalog_auth_lifecycle_backend.rs)
and [Term ownership](../../rsworkspace/crates/trogon-atlas-server/tests/term_domain_backend.rs).
[Storage-fault acceptance](../../rsworkspace/crates/trogon-atlas-server/tests/catalog_context_fault_backend.rs)
checks that unreadable persisted catalog bytes cause an operational error during
incoming discovery and that restoring readable bytes recovers the exact declaration
identity under a new etag. The Decision backend fixture also verifies that an
authorization provider's `Unavailable` and `DeadlineExceeded` statuses remain
operational errors during Document discovery and a guarded update, with
unchanged owner content, etag and change sequence. These injected provider
responses verify propagation; they do not measure network timeout scheduling.
The [composed fixture](../../rsworkspace/crates/trogon-atlas-server/tests/catalog_context_composed.rs)
passes in the parent checkout. Decision transitive safety, exact provenance and
branch-landing controls also have independent passing runs. These checks prove
their explicit assertions. The boundary suite below closes the remaining native
requirements. Studio retained-content presentation/download has component and
generated-codec coverage, including invalid projections and download URL cleanup. Real-browser
verification has not run; the adopted safe-review boundary is covered by actual
codec and component assertions. Persisted restoration, incoming pagination and
caller/snapshot token checks are exercised by the linked backend suites.

[Boundary acceptance](../../rsworkspace/crates/trogon-atlas-server/tests/catalog_context_boundaries_backend.rs)
runs the real authorization layer and Store through public gRPC. It checks that
resource revisions leave an authored CHANGELOG Document unchanged, that
RetainedContent bytes admit exactly 262,144 and reject 0 and 262,145 bytes with
correct digests (distinct from the retained-text limit), and that an EXAMPLE
Document keeps its exact Specification MESSAGE selector and related association
identities without a validated payload. It also checks `related_content` at 128
and 129, the derived 1,280 Document owner maximum, and amendment traversal
exhausting one shared 10,000 work budget at 38 and 39 middle Decisions, alone and
across a batch, with no partial writes. The combined 2,048 owner association
limit is unreachable for Documents (1,024 subjects, 128 navigation, 128 related)
and Decisions (1,537), so the [core tests](../../rsworkspace/crates/trogon-atlas-core/tests/decision_document.rs)
check those reachable maxima and that each collection limit rejects first.

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

Coverage spans core value/admission and reference tests, guarded final-view and
authorization backend tests, Store/client routing and round trips, and CLI/MCP
transport of typed results and binary bytes. Retain the executed command,
candidate revision and result for each claimed boundary. A fixture name alone
does not establish a passing run.

Require explicit rejection evidence for conflicting multi-owner batches and
incomplete safety checks. Verify safe review rendering without executing retained
HTML/SVG/scripts. Inspect manifest and export byte equality and presence of pins,
IDs and ordered associations. No EventCatalog fixture or importer is required.

The contract adopts calendar-day precision, exact binary retention, categories
on Document navigation and shared amendment-check budgets. These assertions
cover those choices. Typed diagram topology has a separate contract. No test
here proves source-file fidelity, approval authority or execution.

## Integrated verification receipt

The parent executed the maintained native matrix at
`6e4859750e3b7d1db0b5df4f80cfdcf0db7e72ed`; Rust and protobuf sources stayed
unchanged during the run. All selected tests passed with no failures or ignored
cases. This is a Catalog-context receipt, not a whole-workspace result.

| Layer | Passing tests | Maintained fixtures |
| --- | --- | --- |
| Core | 47 | Calendar dates, Decision/Document values, exact reference provenance, retained content and Term ownership. |
| Server | 90 | Decision backend and replacement diagnostics; Catalog boundaries, composed graph and faults; Document content, authorization, lifecycle, incoming pages and Source; Term ownership and Source. |
| Client | 8 | Decision/Document manifests and transport; Term-domain transport. |
| CLI | 5 | Decision/Document, binary content and Term-domain integration. |
| MCP | 4 | Decision/Document, binary content and Term-domain integration. |
| Store | 1 | Decision/Document routing. |
| Studio | 99 | `DecisionDocumentInspector.test.tsx`, `Inspector.render.test.tsx` and the actual `decision-document-contract.test.mjs` codec fixture. |

The native invocation used `mise exec -- cargo test`, the named targets above
and `--test-threads=1`. Studio used `mise exec -- pnpm exec vitest run` with the
listed files. The parent retained complete commands and results in its private
run record. Requirement review checks assertion bodies as well as passing logs.
No deployed-browser, source-file recovery or runtime execution claim follows.
