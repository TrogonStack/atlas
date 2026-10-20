# Specification and API acceptance fixture

Status: required acceptance criteria for the
[Specification/API contract](specification-and-api-contracts.md). The resource
schema, service implementation, CLI/MCP support and Studio surfaces are pending.
This fixture describes required outcomes; it is not executable test evidence.

## Connected fixture

Allocate distinct fresh namespaces for each mutation case and retain their
returned IDs as `primary_namespace` and `foreign_namespace`. Unqualified model
labels below belong to `primary_namespace`. Every model reference includes an
exact version. Names below are fixture labels; retained member and association
identities come from the authoritative write response.

| Resource | Required declaration |
| --- | --- |
| `order-request@1` | Integration ReadModel containing a submitted order. |
| `order-receipt@1` | ReadModel containing the receipt exposed by the API response. |
| `order-adapter@1` | Processor observing `order-request@1` and issuing the local `place-order@1` Command through an AutomationSlice. |
| `order-intake@1` | The AutomationSlice containing that exact ReadModel observation and paired Processor; retain its occurrence ID for the Channel selection. |
| `place-order@1`, `order-placed@1` | Local Command and Event connected through the ordinary CommandSlice grammar. |
| `receipt-projection@1` | ReadModelSlice projecting `order-placed@1` into `order-receipt@1`. |
| `order-api@1` | Specification retaining a contract artifact and declaring schema, message and operation members. |
| Request and response messages | Distinct members mapped to `order-request@1` and `order-receipt@1`, respectively. Both may select the same schema member; schema reuse does not merge their roles or ReadModels. |
| Submit operation | Select the request member as REQUEST and the receipt member as RESPONSE, with separate occurrence IDs. Its binding describes the API transport. |
| Another operation | Reuse the receipt message in another RESPONSE occurrence with its own association identity. |
| `public-gateway@1`, `partner-gateway@1` | Components with separate explicit exposures of the submit operation. A provider list alone is insufficient. |
| `checkout@1` | Caller Component with a usage selecting the submit operation and only the public-gateway exposure. |
| Unspecified-provider usage | Another usage of the same operation without selecting an exposure. Its provider remains unspecified. |
| `order-types@1` | Existing TypeLibrary available for a separate schema binding selecting a fully qualified protobuf message name. This does not convert the retained artifact into a TypeLibrary. |
| `native-order-shape@1` | Named native Schema selected by another schema binding, independently of artifact-backed schemas. |
| `shared-contract@1` | Another Specification with a retained artifact and artifact-backed schema. An order-api message selects it through ContractTypeRef, including exact owner UID, schema/artifact member IDs, digest and expected selector. |
| `order-transport@1` | Channel selecting the real order-adapter AutomationSlice observation. Another operation uses a CHANNEL binding selecting this Channel without inventing another observation. |
| `foreign_namespace/foreign-order-contract@1` | Specification with a message mapping to `primary_namespace/order-receipt@1`; a restricted reader cannot inspect that Specification, its member names or artifact content. |

Keep references to both provider exposures, every message and schema member, and
their association IDs for later lifecycle cases. Include an authorized anchor
outside the capped model snapshot so exact-resolution behavior is exercised.

## Required queries

| Query | Required result |
| --- | --- |
| Fetch `order-api@1` through service, CLI and MCP | Preserve retained bytes, digests, value presence, member IDs and association IDs. |
| Classify `order-api@1` | Report API classification because it declares operations and provider exposures selecting local operations. A provider need not expose every operation. |
| Select a schema or operation member | Resolve the exact Specification incarnation, expected member kind and member identity. No matching by title, list position or schema shape. |
| Inspect the submit operation | Return separate REQUEST and RESPONSE message occurrences with independently selected schema and ReadModel mappings. |
| Follow the submit response | Reach `order-receipt@1`; the request's `order-request@1` is not response data. |
| Inspect checkout's provider | Resolve only the public-gateway exposure. Do not produce a caller-to-partner-gateway dependency from the shared operation. |
| Inspect the unspecified-provider usage | Retain a caller-to-operation dependency with no invented provider. |
| Resolve both response occurrences | Reusing the same message does not collapse the occurrences or their IDs. |
| Get incoming references and impact for a ReadModel, Channel, Schema, TypeLibrary, provider or referenced Specification | Discover references nested in messages, schema selections, transport bindings, exposures, usages and dependencies. Return only authorized details. |
| Select an association as a connection subject | Resolve its owning Specification, incarnation and ConnectionId. Member identity alone is not a ConnectionRef. |
| Traverse behavior from `order-request@1` | Reach the declared AutomationSlice, local Command/Event and receipt projection through the existing graph. Specification declarations introduce no extra behavioral edges. |
| Fetch an anchor outside the projection cap | Resolve it by exact authorized key in the selected branch. Omission from a projection is not absence from storage. |
| Export and reapply an unchanged native manifest | Preserve identities and exact bytes; do not parse or rewrite an opaque artifact during the round trip. |

## Admission and identity cases

Rejected writes leave persisted content and identities unchanged. Run each case
through standalone and batch mutation surfaces where applicable, including
preview. Invalid or forbidden target diagnostics must not expose hidden data.

| Case | Required outcome |
| --- | --- |
| Create a schema/message Specification with no operations or exposures | Accept it as a standalone Specification, without API classification. |
| Create an operation-bearing Specification with no provider exposures | Accept it as provider-undeclared, without API classification or an invented provider. |
| Create a provider exposure without its selected local operation | Reject rather than classifying a dangling declaration as an API. |
| Remove an API's last exposure after repairing local usage selections | Preserve a valid provider-undeclared Specification and remove its derived API classification. |
| Create a Specification with omitted native member/association IDs | Assign and return the IDs under a fresh server-owned incarnation. A persisted selectable occurrence never has an absent ID. |
| Reorder members, schema bindings, operation messages, exposures or usages | Preserve the retained identities. |
| Rename display labels or edit documentation | Preserve the member or association identity when its semantic selection is unchanged. |
| Repeat a member ID or an association ID within its owner | Reject. Association IDs are unique across the whole Specification, not just one member list. |
| Select a member using the wrong member kind | Reject even if a member with that display name exists. |
| Change an association role, selected message, schema target, caller, operation or provider exposure while retaining its ID | Reject; the replacement needs a fresh occurrence identity. |
| Remove and later re-add a member | Previously retained member selections remain unresolved; a new member cannot silently inherit the removed identity. No unbounded retired-ID list is persisted. |
| Remove and later re-add an association with the same endpoints | Assign a new ConnectionId under the ordinary connection-identity contract. |
| Delete and recreate `order-api@1` | A new owner incarnation prevents old member and connection selections from resolving into the replacement. |
| Copy the Specification to a new key or version | The new owner has a distinct incarnation. Rewrite intra-owner selections deliberately; external references remain exact and do not follow unrelated copies. |
| Change retained bytes without updating the digest | Reject. A pin to former content must not resolve to changed bytes. |
| Change retained bytes with a valid new digest while retaining the artifact member ID | Reject. Add a new artifact member and explicitly retarget schema declarations instead. Old content pins must not resolve to the replacement. |
| Reference an unresolved local member or an undeclared provider exposure | Reject the new or edited declaration. Do not infer the selection from matching transport text. |
| Select an exposure belonging to another operation | Reject the usage. The operation and exposure must agree. |
| Omit the provider selection on a usage | Accept the explicit caller-to-operation dependency without inventing a provider. |
| Use the same schema for request and response | Accept independent roles and ReadModel mappings; schema equality is not evidence of response identity. |
| Supply a Command or Event as an integration message mapping | Reject; confirmed integration mappings select ReadModels. Local behavior is reached through its existing slices. |
| Record an unsupported or unknown contract format | Retain its declared format and opaque bytes under the first-release support boundary. Do not claim semantic validation or compatibility success. |
| Select an opaque artifact type | Report its selector as unverified, even when its declared syntax resembles a supported native type system. |
| Select a ContractTypeRef with the wrong digest, artifact member ID, selector value or selector presence | Reject without falling back to another member or a matching display name. |
| Retarget a schema definition to another artifact while a foreign ContractTypeRef pins its former definition | Assign a new definition ConnectionId. Preserve the unchanged foreign declaration as diagnostically unresolved; newly writing that stale type pin fails admission. |
| Substitute a URL for retained artifact content | Reject the remote-only declaration. No server fetch may turn a mutable URL into retained content or an immutable pin. |
| Retain UTF-8 text with different newline bytes | Preserve the exact text bytes and distinct digest; do not normalize them during authoring or export. |
| Submit non-UTF-8 binary artifact content | Reject under the first-release source-text boundary; a format label does not enable a binary codec. |
| Exceed an artifact, total-content, member, association or encoded-entity limit | Reject atomically under the contract's concrete ceilings and any lower transport/store limit. Do not truncate the contract or discard references. |
| Select a non-existent protobuf message from a TypeLibrary | Reject the new binding. Retained opaque artifact selectors do not gain protobuf compiler support by sharing a format label. |

## Authorization and lifecycle cases

| Case | Required outcome |
| --- | --- |
| Create or retarget a declaration across namespaces | Require write access to the Specification and authorized reference access to every selected external entity. |
| Query nested references as a restricted reader | Do not expose forbidden namespaces, member content, provider details or connection subjects through diagnostics or impact output. |
| Safely delete an entity with only a hidden Specification referrer | Refuse deletion, individually and in guarded batch preview/apply, without returning hidden referrer details. |
| Safely delete a selected Specification | Discover incoming member and connection selections as incoming references to their owner and refuse deletion. |
| Explicitly force-delete an external target | Preserve referring declarations as unresolved. Do not erase, retarget or fabricate replacement members. |
| Force-delete or revoke reader access to a declared API provider | Preserve the exposure declaration and API classification while reporting only authorized target diagnostics. Classification does not depend on viewer access or current external-target resolution. |
| Make an otherwise permitted target edit that invalidates a retained member selection | Preserve unchanged referring declarations for inspection and report the unresolved selection. A newly authored or edited invalid declaration still fails admission. |
| Add descriptive dependencies forming a Specification cycle | Apply the declared relation-specific cycle policy; traversal remains bounded and preserves the cycle. TypeLibrary import and supersedes rules do not change. |
| Preview and land a branch with unchanged declarations invalidated by a permitted target edit | Use the same admission policy as ordinary writes; do not globally reject all existing unresolved resources. |
| Edit or create a selected declaration on a branch | Resolve exact targets and local members in the post-write merged branch view. A branch tombstone must not fall back to baseline content. |
| Independently create the same Specification key on branch and baseline | Require explicit conflict resolution for distinct owner incarnations. |
| Resolve a member/association conflict or land after a concurrent removal | Preserve the chosen side's valid identities under the documented merge policy. A stale selection must not resurrect a retired identity automatically. |
| Exceed the visible resource reporting budget during guarded deletion | Reporting may be incomplete under its explicit policy; authorization must still use the complete incoming-reference guard. |
| Encounter a complete-guard scan error or timeout | Refuse deletion. An incomplete scan is not proof that a target is unreferenced. |

## Implementation evidence required

The implementation PR must replace this pending status with links to executable
public-service regressions, real store reference-guard tests, native manifest
round trips, CLI/MCP coverage and Studio review coverage. Assert absence of
invented provider cross-products and behavioral edges, not just the presence of
the intended references.

Documentation checks establish link and formatting integrity only. They do not
establish a generated wire contract, backend enforcement, format compatibility,
runtime behavior or deployed-browser readiness.
