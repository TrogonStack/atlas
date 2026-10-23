# Source integration contracts

Status: proposed, native-first design. Source registration, retained snapshots,
overlays, reconciliation runs and their persistence are not implemented by this
reference. Channel/DataStore, Specification/API, DataProduct, Team/Person and
Decision/Document are implemented native foundations. Their contracts remain
authoritative. [Acceptance requirements](source-integration-acceptance.md)
separate reusable native fixtures from future executable source acceptance.
No protobuf tags, entity kinds, RPC names or storage keys are allocated here.

## Release scope

The initial input is an explicitly supplied, immutable native manifest/file
bundle snapshot. Processing reads only supplied bytes and authorized Atlas
state. It never follows links, opens source-declared paths, downloads schemas,
connects to a repository or executes supplied content. File names are logical
bundle locators, not host paths. Namespace creation is outside reconciliation;
all destination namespaces must already exist and be explicitly selected.

Remote connectors, repository credentials, watchers, scheduling, arbitrary schema
compilers, automatic inferred behavioral conversion and portable migration
rebinding are deferred. Atlas Linter and its WASM/WIT execution boundary remain
later work. Retaining a format label or opaque selector does not validate its
semantics. A native TypeLibrary uses its existing supported admission path;
this proposal adds no compiler for retained artifacts.

An initial run may create native entities and update explicitly adopted existing
entities. Cross-owner incarnation selectors must already identify an authorized,
persisted target or an explicitly mapped existing incarnation. This release
does not reserve owner UIDs or introduce temporary native selectors. A graph
requiring UIDs of owners created by the same run is unsupported as one apply.
It requires explicit staged runs with returned identities and fresh previews;
each committed stage is independently visible and is not a migration transaction.

## Native authority

All integration messages retain ReadModel semantics. REQUEST, RESPONSE and
NOTIFICATION are Specification operation roles, not instructions to manufacture
Commands or Events. A Channel selects an exact ReadModel-to-Processor observation
in an AutomationSlice. A DataStore attaches to exact ReadModels. Neither a source
label, transport address, schema name, relation nor ownership claim creates a
causal endpoint, producing path or authorization grant.

Native declarations must satisfy their existing complete-state validation,
identity, authorization and guarded mutation contracts. Source evidence can
remain incomplete without creating incomplete native resources. Unmapped
knowledge belongs in the source overlay. Native responsibility, declared lineage,
checked model paths and runtime evidence remain distinct.

## Proposed control records

These names and shapes are design vocabulary, not new native model entities or
an accepted wire/storage schema. They live in a proposed backend-owned source
control plane and are queryable knowledge. They cannot be smuggled into arbitrary
native annotations to bypass identity or reference validation.

| Proposed record | Required content |
| --- | --- |
| `SourceRegistration` | Opaque source identity, authorized administration scope, existing destination namespaces, input profile revision, field ownership policy revision, retention policy and run budgets. No connection secrets in the initial profile. |
| `SourceSnapshot` | Registration identity, opaque source version, verified inventory and bundle digests, exact supplied file bytes/media types, inventory coverage, source resource/relation identities, source pointers and profile revision. Immutable after acceptance. |
| `SourceOverlay` | Snapshot-scoped source declarations, ordered repeated relation occurrences, raw pointers, mapping status, authorized candidate identities and structured unsupported/loss records. Unmapped/incomplete declarations remain here without native behavioral authority. |
| `SourceMapping` | Explicit source identity to typed native selector, field ownership and adoption decision, merge-base source/native values, checked branch/scope, persisted owner UID/local IDs, revision guards and last accepted run. |
| `ReconciliationRun` | Immutable input/configuration identity, preview digest, bounded operation plan, guards, conflicts/losses, lifecycle, cancellation disposition, native operation receipt linkage and retained outcome/mapping evidence. |

Each record has an opaque identity and concurrency revision. Mutable registration
and mapping revisions never alter historical snapshot bytes or settled run
inputs. Configuration changes require a new preview. Concurrent runs targeting
the same registration, branch or native ownership unit must serialize or fail a
guard; they cannot race to advance a common merge base.

### Source identity, versions and inventory

A source resource key is a typed tuple of registration identity, source resource
category and opaque source-local key. A source revision adds exact opaque source
version text. Case and spelling are significant; no semantic-version parsing,
numeric cast, latest fallback or ordering is implied. A different version may
retain identical bytes. Reusing one registration/version with changed bytes,
inventory scope, coverage, pointers or relations is a conflict. Repeating the
identical snapshot is idempotent. Snapshot identity uses the supplied-byte
encoding below, independently of native content normalization.

### Supplied-byte digest encoding

The proposed `supplied-native-v1` profile accepts an inventory document and a
finite set of named files; it does not accept archives. The inventory is exact
UTF-8 JSON bytes describing the registration, opaque source version, input
profile revision, partition/coverage, resource keys, ordered relation occurrences
and pointers. Repeated object keys, invalid JSON and invalid UTF-8 reject before
acceptance. Its eventual typed wire schema must represent those fields without
an arbitrary-property escape hatch. Whitespace and object-key order are retained
input bytes, so changing either changes source identity even if native content
would compare equal.

Define `frame(bytes)` as an unsigned 64-bit big-endian byte length followed by
those bytes. The bundle digest is SHA-256 over this concatenation:

```text
UTF8("atlas.source.bundle.v1\0")
frame(inventory_document_bytes)
uint64_be(file_count)
for each file ordered by its logical name's UTF-8 bytes:
  frame(UTF8(logical_name))
  frame(UTF8(media_type))
  frame(file_bytes)
```

The inventory digest is SHA-256 over
`UTF8("atlas.source.inventory.v1\0") || frame(inventory_document_bytes)`.
Each file also retains SHA-256 of its exact content bytes. Domain labels contain
a trailing zero byte; the displayed `\0` denotes that byte, not two text
characters. Digests are lowercase hexadecimal SHA-256 values tagged by purpose
and encoding revision. Counts and lengths are checked before allocation and
against the profile's limits.

Logical names are unique, case-sensitive, relative slash-separated names, at
most 1,024 UTF-8 bytes. Empty segments, `.`/`..`, leading/trailing slash,
backslash, control characters, URI schemes and drive prefixes reject; no path
or Unicode normalization is performed. Neither logical names nor media types
are trimmed or inferred. A media type is required nonblank ASCII without control
characters. Duplicate logical names reject rather than overwrite. File-list
transport order does not affect the digest; authored inventory relation/list
order does. Changing version, scope or coverage must change the inventory bytes
and therefore its digest. Transport timestamps, current native resolutions and
mutable mapping decisions are outside snapshot identity; they belong to checked
overlay/run evidence.

Atlas's positive numeric `Id.version` remains an explicitly selected native
breaking-change version. Source versions do not allocate or advance it, set
`supersedes`, supersede a Decision or retarget consumers. Several source revisions
can describe edits at one native version, subject to guards. Distinct source
contract versions remain independently addressable even when an explicit mapping
selects the same native key; incompatible simultaneous ownership is a conflict.
Deleting a source item does not free its identity for silent reassignment.

The snapshot declares a finite inventory partition: exact source categories,
logical roots and selected keys, with COMPLETE or PARTIAL coverage. COMPLETE
asserts that every resource and relation occurrence in that partition is
enumerated and captured without skip, truncation, decode error or unresolved
authorization. It says nothing about another partition or namespace. Missing
coverage defaults to PARTIAL. Coverage cannot be upgraded from a filtered view,
display projection or incomplete export.

### Overlay identity and resolution

Each source relation has its source owner key, stable source occurrence key,
declared role, ordered position and raw target pointer. Repeated equal pointers
remain distinct occurrences. Position is presentation/order data, never durable
identity. Preserve authored storyboard and trace recurrence in native payloads;
those lists do not acquire invented native connection IDs.

For native inputs, explicit persisted local IDs identify selectable occurrences.
An input lacking durable source occurrence keys can retain snapshot-local
locators for diagnosis, but cannot claim continuity across reordered/edited
snapshots. Ambiguous correspondence requires explicit mapping or conflict, never
endpoint/title/index matching. A source pointer is inert typed data plus its
original spelling; it is not a native reference or permission to fetch content.

| Overlay status | Meaning |
| --- | --- |
| RESOLVED | Explicit mapping resolves its full typed selection in the recorded authorized context. It proves neither execution nor external pointer validity. |
| MISSING | Complete authorized lookup establishes that the selected source/native item is absent in that checked scope. |
| AMBIGUOUS | More than one correspondence remains; retain authorized candidates without selecting one. |
| DEFERRED | Mapping or interpretation was intentionally not attempted or is unsupported, including unavailable incarnation rebinding. |
| CONFLICT | Conflicting declarations, ownership, byte identity, guards or merge decisions prevent acceptance. |

Hidden/missing hydration is indistinguishable when disclosure is unsafe; expose
a generic unavailable diagnostic, not a verified MISSING assertion. Operational
errors retain error semantics. Partial inventory, failed lookup and deferred
interpretation never prove absence. Status is checked evidence tied to a context;
it does not track future native changes without another authorized check.

## Exact native mappings

Mappings select a closed existing typed alternative. They retain every selector
component and optional pin's presence/value; a bare string or owner-only key is
insufficient for local selections. All owners' complete persisted identity
inventories and required native context must validate before selection.

| Selection | Required mapping and validation |
| --- | --- |
| Native entity | Exact kind, namespace, slug and positive version. Record observed UID and etag separately to guard reconciliation; a plain native reference still follows its exact key after recreation. |
| Connection occurrence | Existing `ConnectionRef`: exact owner, owner UID and `ConnectionId`, with native role and implicit paired endpoints checked. Equal endpoints do not merge occurrences. |
| Specification member | `ContractMemberRef`: exact Specification, owner UID, member kind and member ID; optional expected digest only for ARTIFACT. |
| Artifact-backed schema type | Existing `ContractTypeRef`, including SCHEMA selection, expected artifact member ID, exact digest and opaque selector including absence. It is not interchangeable with an ARTIFACT selection. |
| DataProduct port | `DataPortRef`: exact product, owner UID, INPUT/OUTPUT direction and port ID. Port selection does not prove data, access or lineage. |
| Document | Whole `DocumentRef` or existing `DocumentAnchorRef` with exact owner UID, anchor ID and optional expected digest. Whole and anchor selections remain distinct. |

Source mappings may retain Decision option or other internal local identities
needed to reconcile native content without adding them to any native subject
inventory. A source relation does not become a selectable native ConnectionRef
unless an existing native contract actually defines that stored occurrence.
Team membership and catalog responsibility never become authenticated identity
or inherited access. Existing native subject inventories remain closed.

### Identity lifecycle

| Change | Required reconciliation behavior |
| --- | --- |
| Reorder, rename or documentation edit | Preserve retained native identity under its kind-specific rules. Preserve repeated occurrences and authored order; do not sort a causal list for comparison. |
| Retarget connection/association or change role | Fresh association ID, including implicit paired endpoint changes. Explicitly replace any dependent mapping/selector; incoming references never follow automatically. |
| Retarget a message mapping or product data selection | Preserve member/port identity where native rules permit; replace changed mapping/data association identity. Recheck dependent type/access pins and claims. |
| Change artifact bytes/format/media/format version | Fresh artifact member under native immutability rules. Explicitly replace dependent schema definitions and pins; retain old artifact while still selected or report unresolved references on permitted removal. |
| Edit Document bytes or anchor | Preserve authored anchor ID for content/label/order edits; old byte pins become stale. Changing fragment requires fresh anchor ID. Link body requires removing anchors. |
| Remove/re-add local occurrence/member/port/anchor/option | Respect native active-ID admission. No revival by name, position, content or endpoint; indistinguishable remove/add requires separate guarded writes. |
| Delete/recreate owner | Fresh UID. Mark the active mapping stale and require explicit adoption of the replacement, including for an entity mapping whose plain native reference would follow the key. Never reuse old local selectors. |
| Clone/new native version | Fresh owner UID; native local IDs may be retained where allowed, but incoming selections remain on the original. Source-version mapping changes only explicitly. |
| Branch/rebase/land | Compare recorded base, baseline and branch identities. Preserve unchanged IDs; ordinary merge cannot resurrect opposing-side deletions in the same incarnation. Explicit restoration follows native fresh-ID rules and requires new mapping evidence. |

For whole-owner deletion, native explicit keep-side restoration can retain the
surviving incarnation. Independent same-key creation remains an incarnation
conflict. Source reconciliation must not convert either case into automatic
identity matching. Branch mappings and merge bases are scoped to that branch;
landing requires a new baseline check and explicit mapping advancement, not
blind promotion of branch provenance. Tombstones participate in the final view.

## Inventory deletion and native lifecycle

PARTIAL input only proposes changes to explicitly supplied items. Omission means
unknown, including omitted relations on an incompletely described source owner;
it never means delete, clear a collection or detach a native association.
Explicit complete native owner payloads are still evaluated as whole native
puts; the profile must distinguish those from partial source patches.

Only a verified COMPLETE inventory can propose removal of items formerly
present in the same partition's accepted merge base. A new scope has no implicit
deletion base. Each candidate must have an active mapping with matching native
incarnation, guards and explicit ownership for the affected fields/occurrences.
Absence in source is a proposal to retire that source mapping, not permission
to delete the native owner. Native deletion additionally requires explicit
deletion policy, exclusive source ownership without retained local fields or
other source claims, a complete dependency check and a reviewed guarded delete.
Otherwise detach the mapping or report conflict and retain the native resource.

No source run implicitly force-deletes or cascades. Existing safe-delete guards
protect every target against the complete final branch overlay, including hidden
referrers. Removing selected local identities follows native local-removal rules
and may leave incoming declarations unresolved; preview must expose authorized
impact. New/edited owners must fully validate; unchanged dangling declarations
remain inspectable under native rules and do not veto unrelated accepted edits.

Historical snapshot/mapping pins are provenance, not incoming native graph
references. They neither block ordinary native deletion nor grant access.
Deleting a native owner leaves retained source evidence intact and marks its
active mapping stale on the next check; no synchronous watcher is implied.
Explicit mapping detach relinquishes future reconciliation authority while
retaining bounded historical evidence. Source registration deletion does not
delete native resources and is refused while a run requires recovery to settle
a possible native commit. It cannot discard that run's protected intent or
marker. Native deletion does not promise historical erasure.

## Field ownership and merge base

Every writable native field or identified collection unit has one explicit
owner: native/local authoring or a named registration. Adoption of an existing
owner requires an explicit authorized decision bound to its observed UID/etag;
matching a slug/title is not adoption. Ownership policy is itself guarded.
Source claims alone cannot seize fields. Another registration claiming the same
unit is a conflict unless an explicit transfer is approved in a new preview.

The first release treats ordered collections as whole ownership units unless
the native contract provides durable item identity and the profile defines a
bounded identity-aware merge. Never merge repeated lists by endpoint or index.
Backend-owned system fields and identity allocation are never source-owned.
Locally owned rationale, strategy, responsibility or other fields are retained
unless explicitly adopted; a source snapshot does not overwrite them by default.

The merge base is the last accepted source value plus the corresponding accepted
native value, persisted identities, revision evidence, ownership policy and
branch. Keep both values: normalization means source bytes alone cannot serve
as the native base. Preview compares base, incoming source and current native
state per ownership unit, including absence and tombstones.

| Comparison | Outcome |
| --- | --- |
| Source and native unchanged | No native write; retain exact identity. |
| Source changes, native equals accepted base | Propose the source-owned change under current guards. |
| Source unchanged, native changed | Report local drift. Do not overwrite it or silently move the base to conceal divergence. |
| Both change to equivalent accepted native value | Convergent no-op; advance evidence only after guards and complete acceptance. |
| Both change differently, deletion versus edit, or identity changed | Explicit conflict; no last-writer-wins, implicit source-wins or automatic retry. |

Conflict resolution names the unit, selected value and observed revisions, then
requires a new preview. A failed, cancelled or partly prepared run never advances
the accepted base. Missing/expired base evidence requires explicit re-adoption;
it is not an empty base authorizing replacement or deletion.

## Run, preview and apply contract

Proposed lifecycle: prepared, previewed, applying, applied, rejected, cancelled
or outcome-unknown. Preparation and preview can retain source evidence but
cannot mutate native state or active mappings. A run binds exact snapshot and
configuration digests, principal/scope, branch, ownership/base revisions, ordered
operations, losses/conflicts, input limits and expiration. Preview shows native
creates, updates, unchanged items, proposed deletions, deferred items and exact
selector impact. Unresolved conflicts or unsupported mutation dependencies
refuse apply. Explicit overlay-only retention may succeed without native writes;
it must not report native reconciliation success.

Apply requires the same immutable plan and fresh authorization. Existing owners
carry exact UID and `if_match`; absent keys carry `create_only`. The backend must
also guard mapping/base revisions, branch state, absence, inventory completeness
and all reference/ownership facts on which the decision depends, including
unchanged owners. A complete scoped snapshot digest may supplement these guards;
it cannot replace UID/etag/absence checks. Snapshot identity excludes system UID
and may be truncated. Assembled reads are not a transaction snapshot.

Admission checks the complete proposed graph under the mutation boundary,
including replacements, tombstones and exact pins. Any stale guard, hidden
dependency, failed lookup or incomplete safety scan refuses publication. Preview
assigned IDs reserve nothing. Preserve unknown new IDs as unassigned until
persistence; use returned authoritative IDs for subsequent selection.

### Publication and recovery requirement

Native client `apply` currently sends guarded put-only `BatchMutate`; it does
not prune inventory or publish source control records. Its unchanged entities
are skipped and not guarded. Existing entity batches do not atomically store
source mappings, accepted bases or source run receipts.

Reliable publication of native writes and accepted source mappings/base/outcome
is a **new backend requirement**. The proposed protocol has the following durable
boundaries, including for branch-scoped runs:

- Before any native commit, persist a guarded intent containing run/plan identity,
  input/configuration/base revisions, authorization scope, complete guards and
  planned operations. Competing source runs cannot advance the same ownership
  base while this intent remains unsettled.
- The native publication boundary must commit a source-run marker together with
  the native effects. The marker binds run identity and plan digest to the exact
  committed entity values, assigned identities, resulting revisions and outcome.
  A crash must not leave native effects without their marker, or a committed
  marker for effects that did not land. This is a new storage/backend contract;
  an ordinary client-side sequence of entity writes and record writes does not
  satisfy it. A backend lacking this capability must refuse source apply.
- Publish mapping/base revisions and the settled source outcome idempotently
  from that marker under the intent's guards. Only then mark the run applied.
  Ordinary native edits after the marker are retained as later drift; recovery
  uses the recorded committed values, never substitutes a newer current value
  for the accepted base or reapplies the native batch.
- Recovery with a committed marker finishes only the evidence-publication step.
  A durable, definitive noncommit outcome settles rejected/cancelled without
  advancing the base. A missing marker or unavailable evidence alone is not
  definitive noncommit: retain outcome-unknown, block competing base advancement
  and require recovery before another source apply. Never infer rollback from
  an absent response or retry with a new identity to escape uncertainty.

Intermediate native effects may be visible while source evidence publication is
recovering; readers must see an unsettled source run rather than a false applied
outcome. This is not a distributed transaction claim. No cross-store clone or
portable owner-UID restoration is promised by existing CLI primitives. Until
these boundaries and their fault-injection evidence are implemented, preparation
and preview cannot establish a source apply runtime. Concrete storage keys,
transaction mechanism and wire encoding belong to the implementation review;
they may not weaken these publication/recovery outcomes.

Replay binds run identity to its exact request digest, scope, branch and caller
authorization. Same identity/digest returns the recorded result; changed input
under that identity rejects. A preview is not an apply receipt. Reuse native
outer operation IDs where supported without putting IDs on nested batch ops;
dry runs omit them. Native branch writes do not provide durable `GetOperation`
receipts, so source run recovery cannot assume that facility exists there.
An absent/expired receipt means unknown, never proof of nonapplication. Recheck
durable evidence and native guards before any new operation.

Cancellation before publication prevents native writes and accepted-base
advancement. Once commit may have started, cancellation records a request and
settles as applied, rejected or outcome-unknown after recovery; a disconnected
caller is not proof of rollback. Cancellation cannot undo committed native
changes. Compensation is a new guarded proposal, not automatic deletion.

### Budgets, retention and privacy

Every run has finite advertised limits for supplied/expanded bytes, file count,
resource/relation/mapping count, owner hydration, examined occurrences, native
operations, result bytes, elapsed time and retained storage. Reject before
native writes when the entire plan cannot fit. All pages, repeated pointers,
batch members and hydration share the run budget; a continuation cannot reset
it. No implicit splitting of an oversized apply into independently committed
batches. The initial apply ceiling is at most 1,000 entity operations, honoring
the lower Store limit rather than the service's 1,024 admission ceiling.
Existing lower entity, artifact, RPC and reference-scan bounds still apply.
Control-record writes and recovery work also require explicit separate bounds.
The proposed `supplied-native-v1` ceilings below are contract limits, not measured
performance guarantees or existing server settings. Registration policy may
lower them; it cannot raise them without a new profile revision. Lower native
limits still take precedence.

| Resource | Initial ceiling |
| --- | --- |
| Inventory document | 4 MiB of supplied UTF-8 bytes. |
| Named files | 256 files, at most 4 MiB each and 32 MiB combined including inventory bytes. No archive expansion. |
| Source resources and relation occurrences | 1,000 resources and 10,000 ordered relation occurrences per snapshot. |
| Active mappings considered by one run | 4,096 mappings, including detach/retirement candidates. |
| Exact native owners read | 5,000 distinct kind/key/version identities, including unchanged dependencies. |
| Selection and replacement work | 10,000 occurrence/member/port/anchor examinations or replacement hops combined; repeated references consume work. |
| Native mutations | 1,000 puts/deletes in one publication; never silently chunked. |
| Control publication work | 4,096 mapping/base updates plus 64 intent/run/marker/audit record transitions per run, including retries. Exceeding the bound leaves recovery explicit rather than publishing partial accepted evidence. |
| Returned report page | 500 records and 4 MiB encoded bytes, with a retained run-wide maximum of 4,096 result/conflict/loss records and 8 MiB. Overflow refuses apply rather than hiding conflicts. |
| Active processing | 60 seconds for preparation/preview and 60 seconds per apply or recovery attempt, including reads and compilation. A timeout after possible commit retains outcome-unknown pending durable resolution. |
| Retained registration data | 128 MiB across raw files, overlays, bases, previews, markers, receipts and audit records. Account shared content once physically but retain each logical reference. Refuse new work before crossing the ceiling. |

Retention defaults are 7 days from immutable backend-recorded creation time for
unaccepted snapshots and previews, including abandoned or partly prepared inputs
that never reach acceptance/settlement. Reads, retries and a caller's supplied
timestamps do not restart that clock. Accepted snapshots/bases expire 30 days
after their recorded acceptance; settled outcomes and their markers/audit records
expire 30 days after settlement. An unsettled recovery record is not eligible for
automatic expiry while needed to disambiguate a possible native commit. It still
counts against the storage ceiling; exhaustion blocks new runs rather than
discarding evidence. An active base reaching expiry disables apply until an
authorized new preview explicitly re-adopts current state. No source version or
run identity is silently recycled because its bytes expire: retain a bounded
identity/digest tombstone until registration deletion, counting it against the
same ceiling. Capacity exhaustion refuses registration work; it never resets
conflict or replay semantics.

Incomplete reporting is explicit and cannot establish complete inventory,
deletion safety, successful reapply or absence of conflicts. Budget exhaustion
is an operational refusal, not a missing target. Safe-deletion decisions retain
their independent complete guard despite capped visible dependency reports.

Retention specifies finite durations/byte ceilings separately for snapshots,
raw bytes, overlays, merge bases, previews, receipts and audit records. Retain
the active base and recovery evidence while needed, or refuse new writes until
explicit re-adoption. Expiry is visible and removes replay/reconstruction claims;
it never authorizes a fresh overwrite. Deleting retained source evidence does
not promise erasure of native revisions, branches, exports or backups. Do not
advertise comprehensive purge or permanent replay.

Registration administration, snapshot submission/read, run preview/apply/cancel
and native mutations require explicit authorized capabilities. Read access to a
source record grants no native write/reference permission. Cross-namespace
reads/references and destination writes are checked separately; bundled foreign
dependencies remain read-only prerequisites unless separately adopted for a
write in an authorized destination. Namespace strings, source ownership and
Team/Person records grant no permissions. Recheck privileges on apply and replay.

Raw files may contain sensitive prose, profiles and contacts. Store them only
within the authorized source scope, keep contents out of routine logs and
redact diagnostics before persistence/export. Retained raw bytes cannot be
silently redacted while retaining the original digest or a lossless claim.
Reject prohibited content or create an explicitly different sanitized snapshot.
Hidden targets/referrers, candidate names, counts and pointers must not leak
through run status, diff, errors, timing detail or export. Queries preserve
already authored selectors without adding hidden hydration. Rendering retained
text is inert and uses existing safe text boundaries.

## Native reapply and fidelity reporting

The existing [manifest reader](../../rsworkspace/crates/trogon-atlas-client/src/manifest.rs)
normalizes schema-aware shorthand and loads tenant types. The
[apply classifier](../../rsworkspace/crates/trogon-atlas-client/src/apply.rs)
ignores the server-owned system block and uses shared semantic/identity
equivalence. The [export renderer](../../rsworkspace/crates/trogon-atlas-client/src/export.rs)
omits that block while preserving local and association IDs and typed selectors
in native content. These are existing mechanisms, not source reconciliation.

| Fidelity claim | Required evidence and limit |
| --- | --- |
| Exact supplied snapshot recovery | Retain every original file byte, logical locator, media type, source version and verified digest. Re-rendered YAML cannot recover comments, whitespace or original file layout. |
| Same-store native unchanged reapply | Complete authorized export, needed tenant types and unchanged existing incarnations. Native comparison yields unchanged; preserve local IDs, ordered repeated lists, selector pins, optional presence and retained artifact/Document bytes and digests. Native system metadata is outside manifest equality. |
| Unassigned-ID reapply | Only existing kind-specific pure equivalence permits an unchanged id-less no-op. The connection helper requires every proposed connection ID absent and complete remaining semantic equality; Decision/Document has an additional local-value helper. Do not promise every partially stripped member/port/anchor combination is accepted. Edited manifests cannot discard retained IDs. |
| Reapply into another store/key/incarnation | Not incarnation-preserving restore. New owners receive new UIDs. Retained cross-owner ConnectionRef/member/type/port/anchor pins cannot be blindly reused or stripped. Initial release fails closed unless pins already resolve to explicitly mapped existing incarnations. |
| Portable rebinding | Deferred extension requiring an explicit authorized identity map, known persisted/reserved identities, renewed changed associations and bounded final-overlay admission. Existing native export/apply is insufficient. |

Snapshot digest and native content hash are different identities. Neither proves
same incarnation or freezes transitive behavior. An export/reapply report must
state which claim was checked, branch, complete captured scope, observed UIDs,
revisions, tenant type context and bytes/digests.

Every output includes a structured fidelity report: checked coverage, exact
source/native identity, field or relation occurrence locator, disposition,
reason and whether native reapply or original-byte recovery is supported.
Dispositions distinguish preserved, normalized, retained-overlay-only,
unsupported/deferred, conflicted, skipped and truncated data. Unsupported fields
and pointers remain in retained overlay/raw input where permitted; they are not
silently dropped or converted into native endpoints. Native export `skipped`
records, partial closure and all upstream incompleteness propagate. An empty
loss list is meaningful only for an explicitly complete checked profile/scope.
If required bytes, types, identity or safety evidence cannot be retained within
policy/budget, reject the claim or operation rather than downgrade it silently.

## Native references

- [Connection identity](connection-identity.md)
- [Channel and DataStore](channel-and-datastore-contracts.md)
- [Specification and API](specification-and-api-contracts.md)
- [DataProduct](data-product-contracts.md)
- [Responsibility](responsibility-contracts.md)
- [Decision and Document](decision-and-document-contracts.md)
- [Stale-plan guards](../explanation/stale-plans.md)
- [Branching](../explanation/branching.md)
- [Snapshot identity](../explanation/snapshot-identity.md)
