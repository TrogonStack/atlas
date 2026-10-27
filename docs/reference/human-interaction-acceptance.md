# UI hosting and human interaction acceptance requirements

Status: proposed requirements for the
[human interaction contract](human-interaction-contracts.md), not implemented
or executed coverage. No passing fixture, schema change or deployed readiness
is claimed. Logical names below are not manifest syntax, predicted owner UIDs
or submitted occurrence IDs.

## Fixture and receipts

Prepare native Component `portal`, UI `review`, human Persona `reviewer`, Command
`approve` and ReadModel `summary`. A CommandSlice selects reviewer/review/approve;
a UiSlice selects reviewer/review and repeated equal summary observations.
Add another UI, Persona, Command and ReadModel as unequal controls. Component
members may include matching elements, but must not supply hosting identity.

Create equal UI-hosting assertions and retain their actual returned Component
UID and distinct connection IDs. Retain actual slice owner UIDs and every
selected edge ID from authoritative reads. Use explicit versions, including
values above JavaScript's exact integer range. Record branch, authorization
context, request and checked row receipts with each assertion.

Establish native persistence and exact reference discovery before client UI
checks. Preserve causal public RED/GREEN evidence for discovered defects.
Component or mocked HTTP fixtures cannot replace real Store, authenticated
transport, CLI/MCP or deployed-browser receipts.

## Native hosting and identity

| Case | Required public outcome |
| --- | --- |
| Create/read | Proposed fields preserve presence, Unicode and order. Equal UI assertions receive distinct IDs. Server owns Component UID. |
| Supplied IDs | Omitted ID assigns; explicit empty/malformed IDs reject. Duplicate IDs across hostings, relationships and communications reject atomically. |
| Wording and reorder | Labels/docs and list order may change without renewing IDs or collapsing equal siblings. Checked wording still needs its etag. |
| Semantic retarget | Exact UI target change requires a fresh occurrence ID; a retained old ID rejects. Old subjects do not follow the new occurrence. |
| Remove/re-add | Removed selector stops resolving; later equal assertion receives a fresh ID. No deleted-ID resurrection through ordinary writes. |
| Same-store native reapply | Export/diff/reapply preserves stored UID, IDs and etag under the exact-content no-op rule. Cross-store migration is not claimed. |
| Preview and concurrency | Preview does not publish/reserve IDs. Stale if-match guards fail; cached candidates must satisfy current guards and final batch/branch admission. No partial publication. |
| Branch lifecycle | Same-incarnation branch edits/landing preserve active identity. Explicit restored conflict IDs renew under existing rules; ordinary deleted-ID revival rejects. |
| New owner incarnation | Clone/version/recreation uses a new UID scope; old hosting ConnectionRefs do not follow it. |
| Reference provenance | Exact outgoing/incoming sets retain owner UID, selected occurrence ID, typed UI target, diagnostic paths and repeated multiplicity. Raw members remain unidentified. |
| Reference boundary | Hosting stores only Component-to-UI references. Inspection and retained query receipts create no stored references or deletion guards for Persona, Command or ReadModel selections. |
| Delete safety | Complete safe-delete guard protects UI and typed subject owners, including hidden referrers. Force deletion leaves unresolved selectors; reporting caps never authorize deletion. |

## Checked human paths

| Case | Required public outcome |
| --- | --- |
| Human issuance | Supplied hosting plus same-CommandSlice persona/UI/command tuple resolves to the selected human path and readable endpoint receipts. |
| Display observation | Same-UiSlice persona/UI/one source ReadModel tuple resolves only that selected observation. Equal-target sibling has a different selector and separate evidence. |
| Wrong role or owner | IDs for an event edge, another role, different slice key/version or different owner UID never substitute. Malformed request rejects; well-shaped stale/role-incompatible selection cannot produce a resolved path. |
| Persona edge alone | A persona-only request rejects; endpoint equality cannot reconstruct omitted companions. |
| UI mismatch | Hosting UI differs by kind/key/version from selected slice UI: no resolved path, even if labels or Screen slots match. |
| Partial human slice | Missing persona/UI/command or selected observation does not produce human issuance/display. AutomationSlice is unsupported for this request. |
| Slice context edit | Retarget command/persona/UI or selected ReadModel under existing fingerprint rules. Old tuples containing renewed/removed edge IDs become unresolved; fresh authoritative tuples can resolve. |
| Unrelated slice edit | Editing unselected emitted events, wording or an equal observation sibling preserves resolution where existing selected fingerprints/IDs remain active. Return current checked etag. |
| Observation reorder | Reorder repeated source ReadModels, preserving IDs. Selection follows its ID, not old list position. |
| Endpoint recreation | Force-delete/recreate the same exact UI/Persona/Command/ReadModel key. Plain references may resolve the replacement; report its new UID/etag and do not claim prior incarnation binding. |
| Owner recreation | Recreated Component or slice at the same key has a different UID. Old hosting/path ConnectionRefs remain unresolved despite equal content or locally reused IDs. |
| Membership-only control | Matching raw members without a selected hosting occurrence cannot pass inspection. Navigation, Screen composition and storyboard recurrence create no substitute path or IDs. |
| Separate declarations | Communication intent and architectural labels do not certify a human path. A shared Decision/Document subject list creates no composite interaction subject. |

## Authorization and operational behavior

| Case | Required public outcome |
| --- | --- |
| Writer/Reader | Authorized Component writer can author hosting with UI reference access. Reader may inspect but receives PERMISSION_DENIED on hosting mutation. |
| No peer write | Hosting does not require writes to slice or Persona owners. No path grants login, role, deployment or Command-execution permission. |
| Hidden containing owner | Unreadable Component/slice uses generic NOT_FOUND without inventory/title/UID disclosure. Test foreign branches separately from namespace visibility. |
| Hidden/missing endpoint | Keep the authored selector readable, with generic unavailable state for either case. Compare responses to prevent hidden title, UID, etag, count or existence leakage. |
| Dependency safety | Safe deletion cannot ignore a hidden dependency. Incomplete or failed guard refuses deletion with its existing operational semantics. |
| Store/decode/timeout | Inject actual backend, decoding and timeout failures. They remain RPC errors rather than absence, unresolved/unavailable state or a successful empty path. |
| Authorization failure | Authorization-service failure remains operational; no fallback to unrestricted/baseline view. Branch visibility uses authenticated request context. |
| Concurrent reads | Mutate selected owners while inspecting. Report only the checked per-owner references/UIDs/etags and consistency boundary; never label a mixed read set an atomic snapshot. |
| Same-owner coherence | Resolve all selected slice roles from one checked version of that owner; never combine different versions into a path. |
| Receipt freshness | A retained response does not authorize a later write or prove current wording. Refresh changed rows; do not combine old provenance with current labels/targets. |

## Bounds and Source isolation

| Case | Required public outcome |
| --- | --- |
| Collection/encoded limits | Exercise 1,024 hosting boundary and whole prepared-owner 1 MiB boundary, including assigned IDs and existing collections. Overflow rejects atomically without truncation. |
| Mixed owner inventory | Share request work across relationships, communications, hostings and selected slice edges. Charge complete identity inventories once for the selected ConnectionRef containing owners. |
| Nested budget | Individually admissible selected inventories exceeding the combined examined-occurrence ceiling return RESOURCE_EXHAUSTED. Repeated role lookups within the same owner do not charge its inventory repeatedly; lower owner limits remain controlling where they prevent an overflow fixture. |
| Plain endpoint reads | Charge each participating UI/Persona/Command/ReadModel row without charging unexamined contained occurrences, such as UI transitions. |
| Resource/output caps | Existing applicable resource/output ceilings remain shared across internal pages. Caps never imply complete interaction discovery or permit destructive mutation. |
| Source create | For every ownership policy, including TITLE-only, incoming hostings reject without native owner or mapping-base publication. |
| Source whole owner | Nonempty current or incoming hostings reject, including a proposed clear of the current collection. |
| Source field update | Supported unrelated field-specific writes preserve local hostings exactly. Hosting occurrence selection and collection ownership reject through closed inventories. |

## Client and evidence boundaries

Native manifests and generated schema require typed UI/connection shapes,
lossless uint64s, present/absent documentation and full occurrence identity.
CLI/MCP must demonstrate real native authoring, exact inspection, reference and
subject queries, export/diff/reapply and branch/authorization forwarding.
Validate-only preview proves no write, not persisted reapply.

Studio must retain hosting/role selectors, owner UIDs, checked row etags and
unavailable/stale state without inferring a path from loaded memberships. Mark
loaded-only correlation separately from backend inspection. Authenticated
browser evidence names the actual backend/fixture; component tests and mocked
gateway results do not establish deployment, native persistence or runtime use.

Implementation completion requires maintained executable fixtures and retained
commands/results at the candidate SHA. No exhaustive Persona/system discovery,
ExternalSystem hosting, persisted composite interaction, abstract system level,
portable selector rebinding, atomic snapshot or runtime interaction proof is
claimed by this proposal.
