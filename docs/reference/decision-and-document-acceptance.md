# Decision and Document acceptance

Status: native Decision/Document schema, backend and client surfaces are
implemented under the [contract](decision-and-document-contracts.md). The
coverage section links executable cases and separates their evidence from
remaining acceptance requirements. Source fixtures below establish reusable
native anchors; their existence alone does not prove the new resource behavior.

## Connected fixture anchors

| Verified existing source | Reused exact anchors |
| --- | --- |
| [Responsibility fixture](../../rsworkspace/crates/trogon-atlas-server/tests/responsibility_fixture.rs) | Namespace `responsibility-fixture`, version 1: ReadModel `receipt`, Processor `notifier`, AutomationSlice `observe`, Specification `receipt-contract`, DataProduct `receipts`, Person `maintainer`, Teams `receipts-team` and `support-team`. `observe.source_read_models` deliberately contains repeated edges to the same receipt. |
| [Specification fixture](../../rsworkspace/crates/trogon-atlas-server/tests/specification_fixture.rs) | Namespace `contracts`, version 1: `contract` helper creates retained `order.graphql` with bytes `type Order { id: ID! }` followed by newline and a native schema member. The connected test subsequently adds message members. Use returned artifact/member IDs and digest for exact selections. |
| [DataProduct fixture](../../rsworkspace/crates/trogon-atlas-server/tests/data_product_fixture.rs) | Namespace `product-fixture`, version 1: `orders`, `api`, `warehouse`, `delivery`, `placed`, `place`, `handle`, `project` and `observe` connect product access and lineage to native behavior. `Submitted` and `Placed` are input port display names, not IDs. |
| [Ecommerce fixture](../../rsworkspace/crates/trogon-atlas-server/tests/phase10_ecommerce_fixture.rs) | `order-to-fulfillment` Storyboard includes `place-order-slice`, `orders-list-slice`, `auto-capture-payment`, `capture-payment-slice` and subsequent fulfillment steps in authored order. Namespace `ecommerce`, version 1; use returned identities, not display positions. |

The [connected extension](../../rsworkspace/crates/trogon-atlas-server/tests/decision_document_fixture.rs)
creates Decision `receipt-delivery-choice@1`
and Document `receipt-delivery-note@1` in `responsibility-fixture`. These are new
fixture resources. The Decision accepts a retained
option with rationale and a RISK consequence. The Document retains Markdown and
a declared `delivery` anchor. Both select `receipt`, each distinct observation
ConnectionRef, the Receipt MESSAGE member and Receipts OUTPUT port. Capture
actual persisted owner UIDs, local IDs, association IDs and etags from responses.
Do not turn fixture array indices or display names into durable selectors.

Add a supporting anchor selection with expected digest, whole-document selection,
a link-only Document, a related Decision and an accepted replacing Decision.
Declare OWNER on the Decision and STEWARD on the Document through the existing
Team/Person responsibility layer. Member contact expansion remains distinct from
assignment lookup. Add an authorized foreign target and separately provision a
hidden namespace/referrer using the authenticated backend harness. Missing and
mismatched selectors are explicit test inputs, never substitute fixture anchors.

Record branch, checked authorization scope, content bytes, digests, returned
identities, guards, query completeness and request results. Do not invent fixed
UIDs or describe an assembled query as a historical transaction snapshot.

## Admission and exact selection

| Case | Required outcome |
| --- | --- |
| Complete connected extension | Inspect decision rationale/options/consequences and retained document content alongside existing Event Modeling behavior; create no behavioral shortcuts. |
| Empty subjects, options, title/context/rationale, required consequence or missing/unknown alternative | Reject the whole owner under contract validation. |
| Every Decision status | Enforce selected-option and replacement presence rules, including rejection of selected options on PROPOSED/REJECTED/WITHDRAWN. |
| Reopen or edit an adopted choice | Apply full proposed-state rules under guards; do not imply immutable approval or execution evidence. |
| Replacement versus version ancestry | Different decision family replacement is allowed; same-family version ancestry alone does not supersede a decision's disposition or move subjects. Document ancestry adds no status. |
| Replacement self-edge/cycle | Reject direct, multi-hop, cross-namespace and final-batch cycles; require an ACCEPTED terminal target. RELATED/navigation cycles between distinct owners remain descriptive. |
| Replacement target rejected, withdrawn, proposed, missing or inaccessible | Reject new/edited replacement admission without leaking hidden intermediate nodes. Retained later-invalid chains remain diagnosable. |
| Repeated observation edges | Select each returned ID independently. Queries retain distinct association provenance despite identical ReadModel endpoints. Removing one occurrence breaks only its selections. |
| Wrong exact entity version/kind | Reject without latest-version, same-title or version-family fallback. |
| Wrong member UID/kind/ID or artifact digest | Reject without name/path/content rematching; nonartifact digest presence rejects. |
| Wrong port UID/direction/ID | Reject, even if another port has equal name/data. |
| Stale connection UID, duplicate/malformed persisted IDs, removed occurrence or wrong native context | Reject selection after complete owner identity validation; no index/endpoint fallback. |
| Selected owner has an unrelated invalid declaration | Resolve the selected identity/context without recursively validating unrelated outgoing edges; report independent diagnostics separately. |
| New Decision/Document entity or association as CatalogSubject | Reject under the closed inventory. Support/navigation/RELATED do not create recursive subject kinds. |
| Team/Person assignment to Decision/Document entity kinds | Preserve direct kinds and provenance; no inherited responsibility, authorization or authenticated approval. Assignment to anchors/options/new associations rejects. |

## Retained content and navigation

| Case | Required outcome |
| --- | --- |
| Retained text/plain or text/markdown | Verify exact UTF-8 bytes and lowercase SHA-256 without newline/Unicode normalization. |
| Link-only document | Preserve inert HTTPS URL as unverified; no server fetch during admission, query, preview or export. |
| Empty/invalid digest, binary bytes, HTML/media parameters, credentials in URL, file URL or arbitrary filesystem path | Reject typed body/link input. Do not interpret arbitrary prose as filesystem instructions or discovered references. |
| Missing/both body alternatives, anchors on link-only body | Reject atomically. Switching retained to link requires explicit anchor removal. |
| Anchor declaration | Enforce valid fragment, unique IDs/fragments and label; do not assert heading/HTML-element verification. |
| Byte edit preserving anchor ID | Unpinned declared anchor selection remains; old digest-pinned selection becomes unresolved. No content-based rematching. |
| Rename label, reorder anchors or change fragment | Preserve ID for label/order; fragment change requires new ID. |
| Same-key owner recreation | Whole DocumentRef follows the key; old anchor UID/ID never resolves into the new incarnation, even with matching text/fragment/digest. |
| Anchor selection of link body, absent ID, wrong UID/digest | Reject new admission and diagnose retained unresolved state safely. |
| URL or active markup inside retained prose | Retain bytes, use safe rendering, never execute/fetch or create hidden graph edges. |
| Every text/collection/entity ceiling | Accept boundary values; reject additional work/content atomically without truncation or remote-only fallback. |

## Lifecycle, branches and references

| Case | Required outcome |
| --- | --- |
| Initial create, preview and existing-owner addition | Returned persisted identities govern selection. Preview IDs reserve nothing; supplied unknown IDs on existing-owner additions reject. |
| Edit/reorder associations | Preserve unchanged IDs; reject identity stripping and fingerprint change retaining an ID, including digest presence/value changes. |
| Remove/re-add | Fresh option/anchor/association identity; no resurrection by endpoint, fragment or position. |
| Native authoring/export/unchanged reapply | Preserve exact body bytes, links, media type, pin presence, option/anchor/association IDs and authored order. Exercise the existing id-less exact-content no-op rule separately from edited identity stripping. |
| Clone/new version | Fresh owner UID, scoped IDs may remain, external exact targets remain explicit and incoming refs stay on the source. Explicit mapped local targets use actual returned identities. |
| Branch land/rebase | Retain unchanged identity; compare base/baseline/branch and prevent ordinary resurrection of opposing-side deletions. |
| Explicit conflict resolution | Renew restored IDs deleted on the opposing same-incarnation owner; preserve selected incarnation. Test independent same-key creation and whole-owner deletion separately. |
| Safe entity/owner delete | Block on subject, nested local owner, supporting document, navigation, RELATED, replacement, version ancestry or responsibility refs. Hidden referrers protect without disclosure. |
| Safe batch removing referrers and targets | Evaluate the complete final branch graph including tombstones and all targets; preview/apply agree under current guards, without partial publication. |
| Force-delete entity or remove selected anchor/member/port/connection | Keep exact incoming selectors unresolved without cascade, replacement lookup or retarget. Unrelated writes remain possible; edited invalid owners require full repair. |
| Delete Decision/Document | Delete outgoing associations only, preserve subjects and document/decision targets, and make no history/backup purge claim. |
| Generic reference and impact, warm/cold caches | Include every nested owner dependency and retain full selector, from UID/field/association provenance. Never collapse local selection to bare owner identity. |
| Generic retarget | Allow supported plain entity refs with renewed associations; refuse owner-key-only substitution for incarnation/local targets. |

## Queries, privacy and budgets

| Case | Required outcome |
| --- | --- |
| Subject reverse lookup | Exact full typed match; member/port/connection lookup does not inherit owner assignments or match siblings. |
| Detail and incoming relation lookup | Return deterministic owner/association ordering, preserve authored detail collection order and reject oversized unpaginated results. Preserve authored selectors, status, disposition, option and anchor identity, byte pins and relation provenance. Whole and anchor selections are distinct matches. |
| Complete empty versus unavailable/incomplete/error | Preserve distinct scope/completeness/error semantics; hidden and missing hydration remain indistinguishable where disclosure is unsafe. |
| Hidden target direct read or query | NOT_FOUND without target titles, content, anchors, digest, path or existence disclosure; Reader mutation PERMISSION_DENIED. |
| Readable owner with hidden target | Preserve already authored selectors; suppress new hydration details. Visible totals/pages reveal no hidden referrers. |
| Exact target outside projection cap | Hydrate authorized owner independently of display caps; absence from projection is not nonexistence. |
| Discovery resource ceiling | Share 5,000 rows across existing resource families and Decision/Document; count each exact selected-view key once. At the boundary permit completed work, beyond it return RESOURCE_EXHAUSTED. |
| Work/output ceilings | Typed catalog queries and replacement admission share 10,000 examined association/anchor/replacement items request-wide; queries return at most 4,096 records. Repeated endpoints cost work; pages, batches and repeated queries within the request cannot reset budgets. |
| Complete retarget enumeration | Unrelated discovery occurrences do not block migration. Preserve complete resource-row bounds, staged-write admission and lower Store batch limits; reject output beyond 4,096 results before preview/apply can publish writes. |
| Replacement admission cannot finish within budget | Reject atomically, never infer cycle absence or ACCEPTED termination from partial discovery. |
| Bounded impact/closure/export truncation | Explicit incomplete metadata and visited identities; no verified negative claim or complete reapply artifact. |
| Safe-delete reporting exceeds visible row budget | Existing capped reporting may be incomplete, but independent complete cross-namespace guard must still protect every target. |
| Guard timeout/backend failure or nonbudget reporting failure | Refuse unsafe deletion/return operational error; never substitute missing dependency or empty results. |
| Historical declaration read | Use checked retained context; never attach current hydration as historical fact. |

## Required implementation evidence

Acceptance requires executable native admission/core identity checks,
authenticated backend tests, generic reference/impact and complete safe-delete
coverage, branch/clone/merge regressions, and native manifest/client/CLI/MCP
round trips. Studio review must preserve incomplete states, safe rendering,
authored storyboard order and repeated occurrences. Browser/deployed evidence
must be recorded separately from source and local test evidence.

No existing fixture, successful documentation check or contract approval marks
every acceptance requirement complete. Linter execution and deployment
acceptance remain separate work.

## Executable coverage and remaining evidence

| Source | Exercised behavior |
| --- | --- |
| [Connected fixture](../../rsworkspace/crates/trogon-atlas-server/tests/decision_document_fixture.rs) | Uses exact version 1 in `responsibility-fixture`, a ReadModel, repeated observation occurrences, a Specification MESSAGE and a DataProduct OUTPUT port. Checks adopted choice, retained bytes and digest-pinned anchor, whole versus anchor incoming declarations, direct responsibility assignments, generic reference/impact provenance, reorder, stale byte pins and guarded/forced deletion with retained selectors. Exact hydration runs with display snapshot cap zero. |
| [Core identity regressions](../../rsworkspace/crates/trogon-atlas-core/tests/decision_document.rs) | Values, complete local identities, exact anchor/digest resolution, id-less reapply, removal and fresh identity preparation, graph references and selector-preserving retarget rules. |
| [Authenticated backend](../../rsworkspace/crates/trogon-atlas-server/tests/decision_document_backend.rs), [handler regressions](../../rsworkspace/crates/trogon-atlas-server/tests/handler_tests.rs) | Admission, replacement chains, typed detail/reverse lookup, hidden targets/referrers, repeated selectors, branches, final-batch delete protection, local-selector retarget refusal, operational failure and shared discovery/work/output ceilings. Delete-report tests distinguish reporting incompleteness from the independent complete safety guard. |
| [Store routing and guards](../../rsworkspace/crates/trogon-atlas-store/tests/decision_document_routing.rs) | New resource keys, nested reference protection and branch-state incoming discovery. The existing complete Store guard remains independent of visible query caps. |
| [Manifest/provenance checks](../../rsworkspace/crates/trogon-atlas-client/tests/decision_document_manifest.rs), [typed transport](../../rsworkspace/crates/trogon-atlas-client/tests/decision_document_transport.rs), [CLI](../../rsworkspace/crates/trogon-atlas-cli/tests/decision_document_cli_integration.rs), [MCP](../../rsworkspace/crates/trogon-atlas-mcp/tests/decision_document_mcp_integration.rs) | Native declarations, exact selectors, byte/pin and generated-ID preservation, unchanged reapply classification, typed operations, malformed input and incomplete-report transport/presentation. |
| [Studio inspector](../../tsworkspace/webapps/studio/src/components/DecisionDocumentInspector.test.tsx), [API context](../../tsworkspace/webapps/studio/src/lib/decision-document-api.test.ts), [serializer boundary](../../tsworkspace/webapps/studio/server/decision-document-contract.test.mjs) | Read-only subject/detail review, exact navigation selections, historical no-current-hydration behavior and strict typed JSON validation before protobuf normalization can change request intent. |

These tests establish the behavior they exercise, not exhaustive coverage of
every row above. Full subject/association inventory permutations, all
clone/merge and cross-namespace combinations, production-scale resource limits,
retention operations and deployed-browser acceptance still require separate
case-to-evidence records. Generic impact keeps an existing representative path
per owner; the connected fixture does not prove enumeration of every parallel
path. Native manifest round trips do not prove external import/synchronization
or source export fidelity.
