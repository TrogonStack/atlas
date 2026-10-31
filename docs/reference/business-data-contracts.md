# Business data contracts

Status: implemented native contract for addressable Schema declarations.
FieldSpec, type references, Swimlane state and identity annotations retain
their existing meaning. The [acceptance requirements](business-data-acceptance.md)
define the verification boundary; final composed verification and checklist
closure remain separate. No importer, migration, provider sync or federation
is included.

## Shipped representations

These behaviors exist in the current tree. The declarations below do not
reinterpret them.

| Shipped representation | Current meaning and boundary |
| --- | --- |
| [Schema](../../proto/trogonatlas/eventmodel/v1alpha1/fields.proto) | Addressable shape when `id` is set: `title`, `doc`, `fields`, `metadata`, `supersedes`. The same message packed into an Event, Command or ReadModel `schema` Any is a payload shape. Equal `type_url` and bytes are nominally the same payload type. |
| [FieldSpec](../../proto/trogonatlas/eventmodel/v1alpha1/fields.proto) | `name`, closed [FieldType](../../proto/trogonatlas/eventmodel/v1alpha1/fields.proto), `repeated`, `optional`, `doc` and typed `metadata`. Object fields nest inline. A repeated field describes one element type plus a repetition flag. |
| [TypeRef](../../proto/trogonatlas/eventmodel/v1alpha1/fields.proto) / `SchemaRef` | A field reuses an exact Event, Command, ReadModel, Processor, UI, Persona, Swimlane or named Schema. Reference walks emit `fields[i].ref`, `schema.fields[i].ref` and `state[i].ref`. The walk stops at the ref. |
| [Swimlane](../../proto/trogonatlas/eventmodel/v1alpha1/moments.proto) | Stream and consistency boundary: `stream_id`, transitions and `state` FieldSpecs. `state` is a source for event-field checks. It is not a business schema. |
| [Uuidv5IdentityAnnotation](../../proto/trogonatlas/eventmodel/v1alpha1/annotations.proto) | Named derivation registered on a Swimlane or other entity. Placeholders in `stream_id` may cite that name. |
| [DerivedFieldAnnotation](../../proto/trogonatlas/eventmodel/v1alpha1/annotations.proto), [DomainTagAnnotation](../../proto/trogonatlas/eventmodel/v1alpha1/annotations.proto), [PersonalDataAnnotation](../../proto/trogonatlas/eventmodel/v1alpha1/annotations.proto) | Field metadata for derivation, a shared domain tag or erasure. Metadata references are not indexed as graph edges. |
| `ReadModel.key` | Names that identify a projection row. Each name must exist on that read model's schema fields. |
| [ConnectionRef](connection-identity.md) | Exact owner kind and version, server-owned incarnation UID, and local `ConnectionId`. Diagnostic index paths are not selectors. |
| Contract revision | Contract revision and both mutation floors are 19. The client gate refuses older writers before mutation. |

`schema_fields` returns only `Schema.fields`. A packed Any whose type is a
TypeLibrary message yields descriptor fields and no business declarations.
`declared_schema_id` reads `Schema.id` from a packed payload Any so a closure
can follow that schema. That id is not a foreign business selection.

## Comparison inputs that this contract answers

Historical comparison only: the following source description is research
provenance, not an active integration requirement. EventCatalog-specific work
is deferred to the final mile under the [native-first scope](eventcatalog-coverage.md#native-first-scope).

Pinned EventCatalog collection schema at
[`2736a2272207cf1b1ee09e6a440401d2215c6380`](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts)
describes an entity with optional `aggregateRoot` (boolean), optional
`identifier` (one property name) and `properties`. A property has required
`name` and `type`, optional `required`, `description`, `enum`, nested
`properties`, and `items` (`type` plus nested `properties`). Foreign data on a
property is optional `references` (string), optional `referencesIdentifier`
(string), optional `referenceTarget` restricted to the literal `entity`, and
optional `relationType` (string).

Rules taken from that shape:

- Absent `aggregateRoot` is unspecified. It is neither true nor false.
- `relationType` is free label text. It is not cardinality, a closed verb, or
  a multiplicity enum.
- `referenceTarget` allows only an entity. This contract stores that target as
  an exact `SchemaRef`.
- Absent `referencesIdentifier` selects no property. It does not fall through
  to a primary identifier.
- Nested `properties` and `items` are shape. Type reuse remains a separate
  native mechanism.

Entity `services` and `domains` memberships, owners, badges, `schemaPath` and
message links belong to other checklist items. This contract does not define
them.

## Where the declarations live

Extend the addressable Schema entity only. Do not add an entity kind. Shipped
`ENTITY_KIND_SCHEMA` is 19. Shipped Schema fields 1 through 6 stay as they are.
Schema fields 7, 8 and 9 carry the native declarations.

| Schema field | Meaning |
| --- | --- |
| `optional bool aggregate_root = 7` | Author intent with presence. Unset, `true` and `false` are three states. |
| `identifiers = 8` | Repeated `BusinessIdentifier` occurrences. |
| `property_relationships = 9` | Repeated `BusinessPropertyRelationship` occurrences. |

Reject these three fields on a Schema message packed into Event, Command or
ReadModel `schema`, including when that Any also sets `Schema.id`. Payload
nominal identity stays `type_url` plus bytes, and those bytes must not carry
connection IDs. `declared_schema_id` is unchanged.

Swimlane `state`, event and command payloads, and read-model schemas do not
gain these collections. A FieldSpec keeps its shipped fields. Business
selections point at FieldSpecs with a path. They are not metadata Anys, because
metadata is not walked as a reference graph.

Anonymous inline objects stay inline. Promoting one to a named Schema is a
separate authoring act. A `FieldType.ref` to a Schema reuses that shape and
does not copy `aggregate_root`, identifiers or relationships onto the using
field or the using schema.

## Native messages

`fields.proto` may import `connection_id.proto`. It must not import
`decision_document.proto`: that file reaches `fields.proto` again through
Specification, reliability and slices. Implementation extracts `CatalogTitle`
and `CatalogText` into a leaf file in the same package, with the same message
names and field numbers, then imports that leaf from both files. Wire encoding
of existing titles and text stays the same. Moving messages between files still
triggers buf FILE breaking rules: follow the
[breaking-change playbook](../how-to/ship-a-breaking-proto-change.md), recording
actual findings in the maintained acknowledged-breaking-changes.txt file and
coordinating advertised revision floors with the combined change. Studio loads
vendored proto files separately and must be checked against the new import graph. Generated Rust and JavaScript
paths follow maintained generation. The shared leaf is `catalog_values.proto`.

```protobuf
message PropertySegment {
  string name = 1;
  bool repeated = 2;
}

message PropertyPath {
  repeated PropertySegment segments = 1;
}

enum BusinessIdentifierRole {
  BUSINESS_IDENTIFIER_ROLE_UNSPECIFIED = 0;
  BUSINESS_IDENTIFIER_ROLE_PRIMARY = 1;
  BUSINESS_IDENTIFIER_ROLE_ALTERNATE = 2;
}

message BusinessIdentifier {
  trogonatlas.type.v1alpha1.ConnectionId connection_id = 1;
  PropertyPath property = 2;
  BusinessIdentifierRole role = 3;
  CatalogText doc = 4;
}

message BusinessPropertyRelationship {
  trogonatlas.type.v1alpha1.ConnectionId connection_id = 1;
  PropertyPath source = 2;
  SchemaRef target_entity = 3;
  PropertyPath target_property = 4;
  CatalogTitle label = 5;
  CatalogText doc = 6;
}
```

`target_property`, `label` and `doc` use message presence. An omitted message
is absent. A present empty path, blank title or blank text rejects.

`CatalogTitle` remains nonblank UTF-8 of at most 256 encoded bytes.
`CatalogText` remains nonblank UTF-8 of at most 16 KiB encoded. `SchemaRef`
and `ConnectionId` keep their shipped rules: exact namespace, slug and positive
version, and a non-nil lowercase hyphenated UUID.

## Property paths

A `PropertyPath` is an ordered list of segments, root first, resolved only
against the structural FieldSpecs of one Schema: top-level `fields`, then
inline `ObjectType.fields`. Each segment matches exactly one sibling by
case-sensitive `name`. The segment's `repeated` flag must equal that field's
`repeated` flag.

| Path rule | Admission result |
| --- | --- |
| Empty segment list, empty name, or more than 32 segments | Reject. |
| Segment name longer than 256 UTF-8 bytes | Reject the selection. The FieldSpec may still exist. Shipped field names have no separate length cap. |
| Two siblings with the same name | The path is ambiguous. Reject the selection. Other fields on that schema stay valid. |
| Non-final segment whose field is not an inline object | Reject. A `RefType`, scalar, enum, bytes or timestamp is a leaf. |
| Final segment | Any shipped FieldType, including a ref, enum or repeated object. |
| Segment spelled `items` | Resolves only when a FieldSpec is actually named `items`. Array element shape is the repeated field's type, not an extra hop. |
| Walk through `FieldType.ref` | Forbidden. Identifiers inside the reused schema are declared on that schema. |

Reorder of uniquely named fields keeps paths valid. Diagnostic index strings
such as `fields[0].type.object.fields[1]` stay diagnostics. They are not stored
as the path.

A path selects a field definition. Repetition does not create a selector per
runtime element.

## Aggregate-root intent

| Stored presence | Meaning |
| --- | --- |
| Field unset | Unspecified. Clients, Studio and exports preserve absence. |
| `true` | The author declares aggregate-root intent for this business schema. |
| `false` | The author declares that this schema is not an aggregate root. |

Neither value creates, requires, or clears a Swimlane, `stream_id`, transition,
event, command, slice or stream name. A Swimlane that mentions the same nouns
remains a separate entity. Matching a field name to a `stream_id` placeholder
or to `Swimlane.state` does not set this flag. Copying the flag onto payload
Anys is rejected. Users of a Schema through `TypeRef` do not inherit the flag.

Changing the flag is an ordinary field edit of the Schema. It mints no
connection ID. A later Schema version is a new owner incarnation. The old
flag does not follow readers that still hold the old version.

## Business identifier roles

A `BusinessIdentifier` declares that one resolved property path has a role on
this Schema. The role is explicit. Field type, field name, `DerivedFieldAnnotation`,
`Uuidv5IdentityAnnotation`, `DomainTagAnnotation`, `PersonalDataAnnotation.subject_field`,
`ReadModel.key` and Swimlane placeholders do not assign a role.

| Role | Rule |
| --- | --- |
| `PRIMARY` | At most one per Schema incarnation. This is the single business identifier when an author names one. |
| `ALTERNATE` | Further identifying properties on other paths. Zero or more. |
| `UNSPECIFIED` and unknown numbers | Reject on native write. |

The same path appears in at most one identifier occurrence. A property may be
an object, a scalar, an enum or repeated. No FieldType is required and none is
forbidden. The declaration does not assert storage uniqueness, a database key,
a row key or a stream id.

`doc` is optional elaboration. It is outside the identity fingerprint.

An empty `identifiers` list means no role is declared. It does not prove that
the business has no identifier.

`ALTERNATE` has no EventCatalog field. The pinned source has one `identifier`
string. `PRIMARY` is the role that string maps onto when an author states it.
`ALTERNATE` is optional Atlas vocabulary for additional properties. Omitting it
is valid.

## Foreign entity and property selections

A `BusinessPropertyRelationship` is one owned occurrence:

| Field | Rule |
| --- | --- |
| `source` | Required path on the owning Schema. The property that carries the reference, including a nested or repeated field. |
| `target_entity` | Required `SchemaRef`. No EntityRef, EventRef, CommandRef, SwimlaneRef, TypeLibrary name or bare string. |
| `target_property` | Optional path resolved on the target Schema's own fields, under the same path rules. Absence selects the entity only. |
| `label` | Optional `CatalogTitle`. Free business wording, stored and returned unchanged. |
| `doc` | Optional `CatalogText`. |

The label is not parsed. Text such as `hasMany`, `belongsTo` or `one-to-many`
stays text. The server does not derive cardinality, navigability or a reverse
edge from it. Absence of a label means no label was authored.

Absence of `target_property` does not select the target's `PRIMARY` identifier,
even when that role exists. A present target path need not be an identifier
role on the target.

A new or fingerprint-changing occurrence admits only when the source path
resolves on the prepared owner and the target key is visible in the guarded
final batch or branch view. A present target path must resolve on that target
version. Preview output and a guessed incarnation UID are not selectors.
`SchemaRef` uses the client-supplied exact key, so a target created in the
same admitted batch is eligible when the final view contains that key.

Self-target, where `target_entity` is this Schema's own key and version, is
allowed. Resolve both paths against the prepared owner. Cycles between schemas
are allowed. No edge is synthesized in the reverse direction.

Equal source, target, target path and label may repeat as distinct occurrences.
Order is retained and has no causal meaning. An empty relationship list means
this Schema declares no foreign selections.

A field may both reuse a type through `FieldType.ref` and carry a relationship
whose source path is that field. The ref and the relationship stay independent.
A guarded entity edit may change either independently. Generic
RetargetReferences rewrites every matching from-key site, so when both select
the same Schema key it changes both. The relationship may name a different
Schema than the type ref; neither reference is inferred from the other.

## Identity, retarget, delete and branches

Reuse [connection identity](connection-identity.md) and the persistence helper.
Identifier IDs and relationship IDs share one uniqueness domain on the Schema
owner. Supplied IDs on an existing owner must already be active. Omission asks
for a UUIDv7. A present empty or malformed ID rejects before generic connection
normalization assigns one. The shared helper that treats a present empty value
as omitted is not sufficient by itself. Schema has no shipped connection IDs;
these collections are the first.

The durable selector is `ConnectionRef`: Schema kind, exact key and version,
`owner_uid`, and `ConnectionId`. A path string, label, list index or target
pair is not a substitute.

| Fingerprint | Included | Excluded |
| --- | --- | --- |
| Identifier | Role plus every segment name and repeated flag | `doc`, list index |
| Relationship | Source path, full `SchemaRef`, and target-path presence plus its segments | `label`, `doc`, list index |

| Operation | Required outcome |
| --- | --- |
| Create | Fresh owner UID. Omitted IDs are assigned. Valid supplied IDs only on a new owner. Duplicates across the two collections reject. |
| Wording or reorder | Label, doc and order changes keep IDs, including equal siblings. |
| Role, path or target change | A retained ID rejects. Omit the ID so assignment mints a new occurrence. |
| Remove then add | The old `ConnectionRef` stops resolving. The new occurrence receives a new ID even when the body matches. |
| Exact-content id-less reapply | Same-store no-op keeps stored UID, IDs and etag. |
| Preview | Shown IDs are not reserved and are rejected if submitted as active IDs on an existing owner. |
| New version, clone, recreate | New incarnation UID. Old `ConnectionRef`s do not follow. Local IDs may exist again only inside the new UID scope. |
| `RetargetReferences` on a Schema key | Relationship occurrences whose `target_entity` matches renew their IDs. Identifier paths are not entity refs and are not rewritten. `FieldType.ref` sites to that same from-key also follow their shipped retarget rules; the API has no per-site selector. |
| Field rename or repeated-flag change | The caller must explicitly remove or replace occurrences whose paths no longer resolve. Otherwise the entire edit rejects; the server never silently drops them. Replacement uses a new ID. |
| Target property removed, source unchanged | The stored occurrence remains readable. `GetEntity` still returns the path. The next edit of the source must repair it with a new ID or remove it. |
| Safe delete of a target Schema | Incoming relationship refs participate in the complete guard, including hidden referrers. A self-reference on the owner being deleted does not block deletion in either single or batch paths. A reporting cap does not authorize deletion. |
| Force delete | Stored `SchemaRef`s remain, unresolved, with no cascade into identifiers, Swimlanes or payload schemas. |
| Same-key recreate of a target | `SchemaRef` may resolve to the replacement. Report that owner's actual UID and etag. Relationship `ConnectionRef`s still pin the source UID and occurrence ID, not the target UID. |
| Branch edit, land, rebase | Same-incarnation active IDs are preserved against the recorded base and both sides. Ordinary writes reject deleted-ID revival. Explicit conflict restoration renews only the restored IDs. Edited declaring owners revalidate paths against the final overlay on land/rebase/resolution; unedited source owners remain inspectable if a target-only change breaks a path. |
| Missing or foreign branch | No silent fallback to baseline or another branch. |

Unchanged dangling targets stay inspectable and do not block unrelated writes
to other entities. An edit of the declaring Schema revalidates every retained
occurrence in the prepared entity. Business admission includes only new
property_relationships target selections; it must not make legacy fields[].ref
or supersedes globally resolution-required. Extend graph catalog_provenance for
property_relationships so returned Reference rows carry their occurrence IDs.

## API

No new RPC and no new entity kind. Authoring and discovery use the shipped
service:

| RPC | Proposed use |
| --- | --- |
| `PutEntity`, batch and `validate_only` | Admit or reject a Schema under the rules above. Preview does not publish. |
| `GetEntity` | Return presence of `aggregate_root` and the stored occurrences, including a target path that no longer resolves. |
| `GetOutgoingReferences` / `GetIncomingReferences` | One hit per relationship to `target_entity`, with `from_uid`, `connection_id` and diagnostic `from_field` `property_relationships[i].target_entity`. Identifiers add no entity hit. `FieldType.ref` hits stay on their shipped field paths and are a different edge. |
| `GetImpact` | Traverse the SchemaRef edge with occurrence provenance. Stop there. Do not continue through type refs, identifier roles or Swimlanes as if they were this edge. |
| `RetargetReferences` | Renew affected relationship IDs only, as in the table above. |
| `DeleteEntity` | Safe and force policies as above. |
| Branch RPCs | Incarnation and ID lifecycle as in the connection contract. |

`Reference` has shipped fields through `connection_id = 9`. The next field at
this reading is 10. Recheck it before generation. Propose an optional status
message set only on relationship hits:

| Status | When it is set |
| --- | --- |
| Entity only | The occurrence stores no `target_property`. |
| Property resolved | The stored path matches the target version in this checked read. |
| Property unresolved | The target Schema is readable and the path does not resolve. |
| Unavailable | The target is missing or hidden. Those two cases share one status. |

Unset status means the server did not evaluate a property. Clients must not
treat unset as resolved. A response field may be ignored by older clients.
Writers that would drop `aggregate_root`, `identifiers` or
`property_relationships` are a breaking mutation. Implementation raises
`CONTRACT_REVISION` and the client mutation floor at that time, per
[wire compatibility](../explanation/wire-compatibility.md). This document does
not change those constants.

Extend the closed CatalogSubject and responsibility connection inventories so a
`ConnectionRef` can name one identifier or one relationship occurrence. Resolve
the ID against that Schema's full inventory. An identifier subject has no
foreign endpoint. A relationship subject's endpoint context is `target_entity`;
the property path is checked separately. Assignment to one occurrence does not
spread to an equal sibling. Raw FieldSpecs and type refs are not occurrence
subjects.

Native manifests must preserve bool presence, path segments, uint64 versions
and full connection identity. An omitted SchemaRef namespace uses the manifest
namespace, and an omitted version is `1`, which is the shipped shorthand. That
shorthand cannot rebuild a `ConnectionRef` or an owner UID.

Studio remains a review surface. It shows unspecified aggregate intent as
unspecified, and it shows an unresolved property path as unresolved. A loaded
graph is not this backend check.

## Authorization, bounds and Source isolation

Writes require Schema write authority. A new or fingerprint-changing
relationship also requires reference access to the target Schema. Identifier
and `aggregate_root` edits do not require a second owner. No declaration grants
a principal, a Swimlane write, or write access to the target. A Reader may read
and may not mutate.

Unreadable declaring Schema uses the shipped generic `NOT_FOUND` before any
inventory is exposed. A hidden or missing target uses one unavailable status
and does not reveal titles, UIDs, etags, path matches or existence. Authorization
service failures stay operational errors. They are not an empty graph and not
permission to read the baseline.

| Bound | Rejection |
| --- | --- |
| Identifiers on one Schema | More than 1,024 rejects. |
| Relationships on one Schema | More than 1,024 rejects. |
| Prepared Schema protobuf | More than 1 MiB rejects, counting assigned IDs and existing fields. |
| Path | More than 32 segments, or a segment name over 256 UTF-8 bytes, rejects. |
| Shared discovery | Existing 5,000 resource-row, 10,000 examined-occurrence and 4,096 emitted-record ceilings stay shared. |
| Overflow | `RESOURCE_EXHAUSTED`. No truncation, no partial impact presented as complete, no capped report that authorizes delete. |

Charge the declaring Schema's identifier and relationship inventory once per
exact owner and view when resolving a `ConnectionRef`. Charge each distinct target Schema as one row. Each checked path segment
consumes shared work; each emitted reference consumes output budget. Existing
impact paths carry generic occurrence provenance, not an implied property-status
receipt unless explicitly evaluated. Resolving a target path does not recursively
walk type references. Lower shipped limits still win.

Source stays native-isolated. Do not reuse `SOURCE_OWNED_FIELD_SCHEMA`: that
shipped field is payload-schema ownership, not authority over these
declarations. The closed Source field inventory does not name the new fields.
Until a later contract adds them, every Source path rejects:

- Creates that carry a set `aggregate_root`, a non-empty identifier list or a
  non-empty relationship list, under every ownership policy, including title-only,
  because create copies incoming content.
- Whole-owner writes when the current or incoming Schema has any of those
  declarations, including an attempt to clear them.
- Field ownership that selects the new fields.
- Occurrence selection of identifier or relationship IDs.

Supported unrelated field updates preserve the three declarations exactly.
A Source-owned fields update that would invalidate an existing identifier or
relationship path rejects atomically with no publication. Source never drops,
repairs or replaces native business declarations to make its update pass.
Rejection publishes neither the native Schema nor a mapping base. Remote
connectors and portable rebinding of incarnation-bound selectors stay deferred.

## Verification and separate work

The shared value leaf, revision floors, reference walk, fingerprints, subject
inventory and Source guards are implemented. Native backend, CLI, MCP and
Studio codec fixtures exercise these declarations. The acceptance requirements
remain authoritative for final composed verification; isolated passing runs
do not close the coverage checklist.

Still outside this contract: entity-to-service and entity-to-domain membership,
channel and storage claims, flow topology, agent metadata, executable slices,
and any reading of EventCatalog files into Atlas.

## Open objections

| Objection | Disposition in this proposal |
| --- | --- |
| A new business-entity kind would separate catalog entities from payload schemas. | Rejected for this contract. Shipped Schema is already the addressable shape. A second kind would fork type reuse. Sibling proposals that allocate a new `Entity` tag do not reserve Schema fields 7 through 9. |
| Stable FieldSpec IDs would let a rename keep relationship identity. | Rejected. FieldSpec has no occurrence ID. The path is the property selection, so a path change renews the connection ID. |
| `ALTERNATE` is not in the pinned EventCatalog schema. | Kept as an optional role. Parity does not require it. A second `PRIMARY` still rejects. |
| Target paths are richer than a single `referencesIdentifier` string. | Kept. One segment spells a top-level name. Nested and repeated target properties stay selectable. The path never inserts an `items` hop. |
| Segment names are capped at 256 bytes while shipped FieldSpec names are not. | A longer name cannot be selected. The field remains valid. Widening the cap is a later edit. |
| `Reference` field 10 may be taken by another in-flight proposal. | Recheck before generation. Status must stay optional so its absence is not success. |
| Extracting `CatalogTitle` may churn generated paths. | Required dependency because of the import cycle. Same package, message names and field numbers. No generation in this task. |
| Readers may treat a stored target path as current after the target changes. | `GetEntity` returns stored data. Reference status, when present, is the checked read. The source etag does not cover the target. |
