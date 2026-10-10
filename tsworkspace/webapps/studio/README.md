# Event Model Studio

A visual board for the `trogonatlas.eventmodel.v1alpha1` service: the classic
Event Modeling layout (personas and wireframes on top, the
command/read-model timeline in the middle, one event-stream lane per
swimlane below), rendered live from the gRPC store.

- **Canvas**: `@xyflow/react` board with sticky-note semantics: events
  (amber), commands (blue), read models (green), automation (violet),
  UIs (white), personas (yellow). Columns follow storyboard slice order;
  edges are derived from slices (persona → UI → command → events,
  events → read model → UI, read models → processor → command).
- **Inspector**: entity doc, fields, and slice membership.
- **Search**: server-side Tantivy search.
- **Live**: polls the change feed and refreshes the board when the model
  changes.

## Architecture

The browser cannot speak tonic gRPC, so `server/index.mjs` is a thin Node
bridge: it loads the canonical proto directly from
`../../../proto/trogonatlas/eventmodel/v1alpha1/event_model.proto` with `@grpc/grpc-js` and exposes
JSON endpoints over `/api/`. Every route is read-only: Studio is an
observation surface, and agents make every change through the CLI or MCP.
The bridge refuses any other method with `403` and forwards only read RPCs,
and it should hold a `reader` key so the server refuses writes from that
credential too. The Vite dev server proxies `/api` to it. See [`server/README.md`](./server/README.md) for the
full route table and authentication documentation.

## Run

```bash
# backend: NATS + trogon-atlas-server, see ../../../devops/docker/compose/dev/README.md

npm install
cd webapps/studio

# bridge (terminal 1)
TROGON_ATLAS_GRPC=http://127.0.0.1:50069 TROGON_ATLAS_AUTH_TOKEN=... npm run dev:api

# web (terminal 2)
npm run dev:web   # http://localhost:5179
```

| Env | Default | Purpose |
| --- | --- | --- |
| `TROGON_ATLAS_GRPC` | `http://127.0.0.1:50069` | gRPC endpoint (`https://` enables TLS) |
| `TROGON_ATLAS_AUTH_TOKEN` | unset | bearer token matching the server, from a `reader` principal |
| `PORT` | `8787` | bridge listen port |
