# System grouping, membership and actor acceptance requirements

Status: acceptance requirements for the implemented
[system structure contract](system-structure-contracts.md). Native fixtures
exercise admission, identity, references, branch lifecycle, authorization,
Source isolation and connected clients. Final composed verification remains
pending. Logical names below are not manifest syntax, predicted owner UIDs or
submitted occurrence IDs. The [fixture inventory](#fixture-inventory) retains
the original acceptance organization.

## Fixture and receipts

Materialize each case in fresh registered namespaces, using exact version `1`
unless a case names another. Logical namespace `commerce` holds architecture
and `strategy` holds Domains. Capture the actual returned owner UIDs,
ConnectionIds and etags from authoritative reads. Retain requests, responses,
backend revision, caller and authorized namespaces, branch, checked scope and
completeness. Create targets first, then owners, then subjects using returned
selectors. A known same-incarnation target may be used in a guarded final-overlay
batch. Do not predict a new owner's UID.

| Exact entity | Fixture content |
| --- | --- |
| Components `commerce/checkout@1`, `checkout@2`, `worker@1` | Deployable units. `checkout@1` also has raw `members` of its own to detect accidental coupling. |
| ExternalSystem `commerce/provider@1` | Third-party boundary with no members. |
| DataStore `commerce/orders-db@1`, DataProduct `commerce/receipts@1` | Valid existing entities for member arms. |
| Personas `commerce/shopper@1`, `commerce/reviewer@1` | Human personas. A Person `maintainer` and a Team exist as unequal controls. |
| SystemGroup `commerce/platform@1` (INTERNAL) | Members: `checkout@1` twice with equal doc, `checkout@2`, `orders-db@1`, group `storefront@1`. Actor links: `shopper` INBOUND "uses", `shopper` OUTBOUND "notified by", `reviewer` INBOUND. Relationship to `payments-vendor@1` labeled "pays through". |
| SystemGroup `commerce/storefront@1` (INTERNAL) | Members: `checkout@1` and group `platform@1`, forming a two-owner cycle with `platform`. |
| SystemGroup `commerce/payments-vendor@1` (EXTERNAL) | Member: `provider@1`. |
| Domain `strategy/retail@1`, `strategy/fulfilment@1` | `retail` members: `platform@1`, `worker@1`, `fulfilment@1`, `receipts@1`. `fulfilment` member: `retail@1`, forming a cycle. |
| Subdomain, BoundedContext | Existing strategic records realizing the fixture namespaces, to prove membership and strategic scope stay independent. |
| Subjects | Team and Person responsibility, a Decision and a Document select one `platform` member occurrence by full ConnectionRef. An independent entity-level subject for `platform@1` and for `checkout@1` detects accidental expansion. |

Occurrences used by name below: `plat_a` and `plat_b` (the equal `checkout@1`
members), `plat_v2`, `plat_db`, `plat_sf`, `store_ck`, `store_plat`,
`act_in`, `act_out`, `act_rev`, `rel_pay`, `dom_sys`, `dom_cmp`, `dom_sub`,
`dom_dp`, `ful_ret`. Each must have a distinct returned ID.

## Native identity and shape

| Case | Required public outcome |
| --- | --- |
| Create and read | Each field persists its typed target, presence of label and doc, order and assigned ID. Owner UID is server-owned. `plat_a` and `plat_b` have distinct IDs despite equal content. |
| Shared membership | `checkout@1` appears under `platform` and `storefront`. Incoming references to `checkout@1` return both containing owners and every repeat. No second copy of `checkout@1` exists. |
| Exact versions | `checkout@1` and `checkout@2` are both members of `platform` as separate occurrences. Incoming lookup of `checkout@2` returns only `plat_v2`. No latest, family or superseding-version match. A member version above 2^53 round-trips losslessly. |
| New owner version | `platform@2` created from `platform@1` has a new UID. Members are retained as exact targets. Old ConnectionRefs stay on `platform@1`. |
| Typed ref shape | Missing, unknown or multiple oneof alternatives, nonpositive versions and a non-numeric version reject atomically. A generic EntityRef or source handle never substitutes. |
| Member-kind closure | Schema and DesignGraph members are admitted in both owner kinds, and agent Components remain exact Components. Only genuinely unlisted target kinds reject. |
| Scope | Unspecified or unknown scope rejects. EXTERNAL scope may retain authorized Component and DataStore members without claiming ownership. INTERNAL listing an ExternalSystem is accepted. Changing scope preserves IDs but re-runs member-kind admission. |
| Domain membership | `dom_sub` is a Domain member, not a Subdomain. A member equal to the Domain's own kind, key and version rejects. Domain-scoped listing results do not change when `Domain.members` changes. |
| Strategic independence | Adding or removing authored members changes no `ListEntitiesByDomain` result, no BoundedContext realization resolution and no project-validation scope. |
| Component unchanged | `Component.members` still accepts the existing kinds and preserves shared members and its unidentified paths. It accepts a known SystemGroup target under the same raw EntityRef rules. No membership occurrence ID appears on Component members. |

## Cycles and traversal

| Case | Required public outcome |
| --- | --- |
| Self membership | A group listing itself at the same kind, key and version rejects with owner content, etag, UID and IDs unchanged. A different version of the same key is accepted. Same for Domain and for group relationships. |
| Two-owner cycle | `platform` to `storefront` and back is accepted in separate guarded writes and also in one batch. No write reads or discloses a third owner. |
| Hidden cycle part | A writer who cannot read one half of a would-be cycle receives the same outcome as one who can. The write result does not reveal the hidden half's existence. |
| Concurrent halves | Two writers each add one half under independent guards. Both can publish; neither is wrongly rejected, and neither observes a partial publication. |
| Reference queries | Outgoing and incoming results list each occurrence once and terminate. `store_plat` and `plat_sf` appear exactly once each. |
| Impact | Reverse traversal from `checkout@1` terminates under existing bounds across the cycle, carries occurrence provenance on every path hop and makes no runtime-dependency claim. |
| Validation | Proposed advisory findings report the `platform` and `storefront` cycle, the `retail` and `fulfilment` cycle and the exact repeat `plat_a`/`plat_b`, citing readable ConnectionRef paths. A caller who cannot read a participant sees a finding without the hidden participant. |
| Client expansion | `system_group_acceptance::fixture_client_expands_exact_members_once_and_marks_back_edges` uses authenticated native reads to expand `platform` with a visited set, expands `checkout@1` once though reachable twice, keeps `checkout@2` distinct, and marks the exact `store_plat` occurrence as a back edge. |
| Findings do not block | Writes proceed with advisory findings present. Clearing a finding must not delete the authored cycle. |

## Actors and Event Modeling boundaries

| Case | Required public outcome |
| --- | --- |
| Direction | Required. Unspecified or unknown rejects. `act_in` and `act_out` coexist for one Persona with distinct IDs and do not satisfy each other. |
| Direction is semantic | Changing `act_in` to OUTBOUND requires a fresh ID; retaining the old ID rejects. Label and doc edits preserve IDs. |
| Typed actors | Human Persona, software Component, ExternalSystem and SystemGroup targets preserve their exact types. Person, principal and unsupported Processor targets reject; software actors do not create Personas. |
| No identity inference | An actor link creates no Person, principal, role, permission or Persona. Persona `role` is unchanged and still descriptive. No `UI.persona`, interaction slice or Persona-to-Component reference is emitted. |
| No membership inference | A Persona in `platform.actors` gives `platform` no members and gives the Persona no containment. Removing the Persona's last slice leaves the link readable. |
| Dangling persona | Force-deleting `shopper` leaves `act_in` readable with an unavailable target, and unrelated `platform` writes still succeed. Editing the dangling link requires repair. |
| Grounding recipe | After hosting is implemented, a client assembles: link `act_in`, a membership chain from `platform` to `checkout@1`, and the Component's hosting plus a checked slice path with the same Persona. Each piece reports its own checked row receipts. The result is labeled client-assembled and carries no backend certification. |
| Grounding negatives | Persona mismatch by key or version, a membership chain with a broken link, a hosting owner UID that differs from the resolved Component, and a Component recreated at the same key all fail to ground. OUTBOUND links have no grounding alternative. |
| Membership alone | `checkout@1` raw `members` matching UI or Persona never grounds an actor claim. Navigation, Screen composition and storyboard recurrence supply no substitute. |
| Separate declarations | Communication intent, architectural labels and a shared Decision or Document subject list create no composite interaction subject. |
| Hosting untouched | No `ui_hostings` field exists on SystemGroup. ExternalSystem hosting remains rejected as unsupported. Hosting acceptance outcomes are unchanged. |

## Lifecycle matrix

Each rejected mutation must leave owner content, etag, UID, IDs and branch state
unchanged. Assert stored effects after rejection, not only the error code.

| Input or operation | Expected result |
| --- | --- |
| New-owner supplied ID | Accept a valid non-nil lowercase hyphenated UUID. Reject malformed, empty-present or duplicate IDs, including duplicates across `members`, `relationships` and `actors`. |
| Existing-owner unknown supplied ID, fingerprint change retaining ID, identity stripping | Reject. Never match by index, label or target. |
| Reorder, doc or label edit | Preserve IDs and subject resolution, including equal siblings. |
| Add occurrence | Omit ID; the backend returns a fresh one. A preview ID is unreserved and rejected as an active ID on apply. |
| Remove and re-add equal content in separate writes | Removed selector becomes unresolved. The re-added occurrence has a fresh ID. Siblings resolve. |
| Exact-content id-less reapply | No-op preserving UID, IDs and etag. An edited id-less manifest rejects. |
| Owner delete and recreate | New UID. Old ConnectionRefs stay unresolved even with a reused local ID. Plain member refs follow the exact key. |
| Retarget of a member key | Guarded retarget renews the IDs of changed occurrences and leaves siblings. Subjects on old IDs stay unresolved until replaced. |
| Safe delete | Existing member, relationship and actor referrers, and typed subject owners, block deletion. Hidden referrers block generically. Final batch deletes consider all requested removals. |
| Force delete of a member | Readable authored selector and ID remain; hydration reports unavailable. |
| Stale etag, preview or guard | Refuse or revalidate under guarded publication. Use deterministic write gates, not sleeps. |
| Revision floor | Writers below the new floor are refused before they can drop the kind or `Domain.members`. Current writers preserve fields on unrelated edits. |

Branch tests independently cover retained-ID edits, baseline-only removal,
branch-only addition, ordinary landing refusing to revive removed IDs, explicit
keep or take renewing only restored IDs, whole-owner baseline deletion with
branch restoration, and independent same-key creation with distinct UIDs. Assert
persisted identities and old selector outcomes after each operation.

## Subjects, references and queries

| Case | Assertions |
| --- | --- |
| Subject lookup | Responsibility, Decision and Document direct and reverse queries for `plat_a` return only that occurrence. `plat_b`, the owner, the member entity and the containing Domain never inherit it. |
| Subject context | The resolver checks the complete persisted inventory and role. An admitted owner kind alone does not authorize a raw Component member, index or ancestry path as a connection subject. |
| Reference provenance | Each hit has exact `from`, `from_uid`, `connection_id`, typed `to` and current diagnostic path. Reordering changes paths and preserves IDs. Component raw members acquire no ConnectionId. |
| Safe selection | Selecting an occurrence asserts neither valid hydration nor containment truth. Target availability is reported independently. |
| Native export and reapply | Same-store export, diff and reapply preserve UID, IDs, typed refs, versions, scope, direction, presence and ordering with unchanged etags on no-op. Truncated export is not a complete reapply artifact. |
| Surfaces | Guarded authoring, validate-only preview and inspection through CLI and MCP preserve full selectors, uint64 versions and incomplete state. Validate-only preview proves no write. |
| Studio | Read-only review retains selectors, owner UIDs, etags and unavailable state, and marks loaded-only correlation separately from backend checks. |

## Authorization, bounds and failures

Use separate owner and target namespaces with Reader, Writer and hidden-target
callers, and hidden referrers outside the visible listing.

| Case | Expected result |
| --- | --- |
| Reader mutation; hidden direct read | PERMISSION_DENIED; NOT_FOUND. Responsibility grants no permission. |
| Member reference access | A writer with owner write and target reference access succeeds without target write. Without reference access it rejects without hidden detail. |
| Persona in a hidden namespace | Missing and hidden Persona targets share one generic unavailable state for a restricted caller. |
| Hidden member hydration | A restricted caller sees authored selectors but no hidden title, UID, etag, count or existence, and no descent through a hidden group. |
| Foreign or deleted branch | NOT_FOUND. Never silently falls back to baseline or another branch. |
| Collection ceilings | 1,024 members, 1,024 relationships and 1,024 actor links fit when other limits permit; one more rejects atomically. Domain members share the 1,024 bound. |
| Entity ceiling | Whole prepared entity at 1 MiB is admissible if other limits permit; above rejects. Lower limits win. |
| Discovery rows | SystemGroup counts in the shared 5,000 visible resource-row ceiling, once per exact owner and view. |
| Occurrence and output ceilings | 10,000 examined occurrences and 4,096 emitted records per request; exercise boundary and overflow with equal repeated members and cycle participants. Pages and batch members do not reset counters. |
| Overflow | RESOURCE_EXHAUSTED, never an apparently complete empty list. |
| Guard and report independence | Display-clipped or hidden referrers still protect deletion. An incomplete guard refuses deletion. `referrers_incomplete` reporting is unchanged. |
| Injected failures | Decode, backend, authorization-service and timeout failures propagate; they never become absence, a model error or partial publication. |
| Source isolation | Source mapping or creation of a SystemGroup rejects. Source create of a Domain with members rejects under every ownership policy including title-only. Whole-owner Source writes with nonempty current or proposed members reject, including a clear. Unrelated field-specific updates preserve local members. The existing members enum value alone grants nothing. No native or mapping-base publication follows a rejection. |

## Negative scope assertions

- No importer, migration, provider sync or federation behavior is implied or
  needed to run these cases.
- Membership proves no deployment, ownership, exclusivity, hosting, permission,
  message flow or runtime dependency. A relationship labeled "contains" is only
  wording.
- ExternalSystem still has no members; no Component proxy is synthesized.
- Persona is not a principal; no actor kind or Person coupling exists.
- Namespace-derived domain scope is unchanged by authored membership.
- Empty collections prove only that nothing is authored in the checked view.

## Fixture inventory

These maintained fixtures cover different boundaries. Record their results
separately at the composed candidate.

| Fixture | Boundary |
| --- | --- |
| `rsworkspace/crates/trogon-atlas-core/tests/system_group.rs` | Value validation, member-kind closure, fingerprints, resolver roles, extraction, cycles and Source restrictions. |
| `rsworkspace/crates/trogon-atlas-server/tests/system_group_backend.rs` | Persistence, final-overlay admission, references, delete guards, branches, budgets and operational failures. |
| `rsworkspace/crates/trogon-atlas-server/tests/system_group_acceptance.rs` | Authenticated gRPC with a real Store over the fixture above, including cycles and shared membership. |
| `rsworkspace/crates/trogon-atlas-server/tests/source_integration_backend.rs` additions | Native-only isolation during Source create and update. |
| `rsworkspace/crates/trogon-atlas-client/tests/system_group_manifest.rs` | Typed shapes, lossless uint64 and presence. |
| CLI and MCP integration additions | Native authoring and exact queries through each surface. |
| Studio component test | Loaded-view inspection and unavailable state, using component fixtures. |

## Completion evidence boundary

Implementation completion requires maintained executable fixtures and retained
commands and results at the candidate SHA. Mocked gateways and component
fixtures do not establish native persistence, authenticated transport or
deployment. Browser evidence names its backend and fixture. Runtime behavior
needs separate instrumentation; none is supplied or required by declarations.
