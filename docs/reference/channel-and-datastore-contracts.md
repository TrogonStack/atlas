# Channel and DataStore contracts

Status: native Channel and DataStore support is implemented in the schema,
backend, CLI/MCP authoring and Studio review surfaces. The protobuf definitions
in [architecture.proto](../../proto/trogonatlas/eventmodel/v1alpha1/architecture.proto)
are the wire contract. This reference defines their field and lifecycle rules;
[acceptance coverage](channel-and-datastore-acceptance.md) distinguishes logical
fixture requirements from executed tests.

## Behavioral anchors

A Channel describes transport for an existing ReadModel-to-Processor
observation in an AutomationSlice. Integration information is modeled as a
ReadModel. A Channel does not replace that ReadModel, create a stream, consume
raw Events, or introduce a Processor-to-Command execution edge.

A DataStore describes storage attached to exact ReadModels. It has no direct
behavioral attachment to a Schema, Swimlane, Component, Event or Command.
Schemas are reached through the attached ReadModel's existing field contract.

Native resources must have a confirmed behavioral anchor. A Channel has at
least one observation binding; a DataStore has at least one ReadModel
attachment. Unmapped source declarations are outside this first release.

## Shared entity and value rules

Each resource has the following fields. Existing entity infrastructure remains
authoritative for identity, concurrency and server-owned system metadata.

| Field | Contract |
| --- | --- |
| `id` | Required existing `Id`: namespace, slug and exact positive version. |
| `title` | Required nonblank display text. |
| `doc` | Optional Markdown. Empty and absent mean no documentation. |
| `metadata` | Existing repeated typed annotation values, subject to the same registered-type and payload validation as other entities. References in supported annotations remain discoverable. |
| `supersedes` | Optional exact prior same-kind, same-namespace, same-slug version; existing terminating single-parent version rules apply. |

Metadata cannot replace typed anchors or introduce behavioral authority.
Arbitrary nested annotation fields do not become graph references; only
supported typed reference paths participate in discovery. Metadata edits
preserve unchanged association IDs.

New typed references `ChannelRef` and `DataStoreRef` each contain an exact `Id`.
ReadModel attachments use the existing typed `ReadModelRef`. No reference uses
a version range, family selector or implicit latest version. A resource's
version does not infer or advance the version of any referenced entity.

Named values below are value objects, not unrestricted metadata bags:

| Value | Validation |
| --- | --- |
| `ProtocolName` | Lowercase ASCII token matching `[a-z][a-z0-9.+-]*`; no implicit protocol inference. |
| `TechnologyName` | Nonblank display string; preserve spelling without inferring a vendor or version. |
| `ParameterName` | Case-sensitive token matching `[A-Za-z_][A-Za-z0-9_]*`. |
| `PolicyText` | Nonblank uninterpreted text; presence makes a declaration, not a verified compliance result. |
| `RegionCode` | Nonblank case-sensitive region identifier with no assumed cloud-provider vocabulary. |
| `PositiveDuration` | Positive integer seconds; no calendar months, negative values or zero. |

Optional scalar claims preserve presence. In particular, absent
`authoritative` means unknown; `false` is an explicit negative claim. Enum
absence means undeclared; unknown enum numbers are rejected on native writes.
Free-form declarations cannot substitute for a required typed reference.

## Channel

| Field | Contract |
| --- | --- |
| `address` | Optional `ChannelAddress` declaration. Absence is valid. |
| `protocols` | Set of `ProtocolName`; may be empty when transport is undeclared. |
| `delivery_guarantee` | Optional enum: `AT_MOST_ONCE`, `AT_LEAST_ONCE`, `EXACTLY_ONCE`. A transport claim, not an end-to-end processing guarantee. |
| `broker` | Optional typed alternative: exact ComponentRef or ExternalSystemRef. |
| `bindings` | Nonempty collection of `ChannelBinding`, each selecting an actual observation. |

`ChannelAddress` contains required nonblank `template` text and a collection of
`AddressParameter` declarations. The template is an opaque transport address,
not necessarily a URL and never an instruction to connect. The only template
substitution syntax is `{ParameterName}`. Literal braces are unsupported.
Each placeholder must have exactly one matching parameter declaration, and
unused declarations are rejected.

An `AddressParameter` has a required unique `name`, optional description,
optional nonempty set of allowed string values, optional string default and a
set of string examples. A default or example must belong to the allowed set
when that set is declared. Empty string is a valid value. Required template
values are supplied by each binding or by the parameter's default.

| ChannelBinding field | Contract |
| --- | --- |
| `connection_id` | Required in persisted state; Channel-owned association occurrence ID. Omitted on addition for server assignment. |
| `observation` | Required existing `ConnectionRef`, restricted as specified below. |
| `parameter_values` | Map from declared `ParameterName` to string value, validated against the address declaration. Empty when no address is declared. |
| `doc` | Optional binding-specific Markdown. |

An observation selects exactly one `ReadModelEdge` occurrence in
`AutomationSlice.source_read_models`. The reference's owner is that exact
AutomationSlice, with its incarnation UID and source edge's connection ID.
The ReadModel is the selected edge's exact target. The consumer is the same
slice's `processor` target. Both targets and the owner must resolve in the
same branch/snapshot context.

The Processor is a derived endpoint, not another editable reference on the
binding. Validation requires a present ProcessorEdge with a valid persisted
connection ID and a resolvable Processor target. It rejects references to a
ProcessorEdge, emitted CommandEdge, projection edge, UI observation, raw
ReadModel reference, missing occurrence or wrong owner kind.

Changing the paired Processor or selected ReadModel changes the observation's
identity under the existing [connection identity](connection-identity.md)
contract. A binding to the old observation becomes unresolved and must be
explicitly replaced. No endpoint, list-position or title matching repairs it.

Within a Channel, no observation may appear more than once, even with different
parameter values. Distinct repeated source edges with identical targets are
distinct observations and may each be bound. Different Channels may describe
the same observation; validation does not infer that either declaration is
exclusive or proves transport availability.

Address values and protocol declarations describe design facts. Credentials,
tokens, connection attempts and broker provisioning are outside this model.

## DataStore

| Field | Contract |
| --- | --- |
| `store_class` | Required `StoreClass` value. |
| `technology` | Optional `TechnologyName`. |
| `purpose` | Optional nonblank prose describing the store's role. |
| `authoritative` | Optional boolean system-of-record claim. |
| `access_mode` | Optional enum: `READ`, `WRITE`, `READ_WRITE`, `APPEND_ONLY`; describes declared store capability. |
| `classification` | Optional enum: `PUBLIC`, `INTERNAL`, `CONFIDENTIAL`, `REGULATED`. |
| `residency` | Optional `ResidencyPolicy`. |
| `retention` | Optional `RetentionPolicy`. |
| `attachments` | Nonempty collection of `ReadModelAttachment`. |

`StoreClass` is a typed alternative: `DATABASE`, `CACHE`, `OBJECT_STORE`,
`SEARCH_INDEX`, `WAREHOUSE`, `EXTERNAL_SERVICE`, or `OTHER` with required
nonblank `name`. Known alternatives reject an additional custom name.

`ResidencyPolicy` has required `PolicyText` and an optional nonempty set of
`RegionCode` values. `RetentionPolicy` has required `PolicyText` and an optional
`PositiveDuration`. Parsed values supplement the policy declaration; they do
not override its text or imply that the policy is enforced. Unparsed policy
text is valid. Classification does not specify personal-data erasure behavior.

| ReadModelAttachment field | Contract |
| --- | --- |
| `connection_id` | Required in persisted state; DataStore-owned association occurrence ID. Omitted on addition for server assignment. |
| `read_model` | Required exact `ReadModelRef`; must resolve. |
| `doc` | Optional attachment-specific Markdown. |

A DataStore may attach to several ReadModels. A ReadModel may attach to several
DataStores. Within one DataStore, the same exact ReadModel may appear only once;
another ReadModel version is a distinct target. Attachment order has no
behavioral meaning. DataStore capability does not assert that every Processor
can read or write the store, or create an execution relationship with one.

## Association identity and validation

The new binding and attachment records are associations with their own IDs.
A binding's `connection_id` identifies the Channel-owned association; its
`observation.id` identifies the AutomationSlice-owned behavioral occurrence.
These values are not interchangeable.

Association identity is the exact resource owner, its server-owned incarnation
UID and its local `ConnectionId`. Occurrence traversal, reference resolution,
indexes and mutation validation support these association records.

Apply existing connection-ID admission rules: IDs are unique across the owner;
omitted IDs request server generation. A new owner may accept caller-supplied
valid IDs and receives a fresh server-owned incarnation UID; supplied IDs on an
existing owner must already be active. Reorder and documentation or parameter-value
edits preserve association IDs. Changing an observation selector or ReadModel
target requires a new association ID. Removed IDs cannot be supplied again on
ordinary writes to that incarnation. Do not add an unbounded deletion ledger.

Reapplying unchanged content with only unassigned association IDs remains an
exact-content no-op under the existing identity contract. This does not omit
the referenced observation's ID or UID, or permit stripping IDs in an edited
resource. Preserve the authoritative returned IDs before subsequent edits.

Validation checks the complete proposed resource and its references against
the operation's branch/snapshot. Guarded batches may create ReadModel targets
and DataStore attachments together when they resolve in the proposed final
graph. A new Channel
must know the observation owner's authoritative UID, not just a predicted edge
ID. Create the anchored slice first and use its returned identity, or use an
existing same-incarnation owner when preparing a guarded batch. This contract
does not introduce temporary references to not-yet-assigned owner UIDs. Preview
assigned IDs are not reserved. Publication must enforce the same reference
rules atomically; a preflight read alone cannot authorize a later stale write.

## Authorization and graph lifecycle

Cross-namespace references are permitted only in the same authorized model
scope. Namespace strings grant no access by themselves. Creating or changing
an association requires write permission on its owning resource and authorized
read/reference access to the selected owner and derived endpoints. A Channel
also checks its optional broker. Referenced entities need not be writable.
Backend reads and impact queries must not reveal inaccessible targets through
titles, parameters or relationship details.

Channel-to-observation and DataStore-to-ReadModel associations are descriptive;
they introduce no execution ordering and no new cycle prohibition on the
existing behavioral graph. Channel composition and store-to-store references
are outside this release. Existing supersession cycle rules still apply.

| Operation | Required behavior |
| --- | --- |
| Safely delete an entity with inbound references | Refuse deletion and expose the authorized dependencies through the existing reference/impact surfaces, including nested observation-owner and attachment references. |
| Explicitly force-delete a Channel or DataStore | Remove its associations; preserve behavioral anchors and targets. Incoming references to the deleted resource become unresolved. |
| Explicitly force-delete a selected owner or target | Preserve resource declarations and report unresolved references. No automatic cascade deletion or rebinding. |
| Remove or retarget an observation through an otherwise accepted guarded edit | Preserve Channel declarations and diagnose the old observation references as unresolved; never follow replacement occurrences. |
| Remove the last binding or attachment | Reject the resource write; delete the resource explicitly if no confirmed anchor remains. |
| Recreate a selected AutomationSlice | Its new UID does not satisfy old observation references, even when endpoints and local IDs match. |
| Recreate a ReadModel at the same key | An exact typed ReadModelRef follows the key, following existing entity-reference semantics; unlike ConnectionRef, it does not pin an incarnation. |
| Retarget an association | Require an explicit guarded edit and a fresh association ID; preserve unrelated associations. |
| Clone a resource or create another version | Give the new owner a fresh UID; local IDs may be retained, but references to the old owner do not follow. Preserve exact targets unless an explicit mapping changes them. |
| Clone an anchored slice | Channel bindings stay on the original owner; an explicit mapping must select the cloned observation and create a new association. |
| Branch, resolve or land | Preserve exact branch context, owner incarnation and unchanged IDs. Use recorded base/current/selected identity checks; never match associations by endpoint or position. |

A validated resource creation/update requires resolving anchors. Explicit
forced target deletion or an accepted behavioral edit can leave an inspectable
invalid graph; unrelated writes do not silently repair it. Existing guarded
mutation and validation rules still apply to the target edit itself. Reference
validation must expose those failures through existing diagnostic and impact
surfaces. This contract adds no automatic garbage collection.

Branch preview and landing apply the same distinction. New or edited resource
declarations must resolve their anchors. An unchanged resource may remain
unresolved when a permitted target deletion or behavioral edit lands; its
diagnostic state does not veto that operation.

Resource diagnostics resolve exact authorized anchors in the selected branch
view. Absence from a capped model projection does not establish that a target
is missing. A removed or inaccessible target remains unresolved; validation
does not restore it from another branch or reveal inaccessible entity details.

## First-release boundary

The implementation scope is native authoring, persistence, exact reference
queries and validation of these resources. It does not depend on a
Specification entity or a source overlay. Message contract catalogs, schema
format parsing, nested/routed Channels, field-level transport mapping, API
operation exposure, source-version mapping, and source-specific import,
synchronization and export formats are deferred. Native manifest export and
reapplication support Channel and DataStore, including their association IDs
and exact references.

Transport delivery, broker configuration, physical storage administration,
policy enforcement and credential management remain outside Atlas execution.

## Reference scan budget

Resource reference and impact scans use an independent budget of 5,000 Channel
and DataStore rows, combined, across the caller's visible namespaces. They
supplement capped model projections so a missing projection row cannot silently
remove an inbound architecture dependency. Exceeding this visible scan budget
returns `RESOURCE_EXHAUSTED`.

The safe-deletion guard separately checks for an incoming reference to the
selected target across namespaces, without returning hidden referrer data or
applying a global resource row limit. It streams candidate keys and stops at
the first matching reference, with bounded concurrent reads and a total timeout.
A negative result can require scanning every candidate key. This is not a
persistent reverse index. Timeout or backend failure refuses deletion rather
than authorizing a mutation from an incomplete scan. Branch checks use the
merged branch view, including replacements and removals.
