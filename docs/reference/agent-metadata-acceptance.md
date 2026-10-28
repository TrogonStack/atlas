# Agent and shared metadata acceptance

Status: partially verified requirements for the
[contract](agent-metadata-contracts.md). Implemented coverage includes
[core metadata tests](../../rsworkspace/crates/trogon-atlas-core/tests/agent_metadata.rs),
[backend admission and lifecycle tests](../../rsworkspace/crates/trogon-atlas-server/tests/agent_metadata_backend.rs),
[Studio protobuf decoding](../../tsworkspace/webapps/studio/server/decode-annotations.test.mjs)
and [Inspector tests](../../tsworkspace/webapps/studio/src/components/AgentMetadataDetails.test.tsx).
Additional implemented coverage includes
[authorized group lifecycle tests](../../rsworkspace/crates/trogon-atlas-server/tests/native_metadata_authz_backend.rs),
[tenant-context digest tests](../../rsworkspace/crates/trogon-atlas-server/tests/native_metadata_digest_backend.rs)
and combined manifest, CLI and MCP acceptance. The authorized lifecycle and digest
targets pass in the combined parent checkout, including Reader refusal, hidden
target parity, hidden-referrer deletion guards, branch identity and same-batch
tenant descriptors. These fixtures do not establish every requirement below;
requirement-by-requirement closure remains open. Component and codec checks are
not deployed-browser evidence.

[Core acceptance](../../rsworkspace/crates/trogon-atlas-core/tests/agent_metadata_acceptance.rs),
[public-stack acceptance](../../rsworkspace/crates/trogon-atlas-server/tests/agent_metadata_acceptance_backend.rs)
and [MCP changed-update and schema-valid rejection](../../rsworkspace/crates/trogon-atlas-mcp/tests/agent_metadata_mcp_integration.rs)
cover partial models, nested scope, singletons, drafts and dates, groups,
responsibility, malformed and native-looking Any, budgets, atomic rejection,
stale incarnation and post-preview revocation.
These additional fixtures pass in their isolated implementation checkout;
combined parent verification remains pending. The aggregate encoded-size
boundary has core coverage; public backend atomic rejection still requires
verification.

Preview has no token. `validate_only` on put and batch binds no revision, owner
UID or principal, so apply re-evaluates the etag (the store revision, new for a
recreated owner), the target and authorization. `PutEntityRequest` has no
owner-UID field, so a preview-bound UID pin is not reachable; the stale
incarnation guarantee is delivered through the etag.

## Common fixture and observable results

Create an authorized branch with Components A and B in readable namespaces,
Team T responsible for A, and a target C in a hidden namespace. Capture exact
native Ids, authoritative owner UIDs and revisions from backend responses.
Use backend-issued or valid fresh occurrence IDs according to existing admission;
never derive them from display indexes. A has AgentAnnotation, optional partial
model, ordered tools, lifecycle, badges and resource groups. Group G contains B
twice using distinct item IDs I and J. Retain fixture bytes and exact response
identities as evidence. Do not use Source fixtures as substitutes for native
objects. Every rejected mutation must leave content, revision and index unchanged.

## Required automated cases

| Case | Input/action | Required assertion |
| --- | --- | --- |
| Agent scope | Empty AgentAnnotation on A; same on Event, Processor, ExternalSystem, FieldSpec or scenario | Component accepted, other scopes rejected with field path. Presence marks agent; absence does not. |
| Partial model | provider only, name only, version only; then empty model, blank value, oversized value | Partial declarations roundtrip exactly; empty present model and invalid values reject. Version is opaque, including non-semver text. |
| Tools | Same name/type twice; same name/different type; reorder tools | Duplicate pair rejects; distinct pairs accepted; order and optional description/icon/url preserved. No outgoing reference or executable action results. |
| Singleton | Repeat each known singleton, byte-identical and conflicting, including LifecycleAnnotation | Reject both duplicates, never first-wins or last-wins. Existing repeatable annotations remain valid. |
| Draft | status draft with notice title/message; status draft without notice; notice under accepted status | First alternatives valid; incompatible notice rejects. Blank required message rejects. No missing title synthesized. |
| Dates | deprecated with 2024-02-29; 2023-02-29, 2026-04-31, 0000-01-01, 10000-01-01, timestamp, timezone, empty | Valid leap date retained; every invalid date rejects. 0001-01-01 and 9999-12-31 accepted. Date under another status rejects. |
| Existing dates | Unparsed NoteAnnotation.created_at; server creation time; future deprecated_on | Existing note attribution semantics unchanged; server time not overwritten; no clock-driven status change. |
| Badge values | Valid mixed-case hex colors, unknown valid icon token; exact duplicate; HTML icon, CSS color, credentialed or non-HTTPS URL | Valid values preserved with safe fallback; duplicate and malformed values reject. Label alone does not deduplicate different styles. |
| Groups | G with B at I and J; reorder items/groups; display_limit 1 | Repeated target occurrences retained, IDs/order preserved; reference extraction sees I and J despite display limit. |
| Invalid groups | Duplicate group/item ID, collision with another native owner occurrence, zero limit, latest version, mismatched target kind, missing target | Reject with privacy-safe locator; no partial index update. |
| Identity changes | Change I target; move I to another group; delete/reinsert I; recreate A at same native key | Require fresh IDs for changed/moved/restored occurrences; old incarnation never binds to recreated owner. Copy/new version renews IDs. |
| Responsibility | Assign T to A; attempt group/tool/badge as ConnectionRef subject | Existing Component assignment resolves; unsupported local subjects reject. No authorization grant. |
| References | Get refs, reverse refs and impact on B | Return both identified occurrences with correct branch/owner context; unrelated metadata strings produce no refs. |
| Delete | Safe-delete B; force-delete B; recreate B | Safe delete blocks; force retains unresolved exact selections. EntityRef uses its existing exact-key semantics on recreation, with no invented target UID pin. Local occurrence selectors still pin owner incarnation. |
| Retarget | Guarded generic entity retarget B to another authorized target | Renew affected item IDs; unrelated items and group IDs preserved; reference indexes agree with content. |
| Malformed Any | Known type URL with invalid wire bytes, wrong payload type, truncated bytes | Admission rejects; reference extraction/queries report malformed state instead of verified empty refs. |
| Unknown Any | Unregistered type URL; valid registered tenant annotation containing a native-looking string | Existing registration policy governs acceptance; unavailable descriptor fails JSON conversion clearly. Strings create no graph edges. No fallback into AgentAnnotation. |
| Budgets | Each maximum and maximum plus one, including UTF-8 multibyte text and aggregate encoded size | Bound accepted, excess rejected atomically. Existing smaller request/owner ceilings win. |

## Authorization, concurrency and Source isolation

Run the same mutation through create, update, guarded batch, preview/apply,
branch land/rebase and explicit conflict resolution where applicable. Test stale
revision, stale owner UID, deleted target and authorization revoked after preview.
Apply must reject stale/unauthorized state and preserve all unaffected owners.
Final-overlay batch removing group references and deleting B succeeds only when
all retained incoming refs are gone. A batch introducing a dangling reference
fails atomically. Unrelated writes do not globally reject a retained broken A;
editing A requires repair of its invalid proposed selections.

A Reader receives PERMISSION_DENIED on writes. Referencing C without target
access fails without titles, contact data or existence disclosure. Reading C
returns NOT_FOUND under existing policy. A hidden referrer blocks deletion of
B without revealing its identity. Hydration and pagination must not leak hidden
rows via counts, diagnostics or cursors. Insufficient read/traversal budget is
explicit incompleteness, never an empty result or permission to delete.

Assert model provider and tool URL cause no network request, credential lookup,
Source registration or execution. Reject Source handles as group EntityRefs;
retained overlays cannot satisfy missing native endpoints. Native writes cannot
alter Source-owned content. Source creates carrying any new annotations reject
even under TITLE-only policy. WholeOwner current/proposed annotations reject,
including clears; unrelated field-specific updates preserve local annotations.
New group/item selectors and annotation ownership paths reject. Source fixtures
remain unchanged after every rejected operation.

## CLI, MCP and Studio roundtrip matrix

Use the same fixture on each path, comparing shared descriptor-based semantic
content, optional presence, list order, exact target keys and occurrence IDs.
Record namespace, branch, backend revision and descriptor context with results.

| Surface | Required test |
| --- | --- |
| CLI | Native manifest apply, exact get, export, reapply to the same existing incarnation under guards. Preserve all fields and IDs; a fresh checkout is not a claim of portable incarnation rebinding. Invalid values fail with matching field diagnostics. |
| MCP | Create/update and get through existing native tools with expanded JSON Any; refs/impact agree with CLI. Tool schema advertises appropriate shape or explicitly documents permissive Any validation; semantic rejects still come from backend. |
| Studio | Backend response decoding and inspector projection retain model, tool order, draft notice, date, badges and all group items. Render escaped text and inert links safely. Review surface does not mutate the model to repair it. |
| Shared JSON | Expanded `{ "@type": "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation", "model": { "provider": { "value": "example" } } }` decodes to protobuf and back without loss. Shared annotations use their annotation package names. |
| Raw protobuf Any | Decode real packed bytes through shared descriptors, then CLI/MCP JSON and Studio decodeAnnotations. Corrupt payloads cannot be silently dropped or converted into a valid empty annotation. |
| Nested Any | Extend the existing Studio nested-schema annotation regression: top-level annotations decode, but these proposed annotations placed on nested fields/scenarios are rejected by backend scope admission. Existing unrelated nested annotations still decode. |
| Schema | manifest_schema well_known_schema currently validates discriminator only. Demonstrate schema-valid malformed AgentAnnotation still rejects at backend; if registry-aware payload schemas are added, verify they match semantic validation without claiming schema alone enforces references. |
| Tenant context | Registered tenant Any alongside built-ins roundtrips with authorized branch type context; missing/incompatible descriptors fail explicitly, with no raw-byte or field-loss success report. |

## Evidence and implementation gates

Suggested suites: core transcode/refs/semantic equality tests; server admission,
authorization, reverse index, deletion and branch tests; client manifest/schema
and CLI transport integration; MCP tool transport; Studio
`server/decode-annotations.test.mjs` and inspector projection tests. Names here
are implementation targets, not claims that new tests exist.

Parent must resolve the ResourceGroups-in-Any occurrence-admission objection,
ordered semantic diff behavior, lifecycle kind coverage and proposed limits
before schema implementation. Generate descriptors/bindings and coordinate
mutation revision floors together. Record exact commands, failures and passing
results before promoting status to implemented. Browser evidence, if required,
is separate from decoder unit tests. Do not close the consolidated coverage
items based solely on this proposal or a single accepted fixture.
