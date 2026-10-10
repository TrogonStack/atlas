# Wire compatibility and mixed-version operation

The trogon-atlas CLI, the MCP adapter, the Studio gateway and the server ship from one tree,
but they are not upgraded at the same instant. An agent's MCP adapter can be a
release ahead of the server it talks to, or a release behind. This page
explains what keeps those combinations safe: a breaking-change gate on the
protobuf contract, and a discovery handshake that runs before any mutation.

## Why the protobuf contract is checked with the FILE rule set

`buf breaking` offers increasingly strict rule sets: `WIRE`, `WIRE_JSON`,
`PACKAGE` and `FILE`. Each includes the ones before it. The contract uses
`FILE`, configured in `proto/buf.yaml`, because more
than the binary encoding is persisted or exchanged:

- Manifests and the MCP `*_json` tools are proto3 JSON. JSON carries field
  names and enum value names, so renaming either breaks every stored manifest
  and every agent prompt that spells them. `WIRE_JSON` is the minimum that
  catches this.
- Annotations are stored as `google.protobuf.Any`, whose type URL embeds the
  fully qualified message name. Renaming a message or moving it to another
  package strands every stored payload; a prior package rename stranded
  stored payloads this way. `WIRE_JSON` does not catch a message rename;
  `PACKAGE` and `FILE` do.
- Studio loads its vendored copy of the `.proto` files at runtime, file by
  file, and the manifest JSON Schema is derived from the descriptors. Moving a
  type between files changes what those consumers see. Only `FILE` catches
  that.

The cost of `FILE` is that purely cosmetic reorganizations, such as moving a
message into another file, need the intentional-break procedure. That is a
deliberate trade: such moves are rare and the procedure is cheap.

## The baseline

The check compares the working tree against a baseline revision of the same
directory. There are no releases yet, so the baseline is `main`: a pull
request may not break what `main` already serves. CI sets the baseline with
`TROGON_ATLAS_PROTO_BASELINE_REF=origin/main`; locally the script defaults to
`main`.

Once releases exist, the supported released contract becomes the baseline.
Tag each release (for example `trogon-atlas-v1.2.0`), and point
`TROGON_ATLAS_PROTO_BASELINE_REF` at the oldest release still supported, in a
second CI step next to the `main` comparison. Comparing against `main` alone
would let a break land in two steps that each look compatible with the
previous commit but not with what users run.

Intentional breaks are not handled by disabling the check. They are
acknowledged finding by finding in
`rsworkspace/crates/trogon-atlas-proto/acknowledged-breaking-changes.txt`; see
[ship a breaking proto change](../how-to/ship-a-breaking-proto-change.md).

## The discovery handshake

`GetServerInfo` is the one discovery contract. It already reported the schema
label, optional RPCs and limits; it now also reports what a client needs to
know before mutating:

| Field | Meaning |
| --- | --- |
| `schema_version` | Proto package the server implements, `trogonatlas.eventmodel.v1alpha1`. A different value means a different contract. |
| `contract_revision` | Monotonic revision within the schema. Zero means the server predates revisions. |
| `min_client_contract_revision` | Oldest client revision the server accepts mutations from. |
| `features.mutations` | The server accepts writes. A read-only deployment reports false. |
| `features.validate_only` | `validate_only`, `dry_run` and `DRY_RUN` never persist. |
| `features.branch_scoped_requests` | The `x-trogon-atlas-branch` header is honored. |
| `features.state_preconditions` | Lookups report etags and the server enforces the revisions a plan or conflict resolution was made against (see [stale plans](stale-plans.md)). Required by apply and resolve. |

The gating flags exist because proto3 is forgiving in a dangerous way.
A server that does not know a request field drops it silently, and a server
that does not know a metadata header ignores it. A dry run sent to such a
server persists, and a branch write lands on baseline. A server that predates
a flag reports it as false, which is the answer that keeps the client from
sending the request.

The trogon-atlas CLI and the MCP adapter call `GetServerInfo` before every mutation and
refuse with a typed `CompatibilityError` when:

- the schema differs,
- the server's `contract_revision` is below the client's
  `MIN_SERVER_CONTRACT_REVISION` (upgrade the server),
- the client's `CONTRACT_REVISION` is below the server's
  `min_client_contract_revision` (upgrade the client), or
- the mutation needs a capability the server does not advertise.

Reads are not gated, so an agent can still call `get_server_info` and see why
a write was refused. The decision lives in `trogon-atlas-client::compat` so
every surface applies the same rules; the revision constants live next to
`SCHEMA_VERSION` in `trogon-atlas-proto`.

## Supported combinations

| Client vs server | Mutations |
| --- | --- |
| Same release | Allowed. |
| Newer server, client revision at or above the server's minimum | Allowed; new server features stay unused. |
| Newer server that raised `min_client_contract_revision` past the client | Refused; upgrade the client. |
| Older server at or above the client's minimum revision | Allowed for every capability it advertises; others are refused. |
| Server that predates contract revisions | Refused; upgrade the server. |

Upgrade the server before its clients. A server upgrade is always safe for
existing clients unless the release deliberately raised
`min_client_contract_revision`, and that only happens with an acknowledged
breaking change.

The Studio gateway forwards Studio's own requests to the server and does not
mutate on an agent's behalf, so it does not run the handshake. Studio loads
the canonical proto tree directly at runtime, so there is no copy that can
drift.

## When the revision changes

- Add a request field, enum value or header whose silent omission by an older
  server would change what the request does: bump `CONTRACT_REVISION`, add a
  `Features` flag for it, advertise it from the server, and require it in
  `compat::MutationIntent` when the client relies on it.
- Add something an older server may safely ignore, such as a new response
  field or an optional RPC already covered by a feature flag: no bump.
- Make an intentional breaking change: bump `CONTRACT_REVISION` and raise the
  `MIN_*` constants as the how-to describes.
