# trogon-atlas Helm chart

Umbrella chart deploying the event model stack to Kubernetes:

- `server` (alias for trogon-atlas-server): the gRPC service (port 50069)
  with an optional Prometheus metrics endpoint (port 9100).
- `studio` (alias for trogon-atlas-studio): the web UI and gateway
  (port 80), reaching the server over gRPC.

Both are standalone charts under `../trogon-atlas-server` and
`../trogon-atlas-studio`; this chart wires them together with local
`file://` dependencies and the `server.enabled`/`studio.enabled`
conditions. Either can also be installed on its own, without this chart.

NATS JetStream is a required external dependency and is not installed by
this chart. Point `server.config.natsUrl` at an existing NATS installation
with JetStream enabled.

## Quick start

```bash
helm dependency update devops/helm/charts/trogon-atlas
helm install trogon-atlas devops/helm/charts/trogon-atlas \
  --set server.config.natsUrl=nats://my-nats:4222 \
  --set server.auth.token.value=change-me
```

`helm dependency update` resolves the `file://` dependencies into
`charts/*.tgz`; it needs rerunning whenever a subchart's `Chart.yaml`
version changes. The `charts/` directory and `Chart.lock` it produces are
gitignored, not vendored.

## Aliases

Every value under `server.*` is trogon-atlas-server's own top-level
`values.yaml`, addressed through the `server` alias; `studio.*` is the
same for trogon-atlas-studio. See each subchart's README for its full
value reference:

- [trogon-atlas-server](../trogon-atlas-server/README.md)
- [trogon-atlas-studio](../trogon-atlas-studio/README.md)

`server.enabled` and `studio.enabled` (both default `true`) toggle whether
each is installed at all.

## Studio's token is no longer implicit

A subchart cannot see a sibling subchart's values, so this chart can no
longer fall back studio to the server's shared token Secret the way the
single-chart version did. When the server authenticates callers, studio
now requires its own `studio.auth.token.value` or
`studio.auth.token.existingSecret`; the umbrella refuses to render
otherwise, with a message naming both the value to set and the env var it
feeds. To share the server's token Secret, point
`studio.auth.token.existingSecret` at the same Secret
`server.auth.token.existingSecret` or `server.auth.tokensFile.existingSecret`
uses.

## Notable values

| Value | Default | Purpose |
|---|---|---|
| `global.imageRegistry` | `ghcr.io/trogonstack` | prepended to image repositories in both subcharts |
| `server.enabled` | `true` | install trogon-atlas-server |
| `studio.enabled` | `true` | install trogon-atlas-studio |

See the subchart READMEs linked above for every `server.*` and `studio.*`
value.
