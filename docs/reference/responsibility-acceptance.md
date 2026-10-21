# Responsibility acceptance

Status: acceptance requirements and executable coverage for the native
[responsibility contract](responsibility-contracts.md). The coverage section
links implemented regressions; the requirements tables also retain cases that
need further evidence. A source link alone is not an executed result or release
approval. Deployed-browser acceptance remains unverified.

## Fixture anchors and evidence context

| Existing source anchor | Responsibility acceptance target |
| --- | --- |
| [Connected product fixture](../../rsworkspace/crates/trogon-atlas-server/tests/data_product_fixture.rs) | Reuse exact version `1` in `product-fixture`: ReadModels `submitted`/`receipt`, Event `placed`, slices and returned product ports, access associations and selected occurrences. Assign a Team to the product, a Person directly to an output port, and another assignment to one particular access association. |
| [Specification fixture](../../rsworkspace/crates/trogon-atlas-server/tests/specification_fixture.rs) | Reuse native artifacts, schemas, messages, operations and exposures created through the server in namespace `contracts`. Assign different kinds to the Specification entity, each member kind and selected associations; distinguish member responsibility from provider exposure and API usage. |
| [Resource fixture](../../rsworkspace/crates/trogon-atlas-server/tests/architecture_resources.rs) | Reuse actual stored Channel bindings, DataStore attachments and repeated observation edges. Different repeated occurrences must support different assignments. |
| [Branch lifecycle](../../rsworkspace/crates/trogon-atlas-server/tests/resource_landing_lifecycle.rs), [retargeting](../../rsworkspace/crates/trogon-atlas-server/tests/resource_retargeting.rs) and [delete scope](../../rsworkspace/crates/trogon-atlas-server/tests/resource_delete_scope.rs) | Extend accepted removal/retarget, unchanged dangling landing, hidden inbound deletion protection and batch/final-view coverage to the new references. |
| [Connection identities](../../rsworkspace/crates/trogon-atlas-server/tests/connection_ids.rs) | Extend identity restoration, repeated endpoints, exact owner UID and branch conflict cases to membership/responsibility associations. |

Fixtures must create native Team/Person entities through public backend
writes. Seed a Team with authored contacts, a Person with a minimal profile and
optional pointer, another Person sharing a contact without identity merging,
an empty Team, and explicit same-slug alternative versions. Add authorized and
hidden cross-namespace cases, Reader/writer principals and branch tombstones.
Contacts and pointers in fixtures must be synthetic values.

Fetch each authoritative persisted response. Build selectors from returned
owner UIDs, ConnectionIds, member IDs and port IDs; never predict UUIDs, infer
IDs from fixture names or use preview identities as persisted ones. A new
selected owner can require a follow-up guarded write before assignment.
Retain actual kind/key/version, owner UID, member kind/port direction, optional
artifact digest, association kind/ID, etags, request guards, branch, authorized
scope and checked revisions with each result. Fixture names alone are not
evidence that the selected native identity exists.

## Profiles, membership and authority

| Case | Required result |
| --- | --- |
| Minimal Person and empty Team | Accept required shared values with absent role/pointer and empty contacts, members and responsibilities. Distinguish no authored members from external-directory truth. |
| Typed optional contacts/pointer | Round-trip presence, values and authored order; reject empty supplied records, unknown alternatives, invalid email/URL/provider/key and size limits. No network fetch occurs. |
| Contact normalization/deduplication | Preserve authored spelling. Reject exact duplicate contacts within an owner; permit shared contacts/pointers across people without merge, principal lookup or uniqueness claim. |
| Raw directory data and credentials | No directory snapshot, arbitrary profile payload or principal-binding field is accepted as part of this contract. Free text/registered metadata are not represented as automatic personal-data detection or erasure. |
| Person, Persona and principal | Reject wrong-kind membership. A Person assignment creates no Persona edge, authenticated identity or authorization grant. OWNER and other kinds remain catalog descriptions. |
| Membership | Only Team-to-exact-Person is authored. Reverse membership is derived; duplicate same-exact-Person membership rejects, distinct versions remain distinct. Each membership result includes the containing exact TeamRef, Team owner UID, membership ConnectionId and exact PersonRef. Query a Person belonging to multiple Teams and distinguish each containing Team and occurrence. No nested Teams or membership roles. |
| Direct versus Team responsibility | A Team assignment does not appear as a member's direct assignment; a Person assignment does not appear as a Team assignment. Multiple assignees/kinds for a subject remain independent. |
| Team contact expansion | Default returns Team contacts only. Explicit member expansion returns separately attributed authorized member contacts, not inherited assignments. Empty, hidden and incomplete expansion remain distinguishable. |
| Metadata and supersession | Registered-type validation and server-owned system fields match native rules. Invalid same-family supersession, wrong-kind versions and cycles reject. Metadata does not replace typed subjects or grant permissions. |
| Profile history | Attributed edit/delete records retain actual before/after images under native history behavior. Show current versus retained historical contact values without claiming complete history, purge or a retention deadline. Historical views never silently hydrate current profile data. |

## Exact subjects and selector inventory

| Case | Required result |
| --- | --- |
| Entity inventory | Exercise every allowed entity kind from the contract; reject unspecified/unknown kinds, non-entities and exact self-assignment. Mutual responsibility between distinct Team/Person entities remains descriptive and nonrecursive. |
| Version selection | A version `1` assignment never follows version `2` or `supersedes`. Another version needs an explicit new selector. Nonbreaking same-version edits remain visible under normal concurrency guards. |
| Entity versus contained subject | Entity responsibility never implies member, port or connection responsibility, and the reverse never implies owner responsibility. Exact reverse-query filters preserve subject alternative and role. |
| Connection owner/path inventory | Exercise every row and stored occurrence kind in the closed contract inventory, including Storyboard entry/outcome, lifecycle/navigation and product lineage/access. Reject other owners/paths. |
| Repeated behavioral edges | Assign to one repeated AutomationSlice source occurrence. Reorder preserves selection; another edge with identical endpoints does not satisfy its selector. |
| Unsupported raw references | Reject Processor raw calls, Storyboard slice/trace positions, measurement spans, screen slots, arbitrary annotations and Team/Person associations as connection subjects. No synthesized IDs or positional fallback. |
| Connection resolution | Require exact kind/key/version, owner UID and unique active ConnectionId. Missing/duplicate/malformed sibling IDs fail closed even when the selected ID appears valid. |
| Contract members | Accept exact ARTIFACT, SCHEMA, MESSAGE and OPERATION identities with returned owner UID/member ID. Reject wrong kind, wrong owner, stale UID, removed/malformed/duplicate IDs and name/path fallback. |
| Artifact digest | ARTIFACT selection accepts absent pin or matching optional SHA-256 pin. Wrong/malformed pin rejects. SCHEMA/MESSAGE/OPERATION reject any expected digest. Pin presence/value is part of the assignment fingerprint. |
| Member semantics | A selected schema member is not ContractTypeRef or compatibility proof. Mutable mapping/definition changes preserve member assignment identity while underlying resolution/diagnostics may change. No artifact parser is invoked. |
| Product ports | Accept exact INPUT/OUTPUT with returned product UID/port ID. Wrong direction, unspecified direction, stale UID, missing/duplicate/malformed IDs or removed port rejects without matching a name/data endpoint. |
| Port semantics | Rename/reorder/data retarget preserves port identity as native rules permit. Assignment to that port does not follow the former data or establish verified lineage, access or execution. |
| Exact hydration beyond projection caps | With general model snapshot cap zero, resolve authorized selected entities/owners/members/ports/occurrences from the actual branch view. A branch tombstone must not hydrate from baseline or another version. |
| Underlying invalid declaration | A valid subject identity can refer to a declaration with independent model diagnostics. Identity resolution does not certify behavior or require recursive validation of unrelated owner dependencies. |

## Association identity, branches and mutation

| Case | Required result |
| --- | --- |
| Create/add and retry | Omitted IDs receive actual persisted UUIDv7; new-owner supplied valid IDs follow native rules. Additions to existing owners must omit IDs; any caller-supplied ID on a new association rejects, including a known active ID. Unchanged retained associations preserve their IDs. Retry preserves operation receipt/idempotency behavior. |
| Duplicate/stripped IDs | Reject duplicate IDs across membership and responsibility collections, nil/malformed IDs and stripped retained identities atomically. |
| Reorder/profile/doc edit | Preserve all unchanged membership and assignment IDs. Native unchanged id-less reapply is a no-op; edited identity-stripping manifests reject. |
| Responsibility role/subject change | OWNER-to-MAINTAINER, entity-to-port, exact version, member kind/digest presence, direction, UID or local selector changes require fresh association IDs. Retaining the former ID rejects. |
| Remove/re-add | Fresh identity after removal; old refs stay unresolved. Same-endpoint replacement honors the separate-write rule where necessary. |
| Selected target/occurrence edit | Accepted removal or retarget leaves unchanged assignments visibly unresolved. Never rebind by endpoint/name or renew the assignment silently. |
| Strict edited-owner admission | Editing profile or documentation on an owner with stale membership/responsibility rejects until the complete proposed owner is repaired. Persisted profile and IDs remain unchanged on failure. |
| Unrelated branch landing | Unchanged invalid assignments remain inspectable and do not veto an otherwise valid unrelated landing or accepted target deletion. |
| Branch/rebase/landing | Preserve unchanged IDs/UID, apply merged view and tombstones, and reject deleted-ID revival against a present opposing same-incarnation owner. |
| Explicit conflict/structural merge | Restore deleted occurrences with fresh IDs only where native resolution requires; keep unaffected IDs. Revalidate full merged owner, duplicate selections, permissions and target existence. |
| Whole-owner deletion/recreation | Explicit keep-ours restoration follows surviving-incarnation rules. Same-key recreation gets a fresh UID; connection/member/port refs stay stale, while plain EntityRef/PersonRef follows the exact key. |
| Clone/new version | Fresh owner UID; local IDs may be retained within the new scope. External exact subjects stay selected unless explicitly mapped. Incoming assignments do not follow copied owners/members/ports/occurrences. |
| Explicit copy mapping | Obtain copied owner UID/local IDs from returned native state, select replacements explicitly, renew changed assignment IDs and reject incomplete/ambiguous mapping. |
| Cycles | Mutual descriptive assignments terminate bounded queries without inheritance. Self-assignment rejects; membership cannot cycle. Existing behavioral/import/supersession cycle policy remains independent. |
| Generic retarget | Exact entity subjects and Person membership can change with fresh IDs. Refuse guessing member/port/connection replacements from only an owner key; explicit selector repair must use current guarded state. |
| Guarded batch preview/apply | Validate complete final overlays before publication. Preserve untouched IDs, reject stale guards and unavailable local identities, and reserve no preview IDs. No partial write on any admission failure. |

## Authorization, references, queries and deletion

| Case | Required result |
| --- | --- |
| Authorized cross-namespace write | Owner write plus selected target/owner and required endpoint read/reference access admits. Selected target need not be writable. Catalog assignment changes no authorization policy. |
| Selected connection context | Check only the selected occurrence's native endpoint/identity context. A CommandSlice emission with readable Event/Command remains selectable despite unrelated hidden UI. A Storyboard entry ReadModel selection needs no observer access, while a human observer Persona edge requires its paired UI context. Hidden required endpoints still prevent admission or resolved disclosure. |
| Reader mutation and hidden target | Reader write returns `PERMISSION_DENIED`; hidden direct target access follows `NOT_FOUND`. No hidden namespace/title/member/contact/bytes/path details through diagnostics or hydration. |
| Authorized reverse/reference queries | Discover membership and every responsibility alternative, including nested selected owners, with original full selector, kind, assignee UID and association ID. No broad entity-to-member expansion. |
| Partial access | Keep owner-visible authored declarations without revealing newly hydrated hidden details. Hidden referrer counts, totals and pagination do not leak their existence. Inaccessible lookup never becomes confirmed empty. |
| Guarded single/batched safe delete | Incoming membership and entity/member/port/connection-owner assignments block deletion, including hidden referrers. Check all requested targets together across namespaces in the final merged graph. |
| Final-graph batch removal | Explicitly remove/replace referrers and delete targets atomically when the complete final graph permits. Branch tombstones and replacements suppress obsolete baseline references. |
| Forced deletion | Remove only chosen entities, retaining unresolved assignments/membership on other owners. Deleting Team/Person preserves its subjects/Persons. A same-key recreation obeys the selector's native incarnation policy. |
| Visible scan budget | Enforce combined 5,000 resource rows including new Teams/Persons, and 10,000 examined occurrences/4,096 output records per request. Pagination and batches do not reset budgets. No partial success masquerades as complete empty. |
| Impact/closure | Preserve exact visited typed subjects and repeated occurrences with checked scope. Mark partial bounded traversal incomplete; no negative responsibility or execution claims. |
| Safe-delete completeness | Independently scan all needed incoming references without visible resource-row ceiling, with bounded concurrency and total timeout. Incomplete scan/backend failure refuses deletion; capped visible reporting never authorizes it. |
| Historical context | Native history/branch inspection identifies its actual checked context and coverage. Current directory hydration is never shown as a historical observation or a retained snapshot guarantee. |

## Rejections and operational failures

`RESPONSIBILITY_VALUE_INVALID` and `RESPONSIBILITY_REFERENCE_UNRESOLVED`
identify value and exact-selection diagnostics. Acceptance records must retain
the actual public status and structured field/association details rather than
infer success from a diagnostic name.

| Failure class | Required distinction |
| --- | --- |
| Native structural/model rejection | Malformed value/ID, wrong kind/direction, missing oneof, duplicate selection, self-target, unsupported selector and stale exact selection have distinguishable privacy-safe reasons. Use existing `InvalidArgument`/`FailedPrecondition` policy as applicable. No persisted mutation. |
| Authorization | Preserve public `NOT_FOUND`/`PERMISSION_DENIED` boundaries and do not turn hidden target details into model findings. |
| Concurrency | Stale etag/revision or changed branch base requires existing guard failure and fresh preview; never report successful retarget or partial batch. |
| Budget | `RESOURCE_EXHAUSTED` is distinct from missing subject. Only existing visible delete-report fallback may return explicitly incomplete referrer reporting. |
| Operational | Timeout, storage/authorization service failure, decoding error and worker failure propagate as operational errors, including `DeadlineExceeded`/`Unavailable` where applicable. Never convert them into missing members, empty assignments or safe deletion. |

## Authoring, transport and review evidence

| Surface | Required acceptance evidence |
| --- | --- |
| Backend public writes/reads | Persist and retrieve the fixture through public service paths, with exact IDs and guards; prove rejections leave stored state unchanged. Core helper tests alone cannot establish persistence or authorization. |
| References/impact/authorized closure | Exercise every nested selector, privacy boundary, branch view and exact-query classification through actual authorized service requests. |
| CLI native authoring | Create/update/read/export/reapply Team/Person with preserved presence and returned identities; reject stale/unknown IDs and unsupported selectors through the service. No handwritten client identity rematching. Use the commands in the contract's typed-query table. |
| MCP native authoring | Exercise equivalent native operations and typed queries over the MCP transport, preserving request guards, structured error classes and exact selectors. Use the tools in the contract's typed-query table. |
| Native copy/delete/branch commands | Prove preview/apply, safe/force deletion, clone mappings, merge/conflict identity and unchanged dangling landing through public surfaces. |
| Studio review | Render backend assignments by full subject and kind; separate Team contacts from direct Person assignments. Show stale/inaccessible/incomplete states, historical context and exact member/port/occurrence identity without client-inferred ownership or authorization. |
| Connected browser | Execute review with authorized deployed service/browser context and retain exact result evidence. Component-only rendering checks do not establish deployed behavior. |

Implementation acceptance requires a case-to-evidence record identifying actual
requests/results, executed test names, diagnostics, retained fixture identities,
checked branch/revisions and authorization context. Unexecuted cases remain
requirements. A passing scoped documentation check or existing native fixture
is not evidence of Team/Person implementation, end-user acceptance or release
readiness.

## Executable coverage and remaining evidence

| Source | Implemented coverage |
| --- | --- |
| [Connected responsibility fixture](../../rsworkspace/crates/trogon-atlas-server/tests/responsibility_fixture.rs) | Uses exact version `1` in `responsibility-fixture`. ReadModel `receipt` connects Specification `receipt-contract` message mapping, DataProduct `receipts` output and distinct repeated sources in AutomationSlice `observe`. Person `maintainer` selects the message and output port; `receipts-team` selects the product and one observation. `support-team` shares that Person without inherited assignments. Checks typed lookup, membership/contact provenance, generic reference/impact selectors, cap-zero hydration, reorder and retained unresolved member after force deletion. |
| [Authenticated backend regressions](../../rsworkspace/crates/trogon-atlas-server/tests/responsibility_backend.rs) | Public service tests cover authorization and hidden-referrer protection, branch/tombstone and unchanged dangling behavior, exact member/port/connection selection, final-batch safe-delete preview/apply with exact guard hydration, retarget refusal and supported retargeting, generic graph provenance and occurrence/output budgets. Generic discovery tests cover baseline/branch and warmed caches, hidden-owner isolation, targeted reads, closure truncation and independent safe deletion under unrelated graph excess. |
| [Core identity tests](../../rsworkspace/crates/trogon-atlas-core/tests/responsibility.rs) | Profile/value limits, optional presence, exact selector and complete sibling identity validation, role/full-subject fingerprints, reorder/reapply/retarget/removal/clone/branch restoration and generic Reference wire provenance. |
| [Complete store reference guards](../../rsworkspace/crates/trogon-atlas-store/tests/reference_guards.rs) | Uncapped final-overlay scan suppresses replaced/deleted owners, includes newly created owners, checks branch state and untyped slice precedence, and rejects duplicate/mismatched overlay keys. |
| [Manifest round trips](../../rsworkspace/crates/trogon-atlas-client/tests/responsibility_manifest.rs), [typed transport](../../rsworkspace/crates/trogon-atlas-client/tests/responsibility_transport.rs) and [CLI integration](../../rsworkspace/crates/trogon-atlas-cli/tests/responsibility_cli_integration.rs) | Typed profiles/selectors, native authoring/export/reapply, query metadata/status preservation, structured budget failures, malformed requests and identity-stripping/unknown-ID refusal. |
| [MCP integration](../../rsworkspace/crates/trogon-atlas-mcp/tests/mcp_integration.rs) | Native Team/Person authoring and typed queries preserve actual IDs, scope, exact reapply values and structured errors through the MCP transport. |
| [Studio inspector](../../tsworkspace/webapps/studio/src/components/ResponsibilityInspector.test.tsx), [subject selectors](../../tsworkspace/webapps/studio/src/lib/responsibility.test.ts), [gateway requests](../../tsworkspace/webapps/studio/server/index.test.mjs) and [production codec contract](../../tsworkspace/webapps/studio/server/responsibility-contract.test.mjs) | Read-only exact subject selection, assignee/membership/contact presentation, stale/inaccessible/incomplete states and historical context. Real serializer-boundary cases reject malformed enum, boolean, oneof and nested values before normalization can change query intent, while preserving valid typed requests. Component/transport coverage does not establish deployed-browser behavior. |

These focused tests establish the behavior they exercise, not exhaustive proof
of every requirement row. Exhaustive entity/connection inventory permutations,
all clone/merge and cross-namespace combinations, the resource-row budget at
production graph sizes, historical retention operations and deployed-browser
acceptance still need their own case-to-evidence records. Directory sync,
external import/export fidelity and runtime execution remain outside this
native slice.
