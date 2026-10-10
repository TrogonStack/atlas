// Vendor binding for OpenSLO export. A model `Signal` is a logical pointer
// (event-store timestamps, or a named span/metric); it never carries a
// vendor query. This file is where that query lives, kept outside the
// model so retargeting a metric to a new vendor never touches a
// committed SLI.
//
// `openslo::convert_namespace` looks a `Signal` up here and, for each
// `MetricRole` the indicator's measure needs (see `MetricRole`), copies
// that role's template into the rendered `ratioMetric`/`thresholdMetric`
// query. Each role is its own template owned by this config file, not a
// shared one, because `good` and `total` (or `threshold`) are usually
// different query shapes, not the same query with a different window.
// A template's string scalars may contain `{{events}}` (comma-joined
// event slugs) or `{{within}}` (duration-shorthand), substituted per
// role/call site; see `openslo::substitute_tokens`. A role the measure
// needs but this binding does not supply is an error naming the
// indicator and the missing role; a `{{token}}` still unresolved after
// substitution is an error naming the indicator and the token, never
// emitted into the YAML.

use std::path::Path;

use anyhow::{bail, Context as _, Result};
use serde::Deserialize;
use trogon_atlas_proto as pb;

/// A validated OpenSLO `metadata.name`: lowercase alphanumerics and `-`,
/// starting and ending alphanumeric, at most 63 characters (RFC1123, same
/// rule the spec requires for every kind). Constructing one is the only
/// way to get a value this type can hold, so a `DnsLabel` reaching the YAML
/// writer is never unvalidated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DnsLabel(String);

impl DnsLabel {
    pub fn new(raw: impl AsRef<str>) -> Result<Self> {
        let raw = raw.as_ref();
        let valid = !raw.is_empty()
            && raw.len() <= 63
            && raw
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && raw.starts_with(|c: char| c.is_ascii_alphanumeric())
            && raw.ends_with(|c: char| c.is_ascii_alphanumeric());
        if !valid {
            bail!(
                "{raw:?} is not a valid OpenSLO metadata.name (RFC1123 DNS label: lowercase \
                 alphanumeric or `-`, up to 63 characters, starting and ending alphanumeric)"
            );
        }
        Ok(Self(raw.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DnsLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DnsLabel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        DnsLabel::new(&raw).map_err(serde::de::Error::custom)
    }
}

/// Mirrors the two shapes of `Signal.source` (see `reliability.proto`),
/// used as the lookup key into the bindings file. `EventTimestamps` has no
/// further identity; a `Telemetry` signal is identified by `(kind, name)`.
///
/// Tagged on `source` (matching the proto oneof's own field name) rather
/// than serde's default externally-tagged form, so a bindings file reads
/// as `signal: {source: telemetry, kind: metric, name: ...}` instead of
/// the harder-to-author nested-map shape a default derive would require.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
pub enum SignalSelector {
    EventTimestamps,
    Telemetry {
        kind: TelemetrySelectorKind,
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TelemetrySelectorKind {
    Span,
    Metric,
}

impl SignalSelector {
    /// Builds the lookup key for a model `Signal`. `None` (an SLI with no
    /// signal) and `TELEMETRY_KIND_UNSPECIFIED` are caller errors, not
    /// binding lookups, so they are rejected here with the signal's shape
    /// in the message.
    pub fn from_signal(signal: Option<&pb::Signal>) -> Result<Self> {
        use pb::signal::Source;
        match signal.and_then(|s| s.source.as_ref()) {
            Some(Source::EventTimestamps(_)) => Ok(Self::EventTimestamps),
            Some(Source::Telemetry(t)) => {
                let kind = match pb::TelemetryKind::try_from(t.kind) {
                    Ok(pb::TelemetryKind::Span) => TelemetrySelectorKind::Span,
                    Ok(pb::TelemetryKind::Metric) => TelemetrySelectorKind::Metric,
                    _ => bail!("telemetry signal {:?} has no telemetry kind", t.name),
                };
                Ok(Self::Telemetry {
                    kind,
                    name: t.name.clone(),
                })
            }
            None => bail!("indicator has no signal to bind a DataSource against"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSourceConfig {
    pub name: DnsLabel,
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(rename = "connectionDetails", default)]
    pub connection_details: serde_yaml::Mapping,
}

/// The OpenSLO query role a `metricSource` template fills. `Threshold`
/// backs `thresholdMetric` (a `Latency` measure); `Good`/`Bad`/`Total`
/// back `ratioMetric` (`OutcomeRatio` and `Completion` measures). OpenSLO
/// allows a ratio to carry `good`+`total` or `bad`+`total`, never all
/// three, which is why `Good` and `Bad` are separate roles rather than
/// one "numerator" role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricRole {
    Threshold,
    Good,
    Bad,
    Total,
}

impl MetricRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::Good => "good",
            Self::Bad => "bad",
            Self::Total => "total",
        }
    }
}

impl std::fmt::Display for MetricRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSourceConfig {
    /// Omit when `metricSourceRef` carries the type (OpenSLO infers it from
    /// the referenced `DataSource`).
    #[serde(rename = "type", default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub threshold: Option<serde_yaml::Mapping>,
    #[serde(default)]
    pub good: Option<serde_yaml::Mapping>,
    #[serde(default)]
    pub bad: Option<serde_yaml::Mapping>,
    #[serde(default)]
    pub total: Option<serde_yaml::Mapping>,
}

impl MetricSourceConfig {
    /// The template for `role`, if this binding supplies one. A `None`
    /// here for a role the indicator's measure needs is always an error
    /// in the caller (`openslo::Context::metric_source`), never silently
    /// substituted with something else.
    #[must_use]
    pub fn role(&self, role: MetricRole) -> Option<&serde_yaml::Mapping> {
        match role {
            MetricRole::Threshold => self.threshold.as_ref(),
            MetricRole::Good => self.good.as_ref(),
            MetricRole::Bad => self.bad.as_ref(),
            MetricRole::Total => self.total.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub signal: SignalSelector,
    #[serde(rename = "dataSource")]
    pub data_source: DataSourceConfig,
    #[serde(rename = "metricSource")]
    pub metric_source: MetricSourceConfig,
}

/// The whole `openslo-bindings.yaml` file: every `Signal` an export may
/// need a `DataSource` for, resolved once up front so a missing binding
/// fails loudly instead of emitting a half-wired SLI.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingsConfig {
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

impl BindingsConfig {
    pub fn parse(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).context("parsing openslo bindings YAML")
    }

    pub fn load(path: &Path) -> Result<Self> {
        let yaml = std::fs::read_to_string(path)
            .with_context(|| format!("reading bindings file {}", path.display()))?;
        Self::parse(&yaml).with_context(|| format!("in bindings file {}", path.display()))
    }

    #[must_use]
    pub fn find(&self, selector: &SignalSelector) -> Option<&Binding> {
        self.bindings.iter().find(|b| &b.signal == selector)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn dns_label_rejects_uppercase_and_leading_dash() {
        assert!(DnsLabel::new("Checkout").is_err());
        assert!(DnsLabel::new("-checkout").is_err());
        assert!(DnsLabel::new("checkout-").is_err());
        assert!(DnsLabel::new("").is_err());
        assert!(DnsLabel::new("checkout-latency").is_ok());
    }

    #[test]
    fn parses_event_timestamps_and_telemetry_bindings() {
        let cfg = BindingsConfig::parse(
            r#"
bindings:
  - signal:
      source: eventTimestamps
    dataSource:
      name: event-store
      type: EventStore
      connectionDetails:
        endpoint: https://events.internal
    metricSource:
      type: EventStore
      total:
        query: "events in {{events}}"
  - signal:
      source: telemetry
      kind: metric
      name: http.server.duration
    dataSource:
      name: datadog-prod
      type: Datadog
      connectionDetails: {}
    metricSource:
      good:
        query: "avg:http.server.duration{{events}}"
"#,
        )
        .unwrap();
        assert_eq!(cfg.bindings.len(), 2);
        let via_event_timestamps = cfg.find(&SignalSelector::EventTimestamps).unwrap();
        assert_eq!(
            via_event_timestamps.data_source.name.as_str(),
            "event-store"
        );
        assert!(via_event_timestamps
            .metric_source
            .role(MetricRole::Total)
            .is_some());
        let via_telemetry = cfg
            .find(&SignalSelector::Telemetry {
                kind: TelemetrySelectorKind::Metric,
                name: "http.server.duration".to_string(),
            })
            .unwrap();
        assert_eq!(via_telemetry.data_source.name.as_str(), "datadog-prod");
        assert!(via_telemetry.metric_source.role(MetricRole::Good).is_some());
        assert!(via_telemetry.metric_source.role(MetricRole::Bad).is_none());
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = BindingsConfig::parse(
            r"
bindings:
  - signal: { source: eventTimestamps }
    dataSource: { name: x, type: Y, bogus: 1 }
    metricSource: { total: {} }
",
        );
        assert!(err.is_err());
    }

    #[test]
    fn rejects_unknown_metric_source_role() {
        let err = BindingsConfig::parse(
            r"
bindings:
  - signal: { source: eventTimestamps }
    dataSource: { name: x, type: Y }
    metricSource: { spec: {} }
",
        );
        assert!(err.is_err(), "`spec` is no longer a valid field");
    }
}
