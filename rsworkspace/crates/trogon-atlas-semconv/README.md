# trogon-atlas-semconv

OpenTelemetry metric instruments generated from the semantic convention
registry at [`otel/registry`](../../../otel/registry), the source of truth
for every metric name, unit, and attribute Trogon Atlas emits.

Every instrument reports the instrumentation scope `trogon-atlas`, shared by
every component that depends on this crate. Tell components apart by the
`service.name` resource attribute, not the scope.

Regenerate with `mise run otel:generate` after changing the registry. Never
edit `src/semconv` by hand; weaver overwrites that directory on every run.
Validate a registry change with `mise run otel:check` before regenerating.
Each codegen target is a line in `.config/weaver/targets` naming the template
target, its params file, and its output directory.
