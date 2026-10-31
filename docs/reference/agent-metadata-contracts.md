# Agent and shared metadata contracts

Status: implemented and verified against the
[acceptance specification](agent-metadata-acceptance.md). Native admission,
reference authorization, occurrence lifecycle, Source isolation and connected
clients are covered by automated acceptance. Studio evidence covers real
protobuf decoding and component inspection. This closes the Agent and shared
metadata items in the [coverage inventory](eventcatalog-coverage.md), without
claiming deployed-browser verification or completion of its other workstreams.

## Existing behavior and field dispositions

Historical comparison only: the following source description is research
provenance, not an active integration requirement. EventCatalog-specific work
is deferred to the final mile under the [native-first scope](eventcatalog-coverage.md#native-first-scope).

The comparison uses EventCatalog commit
[`2736a2272207cf1b1ee09e6a440401d2215c6380`](https://github.com/event-catalog/eventcatalog/tree/2736a2272207cf1b1ee09e6a440401d2215c6380).
Source anchors below describe field shapes, not a migration contract.

| Source anchor and field | Earlier native representation | Native extension |
| --- | --- | --- |
| [agentModel and agentTool](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L430): model provider/name/version, tool name/type/icon/url/description | Component `tech`, `doc` and `metadata` retain descriptive equivalents, without typed model/tool semantics. | Component-only AgentAnnotation. |
| [baseSchema](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config.ts#L120): draft and deprecated | LifecycleAnnotation already retains free status and explanatory doc. | LifecycleAnnotation draft title/message and optional deprecation calendar date; no duplicate lifecycle entity. |
| Same baseSchema: resourceGroups | Component.members curates hosted entities; TagAnnotation groups textually. Neither represents named ordered presentation groups. | ResourceGroupsAnnotation with exact native entity references. |
| [badge](https://github.com/event-catalog/eventcatalog/blob/2736a2272207cf1b1ee09e6a440401d2215c6380/packages/core/eventcatalog/src/content.config-shared-collections.ts#L4): content, colors, icon, URL | TagAnnotation and LinkAnnotation retain labels and links, without badge styling. | BadgesAnnotation for inert presentation. |
| NoteAnnotation.created_at and system creation/update metadata | Annotation attribution text and backend timestamps already exist. | Preserve existing semantics. No generic authored created/updated dates here; authored changelog and ADR dates belong to their owning contracts. |

These are native adaptations, with stricter values than the comparison source.
No importer, migration, provider synchronization or federation is proposed.

## Representation and scope

An agent is a Component carrying one `trogonatlas.eventmodel.v1alpha1.AgentAnnotation`.
It is an implementation unit with existing membership, communication and
responsibility semantics. A separate entity would duplicate those relationships
without an independent lifecycle requirement. Processor remains a behavioral
primitive; an annotation never turns a Component into a Processor or grants it
execution privileges. ExternalSystem is not an alternate agent owner.

AgentAnnotation is presence-significant even with no model or tools. Its
optional `model: AgentModel` contains independently optional `provider:
ModelProvider`, `name: ModelName`, `version: ModelVersion`. A present model must
contain at least one value. Unknown provider names and opaque version labels
are allowed; no provider lookup or semantic-version coercion.

Its ordered `tools: AgentTool[]` contains descriptive declarations with required
`name: ToolName`, required `type: ToolKindText`, optional `description:
MetadataText`, `icon: IconToken`, `url: MetadataUrl`. Tool identity is the exact
(name, type) pair within the Component. Duplicate pairs are rejected; same name
with different type is allowed. Ordering is retained. Tools are values without
independent references, owner UIDs or connection IDs. URL is an inert documentation
link. Credentials, headers, secrets, endpoints for execution, calling, discovery,
permission grants and executable schemas are outside this contract.

Shared annotations belong in `trogonatlas.annotation.v1alpha1`. Admit them only
on top-level native entity metadata, not FieldSpec, scenario, edge or nested
schema metadata. AgentAnnotation remains Component-only. Each new annotation
and LifecycleAnnotation is singleton per owner; duplicate instances are errors,
including byte-identical instances. Existing unrelated annotation multiplicity
is unchanged. Source records and retained source overlays are not native owners.

Extend LifecycleAnnotation with optional `draft: DraftNotice` (optional title,
required message) and optional `deprecated_on: CalendarDate`. Existing `status`
and `doc` remain free strings. Compare status using the service's trimmed,
lowercase convention. DraftNotice requires status draft; deprecated_on requires
status deprecated. Draft status without notice and deprecated status without
date remain valid. Do not synthesize server timestamps or automatically change
status when a date passes. CalendarDate is an exact Gregorian YYYY-MM-DD string,
years 0001 through 9999, without timezone or time; impossible dates are rejected.
No chronological comparison with server creation time is implied.

BadgesAnnotation contains ordered `badges: Badge[]`: required content:
BadgeText, background_color and text_color: HexColor; optional icon: IconToken
and url: MetadataUrl. Reject identical full badge values; the same label with
different styling is allowed. No CSS, HTML, remote icon fetching or runtime URL
navigation is implied. Rendering must escape text and use an allowlisted icon
resolver. Unknown tokens render a text fallback.

ResourceGroupsAnnotation contains ordered `groups: ResourceGroup[]`. Each group
has `group_id: ConnectionId`, optional title: MetadataTitle, ordered `items:
ResourceGroupItem[]`, optional `display_limit: PositiveDisplayLimit`, optional
`sidebar: bool`. Each item has `connection_id: ConnectionId` and `target:
EntityRef`. Absence means no display cap and no sidebar preference. Display
limits never truncate stored items or reference traversal. Repeated exact
targets are allowed as separate identified occurrences, including across groups.
No group nesting, family/latest selectors, computed membership, Source selection,
member/port selection or behavioral implications. This is presentation grouping,
not system/domain membership or hosting.

## Values and budgets

These backend ceilings use encoded UTF-8 bytes for text values.
Nonblank values must contain a non-whitespace character; preserve accepted text
exactly and reject control characters except newline/tab in MetadataText.

| Value or collection | Ceiling or validation |
| --- | --- |
| ModelProvider, ModelName, ModelVersion, ToolName, ToolKindText | Nonblank, 256 bytes each. Exact case-sensitive equality. |
| MetadataTitle, BadgeText | Nonblank, 256 bytes. |
| MetadataText | Nonblank, 4096 bytes. Optional fields must be absent rather than empty. |
| IconToken | 128 ASCII bytes matching `[A-Za-z0-9][A-Za-z0-9_.:-]*`; no URL or path interpretation. |
| HexColor | Exactly # followed by six hexadecimal digits; retain spelling. |
| MetadataUrl | Absolute HTTPS, host required, no userinfo, at most 2048 bytes; never fetched. |
| PositiveDisplayLimit | Integer 1 through 500; zero is invalid. |
| Tools / badges / groups / items | At most 64 tools, 32 badges, 32 groups and 256 total group items per owner. Empty lists valid. |
| Encoded additions | At most 64 KiB total encoded bytes for these annotations per owner, including lifecycle. Existing smaller entity/request/metadata limits still apply. |

Admission counts and validates all items before publication, independent of a
UI display limit. Fail atomically on size or work-budget exhaustion. Reads use
existing pagination, authorization and reference/impact budgets; incomplete
traversal is explicitly reported and cannot authorize deletion or prove absence.
Do not create an unbounded recursive group expansion query.

## Identity, references and authorization

Use existing exact `EntityRef` kind plus native Id (positive numeric version),
not a resource-name string, provider key, Source handle or guessed incarnation.
References to agent Components still have kind Component. Group and item IDs
share the owner's occurrence uniqueness domain and must be checked against all
other identified owner occurrences. Persisted IDs survive reorder and text/style
edits. Changed item target requires a fresh item ID; moving an item between groups
requires a fresh item ID. Removed IDs cannot be reused in the same incarnation.
Group IDs survive title/item edits; a recreated group receives a fresh ID.
Copy/new owner versions renew all local IDs. Apply the existing branch conflict,
restore and incarnation rules documented in
[responsibility contracts](responsibility-contracts.md#association-identity-and-lifecycle).

Add explicit annotation decoding to core outbound-reference extraction and
retargeting in `rsworkspace/crates/trogon-atlas-core/src/refs.rs`. Do not scan
arbitrary Any strings. Emit each item target with its group/item locator; reverse
index and impact must retain occurrence distinction. Safe delete must see these
refs, including hidden owners; force-delete retains unresolved exact refs.
Generic entity retarget renews affected item IDs and validates the complete
edited owner. Malformed known annotation bytes cannot masquerade as zero refs.

Resource-group occurrences are not architectural connections. Do not add them
to ConnectionRef or expand ResponsibilitySubject's closed occurrence inventory.
Team/Person may already assign responsibility to the owning Component or target
entity. Tool and badge values are not independent responsibility subjects.

Require owner namespace write and target read/reference authorization for all
proposed group items, including unchanged selections on an edited owner. Resolve
against the authorized final branch overlay, including tombstones. Hidden reads
retain NOT_FOUND behavior; Reader writes retain PERMISSION_DENIED. Incoming
hidden refs block safe deletion without revealing their identities. Visible
owners may retain their authored exact keys, but hydration, counts and diagnostics
must not reveal hidden target details. No annotation grants runtime access.

Existing Source authority remains isolated: no source overlays, raw pointers,
provider fields or URLs create native reference endpoints. No native mutation
writes Source-owned content. Agent model provider is descriptive text, not a
Source provider registration. References cannot hydrate through Source overlays.
New annotation extensions are native-only. Reject Source creates carrying them
under every ownership policy, including TITLE-only: initial create copies the
whole incoming owner. Reject WholeOwner changes when current or proposed new
annotations are present, including clears. Existing unrelated field-specific
updates preserve local annotations exactly. New metadata paths and contained
group/item selections remain unsupported in the closed Source inventory.

Require current revision/incarnation, branch and authorization guards at apply,
not merely successful preview. Final-overlay validation is atomic across batches.

## Implementation and verification boundaries

The [core implementation](../../rsworkspace/crates/trogon-atlas-core/src/agent_metadata.rs)
validates values and annotation scope, including fields inside decoded Schema
Any payloads. Generic occurrence persistence applies retirement and renewal
rules to identified group items. Shared metadata extraction supplies lifecycle
filtering across native kinds. Use the
[advertised protocol revisions](../../rsworkspace/crates/trogon-atlas-proto/src/lib.rs)
for writer admission and coordinate descriptor generation with schema changes.

Trace anchors for implementation review:

- `proto/trogonatlas/eventmodel/v1alpha1/infrastructure.proto`: Component members, tech, relationships and metadata.
- `proto/trogonatlas/annotation/v1alpha1/annotations.proto`: registered generic message definitions; LifecycleAnnotation status/doc and unparsed note dates.
- `rsworkspace/crates/trogon-atlas-server/src/conv.rs`: validate_entity_content and metadata count admission.
- `rsworkspace/crates/trogon-atlas-server/src/validation.rs`: entity_metadata and annotation findings; diagnostics are not sufficient mutation admission.
- `rsworkspace/crates/trogon-atlas-core/src/transcode.rs`: shared built-in and tenant descriptor JSON Any handling.
- `rsworkspace/crates/trogon-atlas-client/src/manifest_schema.rs`: well_known_schema currently requires @type but leaves Any payload permissive; schema success is not semantic validity.
- `tsworkspace/webapps/studio/server/index.mjs`: decodeAnnotations; real-codec and Inspector fixtures verify retained annotation content.
- `proto/trogonatlas/eventmodel/v1alpha1/responsibility.proto` and `rsworkspace/crates/trogon-atlas-core/src/responsibility_refs.rs`: existing closed typed subject inventory.

Acceptance verifies occurrence admission inside Any, ordered semantic equality
and diff, lifecycle extraction, reference indexing and the stated bounds.
These remain native product choices rather than identical source semantics.
JSON schema accepts some semantically invalid annotations; backend validation
rejects them without publication. Registry-aware payload schemas are optional.
No historical metadata rewrite or source date coercion is implied.
