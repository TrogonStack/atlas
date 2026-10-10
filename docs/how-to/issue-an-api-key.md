# How to issue an API key to a client

The server authenticates callers against a TOML registry of principals. It
re-reads that registry while it is running, so issuing a key, rotating one,
and revoking one are all edits to a file: no restart, and no client loses its
connection. For why the design looks like this, see
`docs/explanation/authorization.md`.

## Generate the key

Write it straight into its own file, so it never passes through your shell
history or your clipboard:

```sh
mkdir -p /run/secrets
openssl rand -hex 32 > /run/secrets/acme.key
chmod 0600 /run/secrets/acme.key
```

The server logs a warning at startup if the file is readable by group or
others. It still starts, because loose modes are routine inside a container
image.

## Add the principal

In your token registry, alongside the principals that are already there:

```toml
[principals.acme]
role = "writer"
token_files = ["/run/secrets/acme.key"]
parent = "acme"
```

Three fields, three different questions:

- `role` decides which RPCs the client may call at all. `writer` is the
  right default for a client that models: it covers reads plus every
  mutation except the bulk-destructive ones. Use `reader` for a client that
  only consumes.
- `parent` binds the client to an owner. This is the tenancy boundary, and
  it is the field that makes a key safe to hand out: on every read, a
  namespace the client does not own is reported as missing rather than as
  forbidden, so reading cannot be turned into a directory of the deployment.
  Writing is the exception, and it has to be: claiming a namespace by writing
  to it is how a new tenant starts, so a client that tries to claim a bare
  name someone else already holds is told the name is unavailable. It learns
  that the name is taken, never by whom. Minted ids are random and give
  nothing away; only bare names, which are guessable anyway, can be probed
  like this. Every client gets its own `parent` unless you mean two clients
  to share a tenant.
- `namespaces` is optional and constrains writes only, as an allow-list of
  exact names and trailing wildcards. Reach for it to limit blast radius
  *within* one tenant. It is not what separates one client from another;
  `parent` is.

A relative path in `token_files` resolves against the directory holding the
registry file, not the server's working directory, so this also works:

```
/etc/trogon-atlas/
  tokens.toml       # token_files = ["acme.key"]
  acme.key
```

## Confirm it took effect

Nothing to restart. Within one reload interval the server logs:

```
INFO reloaded the token registry path=/etc/trogon-atlas/tokens.toml memberships=3
```

`memberships` counts the principals bound to an owner, which is what gets
republished to SpiceDB. A principal with no `parent` is not in that count.

Then check the key from outside:

```sh
grpcurl -H "authorization: Bearer $(cat /run/secrets/acme.key)" \
  server:50069 trogonatlas.api.eventmodel.v1alpha1.EventModelService/ListNamespaces
```

An empty list is the correct answer for a brand new tenant, and it is not the
same as a failure: a rejected key answers `Unauthenticated`.

If the registry has an error in it, the reload is refused, the previous
registry stays in force, and the server says so:

```
ERROR token registry failed to reload; the previous one stays in force ...
```

That line is the one to look for when a key you just added does not work.
Fixing the file is enough; the next tick picks it up.

## Hand it over

The client needs three things: the endpoint, the key, and the knowledge that
it goes in an `authorization: Bearer <key>` header on every call.

It does not need you to create anything for it. A `writer` bound to a
`parent` can call `RegisterNamespace` to claim its own namespaces, and they
are created under its own owner automatically: the request's `parent` field
is either empty or its own owner, and anything else is refused. Moving a
namespace between owners stays an `admin` operation, because it is the only
one that crosses a tenancy boundary.

## Rotate a key

Add the new key as a second source, switch the client, then remove the old
one. Both are valid during the window, and the two forms can be mixed, so a
key can move from inline to a mounted file without a flag day:

```toml
[principals.acme]
role = "writer"
tokens = ["the-old-inline-key"]
token_files = ["/run/secrets/acme.key"]
parent = "acme"
```

Overwriting the secret file in place also works and needs no registry edit at
all: the reload re-reads every file the registry names, so `openssl rand -hex
32 > /run/secrets/acme.key` rotates that principal on the next tick. That is
the abrupt version, with no overlap window, so use it for a key you believe
is compromised rather than for routine rotation.

The server refuses to load a registry where the same key appears under two
principals, and names both sources in the error.

## Revoke a key

Delete the token line, or the whole `[principals.acme]` block. It stops
working within one reload interval.

That interval is therefore a revocation latency, not just a polling knob, and
it is the number to tune by:

```sh
TROGON_ATLAS_AUTH_TOKENS_RELOAD_INTERVAL=2   # seconds; 0 pins the registry
```

Setting it to `0` restores the old behavior, where the registry is whatever
startup read and a key change requires a restart. The server warns at startup
when you do.

Revocation does not depend on anything but this server: the registry is
re-read locally, and a key is refused as soon as the new snapshot is in
force, even if SpiceDB is unreachable at the time.

## Turn on the reference compose stack's authentication

`devops/docker/compose/dev/compose.yaml` runs anonymous on purpose: it is the local
development front door, and a fresh clone has to come up with no local state
at all. `compose.auth.yaml` is the overlay that turns authentication
on:

```sh
cd devops/docker/compose/dev
auth=../services/trogon-atlas-server/auth
openssl rand -hex 32 > $auth/api.key
openssl rand -hex 32 > $auth/studio.key
openssl rand -hex 32 > $auth/spicedb.key
openssl rand -hex 32 > $auth/nats.key
chmod 0600 $auth/*.key
export TROGON_ATLAS_AUTH_TOKEN=$(cat $auth/api.key)
export TROGON_ATLAS_STUDIO_AUTH_TOKEN=$(cat $auth/studio.key)
export TROGON_ATLAS_SPICEDB_PRESHARED_KEY=$(cat $auth/spicedb.key)
export TROGON_ATLAS_NATS_PASSWORD=$(cat $auth/nats.key)
docker compose -f compose.yaml -f compose.auth.yaml up -d
```

`services/trogon-atlas-server/auth/tokens.toml` is committed; the `*.key` files beside it are ignored, so the keys you
generate never reach the repository.

The keys answer different questions. `api.key` and `studio.key` are callers'
identities: `api.key` belongs to the `local` admin principal the agents use,
and `studio.key` to the `studio` reader principal people use to observe.
`spicedb.key` guards the store those identities are resolved against:
anything that reaches SpiceDB can write itself `organization:default#member`,
which through `parent->membership` is edit on every namespace under it.
`nats.key` guards the data itself, and it is the broadest of them, because
the `service` user is every write in the system and the registries are rows
like any other.

The base compose file runs anonymous and gives SpiceDB and NATS a published
default so a fresh clone comes up with no setup, which is the right trade for
one operator on one machine and the wrong one for anything else. Note that none of these are
protected by not publishing a port: OrbStack answers on container DNS
regardless, so the secret is the whole of the control. The overlay refuses to
start when any of them is unset, and prints which.

The overlay gives the MCP adapter `api.key` and the studio bridge
`studio.key`, because those two are clients of the server rather than peers
of it. Without a key they start returning 401 on every route, and from the
operator's side that shows up as failing health checks rather than as an auth
error, which is the failure mode that costs an afternoon.

They get different keys because agents edit and people observe. The studio
bridge refuses every write itself, and the `reader` role makes the server
refuse them too, so a key copied out of a browser and sent straight to the
server cannot change anything either.

The overlay also puts the studio in pass-through mode, described in the next
section, so open <http://localhost:8788> and paste `studio.key` when it asks.
Anonymous requests are refused, and the key you paste is the identity the
server sees.

## Give different keys to different people

The studio bridge holds one token and, by default, makes every upstream call
as that one principal. Two people looking at the same studio therefore see
the same namespaces no matter which key they hold, because neither key ever
reaches the server: the bridge's does. For a single-operator deployment that
is fine and is why it is the default.

For a shared one it is not, and the bridge stops substituting its identity
when you set:

```sh
TROGON_ATLAS_AUTH_PASSTHROUGH=true
```

The compose overlay above sets it already, on the grounds that a reference
stack should show the posture a shared deployment needs rather than the
shortcut. In the Helm chart it is `studio.auth.passthrough`.

Now the caller's own bearer token is forwarded, the server resolves it to a
principal, and everything already described here applies: role, namespace
scope, and the owner the registry records. Issue one `reader` key per person
and the studio scopes itself. People observe through the studio and agents
make every change through the CLI or MCP, so a key you hand a person should
never carry `writer` or `admin`.

Two consequences worth knowing before you turn it on:

- The bridge stops being an authority. It does not hold the token registry
  and cannot say whether a key is real, so it checks only that one was
  presented and lets the 401 come from the server. `TROGON_ATLAS_AUTH_TOKEN`
  is never the identity a caller is authorized as, and the bridge no longer
  falls back to it if a caller's token goes missing partway through a
  request: that is a bug, not a caller to quietly wave through as itself.
  The one exception is `/api/info`, the single route that answers before the
  auth guard runs and so never has a caller token to forward; GA deployments
  that turn passthrough on still need `TROGON_ATLAS_AUTH_TOKEN` set to a
  `reader` key for that route alone.
- The browser needs somewhere to put a key. It prompts on the first refusal
  and keeps what you give it for that tab only. Closing the tab forgets it.

`/api/info` stays public either way.

The realtime feed moves too, and it has to. The studio normally watches the
entity bucket over a NATS WebSocket, and that listener admits every client on
one shared grant that covers every namespace: keys are
`<kind>.<namespace-id>.<name>.<version>` and nothing scopes the middle
segment. Setting `NATS_WS_AUTH_TOKEN` stops a stranger connecting and still
gives every admitted browser the same total read, so it would hand back
exactly what the key you just issued was meant to withhold. So any bridge
that authenticates its callers answers `transport: "sse"` instead of a
credential, and the studio follows `/api/changes/stream`, which runs through
the server and so shows each person only their own namespaces. The cost is a
poll rather than a push; the feed is otherwise the same.

That covers a bridge with a fixed `TROGON_ATLAS_AUTH_TOKEN` as much as a
pass-through one, which matters if you hand a client a key and they run their
own studio with it: the bridge cannot ask how wide its own key is, so it
assumes narrow rather than route browsers past it.

Giving that listener per-person credentials needs NATS JWT auth, which is
deferred until a public-facing deployment exists. Until then the WebSocket
channel is single-tenant by construction, and pass-through mode routes
around it rather than pretending otherwise.
