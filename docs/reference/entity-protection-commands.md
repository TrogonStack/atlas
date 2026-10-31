# Entity protection commands

The `trogon-atlas protection` subcommands and the MCP tools that call the
protection RPCs. [Entity protection](../explanation/entity-protection.md)
explains what protection does and why.

An identity is written `<kind>/<namespace>/<slug>` (or
`<kind>:<namespace>/<slug>`), for example `event/orders/placed`, the same
form refusals name it by. A trailing `@version` is refused: protection
covers every version of the identity.

## CLI

| Command | RPC | Role | Exit codes beyond the failure categories |
| --- | --- | --- | --- |
| `protection protect <identity> --reason <text> [--validate-only]` | `ProtectEntity` | `admin` | none |
| `protection unprotect <identity> [--if-match <revision>] [--validate-only]` | `UnprotectEntity` | `admin` | none |
| `protection get <identity>` | `GetEntityProtection` | `reader` | `11` when the state is unknown |
| `protection list [--namespace <ns>]` | `ListEntityProtections` | `reader` | `11` when the listing is partial |

Failures exit by category, as every other command does; see
[exit codes](trogon-atlas-exit-codes.md). The cases specific to protection:

| Situation | Exit | `failure.code` |
| --- | --- | --- |
| A write (`apply`, `delete`, ...) targets a protected identity | `4` | `ENTITY_PROTECTED` |
| A write targets an identity whose protection could not be read | `75` | `PROTECTION_STATE_UNKNOWN` |
| `unprotect --if-match` names a revision the row no longer has | `3` | `STALE_PRECONDITION` |
| A non-admin protects or unprotects | `7` | none |
| `protect` or `unprotect` with `--branch` | `5` | none |

`protect` and `unprotect` refuse `--branch` (and `TROGON_ATLAS_BRANCH`)
before connecting, because the server refuses a branch header on both.
`get` and `list` ignore the branch: protection is the same on every branch.

`protect` and `unprotect` first check that the server advertises
`GetServerInfo.features.entity_protection`, and exit `8` when it does not.

## MCP tools

| Tool | RPC | Role | Annotations |
| --- | --- | --- | --- |
| `protect_entity` | `ProtectEntity` | `admin` | not read-only, not destructive, idempotent |
| `unprotect_entity` | `UnprotectEntity` | `admin` | not read-only, destructive, idempotent |
| `get_entity_protection` | `GetEntityProtection` | `reader` | read-only |
| `list_entity_protections` | `ListEntityProtections` | `reader` | read-only |

`tools/list` offers each tool only to callers whose role is at least the one
listed. Arguments:

| Tool | Arguments |
| --- | --- |
| `protect_entity` | `kind`, `namespace`, `slug`, `reason`, optional `validate_only` |
| `unprotect_entity` | `kind`, `namespace`, `slug`, optional `if_match` (positive row revision), optional `validate_only` |
| `get_entity_protection` | `kind`, `namespace`, `slug` |
| `list_entity_protections` | optional `namespace` |

`kind` is snake_case, as in every other tool. A refusal arrives as an MCP
error whose `data` carries `code`, `category` and `next_action`; an
`ENTITY_PROTECTED` refusal has `next_action: "request_unprotect"`.

Tool profiles are not in this base yet. When they land, the read tools
belong with the other model reads (`area` model, effect read), and
`protect_entity` and `unprotect_entity` belong to the administration area,
which no profile includes.

## Output

The CLI's `--format json` `data` and each tool's text body carry the same
object.

### Protection row

| Field | Meaning |
| --- | --- |
| `identity` | `<kind>/<namespace>/<slug>` |
| `kind`, `namespace`, `slug` | The identity's parts. |
| `reason` | Why it is protected. |
| `protected_by` | The admin principal that set it. |
| `protected_at` | When it was set. Empty in a `validate_only` answer. |
| `changeset_id` | The changeset that recorded it. Empty in a `validate_only` answer. |
| `revision` | The row revision, for `--if-match` / `if_match`. |

### Unknown row

| Field | Meaning |
| --- | --- |
| `identity`, `kind`, `namespace`, `slug` | As above. |
| `cause` | Why the row could not be read. |
| `revision` | The unreadable row's revision, or `null` when the registry did not answer. |
| `liftable` | `true` when `revision` is set, so `unprotect --if-match <revision>` removes the row. `false` means retry later. |

### Per command

| Command / tool | Fields |
| --- | --- |
| `protect` / `protect_entity` | identity fields, `changed`, `validate_only`, `protection` (a protection row). `changed: false` means the identity was already protected and `protection` is the existing row. |
| `unprotect` / `unprotect_entity` | identity fields, `changed`, `validate_only`, `changeset_id` (`null` when nothing changed or for `validate_only`). |
| `get` / `get_entity_protection` | identity fields, `state` (`protected`, `unprotected` or `unknown`), `protection` (a row or `null`), `unknown` (an unknown row or `null`). |
| `list` / `list_entity_protections` | `namespace` (`null` for all), `verdict`, `complete`, `protections`, `unknown`. |

`verdict` is `partial` when `complete` is `false` or `unknown` is not empty,
and `complete` otherwise. A partial listing is not the whole registry: an
identity missing from it may still be protected. `complete` is the server's
flag, passed through unchanged.

A `get` whose answer from the server has no recognizable `state`, or claims
`protected` without a row, reports `unknown` rather than `unprotected`.

Text output prints one line per row and an `incomplete:` line for each
reason the listing is partial.
