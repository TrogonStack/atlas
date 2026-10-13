# trogon-atlas-semconv

OpenTelemetry metric instruments generated from the semantic convention
registry at [`otel/registry`](../../../otel/registry), the source of truth
for every metric name, unit, and attribute `trogon-atlas-server` and
`trogon-atlas-store` emit.

Regenerate with `mise run otel:generate` after changing the registry. Never
edit `src/semconv` by hand; weaver overwrites that directory on every run.
Validate a registry change with `mise run otel:check` before regenerating.
