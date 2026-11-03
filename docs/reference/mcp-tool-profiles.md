# MCP tool profiles

A tool profile narrows the tools an MCP session is offered and may call to
the ones a task needs. Profiles never grant access. When the server advertises
`who_am_i`, a session sees the tools its caller's role permits intersected
with the tools its profile admits. Without `who_am_i`, `tools/list` cannot
apply a role filter and uses only the profile, while the server still
authorizes every call. The
[MCP tool reference](mcp-tools.md) lists the profiles that admit each tool.

## Profiles

Each tool's policy row names its area: discovery (server identity,
catalogues, type resolution), model (entities, references, projections,
validation, history, namespace registration), branch, source, or
administration. Its registry annotations give its effect: read, write or
destructive. A profile is a rule over area and effect, so a new tool joins
the profiles its row and annotations imply.

| Profile | Admits |
| --- | --- |
| `authoring` | Reads in discovery, model and branch; non-destructive writes in model and branch. |
| `repair` | Everything `authoring` admits, plus destructive model and branch writes such as deleting an entity, a batch mutation or merging the branch a repair was validated on. |
| `review` | Reads in discovery, model and branch. |
| `source-integration` | Every source tool, plus reads in discovery and model. |
| `read-only` | Every read outside administration. |

Administration tools (`delete_by_query`, `move_namespace`, `protect_entity`,
`unprotect_entity`) belong to no profile. They are offered only to a session
that runs without one.

## Selecting a profile

| Source | Name | Scope |
| --- | --- | --- |
| Server flag | `--mcp-profile <profile>` | Every session on the server. |
| Environment | `TROGON_ATLAS_MCP_PROFILE` | Every session on the server, when the flag is unset. |
| Request header | `x-trogon-atlas-profile: <profile>` | The request that carries it, when the server sets no profile. |

Without any of these, a session runs without a profile. A server that
advertises `who_am_i` offers every tool the caller's role permits. A server
without `who_am_i` lists every tool because it cannot learn the role; the
backend still authorizes each call.

Default chosen for review: selection precedence is flag, then environment,
then header. A header that names a different profile than the server's is
refused instead of ignored, so a client never runs with more tools than it
asked for without noticing. A header that names the server's own profile is
accepted.

Default chosen for review: an unknown profile name is an error. The server
refuses to start, and a request naming one is refused, rather than falling
back to every tool.

## Enforcement

- `tools/list` applies the profile filter and, when the server advertises
  `who_am_i`, intersects it with the caller's role. Without `who_am_i`, it
  applies no role filter.
- `tools/call` refuses a tool outside the profile with `invalid_params`,
  whether or not the session listed it. The backend authorizes every call
  independently of the listing and profile checks.
- `initialize` reports the active profile in its `_meta` under
  `trogon-atlas/tool-profile` and in a closing line of its instructions.
  A session without a profile carries neither.
