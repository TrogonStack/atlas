# How to connect an MCP client

Atlas serves MCP over Streamable HTTP at `/mcp` on the server's gRPC port,
once `TROGON_ATLAS_MCP` is set (see the
[server README](../../rsworkspace/crates/trogon-atlas-server/README.md)).
[`examples/mcp-client.json`](../../examples/mcp-client.json) is a client
configuration in the `mcpServers` JSON shape most MCP hosts accept. It keeps
the credential out of the file and starts the session in the `read-only`
[tool profile](../reference/mcp-tool-profiles.md).

```json
{
  "mcpServers": {
    "trogon-atlas": {
      "type": "http",
      "url": "http://localhost:50069/mcp",
      "headers": {
        "Authorization": "Bearer ${TROGON_ATLAS_MCP_TOKEN}",
        "x-trogon-atlas-profile": "read-only"
      }
    }
  }
}
```

## Provide the credential

Issue the client its own key, as in
[Issue an API key to a client](issue-an-api-key.md), and give it the
`reader` role unless it needs to write. Export the key as
`TROGON_ATLAS_MCP_TOKEN` in the environment the host starts from, through
your secret manager rather than a shell profile.

The host expands `${TROGON_ATLAS_MCP_TOKEN}` when it sends each request, so
the file holds only the variable's name. If your host spells environment
references differently, change the reference to its syntax; never replace it
with the key itself, because configuration files get committed, synced and
shared.

Default chosen for review: the example carries only an environment
reference, never a literal credential, and its tests refuse any other form
of the `Authorization` header.

## Install the file

Copy the file to where your host reads MCP servers. Hosts typically read one
of these:

- a JSON file at the root of the project the host opens, shared with
  everyone working in that project;
- a user-level settings file, for servers you use in every project;
- a settings screen that accepts the same JSON pasted in.

Change `url` if the server is not on this machine or listens on another
port. A host name other than `localhost`, `127.0.0.1` or `::1` must also be
in the server's `TROGON_ATLAS_MCP_ALLOWED_HOSTS`, or the server refuses the
request.

Default chosen for review: Atlas ships this one host-neutral file and no
host-specific packaging, so there is a single configuration to keep
correct.

## Check the connection

Restart the host or reload its MCP servers. A working session:

- lists the tools your key's role can call within `read-only`;
- answers `who_am_i` with the role, owner and namespaces the key acts as;
- reports `read-only` as its active profile in the server instructions.

`read-only` lists no task prompts, because every task procedure names a tool
that writes. That is expected.

## Choose a task profile

Default chosen for review: the example starts in `read-only`, so a new
installation can read the model but cannot change it until you choose to
allow it.

For a session that authors models, use a key with the `writer` role and
change the profile header:

```json
"x-trogon-atlas-profile": "authoring"
```

The session then lists the authoring task prompts, such as the storyboard
rules, and fetches the one for its task before it starts. The
[tool profiles reference](../reference/mcp-tool-profiles.md) lists every
profile, and the [MCP tool reference](../reference/mcp-tools.md#task-prompts)
lists the prompts each profile offers.

Keep separate entries, each with its own key and profile, rather than one
entry with the widest profile, so each task runs with only the tools it
needs.
