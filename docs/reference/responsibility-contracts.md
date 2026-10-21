# Responsibility contracts

Status: native Team/Person schema, responsibility admission, typed queries and
native client surfaces are implemented. The
[acceptance reference](responsibility-acceptance.md) separates executable
coverage from remaining acceptance requirements and deployed-browser evidence.

Team and Person use entity kinds 34 and 35 and union fields 35 and 36.
Contract revision and minimum client/server mutation revisions are 13;
upgrade connected clients and servers together.

## Scope and authority

A Team or Person declares catalog responsibility for an explicitly selected
native subject. Responsibility neither grants authorization nor creates
Event Modeling behavior, runtime access, provider exposure or data lineage.
`OWNER`, `MAINTAINER`, `STEWARD` and `CONTACT` describe catalog assignments;
they are not security roles. Multiple parties may declare the same kind for
the same subject; there is no inferred exclusive owner or precedence.

Person is a retained directory profile, distinct from Persona, which models a
human behavioral role, and from an authenticated principal. This release has
no principal binding, email-based deduplication, login provisioning, automatic
directory lookup or identity verification. Matching names, contacts or external
pointers never merge people or establish that they are the same person.

The release selects exact native versions only. Family coverage, latest-version
selection, source overlays, source handles, import/sync from directories or
external catalogs, format parsers and remote fetching are deferred. Native
manifest export/reapply preserves Atlas declarations, not external-source
fidelity. Greenfield wire breaks remain permitted under the existing
[development policy](../explanation/wire-compatibility.md#current-development-policy).

## Entity values and retained profiles

Existing entity concurrency, registered metadata, server-owned system metadata
and namespace authorization rules apply.

| Shared field | Contract |
| --- | --- |
| `id` | Required existing `Id`: namespace, slug and exact positive breaking-change version. |
| `title` | Required `DirectoryDisplayName`, used as the Team label or Person display name. |
| `doc` | Optional Markdown; absent and empty mean no documentation. |
| `metadata` | Existing registered typed annotations and payload validation. Arbitrary nested values do not become discoverable references. |
| `supersedes` | Optional exact prior same-kind, same-namespace, same-slug version, under existing terminating single-parent rules. |
| `contacts` | Collection of `ContactPoint`; empty means no authored contact. |
| `responsibilities` | Collection of identified `Responsibility` associations; empty means no direct authored assignment. |

| Team field | Contract |
| --- | --- |
| `purpose` | Optional nonblank prose describing the team's catalog purpose. |
| `members` | Collection of identified `TeamMember` associations to exact Persons. No nested Team membership or membership role field. |

| Person field | Contract |
| --- | --- |
| `role` | Optional `DirectoryRoleText`, an unverified directory job/role description. It has no Persona or authorization semantics. |
| `directory_pointer` | Optional opaque `ExternalDirectoryPointer`; no automatic resolution. |

| Value object | Validation |
| --- | --- |
| `DirectoryDisplayName` | Nonblank UTF-8 text, at most 256 encoded bytes. Preserve spelling; no identity normalization. |
| `DirectoryRoleText` | Nonblank UTF-8 text, at most 512 encoded bytes. |
| `ContactPoint` | Exactly one of `email: EmailAddress`, `web: ContactUrl`, or `chat: ChatContact`. Optional nonblank label of at most 128 UTF-8 bytes. No custom payload alternative. |
| `EmailAddress` | At most 254 ASCII bytes; nonempty local and domain portions separated by exactly one `@`, with no whitespace/control characters. A catalog address, with no mailbox lookup or ownership claim. |
| `ContactUrl` | Absolute HTTPS URL of at most 2,048 UTF-8 bytes; host required, user information forbidden. Stored inertly, never fetched by admission/query. |
| `ChatContact` | Required `service: DirectoryProvider` and `handle: OpaqueDirectoryKey`. No service-specific SDK prefix or parsing rule is implied. |
| `ExternalDirectoryPointer` | Required `provider: DirectoryProvider` and `key: OpaqueDirectoryKey`. No URL, snapshot, credential or arbitrary attributes bag. |
| `DirectoryProvider` | Case-sensitive ASCII token matching `[a-z][a-z0-9.-]*`, at most 64 bytes. A declared label, not a configured integration. |
| `OpaqueDirectoryKey` | Nonblank UTF-8 text, no control characters, at most 512 encoded bytes; exact comparison only. |
| `TeamRef`, `PersonRef` | Required exact native `Id`; no incarnation or revision pin, consistent with existing typed entity refs. |

Optional records preserve presence; present empty records are invalid.
Unknown enums and missing/empty subject alternatives reject native writes.
Contacts are values, not identified connections. Exact duplicate contacts,
including label, within one owner are rejected; order is preserved for export
but supplies no identity. Removing and reordering contacts requires only the
normal owner write guard. Different owners may retain the same contact or
directory pointer without deduplication or a uniqueness claim.

The ceilings are 16 contacts per owner, 1,024 members per Team,
1,024 responsibilities per owner, 2,048 total identified associations per
owner and 1 MiB for the entire protobuf-encoded entity. Lower existing entity,
RPC or store ceilings still apply. Exceeding a ceiling rejects the complete
write; values and references must never be silently truncated.

Only the authored profile above is retained, together with normal documentation,
metadata and system history. Raw directory snapshots, secrets, credentials and
automatic enrichment have no fields in this design. Free text and metadata
can still contain personal information; this contract supplies no automatic
personal-data classifier or eraser.

Profile edits change the current entity, not every retained copy. Native
[entity revisions](../explanation/changesets.md#entity-history-the-other-half-of-the-log)
record before/after images for attributed writes in a separate unbounded
revision bucket; current entity storage has history depth one. Revision history
is not guaranteed to cover unattributed writes. Branch values, native exports,
configured mirrors and backups can retain additional copies. Current deletion
or contact removal promises no personal-data purge, history rewrite, retention
deadline or complete historical record. Historical reads use their existing
authorization rules and must not be replaced by current profile hydration.

## Membership and responsibility values

| Association | Fields and rules |
| --- | --- |
| `TeamMember` | Persisted `connection_id`, required exact `person: PersonRef`, optional `doc`. Same exact Person may appear only once per Team. Different Person versions are different selections. |
| `Responsibility` | Persisted `connection_id`, required `kind` from OWNER, MAINTAINER, STEWARD or CONTACT, required `subject: ResponsibilitySubject`, optional `doc`. Same kind and full typed subject may appear only once per owner. |
| `ResponsibilitySubject` | Exactly one existing native selector: `entity: EntityRef`, `connection: ConnectionRef`, `member: ContractMemberRef`, or `port: DataPortRef`, subject to the inventory below. |

Membership is authored only on Team. Person-to-Team membership views and
subject-to-assignee views are derived queries, never independently editable
reverse lists. Empty membership means no authored members in the checked Atlas
view, not that an external directory is empty or authoritative. There is no
inherited responsibility: members do not receive their Team's assignments, and
a member's assignments do not become Team assignments. Team contact expansion
is a separate query from direct responsibility lookup. An empty direct lookup
does not prove nobody has operational responsibility.

## Subject inventory and exact resolution

Entity subjects allow the existing kinds Event, Command, ReadModel, Processor,
UI, Persona, Swimlane, CommandSlice, ReadModelSlice, AutomationSlice, UiSlice,
Storyboard, EventModel, Component, ExternalSystem, Tracker, BoundedContext,
Domain, Subdomain, Schema, Project, Screen, Term, Ambiguity,
ServiceLevelIndicator, ServiceLevelObjective, AlertPolicy,
AlertNotificationTarget, TypeLibrary, Channel, DataStore, Specification,
DataProduct, Team and Person. Unspecified/unknown kinds and
non-entity values are rejected. An assignment to the assignee's own exact
kind/key/version is rejected as a redundant self-target. Assignments between
different Team/Person entities, including mutual descriptive cycles, are valid.
They create no delegation or recursive responsibility inheritance.

Entity selection names only that exact kind, namespace, slug and version.
It never expands to members, ports, connections, contained entities, members of
a Team, superseding versions or a version family. Reverse lookups preserve
the typed subject and assignment kind. Member/port/connection responsibility
does not imply responsibility for its owner, endpoints, siblings or children.

Connection subjects allow only persisted occurrences in this closed inventory:

| Exact owner kind | Allowed stored occurrences |
| --- | --- |
| CommandSlice | `persona`, `ui`, `command`, `emitted_events[]` typed edges. |
| ReadModelSlice | `source_events[]`, `read_model` typed edges. |
| AutomationSlice | `source_read_models[]`, `processor`, `emitted_command` typed edges. |
| UiSlice | `persona`, `source_read_models[]`, `ui` typed edges. |
| Storyboard | Entry `read_model`; human observer `persona`/`ui` or automation observer `processor`; outcome `events[]` and `ui` typed edges. |
| Swimlane | `transitions[].next[]` identified lifecycle transitions, including their source Event context. |
| UI | Identified `transitions[]` navigation occurrences. |
| Channel | `ChannelBinding` occurrences. |
| DataStore | `ReadModelAttachment` occurrences. |
| Specification | `SchemaDefinition`, `SchemaBinding`, `MessageReadModelMapping`, `OperationMessage`, `ChannelOperationBinding`, `ProviderExposure`, `ApiUsage`, `ContractDependency` occurrences. |
| DataProduct | `InputDataSelection`, `OutputDataSelection`, `OutputContract`, `OutputAccess`, `LineageInput` occurrences. |

All other connection owners and paths are excluded, including the
Team membership and Team/Person responsibility associations themselves. Those
associations are identified and discoverable but are not responsibility
subjects in this release. Raw reference lists such as `Processor.calls`,
Storyboard slices and trace steps, reliability measurement-span `Connection`,
annotation paths, screen slots and arbitrary document spans acquire no invented
occurrence identity. A diagnostic list index is never a durable subject.

`ConnectionRef` preserves exact `owner: EntityRef`, `owner_uid` and `id`.
Resolution validates the complete owner's persisted occurrence identities,
then selects the unique occurrence under the
[connection identity contract](connection-identity.md). Missing, duplicate,
malformed, removed or stale identities never match by endpoint or list index.
The subject identifies the stored declaration, not proof that its behavior ran.

| Other subject | Required resolution |
| --- | --- |
| `ContractMemberRef` | Existing exact Specification key, owner UID, member kind and member ID. Allow ARTIFACT, SCHEMA, MESSAGE and OPERATION. ARTIFACT alone may include optional `expected_digest`, checked against its retained bytes; all nonartifact kinds forbid that pin. No name/path/digest fallback. |
| `DataPortRef` | Existing exact DataProduct key, owner UID, required INPUT or OUTPUT direction and port ID. Validate the complete owner's port identities; wrong direction, stale UID or removed port fails. |

A member selection is identity selection under the
[Specification contract](specification-and-api-contracts.md#members-and-exact-selections),
not a `ContractTypeRef`, schema compatibility or format-validity claim. A port
selection follows the [product identity contract](data-product-contracts.md#port-and-association-identity),
not an offered-data or lineage proof. Members and ports are not new EntityRefs.
Their owner is discoverable for impact and safe deletion while the exact local
selector remains present in the reference record.

Exact native versions are mutable breaking-change versions. Plain EntityRef
and PersonRef follow their key after same-key recreation. Member, port and
connection refs additionally pin owner incarnation and local identity, and do
not resolve into a recreated owner. Unchanged selectors do not freeze mutable
documentation, member mappings, port data or underlying behavior. ARTIFACT's
optional digest checks bytes only. Historical conclusions require retained
checked content, revision/etag, branch and authorization context. No selector
follows new versions or silently selects a replacement occurrence.

## Association identity and lifecycle

The membership fingerprint is its association role and exact Person key.
The responsibility fingerprint is its association role, responsibility kind
and complete typed subject, including optional artifact digest presence/value.
Documentation, labels on the containing entity and collection order are not
fingerprint components. All identified associations share one ConnectionId
uniqueness domain across the containing Team/Person incarnation.

| Operation | Required identity and validation behavior |
| --- | --- |
| Create/add | Omitted IDs request server-assigned UUIDv7. Valid supplied IDs are allowed only on new-owner creation; existing-owner additions omit IDs. Returned persisted UID/IDs are authoritative. |
| Preview | May display generated identities but reserves none. Never reuse preview IDs as active IDs on existing-owner additions. |
| Reorder, edit documentation or profile | Preserve unchanged association IDs. An edited owner still undergoes complete admission. |
| Change responsibility kind, subject alternative, selected key, UID, local ID, direction or digest pin | New association identity required, including OWNER-to-CONTACT on the same subject. Retaining the old ID rejects. |
| Retarget membership | New membership association ID; no implicit Person merge or responsibility transfer. |
| Remove/re-add | Removed IDs are inactive. Later re-addition receives fresh identity. Same-endpoint replacement needs separate remove/add writes when a single write is indistinguishable from identity stripping. |
| Native unchanged reapply | Existing exact-content id-less no-op exception preserves IDs. Edited manifests cannot strip retained IDs or rematch by position/endpoints. |
| Clone/create another version | Fresh owner UID; local association IDs may be retained in that new identity scope. External exact targets remain unchanged unless explicitly mapped. Incoming refs remain on the original owner. |
| Branch edit/land/rebase | Preserve incarnation and unchanged IDs; compare recorded base, current baseline and selected branch before advancing base. An ordinary landing cannot revive IDs deleted by a present opposing same-incarnation owner. |
| Explicit conflict resolution | Preserve selected-side incarnation; renew restored IDs deleted by the opposing same-incarnation owner. Unaffected IDs remain. Whole-owner deletion with explicit keep-ours restores the surviving branch incarnation under existing rules. |
| Independent same-key creation | Different owner UIDs require explicit conflict resolution; matching profile/subjects do not merge identities. |
| Copy selected Specification/product/slice | Responsibility stays on the original exact subject. Explicitly mapping a cloned member/port/connection requires its actual returned UID/ID and a fresh responsibility association. |

No new cycle rule is added to existing behavior, TypeLibrary imports or
supersession. Membership cannot cycle because it selects only Persons. Allowed
descriptive responsibility cycles use visited typed identities in queries and
never induce execution order or transitive assignment.

## Admission, privacy and graph mutation

New or edited Team/Person owners must validate their complete proposed state,
including unchanged associations on that edited owner. Targets must resolve in
the authorized merged branch view, including guarded batch overlays and
tombstones. Exact hydration must fetch required owners beyond general display
caps; projection absence never proves nonexistence. Resolve local identities
through the existing native resolvers after authorized owner reads. This does
not require recursively validating every unrelated outgoing declaration on a
selected entity. A resolving subject identity can describe an otherwise
invalid declaration; report that declaration's diagnostics separately.

Cross-namespace selections require owner write access and authorized
read/reference access to the selected entity/owner and any endpoints required
to resolve or disclose the selected occurrence. Connection context follows
that occurrence's native role and identity fingerprint, not every reference
on its owner. An emitted Event selects its Command context; unrelated UI or
Persona declarations do not become additional permission requirements.
Reference access grants no
write access. Readable Team/Person declarations grant no access to hidden
Persons, subjects, member names, contacts, bytes or graph paths. Public direct
reads preserve the existing hidden-target `NOT_FOUND` policy, with Reader
mutations `PERMISSION_DENIED`. Queries must redact hidden hydration and avoid
revealing hidden referrers through totals, pagination, diagnostics or titles.
Authored keys already visible on a readable owner need not be falsified; new
hidden target details cannot be inferred through hydration. A hidden or failed
lookup is never a verified empty membership/assignment result.

| Mutation or retained state | Required outcome |
| --- | --- |
| Missing subject, wrong kind/direction, malformed identity, duplicate membership/assignment, invalid pin or self-target | Reject admission atomically with a privacy-safe field/association diagnostic. |
| Force-delete selected Person/entity/owner or accept removal of selected member/port/connection | Retain referring declarations as unresolved, with exact selectors unchanged. No cascade, garbage collection or matching replacement. |
| Delete Team/Person | Remove its current associations only; preserve selected subjects and member Persons. Incoming refs remain unresolved unless separately repaired. |
| Unchanged invalid owner during unrelated write/landing | Preserve and diagnose it; do not globally veto an unrelated otherwise valid landing. |
| Edit an invalid Team/Person | Require explicit repair of every invalid selection in that proposed owner. No profile-only exemption from strict admission. |
| Safely delete referenced entity/owner | Refuse while any retained incoming reference remains, including membership, responsibility and nested member/port/connection owner selections. Hidden referrers protect deletion without disclosure. |
| Guarded batch removing references and deleting targets | Check the complete final merged graph for all requested delete targets together. Deleted/referrer-replaced owners and branch tombstones must participate; no partial publication. |
| Generic entity-key retarget | May rewrite supported exact entity subjects and Person membership refs, with fresh changed association IDs and full validation. Never retarget a member/port/connection merely by swapping its owner key. |
| Explicit selector retarget | Require authorized exact replacement and current concurrency guards, preview/apply agreement and fresh changed assignment identity. No guessed owner UID, member, port or occurrence IDs. |

Subject creation with local selectors requires already known authoritative
identities. Create the selected owner first, then use returned identities in a
guarded write, or select an existing same-incarnation owner in a batch. No
temporary source handles, guessed UIDs or reserved preview identities exist.
Publication must enforce the final graph under the operation's normal guards;
a successful preflight does not authorize a stale write.

## Typed queries and bounds

Typed queries join the existing reference, impact and authorized closure
surfaces. They use the normal branch metadata and authorization context.

| RPC | CLI command | MCP tool |
| --- | --- | --- |
| `ListSubjectAssignments` | `list-subject-assignments` | `list_subject_assignments` |
| `ListAssigneeResponsibilities` | `list-assignee-responsibilities` | `list_assignee_responsibilities` |
| `ListTeamMembers` | `list-team-members` | `list_team_members` |
| `ListPersonTeams` | `list-person-teams` | `list_person_teams` |
| `GetTeamContacts` | `get-team-contacts` | `get_team_contacts` |

CLI commands accept `--request` proto3 JSON; MCP tools accept `request_json`.
Native Team/Person authoring uses existing entity and manifest operations.
No ambiguous bare-string shorthand is accepted for a ResponsibilitySubject.
Generic `Reference` records retain the original `responsibility` or
`membership` declaration and containing `from_uid`, alongside `from`,
`from_field` and `to`. Impact paths preserve those records without replacing a
local subject with its owner's EntityRef.

| Query | Typed input and required output |
| --- | --- |
| Direct assignments for subject | Exact `ResponsibilitySubject`, optional typed responsibility-kind filter. Return authorized assignee TeamRef/PersonRef, assignee UID, assignment ConnectionId, kind and original full subject. No expansion to owner/member/port/endpoint/family. |
| Assignee responsibilities | Exact TeamRef or PersonRef. Return each authored assignment and separate resolved, retained-unresolved or inaccessible hydration state. |
| Membership/reverse membership | Exact TeamRef or PersonRef. Return each containing TeamRef and Team owner UID, membership ConnectionId and exact PersonRef, with checked authorization/scope. No external-directory completeness claim. |
| Team contacts | Exact TeamRef and explicit `include_members` boolean, default false. Return team contacts separately from authorized member contacts, each with provenance and membership identity. Never present these as member responsibility assignments. |

All results identify branch, checked authorized scope, etags/revision digest,
visited exact typed identities and completeness. Existing assembled reads are
not a distributed transaction snapshot or historical replay token. Current
hydration must not be attached to a historical declaration as historical fact.
Missing/unresolved retained data, inaccessible data, complete visible empty
results, budget-incomplete traversal and operational failure remain distinct.

Visible discovery shares the current 5,000-row resource scan ceiling
across Channels, DataStores, Specifications, DataProducts and new Teams/Persons
in visible namespaces. Count each exact kind/key/version once in the selected
view per request; pagination, per-subject loops and batches do not reset it.
Additionally cap examined membership/responsibility occurrences at 10,000 and
returned assignment/membership/contact records at 4,096 per request. Contact
expansion consumes occurrence/output budgets and never recursively expands
responsibilities. Exact requested owner reads still obey entity/RPC limits.

Exceeding visible discovery limits returns `RESOURCE_EXHAUSTED`, never an empty
or falsely complete result. Existing bounded impact traversals may report
explicit incomplete coverage with visited identities; partial coverage proves
neither absence of responsibility nor deletion safety. Backend, authorization
service, decoding, timeout and worker failures remain operational errors, not
missing-subject diagnostics. Validation distinguishes model rejections from
`DeadlineExceeded`/`Unavailable` and other operational failures.

Closure-backed projections, exports and subgraph extraction also use their
existing `truncated` flag when the traversed responsibility-occurrence budget
is exceeded. That flag can describe traversal-budget truncation as well as
entity-cap clipping; callers must not interpret the returned closure as complete.

Safe-delete uses the existing independent incoming-reference guard across
namespaces with bounded concurrent reads and a total timeout, not the visible
row ceiling or a promised persistent reverse index. It checks requested targets
together against the final branch/batch graph. Any unresolved scan, timeout or
backend failure refuses deletion. Visible delete reporting may use the existing
capped fallback only on resource scan budget exhaustion, explicitly marked
incomplete; other errors remain errors. This reporting fallback never grants
permission to delete.
