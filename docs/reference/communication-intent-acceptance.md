# Communication intent acceptance requirements

Status: native support for the
[communication intent contract](communication-intent-contracts.md) is
implemented. This reference defines acceptance requirements and links
executable coverage below. Logical fixture names are not manifest syntax,
submitted occurrence IDs or predicted owner UIDs. Fixture presence alone does
not establish that every requirement has run or that deployment is verified.

## Fixture and evidence

Use fresh registered namespaces and exact version `1` unless stated otherwise.
Create Specification messages and peer boundaries first, retaining returned
Specification UIDs and member IDs. Create owners with communications next,
then Team/Person and Decision/Document subjects using returned ConnectionRefs.
Do not use predicted IDs to bypass the required staging of member selectors.

| Logical entity | Required setup |
| --- | --- |
| Specification `commerce/notifications@1` | Unmapped MESSAGE members `order-hint` and `other-hint` with equal payload definitions but distinct names and member IDs. Include a non-MESSAGE member for negative selection. |
| Component `commerce/checkout@1` | Deployable owner with existing raw members and architectural relationships. `send_a` and `send_b` are equal SEND declarations to ExternalSystem `provider@1` selecting `order-hint`; condition text describes when a notification is intended. |
| Component `commerce/worker@1` | Independent deployable peer. Declarations cover Component-to-Component and RECEIVE on a Component. |
| ExternalSystem `commerce/provider@1` | Has no inverse declaration initially; later authors its own RECEIVE from checkout and SEND to a separate ExternalSystem. |
| Exact-version controls | `checkout@2`, `provider@2` and another Specification version have distinct keys and independent UIDs/member identities. |
| Subjects | Team/Person assignments and Decision/Document subjects select only `send_a`; include owner-entity, peer-entity and exact message-member subjects as independent controls. |
| Behavioral controls | A valid ReadModel/Processor/AutomationSlice and a separately anchored Channel exist but are not bound to these declarations. Keep an unmapped Specification message available throughout intent authoring. |

Capture requests/responses, actual owner/member/connection identities, checked
etags/revisions, branch, authenticated caller, authorized namespace scope and
completeness. Backend service tests, authenticated transport, real CLI/MCP,
Studio component and deployed browser evidence must be recorded separately.
Do not substitute fixture source or mocked HTTP tests for native persistence.

## Declaration and evidence assertions

| Case | Required public outcome |
| --- | --- |
| Native create/read | All fields preserve presence, Unicode, order and exact selectors. Equal declarations receive distinct IDs; owner UID is server-owned. |
| Unmapped message | A resolvable MESSAGE member without ReadModel mapping is accepted. No native Event, Command, Channel, Processor or slice is created or required. |
| SEND and RECEIVE | Each assertion belongs only to its authoring owner. A RECEIVE does not create/satisfy peer SEND, and matching independently authored declarations do not deduplicate or certify delivery. |
| Reference direction | RECEIVE still reports stored outgoing references from its owner to peer and Specification, with full provenance. No inferred reverse persisted edge. |
| Conditions | Absent condition differs from present condition prose; absence does not assert unconditional delivery. No predicate evaluation, scheduling, subscription or automatic execution occurs. Wording edits preserve IDs but change checked content/etag. |
| Behavioral separation | Existing message mapping, Component membership, Processor calls, matching labels or a shared ReadModel do not bind declaration to slice/Channel. API roles/provider claims and runtime paths remain unchanged. |
| No alternate trigger | A message/Event-to-Processor or message-to-Command route is not introduced. Automation continues through ReadModel observation and at most one emitted Command. |
| Exact subjects | Assignment/Decision/Document queries select `send_a` without matching `send_b`, the owner, peer or message member. Primary endpoint context is the peer; message availability is independently checked. |
| Reference multiplicity | For each declaration, return peer and Specification-owner references with its ID, owner UID, exact typed targets and distinct current field paths. Equal endpoints preserve multiplicity; raw members never acquire IDs. |
| Member identity | Missing/stale Specification UID, removed member or wrong member kind cannot resolve through equal payload/name or another version. Specification entity reference alone does not satisfy member validation. |
| Export/reapply | Same-store native export/diff/no-op reapply preserves full content, UIDs, IDs and etags. Report incompleteness; clipped exports are not complete reapply artifacts. New-store rebinding is not claimed. |

Exercise all allowed owner/peer kind combinations and both directions. Keep
declarations with a shared exact peer but different message members, and with
a shared exact message but different peers, to expose accidental deduplication.
Preserve legacy raw members and `relationships[]` during every unrelated edit.

## Admission and lifecycle assertions

For every refusal, read back native content, etag, UID and complete occurrence
inventory to prove no partial owner or branch publication.

| Input/operation | Required result |
| --- | --- |
| Missing/unknown direction; absent/multiple peer alternatives; malformed exact Id; self-peer | Reject atomically. Different versions are distinct peers; cycles between distinct boundaries remain descriptive and legal. |
| Invalid message selector | Reject missing owner UID/member ID, wrong or unknown kind, nonexact Specification ID, absent member and MESSAGE `expected_digest`. Do not accept artifact digest shortcuts. |
| Label/condition/doc values | Required nonblank label; optional text rejects present blank. Valid UTF-8 at 256-byte label and 16-KiB text limits succeeds; limit plus one fails. |
| ID syntax and presence | Omitted IDs are assigned. Explicit empty, malformed, nil or duplicate supplied IDs reject, including cross-collection duplication with architectural relationships. |
| Existing-owner edit | Retain IDs for reorder/label/doc/condition edits; direction, peer or message selector changes require fresh IDs. Reject unknown supplied IDs or ID stripping in edited content. |
| Separate removal/re-add | Old selector becomes unresolved; re-added equal content receives a new ID. Equal sibling identity and subjects remain unchanged. |
| Member content edit/removal | Existing member lifecycle governs content changes; no frozen payload implied by ConnectionRef. Accepted member removal leaves old selectors unresolved; equal sibling does not repair them. |
| Preview/idempotency | No native mutation from preview; returned preview IDs are unreserved. Retry semantics and digest guards use original caller intent. |
| Clone/new version | New owner UID scopes any retained local IDs; incoming subjects stay on the old exact owner. Peer and message selectors do not auto-advance. |
| Delete/recreate | Owner recreation invalidates old communication subjects. Peer recreation follows exact-key semantics; Specification recreation does not satisfy an old UID-bound MESSAGE selector. |
| Target force deletion | Retain readable declaration and identity, report generic unavailable hydration, do not cascade/rebind. Unchanged dangling declarations do not veto unrelated writes. |
| Safe deletion/retarget | Peer, Specification owner and subject-owner referrers protect safe deletion, including hidden namespaces and final batch deletions. Peer retarget renews affected communication IDs; member selectors require explicit rebinding. |
| Stale/final-overlay writes | Validate under guarded publication against final merged graph and tombstones. Stale previews cannot publish invalid peers/members or partial batches. Stage a new Specification before selecting its returned UID. |

Branch fixtures cover ordinary edit/land/rebase preservation, opposing-side
removal with no implicit ID revival, explicit conflict resolution renewing only
restored IDs, whole-owner deletion/restoration and independent same-key owner
creation. Use recorded base/current views and deterministic gates for races.

## Authorization, capacity and failure assertions

| Case | Required public outcome |
| --- | --- |
| Reader authoring | PERMISSION_DENIED before any mutation; directory assignment grants no write authority. |
| Owner-only authority | SEND and RECEIVE require owner write plus peer/member reference access. No peer write is required or granted; a receive declaration cannot write the peer's intent. |
| Hidden/missing targets | New/edited owner refuses unavailable peer/member without hidden data. Readable retained declaration can remain identified, with generic unavailable hydration and no hidden titles/counts/existence. |
| Branch visibility | Foreign branch access fails before snapshot/target lookup; baseline and authorized owner-branch controls retain behavior. |
| Owner limits | Boundary and overflow tests for 1,024 communications and 1-MiB complete prepared owner, including assigned IDs, relationships and metadata. Rejection leaves stored content unchanged. |
| Shared resource scans | Component/ExternalSystem count once per exact owner/view under existing 5,000-row cap, alongside other covered kinds. Exact authorized hydration remains independent of display truncation. |
| Examined occurrence limits | Mixed architecture/communication owners share 10,000 examined occurrences across nested subject resolution; equal endpoints consume work. Selected inventories overflow without pagination or nested-counter bypass. |
| Unrelated query controls | More than 10,000 unselected communications must not exhaust Person-Team or entity-subject assignment queries that do not inspect them. The shared resource-row ceiling still applies. |
| Graph bounds | Incoming/impact indexing that enumerates communication references charges actual work. Baseline/branch overflow and a smaller authorized-lens success distinguish bounded work from unrelated-query charging. |
| Output and incompleteness | Existing 4,096 emitted-record cap and explicit incomplete report/export behavior remain observable. Clients never turn incomplete results into complete claims or reapply artifacts. |
| Operational faults | Store/decode/timeout/authorization-service errors remain errors, not missing targets, empty success or safe-delete authorization. Complete deletion guard failures refuse publication. |

## Source isolation and connected clients

Exercise both Component and ExternalSystem through public supplied-bundle
Source APIs. Reject every create containing communications, including
TITLE-only ownership, before native writes or mapping-base advancement.
Reject selection/adoption of a communication and ownership of its collection.
Whole-owner writes reject when either current or incoming collection is
nonempty, including an attempted silent clear. Existing-owner title-only
reconciliation preserves all communication selectors and unrelated fields.
Preserve independent architectural-relationship Source restrictions.

Real native CLI/MCP tests must retain exact large numeric versions, peer oneof,
message member kind/UID/ID, direction, optional condition presence and returned
IDs through authoring/preview/inspection/export/reapply. Studio remains a
read-only inspection surface: partial raw subjects cannot crash it, equal
siblings must not substitute for a selected ID, and any loaded-only incoming
view declares its scope. Authenticated browser evidence must identify the real
backend/fixture and is separate from component or mocked gateway results.

## Executable coverage and limits

| Fixture | Verification boundary |
| --- | --- |
| [Core communication tests](../../rsworkspace/crates/trogon-atlas-core/tests/communication_intent.rs) | Typed values, exact selected identity, occurrence lifecycle, retargeting, subject resolution and native-only ownership helpers. |
| [Backend communication tests](../../rsworkspace/crates/trogon-atlas-server/tests/communication_intent_backend.rs) | Native persistence, final-overlay admission, message availability, reference and subject handling, and bounds. |
| [Responsibility backend tests](../../rsworkspace/crates/trogon-atlas-server/tests/responsibility_backend.rs) and [Source backend tests](../../rsworkspace/crates/trogon-atlas-server/tests/source_integration_backend.rs) | Authenticated hidden/missing message resolution and native-only Source isolation. |
| [Authenticated gRPC acceptance](../../rsworkspace/crates/trogon-atlas-server/tests/communication_intent_acceptance.rs) | Returned message identity, parallel reference multiplicity, exact subjects, wording/reorder/no-op preservation and unavailable message with a retained peer. |
| [Native manifest tests](../../rsworkspace/crates/trogon-atlas-client/tests/communication_manifest.rs) | Typed message/peer selectors, exact numeric versions and malformed manifest rejection. |
| [CLI integration](../../rsworkspace/crates/trogon-atlas-cli/tests/communication_cli_integration.rs) and [MCP integration](../../rsworkspace/crates/trogon-atlas-mcp/tests/mcp_integration.rs) | Native authoring, preview, inspection and same-store export/reapply. |
| [Studio communication tests](../../tsworkspace/webapps/studio/src/components/CommunicationIntentDetails.test.tsx) | Read-only owner-relative display, loaded-owner scope, exact selectors and partial raw subject handling. Component evidence is distinct from native transport and deployed browser results. |

Record commands/results at the candidate SHA separately from this source
inventory. It does not establish every matrix case or production-scale and
deployed-browser readiness. Behavioral binding, predicate evaluation, native
Event/Command equivalence, connectors and portable migration remain deferred.
