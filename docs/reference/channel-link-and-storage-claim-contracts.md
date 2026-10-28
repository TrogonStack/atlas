# Channel link and storage claim contracts

Status: native authoring is implemented for `Channel.links` and
`StorageAccessClaim` on Component, ExternalSystem and DataProduct. This page
is the contract. [Acceptance and executable coverage](channel-link-and-storage-claim-acceptance.md)
lists what has run and what remains.

Both are owner-local, descriptive declarations. They do not establish routing,
delivery, runtime behavior or storage access control.

## Channel links

| Field | Contract |
| --- | --- |
| `connection_id` | Server-assigned occurrence ID. Equal links may repeat and receive distinct IDs. |
| `kind` | `COMPOSES` or `ROUTES_TO`. Required. |
| `target` | Exact typed Channel reference with kind, key and version. A target equal to the source Channel's own exact key and version is rejected. Another exact version of the same slug is selectable. Descriptive cycles between distinct exact Channels are allowed. |
| `parameter_values` | Authored values for the target address. Names must be declared by the target address, values must be in the allowed set, and a target without an address accepts none. Required target parameters reject omission unless the address authors a default. |
| `doc` | Descriptive text. |

The fingerprint is kind and target only. `parameter_values` and `doc` are
descriptive. A Channel keeps at most 1024 links and its owner stays within 1 MiB.
The Channel anchor observation requirement is unchanged.

Writing a link requires write access to the source Channel and read access to
the target. A hidden target and a missing target fail identically.

## Storage access claims

| Field | Contract |
| --- | --- |
| `connection_id` | Server-assigned occurrence ID. |
| `store` | Exact typed DataStore reference. |
| `direction` | `READ` or `WRITE`. These are distinct occurrences, never merged or inferred from each other. |
| `attachment` | Optional ConnectionRef. It must name an attachment of the same store incarnation (owner UID pinned). |
| `port_id` | Optional local DataPortId. Valid on DataProduct only and must select an existing input or output port. |
| `doc` | Descriptive text. |

The fingerprint is store, direction, attachment and port. `doc` is excluded.
The store's declared `access_mode` is not admission authority. Each owner keeps
at most 1024 claims, and a DataProduct also stays within its 4096-association
ceiling. Removing a DataProduct port must remove or replace its dependent claim
in the same guarded mutation.

## Identity, references and budgets

Links and claims are connection occurrences. The link `target` and the claim
`store` are retarget targets, and an attached claim stays pinned. They take part
in provenance, safe delete, retarget, lifecycle, responsibility and
CatalogSubject inventories. A hidden referrer blocks safe delete without
leaking its identity. Occurrence inventories are bounded to 5,000 visible rows,
10,000 examined occurrences and 4,096 emitted records, with incompleteness
reported. Overflow is a typed `RESOURCE_EXHAUSTED`.

## Source ownership

Source create and WholeOwner paths, including clearing, reject proposed native
content under every policy. A title-only update to an existing owner preserves
local links and claims byte for byte. Rejection is atomic.

## Authoring surfaces

The manifest, CLI and MCP preserve UID, etag, occurrence IDs and exact uint64
values across inspect, diff, export and a no-op reapply. Studio is read-only:
it shows exact references, pinned attachment availability and inert text, and
claims no reversal of loaded-only discovery.
