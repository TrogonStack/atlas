# Email catalog acceptance requirements

Status: requirements for the proposed
[email catalog contract](email-catalog-contracts.md). No executable fixture,
native implementation, connected browser receipt or PDF validation is supplied
by this document. Every requirement below remains open until tied to exact
source and recorded execution evidence. Logical names are fixture descriptions,
not manifest syntax or predicted server-issued IDs.

## Fixture and evidence

Create an authenticated native fixture with isolated namespaces, Reader and
Writer identities, real persistent storage, the implemented artifact service and
the production Studio bridge. Retain authoritative owner UIDs, local IDs,
ConnectionIds, etags, artifact digests, scope and completeness with each receipt.
Reuse the [visual-state acceptance](visual-state-acceptance.md) evidence rules.

| Fixture anchor | Required role |
| --- | --- |
| Team onboarding digest | Server source, retained copy, declared schedule, exact automation subject and several explicitly authored variants. |
| Invitation reminder | Dynamic subject, source-only copy and repeated sequence occurrences with different timing declarations. |
| Refund confirmation | Live template with retained plain/HTML copy, exact behavioral mapping and current capture for the selected target/profile. |
| Refund rejection | Undefined copy with a reason and latest failed capture attempt; no fabricated preview. |
| Exit survey | External provider source, stale capture and explicit provider signal key. |
| Retired notice | Retired lifecycle with an otherwise current capture, proving lifecycle and freshness remain independent. |
| Unknown notice | Unknown source, timing and lifecycle with explicit reasons; freshness unknown rather than fresh. |
| Source collision controls | Equal upstream template keys in different provider accounts, and equal titles/slugs in different native namespaces. |
| Sequence graphs | Classified FLOW graph with repeated variants, branch, join and cycle; another classified graph sharing a variant; unclassified FLOW control. |
| Hidden controls | Unreadable source/behavior/capture/sequence targets and equal missing selections for privacy comparison. |

Keep synthetic recipient data. Do not call a production provider or send email.
A representative catalog-scale fixture must cover hundreds of templates and
variants, repeated sequence occurrences and mixed capture statuses. Record the
actual size, latency, memory, profile, browser and server configuration; fixture
presence alone does not prove performance acceptance.

## Native admission and exact identity

| Case | Required public outcome |
| --- | --- |
| Full create/read | Real guarded write/read preserves all source, variant, copy, trigger, timing, subject and lifecycle alternatives; author order and optional presence survive transport and persistence. |
| Required alternatives | Missing, unknown enum or conflicting source/copy/trigger/timing alternatives reject atomically. Absent timing never defaults to immediate. Unknown/undefined choices require substantive reasons. |
| Default variant | A single-variant template still returns a stable selectable variant ID. |
| Local identity | Reorder, copy edits and variant renames preserve IDs; duplicate/malformed/stripped/retired IDs reject. Equal-looking newly added variants remain distinct. |
| Retired lifecycle | A present RETIRED variant still resolves in sequence targets and historical inspection. Only removed local IDs become unresolved; a dead label cannot erase its documentation. |
| Source retarget | Changing provider account, upstream key or repository location with the old source ID rejects; a new source ID requires explicit variant retargeting. |
| Source revision and variant retarget | A source revision edit preserves source ID; selecting another source preserves variant ID. Earlier captures retain original checked provenance and are re-evaluated without inheriting replacement data. |
| Variant source | Each variant selects exactly one active source; wrong-owner/removed/unknown local selections reject without name fallback. |
| Incarnation | Delete/recreate an equal template key; old variant selectors remain unresolved and newly authored stale selectors reject. |
| Exact version | Equal template slugs at different versions remain distinct; no latest or title fallback through catalog, graph or capture reads. |
| Body bytes | Retained HTML/plain-text and expression bytes and digests survive export/reapply exactly; sanitized derivatives retain independent digests and provenance. |
| Limits | Exercise exact collection/text/prepared-entity boundaries and one-over cases, including server-assigned IDs. Refusal leaves no partial owner or truncated copy. |

## Behavioral links and declaration boundaries

| Case | Required public outcome |
| --- | --- |
| Exact connection mapping | Two interactions with equal endpoints retain separate connection selectors and independent trigger/capture associations. |
| Trigger lifecycle | Timing/doc edit preserves trigger ID; changing trigger kind, native subject or provider signal requires a fresh ID. Remove/re-add cannot revive a stale trigger. |
| Trigger identity inventory | Duplicate IDs across variant triggers or template SubjectAssociations reject. Selecting a trigger from the wrong variant cannot resolve through the containing template alone. |
| Native path inspection | A claimed issuance/automation path is checked by its existing full native tuple. Subject association alone returns relevance without a checked-path verdict. |
| Provider signal | A provider tag/event key never appears as a native Event or transport equivalence without a separate explicit declaration. |
| Schedule | Opaque schedule text and timezone round-trip; no job starts and no cron correctness is claimed. |
| Dynamic subject | Inspector distinguishes source expression, synthetic example and undefined subject. No expression evaluation or inferred variant expansion occurs. |
| Copy state | Source-only content, undefined content, retained empty-supported values and missing capture remain distinguishable. |
| Authority | Team, Persona, provider/source key, lifecycle and recipient descriptions do not alter existing authorization. |
| No behavior mutation | Catalog/sequence creation does not add slices, Commands, Events, transitions or sends to the behavioral model. |

## Sequence occurrence preservation

| Case | Required public outcome |
| --- | --- |
| Typed target | Graph step resolves the exact template incarnation and variant ID; wrong kind/version/owner/local ID rejects or remains explicitly unresolved under retained-reference rules. |
| Repeated variant | Reuse a variant at separate step IDs in the same graph and another graph; all memberships and occurrence links remain distinct. |
| Reminder timing | Two steps select the same variant and distinct exact trigger IDs, preserving different delay/schedule declarations. An omitted selector shows timing unselected; wrong-variant or removed trigger selection refuses without first-trigger fallback. |
| Branch/join/loop | Labels, transition IDs and authored listing order survive; badges do not claim a total execution order. |
| Retarget/reorder | Step reorder preserves step/transition IDs. Changing an email or trigger target preserves step ID but cannot reuse a preview from its previous context; the original receipt remains evidence for its original subject. |
| Removed step | Remove and later re-add an equal step; old occurrence deep links cannot resolve to its replacement. |
| Subject-only control | General DesignGraph subject association and an unclassified graph do not silently become email-sequence membership. |
| Position badge | Complete sequence listing yields correct occurrence position and denominator; hidden/incomplete listing shows position unavailable without global counts or renumbered partial completion. |
| Exact deep link | Open badge and preserve graph version/incarnation/step plus template/variant and active scope/profile; another occurrence of equal content remains separately selectable. |

## Evidence and status

| Case | Required public outcome |
| --- | --- |
| Shared evidence subject | Email-variant captures use their own typed subject; no fake Screen/UI/VisualState is created. |
| Checked capture | Accepted receipt records exact variant/source/copy/trigger/timing bindings, synthetic fixture and render-client profile plus verified immutable image artifact. |
| Current/stale | Matching target/profile/bindings yields current. Change one required binding, source revision or age-policy target; only affected captures become stale. |
| Unknown target | Recent capture without required source target/provenance stays unknown. |
| Profile isolation | Current desktop/client/locale capture does not count as current for another profile or mail client. |
| Trigger context | Generic variant preview does not certify a selected reminder trigger context. Captures from distinct selected triggers stay in separate evidence partitions. |
| Candidate ordering | A newer accepted stale or unknown capture remains the selected candidate even when an older capture matches the comparison target. Producer timestamps cannot change candidate or latest-attempt ordering. |
| Latest failure | Failed or expired attempt preserves successful history and baseline. Badge/inspector expose both attempt outcome and older capture freshness. No attempt remains explicit. |
| Concurrent attempt order | A starts before B in one exact variant/profile/trigger partition. B succeeds before A later fails; B remains the latest attempt by durable creation sequence, and A cannot replace its status. |
| Baseline guard | Successful capture is never automatically selected as baseline; stale/concurrent baseline changes refuse under guards. No baseline means Unreviewed; equal/different comparable bytes are Unchanged/Changed. A selected old baseline does not imply current freshness or a perceptual verdict. |
| Independent lifecycle | Retired/current, live/missing and unknown/failed combinations remain representable and correctly filterable. |
| Partial bytes | Interrupted upload or missing verification cannot produce an accepted capture; failed attachment leaves any Ready artifact recoverable under artifact lifecycle. |
| History/branch | Evidence compares against the selected branch/historical target, or explicitly reports historical comparison unavailable without substituting live data. |
| Image claims | Test wrong media declaration, malformed image, digest/size mismatch and invalid dimensions before image certification. Artifact upload success alone does not pass these cases. |

## Catalog, authorization and completeness

| Case | Required public outcome |
| --- | --- |
| Search/filter | Backend search and source/trigger/area/sequence/lifecycle/evidence predicates match the fixture; list and grid consume the same filtered results. |
| Unit counts | Template and variant totals are named and distinguish repeated sequence occurrences. Total scope extends beyond the loaded page only when the authorized scan is complete. |
| Stable pagination | Equal titles and repeated memberships span page boundaries without loss or duplication under one snapshot. |
| Cursor binding | Reusing cursor with different identity, visibility, namespace, branch/history, query, profile or target refuses. Changed snapshots cannot yield mixed-version complete results. |
| Source collisions | Equal upstream keys in different accounts remain separate and never share preview/capture status accidentally. |
| Inline source visibility | Inline bindings use the containing template's visibility. Hidden-source controls use an unreadable template or referenced native dependency and do not invent provider ACLs. |
| Hidden versus missing | Hidden and missing selected targets have equivalent opaque results. No hidden names, counts, facet values, sequence denominators, thumbnail or export bytes leak. |
| Operational failure | Inject storage/decode/auth-service/timeout failures; reads fail visibly instead of empty success, unknown lifecycle, missing capture or fresh status. |
| Budget exhaustion | Exceed scan/output caps; report exhaustion/incompleteness without exact totals or a complete-looking clipped export. |
| Read-only roles | Reader inspects permitted data but cannot author, ingest captures or choose baselines. Revoking access prevents subsequent detail, artifact and export access. |

## Inert preview and PDF

Use a hostile synthetic HTML corpus through the actual preview and PDF paths.
Instrument the renderer's network boundary and record zero requests, script
effects, navigation, downloads, popups or host filesystem reads. A static
sanitizer unit test or absence of script tags is insufficient.

| Case | Required public outcome |
| --- | --- |
| Escaped source | Script elements, handlers and HTML-like subject text render as source text when inspected. |
| Active HTML | Scripts, handlers, forms, frames, embeds, navigation and service-worker attempts do not execute in preview or PDF paths. |
| Tracking/network | Remote image pixels, CSS imports/URL loads, fonts, links, nested stylesheets and provider URLs cannot initiate requests. |
| Inline payload | Arbitrary SVG/data/blob payloads are refused or inert; permitted image content uses independently admitted bounded artifacts. |
| Unsupported source | Preview unavailable is explicit while escaped retained source remains inspectable; no broken/sanitized preview is labeled certified. |
| PDF selection | Export exactly the selected authorized variants/profile and state the source scope, target, completeness and missing content. It does not silently export the entire catalog. |
| PDF bytes | Open a generated PDF with a maintained independent parser and render representative pages. Verify readable content, statuses/identities and retained image appearance; a successful HTTP response alone is insufficient. |
| PDF safety | Output contains no credentials, expiring delivery URLs, real recipient data, active HTML, remote dependencies or unsafe filename. |
| PDF budgets | Exercise each input/variant/page/output/time ceiling and cancellation; overflow/timeout yields no successful partial artifact. |
| Export privacy | Hidden inputs cannot enter PDF text, images, metadata or totals; persisted export access follows current namespace authorization. |
| Cross-namespace revocation | An output readable in one namespace cannot disclose input bytes from another after that input's access is revoked. Each output read checks the complete retained input set, including transitive derivatives. |
| Frozen render inputs | Change or recreate a same-key template/capture during rendering. Export uses its recorded exact input revisions or refuses; it cannot mix refreshed content into a complete old-scope receipt. Revoke required input authority before release and verify no output is delivered. |

## Connected authoring and Studio acceptance

Prove schema/core behavior with meaningful public tests, then exercise the same
fixture through the connected native client, CLI and MCP. Cover guarded preview
and apply, branch conflicts, scope/auth propagation, manifest export/diff/reapply,
reference discovery, inverse sequence lookup, impact and safe/forced deletion.
Source reconciliation must fail closed for unsupported email kinds and nested
associations. Confirm existing native resource fixtures remain unchanged.

Run authenticated Studio against the real fixture and production bridge. Verify
list/grid parity, filters, variant expansion, capture/attempt/baseline detail,
source and behavioral navigation, exact occurrence links, historical/branch
context, loading/error/empty/incomplete states, keyboard inspection and PDF
download. Client mocks and component snapshots are supporting evidence only.

Each completed requirement must link exact implementation source, named test
assertions, actual command exits and immutable run receipts. Record checked
commit, service/fixture provenance, resource scope and acceptance limits. A
schema-only change, compiled test, queued CI job or retained object pointer does
not establish connected capability or release readiness.
