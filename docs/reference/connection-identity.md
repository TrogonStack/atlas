# Connection identity

Connections with design data have persisted occurrence IDs. A connection is
identified by its exact owning entity, that entity's server-owned incarnation
UID, and its `ConnectionId`. Endpoint pairs, list positions and diagram edge
IDs are not durable connection references.

## Stored occurrences

`ConnectionId.value` is a non-nil UUID in lowercase hyphenated form. Atlas mints
UUIDv7 values when an occurrence has no ID. Clients may supply valid IDs when
creating a new owner, which receives a fresh incarnation UID. On an existing
owner, supplied IDs must already be active; new occurrences omit their IDs for
server assignment. IDs must be unique across the owning entity, including
occurrences with different roles and repeated connections to the same target.

The identity fields are:

| Stored structure | Identity field |
| --- | --- |
| `EventEdge`, `CommandEdge`, `ReadModelEdge` | `connection_id` |
| `ProcessorEdge`, `UIEdge`, `PersonaEdge` | `connection_id` |
| `Swimlane.Transition.Next` | `connection_id` |
| `UI.Transition` | `connection_id` |
| `ChannelBinding`, `ReadModelAttachment` | `connection_id` |
| `SchemaDefinition`, `SchemaBinding`, `MessageReadModelMapping` | `connection_id` |
| `OperationMessage`, `ChannelOperationBinding` | `connection_id` |
| `ProviderExposure`, `ApiUsage`, `ContractDependency` | `connection_id` |

The typed edges include slice endpoints and storyboard entry/outcome endpoints.
Their ownership is the containing entity, not the referenced primitive. A local
ID does not introduce a new top-level entity or another behavioral primitive.

Specification associations belong to the containing Specification incarnation,
including associations nested in members. Their endpoint identity includes the
containing member and declared role or selection. Selectable members have
separate `ContractMemberId` values and use `ContractMemberRef`; they are not
connections. The [Specification contract](specification-and-api-contracts.md)
defines member restoration, immutable content pins and local selection rules.

`ConnectionRef` contains `owner: EntityRef`, `owner_uid` and `id: ConnectionId`.
The core resolver checks the exact owner kind/version, incarnation and unique
stored occurrence. Missing, duplicate, malformed or removed selections do not
fall back to another occurrence with matching endpoints.

## Edits and lifecycle

| Operation | Identity behavior |
| --- | --- |
| Reorder repeated occurrences | Preserve each occurrence's ID. |
| Change documentation or annotations | Preserve the ID. |
| Add another connection to the same target | Omit its ID for a distinct server-assigned ID. |
| Change a connection's role or endpoint | Omit the ID for a new occurrence. Implicit source/consumer endpoints supplied by the owning slice count too. |
| Remove a connection | Existing references become unresolved. Its ID is no longer accepted on ordinary writes to that owner. |
| Re-add a removed connection | Remove it in one write, then add an id-less occurrence in a later write, even when the endpoints are unchanged. |
| Delete and recreate the owner | The new server-owned UID prevents old references from resolving into the replacement. |
| Copy to a new owner/version | The owner identity differs. Local IDs may be retained, but references to the old owner do not follow the copy. |
| Edit on a branch | Preserve the owner's incarnation and connection IDs. Branch selection remains part of the read context. |
| Independently create the same owner key on baseline and a branch | Distinct incarnation UIDs require explicit conflict resolution, even when authored content matches. |
| Resolve a branch conflict | Preserve the selected side's content and incarnation. Within the same incarnation, use the recorded branch base to detect occurrences deleted by the discarded side and assign those fresh IDs. |
| Keep the branch owner after baseline owner deletion | Explicit resolution restores the surviving branch incarnation with its unchanged connection IDs. |
| Land a branch | Preserve connection identity and check deletions against the recorded branch base. An intentional owner replacement keeps its new UID; a stale branch must resolve a recreated baseline explicitly. |

For lifecycle transitions the source event is part of the connection. For slice
connections, changing the paired Command, ReadModel, UI or Processor can change
the connection even when its explicitly stored target stays the same. Unrelated
sibling occurrences do not change its identity.

To request a server-generated ID when replacing a connection with the same
endpoints, remove the old occurrence in one write and add the id-less occurrence
in a later write. A single write cannot distinguish that replacement from
stripping the existing occurrence's ID.

Normal writes reject endpoint changes that retain an existing ID. The guarded
reference-retarget operation clears IDs for the occurrences it changes, and
persistence assigns fresh ones. Consumers must resolve the new occurrences
explicitly rather than have old connection references follow the retarget.

Atlas stores active connection IDs without an accumulating deletion ledger.
Ordinary writes admit supplied IDs only from the current owner protected by
the write's concurrency guard, including branch and batch writes. A valid but
unknown supplied ID is rejected; new occurrences request server assignment.

Explicit keep-ours and take-theirs resolution can restore a connection's
endpoints, but cannot revive its deleted ID. Only those restored occurrences
receive fresh IDs; unaffected occurrences retain theirs. Taking the baseline's
content can therefore leave a branch delta carrying new IDs. It is resolved
against the current baseline and can be landed normally.

Branch landing and rebasing compare the recorded base, current baseline and
selected branch value. An ID present at the base but missing from the opposing
side denotes a removed occurrence; an ID introduced on the branch can land with
its identity intact. Check this before advancing the branch's recorded base.
Different owner incarnations have separate identity scopes. Ordinary landing
rejects revival rather than renewing deleted IDs implicitly.

Occurrence deletion is compared against a present opposing owner in the same
incarnation. If baseline deleted the entire owner, explicit keep-ours resolution
restores the surviving branch incarnation and its unchanged connection IDs.

## Creation and authoring

Atlas is currently greenfield. Schema changes may break development data and
older clients; this contract does not provide a migration path for id-less
stored connections. Existing stored occurrences must have valid, unique IDs.
Writes against an owner with missing persisted connection IDs are rejected;
development data must be recreated or reimported through the current contract.

On creation and when adding an occurrence, an omitted ID requests a fresh
server-generated ID. Distinct occurrences receive distinct IDs even when their
endpoints match. The returned/stored entity is authoritative: clients preserve
its IDs on subsequent edits. A caller cannot introduce an arbitrary supplied ID
on an existing owner, even if it has valid UUID syntax. An edit that strips IDs
from retained connections is rejected rather than silently rematched by list
index or endpoint.
For declarative manifests, export the stored result after the initial apply and
retain the assigned IDs in the authored manifests before editing them again.
Reapplying an unchanged id-less manifest remains a no-op: comparison can ignore
assigned IDs only when the complete remaining content matches. The backend
preserves the stored IDs for that exact-content case too. This does not authorize
an identity-stripping update or match edited connections by endpoint or position.

Validation-only preparation may show newly assigned IDs but does not reserve
them. Keep new occurrence IDs omitted in the eventual write to an existing
owner: preview-assigned IDs are not active and would be rejected. Persistence
can assign different IDs from those shown in the preview.
Operation receipts hash the original request before persistence assigns
IDs, so retries do not acquire a different operation digest solely because of
ID generation.

Contract revision 9 gates writers that could otherwise drop the new fields.
See [wire compatibility](../explanation/wire-compatibility.md).

## Scope

The reliability `Connection` message remains a typed measurement-span selector.
Its CommandHandling, Projection, Reaction, Display, Integration and Journey
variants may span several occurrences; an ID on that selector would not identify
the underlying stored connections.

Raw entity-reference lists, such as `Processor.calls`, and repeated storyboard
slice/trace moments do not acquire occurrence IDs through this change. Those
need an explicit connection or moment representation before becoming durable
selections. Greenfield development permits changing those shapes directly;
compatibility aliases or migration layers are not a prerequisite. Positional
reference paths remain diagnostics.
Channel bindings and DataStore attachments identify their own descriptive
association occurrences. A binding's observation reference separately pins
the AutomationSlice source occurrence; those IDs are not interchangeable.
Generic entity-key retargeting discovers the pinned owner for impact but does
not rewrite a Channel observation selector. Explicitly select the replacement
observation instead. See the [resource contracts](channel-and-datastore-contracts.md).

This contract does not claim that every model reference is an individually
identified connection, or implement fitness subjects.

The core resolver is a model helper. It does not bypass backend authorization or
provide an additional RPC: callers first obtain the owner through the existing
authorized read surface. Historical reports must also retain the checked
snapshot; a connection ID alone does not freeze owner contents.
Privileged snapshot recovery restores the historical identity state recorded in
that snapshot.
