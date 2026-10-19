# Connection identity

Connections with design data have persisted occurrence IDs. A connection is
identified by its exact owning entity, that entity's server-owned incarnation
UID, and its `ConnectionId`. Endpoint pairs, list positions and diagram edge
IDs are not durable connection references.

## Stored occurrences

`ConnectionId.value` is a non-nil UUID in lowercase hyphenated form. Atlas mints
UUIDv7 values when an occurrence has no ID. Clients may supply a valid ID for a
new occurrence. IDs must be unique across the owning entity, including
occurrences with different roles and repeated connections to the same target.

The identity fields are additive:

| Stored structure | Identity field |
| --- | --- |
| `EventEdge`, `CommandEdge`, `ReadModelEdge` | `connection_id` |
| `ProcessorEdge`, `UIEdge`, `PersonaEdge` | `connection_id` |
| `Swimlane.Transition.Next` | `connection_id` |
| `UI.Transition` | `connection_id` |

The typed edges include slice endpoints and storyboard entry/outcome endpoints.
Their ownership is the containing entity, not the referenced primitive. A local
ID does not introduce a new top-level entity or another behavioral primitive.

`ConnectionRef` contains `owner: EntityRef`, `owner_uid` and `id: ConnectionId`.
The core resolver checks the exact owner kind/version, incarnation and unique
stored occurrence. Missing, duplicate, malformed or retired selections do not
fall back to another occurrence with matching endpoints.

## Edits and lifecycle

| Operation | Identity behavior |
| --- | --- |
| Reorder repeated occurrences | Preserve each occurrence's ID. |
| Change documentation or annotations | Preserve the ID. |
| Add another connection to the same target | Give it a distinct ID. |
| Change a connection's role or endpoint | Replace it with a new ID. Implicit source/consumer endpoints supplied by the owning slice count too. |
| Remove a connection | Retire its ID in the owning incarnation; existing references become unresolved. |
| Re-add a removed connection | Use a new ID, even when the endpoints are unchanged. |
| Delete and recreate the owner | The new server-owned UID prevents old references from resolving into the replacement. |
| Copy to a new owner/version | The owner identity differs. Local IDs may be retained, but references to the old owner do not follow the copy. |
| Edit on a branch | Preserve the owner's incarnation and connection IDs. Branch selection remains part of the read context. |
| Land a branch | Preserve connection identity and retirement history. An intentional owner replacement keeps its new UID; a stale branch must resolve a recreated baseline explicitly. |

For lifecycle transitions the source event is part of the connection. For slice
connections, changing the paired Command, ReadModel, UI or Processor can change
the connection even when its explicitly stored target stays the same. Unrelated
sibling occurrences do not change its identity.

Normal writes reject endpoint changes that retain an existing ID. The guarded
reference-retarget operation renews IDs for the occurrences it changes and the
store retires their previous IDs. Consumers must resolve the new occurrences
explicitly rather than have old connection references follow the retarget.

`SystemMeta.retired_connection_ids` is backend-owned. Client-supplied system
metadata cannot erase retirement history. Store preparation runs against the
value protected by the write's concurrency guard, including branch and batch
writes. A branch that only adds and removes a connection still carries relevant
retirement history when it lands.

## Legacy data and compatibility

Existing id-less records remain readable. Their next successful write assigns
IDs once. The returned/stored entity is authoritative: clients preserve its IDs
on subsequent edits. An edit that strips IDs from retained connections is
rejected rather than silently rematched by list index or endpoint. Distinct
legacy occurrences receive distinct IDs even when their endpoints match.
For declarative manifests, export the stored result after the initial apply and
retain the assigned IDs in the authored manifests before editing them again.
Reapplying an unchanged id-less manifest remains a no-op: comparison can ignore
assigned IDs only when the complete remaining content matches. The backend
preserves the stored IDs for that exact-content case too. This does not authorize
an identity-stripping update or match edited connections by endpoint or position.

Validation-only preparation may show newly assigned IDs but does not reserve
them. Retrying the eventual write without those proposed IDs may mint different
ones. Operation receipts hash the original request before persistence assigns
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
need their own representation and migration contract before becoming durable
connection or moment selections. Positional reference paths remain diagnostics.
This contract does not claim that every model reference is an individually
identified connection, or implement architecture resource or fitness subjects.

The core resolver is a model helper. It does not bypass backend authorization or
provide an additional RPC: callers first obtain the owner through the existing
authorized read surface. Historical reports must also retain the checked
snapshot; a connection ID alone does not freeze owner contents.
