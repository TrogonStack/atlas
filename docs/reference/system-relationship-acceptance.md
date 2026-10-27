# System relationship acceptance requirements

Status: native support for the [contract](system-relationship-contracts.md) is
implemented. This reference defines acceptance requirements and links executable
coverage below. Logical fixture notation is not verbatim manifest syntax or an
RPC payload. Linked service, client and component tests establish different
boundaries; they do not establish deployment or runtime behavior.

## Fixture and evidence

Materialize each case in a fresh registered namespace, using exact native
version `1` unless a case names another version. Logical namespace `commerce`
stands for the recorded fixture namespace. Capture actual returned owner UIDs,
ConnectionIds and etags; symbolic occurrence names below are not submitted IDs.
Retain requests, responses, backend revision, caller/authorized namespaces,
branch, checked scope, completeness and storage changes for assertions.

Create targets first, then source owners, then responsibility/Decision/Document
subjects using returned selectors. A known same-incarnation target can be used
in a guarded final-overlay batch. Do not predict the UID of a newly created owner.
Each fixture also satisfies unrelated existing entity admission rules.

| Exact entity | Fixture declaration |
| --- | --- |
| Component `commerce/checkout@1` | Actual deployable checkout unit. Members include an exact Processor, a UI, a Specification and Component `commerce/worker@1`. These remain raw references. |
| Component `commerce/worker@1` | Actual deployable worker unit; shares the Processor member intentionally. No exclusive hosting inference. |
| ExternalSystem `commerce/provider@1` | Third-party integration boundary, with no members. |
| Component `commerce/checkout@2` | A separate breaking-change version for exact-version assertions. |
| `checkout@1` relationships | `out_a` and `out_b` both target ExternalSystem `provider@1`, label “submits orders”, with equal doc. `local` targets Component `worker@1`. |
| `provider@1` relationships | `back` targets Component `checkout@1`, label “notifies”; `peer` targets a separately created ExternalSystem `commerce/auditor@1`. |
| `worker@1` relationships | `next` targets Component `checkout@2`. |
| Team/Person and Decision/Document | Valid existing entities select `out_a` through full ConnectionRef. Also include an independent entity-level subject for `checkout@1` to detect accidental expansion. |

The graph covers all allowed endpoint kind combinations and opposite arrows
without generating an inverse declaration automatically. No broker, network
call, runtime service, source registration or actor entity is needed.

## Required public outcomes

Implement executable backend/service tests through public authoring and query
surfaces. Name their actual requests in retained evidence; do not substitute
core-only tests for persistence, authorization or transport results.

| Case | Assertions |
| --- | --- |
| Create/read | Each relationship persists its typed target, label/doc presence, order and assigned ID. Owner UID is server-owned. `out_a` and `out_b` have distinct IDs despite equal content. |
| Direction | Outgoing checkout references contain `out_a`, `out_b`, `local`; provider references contain `back`, `peer`. Incoming provider references retain both checkout occurrences. A relation from provider to checkout does not reverse or satisfy a checkout declaration. |
| Reference provenance | Each hit has exact `from`, `from_uid`, `connection_id`, typed `to` and current diagnostic field path. Reordering changes paths where needed, preserving IDs. No member reference acquires a ConnectionId. |
| Exact versions | Incoming lookup of `checkout@1` finds `back`; lookup of `checkout@2` finds `next`. No latest, superseding-version or version-family match. |
| Subject lookup | Responsibility, Decision and Document direct/reverse queries for `out_a` return only exact matching declarations. `out_b`, owner entity, endpoint entity and member selections never inherit those matches. |
| Subject context | Resolver checks complete persisted inventory and occurrence role. A Component/ExternalSystem owner alone does not authorize a member/index/ancestry connection subject. Target hydration is separate from identifying a retained occurrence. |
| Impact | Returned paths retain relationship provenance; declared cycles terminate under existing traversal bounds. Owner-level impact need not enumerate every parallel path. It reports graph reachability without a runtime-dependency claim. |
| Native export/reapply | In the same store, preserve UID/local IDs, typed refs, exact versions, labels/doc presence, ordering and unchanged etags on no-op. Truncation cannot be passed as a complete reapply artifact. |
| Native surfaces | Guarded authoring, validation/preview and inspection through CLI and MCP preserve full selectors, errors and incomplete state. Record native backend transport evidence independently of any mocked client tests. |
| Revision floor | Old writer revisions are refused before they can drop relationships. Current writers preserve fields on unrelated edits. |

## Admission and lifecycle matrix

Each rejected mutation must leave owner content, etag, UID, IDs and branch state
unchanged. Test observable stored effects after rejection, not just an error code.

| Input or operation | Expected result |
| --- | --- |
| Missing/multiple target alternatives; unsupported kind; malformed or nonpositive exact Id | Atomic rejection. No generic EntityRef or source-handle fallback. |
| Self-target exact kind/key/version | Reject; different versions are distinct targets. Cycles between different endpoints remain valid descriptive declarations. |
| Missing/blank/over-limit label; present blank/over-limit doc | Reject under CatalogTitle/CatalogText rules. Valid Unicode at encoded-byte limit succeeds; limit plus one fails. |
| Repeated equal target/label/doc | Accept distinct IDs, including parallel occurrences within one owner; never deduplicate by content. |
| New-owner supplied ID | Accept valid non-nil lowercase hyphenated UUID; reject malformed/duplicate IDs, including duplicates across roles. |
| Existing-owner unknown supplied ID; target change retaining ID; identity stripping in edited content | Reject; never match by index, label or target. |
| Reorder or label/doc edit | Preserve IDs and subject resolution. Label wording is descriptive, not a role/target change. |
| Add occurrence | Omit ID; backend returns fresh ID. Preview ID is unreserved and must not be reused as an active ID on apply. |
| Remove/re-add same content in separate guarded writes | Removed selector becomes unresolved; new occurrence receives fresh ID; sibling subjects still resolve. |
| Exact-content id-less manifest reapply | No-op exception preserves stored IDs/UID/etag. An edited id-less manifest rejects. |
| Owner delete/recreate | New UID prevents old ConnectionRef resolution even if a local ID is supplied again. Plain endpoint refs retain existing exact-key recreation semantics. |
| Clone/new version | New owner UID and exact owner key; incoming selectors stay on original. Local IDs may survive only in their new owner scope. |
| Missing/unreadable target on new or edited owner | Reject without hidden detail; inspect retained dangling owners independently. |
| Force-delete target | Preserve readable source declaration/ID and authored selector; target hydration is unavailable. Unrelated owner writes remain possible, but editing the dangling source requires repair. |
| Remove selected local relation | Existing incoming subjects become unresolved without retargeting to `out_b`. Safe deletion of the containing owner remains protected. |
| Retarget provider key under guards | Retargeted relationships receive fresh IDs; unaffected siblings retain IDs. Incoming ConnectionRefs to changed relationships remain unresolved until explicitly replaced. |
| Safe delete | Existing relationship and typed subject referrers block deletion. Hidden referrers block generically without hidden hydration. Final batch deletes consider all requested removals/remaining referrers. |
| Stale etag/preview/guard | Refuse or revalidate under guarded publication; no stale admission can publish an invalid endpoint or partial batch. Use deterministic write gates, not timing sleeps. |

Branch tests must independently cover retained-ID edits, baseline-only removal,
branch-only addition, ordinary landing/rebase refusal to revive removed IDs,
explicit keep/take resolution renewing only restored IDs, whole-owner baseline
deletion with explicit branch restoration, and independent same-key creation
with distinct UIDs. Assert persisted identities and old selector outcomes after
each operation against the recorded branch base.

## Authorization, bounds and failures

Use separate owner and target namespaces with Reader, Writer and hidden-target
callers. Include hidden referrers outside the visible listing scope. Verify
absence of hidden labels, paths, titles, counts and existence in error payloads,
query totals, hydration and delete reports.

| Case | Expected result |
| --- | --- |
| Reader mutation; hidden direct read | PERMISSION_DENIED; NOT_FOUND respectively. Catalog responsibility grants no additional permission. |
| Unreadable selected subject | Query fails without existence disclosure. A readable source can show its already authored target selector, with no hidden hydration. |
| Missing versus hidden target hydration | Same generic unavailable result for the restricted caller. Authorized stale UID/removed-ID diagnostics remain specific where safe. |
| Source write with target reference access | Succeeds without granting target write access. Authoring the opposite arrow still requires write access on its source owner. |
| Relationship ceiling | 1,024 valid relationships fit if other limits permit; 1,025 reject atomically. |
| Entity ceiling | Entire encoded entity at 1 MiB is admissible if other limits permit; above rejects. Lower existing ceilings remain effective. |
| Discovery rows | Component/ExternalSystem share the existing 5,000-row resource ceiling with other covered resource kinds. Cross-kind work cannot reset the budget. Exact authorized hydration still works beyond a display cap. |
| Occurrence/output ceilings | Shared 10,000 examined occurrences and 4,096 emitted records per request; exercise boundary and overflow with equal-endpoint repetitions and nested subject lookups. Internal scan pages and batch members cannot restart counters within a request. A new independent request receives fresh counters but its continuation token cannot bypass required complete-scan checks. |
| Discovery overflow | RESOURCE_EXHAUSTED, never an apparently complete empty list. Existing bounded impact/export/report incompleteness remains explicit and cannot establish absence. |
| Guard/report independence | Hidden or display-clipped referrers still protect deletion through the complete guard. Incomplete/failed guard refuses deletion. Preserve `referrers_incomplete` reporting contract without using a report as the safety decision. |
| Injected decode/backend/auth-service/timeout failure | Operational failure propagates; no successful absence result, model-error disguise or partial publication. |
| Source integration isolation | Reject source mapping/adoption of new occurrences, ownership of this field and whole-owner writes with current/proposed architectural relationships. Reject every Source create containing relationships, including a title-only mapping; assert no native entity or mapping-base publication. Existing-owner supported unrelated field-specific writes preserve local declarations. RELATIONSHIPS enum presence alone is insufficient admission. |

## Grouping and human behavior boundaries

These are negative acceptance assertions for the selected scope, not acceptance
of a future interaction feature.

- Component membership still accepts supported exact entity references and
  preserves shared members. No abstract group kind, transitive membership,
  exclusivity, membership occurrence or behavioral validation rule is introduced.
- ExternalSystem still has no authored members. A “contains” relationship is
  only wording, never containment, and no Component proxy is synthesized.
- Persona remains human, with descriptive `role`; no directory Person, principal,
  agent or automation identity is inferred from it.
- Inspect existing CommandSlice persona/UI/command edge refs and UiSlice
  persona/UI/source-ReadModel refs using exact current types. Repeated equal
  source ReadModelEdge occurrences remain individually selected; storyboard
  recurrence retains order. No `InteractionSlice` or `UI.persona` path is emitted.
- A label such as “sends” produces no Processor, AutomationSlice, event producer,
  API provider exposure, Channel binding, message access or checked behavioral path.
- Empty declarations prove only absence of authored declarations in the checked
  view. Accepted declarations and resolved model references prove no runtime call.

## Completion evidence boundary

Implementation completion requires maintained executable fixtures for the
selected native contract, public transport assertions and retained outputs for
these cases. Record test names/commands and their actual results at that time.
This document itself is a specification and reports no passing tests.

Studio inspection/browser evidence, when supplied, must identify the native
backend, fixture and exact selectors. Mocked transport does not establish native
persistence. Runtime observations require separate deployment/instrumentation
evidence; no runtime proof is needed or supplied by this declaration contract.

## Executable coverage inventory

The implementation fixtures below exercise different boundaries. A fixture's
presence is source evidence; its test result must be recorded separately.

| Fixture | Covered boundary |
| --- | --- |
| [Core relationship tests](../../rsworkspace/crates/trogon-atlas-core/tests/architectural_relationships.rs) | Value validation, exact typed endpoints, occurrence inventory and lifecycle, retargeting, subject resolution and Source ownership restrictions. |
| [Backend relationship tests](../../rsworkspace/crates/trogon-atlas-server/tests/system_relationship_backend.rs) | Service persistence, atomic final-overlay admission, reference provenance, safe/force deletion, branch lifecycle and nested discovery budgets. |
| [Authenticated composed fixture](../../rsworkspace/crates/trogon-atlas-server/tests/system_relationship_acceptance.rs) | Public gRPC with a real Store, parallel declarations alongside raw membership, forward/reverse provenance and exact Document/Person subjects preserved through reorder and wording edits. |
| [Source backend tests](../../rsworkspace/crates/trogon-atlas-server/tests/source_integration_backend.rs) | Native-only isolation during Source creation and updates, with retained local relationships. |
| [Client manifest tests](../../rsworkspace/crates/trogon-atlas-client/tests/architectural_relationship_manifest.rs) | Explicit endpoint shape, native manifest decoding and preservation. |
| [CLI integration tests](../../rsworkspace/crates/trogon-atlas-cli/tests/architectural_relationship_cli_integration.rs) | Native manifest authoring through the CLI. |
| [MCP integration tests](../../rsworkspace/crates/trogon-atlas-mcp/tests/mcp_integration.rs) | Native tool authoring and exact query selectors through MCP. |
| [Studio declaration tests](../../tsworkspace/webapps/studio/src/components/ArchitecturalRelationshipDetails.test.tsx) | Loaded-view declaration inspection, exact endpoint navigation and parallel occurrences, using component fixtures. |

These fixtures do not add abstract grouping, ExternalSystem-owned membership
or actor aggregation. Studio component fixtures do not establish a deployed
browser workflow or native persistence. Their aggregate presence does not
establish that every requirement above has been executed.
