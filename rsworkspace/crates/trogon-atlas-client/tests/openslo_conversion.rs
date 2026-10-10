#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_client::{openslo::convert_namespace, openslo_bindings::BindingsConfig};
use trogon_atlas_proto as pb;

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn dur(seconds: i64, nanos: i32) -> prost_types::Duration {
    prost_types::Duration { seconds, nanos }
}

fn event_ref(ns: &str, slug: &str) -> pb::EventRef {
    pb::EventRef {
        id: Some(id(ns, slug, 1)),
    }
}

fn component_entity() -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id("reliability", "checkout", 1)),
            title: "Checkout".into(),
            doc: "Owns checkout order placement and fulfillment.".into(),
            ..Default::default()
        })),
    }
}

fn sli_entity(
    slug: &str,
    title: &str,
    connection: pb::Connection,
    measure: pb::service_level_indicator::Measure,
    signal: pb::Signal,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ServiceLevelIndicator(
            pb::ServiceLevelIndicator {
                id: Some(id("reliability", slug, 1)),
                title: title.into(),
                connection: Some(connection),
                measure: Some(measure),
                signal: Some(signal),
                ..Default::default()
            },
        )),
    }
}

fn event_timestamps_signal() -> pb::Signal {
    pb::Signal {
        source: Some(pb::signal::Source::EventTimestamps(
            pb::signal::EventTimestamps {},
        )),
    }
}

fn telemetry_signal(kind: pb::TelemetryKind, name: &str) -> pb::Signal {
    pb::Signal {
        source: Some(pb::signal::Source::Telemetry(pb::signal::Telemetry {
            kind: kind as i32,
            name: name.into(),
        })),
    }
}

fn latency_measure() -> pb::service_level_indicator::Measure {
    pb::service_level_indicator::Measure::Latency(pb::service_level_indicator::Latency {})
}

fn outcome_ratio_measure(
    good: Vec<pb::EventRef>,
    bad: Vec<pb::EventRef>,
) -> pb::service_level_indicator::Measure {
    pb::service_level_indicator::Measure::OutcomeRatio(pb::service_level_indicator::OutcomeRatio {
        good,
        bad,
    })
}

fn completion_measure(within: prost_types::Duration) -> pb::service_level_indicator::Measure {
    pb::service_level_indicator::Measure::Completion(pb::service_level_indicator::Completion {
        within: Some(within),
    })
}

#[allow(clippy::too_many_arguments)]
fn slo_entity(
    slug: &str,
    title: &str,
    indicator_slug: &str,
    objectives: Vec<pb::service_level_objective::Objective>,
    time_window: pb::TimeWindow,
    budgeting_method: pb::BudgetingMethod,
    alert_policy_slugs: Vec<&str>,
    commitment: pb::Commitment,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ServiceLevelObjective(
            pb::ServiceLevelObjective {
                id: Some(id("reliability", slug, 1)),
                title: title.into(),
                service: Some(pb::ComponentRef {
                    id: Some(id("reliability", "checkout", 1)),
                }),
                indicator: Some(pb::ServiceLevelIndicatorRef {
                    id: Some(id("reliability", indicator_slug, 1)),
                }),
                objectives,
                time_window: Some(time_window),
                budgeting_method: budgeting_method as i32,
                alert_policies: alert_policy_slugs
                    .into_iter()
                    .map(|s| pb::AlertPolicyRef {
                        id: Some(id("reliability", s, 1)),
                    })
                    .collect(),
                commitment: Some(commitment),
                ..Default::default()
            },
        )),
    }
}

fn alert_policy_entity() -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::AlertPolicy(pb::AlertPolicy {
            id: Some(id("reliability", "checkout-burn-policy", 1)),
            title: "Checkout burn policy".into(),
            doc: "Pages on fast burn, tickets on slow burn.".into(),
            conditions: vec![
                pb::AlertCondition {
                    display_name: "Fast burn".into(),
                    severity: pb::Severity::Page as i32,
                    burn_rate: Some(pb::alert_condition::BurnRate {
                        op: pb::Comparison::Gt as i32,
                        threshold: 14.4,
                        lookback_window: Some(dur(3600, 0)),
                        alert_after: Some(dur(300, 0)),
                    }),
                },
                pb::AlertCondition {
                    display_name: "Slow burn".into(),
                    severity: pb::Severity::Ticket as i32,
                    burn_rate: Some(pb::alert_condition::BurnRate {
                        op: pb::Comparison::Gt as i32,
                        threshold: 2.0,
                        lookback_window: Some(dur(86400, 0)),
                        alert_after: None,
                    }),
                },
            ],
            notification_targets: vec![pb::AlertNotificationTargetRef {
                id: Some(id("reliability", "oncall-pagerduty", 1)),
            }],
            alert_when_no_data: true,
            alert_when_resolved: true,
            alert_when_breaching: true,
            runbook: "https://runbooks.internal/checkout-burn".into(),
            ..Default::default()
        })),
    }
}

fn alert_notification_target_entity() -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::AlertNotificationTarget(
            pb::AlertNotificationTarget {
                id: Some(id("reliability", "oncall-pagerduty", 1)),
                title: "On-call PagerDuty".into(),
                doc: "Pages the checkout on-call rotation.".into(),
                target: "pagerduty:service-123".into(),
                ..Default::default()
            },
        )),
    }
}

fn bindings() -> BindingsConfig {
    BindingsConfig::parse(
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
      good:
        query: "events in {{events}}"
      bad:
        query: "events in {{events}}"
      total:
        query: "events in {{events}}"
  - signal:
      source: telemetry
      kind: span
      name: checkout.duration
    dataSource:
      name: tracing-backend
      type: Datadog
      connectionDetails:
        site: datadoghq.com
    metricSource:
      threshold:
        query: "p95:trace.checkout.duration{service:checkout}"
  - signal:
      source: telemetry
      kind: metric
      name: checkout.duration
    dataSource:
      name: metrics-backend
      type: Prometheus
      connectionDetails:
        url: http://prometheus.internal:9090
    metricSource:
      threshold:
        query: "histogram_quantile(0.95, checkout_duration_seconds)"
  - signal:
      source: telemetry
      kind: span
      name: checkout.journey.duration
    dataSource:
      name: journey-tracing
      type: Datadog
      connectionDetails:
        site: datadoghq.com
    metricSource:
      good:
        query: "span.duration:checkout.journey{within:{{within}}}"
      total:
        query: "span.started:checkout.journey"
  - signal:
      source: telemetry
      kind: metric
      name: role-missing.signal
    dataSource:
      name: role-missing-backend
      type: Datadog
      connectionDetails: {}
    metricSource:
      good:
        query: "irrelevant, this binding has no threshold role"
  - signal:
      source: telemetry
      kind: span
      name: leaky-token.signal
    dataSource:
      name: leaky-token-backend
      type: Datadog
      connectionDetails: {}
    metricSource:
      threshold:
        query: "{{bogus}}"
"#,
    )
    .unwrap()
}

/// Builds every entity the fixture needs: a Component, SLIs covering
/// every `Connection` variant and every `measure` kind (including an
/// `OutcomeRatio` with only `bad` events, to exercise the `bad`+`total`
/// ratioMetric pairing), SLOs covering a rolling window with an internal
/// commitment and a calendar window with an external commitment, an
/// AlertPolicy with inline conditions, and its AlertNotificationTarget.
fn fixture_entities() -> Vec<pb::Entity> {
    vec![
        component_entity(),
        sli_entity(
            "checkout-latency",
            "Checkout latency",
            pb::Connection {
                kind: Some(pb::connection::Kind::CommandHandling(
                    pb::connection::CommandHandling {
                        command_slice: Some(pb::SliceRef {
                            id: Some(id("reliability", "place-order", 1)),
                        }),
                        events: vec![],
                    },
                )),
            },
            latency_measure(),
            telemetry_signal(pb::TelemetryKind::Span, "checkout.duration"),
        ),
        sli_entity(
            "checkout-success-ratio",
            "Checkout success ratio",
            pb::Connection {
                kind: Some(pb::connection::Kind::Projection(
                    pb::connection::Projection {
                        read_model_slice: Some(pb::SliceRef {
                            id: Some(id("reliability", "order-status", 1)),
                        }),
                        source_events: vec![],
                    },
                )),
            },
            outcome_ratio_measure(
                vec![event_ref("reliability", "order-approved")],
                vec![event_ref("reliability", "order-rejected")],
            ),
            event_timestamps_signal(),
        ),
        sli_entity(
            "checkout-completion",
            "Checkout completion",
            pb::Connection {
                kind: Some(pb::connection::Kind::Reaction(pb::connection::Reaction {
                    automation_slice: Some(pb::SliceRef {
                        id: Some(id("reliability", "notify-warehouse", 1)),
                    }),
                })),
            },
            completion_measure(dur(300, 0)),
            telemetry_signal(pb::TelemetryKind::Span, "checkout.journey.duration"),
        ),
        sli_entity(
            "checkout-latency-display",
            "Checkout latency (display)",
            pb::Connection {
                kind: Some(pb::connection::Kind::Display(pb::connection::Display {
                    ui_slice: Some(pb::SliceRef {
                        id: Some(id("reliability", "checkout-screen", 1)),
                    }),
                })),
            },
            latency_measure(),
            telemetry_signal(pb::TelemetryKind::Metric, "checkout.duration"),
        ),
        sli_entity(
            "checkout-payment-integration",
            "Checkout payment integration",
            pb::Connection {
                kind: Some(pb::connection::Kind::Integration(
                    pb::connection::Integration {
                        processor: Some(pb::ProcessorRef {
                            id: Some(id("reliability", "charge-card", 1)),
                        }),
                        system: Some(pb::ExternalSystemRef {
                            id: Some(id("reliability", "payment-gateway", 1)),
                        }),
                    },
                )),
            },
            outcome_ratio_measure(vec![event_ref("reliability", "payment-captured")], vec![]),
            event_timestamps_signal(),
        ),
        sli_entity(
            "checkout-failure-ratio",
            "Checkout failure ratio",
            pb::Connection {
                kind: Some(pb::connection::Kind::Projection(
                    pb::connection::Projection {
                        read_model_slice: Some(pb::SliceRef {
                            id: Some(id("reliability", "order-status", 1)),
                        }),
                        source_events: vec![],
                    },
                )),
            },
            outcome_ratio_measure(vec![], vec![event_ref("reliability", "order-timed-out")]),
            event_timestamps_signal(),
        ),
        sli_entity(
            "checkout-journey-completion",
            "Checkout journey completion",
            pb::Connection {
                kind: Some(pb::connection::Kind::Journey(pb::connection::Journey {
                    scope: Some(pb::connection::journey::Scope::Storyboard(
                        pb::StoryboardRef {
                            id: Some(id("reliability", "place-order-journey", 1)),
                        },
                    )),
                    start: Some(event_ref("reliability", "order-placed")),
                    end: Some(event_ref("reliability", "order-fulfilled")),
                    correlate_on: vec!["order_id".to_string()],
                })),
            },
            completion_measure(dur(3600, 0)),
            telemetry_signal(pb::TelemetryKind::Span, "checkout.journey.duration"),
        ),
        slo_entity(
            "checkout-latency-slo",
            "Checkout latency SLO",
            "checkout-latency",
            vec![pb::service_level_objective::Objective {
                display_name: "P95 under 300ms".into(),
                threshold: Some(pb::LatencyThreshold {
                    op: pb::Comparison::Lte as i32,
                    value: Some(dur(0, 300_000_000)),
                }),
                target: Some(pb::Target { ratio: 0.95 }),
                time_slice_target: None,
                time_slice_window: None,
            }],
            pb::TimeWindow {
                kind: Some(pb::time_window::Kind::Rolling(pb::time_window::Rolling {
                    duration: Some(dur(28 * 86400, 0)),
                })),
            },
            pb::BudgetingMethod::Occurrences,
            vec!["checkout-burn-policy"],
            pb::Commitment {
                kind: Some(pb::commitment::Kind::Internal(pb::commitment::Internal {})),
            },
        ),
        slo_entity(
            "checkout-success-slo",
            "Checkout success SLO",
            "checkout-success-ratio",
            vec![pb::service_level_objective::Objective {
                display_name: "Success ratio".into(),
                threshold: None,
                target: Some(pb::Target { ratio: 0.999 }),
                time_slice_target: Some(pb::Target { ratio: 0.98 }),
                time_slice_window: Some(dur(300, 0)),
            }],
            pb::TimeWindow {
                kind: Some(pb::time_window::Kind::Calendar(pb::time_window::Calendar {
                    period: pb::CalendarPeriod::Month as i32,
                    start_time: "2024-01-01T00:00:00Z".into(),
                    time_zone: "UTC".into(),
                })),
            },
            pb::BudgetingMethod::Timeslices,
            vec![],
            pb::Commitment {
                kind: Some(pb::commitment::Kind::External(pb::commitment::External {
                    counterparty: Some(pb::commitment::external::Counterparty::Persona(
                        pb::PersonaRef {
                            id: Some(id("reliability", "vp-eng", 1)),
                        },
                    )),
                    agreement: "msa-2024".into(),
                    consequence: "10% service credit".into(),
                })),
            },
        ),
        alert_policy_entity(),
        alert_notification_target_entity(),
    ]
}

#[test]
fn converts_fixture_namespace_to_openslo_yaml() {
    let entities = fixture_entities();
    let outcome = convert_namespace(&entities, &bindings(), false).unwrap();
    assert!(outcome.skipped.is_empty(), "skipped: {:?}", outcome.skipped);

    let expected = include_str!("fixtures/openslo/checkout.golden.yaml");
    assert_eq!(outcome.yaml, expected, "actual YAML:\n{}", outcome.yaml);
}

#[test]
fn missing_binding_fails_with_the_indicator_named() {
    let entities = vec![sli_entity(
        "checkout-unbound",
        "Checkout unbound",
        pb::Connection {
            kind: Some(pb::connection::Kind::CommandHandling(
                pb::connection::CommandHandling {
                    command_slice: Some(pb::SliceRef {
                        id: Some(id("reliability", "place-order", 1)),
                    }),
                    events: vec![],
                },
            )),
        },
        latency_measure(),
        telemetry_signal(pb::TelemetryKind::Span, "unbound.signal"),
    )];
    let empty_bindings = BindingsConfig::default();
    let err = convert_namespace(&entities, &empty_bindings, false).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("reliability-checkout-unbound"),
        "error should name the indicator: {message}"
    );
}

#[test]
fn missing_binding_emits_placeholder_when_allowed() {
    let entities = vec![sli_entity(
        "checkout-unbound",
        "Checkout unbound",
        pb::Connection {
            kind: Some(pb::connection::Kind::CommandHandling(
                pb::connection::CommandHandling {
                    command_slice: Some(pb::SliceRef {
                        id: Some(id("reliability", "place-order", 1)),
                    }),
                    events: vec![],
                },
            )),
        },
        latency_measure(),
        telemetry_signal(pb::TelemetryKind::Span, "unbound.signal"),
    )];
    let empty_bindings = BindingsConfig::default();
    let outcome = convert_namespace(&entities, &empty_bindings, true).unwrap();
    assert!(outcome.yaml.contains("type: Unbound"));
    assert!(outcome.yaml.contains("reliability-checkout-unbound"));
    assert!(
        !outcome.yaml.contains("{{"),
        "placeholder must not leak a template token"
    );
}

#[test]
fn binding_missing_required_role_fails_naming_indicator_and_role() {
    let entities = vec![sli_entity(
        "checkout-role-missing",
        "Checkout role missing",
        pb::Connection {
            kind: Some(pb::connection::Kind::CommandHandling(
                pb::connection::CommandHandling {
                    command_slice: Some(pb::SliceRef {
                        id: Some(id("reliability", "place-order", 1)),
                    }),
                    events: vec![],
                },
            )),
        },
        latency_measure(),
        telemetry_signal(pb::TelemetryKind::Metric, "role-missing.signal"),
    )];
    // allow_placeholder_metrics is irrelevant here: the binding exists for
    // this signal, it just has no `threshold` template, which is always an
    // error, never a placeholder.
    let err = convert_namespace(&entities, &bindings(), true).unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("reliability-checkout-role-missing"),
        "error should name the indicator: {message}"
    );
    assert!(
        message.contains("threshold"),
        "error should name the missing role: {message}"
    );
}

#[test]
fn unresolved_template_token_fails_naming_indicator_and_token() {
    let entities = vec![sli_entity(
        "checkout-leaky-token",
        "Checkout leaky token",
        pb::Connection {
            kind: Some(pb::connection::Kind::CommandHandling(
                pb::connection::CommandHandling {
                    command_slice: Some(pb::SliceRef {
                        id: Some(id("reliability", "place-order", 1)),
                    }),
                    events: vec![],
                },
            )),
        },
        latency_measure(),
        telemetry_signal(pb::TelemetryKind::Span, "leaky-token.signal"),
    )];
    let err = convert_namespace(&entities, &bindings(), false).unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("reliability-checkout-leaky-token"),
        "error should name the indicator: {message}"
    );
    assert!(
        message.contains("bogus"),
        "error should name the unresolved token: {message}"
    );
}
