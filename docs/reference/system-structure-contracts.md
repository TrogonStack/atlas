# System grouping, membership and actor contracts

Status: implemented native contract. The
[acceptance requirements](system-structure-acceptance.md) define verification
obligations; final composed execution and checklist closure remain separate.
It addresses the system structure item of the
[EventCatalog checklist](eventcatalog-coverage.md#remaining-model-checklist):
higher-level system grouping, authored domain, system and service memberships,
and system-level actor relationships. It reads no external catalog and adds no
migration, provider or federation behavior.

## Decision

A dedicated `SystemGroup` kind is required for abstract system grouping.
Dedicated fields on existing kinds cannot carry it without changing what those
kinds mean:

| Candidate | Why it cannot hold the grouping |
| --- | --- |
| Component | A [deployable implementation unit](system-relationship-contracts.md#scope-and-existing-representations). Its raw `members` list is curated implementation intent. Making it an abstract container would drop the deployable meaning and make every grouping claim look like a deployment claim. |
| ExternalSystem | A third-party boundary with no members field. Giving it members would make "owned by the vendor" and "grouped by us" the same assertion. |
| Domain, Subdomain, BoundedContext | The [strategic hierarchy](domain-scoped-listing.md): problem space, classification and realization. Subdomain has one parent and a required classification, so it cannot be arbitrary nesting. |
| Persona | A human behavioral role. It is neither a directory Person nor an authenticated principal and cannot become a system actor registry. |

Dedicated fields are also required, because the source authors them on the
grouping side and Atlas must not derive them from namespaces:

- `Domain.members`, for authored domain nesting and shared memberships.
- `SystemGroup.actors`, for typed human or software actor declarations.

Service and Actor remain non-kinds. A source "service" maps to Component only
when the deployable meaning holds; otherwise it is uncovered, not coerced. Human actors select exact Personas; software actors select explicit native boundaries. Neither is inferred from an inline name, directory entry or principal.
This contract replaces the earlier deferral of abstract systems. The other deferrals in the
[relationship contract](system-relationship-contracts.md#scope-and-existing-representations)
stay in force.

## Scope and existing representations

| Representation | Status | Boundary kept by this proposal |
| --- | --- | --- |
| Component `members`, `relationships`, `communications` | Shipped | Unchanged. `members` stays a raw, unidentified `EntityRef` list. No membership occurrence IDs are retrofitted. |
| ExternalSystem | Shipped | Unchanged. Still no members field and no scope. |
| Domain, Subdomain, BoundedContext | Shipped | Strategic realization unchanged. [Domain-scoped listing](domain-scoped-listing.md) stays namespace-derived and does not read authored membership. |
| Persona | Shipped | Human only, descriptive `role`. [Slice edges](../../proto/trogonatlas/eventmodel/v1alpha1/slices.proto) keep issuing and display meaning. |
| `Component.ui_hostings`, `InspectHumanInteraction` | Proposed in [human interaction contracts](human-interaction-contracts.md) | Neither implemented nor changed here. See [hosting reconciliation](#reconciliation-with-ui-hosting). |
| Source integration | Shipped | Native-only for everything below; see [Source isolation](#source-isolation). |
| SystemGroup, `Domain.members`, memberships, group relationships, actor links | Implemented | Defined below. |

Event Modeling stays the behavioral foundation, per the
[Event Modeling principle](../explanation/principles/event-modeling.md). A
group has no slices, time or causal order. It adds a navigable layer over
existing identities and does not introduce a competing behavioral language.
Incomplete or dangling groups stay inspectable, per
[layered information](../explanation/principles/layered-information.md).

## Source evidence and disposition

The pinned source is EventCatalog commit
[`2736a22`](https://github.com/event-catalog/eventcatalog/tree/2736a2272207cf1b1ee09e6a440401d2215c6380).
This table accounts for each source field relevant to the first checklist item.
Dispositions describe Atlas representations and exclusions, not parity.

| Source field | Evidence | Disposition |
| --- | --- | --- |
| System `scope` (`internal` or `external`, default internal) | [systems schema](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L761-L818) | `SystemGroup.scope`, required, no default. |
| System `services` | same | Component members only when deployable. Otherwise uncovered. |
| System `containers` | same | DataStore member. |
| System `flows`, `entities` | same | Exact Schema and DesignGraph member alternatives; coordinated with those implementation slices. |
| System `relationships` (`id`, `version`, optional `label`) | [relationship pointer](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L740-L748) | `SystemGroupRelationship` to an exact SystemGroup. Label optional. An omitted drawing label does not erase the authored relationship. |
| System `actors` (`id`, `name`, `label`, `direction`) | [actor schema](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L750-L759) | SystemActorLink selects an explicit Persona or software boundary; direction and optional label are authored. No human classification is inferred from a name. |
| Nested systems | [systems loader](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L761-L792) | SystemGroup member of SystemGroup. A graph, not a tree. |
| Domain `services`, `systems`, `domains`, `data-products` | [domains schema](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L689-L738) | `Domain.members` arms for Component, SystemGroup, Domain and DataProduct. The `domains` arm is not a Subdomain. |
| Domain `agents`, `entities`, `flows`, `sends`, `receives` | same | Agent Components, Schema and DesignGraph membership are included. Domain message claims are independent declarations in the resource-association contract. |
| Service `externalSystem` flag | [services schema](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L396-L451) | Existing ExternalSystem kind. No boolean flag on Component. |
| Pointer `version` default `latest` | [pointer schema](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/utils/pointer-schema.ts#L4-L7) | Exact positive numeric version. `latest` is not representable. |
| Subdomain parentage as authored reference, graph allowed | [subdomain guide](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/docs/development/guides/domains/03-model-subdomains.md#L51-L69) | Shared parents and cycles are legal in `Domain.members`; see [Cycles](#cycles). |
| `detailsPanel` visibility flags | systems and domains schemas above | View policy, outside this contract. |

## Native wire shape

Proposed new file `proto/trogonatlas/eventmodel/v1alpha1/system_group.proto`.
Tags below are proposals; recheck availability before controlled generation.

```proto
// identity.proto
message SystemGroupRef { trogonatlas.type.v1alpha1.Id id = 1; }
// EntityKind: ENTITY_KIND_SYSTEM_GROUP = 39;   // DesignGraph reserves enum 38
// event_model.proto Entity oneof: SystemGroup system_group = 40;
//   The proto field name `system` is taken by Entity.system (SystemMeta),
//   so the kind is named SystemGroup.

enum SystemGroupScope {
  SYSTEM_GROUP_SCOPE_UNSPECIFIED = 0;
  SYSTEM_GROUP_SCOPE_INTERNAL = 1;
  SYSTEM_GROUP_SCOPE_EXTERNAL = 2;
}

message SystemGroupMember {
  oneof target {
    SystemGroupRef system_group = 1;
    ComponentRef component = 2;
    ExternalSystemRef external_system = 3;
    DataStoreRef data_store = 4;
    SchemaRef schema = 5;
    DesignGraphRef design_graph = 6;
  }
}

message SystemGroupMembership {
  trogonatlas.type.v1alpha1.ConnectionId connection_id = 1;
  SystemGroupMember member = 2;
  CatalogText doc = 3;
}

message SystemGroupRelationship {
  trogonatlas.type.v1alpha1.ConnectionId connection_id = 1;
  SystemGroupRef target = 2;
  CatalogTitle label = 3;
  CatalogText doc = 4;
}

enum SystemActorDirection {
  SYSTEM_ACTOR_DIRECTION_UNSPECIFIED = 0;
  SYSTEM_ACTOR_DIRECTION_INBOUND = 1;   // persona toward the group
  SYSTEM_ACTOR_DIRECTION_OUTBOUND = 2;  // group toward the persona
}

message SystemActorTarget {
  oneof target {
    PersonaRef persona = 1;
    ComponentRef component = 2;
    ExternalSystemRef external_system = 3;
    SystemGroupRef system_group = 4;
  }
}

message SystemActorLink {
  trogonatlas.type.v1alpha1.ConnectionId connection_id = 1;
  SystemActorTarget target = 2;
  SystemActorDirection direction = 3;
  CatalogTitle label = 4;   // optional; presence is authored
  CatalogText doc = 5;
}

message SystemGroup {
  trogonatlas.type.v1alpha1.Id id = 1;
  string title = 2;
  string doc = 3;
  repeated google.protobuf.Any metadata = 4;
  trogonatlas.type.v1alpha1.Id supersedes = 5;
  SystemGroupScope scope = 6;
  repeated SystemGroupMembership members = 7;
  repeated SystemGroupRelationship relationships = 8;
  repeated SystemActorLink actors = 9;
}

message DomainMember {
  oneof target {
    DomainRef domain = 1;
    SystemGroupRef system_group = 2;
    ComponentRef component = 3;
    DataProductRef data_product = 4;
    SchemaRef schema = 5;
    DesignGraphRef design_graph = 6;
    ExternalSystemRef external_system = 7;
  }
}

message DomainMembership {
  trogonatlas.type.v1alpha1.ConnectionId connection_id = 1;
  DomainMember member = 2;
  CatalogText doc = 3;
}
// strategic.proto: Domain gains `repeated DomainMembership members = 8; // message_claims reserves tag 7`
```

The field layout follows the [Component and ExternalSystem
convention](../../proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto).
Implementation must confirm the import graph stays acyclic when
`strategic.proto` imports the new file, and register the kind in the
[entity kind catalog](../../proto/trogonatlas/api/eventmodel/v1alpha1/entity_kinds.proto).

| Rule | Requirement |
| --- | --- |
| Typed refs | Every ref requires namespace, slug and positive numeric version. Missing, unknown or multiple oneof alternatives reject. No generic `EntityRef`, latest, family or source-handle fallback. Versions above JavaScript's exact integer range are valid. |
| Member kinds | Closed per owner. SystemGroup accepts SystemGroup, Component, ExternalSystem, DataStore, Schema and DesignGraph. Domain accepts Domain, SystemGroup, Component, ExternalSystem, DataProduct, Schema and DesignGraph. Another kind needs a later contract that adds an arm and advances the floors. |
| Scope | Required. `UNSPECIFIED` or unknown rejects. Scope is descriptive and grants no ownership. EXTERNAL does not prohibit authorized Component or DataStore members; their catalog presence does not establish who deploys them. |
| Labels and docs | `CatalogTitle` and `CatalogText` keep their existing nonblank and byte limits (256 and 16 KiB). Group relationship label is optional; present blank labels reject. Actor label and all docs are optional; absence means no elaboration. |
| Direction | Required `INBOUND` or `OUTBOUND`. A source default of inbound is not assumed. |
| Self reference | A SystemGroup member or relationship equal to the owner's exact kind, key and version rejects. A Domain member equal to the owner rejects. No other shape check reads a second owner. |
| Presence | `connection_id` presence is inspected before generic normalization, as in the [hosting proposal](human-interaction-contracts.md#proposed-hosting-shape): present empty or malformed IDs reject; omission requests assignment. |

## Membership semantics

Membership is an authored curation claim made by the containing group or
domain. It is not ownership, exclusivity, deployment, a permission boundary or
a behavioral rule.

- **Shared.** The same exact Component, DataStore, SystemGroup or ExternalSystem
  may appear in any number of groups and domains, in any namespace the author
  can reference. Adding a member needs write access on the owner and authorized
  reference access to the member, never write access to the member.
- **Authored on the container.** The member does not learn of its containers.
  Containers are discovered with incoming references. Authoring from the
  container side matches how the source authors them and keeps the group's
  steward in control.
- **Exact versions.** `checkout@1` and `checkout@2` are different members and may
  both appear in one group. A new SystemGroup or Domain version is a new owner
  incarnation and does not inherit members as live links; its member targets
  stay exact unless explicitly changed.
- **Repeated occurrences.** Equal member and doc values may repeat with
  distinct IDs. Write paths never deduplicate. Queries that return entities
  deduplicate the entity and retain every occurrence. Preserve order without
  causal meaning. An advisory validation finding may flag exact repeats.
- **Empty and dangling.** An empty list means no authored members in the checked
  owner. A member that is deleted or hidden leaves a readable authored selector
  and reports unavailable on hydration. It does not veto unrelated writes.
- **No derivation.** Authored membership never feeds `ListEntitiesByDomain`,
  project validation scope, authorization or the namespace-derived scope. Namespace
  placement and membership are separate facts and may disagree.
- **No transitive meaning.** Group-in-group membership is a graph edge. A
  Component reached through two levels is not a direct member of the outer
  group, and no inherited ownership or hosting follows.

### ExternalSystem

ExternalSystem remains a member-less third-party boundary. Third-party grouping
uses a SystemGroup with EXTERNAL scope and authorized typed members. Scope
does not imply deployment ownership or restrict member kinds:

- The container owns the membership claim. Adding an ExternalSystem to a group
  needs no write access to it and does not mean anyone deploys it.
- Responsibility for the group, or for a single membership occurrence, uses
  existing Team and Person assignments. It does not spread to the member.
- No Component proxy is synthesized for an external group, and a relationship
  labeled "contains" is still only wording.
- An INTERNAL group may list an ExternalSystem as a documented dependency
  member, mirroring what `Component.members` already allows.

### Cycles

Containment cycles are legal at write time except self membership. Rejecting
longer cycles would force a write to read other owners, which both discloses
hidden structure and cannot be made race-free across owners, because two
writers can each add one half of a cycle under independent guards. The
descriptive relationship contract already accepts cycles between distinct
endpoints.

| Surface | Behavior |
| --- | --- |
| Write | Reject exact self membership and self relationship only. Direct, longer and cross-owner cycles are admitted. |
| Reference queries | Return every occurrence once; a cycle adds no duplicate or nonterminating output. |
| Impact | Existing traversal bounds already terminate declared cycles; membership references join that traversal with full occurrence provenance. |
| Validation | Proposed advisory rules `SYSTEM_GROUP_MEMBERSHIP_CYCLE` and `DOMAIN_MEMBERSHIP_CYCLE` report a cycle with its readable ConnectionRef path. They read only authorized owners and do not claim completeness beyond the checked scope. |
| Client expansion | A client expanding members keeps a visited set of exact kind, key and version. It expands each entity once even when reached from several parents, and marks the returning edge as a back edge rather than recursing. |

No traversal RPC is selected. `GetEntity`, outgoing and incoming
`GetReferences`, subject queries and impact give a bounded, authorization-correct
route, as in the relationship contract.

## Actor relationships

A `SystemActorLink` says that a group declares an interaction with an exact
human Persona or software boundary. It is a declaration, not a path.

| Direction | Authored meaning | Event Modeling anchor |
| --- | --- | --- |
| `INBOUND` | The persona acts on or uses the group. | Existing CommandSlice persona/UI/command or UiSlice persona/UI/read model edges can ground it through a Component the group contains. |
| `OUTBOUND` | The group reaches the persona, for example by notification. | None in the shipped slice grammar. It stays an ungrounded declaration. |

- A human target is a Persona; software targets are explicitly selected Component,
  ExternalSystem or SystemGroup references. No software actor is coerced into a
  Persona, and no directory Person or principal is an actor target.
- A link creates no Person, principal, login, role, permission or directory
  record, and grants no authorization.
- It adds no `Persona` field to UI, Component or slices and emits no
  `UI.persona` or interaction slice.
- The same Persona may appear in several groups and in both directions as
  distinct occurrences. A Persona in a group's actors gives it no membership.
- A source actor identified only by an inline id and name has no native object
  until an author selects a native actor with the intended meaning. The link never auto-creates
  one.

### Reconciliation with UI hosting

The [hosting proposal](human-interaction-contracts.md) stays Component-only and
unchanged. A group hosts nothing, ExternalSystem hosting stays deferred and no
`ui_hostings` field is added to SystemGroup. A system-level actor claim can be
supported by evidence assembled from separate exact selections:

1. The actor link occurrence (`ConnectionRef` on the SystemGroup).
2. A chain of membership occurrences (`ConnectionRef` on each group) from that
   group to a Component.
3. The Component's hosting occurrence and one checked slice path from
   `InspectHumanInteraction`, whose persona equals the link's persona by exact
   kind, key and version.

Each piece is read and checked on its own. The proposal adds no composite
RPC, no composite subject and no certified "this actor reaches this system"
result. This matches the hosting document's statement that selecting
declarations together does not certify a composite interaction. A composite
check is an [open objection](#open-objections). Membership chains pin their
owners' UIDs, but the Component target is a mutable exact key, so recreating
the Component at the same key can resolve to a replacement.

## Identity and lifecycle

Reuse [connection identity](connection-identity.md), the [connection
inventory and resolver](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs) and
[persistence helpers](../../rsworkspace/crates/trogon-atlas-core/src/connection_persistence.rs).
All new occurrences share the owner's complete uniqueness domain. A Domain and
a SystemGroup each have their own domain; IDs are unique across roles inside
one owner. The selector is the existing `ConnectionRef` with owner,
`owner_uid` and ID. No new selector union arm is added.

Adding protobuf fields alone does not integrate semantic fingerprints, resolved
targets or graph discovery. These are implementation obligations.

| Role (new) | Owner | Fingerprint, excluding doc, label and order |
| --- | --- | --- |
| Group member | SystemGroup | Role plus the full typed member. |
| Group relationship | SystemGroup | Role plus the target SystemGroup. |
| Actor link | SystemGroup | Role plus full typed actor target and direction. |
| Domain member | Domain | Role plus the full typed member. |

Lifecycle rules are those of the [relationship contract](system-relationship-contracts.md#identity-versions-and-branches)
applied to these roles. Edits that change a fingerprint need a fresh ID.
Changing `scope` does not renew IDs but re-runs member-kind admission. Changing
an actor's direction is a semantic retarget. Exact-key retargeting and
force-delete recreation follow the existing rules. Existing `Component.members`
paths remain diagnostic only.

Component.members retains its shipped raw EntityRef behavior, including new
known native kinds. It gains neither occurrence IDs nor new group semantics;
this feature does not retroactively restrict its membership admission.

## Subjects, references and queries

Extend the closed ConnectionRef inventories of
[ResponsibilitySubject](../../proto/trogonatlas/eventmodel/v1alpha1/responsibility.proto)
and [CatalogSubject](../../proto/trogonatlas/eventmodel/v1alpha1/decision_document.proto)
to admit SystemGroup and Domain owners for exactly these roles:
`SystemGroup.members[]`, `.relationships[]`, `.actors[]` and `Domain.members[]`.
Reuse the existing arms. Entity subjects for SystemGroup are admitted through
the existing entity arm once the kind exists. Assignment on a group or domain
does not spread to its members, and an assignment on one occurrence does not
spread to an equal sibling.

| Surface | Required extension |
| --- | --- |
| Reference extraction | Add outgoing paths `members[i].{system_group,component,external_system,data_store}`, `relationships[i].target`, `actors[i].target.*` for SystemGroup, and `members[i].{domain,system_group,component,data_product}` for Domain. Return the existing [Reference](../../proto/trogonatlas/api/eventmodel/v1alpha1/graph.proto) fields `from`, `from_uid`, `connection_id`, `from_field`, `to`, preserving parallel occurrences. Paths are diagnostic. |
| Incoming discovery | A Component, DataStore, ExternalSystem, DataProduct, Persona, Domain or SystemGroup reports every containing or linking occurrence, including repeats and every group that shares it. |
| Subject discovery | Responsibility, Decision and Document queries match the full ConnectionRef and preserve owner UID, ID and unchanged selector. |
| Impact | Membership, relationship and actor-link references join reverse reachability with occurrence provenance. This is graph reachability, not a deployment or runtime dependency claim. |
| Retarget | Guarded exact-key retarget renews the IDs of changed occurrences. Existing selectors do not follow. |
| Safe and force delete | The complete incoming-reference guard sees every new reference. Safe delete protects hidden referrers. Force delete leaves selectors unresolved without cascading. |
| Native surfaces | Guarded put and batch, validation, diff, branch operations, preview, native manifest, export and snapshot recovery preserve fields, order and returned identities. Manifest typed-field shorthand follows the [parser](../../rsworkspace/crates/trogon-atlas-client/src/manifest.rs): omitted namespace uses the manifest's, omitted version is `1`, and shorthand never reconstructs a ConnectionRef. |
| Revision | Contract and both mutation floors are 19. The client mutation gate refuses older writers before they can drop the new kind or `Domain.members`. |

## Admission, visibility and bounds

Backend authority, namespace access, write guards and entity limits remain
applicable, per [backend authority](../explanation/principles/backend-authority.md).
Labels, memberships, actor links and responsibility grant no security authority.

| Boundary | Requirement |
| --- | --- |
| Changed owner | Validate the complete resulting owner and every exact target in the authorized final batch or branch overlay, including tombstones. Targets must exist and be readable and referenceable. Unchanged dangling owners stay inspectable and do not veto unrelated writes. |
| Concurrency | Publish under the existing guarded commit coordination. No partial owner, batch or landing publication. |
| Write authority | SystemGroup or Domain write on the owner, plus reference access to each target. A Reader mutation returns PERMISSION_DENIED; hidden direct reads return NOT_FOUND. |
| Ceilings | At most 1,024 members, 1,024 relationships and 1,024 actor links per SystemGroup, and 1,024 members per Domain. Entire encoded entity at most 1 MiB. Lower existing limits win; rejection never truncates. Larger estates nest groups. |
| Discovery work | SystemGroup joins the shared 5,000 visible resource-row ceiling. The shared 10,000 examined identified occurrences and 4,096 emitted records per request apply. Repeats consume work. Internal pages and batch members do not reset them. |
| Overflow | Budget exhaustion is RESOURCE_EXHAUSTED, never an apparently complete result. Decode, backend, timeout and authorization failures stay operational. |
| Hidden targets | Missing and hidden targets share one generic unavailable state. No hidden title, UID, etag, count, namespace inventory or referrer is disclosed. Authored selectors on a readable owner stay visible. |

## Source isolation

Authoring is native-only, following the pattern in the [relationship
contract](system-relationship-contracts.md#subjects-references-and-native-operations).

- SystemGroup is not an admitted Source target kind. Source mapping, adoption
  and creation of it reject atomically.
- `Domain.members` is not a Source-owned field. Source create of a Domain
  carrying members rejects under every ownership policy, including TITLE-only,
  because create copies incoming content.
- Whole-owner Source writes reject when current or proposed `members` is
  nonempty, including an attempted clear.
- Supported field-specific updates to unrelated Domain fields preserve local
  members.
- The existing `SOURCE_OWNED_FIELD_MEMBERS` enum value is not admission. It
  must not become usable for Domain members by presence alone.
- Rejection leaves no native or mapping-base publication. Remote connectors
  and portable selector rebinding stay deferred.

## Rollout and dependencies

| Step | Content | Depends on |
| --- | --- | --- |
| Schema | `SystemGroupRef`, enum39/union40, `Entity.system_group`, `system_group.proto`, `Domain.members`, kind catalog entry. Controlled generation, owned by the parent queue. | Tag recheck; import-graph check. |
| Core | Value validation, fingerprints, resolver roles, extraction, subject inventories. | [connections.rs](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs), [refs.rs](../../rsworkspace/crates/trogon-atlas-core/src/refs.rs), [responsibility.rs](../../rsworkspace/crates/trogon-atlas-core/src/responsibility.rs). |
| Backend | Final-overlay admission, indexes, bounds, delete guards, Source rejection, revision floors. | [source_integration.rs](../../rsworkspace/crates/trogon-atlas-server/src/source_integration.rs), [architecture_admission.rs](../../rsworkspace/crates/trogon-atlas-server/src/architecture_admission.rs). |
| Clients | Manifest, CLI, MCP and read-only Studio inspection preserving full selectors. | Native backend. |
| Validation rules | Advisory cycle, duplicate and scope-mix findings. | Core extraction. |
| Grounding recipe | Client-assembled evidence for INBOUND links. | Hosting and `InspectHumanInteraction` implemented. Not required for authoring. |

### Implementation integration obligations

The design identified the following integration obligations. Implementations
must preserve these paths when composing new native kinds and collections:

| Observation | Consequence |
| --- | --- |
| The [core Source guard](../../rsworkspace/crates/trogon-atlas-core/src/source_integration.rs) names only Component and ExternalSystem for relationship and communication rejection; other kinds fall through to a whole-owner field copy. | Name `Domain` `members` and the SystemGroup kind explicitly. Closed-inventory wording alone would let a Domain whole-owner copy carry members. |
| The [fingerprint wrapper table](../../rsworkspace/crates/trogon-atlas-core/src/connections.rs) maps single-target wrappers and handles `ArchitecturalEndpoint` separately. | `SystemGroupMember` and `DomainMember` are multi-arm oneofs. Give them the oneof-aware path, and add single-target handling for `SystemGroupRelationship` and `SystemActorLink`. |
| [Admission](../../rsworkspace/crates/trogon-atlas-server/src/architecture_admission.rs) filters Component and ExternalSystem references by `relationships[` and `communications[` prefixes. [Provenance](../../rsworkspace/crates/trogon-atlas-server/src/graph.rs) attaches occurrence IDs by the same prefixes. | Extend both for `members[`, `relationships[` and `actors[` on SystemGroup and `members[` on Domain, or new references are invisible to admission and provenance. |
| The sibling resource-association proposal also adds fields to Component, ExternalSystem and possibly Domain. | Coordinate tag numbers and revision floors across proposals before generation. |

## Open objections

- **Kind name.** `SystemGroup` is forced by the `Entity.system` field name and
  avoids confusion with Component and ExternalSystem, but may read as a group
  of systems. A rename is cheap before generation.
- **Domain carries structure.** `Domain.members` puts Component and
  DataProduct references into a "pure knowledge" kind. An alternative is
  allowing only SystemGroup and Domain members and nothing deployable.
- **Cycles.** Admitting cross-owner cycles trades early rejection for no hidden
  structure oracle and no cross-owner race. A reviewer may prefer rejecting
  same-owner chains that are visible without extra reads.
- **OUTBOUND grounding.** Whether a display path should ground OUTBOUND links
  is undecided; this proposal grounds none.
- **No composite check or traversal RPC.** Client assembly and impact may prove
  noisy for large estates.
- **Ceiling of 1,024.** Large domains must nest groups.
- **Component.members compatibility.** Its existing raw membership behavior is retained.
- **Excluded member kinds.** Flow, entity, agent, Specification and Channel
  members await their own checklist items.
