# trogon-atlas-studio Helm chart

Deploys the trogon-atlas-studio UI, a web frontend for browsing and editing
the event model served by trogon-atlas-server.

Installed on its own, every value below drops the `studio.` prefix it would
carry under the `trogon-atlas` umbrella chart; `studio.auth.token.value`
here is just `auth.token.value`.

## Quick start

```bash
helm install trogon-atlas-studio devops/helm/charts/trogon-atlas-studio \
  --set server.host=trogon-atlas-server \
  --set auth.token.value=change-me
```

## Pointing at the server

`server.host` (default `""`) names the trogon-atlas-server Service to talk
to. Left empty, the chart assumes a server installed under the same release
name convention, one `-server` away from this release: a standalone studio
release named `s` talks to `http://s-trogon-atlas-server:50069`, matching
what an umbrella install's `server` alias would produce for a server
release also named `s`. `server.grpcPort` (default `50069`) overrides the
port, and `config.grpcUrl` overrides the whole URL for anything that does
not fit the naming convention (a server in another namespace, TLS, etc).

## Authentication

`auth.token.value` or `auth.token.existingSecret` sets the bearer token
studio uses when calling the server. Studio is observation only: the
bridge refuses every write itself, and a `reader` principal's key makes
the server refuse the same key again if it is ever sent there directly.
Unlike the single-chart version of this chart, there is no implicit
fallback to the server's own shared token Secret: a subchart cannot see a
sibling subchart's values, so that lookup cannot be reconstructed from
here. If you want studio to share the server's token, point
`auth.token.existingSecret` at the same Secret the server's
`auth.token.existingSecret` or `auth.tokensFile.existingSecret` uses.

`auth.passthrough` (default `false`) forwards each caller's own bearer
token to the server instead of the bridge's. Off, every upstream call is
made as one principal, so every browser sees the same namespaces whatever
key it holds; on, the server scopes each person by their own key. Turn
this on for any deployment with more than one user.

When the server is reachable anonymously (`auth.insecureAllowAnonymous=true`
on trogon-atlas-server), studio needs no token and this chart renders
without one.

## The realtime feed follows the credential

Studio can watch the entity bucket over a NATS WebSocket for live updates,
and that listener admits every browser on one identical grant covering
every namespace. So the bridge offers it only when nothing on the
deployment is scoped: the server runs with `insecureAllowAnonymous` and
studio has no `auth.token` configured. Any other configuration gives
studio a credential, and it routes the feed through its own change stream
instead, which the server authorizes per caller. `config.natsWsUrl` is the
browser-reachable WebSocket URL for that anonymous path.
`config.natsWsAuthToken` closes the listener but is never handed to a
browser, so setting it forces that same per-caller routing rather than
enabling the WebSocket. The cost of the change-stream path is a poll
instead of a push.

## Ingress

`ingress.enabled=true` fronts the studio Service with an Ingress; set
`ingress.hosts` and, for TLS, `ingress.tls`. Without it, `NOTES.txt`
prints a `kubectl port-forward` command instead.

## Notable values

| Value | Default | Purpose |
|---|---|---|
| `global.imageRegistry` | `ghcr.io/trogonstack` | prepended to image repositories |
| `image.repository` | `atlas/trogon-atlas-studio` | studio image |
| `server.host` | `""` | server Service name; empty derives `<release>-trogon-atlas-server` |
| `server.grpcPort` | `50069` | server gRPC port |
| `config.grpcUrl` | `""` | full override of the derived server URL |
| `auth.token.value` | `""` | bearer token sent to the server as `TROGON_ATLAS_AUTH_TOKEN` |
| `auth.token.existingSecret` | `""` | read that token from an existing Secret instead |
| `auth.passthrough` | `false` | forward each caller's own bearer token instead of the bridge's |
| `config.natsWsUrl` | `""` | browser-reachable NATS WebSocket URL for the realtime feed; used only under `insecureAllowAnonymous` |
| `config.natsWsAuthToken.value` | `""` | credential closing that listener; forces the per-caller change-stream path instead of enabling it |
| `ingress.enabled` | `false` | front the Service with an Ingress |
| `ingress.hosts` | `[]` | Ingress hostnames |
| `ingress.tls` | `[]` | Ingress TLS blocks |

See `values.yaml` for the full list.
