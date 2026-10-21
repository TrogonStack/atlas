# DataProduct acceptance fixture

Status: acceptance criteria and executable coverage for the
[DataProduct contract](data-product-contracts.md). Tables use logical fixture
notation, not literal manifest syntax. The executable coverage section records
which native checks exist; illustrative tables are not evidence that every
permutation has run.

## Checked context and existing anchors

Retain the baseline snapshot identity, branch, exact keys, authoritative owner
UIDs, member IDs and connection IDs. Every entity below uses exact version `1`
unless a case explicitly selects version `2`. Symbolic labels identify returned
UUIDs, never list positions or UUID literals to guess. Mutation cases start
from independent copies of the baseline, with fresh namespaces recorded when
needed for isolation. No case depends on running another case first unless
the sequence is explicit.

The existing
[Specification public-service fixture](../../rsworkspace/crates/trogon-atlas-server/tests/specification_fixture.rs)
uses namespace `contracts`, with the following exact model paths and labels.
These are the executable fixture's names, not every illustrative name in its
[acceptance reference](specification-and-api-acceptance.md).

| Exact fixture selection | Existing declaration |
| --- | --- |
| `contracts/order-request@1` | Integration ReadModel. |
| `contracts/order-adapter@1` | Processor selected by `contracts/order-intake@1` AutomationSlice. |
| `contracts/order-intake@1` | Observes `order-request@1` and emits `contracts/place-order@1`; retain its source observation ConnectionRef and paired Processor occurrence. |
| `contracts/handle-order@1` | CommandSlice handling `place-order@1`, emitting local Event `contracts/order-placed@1`. |
| `contracts/receipt-projection@1` | Projects `order-placed@1` into ReadModel `contracts/order-receipt@1`. |
| `contracts/order-transport@1` | Channel bound to the `order-intake@1` observation of `order-request@1`, not `order-receipt@1`. |
| `contracts/order-api@1`, MESSAGE `Submission` | Maps to `order-request@1`. |
| `contracts/order-api@1`, MESSAGE `Receipt` | Maps to `order-receipt@1`; shares the local schema with Submission without merging roles. |
| `contracts/order-api@1`, OPERATION `Submit` | HTTP `POST /orders`, status `201`; REQUEST Submission and RESPONSE Receipt have distinct association IDs. |
| `contracts/order-api@1`, OPERATION `Read receipt` | HTTP `GET /orders/{id}`, status `200`; another RESPONSE Receipt occurrence. |
| `contracts/order-api@1`, OPERATION `Receipt notice` | CHANNEL binding selects `order-transport@1`; NOTIFICATION Receipt. This is not response exposure or a receipt observation. |
| `contracts/public-gateway@1`, `contracts/partner-gateway@1` | Separate Component ProviderExposure associations selecting Submit only. |
| `contracts/checkout@1` | ApiUsage selects Submit with public-gateway exposure; another usage leaves its provider unspecified. Neither usage becomes product access automatically. |
| `contracts/native-order-shape@1` | Named native Schema. |
| `contracts/fixture.contract@1` | TypeLibrary defining protobuf message `fixture.contract.Order` in `orders.proto`. |
| `contracts/shared-contract@1` | Specification SCHEMA `Order shape` defined by ARTIFACT `Order source`, path `order.graphql`, selector syntax `graphql-type`, value `Order`. |

The shared artifact contains exact UTF-8 text `type Order { id: ID! }` followed
by a newline. Its SHA-256 digest is
`ead4699b692541fca1d93a621700779902c58f2eec55ed73c3d32597a00ec136`.
Retain the returned Specification UID, schema/artifact IDs and expected selector
along with that digest for the ContractTypeRef. The source remains opaque;
these identities and bytes do not establish GraphQL semantic validation.

The existing
[resource public-service fixture](../../rsworkspace/crates/trogon-atlas-server/tests/architecture_fixture.rs)
uses namespace `commerce`. Retain its independent
[resource acceptance context](channel-and-datastore-acceptance.md).

| Exact fixture selection | Existing declaration |
| --- | --- |
| `commerce/order-feed@1` | Integration ReadModel observed by Processor `commerce/order-intake@1` through AutomationSlice `commerce/observe-orders@1`. |
| `commerce/observe-orders@1` | Distinct source occurrences `observation_a` and `observation_b` both select `order-feed@1`, paired with the same Processor; emits `commerce/import-order@1`. |
| `commerce/handle-import@1` | Handles `import-order@1` and emits local Event `commerce/order-imported@1`. |
| `commerce/project-order@1` | Projects `order-imported@1` into ReadModel `commerce/order-view@1`. |
| `commerce/order-delivery@1` | Channel with distinct bindings to observation_a and observation_b. |
| `commerce/feed-cache@1` | DataStore attachment selects `order-feed@1`. |
| `commerce/order-index@1` | DataStore attachment selects `order-view@1`. |

The namespace-qualified distinction matters: `contracts/order-intake@1` is a
slice, while `commerce/order-intake@1` is a Processor. Do not merge fixtures by
slug. Their sources set up separate test contexts. A combined acceptance run
must explicitly materialize the chosen anchors into one authorized model scope
and retain the resulting checked context. The native DataProduct fixture below
uses its own explicit anchors.

## Product declaration criteria

Add DataProduct `contracts/order-data@1`. Port names below are display labels;
retain their independent DataPortIds. For setup, use valid client-supplied IDs
on a new owner or stage id-less additions and use the authoritative returned
IDs in later guarded local selections. Do not submit preview-assigned IDs as
active IDs or predict server-owned UIDs.

| Direction and port name | Proposed declaration |
| --- | --- |
| INPUT `Submitted order` | Data selects ReadModel `contracts/order-request@1`. |
| INPUT `Submitted order audit` | Separately identified input selecting that same ReadModel; no endpoint deduplication. |
| INPUT `Placed fact` | Data selects Event `contracts/order-placed@1`; local-fact evidence is the exact emission in `contracts/handle-order@1`. |
| INPUT `Unspecified input` | No data selection; classified undeclared. |
| OUTPUT `Receipt` | Data selects `contracts/order-receipt@1`; ContractTypeRef pins shared-contract Order shape as above; declared lineage selects only Placed fact. |
| OUTPUT `Receipt native` | Separate port selecting the same receipt ReadModel; contract selects exact `contracts/native-order-shape@1`; lineage absent. |
| OUTPUT `Receipt protobuf` | Same ReadModel again, contract selects exact `contracts/fixture.contract@1` and `fixture.contract.Order`; present empty lineage. |
| OUTPUT `Submission feed` | Data selects `contracts/order-request@1`; Channel access selects order-transport and its exact binding/observation; lineage absent. |
| OUTPUT `Commerce feed` | Data selects `commerce/order-feed@1`; store access selects feed-cache attachment and separate Channel accesses select order-delivery bindings for observation_a and observation_b. |
| OUTPUT `Commerce view` | Data selects `commerce/order-view@1`; store access selects order-index attachment; no inferred feed-cache access. |
| OUTPUT `Unspecified output` | No data, no contract, empty access and absent lineage; visibly undeclared, not imported/source-only. |

On Receipt, declare independent OPERATION accesses selecting Submit's exact
RESPONSE Receipt occurrence: one with public-gateway's exposure, one with
provider undeclared, and another repeated public-gateway selection with its own
access ID. Add access selecting Read receipt's RESPONSE occurrence with provider
undeclared. Partner-gateway is deliberately not selected. Receipt native and
Receipt protobuf receive no access merely because they select the same data.

Retain all product data, contract, access and lineage association IDs separately
from port IDs and from the selected external association IDs. Source data and
contract targets are declarations, not assertions that the ReadModel fields
have been compared to GraphQL, protobuf or native Schema shapes.

## Required query results

These are semantic queries. Implementation evidence must identify the actual
supported public requests; this reference invents no RPC names or flags.

| Query | Required result |
| --- | --- |
| Fetch product through service, CLI and MCP; inspect through Studio | Preserve every port, authored order, exact reference, optional presence and association identity. Studio reviews the same authoritative data. |
| Inspect Receipt access | Return only explicitly selected operation/response occurrences and optional exposure; no partner-gateway, checkout usage, inferred URL or deployment claim. Preserve repeated accesses independently. |
| Inspect Receipt native or Receipt protobuf access | Undeclared, even though another port offers the same ReadModel through explicit API selections. |
| Inspect Submission feed | Resolve order-transport's selected binding to the exact order-intake observation, request ReadModel and paired order-adapter Processor. |
| Inspect Commerce feed | Resolve exact feed-cache attachment and both distinct order-delivery observations/bindings; do not collapse them by shared ReadModel or Processor. |
| Inspect Commerce view | Resolve order-index only; sharing a product or behavior path does not select feed-cache or order-delivery. |
| Inspect contract alternatives | Preserve native Schema, exact artifact definition pin and protobuf type independently; opaque GraphQL remains unverified, with no inferred compatibility verdict. |
| Inspect Receipt declared lineage | Only Placed fact, with its local port ID and lineage association ID; neither repeated Submitted order port is added automatically. |
| Inspect Receipt producing lineage | Existing receipt-projection connects order-placed to order-receipt. Existing slices also connect order-request to order-receipt; report model-path evidence separately from the narrower authored declaration and runtime proof. |
| Inspect Receipt native lineage | Unknown declaration because absent; any checked producing path remains a separate result. |
| Inspect Receipt protobuf lineage | Explicit empty declaration, independently of existing producing behavior; do not claim no real dependencies. |
| Select an undeclared input in a separate lineage variation | Retain a valid local port selection with unknown data/producing relationship, without fabricating an external reference. |
| Inspect Unspecified output or ownership | Output data and ownership undeclared; no source-import, owner or mapped-offering claim. The baseline product is a draft because this output is undeclared. |
| Remove Unspecified output in a guarded variation | Remaining resolving outputs yield confirmed mapped data; empty optional access/provider/lineage remains visibly undeclared and does not assert deployed availability. |
| Incoming references, impact and authorized closure | Discover nested exact data/type/access owners and selections, with local lineage identified by port. Preserve occurrence identity and authorized scope. |
| Resolve a port or product association externally | Use exact product key and UID plus typed port ID/direction or ConnectionId respectively; reject swapping port and association selectors. |
| Export and unchanged native reapply | Preserve all values, authored order, IDs, selector presence and retained pins. No external-source conversion or parser is invoked. |

## Admission and access failures

Run cases through authorized standalone and guarded batch writes and previews
where applicable. Rejection leaves the stored content and identities unchanged.

| Case | Required outcome |
| --- | --- |
| Empty outputs, blank name/title/purpose, nil/malformed/duplicate port IDs, duplicate association IDs across nested lists, missing required oneof or unknown native enum | Reject atomically. Absent optional purpose is valid; blank supplied purpose is not. |
| Duplicate input name within INPUT, or output name within OUTPUT | Reject; case-sensitive exact comparison applies. Identical names across directions remain valid. |
| Repeated targets on distinct named ports; repeated identical access paths | Accept with distinct identities and no endpoint-based deduplication. |
| Output data selects Event, Command, DataStore or operation | Reject; confirmed output data must select a ReadModel. |
| Event input exists without a confirmed local CommandSlice emission | Reject the supplied Event selection. Creating a ReadModel for external integration is a separate explicit modeling action. |
| Several authorized CommandSlice emissions select the same Event input | Accept when confirmed, reporting checked occurrence identities without requiring or inventing a unique emitter. |
| Emission evidence is outside a display projection, or its discovery is incomplete | Discover authorized evidence beyond the projection cap. If the search exceeds its budget or fails, reject new admission with the appropriate incomplete/operational result; do not claim no emission exists. |
| Event confirmation reaches its separate row or occurrence ceiling | Count up to 5,000 distinct Event/CommandSlice rows combined and 5,000 emission examinations independently of the visible resource budget. Include nonmatching candidate rows and repeated emission targets. Accept completed confirmation at the boundary; reject an admission requiring additional work with RESOURCE_EXHAUSTED and no writes. |
| A guarded batch repeats Event confirmation across products or pages | Share the request budget; reused exact rows count once, repeated emission examinations consume work, and pagination/product iteration never resets either ceiling. Diagnostic exhaustion leaves evidence incomplete rather than reporting a missing emission. |
| Omit input/output data | Accept undeclared. Supplied nonexistent or inaccessible targets do not become accepted undeclared ports. |
| Put access on an undeclared output | Reject; optional standalone shape/lineage declarations remain valid without access. |
| Select order-transport as Receipt Channel access | Reject: its observation targets order-request, even though Receipt notice binds that Channel and mentions Receipt. |
| Select feed-cache attachment for Commerce view, or order-index for Commerce feed | Reject exact ReadModel mismatch. |
| Pair observation_a with observation_b's Channel binding | Reject; matching endpoints cannot substitute for exact observation identity. |
| Supply a raw ReadModel, ProcessorEdge, emitted CommandEdge or projection edge as the observation | Reject wrong connection role or owner kind. |
| Select Submit REQUEST as Receipt or Submission feed OPERATION access | Reject: REQUEST never proves an output exposure, even when its mapping matches the selected output. |
| Select Receipt notice NOTIFICATION as Receipt access | Reject despite its mapping to order-receipt. |
| Select another operation's response with Submit, or another Specification's exposure | Reject exact owner/member relationship mismatch. |
| Select public-gateway's Submit exposure with Read receipt | Reject; that provider has no explicit exposure of Read receipt in the fixture. |
| Omit provider_exposure for valid Submit RESPONSE access | Accept provider undeclared without expanding public-gateway and partner-gateway. |
| Remove provider exposures after repairing their existing local usages; retain provider-undeclared product operation access | Accept contract-level access with a valid RESPONSE mapping; Specification API classification follows its own exposure declarations, not the product selection. |
| Select lineage output/foreign port ID, missing input, or duplicate same input ID for one output | Reject. Separate repeated inputs may be independently selected. |
| Wrong ContractTypeRef UID, kind, artifact ID, digest, selector value or selector presence | Reject without matching by name, bytes elsewhere or position. |
| ARTIFACT ContractMemberRef used as output type; missing native Schema or nonexistent protobuf message | Reject. No automatic format conversion or parser repairs the type. |
| Exceed 1,024 total ports, 4,096 total associations or 1 MiB encoded entity | Reject atomically; exercise each ceiling independently and the accepted boundary where lower transport/storage limits permit. |

## Identity, stale access and branch cases

| Case | Required outcome |
| --- | --- |
| Add omitted port/association IDs | Assign distinct server IDs and retain the authoritative response. Preview generates no reserved identities. |
| Rename, reorder or document repeated ports/accesses/lineage | Preserve all retained identities and optional presence. |
| Change data, contract, access selection/provider presence or lineage endpoint while retaining association ID | Reject; an explicit replacement gets a fresh association ID while its containing port survives. |
| Retarget input data with a fresh data-selection ID while lineage keeps selecting that input port | Preserve the port and lineage IDs; recompute producing evidence in the new checked context rather than carrying a former verified conclusion forward. |
| Retarget output data to another ReadModel | Require explicit pruning or replacement of mismatched access in the guarded final graph; external paths and type pins do not follow the new data implicitly. |
| Move an association between output ports, or change a port direction while keeping IDs | Reject identity reuse. |
| Remove then re-add a port/association in separate guarded writes | Assign fresh IDs; former external selectors remain unresolved. No revival by endpoint equality. |
| Remove Placed fact while Receipt still selects it locally | Reject unless the same final graph explicitly removes or replaces that lineage association. |
| Remove output data but retain its access | Reject. Explicitly pruning access in the same guarded graph permits the port to become undeclared. |
| Remove selected Channel binding, DataStore attachment, RESPONSE occurrence, provider exposure or message mapping through an otherwise accepted target edit | Preserve unchanged product access as diagnostically stale. Do not select another equivalent relationship or silently omit it. |
| Change Receipt message mapping to order-request while its RESPONSE occurrence survives | Existing Receipt access becomes invalid for that output; no product association renewal or inferred retarget occurs. |
| Re-add an equivalent target attachment/response/exposure after removal | Former occurrence pin remains stale; require explicit selection and a fresh changed access ID. |
| Rewrite a product retaining stale access | Reject until an explicit guarded repair prunes or replaces it. Unrelated writes may retain the unchanged diagnostically invalid product. |
| Recreate product, Specification, Channel or observation owner at the same exact key | New UID invalidates former port/member/connection selectors. |
| Force-delete and recreate only an output ReadModel at the same exact key | Plain ReadModelRef follows the key after recreation; it is unresolved while absent and never claims a pinned historical incarnation. |
| Create target version 2 or copy a product | Existing exact version-1 selections do not follow. Copy uses a fresh owner UID, consistent local lineage IDs and explicit clone mapping for changed external targets. |
| Nonbreaking edit at a selected exact version | Plain entity pins remain on that mutable key; retain the checked snapshot for former conclusions. Artifact definition changes must still satisfy the full ContractTypeRef pin or become stale. |
| Guarded batch creates a ReadModel and product data selection | Resolve the complete final graph atomically; member/connection owner UIDs must already be authoritative, not guessed. |
| Branch override or tombstone differs from baseline | Resolve against merged branch view, including exact targets outside the projection; never fall back through a tombstone. |
| Preview/land new or edited product with invalid access | Reject under ordinary admission rules. Preview makes no durable changes. |
| Land permitted target edit invalidating an unchanged product | Preserve diagnostic stale state without globally rejecting unrelated landing. |
| Same owner key independently created on branch and baseline | Require explicit incarnation conflict resolution. |
| Opposing same-incarnation side removed a port or association before merge/rebase/land | Prevent identity revival using recorded base/current/selected comparison. Explicit restoration renews removed identities and rewrites local lineage consistently or rejects ambiguity; external selectors do not follow silently. |
| Baseline deletes entire owner, explicit keep-ours keeps surviving branch | Preserve surviving branch incarnation and unchanged identities under existing whole-owner resolution rules. Recreated baseline requires explicit conflict resolution. |

## Authorization, bounded scans and operational failures

| Case | Required outcome |
| --- | --- |
| Reference readable cross-namespace anchors without target write permission | Accept with product write and authorized read/reference access to all selected owners and derived endpoints. No target write permission is granted. |
| Deny access to selected data, type, response mapping, provider or observation endpoint | Reject new/edited declaration without disclosing hidden details. Restricted reads/impact must not leak keys, names, bytes, providers or graph paths. |
| Retain product after selected target becomes inaccessible | Return privacy-safe unresolved state, never a public confirmation or invented undeclared field. |
| Anchor omitted only from capped model projection | Resolve exact authorized anchor in the selected branch; do not report missing or invalidate data merely because the projection is capped. |
| Safe-delete a data/type/access owner with only a hidden product referrer | Refuse in standalone and guarded batch preview/apply, without hidden-referrer details. Include every nested selected owner in protection. |
| Force-delete product or selected external owner | Preserve other entities; diagnose incoming selectors as unresolved. No cascade/pruning or target replacement. |
| Visible scan reaches combined resource budget with products included | Up to 5,000 rows remains within the budget; exceeding returns RESOURCE_EXHAUSTED for reference/impact reporting. Existing scanner coverage alone is not DataProduct coverage. |
| Delete reporting exceeds only visible scan budget | Permit explicit incomplete capped reporting under existing policy, while independently executing the complete deletion guard across namespaces. |
| Hidden referrer lies beyond visible budget or projection cap | Complete guard still refuses deletion. Test both single-target and multi-target guarded batches in the merged branch view. |
| Complete safe-delete scan times out, errors, or cannot finish | Refuse mutation; incomplete scan cannot prove absence of references. No partial success in a guarded batch. |
| Visible scanner has backend/authorization/decoding operational error other than budget exhaustion | Return an error; do not silently use the budget fallback or report no referrers. |
| Lineage/impact traversal hits a budget, cycle or inaccessible segment | Preserve checked scope and explicit incomplete/unknown state, with visited identities and no unbounded expansion. Do not assert negative or verified lineage from a partial path. |
| Add descriptive references around existing cycles | No invented product execution ordering, product-to-product edge or replacement of supersession/TypeLibrary/behavioral cycle rules. |

## Evidence required before implementation acceptance

Acceptance needs executed public backend writes, reads, reference/impact
queries, authorization boundaries, guarded mutations, branch preview/landing,
native CLI/MCP export/reapply and Studio review coverage. Core helper checks
alone cannot prove authorization or persistence. Component checks alone cannot
prove deployed browser behavior. Verification must identify which cases ran,
their exact requests/results and rejection diagnostics, retained fixture keys,
UIDs and IDs, snapshot/revision, branch and authorization context.

## Executable native coverage

The [connected DataProduct fixture](../../rsworkspace/crates/trogon-atlas-server/tests/data_product_fixture.rs)
materializes namespace `product-fixture`, exact version `1`, with ReadModels
`submitted` and `receipt`, Event `placed`, Command `place`, Processor `adapter`,
and slices `intake`, `handle`, `project` and `observe`. Returned identities pin
repeated Channel observations, a DataStore attachment, API RESPONSE access and
selected or undeclared providers. `orders` keeps declared lineage narrower
than its checked producing paths. Inspection resolves exact anchors with a
zero general snapshot cap; native export and unchanged reapply preserve port,
association and owner identities under the normal export budget.

The [authenticated backend checks](../../rsworkspace/crates/trogon-atlas-server/tests/data_product_backend.rs)
exercise branch tombstones, Reader authorization, hidden targets and referrers,
exact Store/Channel/RESPONSE selections, artifact and compiled type pins,
undeclared providers, Event confirmation, final batch overlays and shared
confirmation work budgets. Core identity and budget helper tests cover local
lifecycle rules and boundary accounting independently.

The [CLI roundtrip](../../rsworkspace/crates/trogon-atlas-cli/tests/data_product_cli_integration.rs)
and [MCP transport checks](../../rsworkspace/crates/trogon-atlas-mcp/tests/mcp_integration.rs)
exercise native authoring, exact inspection and export/reapply. The
[Studio inspector checks](../../tsworkspace/webapps/studio/src/components/DataProductInspector.test.tsx)
exercise backend result rendering, repeated selections and historical-context
handling. These tests use controlled local stores and service transports.

This coverage does not establish deployed browser behavior, distributed
transactional reads, exhaustive branch recovery or operational failure
permutations, transformation execution, ownership directories, external
import/sync, format parsing or schema compatibility. Those require their own
acceptance evidence; illustrative cases remain requirements where no linked
test exercises them.
