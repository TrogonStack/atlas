# Channel and DataStore acceptance fixture

Status: acceptance contract for the
[resource contracts](channel-and-datastore-contracts.md). The tables are logical
fixture notation rather than literal manifest syntax. The executable
[backend fixture](../../rsworkspace/crates/trogon-atlas-server/tests/architecture_fixture.rs)
materializes the connected graph through supported writes and retains returned
keys, incarnation UIDs, association IDs and checked snapshot identity.

## Read context and identifiers

Use a retained baseline snapshot with the fixture namespace `commerce` and
exact version `1` for every entity below. An identifier written as
`commerce/order-feed@1` means namespace `commerce`, slug `order-feed`, version
`1`; it does not mean latest or a version family. Record the checked snapshot
identity with query results. Branch cases use an independently named branch
forked from that baseline.

The labels `observation_a`, `observation_b`, `binding_a`, `binding_b` and
`attachment` name returned IDs in this specification. They are not UUID values
to submit. Store the actual IDs assigned by the backend, together with the
owning entity's exact key and incarnation UID. No expected result uses a list
index as identity.

Create primitives and slices first, retaining the authoritative AutomationSlice
UID and source edge IDs, then create the Channel and DataStores. A Channel
cannot predict the UID of a new slice in the same batch. A batch may instead
use existing owner identities, provided all references resolve in its proposed
final graph. Valid client-supplied occurrence IDs on a new owner are permitted
by the existing contract; they do not let clients assign the owner's UID.

## Behavioral entities

| Exact ID | Kind | Fixture content |
| --- | --- | --- |
| `commerce/order-feed@1` | ReadModel | Integration input. |
| `commerce/order-intake@1` | Processor | Processes the integration information. |
| `commerce/import-order@1` | Command | Local intent issued by the processor. |
| `commerce/order-imported@1` | Event | Local fact produced by handling that command. |
| `commerce/order-view@1` | ReadModel | Projection of the local fact. |
| `commerce/observe-orders@1` | AutomationSlice | Contains distinct ReadModelEdge occurrences `observation_a` and `observation_b`, both referencing `order-feed@1`; paired ProcessorEdge references `order-intake@1`; emitted CommandEdge references `import-order@1`. |
| `commerce/handle-import@1` | CommandSlice | CommandEdge references `import-order@1`; emitted EventEdge references `order-imported@1`. |
| `commerce/project-order@1` | ReadModelSlice | Source EventEdge references `order-imported@1`; target ReadModelEdge references `order-view@1`. |

The repeated observation endpoints are deliberate. The channel selects each
occurrence independently. All other required fields follow the existing slice
validation contract. The fixture declares no UI, human issuance, storage access
edge or additional execution order beyond these slices.

## Architecture resources

| Exact ID | Kind | Fixture content |
| --- | --- | --- |
| `commerce/order-delivery@1` | Channel | Address `orders.received`; protocol `nats`; declared transport guarantee `AT_LEAST_ONCE`; `binding_a` selects `observation_a` and `binding_b` selects `observation_b`. Both references pin the AutomationSlice incarnation. |
| `commerce/feed-cache@1` | DataStore | Class `CACHE`; technology `redis`; attachment selects `order-feed@1`. |
| `commerce/order-index@1` | DataStore | Class `SEARCH_INDEX`; technology `opensearch`; attachment selects `order-view@1`. |

These are declared design facts. The fixture does not connect to a broker,
cache or search service. No broker entity, API contract, owner directory or
source overlay is necessary to create these resources.

## Expected queries

The descriptions below define query semantics. They do not introduce RPC names
or CLI flags. The implementation must identify the public query surface used
and retain its requests and results as acceptance evidence.

| Query | Exact expected result |
| --- | --- |
| Resolve `binding_a`'s observation | AutomationSlice `observe-orders@1`, its recorded UID, occurrence `observation_a`, ReadModel `order-feed@1`, Processor `order-intake@1`. |
| Resolve `binding_b`'s observation | The same owner and endpoints, occurrence `observation_b`. It must remain distinct from `observation_a`. |
| Find channels for `observation_a` | Only `order-delivery@1` with `binding_a`; no endpoint-based match to `binding_b`. |
| Find channels for `observation_b` | Only `order-delivery@1` with `binding_b`. |
| Find stores attached to `order-feed@1` | Only `feed-cache@1` and its exact attachment. |
| Find stores attached to `order-view@1` | Only `order-index@1` and its exact attachment. |
| Find reverse references to `observe-orders@1` | Channel `order-delivery@1`, with both nested observation references separately discoverable. |
| Find reverse references to `order-feed@1` | The AutomationSlice source occurrences and the `feed-cache@1` attachment. The Channel's ReadModel association is derived through its selected observation, not an additional authored ReadModel field. |
| Traverse confirmed behavior from `order-feed@1` | The selected automation observation, `order-intake@1`, `import-order@1`, `order-imported@1`, then `order-view@1`, through the declared slices. |
| Inspect declared transport/storage facts | Channel guarantee remains a transport claim; no exactly-once processing assertion or Processor-to-DataStore access relationship is inferred. |

## Mutations and failures

Start each case from a fresh fixture unless the case explicitly describes a
sequence. A rejected write must not change the stored resource or its IDs.
Resolve failures must distinguish missing, invalid and forbidden internally;
unauthorized callers must not receive hidden entity data through diagnostics.

| Case | Required outcome |
| --- | --- |
| Create with omitted association IDs | Assign distinct IDs and return them in the authoritative stored entities. |
| Create a new resource with valid supplied association IDs | Accept them under the new owner's fresh UID. Reject malformed or duplicate IDs. |
| Supply an unknown association ID on an existing resource, or strip an ID from a retained occurrence in an edited update | Reject without matching by position or endpoint. |
| Reapply a resource with only its association IDs omitted and all remaining content unchanged | No-op; preserve stored association IDs. Referenced observation IDs and owner UIDs must still be present and unchanged. |
| Reorder source observations | Preserve `observation_a` and `observation_b`; Channel selections still resolve to their original occurrences. |
| Reorder Channel bindings or edit prose | Preserve `binding_a` and `binding_b`. |
| Add a binding to an already selected observation | Reject the duplicate within that Channel; separate observations with identical endpoints remain valid. |
| Attach the same exact ReadModel twice to one DataStore | Reject the duplicate; a different exact version is a distinct target. |
| Create a Channel binding targeting a CommandEdge, ProcessorEdge, ReadModelSlice or UiSlice occurrence | Reject: the selection must be an AutomationSlice source ReadModelEdge with a paired Processor. |
| Create a Channel binding with an index, endpoint pair, stale owner UID or unknown occurrence ID | Reject without fallback. |
| Create a DataStore attachment to Event, Command, Schema, Component or Swimlane | Reject: only an exact ReadModel reference is admissible. |
| Remove the resource's last binding or attachment | Reject; deleting the resource is a separate explicit operation. |
| Change an association target while retaining its association ID | Reject; replace the occurrence through the identity contract. |
| Safely delete a selected AutomationSlice or attached ReadModel | Report the inbound dependency and refuse the deletion. Explicit forced deletion preserves the architecture resource and surfaces its unresolved reference; it does not retarget it. |
| Safely delete an unreferenced entity while foreign namespaces exceed the resource scan budget | Permit deletion without scanning foreign resources into the caller's reference index. |
| Safely delete a target with only hidden referrers, individually or in a guarded batch | Refuse deletion, including batch preview, without exposing hidden owner keys, titles or relationship details. |
| Remove a selected observation through an otherwise accepted guarded edit | Preserve the Channel and diagnose its unresolved binding. An attempt to write that resource with the same unresolved binding fails validation. |
| Retarget a selected source or its paired Processor | A changed behavioral occurrence gets a fresh ID. Existing Channel bindings do not follow it and require explicit repair. |
| Delete and recreate the AutomationSlice at the same exact key | Its new UID prevents old Channel observation references from resolving. |
| Force-delete and recreate an attached ReadModel at the same exact key | The attachment is unresolved while absent, then resolves by exact key after recreation. It does not pin the ReadModel's former UID. |
| Create `order-feed@2` | Existing DataStore attachments remain on `order-feed@1`. |
| Copy the Channel to a new exact owner | Association ownership changes; references to bindings on the original owner do not select the copy. External observation targets remain pinned unless an explicit clone map changes them. |
| Modify on a branch, preview and land | Reorder preserves IDs and preview makes no durable changes. Land obeys the same identity and reference rules against the current baseline. Conflicts require resolution before application. |
| Land an accepted behavioral edit or forced target deletion that leaves an unchanged resource unresolved | Preview and landing succeed under the target edit's rules. Preserve the resource for inspection and report its unresolved anchor through validation. |
| Land a new or edited resource whose anchor does not resolve | Reject without publishing the resource. Unchanged unresolved declarations do not relax admission for selected resource writes. |
| Validate a retained resource whose anchors are omitted from a capped projection | Resolve exact authorized anchors from the selected branch view. Diagnose genuinely removed or inaccessible anchors without disclosing hidden entity details. |
| Remove an association on baseline while a branch edits the resource | Apply the connection-identity branch-base rules. Removed IDs cannot silently revive during landing; explicit resolution preserves unaffected occurrences and renews restored ones. |
| Target a readable entity in another namespace | Allow only with authorization to write the resource and read every referenced owner and resolved endpoint. No write permission to the target namespace is granted or required by the attachment alone. |
| Target an unreadable namespace | Reject the new association without disclosing the hidden target or its graph. |
| Create a ReadModel and DataStore attachment in a guarded batch | Resolve the attachment against the final graph and commit both together. |
| Create a Channel using a guessed UID for an AutomationSlice created in the same batch | Reject; use the staged owner identity or a known existing owner. |
| Introduce a Channel-to-Channel route or DataStore-to-DataStore relation | Reject as outside the first-release field contract. |

## Declared value cases

These variations validate resource metadata without contacting infrastructure.

| Case | Required outcome |
| --- | --- |
| Address `orders.{region}` with parameter `region`, allowed values `east`/`west`, default `east` | A binding with no explicit value uses the declared default; explicit `west` is valid. |
| Supply `north`, an undeclared parameter, an unused parameter declaration, an unmatched brace or a placeholder without its declaration | Reject the resource write. |
| Declare a default or example outside its parameter's allowed set | Reject; do not coerce or silently discard the value. |
| Omit both a required placeholder value and its default | Reject; no empty or inferred value is manufactured. |
| Supply parameter values when the Channel has no address | Reject; an address-free Channel with empty parameter values remains valid. |
| Omit delivery guarantee and DataStore authoritative claim | Preserve absence as unknown. An explicit `authoritative: false` remains present and false after a round trip. |
| Compare resource declarations | Render stable proto JSON with named enums and sorted object keys; preserve absent versus false claims and retain undecodable metadata's type and payload. |
| Declare transport guarantee `EXACTLY_ONCE` | Preserve the transport claim without inferring exactly-once command handling. |
| Supply an invalid protocol token or unknown native enum number | Reject without silently coercing it to a supported value. |
| Omit store class or use `OTHER` without a nonblank name | Reject. Known store classes reject an additional custom name. |
| Declare raw retention/residency policy text without parsed values | Accept and preserve the text without claiming policy enforcement. |
| Supply zero/negative retention duration, an empty declared region set or a blank region code | Reject; absence of parsed values is distinct from invalid supplied values. |

## Implementation acceptance evidence

Acceptance requires public backend writes, reads and guarded mutations, nested
reference and impact results, CLI/MCP coverage, and a branch preview/land run.
Negative cases must exercise the authorized service boundary; calling a core
resolver directly is insufficient authorization proof. Inspect the resources
and unresolved-reference diagnostics through Studio as a review surface.

Retain the exact fixture version, returned keys/UIDs/IDs, checked snapshot and
branch context, query requests/results and mutation outcomes. Evidence must
identify which cases actually ran. No source importer, external infrastructure
or fitness runtime result is implied by passing this fixture.

## Executable coverage

- [Resource API and authorization tests](../../rsworkspace/crates/trogon-atlas-server/tests/architecture_resources.rs)
  cover field failures, exact anchors, optional false, guarded batches,
  supersession, namespace authorization, capped-snapshot references/impact,
  safe deletion, recreation and branch preview/landing.
- [Mutation path regressions](../../rsworkspace/crates/trogon-atlas-server/tests/resource_retargeting.rs)
  cover retarget uniqueness and identity, revert and structural-resolution
  admission, and receipt replay after a forced target deletion.
- [Landing lifecycle regressions](../../rsworkspace/crates/trogon-atlas-server/tests/resource_landing_lifecycle.rs)
  distinguish permitted unresolved declarations from invalid selected resource
  writes. [Validation context regressions](../../rsworkspace/crates/trogon-atlas-server/tests/architecture_validation_context.rs)
  exercise exact anchor resolution outside the model projection.
- [Scoped deletion regression](../../rsworkspace/crates/trogon-atlas-server/tests/resource_delete_scope.rs)
  covers foreign resource volume, hidden dependencies, batch preview/apply and
  diagnostic privacy. [Store reference guard tests](../../rsworkspace/crates/trogon-atlas-store/tests/reference_guards.rs)
  cover merged branch references and untyped slice resolution precedence.
- [Resource diff tests](../../rsworkspace/crates/trogon-atlas-server/src/diff.rs)
  cover canonical values, map ordering, metadata rendering and claim presence.
- [Core association tests](../../rsworkspace/crates/trogon-atlas-core/tests/resource_connections.rs)
  cover association identity, repeated observation selection, copying and pinned
  references.
- [MCP transport tests](../../rsworkspace/crates/trogon-atlas-mcp/tests/mcp_integration.rs)
  and [CLI integration tests](../../rsworkspace/crates/trogon-atlas-cli/tests/export_cli_integration.rs)
  exercise resource authoring and reading through their public surfaces.
- [Studio component tests](../../tsworkspace/webapps/studio/src/components/ArchitectureResourceDetails.test.tsx)
  cover declarations, exact selectors, repeated observations and optional claim
  presence. These are component tests, not a deployed-browser acceptance run.

The executable backend fixture verifies the returned snapshot identity and
round-trips the complete connected model through export and client manifest
parsing. Tests record their own checked identities; this reference does not pin
a transient test database or report those values as deployed state.
