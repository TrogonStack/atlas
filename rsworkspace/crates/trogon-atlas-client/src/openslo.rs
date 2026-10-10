// Vendor-neutral OpenSLO v1 export. Converts reliability entities
// (ServiceLevelIndicator, ServiceLevelObjective, AlertPolicy,
// AlertNotificationTarget) plus the Components they reference into a
// multi-document OpenSLO v1 YAML stream, per
// https://github.com/OpenSLO/OpenSLO (website/docs/specification.md).
//
// Design choices the spec leaves to the implementer:
//
// - `metadata.name` is derived from `{namespace}-{slug}` (never `version`,
//   which is a change counter, not identity), slugified to an RFC1123 DNS
//   label and truncated to 63 characters. See `dns_name`.
// - `Connection` and `Commitment` have no OpenSLO field, so they travel
//   whole, as JSON, in `metadata.annotations` under the `trogonatlas.eventmodel.v1alpha1/`
//   prefix (this crate's own proto package, never the spec-reserved
//   `openslo.com/`, and never an unowned `eventmodel.io/`-style domain).
//   Same for `AlertPolicy.runbook`, as a plain string.
// - `google.protobuf.Duration` values render as the restricted
//   duration-shorthand `{n}d`, `{n}h` or `{n}m` (never `w`/`M`/`Q`/`Y`,
//   which the spec defines only for calendar periods that have no fixed
//   length). `CalendarPeriod` renders via a fixed table: week -> `1w`,
//   month -> `1M`, quarter -> `1Q`, year -> `1Y`.
// - `LatencyThreshold.value` (a `Duration`) renders as OpenSLO's bare
//   numeric `objectives[].value`, in fractional seconds (so a 300ms
//   threshold renders as `0.3`, not `300`).
// - `ServiceLevelIndicator.measure.outcome_ratio` renders as `ratioMetric`
//   with `good`+`total` when the measure has `good` events, or
//   `bad`+`total` when it has only `bad` events (OpenSLO allows either
//   pairing, never both); `total`'s event list is `good` union `bad`
//   either way, which is the same ratio by the formula in the proto's
//   own doc comment.
// - `ServiceLevelIndicator.measure.completion` renders as `ratioMetric`
//   too: `good` is "instances that finished within the bound", `total`
//   is "instances that started".
// - Every metric query comes from an `openslo-bindings.yaml` entry
//   (`crate::openslo_bindings`) keyed by the SLI's `Signal`. Each entry
//   supplies one template per `MetricRole` (`threshold`, `good`, `bad`,
//   `total`); the converter picks the roles the measure needs (see
//   `Context::sli_doc`) and never reuses one role's template for
//   another, so `good` and `total` can be, and usually are, differently
//   shaped queries rather than the same query with a different window.
//   A role the measure needs but the binding has no template for is an
//   error naming the indicator and the missing role. Template string
//   scalars may contain `{{events}}` (that role's comma-joined event
//   slugs) or `{{within}}` (the `Completion` bound, duration-shorthand),
//   substituted per role/call site; see `substitute_tokens`. Per
//   measure: Latency's `threshold` gets no tokens; OutcomeRatio's
//   `good`/`bad` gets `{{events}}` for that role's own slugs and `total`
//   gets `{{events}}` for good union bad; Completion's `good` gets
//   `{{within}}` and `total` gets no tokens. A `{{token}}` still
//   unresolved after substitution is an error naming the indicator and
//   the token: it is never emitted into the rendered YAML, placeholder
//   or not.
// - A referenced `AlertNotificationTarget` and `ServiceLevelIndicator`
//   are always exported as their own standalone documents (never
//   inlined), so cross-references use `indicatorRef`/`targetRef`.
//   `AlertCondition` has no identity of its own in the model, so it is
//   always inlined in its `AlertPolicy`, with a derived name
//   `{policy}-cond-{1-based index}`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context as _, Result};
use serde_yaml::{Mapping, Value};
use trogon_atlas_core::transcode;
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    openslo_bindings::{BindingsConfig, DataSourceConfig, DnsLabel, MetricRole, SignalSelector},
};

const API_VERSION: &str = "openslo/v1";
// `trogonatlas.eventmodel.v1alpha1` is this crate's own proto package, a domain we actually
// own the meaning of; `eventmodel.io` would invent a domain we do not own.
const ANNOTATION_PREFIX: &str = "trogonatlas.eventmodel.v1alpha1";

/// The result of converting a namespace's entities to OpenSLO: the
/// rendered multi-document YAML plus the kinds that have no OpenSLO
/// shape and were left out (for diagnostics, mirroring
/// `export::ExportOutcome::skipped`).
#[derive(Debug, Clone, Default)]
pub struct OpensloOutcome {
    pub yaml: String,
    pub skipped: Vec<String>,
}

/// Converts a namespace's entities into OpenSLO v1 multi-document YAML.
///
/// `allow_placeholder_metrics` controls what happens when an SLI's
/// `Signal` has no entry in `bindings`: `false` fails with an error
/// naming the SLI; `true` emits a documented placeholder `metricSource`
/// instead (type `Unbound`, no `DataSource` document), so a draft export
/// can still be produced before every signal is wired up.
pub fn convert_namespace(
    entities: &[pb::Entity],
    bindings: &BindingsConfig,
    allow_placeholder_metrics: bool,
) -> Result<OpensloOutcome> {
    let mut ctx = Context {
        bindings,
        allow_placeholder_metrics,
        used_data_sources: BTreeMap::new(),
        components_by_id: BTreeMap::new(),
    };
    for entity in entities {
        if let Some(pb::entity::Kind::Component(c)) = &entity.kind {
            if let Some(id) = &c.id {
                ctx.components_by_id
                    .insert((id.namespace.clone(), id.slug.clone()), c.clone());
            }
        }
    }

    let mut docs: Vec<Mapping> = Vec::new();
    let mut skipped = Vec::new();
    let mut services_emitted: BTreeSet<(String, String)> = BTreeSet::new();

    for entity in entities {
        match &entity.kind {
            Some(pb::entity::Kind::ServiceLevelIndicator(sli)) => {
                docs.push(ctx.sli_doc(sli)?);
            }
            Some(pb::entity::Kind::ServiceLevelObjective(slo)) => {
                if let Some(service_ref) = &slo.service {
                    if let Some(id) = &service_ref.id {
                        let key = (id.namespace.clone(), id.slug.clone());
                        if services_emitted.insert(key.clone()) {
                            docs.push(ctx.service_doc(id)?);
                        }
                    }
                }
                docs.push(Context::slo_doc(slo)?);
            }
            Some(pb::entity::Kind::AlertPolicy(policy)) => {
                docs.push(Context::alert_policy_doc(policy)?);
            }
            Some(pb::entity::Kind::AlertNotificationTarget(target)) => {
                docs.push(Context::alert_notification_target_doc(target)?);
            }
            Some(pb::entity::Kind::Component(_)) | None => {}
            Some(other) => skipped.push(entity_label(other)),
        }
    }

    for config in ctx.used_data_sources.values() {
        docs.push(data_source_doc(config));
    }

    let yaml = render_docs(&docs)?;
    Ok(OpensloOutcome { yaml, skipped })
}

fn entity_label(kind: &pb::entity::Kind) -> String {
    format!("{kind:?}")
        .split('(')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

/// Fetches every entity in `namespace` at its latest version only: an
/// OpenSLO export represents the namespace's current state, never its
/// full history (unlike `export::export_namespace`, whose manifests are
/// meant to round-trip the whole changelog).
async fn fetch_latest_namespace_entities(
    client: &mut Client,
    namespace: &str,
) -> Result<Vec<pb::Entity>> {
    let mut entities: Vec<pb::Entity> = Vec::new();
    let mut page_token = String::new();
    loop {
        let resp = client
            .list_entities(pb::ListEntitiesRequest {
                kinds: vec![],
                namespaces: vec![namespace.to_string()],
                latest_versions_only: true,
                page_size: 200,
                page_token: page_token.clone(),
                lifecycle_status_in: vec![],
                lifecycle_status_not_in: vec![],
            })
            .await
            .context("ListEntities failed")?
            .into_inner();
        entities.extend(resp.entities);
        if resp.next_page_token.is_empty() {
            break;
        }
        page_token = resp.next_page_token;
    }
    if entities.is_empty() {
        crate::invalid_input!("namespace {namespace:?} has no entities");
    }
    Ok(entities)
}

fn render_docs<'a>(docs: impl IntoIterator<Item = &'a Mapping>) -> Result<String> {
    let mut out = String::new();
    for (i, doc) in docs.into_iter().enumerate() {
        if i > 0 {
            out.push_str("---\n");
        }
        out.push_str(&serde_yaml::to_string(doc).context("rendering YAML")?);
    }
    Ok(out)
}

/// Fetches `namespace`'s current entities and converts them to OpenSLO
/// v1 multi-document YAML. See `convert_namespace` for conversion
/// semantics and the meaning of `allow_placeholder_metrics`.
pub async fn export_namespace_openslo(
    client: &mut Client,
    namespace: &str,
    bindings: &BindingsConfig,
    allow_placeholder_metrics: bool,
) -> Result<OpensloOutcome> {
    let entities = fetch_latest_namespace_entities(client, namespace).await?;
    convert_namespace(&entities, bindings, allow_placeholder_metrics)
}

struct Context<'a> {
    bindings: &'a BindingsConfig,
    allow_placeholder_metrics: bool,
    used_data_sources: BTreeMap<DnsLabel, DataSourceConfig>,
    components_by_id: BTreeMap<(String, String), pb::Component>,
}

impl Context<'_> {
    fn service_doc(&self, id: &pb::Id) -> Result<Mapping> {
        let component = self
            .components_by_id
            .get(&(id.namespace.clone(), id.slug.clone()));
        let name = dns_name(id)?;
        let mut spec = Mapping::new();
        if let Some(c) = component {
            if !c.doc.is_empty() {
                spec.insert(key("description"), Value::String(c.doc.clone()));
            }
        }
        let display_name = component
            .map(|c| c.title.as_str())
            .filter(|t| !t.is_empty());
        Ok(document("Service", &name, display_name, None, spec))
    }

    fn sli_doc(&mut self, sli: &pb::ServiceLevelIndicator) -> Result<Mapping> {
        let id = sli.id.as_ref().context("ServiceLevelIndicator has no id")?;
        let name = dns_name(id)?;
        let mut spec = Mapping::new();
        if !sli.doc.is_empty() {
            spec.insert(key("description"), Value::String(sli.doc.clone()));
        }

        let selector = SignalSelector::from_signal(sli.signal.as_ref())
            .with_context(|| format!("indicator {} ({})", name, id_string(id)))?;

        use pb::service_level_indicator::Measure;
        match sli.measure.as_ref() {
            Some(Measure::Latency(_)) => {
                let metric_source =
                    self.metric_source(&selector, &name, MetricRole::Threshold, &[])?;
                let mut threshold_metric = Mapping::new();
                threshold_metric.insert(key("metricSource"), Value::Mapping(metric_source));
                spec.insert(key("thresholdMetric"), Value::Mapping(threshold_metric));
            }
            Some(Measure::OutcomeRatio(ratio)) => {
                let good_slugs = event_slugs(&ratio.good);
                let bad_slugs = event_slugs(&ratio.bad);
                let total_slugs: Vec<String> = good_slugs
                    .iter()
                    .chain(bad_slugs.iter())
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if total_slugs.is_empty() {
                    bail!(
                        "indicator {} ({}) OutcomeRatio measure has neither good nor bad events",
                        name,
                        id_string(id)
                    );
                }
                let total_source = self.metric_source(
                    &selector,
                    &name,
                    MetricRole::Total,
                    &[("events", join_events(&total_slugs))],
                )?;
                let mut ratio_metric = Mapping::new();
                ratio_metric.insert(key("counter"), Value::Bool(true));
                if good_slugs.is_empty() {
                    let bad_source = self.metric_source(
                        &selector,
                        &name,
                        MetricRole::Bad,
                        &[("events", join_events(&bad_slugs))],
                    )?;
                    ratio_metric.insert(key("bad"), metric_source_field(bad_source));
                } else {
                    let good_source = self.metric_source(
                        &selector,
                        &name,
                        MetricRole::Good,
                        &[("events", join_events(&good_slugs))],
                    )?;
                    ratio_metric.insert(key("good"), metric_source_field(good_source));
                }
                ratio_metric.insert(key("total"), metric_source_field(total_source));
                spec.insert(key("ratioMetric"), Value::Mapping(ratio_metric));
            }
            Some(Measure::Completion(completion)) => {
                let within = completion
                    .within
                    .as_ref()
                    .context("Completion measure has no `within` duration")?;
                let within_shorthand = duration_shorthand(within)?;
                let good_source = self.metric_source(
                    &selector,
                    &name,
                    MetricRole::Good,
                    &[("within", within_shorthand)],
                )?;
                let total_source = self.metric_source(&selector, &name, MetricRole::Total, &[])?;
                let mut ratio_metric = Mapping::new();
                ratio_metric.insert(key("counter"), Value::Bool(true));
                ratio_metric.insert(key("good"), metric_source_field(good_source));
                ratio_metric.insert(key("total"), metric_source_field(total_source));
                spec.insert(key("ratioMetric"), Value::Mapping(ratio_metric));
            }
            None => bail!("indicator {} ({}) has no measure", name, id_string(id)),
        }

        let display_name = (!sli.title.is_empty()).then_some(sli.title.as_str());
        let annotations = connection_annotations(sli.connection.as_ref())?;
        Ok(document("SLI", &name, display_name, annotations, spec))
    }

    /// Builds one `metricSource` mapping for `role`: `metricSourceRef`
    /// plus that role's template (substituted and checked for leftover
    /// `{{token}}`s) for a real binding, or a documented placeholder when
    /// `allow_placeholder_metrics` is set and the signal has no binding
    /// at all.
    ///
    /// A binding that exists for the signal but has no template for
    /// `role` is always an error, regardless of
    /// `allow_placeholder_metrics`: that flag covers a signal nobody has
    /// wired up yet, not an incomplete binding someone already wrote.
    fn metric_source(
        &mut self,
        selector: &SignalSelector,
        indicator_name: &DnsLabel,
        role: MetricRole,
        tokens: &[(&str, String)],
    ) -> Result<Mapping> {
        match self.bindings.find(selector) {
            Some(binding) => {
                let template = binding.metric_source.role(role).with_context(|| {
                    format!(
                        "indicator {indicator_name} has an openslo-bindings.yaml entry for its \
                         signal but no `{role}` metricSource template; add \
                         metricSource.{role} to that binding"
                    )
                })?;
                self.used_data_sources.insert(
                    binding.data_source.name.clone(),
                    binding.data_source.clone(),
                );
                let spec = substitute_tokens(template, tokens);
                if let Some(token) = find_unsubstituted_token(&Value::Mapping(spec.clone())) {
                    bail!(
                        "indicator {indicator_name}'s `{role}` metricSource template still has \
                         an unresolved `{{{{{token}}}}}` after substitution; {role} does not \
                         receive a `{token}` value for this measure, so the template should not \
                         reference it"
                    );
                }
                let mut m = Mapping::new();
                m.insert(
                    key("metricSourceRef"),
                    Value::String(binding.data_source.name.as_str().to_string()),
                );
                m.insert(key("spec"), Value::Mapping(spec));
                Ok(m)
            }
            None if self.allow_placeholder_metrics => {
                let mut spec = Mapping::new();
                spec.insert(
                    key("note"),
                    Value::String(format!(
                        "no openslo-bindings.yaml entry for this signal; placeholder emitted \
                         for indicator {indicator_name}'s `{role}` metricSource"
                    )),
                );
                let mut m = Mapping::new();
                m.insert(key("type"), Value::String("Unbound".to_string()));
                m.insert(key("spec"), Value::Mapping(spec));
                Ok(m)
            }
            None => bail!(
                "indicator {indicator_name} has no openslo-bindings.yaml entry for its signal \
                 ({selector:?}); add one, or pass --allow-placeholder-metrics to emit a draft"
            ),
        }
    }

    fn slo_doc(slo: &pb::ServiceLevelObjective) -> Result<Mapping> {
        let id = slo.id.as_ref().context("ServiceLevelObjective has no id")?;
        let name = dns_name(id)?;
        let mut spec = Mapping::new();
        if !slo.doc.is_empty() {
            spec.insert(key("description"), Value::String(slo.doc.clone()));
        }
        if let Some(service_ref) = &slo.service {
            if let Some(service_id) = &service_ref.id {
                spec.insert(
                    key("service"),
                    Value::String(dns_name(service_id)?.to_string()),
                );
            }
        }
        let indicator_ref = slo
            .indicator
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .context("ServiceLevelObjective has no indicator reference")?;
        spec.insert(
            key("indicatorRef"),
            Value::String(dns_name(indicator_ref)?.to_string()),
        );

        if let Some(window) = &slo.time_window {
            spec.insert(
                key("timeWindow"),
                Value::Sequence(vec![time_window(window)?]),
            );
        }

        let budgeting_method = budgeting_method_str(slo.budgeting_method)?;
        spec.insert(
            key("budgetingMethod"),
            Value::String(budgeting_method.to_string()),
        );

        let objectives: Result<Vec<Value>> = slo.objectives.iter().map(objective_doc).collect();
        spec.insert(key("objectives"), Value::Sequence(objectives?));

        if !slo.alert_policies.is_empty() {
            let refs: Result<Vec<Value>> = slo
                .alert_policies
                .iter()
                .map(|r| {
                    let id = r.id.as_ref().context("AlertPolicyRef has no id")?;
                    let mut m = Mapping::new();
                    m.insert(
                        key("alertPolicyRef"),
                        Value::String(dns_name(id)?.to_string()),
                    );
                    Ok(Value::Mapping(m))
                })
                .collect();
            spec.insert(key("alertPolicies"), Value::Sequence(refs?));
        }

        let display_name = (!slo.title.is_empty()).then_some(slo.title.as_str());
        let annotations = commitment_annotations(slo.commitment.as_ref())?;
        Ok(document("SLO", &name, display_name, annotations, spec))
    }

    fn alert_policy_doc(policy: &pb::AlertPolicy) -> Result<Mapping> {
        let id = policy.id.as_ref().context("AlertPolicy has no id")?;
        let name = dns_name(id)?;
        let mut spec = Mapping::new();
        if !policy.doc.is_empty() {
            spec.insert(key("description"), Value::String(policy.doc.clone()));
        }
        spec.insert(
            key("alertWhenNoData"),
            Value::Bool(policy.alert_when_no_data),
        );
        spec.insert(
            key("alertWhenResolved"),
            Value::Bool(policy.alert_when_resolved),
        );
        spec.insert(
            key("alertWhenBreaching"),
            Value::Bool(policy.alert_when_breaching),
        );

        let conditions: Result<Vec<Value>> = policy
            .conditions
            .iter()
            .enumerate()
            .map(|(i, c)| alert_condition_inline(&name, i, c))
            .collect();
        spec.insert(key("conditions"), Value::Sequence(conditions?));

        if !policy.notification_targets.is_empty() {
            let refs: Result<Vec<Value>> = policy
                .notification_targets
                .iter()
                .map(|r| {
                    let id =
                        r.id.as_ref()
                            .context("AlertNotificationTargetRef has no id")?;
                    let mut m = Mapping::new();
                    m.insert(key("targetRef"), Value::String(dns_name(id)?.to_string()));
                    Ok(Value::Mapping(m))
                })
                .collect();
            spec.insert(key("notificationTargets"), Value::Sequence(refs?));
        }

        let display_name = (!policy.title.is_empty()).then_some(policy.title.as_str());
        let mut annotations = Mapping::new();
        if !policy.runbook.is_empty() {
            annotations.insert(
                key(&format!("{ANNOTATION_PREFIX}/runbook")),
                Value::String(policy.runbook.clone()),
            );
        }
        let annotations = (!annotations.is_empty()).then_some(annotations);
        Ok(document(
            "AlertPolicy",
            &name,
            display_name,
            annotations,
            spec,
        ))
    }

    fn alert_notification_target_doc(target: &pb::AlertNotificationTarget) -> Result<Mapping> {
        let id = target
            .id
            .as_ref()
            .context("AlertNotificationTarget has no id")?;
        let name = dns_name(id)?;
        let mut spec = Mapping::new();
        spec.insert(key("target"), Value::String(target.target.clone()));
        if !target.doc.is_empty() {
            spec.insert(key("description"), Value::String(target.doc.clone()));
        }
        let display_name = (!target.title.is_empty()).then_some(target.title.as_str());
        Ok(document(
            "AlertNotificationTarget",
            &name,
            display_name,
            None,
            spec,
        ))
    }
}

fn metric_source_field(metric_source: Mapping) -> Value {
    let mut wrapper = Mapping::new();
    wrapper.insert(key("metricSource"), Value::Mapping(metric_source));
    Value::Mapping(wrapper)
}

fn alert_condition_inline(
    policy_name: &DnsLabel,
    index: usize,
    c: &pb::AlertCondition,
) -> Result<Value> {
    let name = with_suffix(policy_name, &format!("cond-{}", index + 1));
    let burn_rate = c
        .burn_rate
        .as_ref()
        .context("AlertCondition has no burn_rate")?;
    let mut condition = Mapping::new();
    condition.insert(key("kind"), Value::String("burnrate".to_string()));
    condition.insert(
        key("op"),
        Value::String(comparison_str(burn_rate.op)?.to_string()),
    );
    condition.insert(
        key("threshold"),
        Value::Number(serde_yaml::Number::from(burn_rate.threshold)),
    );
    let lookback = burn_rate
        .lookback_window
        .as_ref()
        .context("AlertCondition.burn_rate has no lookback_window")?;
    condition.insert(
        key("lookbackWindow"),
        Value::String(duration_shorthand(lookback)?),
    );
    if let Some(alert_after) = &burn_rate.alert_after {
        condition.insert(
            key("alertAfter"),
            Value::String(duration_shorthand(alert_after)?),
        );
    }

    let mut spec = Mapping::new();
    spec.insert(
        key("severity"),
        Value::String(severity_str(c.severity)?.to_string()),
    );
    spec.insert(key("condition"), Value::Mapping(condition));

    let mut metadata = Mapping::new();
    metadata.insert(key("name"), Value::String(name.to_string()));
    if !c.display_name.is_empty() {
        metadata.insert(key("displayName"), Value::String(c.display_name.clone()));
    }

    let mut doc = Mapping::new();
    doc.insert(key("kind"), Value::String("AlertCondition".to_string()));
    doc.insert(key("metadata"), Value::Mapping(metadata));
    doc.insert(key("spec"), Value::Mapping(spec));
    Ok(Value::Mapping(doc))
}

fn objective_doc(o: &pb::service_level_objective::Objective) -> Result<Value> {
    let mut m = Mapping::new();
    if !o.display_name.is_empty() {
        m.insert(key("displayName"), Value::String(o.display_name.clone()));
    }
    if let Some(threshold) = &o.threshold {
        m.insert(
            key("op"),
            Value::String(comparison_str(threshold.op)?.to_string()),
        );
        let value = threshold
            .value
            .as_ref()
            .context("LatencyThreshold has no value")?;
        let seconds = duration_to_fractional_seconds(value);
        m.insert(
            key("value"),
            Value::Number(serde_yaml::Number::from(seconds)),
        );
    }
    if let Some(target) = &o.target {
        m.insert(
            key("target"),
            Value::Number(serde_yaml::Number::from(target.ratio)),
        );
    }
    if let Some(time_slice_target) = &o.time_slice_target {
        m.insert(
            key("timeSliceTarget"),
            Value::Number(serde_yaml::Number::from(time_slice_target.ratio)),
        );
    }
    if let Some(window) = &o.time_slice_window {
        m.insert(
            key("timeSliceWindow"),
            Value::String(duration_shorthand(window)?),
        );
    }
    Ok(Value::Mapping(m))
}

fn time_window(window: &pb::TimeWindow) -> Result<Value> {
    use pb::time_window::Kind;
    let mut m = Mapping::new();
    match window.kind.as_ref() {
        Some(Kind::Rolling(rolling)) => {
            let duration = rolling
                .duration
                .as_ref()
                .context("Rolling time window has no duration")?;
            m.insert(
                key("duration"),
                Value::String(duration_shorthand(duration)?),
            );
            m.insert(key("isRolling"), Value::Bool(true));
        }
        Some(Kind::Calendar(calendar)) => {
            let duration = calendar_period_shorthand(calendar.period)?;
            m.insert(key("duration"), Value::String(duration.to_string()));
            let mut cal = Mapping::new();
            cal.insert(
                key("startTime"),
                Value::String(calendar_start_time(&calendar.start_time)?),
            );
            cal.insert(key("timeZone"), Value::String(calendar.time_zone.clone()));
            m.insert(key("calendar"), Value::Mapping(cal));
            m.insert(key("isRolling"), Value::Bool(false));
        }
        None => bail!("TimeWindow has neither rolling nor calendar set"),
    }
    Ok(Value::Mapping(m))
}

fn data_source_doc(config: &DataSourceConfig) -> Mapping {
    let mut spec = Mapping::new();
    spec.insert(key("type"), Value::String(config.source_type.clone()));
    spec.insert(
        key("connectionDetails"),
        Value::Mapping(config.connection_details.clone()),
    );
    document("DataSource", &config.name, None, None, spec)
}

fn document(
    kind: &str,
    name: &DnsLabel,
    display_name: Option<&str>,
    annotations: Option<Mapping>,
    spec: Mapping,
) -> Mapping {
    let mut metadata = Mapping::new();
    metadata.insert(key("name"), Value::String(name.to_string()));
    if let Some(display_name) = display_name {
        metadata.insert(key("displayName"), Value::String(display_name.to_string()));
    }
    if let Some(annotations) = annotations {
        metadata.insert(key("annotations"), Value::Mapping(annotations));
    }
    let mut doc = Mapping::new();
    doc.insert(key("apiVersion"), Value::String(API_VERSION.to_string()));
    doc.insert(key("kind"), Value::String(kind.to_string()));
    doc.insert(key("metadata"), Value::Mapping(metadata));
    doc.insert(key("spec"), Value::Mapping(spec));
    doc
}

fn connection_annotations(connection: Option<&pb::Connection>) -> Result<Option<Mapping>> {
    let Some(connection) = connection else {
        return Ok(None);
    };
    let json =
        transcode::message_json_value("trogonatlas.eventmodel.v1alpha1.Connection", connection)
            .context("serializing Connection for metadata.annotations")?;
    let mut m = Mapping::new();
    m.insert(
        key(&format!("{ANNOTATION_PREFIX}/connection")),
        Value::String(serde_json::to_string(&json).context("rendering Connection JSON")?),
    );
    Ok(Some(m))
}

fn commitment_annotations(commitment: Option<&pb::Commitment>) -> Result<Option<Mapping>> {
    let Some(commitment) = commitment else {
        return Ok(None);
    };
    let json =
        transcode::message_json_value("trogonatlas.eventmodel.v1alpha1.Commitment", commitment)
            .context("serializing Commitment for metadata.annotations")?;
    let mut m = Mapping::new();
    m.insert(
        key(&format!("{ANNOTATION_PREFIX}/commitment")),
        Value::String(serde_json::to_string(&json).context("rendering Commitment JSON")?),
    );
    Ok(Some(m))
}

fn event_slugs(events: &[pb::EventRef]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| e.id.as_ref())
        .map(|id| id.slug.clone())
        .collect()
}

fn join_events(slugs: &[String]) -> String {
    slugs.join(",")
}

/// Replaces `{{token}}` occurrences in every string scalar of `spec`
/// (recursively through mappings and sequences) with the matching value
/// from `tokens`. A token with no matching value is left untouched here;
/// callers are expected to run `find_unsubstituted_token` on the result
/// and turn any survivor into an error, never emit it into the YAML.
fn substitute_tokens(spec: &Mapping, tokens: &[(&str, String)]) -> Mapping {
    let value = Value::Mapping(spec.clone());
    match substitute_value(value, tokens) {
        Value::Mapping(m) => m,
        _ => spec.clone(),
    }
}

fn substitute_value(value: Value, tokens: &[(&str, String)]) -> Value {
    match value {
        Value::String(s) => {
            let mut out = s;
            for (token, replacement) in tokens {
                out = out.replace(&format!("{{{{{token}}}}}"), replacement);
            }
            Value::String(out)
        }
        Value::Sequence(seq) => Value::Sequence(
            seq.into_iter()
                .map(|v| substitute_value(v, tokens))
                .collect(),
        ),
        Value::Mapping(map) => {
            let mut out = Mapping::new();
            for (k, v) in map {
                out.insert(k, substitute_value(v, tokens));
            }
            Value::Mapping(out)
        }
        other => other,
    }
}

/// Finds the first `{{token}}` left in any string scalar of `value`
/// (recursively through mappings and sequences), returning just the
/// token name. Called after `substitute_tokens` so a template that
/// references a token its role/measure never supplies a value for is
/// caught here, rather than reaching the rendered YAML.
fn find_unsubstituted_token(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => extract_token(s),
        Value::Sequence(seq) => seq.iter().find_map(find_unsubstituted_token),
        Value::Mapping(map) => map.values().find_map(find_unsubstituted_token),
        _ => None,
    }
}

fn extract_token(s: &str) -> Option<String> {
    let after_open = s.split_once("{{")?.1;
    let token = after_open.split_once("}}")?.0;
    Some(token.to_string())
}

fn key(s: &str) -> Value {
    Value::String(s.to_string())
}

fn id_string(id: &pb::Id) -> String {
    format!("{}/{}", id.namespace, id.slug)
}

/// Derives an OpenSLO `metadata.name` from an `Id`'s `namespace` and
/// `slug` (never `version`, which is a change counter, not identity).
pub fn dns_name(id: &pb::Id) -> Result<DnsLabel> {
    let raw = slugify(&format!("{}-{}", id.namespace, id.slug));
    let truncated = truncate_dns_label(&raw);
    DnsLabel::new(truncated)
        .with_context(|| format!("deriving metadata.name for {}", id_string(id)))
}

/// Appends `-{suffix}` to `base`, truncating `base` (never the suffix)
/// so the combined label still fits the 63-character RFC1123 limit.
pub fn with_suffix(base: &DnsLabel, suffix: &str) -> DnsLabel {
    let budget = 63usize.saturating_sub(suffix.len() + 1);
    let base_str = base.as_str();
    let truncated: String = base_str.chars().take(budget).collect();
    let truncated = truncated.trim_end_matches('-');
    let combined = format!("{truncated}-{suffix}");
    DnsLabel::new(&combined).unwrap_or_else(|_| {
        // Caller-controlled suffix is always ASCII-lowercase-alnum-dash
        // in this module, so this only triggers on a degenerate (empty)
        // base; fall back to the suffix alone.
        DnsLabel::new(suffix).unwrap_or_else(|_| unreachable!("suffix is a valid DNS label"))
    })
}

fn slugify(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_dash = false;
    for c in raw.chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            Some(c.to_ascii_lowercase())
        } else {
            Some('-')
        };
        if let Some(c) = mapped {
            if c == '-' {
                if !last_was_dash && !out.is_empty() {
                    out.push('-');
                }
                last_was_dash = true;
            } else {
                out.push(c);
                last_was_dash = false;
            }
        }
    }
    out.trim_end_matches('-').to_string()
}

fn truncate_dns_label(raw: &str) -> String {
    let truncated: String = raw.chars().take(63).collect();
    truncated.trim_end_matches('-').to_string()
}

/// Renders a `google.protobuf.Duration` as OpenSLO duration-shorthand
/// restricted to minutes, hours and days: the plan's own examples
/// (`28d`, `1h`, `5m`) rule out `w`/`M`/`Q`/`Y` here, which the spec
/// defines only for calendar periods of variable length.
pub fn duration_shorthand(d: &prost_types::Duration) -> Result<String> {
    if d.nanos != 0 {
        bail!(
            "duration {d:?} has a sub-second component, which duration-shorthand cannot represent"
        );
    }
    if d.seconds < 0 {
        bail!("duration {d:?} is negative, which duration-shorthand cannot represent");
    }
    if d.seconds % 60 != 0 {
        bail!("duration {d:?} is not a whole number of minutes");
    }
    let minutes = d.seconds / 60;
    if minutes != 0 && minutes % (60 * 24) == 0 {
        return Ok(format!("{}d", minutes / (60 * 24)));
    }
    if minutes != 0 && minutes % 60 == 0 {
        return Ok(format!("{}h", minutes / 60));
    }
    Ok(format!("{minutes}m"))
}

// Durations in this module are human-authored SLO objectives/windows,
// nowhere near f64's 2^53-second exact-integer range, so this cast
// loses no precision in practice.
#[allow(clippy::cast_precision_loss)]
fn duration_to_fractional_seconds(d: &prost_types::Duration) -> f64 {
    d.seconds as f64 + f64::from(d.nanos) / 1_000_000_000.0
}

fn calendar_period_shorthand(period: i32) -> Result<&'static str> {
    use pb::CalendarPeriod as P;
    match P::try_from(period) {
        Ok(P::Week) => Ok("1w"),
        Ok(P::Month) => Ok("1M"),
        Ok(P::Quarter) => Ok("1Q"),
        Ok(P::Year) => Ok("1Y"),
        _ => bail!("calendar time window has no (or an unspecified) period"),
    }
}

fn comparison_str(op: i32) -> Result<&'static str> {
    use pb::Comparison as C;
    match C::try_from(op) {
        Ok(C::Lt) => Ok("lt"),
        Ok(C::Lte) => Ok("lte"),
        Ok(C::Gt) => Ok("gt"),
        Ok(C::Gte) => Ok("gte"),
        _ => bail!("comparison operator is unset"),
    }
}

fn budgeting_method_str(method: i32) -> Result<&'static str> {
    use pb::BudgetingMethod as B;
    match B::try_from(method) {
        Ok(B::Occurrences) => Ok("Occurrences"),
        Ok(B::Timeslices) => Ok("Timeslices"),
        Ok(B::RatioTimeslices) => Ok("RatioTimeslices"),
        _ => bail!("budgeting method is unset"),
    }
}

fn severity_str(severity: i32) -> Result<&'static str> {
    use pb::Severity as S;
    match S::try_from(severity) {
        Ok(S::Page) => Ok("page"),
        Ok(S::Ticket) => Ok("ticket"),
        Ok(S::Info) => Ok("info"),
        _ => bail!("severity is unset"),
    }
}

/// Converts an RFC3339 timestamp (e.g. `2020-01-21T12:30:00Z` or
/// `2020-01-21T12:30:00-05:00`) to OpenSLO's calendar `startTime` format,
/// a bare `YYYY-MM-DD HH:MM:SS` with no offset (the offset is carried
/// separately by `timeZone`, an IANA name).
fn calendar_start_time(rfc3339: &str) -> Result<String> {
    let (date, rest) = rfc3339
        .split_once('T')
        .with_context(|| format!("{rfc3339:?} is not RFC3339 (missing 'T')"))?;
    let cut = rest
        .char_indices()
        .skip(2) // skip past "HH" so a leading '-' in "HH:MM:SS" offset isn't mistaken for the start
        .find(|(_, c)| matches!(c, 'Z' | '+' | '-' | '.'))
        .map_or(rest.len(), |(i, _)| i);
    Ok(format!("{date} {}", &rest[..cut]))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn dur(seconds: i64) -> prost_types::Duration {
        prost_types::Duration { seconds, nanos: 0 }
    }

    #[test]
    fn dns_name_strips_dots_and_joins_namespace_slug() {
        let name = dns_name(&id("reliability", "checkout.latency")).unwrap();
        assert_eq!(name.as_str(), "reliability-checkout-latency");
    }

    #[test]
    fn dns_name_truncates_long_labels() {
        let long_slug = "a".repeat(100);
        let name = dns_name(&id("reliability", &long_slug)).unwrap();
        assert!(name.as_str().len() <= 63);
    }

    #[test]
    fn with_suffix_truncates_base_not_suffix() {
        let base = DnsLabel::new("a".repeat(63)).unwrap();
        let suffixed = with_suffix(&base, "cond-12");
        assert!(suffixed.as_str().ends_with("-cond-12"));
        assert!(suffixed.as_str().len() <= 63);
    }

    #[test]
    fn duration_shorthand_picks_largest_exact_unit_excluding_weeks() {
        assert_eq!(duration_shorthand(&dur(28 * 86400)).unwrap(), "28d");
        assert_eq!(duration_shorthand(&dur(3600)).unwrap(), "1h");
        assert_eq!(duration_shorthand(&dur(5 * 60)).unwrap(), "5m");
        assert_eq!(duration_shorthand(&dur(2 * 60)).unwrap(), "2m");
    }

    #[test]
    fn duration_shorthand_rejects_sub_minute_precision() {
        let sub_second = prost_types::Duration {
            seconds: 1,
            nanos: 500_000_000,
        };
        assert!(duration_shorthand(&sub_second).is_err());
        assert!(duration_shorthand(&dur(90)).is_err());
    }

    #[test]
    fn calendar_period_shorthand_table() {
        assert_eq!(
            calendar_period_shorthand(pb::CalendarPeriod::Week as i32).unwrap(),
            "1w"
        );
        assert_eq!(
            calendar_period_shorthand(pb::CalendarPeriod::Month as i32).unwrap(),
            "1M"
        );
        assert_eq!(
            calendar_period_shorthand(pb::CalendarPeriod::Quarter as i32).unwrap(),
            "1Q"
        );
        assert_eq!(
            calendar_period_shorthand(pb::CalendarPeriod::Year as i32).unwrap(),
            "1Y"
        );
    }

    #[test]
    fn calendar_start_time_strips_offset() {
        assert_eq!(
            calendar_start_time("2020-01-21T12:30:00Z").unwrap(),
            "2020-01-21 12:30:00"
        );
        assert_eq!(
            calendar_start_time("2020-01-21T12:30:00-05:00").unwrap(),
            "2020-01-21 12:30:00"
        );
        assert_eq!(
            calendar_start_time("2020-01-21T12:30:00.123Z").unwrap(),
            "2020-01-21 12:30:00"
        );
    }

    #[test]
    fn substitute_tokens_replaces_nested_string_scalars() {
        let mut inner = Mapping::new();
        inner.insert(
            key("query"),
            Value::String("events in {{events}}".to_string()),
        );
        let mut spec = Mapping::new();
        spec.insert(key("nested"), Value::Mapping(inner));
        let out = substitute_tokens(&spec, &[("events", "a,b".to_string())]);
        let Value::Mapping(nested) = out.get(key("nested")).unwrap() else {
            panic!("expected nested mapping");
        };
        assert_eq!(
            nested.get(key("query")),
            Some(&Value::String("events in a,b".to_string()))
        );
    }

    #[test]
    fn find_unsubstituted_token_reports_the_token_name() {
        let mut spec = Mapping::new();
        spec.insert(key("withinWindow"), Value::String("{{within}}".to_string()));
        assert_eq!(
            find_unsubstituted_token(&Value::Mapping(spec)),
            Some("within".to_string())
        );
    }

    #[test]
    fn find_unsubstituted_token_is_none_once_fully_substituted() {
        let out = substitute_tokens(
            &{
                let mut spec = Mapping::new();
                spec.insert(key("query"), Value::String("events in {{events}}".into()));
                spec
            },
            &[("events", "a,b".to_string())],
        );
        assert_eq!(find_unsubstituted_token(&Value::Mapping(out)), None);
    }

    #[test]
    fn comparison_mappings_round_trip_known_values() {
        assert_eq!(comparison_str(pb::Comparison::Lt as i32).unwrap(), "lt");
        assert_eq!(comparison_str(pb::Comparison::Lte as i32).unwrap(), "lte");
        assert_eq!(comparison_str(pb::Comparison::Gt as i32).unwrap(), "gt");
        assert_eq!(comparison_str(pb::Comparison::Gte as i32).unwrap(), "gte");
        assert!(comparison_str(pb::Comparison::Unspecified as i32).is_err());
    }

    #[test]
    fn budgeting_method_mappings_round_trip_known_values() {
        assert_eq!(
            budgeting_method_str(pb::BudgetingMethod::Occurrences as i32).unwrap(),
            "Occurrences"
        );
        assert_eq!(
            budgeting_method_str(pb::BudgetingMethod::Timeslices as i32).unwrap(),
            "Timeslices"
        );
        assert_eq!(
            budgeting_method_str(pb::BudgetingMethod::RatioTimeslices as i32).unwrap(),
            "RatioTimeslices"
        );
    }

    #[test]
    fn severity_mappings_round_trip_known_values() {
        assert_eq!(severity_str(pb::Severity::Page as i32).unwrap(), "page");
        assert_eq!(severity_str(pb::Severity::Ticket as i32).unwrap(), "ticket");
        assert_eq!(severity_str(pb::Severity::Info as i32).unwrap(), "info");
    }
}
