# Domain-scoped listing

`ListEntitiesByDomain` lists native entities through their declared strategic
references. Its [request](../../proto/trogonatlas/api/eventmodel/v1alpha1/service.proto)
selects a Domain ID, optional entity kinds, pagination and
`latest_versions_only`. This is a query over the existing strategic model,
not a new system-membership or authorization mechanism.

## Exact strategic selection

Domain and Subdomain references use namespace, slug and numeric version.
Selecting a Domain does not select another version of that Domain. A context
realizing a Subdomain version does not fall back to a different version of
that Subdomain.

| Returned entity | Membership rule |
| --- | --- |
| Domain | Its complete ID matches the requested Domain ID. |
| Subdomain | Its declared Domain reference matches the complete requested ID. |
| BoundedContext | At least one of its own realized Subdomain references resolves to a stored Subdomain declaring the requested Domain ID. |
| Other native entity | Its namespace has a BoundedContext realization chain to the requested Domain ID. |

Every declared realization is considered. A missing Subdomain does not suppress
a different valid realization. Repeated references and multiple matching paths
do not duplicate returned entities. A context realizing subdomains in different
domains may contribute to each domain's result.

Each BoundedContext version is checked against its own references. Other
entities retain namespace-derived scope across the visible context versions;
their numeric versions are not matched to a context's numeric version.
This scope does not establish Component containment, explicit service
membership, ownership or execution.

## Filtering and pagination

The optional kind filter restricts returned entity kinds. With
`latest_versions_only`, the result retains the greatest numeric version of each
matching `(kind, namespace, slug)` before pagination. Exact strategic reference
resolution still uses the pinned versions, including older Subdomain versions.
The flag does not retarget references or replace an explicitly selected Domain
with a newer Domain version.

Pages use native `(kind, namespace, slug, version)` ordering. A page token
continues the same query; keep the Domain ID, filters, branch and authorization
context unchanged. Independent requests do not establish a historical
transaction snapshot if the model changes between pages.

## Branch and client scope

Queries use the current authorized branch view. Branch-only entities appear
only in that branch, and hidden namespaces cannot contribute returned entities
or realization paths.

Branch visibility is checked independently of namespace visibility. An
inaccessible branch returns `NOT_FOUND`, even when it contains entities in a
namespace the caller may otherwise read.

Resolution uses the existing capped model snapshot. An omitted intermediate
record can prevent a realization chain from resolving. Empty results or an
exhausted page token therefore do not establish whole-store completeness.

MCP `list_entities_by_domain` requires an explicit positive Domain version and
defaults `latest_versions_only` to true. Studio's current `/api/by-domain`
route selects Domain version `1`; this backend query contract does not add a
version selector to that route. `ValidateProject` has a separate scope path
and is unchanged by this correction.

## Executable coverage

The [public transport regressions](../../rsworkspace/crates/trogon-atlas-server/tests/domain_query_membership.rs)
exercise exact Domain/Subdomain versions, context-specific realizations,
multiple and dangling realization paths, deduplication, latest matching
versions before filtered pagination, branch overlays, foreign-branch refusal
and owner visibility.
They use supported writes and authenticated gRPC against a real NATS Store.
Their presence alone is not a recorded passing run or proof of every
strategic-model acceptance case.
