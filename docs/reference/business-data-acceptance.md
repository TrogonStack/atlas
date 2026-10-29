# Business data acceptance requirements

Status: acceptance requirements for the implemented
[business data contract](business-data-contracts.md). Native backend, CLI, MCP
and Studio codec fixtures exist. Final composed execution and requirement
review remain necessary before checklist closure. Logical names below are not
manifest syntax, predicted owner UIDs or submitted occurrence IDs.

## Fixture sketch

Prepare these native records before the cases that need them. Retain every
returned Schema UID, etag, connection ID and branch from the authoritative
read. Use explicit versions, including values above JavaScript's exact integer
range.

| Record | Shipped shape used as the control |
| --- | --- |
| Schema `order` | Top-level fields: inline object `customer` with child `id`; repeated object `lines` with child `sku`; field `product_ref` whose `FieldType.ref` is Schema `product` at an exact version. |
| Schema `product` | Field `sku`. No business declarations until a case adds them. |
| Schema `customer-record` | Field `id`. Distinct from the inline `customer` object on `order`. |
| Swimlane `orders` | `stream_id` containing `{order_id}`, `state` field named `order_id`, and a `Uuidv5IdentityAnnotation` named `order_id`. |
| Event `OrderPlaced` | Payload Schema Any with field `order_id`. `declared_schema_id` may cite Schema `order` only in the case that says so. |
| ReadModel `order-row` | `key` containing `order_id`. |

A second Schema `product` version, a second relationship with the same target,
and a field literally named `items` are added only by the cases that require
them. Establish native persistence and exact reference discovery before any
Studio check. Component fixtures and mocked gateways do not replace Store,
authenticated transport, CLI or MCP receipts.

## Aggregate-root presence

| Case | Required public outcome |
| --- | --- |
| Unset on Schema `order` | Get, export and reapply preserve absence. The value is unspecified. |
| Set `true` | Stored and returned as `true`. No Swimlane, `stream_id`, transition, event or slice is created. |
| Set `false` | Stored and returned as `false`, distinct from unset. The Swimlane `orders` is unchanged. |
| Name match | A field, state field or annotation named `order_id` does not set the flag. |
| Type reuse | `product_ref` pointing at Schema `product` does not copy `product`'s flag onto `order` or onto the field. |
| Payload Any | An Event, Command or ReadModel `schema` Any that sets `aggregate_root` rejects with no publication, whether or not the Any sets `Schema.id`. |
| Packed id only | A payload Any that sets `Schema.id` and omits the three business fields still admits. `declared_schema_id` follows that id. The Schema entity's declarations are not copied into the Any. |
| New version | Schema `order` version 2 has a new UID. Old reads of version 1 keep version 1's flag. |

## Identifier roles

| Case | Required public outcome |
| --- | --- |
| Primary | One `PRIMARY` on path `customer` / `id` (both segments `repeated` false) admits. A second `PRIMARY` on `lines` / `sku` rejects atomically. |
| Alternate | `ALTERNATE` on a different path admits beside the single primary. Omitting every alternate admits. |
| Unspecified role | Role 0 and an unknown enum number reject. |
| Same path twice | Two occurrences whose paths match reject, including primary plus alternate on that path. |
| Repeated segment | Path `lines` (`repeated` true) then `sku` (`repeated` false) resolves. A segment `lines` with `repeated` false rejects when the field is repeated. |
| No `items` hop | Path `lines` / `items` / `sku` rejects unless a real field is named `items`. |
| Ref leaf | A path that continues through `product_ref` into `product.sku` rejects. Declaring `PRIMARY` on Schema `product` path `sku` admits on `product` only. |
| Ambiguous siblings | Two top-level fields named `sku` make a `sku` path reject. The schema's other fields stay stored. |
| Type and name traps | A UUID field, a derived-field annotation, a domain tag, a personal-data `subject_field`, `ReadModel.key` `order_id`, Swimlane state `order_id`, and the UUIDv5 annotation name do not create an identifier occurrence. |
| Empty list | A Schema with no identifier occurrences reads back as no declared role. |
| Doc edit | Changing `doc` keeps the connection ID. |
| Role or path edit | Keeping the ID while changing role or any segment rejects. An omitted ID assigns a new one. The old `ConnectionRef` no longer resolves. |

## Relationships and labels

| Case | Required public outcome |
| --- | --- |
| Entity only | Source `product_ref`, target Schema `product` at its exact version, no `target_property` and no label, admits. The target's primary identifier is not filled in. |
| Property selection | The same source plus target path `sku` resolves against Schema `product` only. It need not be an identifier role. |
| Free label | Label `hasMany` is stored and returned unchanged. No cardinality field is set. A second occurrence labeled `belongsTo` to the same target receives a different ID. |
| Blank label or doc | Present blank title or text rejects. Omission admits. |
| Label edit | Changing only the label or doc keeps the ID and the subject binding. |
| Nested source | Source path `customer` / `id` admits and survives a reorder of `order.fields`. |
| Wrong target kind | An Event, Command, Swimlane, TypeLibrary or bare string in place of `SchemaRef` rejects. |
| Independent ref | A guarded explicit entity edit can change either site independently. Generic RetargetReferences changes both when their exact from-key matches; identifier paths stay unchanged and relationship IDs renew. |
| Self-target | Schema `product` may select itself. Both paths resolve against the prepared entity. |
| Same-batch target | A Schema key created in the admitted batch is selectable in that final view. A preview-only ID is not. |
| Dangling unchanged | Deleting the target property on `product` leaves an unedited `order` readable, with the stored path still present. |
| Repair | The next edit of `order` that keeps the broken path and the old ID rejects. Removing the occurrence, or omitting the ID and writing a resolving path, admits. |
| Removed selector | After removal, that `ConnectionRef` does not resolve to a sibling with the same target. |

## Identity, branch, retarget and delete

| Case | Required public outcome |
| --- | --- |
| Assignment | Omitted IDs on create are server UUIDv7s. Explicit empty or malformed IDs reject before assignment, with no owner revision published. |
| Shared domain | The same connection ID on an identifier and a relationship rejects. |
| Reorder | Reordering either collection preserves IDs. |
| Reapply | Same-store export of unchanged content, applied without IDs, keeps UID, IDs and etag. |
| Preview | `validate_only` shows candidates and does not reserve them. Submitting a preview ID on the existing owner rejects. |
| Concurrency | A stale `if_match` publishes nothing. |
| Branch | Edited source owners revalidate paths against the final target overlay on land/rebase/resolution. Unedited sources stay inspectable after target-only shape changes. A same-incarnation branch edit keeps active IDs. A missing branch does not read baseline. Landing preserves those IDs. Ordinary revival of a deleted ID rejects. Explicit conflict restoration renews only restored IDs. |
| Owner recreate | A new Schema `order` at the same key has a new UID. Old connection refs stay unresolved even if local IDs were reused. |
| Target recreate | Force-deleting and recreating Schema `product` at the same key may satisfy `SchemaRef`. Report the new target UID and etag. Source connection refs still name the source UID. |
| Retarget | `RetargetReferences` from Schema `product` version 1 to version 2 renews relationship IDs on `order` that targeted version 1. Identifier occurrences on `order` stay. A `product_ref` type ref selecting the same version1 key also follows the shipped global retarget. No per-site selection is inferred. |
| Self-target deletion | A Schema with only a self business ref is deletable through single and batch safe delete; external incoming refs still block. |
| Safe delete | An incoming relationship blocks safe delete of `product`, including when the referrer is hidden from the caller. An incomplete guard refuses deletion. |
| Force delete | `product` is deleted. `order` still stores the `SchemaRef`. No Swimlane or payload schema is edited. |
| Subjects | A responsibility or Decision subject can name one full connection ref. The equal-target sibling is a different subject. A FieldSpec index and a type ref are not accepted as that subject. |

## Authorization and checked reads

| Case | Required public outcome |
| --- | --- |
| Writer | An authorized Schema writer admits identifier and aggregate-root edits with no write on any other owner. |
| Relationship write | The writer also needs reference access to the target. The target owner is not writable through this declaration. |
| Reader | A Reader can Get the Schema and receives `PERMISSION_DENIED` on mutation. |
| Hidden owner | An unreadable Schema is generic `NOT_FOUND` with no inventory, title, UID or count. |
| Hidden target | Outgoing status is unavailable for both a missing and a hidden target. Responses do not differ in a way that leaks existence, titles, UIDs or path matches. |
| No privilege grant | `aggregate_root` true, a primary identifier and a label grant no principal, Swimlane permission or target write. |
| Operational failure | Store, decode, timeout and authorization-service failures remain RPC errors, not an empty declaration list and not a resolved path. |
| Same-owner coherence | One Get of Schema `order` is one checked version. Relationship status for a target comes from that target's own checked version, and the source etag does not cover it. |
| Receipt freshness | A retained response does not authorize a later write. |

Reference hits for a relationship include owner UID, connection ID, exact target
key and `from_field` `property_relationships[i].target_entity`. Two relationships
to Schema `product` produce two hits. Identifier occurrences produce none.
`product_ref` still produces its shipped type-ref hit on the field path.
Impact stops at the target Schema. It does not continue into Swimlane `orders`
or into `ReadModel.key`.

When the optional reference status is implemented, entity-only, property
resolved, property unresolved and unavailable match the contract table. An
absent status is not reported as resolved.

## Bounds and Source isolation

| Case | Required public outcome |
| --- | --- |
| Collection caps | The 1,024th identifier admits and the next rejects. The same boundary holds for relationships. Rejection does not truncate. |
| Entity size | A prepared Schema over 1 MiB rejects, including assigned IDs. |
| Path caps | 32 segments is the maximum that admits. A longer path rejects. A segment name of 256 UTF-8 bytes admits. One more byte rejects that selection and leaves the FieldSpec in place. |
| Shared budgets | Examinations that cross 5,000 rows, 10,000 occurrences or 4,096 emitted records return `RESOURCE_EXHAUSTED`. Repeated lookups of one owner in the same request do not recharge its inventory. Pages do not reset the request budget. |
| Target charge | Checking one target path charges that Schema row and the path segments. It does not walk `FieldType.ref` closures as relationship work. |
| Source create | Under every ownership policy, including title-only, incoming aggregate intent, identifiers or relationships reject with no native Schema and no mapping base. |
| Source whole owner | Nonempty current or incoming declarations reject, including a clear. |
| Source field ownership | Selecting the new fields, or selecting an occurrence ID, rejects. `SOURCE_OWNED_FIELD_SCHEMA` still does not write these fields. |
| Unrelated Source update | A supported field update that is not one of these declarations preserves them byte for byte, including unset aggregate intent. |

## Client boundary

CLI and MCP preserve bool presence, segment repeated flags,
uint64 versions, connection IDs and owner UIDs on native authoring, get,
reference, impact, retarget, delete, export, diff, reapply and branch calls.
Manifest shorthand for a SchemaRef does not invent a connection ref.

Studio shows unset aggregate intent as unspecified and a stale target path as
unresolved. A type ref drawn next to a relationship is two facts. Loaded-model
correlation is not the backend check.

Completion requires maintained executable fixtures and retained commands and
results at the candidate SHA. The assertion map below identifies the native
proof boundaries. It does not cover service or domain membership, source import,
or stream behavior.

## Executable assertion map

| Requirement boundary | Maintained evidence |
| --- | --- |
| Explicit aggregate intent and behavioral controls | `business_data::authenticated_order_fixture_keeps_business_declarations_explicit` |
| Identifier and relationship admission with atomic refusal | `business_data::public_identifier_and_relationship_rules_reject_without_publishing` |
| Real `items` field and independent type reuse | `business_data::closure_real_items_field_resolves_without_a_synthetic_hop`, `closure_type_ref_edit_leaves_the_relationship_and_its_id` |
| Shared discovery work, complete Schema scans and page-only output | `business_data::review_red_incoming_discovery_charges_every_examined_schema`, `review_red_truncated_snapshot_does_not_present_omitted_schema_referrers_as_complete`, `review_red_one_row_page_of_4097_references_stays_valid` |
| Prepared size including assigned IDs | `business_data::assigned_business_ids_cannot_grow_a_schema_past_its_byte_ceiling` |
| Path bounds, reorder and incarnation identity | `business_data::remainder_path_caps_resolve_and_refuse_the_next_byte`, `remainder_collection_reorder_keeps_occurrence_ids`, `remainder_owner_recreate_scopes_reused_ids` |
| Stale receipts and operational failures | `business_data::remainder_stale_match_and_receipt_leave_the_index`, `remainder_store_and_decode_failures_publish_nothing`, `remainder_authorization_failures_publish_nothing` |
| Target privacy and no privilege grant | `business_data::closure_hidden_schema_get_and_checked_target_are_generic`, `closure_target_reference_denial_is_not_source_write_denial` |
| Source policy matrix and legacy tolerance | `business_data::closure_source_policies_reject_create_and_whole_owner_matrix`, `closure_missing_legacy_ref_and_supersedes_stay_admitted` |
| Native CLI identity, presence, version isolation and branches | `trogon-atlas-cli/tests/business_data_cli.rs` |
| Authenticated MCP authoring, references, relationship-only delete, retarget and branches | `trogon-atlas-mcp/tests/business_data_mcp.rs` |
| Studio wire codec, presence and exact loaded selectors | `server/schema-business-codec.test.mjs`, `SchemaBusinessDetails.test.tsx` |

The composed baseline passed workspace, protobuf, schema equality and Studio
checks. The subsequent retained-claim correction and Business proof additions
require their own focused execution receipts. Checklist closure remains pending
the final requirement audit; this map alone does not claim a passing run.

## Integration controls

Legacy fields[].ref and supersedes resolution tolerance must stay unchanged when
new business admission is added. Reference provenance includes the business
occurrence ID. Source fields updates that break local paths reject without
removing or repairing local declarations. Validate the shared CatalogTitle/Text
leaf extraction with buf FILE acknowledgments and Studio vendored proto loading;
wire message equality alone does not prove the import graph is compatible.
