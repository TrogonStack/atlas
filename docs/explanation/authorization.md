# Authorization Design

Status: Phase 1 implemented (`rsworkspace/crates/trogon-atlas-server/src/auth.rs`, wired in
`main.rs`, composed-stack tests in `tests/auth_stack.rs`). Two further axes
shipped on top of it: namespace scope (`src/scope.rs`) and ownership
(`src/ownership.rs`), the latter with a pluggable backend so the ownership
decision can be delegated to SpiceDB (`src/spicedb.rs`). Ownership also now
extends to branches: an owned branch (`@owner:name`) attributes itself to an
owner, and a principal delegated by that owner (`PrincipalKind::Agent`) may
see and write it without needing an unrestricted token. This documents all
three axes and the boundary of what is deliberately deferred to Phase 2.

There are three independent axes, and a request must clear all of them:

| Axis | Answers | Source | Applies to |
| ---- | ------- | ------ | ---------- |
| Role | "may this caller call this RPC at all" | token file, static | every RPC |
| Namespace scope | "which of the namespaces it can see may it write" | token file, static | writes |
| Ownership | "which namespaces can it see" | namespace registry or SpiceDB, runtime | reads and writes |

They are separate because they answer different questions and are maintained
by different people on different cadences. Role and scope are operator
decisions, written into a file by hand. Ownership is data: a namespace changes
hands through an API call, and requiring a restart for that would make "move
this namespace" an operations ticket.

Neither one needs a restart. The token file is re-read on an interval
(`TROGON_ATLAS_AUTH_TOKENS_RELOAD_INTERVAL`, five seconds by default), so
issuing a key to a new client and revoking one from a departing client are
both edits to a file, applied while the server keeps serving.

Implementation notes that refine the original proposal:

- `DeleteEntity` (single, targeted) requires `writer`, not `admin`; only the
  bulk-destructive `DeleteByQuery` requires `admin`.
- Enforcement wiring: the `BearerAuth` interceptor (authentication) wraps an
  inner tower `AuthzLayer` (authorization), so the Principal is resolved
  before the role check runs. `tests/auth_stack.rs` drives real requests
  through the same composition and fails if the ordering is ever inverted.
- The legacy single-token mode maps to a synthetic `legacy-admin` principal
  and logs a deprecation warning.

## Current state

The server already has authentication, but no authorization:

- A shared bearer token (`TROGON_ATLAS_AUTH_TOKEN` / `--auth-token`) is enforced
  on every RPC by the `BearerAuth` tonic interceptor
  (`rsworkspace/crates/trogon-atlas-server/src/main.rs`), using constant-time comparison via
  the `subtle` crate.
- The server refuses to start without a token unless
  `--insecure-allow-anonymous` (`TROGON_ATLAS_INSECURE_ALLOW_ANONYMOUS=true`) is
  passed explicitly. This escape hatch exists for local development and CI.
- The gRPC health service is mounted outside the interceptor so probes work
  without credentials.

The gaps:

- The token is all-or-nothing. Every caller that holds it has full rights,
  including destructive RPCs (`DeleteByQuery`, `BatchMutate`).
- There is no caller identity. The `x-trogon-atlas-author-name` and
  `x-trogon-atlas-author-email` metadata used for git mirror attribution are
  self-asserted by the client and unauthenticated, so mirror history is
  forgeable by any token holder.

## What this design assumes

Every rule below is enforced by the gRPC server. That makes the server the
only enforcement point, and it makes the store underneath it the trust root:
the ownership registry is just rows in a NATS KV bucket, so anything that can
write that bucket can reassign a namespace to itself and inherit the reads
that come with it. The server picks the change up on its next startup sync,
at which point the forged owner is indistinguishable from a real one.

So the guarantees here hold only while write access to NATS is confined to
the server. Three consequences worth stating plainly:

- The reference compose stack (`devops/docker/compose/dev/services/nats/nats-server.conf`) gives the
  two listeners two different identities, because they have two different
  threat models. The tcp port (4222) runs as `service` with
  `publish`/`subscribe` on `>` and requires a password; only
  trogon-atlas-server holds it. The WebSocket listener (8080, published on
  127.0.0.1 for the studio) is anonymous and lands on `studio_browser`,
  which can watch the `trogon-atlas-entities` bucket and nothing else.

  That split exists because binding 8080 to 127.0.0.1 is not a boundary
  against the machine's own browser. WebSockets do not get the same-origin
  protection XHR does: the browser sends `Origin`, but the server has to
  enforce it. While both listeners shared one `publish: ">"` identity and no
  origin check, any page a developer merely visited could open
  `ws://127.0.0.1:8080` and forge a registry row.

  Two controls now stand in the way, and they cover different attackers.
  `allowed_origins` stops the drive-by case, since a browser cannot forge
  `Origin`. It does nothing against a non-browser client, which simply omits
  the header and is admitted. What constrains *that* attacker is the
  `studio_browser` permission set: no `$KV.>` publish, so no writes, and
  JetStream subjects scoped per stream, so the ownership registry is not
  even readable. Neither control is sufficient alone.

  Use `allowed_origins`, not `same_origin: true`. `same_origin` compares
  Origin against the request Host including port, and the studio is served
  from a different port than NATS, so it rejects the studio along with the
  attacker.
- Reads matter as much as writes here. Before the split, a drive-by page
  could enumerate every namespace and entity in the store, which made this a
  confidentiality problem before it was an integrity one. Multi-tenant
  isolation in the gRPC layer does not survive a client that can bypass the
  gRPC layer, so the browser's grant has to be narrow on reads too, not just
  on writes.
- Delegating to SpiceDB does not move the trust root. SpiceDB is the runtime
  authority for the lens, but its contents are synced *from* the registry, so
  a forged registry row still propagates. SpiceDB narrows who can ask; it does
  not defend the answer's source.

**4222 is now password-protected**, and the reasoning behind that change is
worth keeping. This document previously said the port was unpublished and
therefore only exposed to other containers on the compose network. That was
wrong: OrbStack resolves every container by DNS regardless of what compose
publishes, so `nats.trogon-atlas.orb.local:4222` answers from the host. A probe
against the running stack confirmed it, accepting an unauthenticated CONNECT
and returning the contents of the namespace bucket over `$JS.API.STREAM.INFO`.
The exposure was every process on the developer's machine, not every
container, and "can reach 4222" did mean "is an admin".

The general lesson: an unpublished port is not a boundary, it is an absence of
one convenience. Reachability is a property of the network the container is
on, and something is nearly always bridging that network to somewhere else.

The password is a shared secret between two containers, which is the weakest
form of this that still closes the hole. nkey or JWT accounts remain the
better answer for a deployment where the two sides are not started by the same
compose file, and the change stays confined to `nats-server.conf` and the
server's connection options.

## Phase 1 design

The core principle: **permission rules live in code, token assignments live in
config.** They are maintained in different places, by different people, on
different cadences.

### Roles

Three ordered roles:

| Role     | Grants                                                              |
| -------- | ------------------------------------------------------------------- |
| `reader` | Read-only RPCs: `Get*`, `List*` (including `ListBranches`), `Search*`, `Diff*`, `BatchGet`, projections, references, impact |
| `writer` | Everything `reader` has, plus `PutEntity`, `BatchMutate`, and LLM-backed inference RPCs (they spend tokens) |
| `admin`  | Everything `writer` has, plus `DeleteByQuery`, `MoveNamespace`, and any future destructive RPC |

`RegisterNamespace` requires `writer`: claiming a namespace is ordinary setup
for anyone who can write. `MoveNamespace` requires `admin` because it
reassigns a tenancy boundary, and by definition the caller performing it is
acting across one.

People observe and agents edit. Studio is an observation surface, so its
gateway and every key handed to a person belong to `reader` principals, and
agents hold the `writer` and `admin` keys through the CLI and MCP. The Studio
gateway refuses every mutating request and RPC on its own, but that check
lives in the gateway; the `reader` role is what makes the server refuse the
same key when someone sends it straight to the gRPC port.

### The method map (code, maintained by developers)

A static table in a new `rsworkspace/crates/trogon-atlas-server/src/auth.rs` maps each fully
qualified gRPC method path to the role it requires:

```rust
fn required_role(grpc_path: &str) -> Option<Role> {
    Some(match grpc_path {
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetEntity"     => Role::Reader,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity"     => Role::Writer,
        "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/DeleteByQuery" => Role::Admin,
        // ... every RPC, exhaustively
        _ => return None, // unknown method: request is rejected
    })
}
```

Maintenance rules:

- **Deny by default.** A method absent from the map returns
  `PERMISSION_DENIED` for every caller. A newly added RPC cannot ship
  unprotected; forgetting the map entry fails closed and is noticed
  immediately in development.
- A unit test asserts that every method in the service descriptor has an
  entry, so the map cannot silently drift from the proto.
- The map changes only when the API changes, in the same PR, under normal
  code review. There is no runtime permission store and no admin UI.

### The token registry (config, maintained by the operator)

A TOML file, keyed by principal name, with each principal holding a role and a
list of currently valid tokens:

```toml
# trogon-atlas token registry
# Generate tokens with: openssl rand -hex 32

[principals.studio-gateway]
role = "reader"
tokens = ["9f2c4a8e1b7d3f6a0c5e8b2d4f7a9c1e3b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a"]

[principals.mcp]
role = "writer"
tokens = [
  "1a3c5e7b9d2f4a6c8e0b3d5f7a9c1e4b6d8f0a2c5e7b9d1f3a5c7e9b0d2f4a6c",
  # rotation in progress: remove after the MCP host switches over
  "7e9b1d3f5a8c0e2b4d6f9a1c3e5b7d0f2a4c6e8b1d3f5a7c9e0b2d4f6a8c1e3b",
]

[principals.alex-cli]
role = "admin"
tokens = ["4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a9f2c4a8e1b7d3f6a0c5e8b2d4f7a9c1e"]
```

Parsed via serde with `#[serde(deny_unknown_fields)]`. Validation failures
abort startup with a clear error rather than running misconfigured, and on a
reload they are refused with the previous registry left in force:

- `role` must be exactly `reader`, `writer`, or `admin`.
- The same token string may not appear under two principals (ambiguous
  identity).
- Principal names are restricted to the same safe charset as
  `sanitize_author_field`, because they appear in logs, traces, and git
  mirror commits.
- A principal must declare at least one key, via `tokens` or `token_files`.
  A principal nobody can authenticate as is a typo, and it fails in the
  direction where an operator believes a key is live when it is not.

The token array exists to make rotation a first-class operation: add the new
token, switch the client, remove the old token. Two entries under one
principal during a rotation window is the intended state, not a workaround.

### Keys from a static secret file

`tokens` writes the key inline, which is fine for a dev stack and for tests.
`token_files` names a file holding one key instead:

```toml
[principals.ci]
role = "writer"
token_files = ["/run/secrets/trogon-atlas-ci.key"]
namespaces = ["orders"]
parent = "acme"
```

The two sources are interchangeable and can be mixed under one principal, so
a rotation can move a key from inline to a mount without a flag day. Both are
deduplicated against each other: the same key under two principals aborts
startup either way, and the error names both sources rather than saying only
that a duplicate exists.

This exists because the alternatives leak. An environment variable is visible
to anyone who can describe the running service, and it is what an
orchestrator prints back when asked to. Putting the key inline in this file
means the key and the policy that grants it share a blast radius: the file
has to be readable to be useful, and now reading it hands over the secret as
well as the shape of the deployment. A secret mount separates the two.

Three details that are decisions rather than accidents:

- **A relative path resolves against the directory holding the registry
  file**, not the server's working directory. A registry that means different
  things depending on where the process was started from is a trap, and
  moving the pair of files together keeps them working.
- **A trailing newline is stripped, and nothing else is.** `echo secret >
  file` and every text editor append one, and no operator intends it as part
  of the key. Leading and interior whitespace is preserved, because trimming
  it would silently authenticate a key the operator did not write, and a
  secret is the one value that must never be helpfully rewritten.
- **A missing or empty file aborts startup.** The alternative is a principal
  that exists in the config and authenticates nobody, which looks like a
  working grant right up until someone depends on it.

A file readable by group or others logs a warning rather than refusing to
start, because loose modes are routine inside a container image and a hard
failure there would be noise rather than protection.

### Comparing a presented key

Keys are compared as SHA-256 digests, not as their own bytes
(`TokenDigest` in `auth.rs`).

`subtle`'s slice comparison documents that it short-circuits when the lengths
differ. Comparing raw tokens therefore answers a wrong-length guess faster
than a right-length one, which leaks the length of the real key. Digesting
both sides first makes every comparison examine exactly 32 bytes.

Lookup also iterates the whole registry rather than returning on the first
match, so the time taken does not reveal *which* principal matched, only that
one did.

A digest is the right primitive here and a slow KDF is not: the input is a
high-entropy random token rather than a human-chosen password, so there is no
guessing attack for key stretching to slow down, and a deliberate work factor
would only add latency to every authenticated request.

Two consequences worth naming. The registry stops holding plaintext secrets
once startup finishes, so a stray `Debug` of server state cannot print them;
`TokenDigest` renders as `TokenDigest(redacted)` and is covered by a test.
And `TokenDigest` deliberately does not implement `PartialEq`, so `==` cannot
quietly reintroduce a short-circuiting comparison on an authentication path.

## Namespace scope

Status: **SHIPPED** (`rsworkspace/crates/trogon-atlas-server/src/scope.rs`, enforced in
`service.rs`, tests in `tests/namespace_scope.rs`).

The role answers *whether* a caller may write. It cannot answer *where*: the
gRPC path the `AuthzLayer` keys on carries no namespace, only the request body
does. For a model with several bounded contexts in it, which is the whole
premise of seams, "any writer may write anything" is the wrong default.

A principal may carry a write allow-list:

```toml
[principals.orders-team-agent]
role = "writer"
tokens = ["..."]
namespaces = ["orders", "orders-*"]
```

- **Patterns are exact names or a trailing wildcard.** `orders` matches that
  namespace and nothing else; `orders-*` matches every namespace starting with
  `orders-`, and deliberately *not* the bare `orders`. A pattern that no
  namespace could ever match (`orders/*`, `a b`, a `*` in the middle) aborts
  startup rather than silently never matching.
- **Empty means unrestricted.** A registry written before this field existed
  parses into an empty allow-list and behaves exactly as it did. That is what
  makes the feature opt-in.
- **Scope constrains writes only.** Reading across a seam is what makes a
  neighbouring context useful; a scoped principal that could not read the rest
  of the model could not model against it.
- **Scope is not a role.** An Admin with an allow-list is still bound by it.
  The two axes multiply, they do not override each other.

Enforcement sits in the handlers, at the same boundary that already resolves
branch context, because that is the first place the namespaces are known.

### What it applies to

| RPC | Bound by |
| --- | --- |
| `PutEntity`, `DeleteEntity` | the entity's namespace |
| `BatchMutate` | the union over its ops; one out-of-scope op refuses the whole batch, because a batch is one atomic act |
| `MergeBranch` | the namespaces the merge lands. A branch is a staging area for baseline, so scope is checked at branch-write time *and* at merge time; without the second check a scoped principal could write anywhere by branching first |
| `DeleteByQuery` | its `namespace` filter, when set. Without one the query selects its own victims from the whole store and a scoped principal is refused |
| `RetargetReferences` | nothing can bound it. It rewrites whatever referenced an entity, anywhere, and the set is known only after a scan, so a scoped principal is refused outright |
| `RevertChangeset` | the union over the ops of the changeset being undone. The targets are a fixed list read off a recorded changeset, not a query the caller composes, which is why it is `writer` rather than `admin` |

The last two are the honest edges of the design: an allow-list can only
authorize a write whose reach is known before it happens. Refusing is the
conservative answer, and it is the answer a reviewer can verify.

### Baseline protection is per namespace

The same patterns configure the server side of the seam:

```
--protect-baseline                        # every namespace (unchanged)
--protect-baseline-namespaces=orders,shop-*  # only these
TROGON_ATLAS_PROTECT_BASELINE_NAMESPACES=orders,shop-*
```

Naming namespaces narrows the blanket flag rather than conflicting with it: it
is the more specific statement, so it wins. A namespace outside the list keeps
accepting direct baseline writes, which is what lets a context under review and
a scratch namespace live in one store.

A write whose targets cannot be bounded before it runs (`RetargetReferences`,
an unfiltered `DeleteByQuery`) is treated as protected whenever protection is
on at all: it could land in a protected namespace and nothing can prove
otherwise in advance.

### Where the file lives

The server takes a path via `--auth-tokens-file` /
`TROGON_ATLAS_AUTH_TOKENS_FILE` and only requires that the file be readable.
The contract is deliberately "a file at a configurable path" because every
secrets system can materialize one:

- **Local dev**: not needed; `--insecure-allow-anonymous` keeps `cargo run`
  working with zero setup. To exercise auth locally, put the file anywhere
  (for example `~/.config/trogon-atlas/tokens.toml`) and point the env var at
  it.
- **Docker Compose / OrbStack**: a gitignored file next to the compose file,
  mounted through the `secrets:` mechanism to
  `/run/secrets/trogon-atlas-tokens` inside the container.
- **Production later**: Kubernetes Secret mount, Vault agent template, or
  equivalent writes the same file shape; server code does not change.

The repository must never contain real tokens. It ships only a documented
example (`rsworkspace/crates/trogon-atlas-server/examples/tokens.toml`) with fake values, and
each client stores only its own token in its environment (`TROGON_ATLAS_AUTH_TOKEN`
on the MCP side, the gateway env on the Studio side).

### Request flow

```
request arrives with "authorization: Bearer <token>"
        |
        v
BearerAuth interceptor: look up token in the registry
        -> found: Principal { name, role } inserted into request extensions
        -> not found: UNAUTHENTICATED
        |
        v
authz layer (tower, keyed on URI path): required_role(path)
        -> caller role >= required role: proceed
        -> otherwise, or path unmapped: PERMISSION_DENIED
        |
        v
handler runs; git mirror commits are attributed to principal.name
```

Token lookup keeps constant-time comparison per candidate token. The client
supplied `x-trogon-atlas-author-*` headers are demoted to advisory metadata;
the authenticated principal name becomes the committer identity.

### Operations

The entire runtime maintenance surface is three file edits, none of which
restarts anything:

- **Onboard a client**: generate a token (`openssl rand -hex 32`), add a
  principal (or a token to an existing one), hand the token to the client.
- **Revoke**: delete the token line. It stops working within one reload
  interval.
- **Rotate**: append the new token to the principal's `tokens` array, switch
  the client over, remove the old token.

The reload interval is therefore a revocation latency, and that is the number
to tune it by. See `docs/how-to/issue-an-api-key.md` for the procedure.

## Ownership

Namespace scope bounds the blast radius of a caller you already trust to see
the whole store. Ownership is the other thing: a tenancy boundary, where the
caller must not learn that the rest of the store exists.

A principal may carry a `parent`:

```toml
[principals.acme-agent]
role = "writer"
tokens = ["..."]
parent = "acme"
```

A principal with no `parent` sees everything, which is every principal in a
token file written before ownership existed. That is what makes the axis
opt-in, the same property `namespaces = []` has.

### Namespace id, not namespace name

The registry holds one row per namespace: an immutable `id`, a human `name`,
and the owning `parent`. Two things follow from that split, and they are the
two requirements the design exists to satisfy at the same time:

- **Collision-free between owners.** Two tenants can each have a namespace
  called `orders`; they get different ids, so their entity keys never
  overlap.
- **A move is not a rekey.** Ownership lives in the registry row, not in the
  storage key, so transferring a namespace rewrites one row and touches no
  entity.

The critical detail is that `Id.namespace` on a stored entity is the **id**,
not the name. Every namespace segment in a storage key and every namespace in
a reference is an id too.

The tempting alternative -- names in entities, ids in keys -- breaks quietly.
Two owners who each call a namespace `orders` would produce entities whose
references are textually identical, and every consumer that joins references
by `(namespace, slug, version)` (the reverse index, impact analysis,
validation, the projections) would splice the two tenants' models together.
Holding the id in both places means a reference resolves to exactly one
entity or to none.

The cost is that a minted id (`ns_0199...`, a UUIDv7) is not readable. That
cost is paid at the edges: the CLI, MCP, and Studio resolve a typed name to
an id through the registry before calling.

### Why existing data did not have to move

A namespace that predates the registry adopts its own name as its id. Ids and
names share the same character class, so `orders` is a perfectly legal id, and
every key already written under it stays valid. Minted ids and adopted ids
coexist in one bucket because the id is opaque below the registry.

`trogon-atlas-server backfill-namespaces` writes one registry row per distinct
namespace found in the store, owned by `default`. It is idempotent, and it
refuses to run against a truncated load rather than leave part of the store
unowned and therefore invisible.

### Claim on first write

A tenant with an empty registry has to be able to start writing without an
operator declaring namespaces first. So a write to a namespace with no
registry row claims it for the caller's parent, with `id == name`.

That path is collision-*safe* but not collision-*free*: the second owner to
want the bare id `orders` is refused, because adoption cannot mint. Their
answer is `RegisterNamespace`, which always succeeds and hands them a minted
id. The name-index row is written with a KV `create`, so per-owner name
uniqueness is a storage-level guarantee rather than a read-then-write race.

Rows are written record-first, index-second. A crash in between leaves a
namespace that is unreachable by name but repairable. The opposite order
would permanently claim a name for a namespace that does not exist.

The refusal handed to the second owner does not say who the first one is. It
is tempting to name them, on the reasoning that the caller has already typed
the namespace and so already knows it exists. But existence and ownership are
different facts. A caller who can ask "who owns `orders`?" can ask it for
`acme`, `beta`, and every other plausible name, and assemble a directory of
everyone else on the deployment from a handful of guesses. The caller is told
the name is unavailable and pointed at `RegisterNamespace`, which is all it
needs in order to act. The holder goes to the server log, where the operator
debugging the same refusal can read it.

### Claims made on a branch

A namespace enters the registry on its first write, and that write is either
on baseline or on a branch. The two cannot leave the same trace.

Baseline work is public the moment it lands, so the row naming its owner is
public too and outlives every branch. Branch work is invisible outside its
branch and disappears when the branch is deleted. A row created only to hold
branch work therefore disappears with it: otherwise starting an experiment
and abandoning it would reserve a namespace name globally and forever, which
is exactly what `docs/explanation/branching.md` promises a branch cannot do.

The row does exist while the branch is alive, and it has to. Every read is
filtered through the registry, so a namespace with no row is one its own
author cannot see. Withholding the row until merge would make branch work
invisible to the person doing it, which trades a bookkeeping leak for a
correctness bug.

So a branch write claims *provisionally*, and the claim is settled when the
branch merges or is deleted. Settling asks baseline rather than asking how
the branch ended: if the namespace holds baseline entities the row is made
permanent, and if it holds nothing the row is released and the name is free
again. That one question covers every case without enumerating them. A merge
that landed nothing leaves the namespace as empty as an abandoned branch
does, and a namespace somebody wrote on baseline while the branch was open is
public work whatever the branch went on to do.

Releasing a row also withdraws the grant an external authorizer was told
about. A grant that outlived its row would let its owner keep writing into a
namespace nobody is recorded as owning.

### Reads

Most reads run off a cached snapshot of the store, and those inherit the
filter for free: the snapshot is narrowed once per owner and memoized.

Reads that address the store by exact key do not go near the snapshot, so
each carries its own gate.

The full list of key-shaped reads that carry their own gate:
`GetEntity`, `BatchGetEntities`, `GetEntityHistory`, `DiffEntities`,
`GetLatestVersion`, `GetOutgoingReferences`, `GetSliceProjection`,
`GetStoryboardProjection`, `GetEventModelProjection`, `ValidateEventModel`,
and the unscoped `ListChanges` poll. `DiffEntities` is two key lookups
wearing a diff for a hat; `GetLatestVersion` accepts an empty namespace
meaning "any", so its filter runs on the results rather than on the request.

The last five are the ones worth naming individually, because they read
*both* ways and only one of the two was covered. Each fetches a root entity
by key and then walks its neighbourhood through the lens-filtered snapshot.
The neighbourhood was narrowed correctly, which made the handler look
guarded from the outside and in review: the response held nothing but the
root. But the root is the entity the caller named, and it was returned
whole. A projection shipped another owner's slice with every field on it; a
reference walk described the entity's outbound edges and where they pointed;
validating a model by id loaded that model and reported issues naming its
members. Taking a lensed snapshot somewhere in a handler says nothing about
whether the by-key read in the same handler is gated. They are separate
reads and each needs its own gate.

`GetEntity`, `GetEntityHistory`, and `DiffEntities` answer **NOT_FOUND**, not
PERMISSION_DENIED, for a namespace the caller cannot see. PERMISSION_DENIED
would confirm that the entity exists, which turns the endpoint into an
existence oracle over another tenant's model. `BatchGetEntities` drops
invisible keys before the store call and reports them as `found = false`, so
one out-of-scope key does not fail the visible ones and dense mode stays
positional.

An unregistered namespace is visible only to an unbound caller. Failing
closed matters: a namespace with no row has no owner, and guessing one would
hand a tenant somebody else's data.

### What filtering the rows is not enough for

Narrowing the result set is the easy half of partitioning a read. Several
surfaces compute something else alongside the rows -- a pagination cursor, a
relevance score, a cap on how many entities to consider -- and each of those
is itself a channel a bound caller should not be able to read another
tenant's write volume or content off of, even when the rows never cross the
boundary.

`ListChanges` scans ahead, bounded, past records a bound caller cannot see or
that fall outside its requested scopes, so a poll whose entire window is
invisible to the caller still reaches the end of the log instead of echoing
the same cursor forever. That scan has to advance the cursor past records the
caller was never shown, which is itself a channel: a plain sequence number
handed back in that case would tell an otherwise empty-handed caller exactly
how far other tenants, or data outside its scopes, have been written,
including on the very first poll, when an empty `since_token` resolves to the
store's current tip before any visibility check runs. The cursor stays a
plain number only when the response's own rows already disclosed that value,
or the caller supplied it itself as a plain `since_token` and no further scan
was needed; otherwise it comes back sealed, opaque to the caller and resolved
only for the principal it was minted for.

`SearchEntities` narrows its namespace filter for a bound caller the same way
every other collection read does, but BM25 ranking draws its term statistics
(document frequency, average length) from the whole index they are computed
over. A shared index spans every tenant, so a bound caller's own result score
could still shift when another tenant wrote more matching content, which is a
volume leak with no row ever crossing the boundary. A bound caller is served
off a per-owner index built from its own lens-filtered snapshot instead, so
its scores depend on nothing it cannot see. An unbound caller, who can already
see everything, keeps the shared index.

`GetLatestVersion` accepts an empty namespace meaning "any of the caller's
own", and applies its entity cap to the store call before the per-record
visibility filter runs. Handing the store an unconstrained namespace filter
for that call means another tenant's entities, sorted ahead of the caller's
own, can exhaust the cap before the caller's entity is ever reached -- which
is truncation standing in for a leak, since visibility is consulted too late
to matter. The namespaces handed to the store call are the caller's own
visible set, never "every namespace", so the cap can only ever be exhausted by
entities the caller could see anyway.

### A delete's gate may see what its response may not

`DeleteEntity`'s `FailIfReferenced` check narrows its referrer scan to the
caller's own lens-filtered view, same as every other read -- which is correct
for the list of referrers the response reports, and wrong for the gate that
decides whether the delete is allowed to proceed at all. A referrer the
caller cannot see is still a referrer: letting the delete through because it
happens to be invisible would silently break another tenant's reference. The
gate therefore checks an unrestricted index for existence alone, while the
`referrers` field in the response stays scoped to the caller's own lens, so
passing the check never tells a bound caller anything about who else
references the entity, only that something does.

### Owner-delegated principals

Namespace tenancy asks one question of `parent`: does it equal the
namespace's owner. Branch delegation asks a related but different one, so it
is not reused as-is: a `PrincipalKind` on each principal (`User` or `Agent`,
default `User`) decides how `Principal::authorizes_owner(owner)` is answered.
A `User` is the owner acting as itself, matched by its own name. An `Agent`
holds no identity of its own here; it matches only when the owner it names in
`parent` delegated to it, the same field namespace tenancy already consults,
read through a different lens. Delegation is per owner, never per namespace
or per branch: an agent named the same as an owner still needs that owner's
`parent` to match, and an agent bound to one owner never authorizes another
no matter what it is called.

### Owned branches: filtered reads, delegated writes

A changeset and a classic/global branch each describe one unit of work that
may span several owners, and neither carries an owner of its own. Stripping
the other owners' rows out would describe a change that never happened;
returning them whole would leak. So a baseline changeset and a classic branch
stay exactly as invisible to a bound caller as before: `ListChangesets` and
`ListBranches` filter them out of the page rather than erroring, `GetChangeset`
and `DiffBranch` answer `NotFound` for one, and every classic branch mutation
(`CreateBranch`, `UpdateBranch`, `DeleteBranch`, `ResolveBranchEntry`,
`MergeBranch`) still refuses a bound caller outright with `PermissionDenied`.
`RevertChangeset` refuses unconditionally, because a changeset carries no
owner at all for anything below to attach to.

An *owned* branch is different: its name is `@owner:name`
(`OwnedBranchName` in `src/ownership.rs`), so it carries its owner the way a
namespace carries its `parent`, without needing a registry row the way a
namespace does. `branch_visible` (reads) and `authorize_branch_write`
(writes), both in `src/service.rs`, consult it instead of refusing outright:

- An unrestricted principal (no `parent`) sees and writes every branch,
  exactly as before owned branches existed.
- A bound principal sees, and may create, update, resolve, delete, or merge,
  only an owned branch whose owner it authorizes. A classic/global branch, or
  one owned by someone else, stays filtered out of a list, `NotFound` for a
  direct read, or `PermissionDenied` for a write, exactly as it was before
  owned branches existed.
- `MergeBranch` lands real namespace writes, so owning the branch only
  clears who may propose a merge. Every namespace the merge diff touches
  still has to clear the merging principal's own write scope
  (`require_namespace_scope`), the same check a direct branch-scoped
  `PutEntity` already runs. The refusal names the first namespace outside
  that scope, and nothing in the diff lands, not even the part that was in
  scope.

`StreamChanges` is refused for a different reason: a subscription outlives
the registry view it would be filtered with, so a namespace moved away
mid-stream would keep feeding its former owner until the connection dropped.
`ListChanges` re-reads the registry on every poll and has no such window,
so bound callers use it instead.

A changeset remains the honest answer to "not partitioned yet", not the final
one. Giving it an owner of its own, the way an owned branch already has one,
is what would lift the remaining restriction.

### Moving a namespace

`MoveNamespace` rewrites the parent on the registry row and re-points the
name index. The id is untouched, so no entity moves. A move onto a name the
destination owner already holds is refused and leaves the source intact,
because resolution would otherwise be ambiguous.

The registry is cached behind a single-flight loader and invalidated on every
registry write, so a move takes effect on the next request rather than on the
next restart.

That invalidation only reaches the process that served the move, which is the
whole story on one server and none of it on two. The cached view therefore
also expires, on `TROGON_ATLAS_NAMESPACE_REGISTRY_RELOAD_INTERVAL` (five seconds
by default, `0` to pin it), which bounds how long a replica that did not serve
the move keeps answering with the previous owner. Like the token interval, it
is a revocation latency rather than a polling knob: raising it raises the
window in which two replicas disagree about who owns a namespace.

The per-owner read caches are bounded differently, because a bound in seconds
would be the wrong shape for them. Each caches the entity list an owner may
see, and is keyed by the visibility decision that produced it, so it survives
exactly as long as that decision does and no longer. That matters most under
SpiceDB, where a grant can be revoked by a writer this server never observes:
keyed by owner alone, the entities kept being served after the lens had
already stopped admitting them.

## Delegating ownership to SpiceDB

The registry answers "who owns this namespace" with a single column, which is
enough for a tenancy boundary and not enough for anything else. There is no
way to spell "acme's `catalog`, readable by beta", or "read but not write", or
"this division belongs to that group", because each of those needs a
namespace to be reachable from more than one place and the column holds one
value.

Setting `--spicedb-endpoint` moves that one decision, and only that one, to
SpiceDB.

### What moves and what does not

The registry stays the source of truth for whether a namespace exists, what
its id is, and what its human name is. Those are storage facts, and a
permission system is the wrong place to keep them. Concretely, `may_write`
still asks the registry first, because "no grant" and "no such namespace" lead
to opposite actions: refuse the write, versus claim the namespace for the
caller. Asking SpiceDB alone would collapse the two and break claim-on-first-
write.

The seam is one trait, `NamespaceAuthorizer` (`src/ownership.rs`), with five
methods: `admits` (may this caller see this namespace), `lens` (which
namespaces may it see), `may_write`, and the two notifications `on_registered`
and `on_moved`. `RegistryAuthorizer` implements it by reading the registry and
is what runs when no endpoint is configured. `SpiceDbAuthorizer` implements it
by asking SpiceDB. No RPC handler knows which one it has.

### The resource is the namespace, never the entity

A check per entity would be thousands of round trips on one `ListEntities`,
and it would buy nothing, because the filter is per namespace either way. So
a read resolves the caller's entire namespace set once, with a single
`LookupResources`, into a synchronous `Lens` that the per-record filters
consult without touching the network. A write is one `CheckPermission`.

### The schema

```zed
definition user {}

definition organization {
    relation parent: organization
    relation member: user

    permission membership = member + parent->membership
}

definition namespace {
    relation parent: organization
    relation viewer: user | organization#member
    relation editor: user | organization#member

    permission edit = editor + parent->membership
    permission view = viewer + edit
}
```

`view` is derived from `edit` rather than granted separately, so it is not
possible to leave somebody able to write a namespace they cannot read. That
is a mistake which is easy to make in data and impossible to make here.

`organization#parent` is recursive, which is the nested-owner case Phase 2
listed as deferred: a member of the parent organisation reaches everything its
children own, and membership does not travel upwards.

### The subject is the principal, not the owner

If the subject were the owner, SpiceDB would hold exactly the same
one-owner-per-namespace mapping the registry already holds, and none of the
above would be expressible. So the subject is `user:<principal>`, and
`Visibility` carries the principal alongside the owner for the authorizer to
choose from.

Which principal belongs to which organisation still comes from the token file,
because that is where identity lives. `reconcile` copies it across as
`organization:<parent>#member@user:<principal>`, on every server start and
again after every reload that changes the file.

It replaces rather than adds. For each principal the file names, the old
memberships are deleted before the declared one is written, so editing a
`parent` actually moves that principal instead of granting it both
organisations. The delete is scoped to one subject and to the `member`
relation, which is what keeps it from touching anybody else's membership or
any share.

Two things survive a sync untouched. A principal deleted from the file outright
keeps its membership edge, because reconciliation never learns the name to
delete; that is harmless, since authentication happens first and a principal
with no token never presents an identity for the edge to match. And
`namespace#viewer`, `namespace#editor` and `organization#parent` are never
read or written here at all, so a share or an organisation hierarchy granted
directly against SpiceDB is not at risk from a restart.

Namespace grants are only ever added. A move already deletes its old grant in
the same transaction that writes the new one, so reconciliation has nothing to
clean up there, and deleting first would blind every caller for as long as the
re-write took.

### Object id encoding

SpiceDB object ids allow `[a-zA-Z0-9/_|\-=+]`; namespace and owner ids allow
`[A-Za-z0-9_.-]`. The alphabets differ in exactly one character, so the
encoding is the identity for every id without a dot, and `orders.v2` becomes
`orders=2Ev2`. `=` escapes itself, which is what keeps the mapping injective:
without that, an id literally spelled `orders=2Ev2` and an encoded `orders.v2`
would be the same object and would share one set of grants.

Principal names are arbitrary strings, so the same encoder handles any byte.

### Ordering, and the window it leaves

A namespace is registered before its grant is written, because the id the
grant refers to does not exist until the registry mints it. Between the two,
the namespace exists and nobody can see it. That fails closed, and it is
repairable twice over: the failing RPC returns an error so the caller retries
(`RegisterNamespace` is idempotent precisely so that retry is available), and
`sync-spicedb` re-derives every grant from the registry.

### Freshness

`--spicedb-freshness=minimize-latency` (the default) reads from whichever
replica answers first, floored by this process's own writes, so a caller
always sees a grant this server just made. It can still miss, for the length
of SpiceDB's revision quantization window, a grant some other process wrote.
That is the trade every Zanzibar deployment makes.

`fully-consistent` quorum-reads every check and declines the trade. It is not
merely a stronger default for the first request: an at-least-as-fresh floor
permits any snapshot at or after the floor, so it cannot substitute for full
consistency once a write has happened.

### Failure is not denial

Every SpiceDB error surfaces as `UNAVAILABLE` or `INTERNAL`, never as an empty
lens. An authorization service that cannot be reached has not said no, it has
said nothing, and reporting nothing as a denial would turn a SpiceDB outage
into what looks, from the caller's side, like total data loss.

The one exception is an object id in a lookup result that does not decode back
to a `NamespaceId`. That can only be a relationship somebody wrote by hand
against something this server could never have produced, so it is logged and
dropped: the cost is that one grant, where failing the request would cost the
caller every other one.

### Operating it

```
trogon-atlas-server --spicedb-endpoint=http://spicedb:50051 \
                  --spicedb-preshared-key=... \
                  --auth-tokens-file=/etc/trogon-atlas/tokens.toml
```

The server installs the schema and re-derives every grant at startup, and
republishes memberships whenever the token file changes underneath it;
`--spicedb-skip-startup-sync` turns both off for a deployment where the server
must not write to SpiceDB. Because a sync revokes memberships the file no
longer declares, a server started against a truncated token file will remove
the memberships of the principals it does still list, so that flag is the way
to bring one up without letting it rewrite policy. For the same reason,
replicas must be given the same token file: two servers disagreeing about one
principal's `parent` will overwrite each other, now continuously rather than
only at restart. The `sync-spicedb` subcommand does the same thing without
starting a server, and `--dry-run` prints what it would write.

Publishing is deliberately not a precondition for a reload taking effect. The
token file is the authority on who may authenticate, so a revocation has to
land even when SpiceDB is unreachable; the new snapshot is installed first and
the publish is retried on each subsequent tick until it succeeds. The cost is
that a newly added principal can be authenticated but not yet visible to
SpiceDB, which reads as "sees nothing" for at most one interval, and resolves
itself.

Sharing, per-principal grants, and organisation hierarchies are relationships
this server never writes. They are written directly against SpiceDB, by
whatever owns that policy.

## Caller identity and role-filtered tool listing

`WhoAmI` lets an authenticated caller learn exactly what `BearerAuth` resolved
for it: name, kind (user or agent), role, the namespace write allow-list, and
the owner it is bound to. It is Reader-level, because a principal learning
its own identity is never more privileged than the identity itself, and any
authenticated caller can always ask who it is.

The MCP adapter uses this to narrow what it offers rather than what the
server will accept. On connect, it calls `GetServerInfo`, checks the
`who_am_i` feature flag, and calls `WhoAmI`; the resulting role filters the
tool list it hands back to `list_tools`, so a reader-scoped agent is never
shown `put_entity` or `merge_branch` only to have the server refuse them with
`PERMISSION_DENIED`. A tool the adapter does not yet have a policy for is
treated as admin-only rather than silently listed, so a new tool added
without updating the policy table fails toward hiding it, not exposing it.

None of this moves the enforcement boundary. `AuthzLayer` still authorizes
every RPC on the server, independent of what the MCP adapter chose to list;
a caller that invokes a tool outside its role some other way (a stale tool
list, a hand-rolled client, a cached response) is refused exactly as before.
Tool filtering is a courtesy that spares a well-behaved agent a round trip
it was always going to lose, not a second authorization system.

A server that predates the `who_am_i` feature flag, or a `WhoAmI` call that
fails for any reason, leaves the adapter without a role to filter by. It
degrades to listing every tool, the same menu it offered before role
filtering existed, because a client-side listing miss is never a safety
gap; the server decides what actually executes.

## Decisions and rationale

- **Raw tokens in the file, not hashes.** With a handful of machine clients
  and a file that is already secret-managed, hashing adds ceremony without
  much protection. If the registry file's exposure risk grows, storing
  SHA-256 digests is a small local change to the lookup.
- **TOML over a flat line format.** Nested-but-simple config, free
  validation and line-numbered parse errors via serde, idiomatic in a Rust
  workspace that already carries `deny.toml` and `rustfmt.toml`. The optional
  `namespaces = [...]` scope was added later exactly as anticipated here, with
  no format migration.
- **Static method map over annotations or config.** One place to audit the
  entire permission surface, testable against the service descriptor,
  impossible to change without code review.
- **Restart-to-apply, no hot reload.** Superseded. The original reasoning was
  that token changes are rare and a restart is cheap. The second half turned
  out to be false: this server holds long-lived streams, so a restart is paid
  by every connected client, and pricing key issuance at "everyone
  reconnects" is what makes an operator batch it up instead of revoking
  promptly. The registry is now re-read on an interval.

  The consistency window the original decision worried about is real and is
  handled explicitly: a snapshot is immutable, and reloading swaps a pointer
  to a whole new one, so no request ever observes a half-applied edit. A
  registry that fails to parse or validate is refused and the previous one
  stays in force, because the failure mode of an auth reload must be "keeps
  working", never "denies everybody". Change is judged by what the registry
  grants rather than by mtime or bytes, so a `touch` or a reformat is
  correctly a no-op.
- **NOT_FOUND, not PERMISSION_DENIED, for anything key-addressed; no
  identifying detail in any refusal.** Every point in this document where a
  bound caller is turned away -- a by-key read, a second `RegisterNamespace`
  claim, a `FailIfReferenced` delete -- answers with the same shape: a
  generic refusal that gives the caller nothing to learn about what it
  cannot see. The same doctrine extends past the response rows themselves to
  anything else a handler derives from the full, unfiltered view: a
  pagination cursor, a relevance score, an entity cap. Each must be computed
  from, or gated on, only what the caller may see, never the store's total
  state, or the derived value becomes its own side channel even when no row
  ever crosses the boundary.

## Phase 2 (deferred until a real requirement exists)

- **JWT/OIDC**: signed tokens carrying identity and role as claims, replacing
  the server-side registry. This is also the natural fix for the Studio
  realtime gap: NATS supports JWT auth natively, so the same issuer can mint
  scoped, expiring NATS user credentials for the browser instead of the
  shared `NATS_WS_AUTH_TOKEN`.
- **mTLS** for service-to-service clients (MCP, Studio gateway) to remove
  shared secrets entirely. The server's tonic `tls` feature is already
  enabled.
- **Owned changesets.** A changeset still carries no owner, which is why it
  stays invisible to a bound caller and `RevertChangeset` still refuses one
  outright. Branches already carry an owner (`OwnedBranchName`); extending
  the same attribution to changesets is what would turn the remaining
  refusal into a filter too.
- **Nested owners in the registry itself.** `parent` is a flat id, and the
  built-in `RegistryAuthorizer` compares it for equality, so a caller bound to
  `acme` sees exactly `acme`. Delegating to SpiceDB supersedes this rather
  than deferring it: `organization#parent` is recursive there. Teaching the
  registry its own hierarchy only becomes worthwhile if a deployment wants
  nesting without running SpiceDB.

Phase 1 stops where it does because a role check on `DeleteByQuery` plus
authenticated mirror authorship buys most of the risk reduction, and the
JWT/NATS work only pays for itself once Studio needs per-user identity.
