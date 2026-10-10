# Playwright e2e

## Why this suite exists

294 vitest (jsdom) tests were green while the app was broken in a real
browser: a stale in-tab ES module of `shared/safe-namespace.mjs` threw
`doesn't provide an export named 'isSafeBranchName'` at runtime. jsdom
never instantiates a real browser module registry and never reloads a
page, so no amount of unit-test coverage under vitest can catch this bug
class. This suite exists specifically to catch it: `smoke.spec.ts`'s first
test fails on *any* console error at page load, which is exactly the
symptom this class of bug produces.

Run with `pnpm test:e2e`. It is **not** part of `pnpm test`: keep the unit
gate fast; run e2e separately (locally or as its own CI job).

## Harness design: real bridge + real Vite + a hermetic fake gRPC backend

Two topologies were on the table:

1. **Mock at the browser boundary** (`page.route()` intercepting `/api/*`
   before it leaves the page). This is what `overview-to-board.spec.ts`
   already did before this suite existed. It's cheap and still useful as a
   fast routing/navigation check, so it stays. But it never boots the real
   bridge process and never exercises the real Vite dev-server proxy, so it
   would **not** have caught the stale-module bug (the browser never loads
   the real module graph paths the bug lived in, since Vite never serves
   any of it when every fetch is intercepted at the page level).

2. **Real bridge (`server/index.mjs`) + real Vite dev server, backed by a
   real gRPC service**. This is what `smoke.spec.ts` does. The studio's
   own dev script (`pnpm dev` / `concurrently -k "node server/index.mjs"
   "vite"`) already runs exactly these two processes; e2e reuses that
   topology unchanged. The only question was what to put behind
   `TROGON_ATLAS_GRPC`.

For the backend, the options were:

- **The real Rust `trogon-atlas-server`**, the way `trogon-atlas-testsupport`
  boots ephemeral NATS via testcontainers for Rust integration tests. Ruled
  out: it needs a multi-minute `cargo build` before the first browser test,
  and it would add a Docker/testcontainers dependency to a JS test suite for
  a browser-level smoke check that doesn't need real persistence semantics.
- **The docker-compose dev stack, read-only + a disposable branch**. Ruled
  out: gating e2e on the developer's live data would make the suite
  non-hermetic and CI-unfriendly.
- **A small fake gRPC server implemented in Node**, using `@grpc/grpc-js`'s
  server APIs loaded through the same `service.proto` bundle the bridge
  already uses (`@grpc/proto-loader`, same options). This is what
  `fixtures/fake-grpc-server.mjs` is. It is wire-compatible with the real
  client the bridge constructs (same proto, same message shapes), starts
  in milliseconds, needs no Docker, and returns fixed fixture data. No
  live/shared data ever enters the picture. This is dependency injection at the
  transport boundary (`TROGON_ATLAS_GRPC` pointed at an ephemeral loopback
  port) since the bridge has no in-process seam for swapping its gRPC client.

`fixtures/run-e2e-stack.mjs` is the Playwright `webServer` command: it
starts the fake gRPC server first (to learn its OS-assigned port), then
spawns the real bridge and the real Vite dev server as child processes
with `TROGON_ATLAS_GRPC` set to that port. Nothing about the bridge or Vite
config is test-only: the only thing this script controls is which
backend they're pointed at, via the same env var used in every other
deployment.

One environment quirk found while wiring this up: spawning `vite`
directly (not through a shell/npm script) on this machine binds Vite's
default `localhost` host to the IPv6 loopback only, which the bridge and
Playwright's `baseURL` (both `127.0.0.1`) can't reach. The launcher passes
`--host 127.0.0.1` explicitly to avoid this.

### Fixture data

`fixtures/fake-grpc-server.mjs` serves one `EventModel` (`shop/order-service`,
one `Event` member) and one branch (`e2e/smoke-fixture`) with a
single-entry diff (`STATUS_CHANGED`, one conflicting field) so the
conflict drawer has something to render. The bridge itself never exposes
a create-branch HTTP route: `CreateBranch`/`ListBranches`/`DeleteBranch`
are exempt from branch-context resolution and the bridge only proxies the
read side (`GET /api/branches`, `GET /api/branch-diff`), so "a branch
created via the bridge API" in practice means the fixture backend already
has one when the bridge asks for the list, standing in for a branch a
human or CI job created out-of-band before the suite runs.

## Specs

- `smoke.spec.ts` (the real-stack suite):
  - (a) Overview loads with zero console errors.
  - (b) a model board URL (`/em/<ns>/<slug>`) loads and its sticky renders.
  - (c) `?branch=` renders the BranchBar chip + read-only-preview hint.
  - (d) the branch picker lists the fixture branch and switches to/from it.
  - (e) the conflict drawer opens from the Review button and shows the
    fixture's diff entry.
- `overview-to-board.spec.ts`: the original browser-route-mocked navigation
  check (Overview → board). Kept as-is: cheap, still a valid regression check
  for the click-through, complementary to the real-stack suite above.

## The `shared/safe-namespace.mjs` serving path (structural fix investigation)

Investigated whether the stale-module bug points at something structurally
wrong in how `shared/safe-namespace.mjs` is served, and concluded **no
code change is warranted**:

- The client already imports it through Vite's module graph like any other
  source file: `src/components/Inspector.tsx`, `src/components/
  ModelShell.tsx`, and `src/lib/branch.ts` all `import ... from
  '../../shared/safe-namespace.mjs'` with a plain relative path. `shared/`
  sits inside the Vite project root (no custom `root`, no `publicDir`, no
  `server.fs` restriction in `vite.config.ts`), so Vite transforms and
  HMR-versions this file exactly like anything under `src/`; it is not
  served as a loose static file and does not bypass Vite's graph.
- The server side (`server/index.mjs`) imports the same file directly as
  plain Node ESM (no bundler involved there at all; it's a long-running
  Node process, not a browser tab).
- There is no `public/` directory and no duplicate static route serving
  `shared/` (the only `express.static` mount in `server/index.mjs` is
  `dist/`, the production build output, which bundles this module like any
  other import).
- The production build (`vite build`) bundles `shared/safe-namespace.mjs`
  into the built assets; it is never shipped as a standalone file to
  browsers.

Given all of that, "stale in-tab module" is a real but non-structural dev-
server transient (a long-lived tab holding a previous module instance
across an HMR update that changed the file's export shape): exactly the
class of bug a real-browser e2e check catches and a code change to the
import path would not prevent. `smoke.spec.ts`'s zero-console-error
assertion at page load is the mitigation: it fails loudly the moment any
future change reintroduces a load-time module error, in a real browser,
on every run.
