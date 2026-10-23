# Source integration acceptance

Status: proposed acceptance requirements for the
[source integration contract](source-integration-contracts.md). Source control
records and source run operations are not implemented by this document. Linked
native fixtures provide existing anchors and examples of native behavior;
they do not prove source registration, snapshot retention, reconciliation,
coordinated publication, cancellation or fidelity reporting. Every source case
below requires future executable evidence.

## Connected fixture inputs

Capture a bounded native manifest/file bundle from the existing fixtures, keeping
each fixture's namespaces separate. Fixture composition is a future acceptance
extension, not an assertion that these independent tests share a running store.
All identities below have numeric version 1. Record exact kind and namespace;
display names never substitute for returned local IDs or owner UIDs.

| Existing source | Exact identities and required capture |
| --- | --- |
| [Specification fixture](../../rsworkspace/crates/trogon-atlas-server/tests/specification_fixture.rs) | Namespace `contracts`: ReadModels `order-request`, `order-receipt`; Specification `order-api` and `shared-contract`; Components `public-gateway`, `partner-gateway`, `checkout`. Capture request/response/notification roles, separate message mappings, selected/undeclared providers and external type pins from persisted results. The `contract` helper retains `order.graphql` with exact UTF-8 bytes `type Order { id: ID! }` followed by newline; capture its verified digest and returned member IDs. |
| [DataProduct fixture](../../rsworkspace/crates/trogon-atlas-server/tests/data_product_fixture.rs) | Namespace `product-fixture`: ReadModels `submitted`, `receipt`; Event `placed`; Command `place`; Processor `adapter`; slices `intake`, `handle`, `project`, `observe`; Channel `delivery`, DataStore `warehouse`, Specification `api`, DataProduct `orders`. Capture both repeated `observe` ReadModel observation occurrences, their paired Processor, Channel binding IDs, exact ReadModel attachment, INPUT/OUTPUT ports, access selections and declared lineage independently. |
| [Responsibility fixture](../../rsworkspace/crates/trogon-atlas-server/tests/responsibility_fixture.rs) | Namespace `responsibility-fixture`: ReadModel `receipt`, Processor `notifier`, AutomationSlice `observe`, Specification `receipt-contract`, DataProduct `receipts`, Person `maintainer`, Teams `receipts-team`, `support-team`. Retain repeated observation selections, MESSAGE and OUTPUT port selectors, explicit assignment kinds and separate membership identity. |
| [Decision/Document fixture](../../rsworkspace/crates/trogon-atlas-server/tests/decision_document_fixture.rs) | Separate extension in `responsibility-fixture`: Decision `receipt-delivery-choice`, Document `receipt-delivery-note`, anchor fragment `delivery`. Capture accepted option, rationale, consequences, subject associations, retained Markdown bytes/digest and full anchor selection. Fragment and option text are not durable IDs. |
| [Ecommerce fixture](../../rsworkspace/crates/trogon-atlas-server/tests/phase10_ecommerce_fixture.rs) | Namespace `ecommerce`: Storyboard `order-to-fulfillment` with `place-order-slice`, `orders-list-slice`, `auto-capture-payment`, `capture-payment-slice` and subsequent authored steps. Preserve the full persisted order. Add an explicitly authored repeated slice occurrence in the source acceptance extension to test recurrence without inventing moment IDs. |

Capture each owner's returned UID, local IDs, association IDs, etag, branch,
source bytes, resolved tenant type context and authorized scope before making
selectors. Native exports omit system metadata, so retain observed identity
evidence separately in the proposed control records. Do not fabricate UUIDs or
derive them from fixture array indices. Verify supplied bytes and complete
inventory independently of native semantic equality.

The proposed source fixture adds a registration and immutable snapshot with
explicit opaque source version text, complete partition declarations and typed
source-to-native mappings. Add separate partial snapshots, unsupported retained
files, ordered repeated source pointers, ambiguous mappings, a foreign readable
namespace and a hidden target/referrer via authenticated harnesses. These are
new source test inputs, not existing native features. Mutation cases use isolated
stores/branches or reset fixtures and retain exact returned identities.

## Input, retention and behavioral authority

| Case | Required outcome |
| --- | --- |
| Supplied immutable native bundle | Verify exact bytes, logical names, media types, inventory/coverage and bundle digest; never read mutable input again after preview. |
| Digest encoding | Retain golden inventory/file byte vectors and expected hashes for the framed `supplied-native-v1` encoding. File-list reorder preserves digest; inventory order/whitespace, bytes, media type, version, coverage or logical name edits change it. Duplicate JSON keys or logical names reject. |
| Same registration/version and identical snapshot | Idempotent retention. Same version with changed byte, scope, coverage, pointer, relation order or profile identity rejects as conflict. |
| Different source version, identical bytes | Retain independently addressable source revision. No automatic native numeric version or `supersedes` change. |
| Distinct opaque contract versions | Preserve spelling/case and explicit mapping decisions. No semver cast, latest fallback or automatic consumer retarget. Conflicting ownership of one native unit rejects. |
| Link, absolute/traversal path, network pointer, executable content or archive expansion beyond budget | No fetch, host path access or execution. Reject invalid bundle locators/unsupported packaging; permitted inert content remains retained with explicit disposition. |
| Unsupported declaration or partial relation | Keep original bytes/pointer in authorized overlay and report DEFERRED or another exact status. Create no phantom native entity, edge, member or endpoint. |
| Repeated equal source pointers | Distinct source occurrence identities and authored order. Reordering preserves known occurrence continuity; absent stable keys cannot be rematched by index or endpoint. |
| RESOLVED/MISSING/AMBIGUOUS/DEFERRED/CONFLICT | Exercise every status with checked context and authorized candidates. Failed, hidden or incomplete lookup cannot produce a verified MISSING or RESOLVED result. |
| Request, response and notification messages | Preserve individual ReadModel mappings and operation roles. No source label creates Command/Event behavior. |
| Channel and DataStore | Preserve exact observation-to-Processor context and exact ReadModel attachments. Source transport/storage labels alone cannot supply missing anchors. |
| DataProduct, Team/Person and Decision/Document | Preserve declared versus checked lineage, exact access, explicit responsibility, membership, rationale and document selections. Source ownership grants no native authority, approval or runtime guarantee. |

## Identity and local selection

| Case | Required outcome |
| --- | --- |
| Entity kind/key/version and source version mapping | Exact typed selection; no same-title, numeric cast, family or latest matching. Record observed incarnation separately from plain native key semantics. |
| Repeated observation reorder/removal | Preserve both IDs through reorder; removal breaks only exact selected occurrence dependencies. Retain other equal-endpoint occurrences independently. |
| Paired Processor or ReadModel retarget | Renew affected native occurrence identity and Channel binding as required; never follow the old observation by endpoint. |
| Member rename/reorder versus mapping retarget | Preserve member ID; renew changed association ID. Wrong owner UID/member kind/ID rejects. MESSAGE membership does not freeze its ReadModel mapping. |
| Artifact/type pins | Verify exact bytes, artifact identity, digest and opaque selector including absence. Changed artifact bytes/format/media/version require a new artifact member; old external type selection never silently follows. |
| Port rename/reorder versus data/access/lineage retarget | Preserve port where allowed, replace changed associations, check INPUT/OUTPUT direction and full access pins. Wrong direction, UID or port ID rejects. |
| Document content/label/order versus fragment edit | Preserve authored anchors for content/label/order; byte-pinned selection goes stale after content change. Fragment change uses fresh anchor ID. Whole document and anchor selection remain distinct. |
| Remove/re-add local identity | Fresh identity, with separate writes where native admission requires them. No resurrection by name, endpoint, fragment, bytes or position. |
| Existing owner addition | Omit new IDs, persist, then use authoritative returned IDs for dependent follow-up. Preview IDs are not active or reserved. |
| Same-key recreation | Plain native references keep key semantics; active source mapping becomes stale until explicit adoption. All old connection/member/type/port/anchor incarnation pins fail. |
| Unsupported local subject | No new native selector inventory for source pointers, Decision options, Team membership, raw reference lists, arbitrary document spans or repeated storyboard positions. Retain reconciliation evidence without inventing subject kinds. |

## Complete and partial inventory

| Case | Required outcome |
| --- | --- |
| Partial snapshot omits previously mapped resource/relation | Retain native item and accepted mapping; omission means unknown. Never delete or clear through omission. |
| Complete source owner payload in partial partition | Apply only its explicitly complete native owner semantics; partition omission still cannot prune other owners. Partial patches cannot masquerade as complete puts. |
| Complete partition removes previously accepted item | Preview retirement/deletion candidate only within the same exact base partition. Newly broadened scope has no implicit deletion base. |
| Complete partition, local fields or another source owns part of native owner | Retire/detach source mapping or conflict; retain native owner. No whole-owner deletion without exclusive ownership and explicit policy. |
| Complete partition, exclusively source-owned native deletion enabled | Explicit guarded safe delete after full final-overlay dependency checks. Omission alone never force-deletes. |
| Inventory truncated, skipped, filtered, unauthorized or decode failed | Preserve incomplete/error state; reject complete-inventory and deletion claims before writes. Empty pages or display cap absence prove no deletion. |
| Remove selected local identity with external referrers | Follow native local-removal rules, preserve exact incoming declarations unresolved and show authorized impact. Edited owners still require complete validation. |
| Safe batch deletes referrers and targets | Evaluate all requested targets in the final overlay, including tombstones. Hidden surviving referrer blocks without disclosure; reporting caps never weaken the guard. |
| Native owner deleted outside source runs | Retain historical source evidence without blocking native deletion through provenance. On next check mark active mapping stale; no implied watcher or automatic recreation. |
| Registration/mapping detach or deletion | Relinquish future source mutation authority; keep native resources. Registration deletion cannot discard recovery evidence and is refused while a possible native commit remains unsettled. Historical retention follows explicit policy, with no comprehensive purge claim. |

## Ownership, conflicts and branches

| Case | Required outcome |
| --- | --- |
| Existing namespace and native resource adoption | Require explicit destination and ownership decision bound to UID/etag. No namespace creation, slug-based adoption or source-derived access. |
| Foreign dependency supplied in bundle | Authorized exact read/reference prerequisite only. No foreign write unless separately adopted and explicitly authorized. |
| Source-only change, native at base | Propose source-owned change; preserve native-owned fields and all unaffected IDs. |
| Native-only drift | Report it, preserve it and keep divergence visible. No automatic overwrite or silent base movement. |
| Both sides converge/diverge | Convergence may be a guarded no-op; divergence, edit/delete and recreation require explicit resolution and new preview. |
| Conflicting registrations or ownership transfer | Refuse contested writes; explicit guarded transfer cannot erase the prior accepted evidence. |
| Ordered collection without durable identity | Whole-unit ownership/conflict semantics. No automatic positional merge; authored recurrence/order preserved. |
| Expired/missing merge base | Fail closed for writes until explicit re-adoption. Never treat missing evidence as empty native state. |
| Branch overlay/tombstone and final batch replacement | Resolve exact selectors in the authorized merged final view; no fallback to baseline rows hidden by tombstones. |
| Rebase/land opposing-side local deletion | Compare recorded base/baseline/branch. Ordinary landing rejects resurrection; explicit restoration follows fresh-ID/local-selection rules. |
| Independent same-key creation or recreated baseline | Distinct UID conflict even for equal content. Explicit whole-owner keep-side restoration follows native surviving-incarnation rules separately. |
| Branch mapping after landing | Recheck baseline identity/guards and explicitly advance baseline mapping. Branch provenance alone cannot authorize later baseline writes. |

## Run publication, replay and cancellation

| Case | Required outcome |
| --- | --- |
| Prepare/preview/overlay-only retention | No native writes or accepted-base advancement. Overlay-only success never claims native apply. |
| Preview input/configuration/ownership/scope changes | Invalidate preview; a new immutable plan is required. |
| Create/update/delete guards | `create_only` for observed absence, `if_match` plus observed UID for existing owners, guarded mapping/base/configuration revisions and branch context. |
| Unchanged dependency changes after preview | Recheck every fact used to decide the plan, even if no put targets that owner. Existing native client skip behavior alone does not satisfy this requirement. |
| Stale UID/etag/base, permission revoked or newly hidden dependency | Refuse the entire publication. No automatic replan/source-wins retry. |
| New owner and dependent cross-owner incarnation pin in one bundle | Fail closed as unsupported initial migration. No guessed UID, preview identity, blanket pin stripping or implied atomic clone. |
| Explicit staged creation then mapping | New run uses persisted authorized identities and fresh preview/guards. Report stages independently; no combined atomic migration claim. |
| Native writes plus source records | Exercise new backend coordination/recovery, not just entity `BatchMutate`. Fault injection before/during/after native commit and evidence publication proves settled outcomes and accepted bases agree. |
| Native publication marker | Crash at each marker/native-effects boundary cannot expose effects without a committed marker or a committed marker without effects. Recovery publishes evidence once from recorded committed values, even after a later native edit; it never reapplies the batch or adopts that newer value as its base. A backend without coordinated marker publication refuses apply. |
| Recovery incomplete or result ambiguous | Retain outcome-unknown/recovery state. Never mark applied, advance the accepted base or infer rollback from missing acknowledgment. |
| Same run identity/request retry | Return retained authorized outcome without duplicate native changes, identities or base advancement. Changed request/digest/scope under the same identity rejects. |
| Native operation IDs and dry run | Outer batch ID only; dry run has no mutation receipt. Branch `GetOperation` durable receipt absence is handled by the new source protocol, never assumed away. |
| Expired or UNKNOWN native/source receipt | Not proof of nonapplication; inspect durable evidence/guards before another apply. PENDING is not successful completion. |
| Cancel before commit | No native changes or accepted-base advancement; retain cancellation disposition. |
| Cancel/disconnect during commit/recovery | Settle from durable outcome, possibly applied or outcome-unknown. No automatic undo. Any compensation is a new guarded proposal. |
| Concurrent runs | Guard/serialize common registration/base/ownership units; disjoint work remains bounded. No race advancing one accepted base over another. |

## Authorization, budgets and retention

| Case | Required outcome |
| --- | --- |
| Source reader versus administrator/apply principal | Source visibility alone grants no run mutation, native write or reference authority. Reader native mutations remain denied. Reauthorize apply, replay, cancellation and historical reads. |
| Cross-namespace mapping | Check destination write and each target read/reference independently. Hidden/missing hydration remains indistinguishable where required. |
| Hidden target/referrer/candidate | No titles, contact data, retained bytes, paths, counts or existence leaks through status, errors, reports or export. Already authored selectors do not authorize hydration. |
| Source/provenance historical pin | No native incoming-delete guard or causal edge created solely by retention. Historical context does not bypass current access restrictions. |
| Raw sensitive content/logging/rendering | No source bodies or credentials in routine logs; inert rendering, no fetch or execution. Reject prohibited content or retain a separately identified sanitized snapshot without claiming original digest fidelity. |
| Every advertised run limit | Boundary succeeds when otherwise valid; exceeding bytes/files/resources/relations/hydration/work/output/time/storage refuses before native writes. Repeated equal pointers consume work. |
| Profile and retention bounds | Exercise the exact `supplied-native-v1` ceilings and lower registration limits. Retained unknown-outcome records and identity/digest tombstones count against storage; expiry never recycles a version/run identity or proves nonapplication. Expired active base disables apply until explicit re-adoption. |
| Abandoned input expiry | Unaccepted snapshots and previews expire seven days after backend-recorded creation even if no acceptance/settlement occurs. Reads, retries or supplied timestamps cannot extend retention; releasing expired bytes restores quota without discarding protected recovery evidence or identity/digest tombstones. |
| Entity mutation ceiling | At most 1,000 operations, honoring Store below service's 1,024 ceiling. Include puts and deletes, apply lower applicable limits and bound control/recovery writes separately. Never silently chunk an oversized apply. |
| Pagination, staged work and continuations | No budget resets inside one run; explicit later run has a new preview/base and separately visible commit. |
| Guard timeout, backend/decoding failure or budget exhaustion | Operational error, never empty inventory, verified absence or mutation authorization. Capped impact/report output remains explicitly incomplete. |
| Snapshot/base/preview/receipt retention expiry | Enforce advertised finite durations/storage limits and expose lost reconstruction/replay ability. Protect needed active base/recovery evidence or refuse writes and require re-adoption. |
| Source evidence deletion | No promised purge of native history, revisions, branches, exported files or backups. No accepted-base substitution with empty state. |

## Export and reapply fidelity

| Case | Required outcome |
| --- | --- |
| Retained original file recovery | Exact byte equality/digests, media types, source version and logical layout. Native YAML rendering alone is insufficient for original comments/formatting. |
| Same-store unchanged native export/reapply | Complete scoped manifests and tenant types produce unchanged classification and no new native revisions/IDs. Retain exact selectors, pin presence, local/association IDs, artifact/Document bytes/digests and list recurrence/order. |
| Native system fields | Explicitly omitted from manifest content equality. Store observation evidence separately; do not claim owner-UID-preserving portable restore. |
| Fully unassigned connection IDs, exact same content | Existing pure connection equivalence can classify unchanged. Check Decision/Document local equivalence independently; preserve persisted IDs. |
| Partially stripped IDs or edited id-less content | Assert actual kind-specific admission/equivalence result, with no blanket id-less roundtrip claim. Reject edited retained identity stripping, never rematch by position/endpoint. |
| Foreign-store/key/incarnation reapply | New owners get new UIDs. Initial release rejects unresolved cross-owner pins; explicit mappings to already persisted authorized incarnations are checked in full. |
| Portable rebinding request | Report deferred/unsupported initial capability and exact affected selectors. Never strip UIDs, guess replacements or imply existing CLI offers coordinated migration. |
| Unknown tenant type, render failure, missing bytes or native export skipped item | Propagate loss/error with exact authorized locator. Do not label output complete or lossless. |
| Normalization, unsupported field, overlay-only relation, conflict or truncation | Structured fidelity disposition and scope retained. Raw source information is retained where permitted rather than silently discarded. |
| Snapshot digest versus native content hash | Distinguish exact input identity from semantic model content. Neither guards incarnation; truncated IDs are unusable for complete-scope claims. |
| Empty loss report | Allowed only for complete checked profile/scope with no skipped upstream data. State whether claim is native same-store reapply or exact supplied-byte recovery. |

## Required executable evidence

Future source acceptance must link case IDs to executable tests and retained
results, covering typed input/control persistence, native admission, authenticated
backend authorization, final-overlay reference checks, identity lifecycle,
coordinated recovery fault injection, budgets, retention and CLI/MCP transport.
Review surfaces must distinguish proposed, conflicted, incomplete, stale,
cancel-requested and outcome-unknown results while preserving exact selectors
and authored order. No Studio component test alone proves backend publication.

Each executed record must name the source snapshot/configuration digests, exact
native keys/UIDs/local IDs, source version, numeric native versions, branch,
scope/principal, base/current guards, requests, outcomes, completeness, bytes and
digests, fault/cancel point, native receipt/change linkage and fidelity report.
Retain temporary fixture construction source with the private run evidence.
An assembled read is not historical transactional proof; unavailable history
and retention gaps must remain visible.

Existing native export/apply, resource fixture and identity tests establish only
their exercised native behavior. Documentation checks and this proposed fixture
do not establish an implemented source runtime. Remote connector, scheduler,
watcher, portable migration, schema semantic compilation, deployed-browser and
Atlas Linter/WASM acceptance remain separate future work.
