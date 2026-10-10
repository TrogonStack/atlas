# Studio Gateway Server

Express-based JSON bridge between the browser SPA and the trogon-atlas gRPC service. The browser cannot speak tonic gRPC directly, so this thin server is the only Node-side component.

## Observation only

Studio is an observation surface. Agents make every change, including namespace registration, ownership moves, deletion, and model edits, through the CLI or MCP. The bridge enforces that rather than relying on the SPA not offering a button:

- Every `/api/*` request whose method is not `GET`, `HEAD`, or `OPTIONS` is refused with `403` before its body or credential is read, whatever token it carries and whether or not a route exists for it. The one exception is `POST /api/type-libraries/compile`, a dry run that writes nothing and takes a body only because proto source text does not fit in a query string. It still goes through authentication and rate limiting, and the RPC it forwards is on the read allowlist.
- The bridge forwards only RPCs on an allowlist of reads. An RPC added to the proto later is refused until someone adds it to that list.
- The credential Studio uses upstream must belong to a `reader` principal, so a key copied out of a browser and sent straight to the gRPC server is refused every write there too. See [issue an API key](../../../../docs/how-to/issue-an-api-key.md).

## Routes

| Method | Path | Auth required | Description |
|--------|------|--------------|-------------|
| GET | `/api/info` | No (public health endpoint) | Server version and feature flags |
| GET | `/api/namespaces` | Yes (when token configured) | Distinct namespaces from event model roots |
| GET | `/api/namespaces/registry` | Yes (when token configured) | Namespace registry rows with owner, id, and entity count |
| GET | `/api/branches` | Yes (when token configured) | Branches and their delta counts |
| GET | `/api/branch-diff` | Yes (when token configured) | Diff of one branch against baseline via `?name=` |
| GET | `/api/model` | Yes (when token configured) | Paginated entity list, filtered by `?namespace=` |
| GET | `/api/overview` | Yes (when token configured) | Problem-space entities (read models, event models, bounded contexts, domains, subdomains) |
| GET | `/api/event-models` | Yes (when token configured) | All EventModel entities (index page) |
| GET | `/api/event-model/:namespace/:slug` | Yes (when token configured) | Single EventModel with halo and knowledge graph |
| GET | `/api/search` | Yes (when token configured) | Full-text entity search via `?q=` |
| GET | `/api/changes` | Yes (when token configured) | Change feed page via `?after=` |
| GET | `/api/changes/stream` | Yes (when token configured) | SSE stream of the change feed |
| GET | `/api/validate` | Yes (when token configured) | Validate one EventModel via `?namespace=&slug=` |
| GET | `/api/validate-project` | Yes (when token configured) | Validate all EventModels in a project or domain |
| GET | `/api/entity-kinds` | Yes (when token configured) | Static catalog of all EntityKind values |
| GET | `/api/validation-rules` | Yes (when token configured) | Static catalog of all validation rules |
| GET | `/api/by-domain/:namespace/:slug` | Yes (when token configured) | Entities scoped to a Domain |
| POST | `/api/type-libraries/compile` | Yes (when token configured) | Compiles a type library's source text without saving it; returns diagnostics and compatibility violations |
| GET | `/api/type-libraries/messages` | Yes (when token configured) | Message full names the live type libraries in `?namespace=` declare |
| GET | `/api/nats-auth` | Yes (when token configured) | Returns `{ token: string \| null, transport: "nats" \| "sse" }` naming the realtime channel the caller may use |

The server also serves the built SPA from `dist/` when it exists (container mode). In development, the Vite dev server takes this role.

## Authentication

When `TROGON_ATLAS_AUTH_TOKEN` is set, all `/api/*` routes require a `Authorization: Bearer <token>` header. The comparison uses `crypto.timingSafeEqual` to prevent timing-based token enumeration.

The single exception is `/api/info`, which remains public regardless of token configuration. Load balancers and monitoring probes use this endpoint before authentication is available.

With `TROGON_ATLAS_AUTH_PASSTHROUGH=true`, the bridge stops answering for everyone. The caller's own token is forwarded to the gRPC service, which resolves it to a principal and scopes what that principal may read and write. The bridge does not hold the token registry and so cannot validate a key it did not issue: it checks only that one was presented and lets the 401 come from upstream. `TROGON_ATLAS_AUTH_TOKEN` is never the identity anything is authorized as, and it is no longer a fallback for a caller's missing token: the only route that still uses it is `/api/info`, which answers before the auth guard runs and so never has a caller token to forward. A GA deployment that turns passthrough on still needs `TROGON_ATLAS_AUTH_TOKEN` set to a `reader` key for that one route.

Leave it off for a single-operator deployment, where one bridge identity is exactly right. Turn it on wherever more than one person shares a studio, or every one of them sees the union of what the bridge can see.

`/api/nats-auth` hands out a credential rather than data, so in passthrough mode it asks the server to vouch for the caller before answering: a token that was merely presented is not evidence of anything.

It also names the realtime transport. The NATS WebSocket listener admits every client on one shared, unscoped grant over the whole entity bucket, so a deployment that scopes namespaces per principal cannot use it without undoing that scoping. In passthrough mode the route therefore answers `{ token: null, transport: "sse" }` and the SPA follows `/api/changes/stream`, which polls `listChanges` with the caller's own credential. Otherwise it answers `transport: "nats"` and the browser connects directly, which is correct for a single-operator stack.

In passthrough mode `/api/changes/stream` asks the server to vouch for the caller before it flushes the SSE headers. A status code is only available until the stream opens, so without that a credential the server rejects would reach the browser as a 200 that never delivers anything, instead of a 401 it can prompt about.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `8787` | TCP port to listen on. Must be a positive integer; falls back to default if invalid. |
| `HOST` | `127.0.0.1` | Bind address |
| `PROTO_PATH` | `../../../../proto/trogonatlas/api/eventmodel/v1alpha1/service.proto` | Path to the proto entry file |
| `TROGON_ATLAS_GRPC` | `http://127.0.0.1:50069` | gRPC endpoint of the trogon-atlas server |
| `TROGON_ATLAS_AUTH_TOKEN` | _(empty)_ | Bearer token for the gRPC service. Use a `reader` principal's key. |
| `TROGON_ATLAS_AUTH_PASSTHROUGH` | `false` | Forward each caller's own bearer token upstream instead of this bridge's. See [Authentication](#authentication). |
| `NATS_WS_AUTH_TOKEN` | _(empty)_ | The credential the NATS WebSocket listener requires, if you closed it. `/api/nats-auth` never hands it to a browser: one shared token is one shared permission set, so a bridge that authenticates its callers owes them a per-principal answer, and a bridge that does not would be publishing the credential to the stranger it was meant to stop. Setting it therefore only routes the realtime feed through `/api/changes/stream`, and says so at startup. |
| `TROGON_ATLAS_STUDIO_CORS_ORIGINS` | _(empty)_ | Comma-separated allowed CORS origins. Empty = same-origin only. |
| `TROGON_ATLAS_STUDIO_MAX_PAGES` | `50` | Maximum pagination pages per request |
| `TROGON_ATLAS_STUDIO_GRPC_TIMEOUT_MS` | `15000` | gRPC deadline per call (ms) |
| `TROGON_ATLAS_STUDIO_RATE_WINDOW_MS` | `60000` | Rate limit window duration (ms) |
| `TROGON_ATLAS_STUDIO_RATE_MAX` | `600` | Max requests per IP per window. Set to `0` to disable. |
| `TROGON_ATLAS_STUDIO_SSE_MAX_PER_IP` | `4` | Max concurrent SSE streams per IP |

## CORS Policy

CORS is disabled by default (empty `TROGON_ATLAS_STUDIO_CORS_ORIGINS`). Only `GET` and `OPTIONS` methods are permitted on allowed origins. Set to `*` for fully public read APIs.

## Security Headers

Every response carries: `X-Content-Type-Options`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, `Cross-Origin-Resource-Policy: same-origin`, `Cross-Origin-Opener-Policy: same-origin`, `Permissions-Policy`, and a `Content-Security-Policy` scoped to same-origin assets plus WebSocket connections.
