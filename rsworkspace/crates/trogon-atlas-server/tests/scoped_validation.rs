#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use support::two_tenants::{authed, register_ns, ACME_TOKEN, BETA_TOKEN, ROOT_TOKEN};
use tonic::transport::Channel;
use trogon_atlas_core::refs::entity_kind;
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_store::{store::WriteContext, Store};

const HIDDEN_DETAIL: &str = "hidden-validation-detail";

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn member(kind: pb::EntityKind, namespace: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(namespace, slug)),
    }
}

struct Fixture {
    client: EventModelServiceClient<Channel>,
    store: Arc<dyn Store>,
    own: String,
    hidden: String,
}

impl Fixture {
    async fn new() -> Self {
        let store: Arc<dyn Store> =
            Arc::new(trogon_atlas_testsupport::shared().await.store().await);
        let service = trogon_atlas_server::service::EventModelServiceImpl::try_new(
            store.clone(),
            trogon_atlas_server::metrics::ServerMetrics::default(),
        )
        .unwrap();
        let mut client = support::two_tenants::serve_with_registry(
            service,
            support::two_tenants::make_registry(),
        )
        .await;
        let own = register_ns(&mut client, ACME_TOKEN).await;
        let hidden = register_ns(&mut client, BETA_TOKEN).await;
        Self {
            client,
            store,
            own,
            hidden,
        }
    }

    async fn seed(&self, kind: pb::entity::Kind) {
        let entity = pb::Entity {
            kind: Some(kind),
            ..Default::default()
        };
        self.store
            .create(
                entity_kind(&entity).unwrap(),
                &entity,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }

    fn model(&self, members: Vec<pb::EntityRef>) -> pb::EventModel {
        pb::EventModel {
            id: Some(id(&self.own, "model")),
            title: "Model".into(),
            members,
            ..Default::default()
        }
    }

    async fn validate(
        &mut self,
        model: &pb::EventModel,
        token: &str,
    ) -> Vec<Vec<pb::ValidationIssue>> {
        let mut results = Vec::new();
        for inline in [true, false] {
            results.push(
                self.client
                    .validate_event_model(authed(
                        pb::ValidateEventModelRequest {
                            event_model: inline.then(|| model.clone()),
                            event_model_id: (!inline).then(|| model.id.clone().unwrap()),
                        },
                        token,
                    ))
                    .await
                    .unwrap()
                    .into_inner()
                    .issues,
            );
        }
        results.push(
            self.client
                .validate_project(authed(
                    pb::ValidateProjectRequest {
                        scope: Some(pb::validate_project_request::Scope::Namespace(
                            self.own.clone(),
                        )),
                    },
                    token,
                ))
                .await
                .unwrap()
                .into_inner()
                .reports
                .into_iter()
                .flat_map(|report| report.issues)
                .collect(),
        );
        results
    }
}

fn count(issues: &[pb::ValidationIssue], code: &str) -> usize {
    issues.iter().filter(|issue| issue.code == code).count()
}

fn fields(names: &[&str]) -> Vec<pb::FieldSpec> {
    names
        .iter()
        .map(|name| pb::FieldSpec {
            name: (*name).into(),
            r#type: Some(pb::FieldType {
                kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
            }),
            ..Default::default()
        })
        .collect()
}

fn count_subject(issues: &[pb::ValidationIssue], code: &str, slug: &str) -> usize {
    issues
        .iter()
        .filter(|issue| {
            issue.code == code
                && issue
                    .subject
                    .as_ref()
                    .and_then(|subject| subject.id.as_ref())
                    .is_some_and(|id| id.slug == slug)
        })
        .count()
}

fn scoped_signature(issues: &[pb::ValidationIssue], own: &str, hidden: &str) -> Vec<String> {
    assert!(issues
        .iter()
        .all(|issue| !issue.message.contains(HIDDEN_DETAIL)));
    assert!(issues
        .iter()
        .filter(|issue| issue.code == "REF_OUTSIDE_VISIBLE_SCOPE")
        .all(|issue| issue.severity == pb::validation_issue::Severity::Info as i32));
    let mut signature: Vec<_> = issues
        .iter()
        .map(|issue| {
            format!("{issue:?}")
                .replace(own, "visible-namespace")
                .replace(hidden, "excluded-namespace")
        })
        .collect();
    signature.sort();
    signature
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_source_events_do_not_imply_unsourced_read_model_fields() {
    let mut absent_signatures = Vec::new();
    for hidden_exists in [false, true] {
        let mut fixture = Fixture::new().await;
        let model = fixture.model(vec![
            member(pb::EntityKind::ReadModel, &fixture.own, "hidden-source"),
            member(pb::EntityKind::ReadModel, &fixture.own, "visible-source"),
        ]);
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        for (slug, namespace, event_slug) in [
            ("hidden-source", &fixture.hidden, "hidden-event"),
            ("visible-source", &fixture.own, "visible-event"),
        ] {
            fixture
                .seed(pb::entity::Kind::ReadModel(pb::ReadModel {
                    id: Some(id(&fixture.own, slug)),
                    title: slug.into(),
                    source_events: vec![pb::EventRef {
                        id: Some(id(namespace, event_slug)),
                    }],
                    schema: Some(trogon_atlas_core::schema::pack_fields(fields(&["row_id"]))),
                    key: vec!["row_id".into()],
                    ..Default::default()
                }))
                .await;
        }
        fixture
            .seed(pb::entity::Kind::Event(pb::Event {
                id: Some(id(&fixture.own, "visible-event")),
                title: "Visible event".into(),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields(&[]))),
                ..Default::default()
            }))
            .await;
        if hidden_exists {
            fixture
                .seed(pb::entity::Kind::Event(pb::Event {
                    id: Some(id(&fixture.hidden, "hidden-event")),
                    title: HIDDEN_DETAIL.into(),
                    schema: Some(trogon_atlas_core::schema::pack_fields(fields(&["row_id"]))),
                    ..Default::default()
                }))
                .await;
        }
        for (index, issues) in fixture
            .validate(&model, ACME_TOKEN)
            .await
            .iter()
            .enumerate()
        {
            assert_eq!(
                count_subject(issues, "READ_MODEL_FIELD_UNSOURCED", "hidden-source"),
                0,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(issues, "READ_MODEL_FIELD_UNSOURCED", "visible-source"),
                1,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(issues, "REF_OUTSIDE_VISIBLE_SCOPE", "hidden-source"),
                1,
                "{issues:?}"
            );
            let signature = scoped_signature(issues, &fixture.own, &fixture.hidden);
            if hidden_exists {
                assert_eq!(signature, absent_signatures[index]);
            } else {
                absent_signatures.push(signature);
            }
        }
        for issues in fixture.validate(&model, ROOT_TOKEN).await {
            assert_eq!(
                count_subject(&issues, "READ_MODEL_FIELD_UNSOURCED", "hidden-source"),
                usize::from(!hidden_exists),
                "{issues:?}"
            );
            assert_eq!(
                count_subject(&issues, "READ_MODEL_FIELD_UNSOURCED", "visible-source"),
                1,
                "{issues:?}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_source_lane_state_does_not_imply_unsourced_read_model_fields() {
    let mut absent_signatures = Vec::new();
    for hidden_exists in [false, true] {
        let mut fixture = Fixture::new().await;
        let model = fixture.model(vec![
            member(pb::EntityKind::ReadModel, &fixture.own, "view"),
            member(pb::EntityKind::Event, &fixture.own, "observed"),
        ]);
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        fixture
            .seed(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(&fixture.own, "view")),
                title: "View".into(),
                source_events: vec![pb::EventRef {
                    id: Some(id(&fixture.own, "observed")),
                }],
                schema: Some(trogon_atlas_core::schema::pack_fields(fields(&["status"]))),
                key: vec!["status".into()],
                ..Default::default()
            }))
            .await;
        fixture
            .seed(pb::entity::Kind::Event(pb::Event {
                id: Some(id(&fixture.own, "observed")),
                title: "Observed".into(),
                swimlane: Some(pb::SwimlaneRef {
                    id: Some(id(&fixture.hidden, "lane")),
                }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields(&[]))),
                ..Default::default()
            }))
            .await;
        if hidden_exists {
            fixture
                .seed(pb::entity::Kind::Swimlane(pb::Swimlane {
                    id: Some(id(&fixture.hidden, "lane")),
                    title: HIDDEN_DETAIL.into(),
                    state: fields(&["status"]),
                    ..Default::default()
                }))
                .await;
        }
        for (index, issues) in fixture
            .validate(&model, ACME_TOKEN)
            .await
            .iter()
            .enumerate()
        {
            assert_eq!(
                count_subject(issues, "READ_MODEL_FIELD_UNSOURCED", "view"),
                0,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(issues, "REF_OUTSIDE_VISIBLE_SCOPE", "observed"),
                1,
                "{issues:?}"
            );
            let signature = scoped_signature(issues, &fixture.own, &fixture.hidden);
            if hidden_exists {
                assert_eq!(signature, absent_signatures[index]);
            } else {
                absent_signatures.push(signature);
            }
        }
        for issues in fixture.validate(&model, ROOT_TOKEN).await {
            assert_eq!(
                count_subject(&issues, "READ_MODEL_FIELD_UNSOURCED", "view"),
                usize::from(!hidden_exists),
                "{issues:?}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_projection_sources_do_not_imply_missing_keys() {
    let mut absent_signatures = Vec::new();
    for hidden_exists in [false, true] {
        let mut fixture = Fixture::new().await;
        let model = fixture.model(vec![
            member(pb::EntityKind::ReadModelSlice, &fixture.own, "projection"),
            member(pb::EntityKind::ReadModel, &fixture.own, "view"),
        ]);
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        let sources = vec![
            pb::EventRef {
                id: Some(id(&fixture.hidden, "hidden-event")),
            },
            pb::EventRef {
                id: Some(id(&fixture.own, "visible-event")),
            },
        ];
        fixture
            .seed(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(&fixture.own, "view")),
                title: "View".into(),
                source_events: sources.clone(),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields(&["row_id"]))),
                key: vec!["row_id".into()],
                ..Default::default()
            }))
            .await;
        fixture
            .seed(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(&fixture.own, "projection")),
                title: "Projection".into(),
                source_events: sources
                    .into_iter()
                    .map(|event| pb::EventEdge {
                        event: Some(event),
                        ..Default::default()
                    })
                    .collect(),
                read_model: Some(pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id(&fixture.own, "view")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .await;
        fixture
            .seed(pb::entity::Kind::Event(pb::Event {
                id: Some(id(&fixture.own, "visible-event")),
                title: "Visible event".into(),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields(&[]))),
                ..Default::default()
            }))
            .await;
        if hidden_exists {
            fixture
                .seed(pb::entity::Kind::Event(pb::Event {
                    id: Some(id(&fixture.hidden, "hidden-event")),
                    title: HIDDEN_DETAIL.into(),
                    schema: Some(trogon_atlas_core::schema::pack_fields(fields(&["row_id"]))),
                    ..Default::default()
                }))
                .await;
        }
        for (index, issues) in fixture
            .validate(&model, ACME_TOKEN)
            .await
            .iter()
            .enumerate()
        {
            assert_eq!(count(issues, "RM_SLICE_KEY_UNSOURCED"), 1, "{issues:?}");
            assert!(issues
                .iter()
                .filter(|issue| issue.code == "RM_SLICE_KEY_UNSOURCED")
                .all(|issue| issue.subject_field == "source_events[1]"));
            assert_eq!(count(issues, "RM_SLICE_ROLE_MISSING"), 1, "{issues:?}");
            assert_eq!(
                count_subject(issues, "REF_OUTSIDE_VISIBLE_SCOPE", "projection"),
                1,
                "{issues:?}"
            );
            let signature = scoped_signature(issues, &fixture.own, &fixture.hidden);
            if hidden_exists {
                assert_eq!(signature, absent_signatures[index]);
            } else {
                absent_signatures.push(signature);
            }
        }
        for issues in fixture.validate(&model, ROOT_TOKEN).await {
            assert_eq!(
                count(&issues, "RM_SLICE_KEY_UNSOURCED"),
                1 + usize::from(!hidden_exists),
                "{issues:?}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_reliability_connections_do_not_imply_missing_slices() {
    let mut absent_signatures = Vec::new();
    for hidden_exists in [false, true] {
        let mut fixture = Fixture::new().await;
        let variants = [
            "command",
            "projection",
            "reaction",
            "display",
            "missing",
            "wrong-kind",
        ];
        let model = fixture.model(
            variants
                .iter()
                .map(|slug| member(pb::EntityKind::ServiceLevelIndicator, &fixture.own, slug))
                .collect(),
        );
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        fixture
            .seed(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(&fixture.own, "wrong-kind")),
                title: "Wrong kind".into(),
                ..Default::default()
            }))
            .await;
        for slug in variants {
            let namespace = if ["missing", "wrong-kind"].contains(&slug) {
                &fixture.own
            } else {
                &fixture.hidden
            };
            let target = Some(pb::SliceRef {
                id: Some(id(namespace, slug)),
            });
            let connection = match slug {
                "command" => {
                    pb::connection::Kind::CommandHandling(pb::connection::CommandHandling {
                        command_slice: target,
                        events: Vec::new(),
                    })
                }
                "projection" => pb::connection::Kind::Projection(pb::connection::Projection {
                    read_model_slice: target,
                    source_events: Vec::new(),
                }),
                "display" => {
                    pb::connection::Kind::Display(pb::connection::Display { ui_slice: target })
                }
                _ => pb::connection::Kind::Reaction(pb::connection::Reaction {
                    automation_slice: target,
                }),
            };
            fixture
                .seed(pb::entity::Kind::ServiceLevelIndicator(
                    pb::ServiceLevelIndicator {
                        id: Some(id(&fixture.own, slug)),
                        title: slug.into(),
                        connection: Some(pb::Connection {
                            kind: Some(connection),
                        }),
                        measure: Some(pb::service_level_indicator::Measure::Latency(
                            pb::service_level_indicator::Latency {},
                        )),
                        ..Default::default()
                    },
                ))
                .await;
            if hidden_exists && namespace == &fixture.hidden {
                let slice_id = Some(id(&fixture.hidden, slug));
                let kind = match slug {
                    "command" => pb::entity::Kind::CommandSlice(pb::CommandSlice {
                        id: slice_id,
                        title: HIDDEN_DETAIL.into(),
                        ..Default::default()
                    }),
                    "projection" => pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                        id: slice_id,
                        title: HIDDEN_DETAIL.into(),
                        ..Default::default()
                    }),
                    "display" => pb::entity::Kind::UiSlice(pb::UiSlice {
                        id: slice_id,
                        title: HIDDEN_DETAIL.into(),
                        ..Default::default()
                    }),
                    _ => pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                        id: slice_id,
                        title: HIDDEN_DETAIL.into(),
                        ..Default::default()
                    }),
                };
                fixture.seed(kind).await;
            }
        }
        for (index, issues) in fixture
            .validate(&model, ACME_TOKEN)
            .await
            .iter()
            .enumerate()
        {
            for slug in ["command", "projection", "reaction", "display"] {
                assert_eq!(
                    count_subject(issues, "SLI_CONNECTION_UNRESOLVED", slug),
                    0,
                    "{issues:?}"
                );
                assert_eq!(
                    count_subject(issues, "REF_OUTSIDE_VISIBLE_SCOPE", slug),
                    1,
                    "{issues:?}"
                );
                assert_eq!(
                    count_subject(issues, "SLI_SIGNAL_MISMATCH", slug),
                    1,
                    "{issues:?}"
                );
            }
            for slug in ["missing", "wrong-kind"] {
                assert_eq!(
                    count_subject(issues, "SLI_CONNECTION_UNRESOLVED", slug),
                    1,
                    "{issues:?}"
                );
            }
            let signature = scoped_signature(issues, &fixture.own, &fixture.hidden);
            if hidden_exists {
                assert_eq!(signature, absent_signatures[index]);
            } else {
                absent_signatures.push(signature);
            }
        }
        for issues in fixture.validate(&model, ROOT_TOKEN).await {
            for slug in ["command", "projection", "reaction", "display"] {
                assert_eq!(
                    count_subject(&issues, "SLI_CONNECTION_UNRESOLVED", slug),
                    usize::from(!hidden_exists),
                    "{issues:?}"
                );
            }
            for slug in ["missing", "wrong-kind"] {
                assert_eq!(
                    count_subject(&issues, "SLI_CONNECTION_UNRESOLVED", slug),
                    1,
                    "{issues:?}"
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_indicator_measure_does_not_imply_threshold_mismatch() {
    let mut absent_signatures = Vec::new();
    for hidden_exists in [false, true] {
        let mut fixture = Fixture::new().await;
        let model = fixture.model(vec![
            member(
                pb::EntityKind::ServiceLevelObjective,
                &fixture.own,
                "hidden-indicator",
            ),
            member(
                pb::EntityKind::ServiceLevelObjective,
                &fixture.own,
                "visible-indicator",
            ),
        ]);
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        for (slug, namespace, ratio) in [
            ("hidden-indicator", &fixture.hidden, 1.5),
            ("visible-indicator", &fixture.own, 0.99),
        ] {
            fixture
                .seed(pb::entity::Kind::ServiceLevelObjective(
                    pb::ServiceLevelObjective {
                        id: Some(id(&fixture.own, slug)),
                        title: slug.into(),
                        indicator: Some(pb::ServiceLevelIndicatorRef {
                            id: Some(id(namespace, slug)),
                        }),
                        objectives: vec![pb::service_level_objective::Objective {
                            threshold: Some(pb::LatencyThreshold::default()),
                            target: Some(pb::Target { ratio }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                ))
                .await;
        }
        fixture
            .seed(pb::entity::Kind::ServiceLevelIndicator(
                pb::ServiceLevelIndicator {
                    id: Some(id(&fixture.own, "visible-indicator")),
                    title: "Completion".into(),
                    measure: Some(pb::service_level_indicator::Measure::Completion(
                        pb::service_level_indicator::Completion::default(),
                    )),
                    ..Default::default()
                },
            ))
            .await;
        if hidden_exists {
            fixture
                .seed(pb::entity::Kind::ServiceLevelIndicator(
                    pb::ServiceLevelIndicator {
                        id: Some(id(&fixture.hidden, "hidden-indicator")),
                        title: HIDDEN_DETAIL.into(),
                        measure: Some(pb::service_level_indicator::Measure::Latency(
                            pb::service_level_indicator::Latency {},
                        )),
                        ..Default::default()
                    },
                ))
                .await;
        }
        for (index, issues) in fixture
            .validate(&model, ACME_TOKEN)
            .await
            .iter()
            .enumerate()
        {
            assert_eq!(
                count_subject(issues, "SLO_THRESHOLD_MISMATCH", "hidden-indicator"),
                0,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(issues, "SLO_THRESHOLD_MISMATCH", "visible-indicator"),
                1,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(issues, "SLO_TARGET_RANGE", "hidden-indicator"),
                1,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(issues, "SLO_UNALERTED", "hidden-indicator"),
                1,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(issues, "REF_OUTSIDE_VISIBLE_SCOPE", "hidden-indicator"),
                1,
                "{issues:?}"
            );
            let signature = scoped_signature(issues, &fixture.own, &fixture.hidden);
            if hidden_exists {
                assert_eq!(signature, absent_signatures[index]);
            } else {
                absent_signatures.push(signature);
            }
        }
        for issues in fixture.validate(&model, ROOT_TOKEN).await {
            assert_eq!(
                count_subject(&issues, "SLO_THRESHOLD_MISMATCH", "hidden-indicator"),
                usize::from(!hidden_exists),
                "{issues:?}"
            );
            assert_eq!(
                count_subject(&issues, "SLO_THRESHOLD_MISMATCH", "visible-indicator"),
                1,
                "{issues:?}"
            );
            assert_eq!(
                count_subject(&issues, "SLO_TARGET_RANGE", "hidden-indicator"),
                1,
                "{issues:?}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_members_do_not_produce_content_findings() {
    for hidden_exists in [false, true] {
        let mut fixture = Fixture::new().await;
        let model = fixture.model(vec![
            member(pb::EntityKind::Command, &fixture.hidden, "issue"),
            member(pb::EntityKind::Event, &fixture.hidden, "issued"),
        ]);
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        if hidden_exists {
            fixture
                .seed(pb::entity::Kind::Command(pb::Command {
                    id: Some(id(&fixture.hidden, "issue")),
                    title: HIDDEN_DETAIL.into(),
                    ..Default::default()
                }))
                .await;
            fixture
                .seed(pb::entity::Kind::Event(pb::Event {
                    id: Some(id(&fixture.hidden, "issued")),
                    title: HIDDEN_DETAIL.into(),
                    ..Default::default()
                }))
                .await;
        }
        for issues in fixture.validate(&model, ACME_TOKEN).await {
            assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 2, "{issues:?}");
            assert!(
                issues
                    .iter()
                    .all(|issue| issue.code == "REF_OUTSIDE_VISIBLE_SCOPE"),
                "unreadable members must not acquire content findings: {issues:?}"
            );
            assert!(issues.iter().all(|issue| {
                issue.severity == pb::validation_issue::Severity::Info as i32
                    && !issue.message.contains(HIDDEN_DETAIL)
            }));
        }
        for issues in fixture.validate(&model, ROOT_TOKEN).await {
            assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 0, "{issues:?}");
            assert_eq!(count(&issues, "COMMAND_HAS_NO_TRIGGER"), 1, "{issues:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_strategic_references_are_informational() {
    for hidden_exists in [false, true] {
        let mut fixture = Fixture::new().await;
        let model = fixture.model(Vec::new());
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        fixture
            .seed(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                id: Some(id(&fixture.own, &fixture.own)),
                title: "Context".into(),
                realizes: vec![pb::SubdomainRef {
                    id: Some(id(&fixture.hidden, "capability")),
                }],
                relationships: vec![pb::bounded_context::ContextRelationship {
                    upstream: Some(pb::BoundedContextRef {
                        id: Some(id(&fixture.hidden, &fixture.hidden)),
                    }),
                    intent: pb::bounded_context::context_relationship::Intent::SeparateWays as i32,
                    ..Default::default()
                }],
                ..Default::default()
            }))
            .await;
        if hidden_exists {
            fixture
                .seed(pb::entity::Kind::Subdomain(pb::Subdomain {
                    id: Some(id(&fixture.hidden, "capability")),
                    title: HIDDEN_DETAIL.into(),
                    classification: pb::subdomain::Classification::Supporting as i32,
                    ..Default::default()
                }))
                .await;
            fixture
                .seed(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                    id: Some(id(&fixture.hidden, &fixture.hidden)),
                    title: HIDDEN_DETAIL.into(),
                    ..Default::default()
                }))
                .await;
        }
        for issues in fixture.validate(&model, ACME_TOKEN).await {
            assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 2, "{issues:?}");
            assert!(issues
                .iter()
                .filter(|issue| issue.code == "REF_OUTSIDE_VISIBLE_SCOPE")
                .all(|issue| issue.severity == pb::validation_issue::Severity::Info as i32));
            assert!(issues
                .iter()
                .all(|issue| !issue.message.contains(HIDDEN_DETAIL)));
            assert!(
                issues
                    .iter()
                    .all(|issue| issue.severity != pb::validation_issue::Severity::Error as i32),
                "excluded strategic references are unknown: {issues:?}"
            );
        }
        for issues in fixture.validate(&model, ROOT_TOKEN).await {
            assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 0, "{issues:?}");
            let expected_missing = usize::from(!hidden_exists);
            assert_eq!(
                count(&issues, "REALIZES_DANGLING"),
                expected_missing,
                "{issues:?}"
            );
            assert_eq!(
                count(&issues, "RELATIONSHIP_DANGLING"),
                expected_missing,
                "{issues:?}"
            );
            assert!(issues
                .iter()
                .filter(|issue| matches!(
                    issue.code.as_str(),
                    "REALIZES_DANGLING" | "RELATIONSHIP_DANGLING"
                ))
                .all(|issue| issue.severity == pb::validation_issue::Severity::Error as i32));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn visible_missing_members_remain_errors() {
    let mut fixture = Fixture::new().await;
    let model = fixture.model(vec![member(
        pb::EntityKind::Command,
        &fixture.own,
        "missing",
    )]);
    fixture
        .seed(pb::entity::Kind::EventModel(model.clone()))
        .await;
    for issues in fixture.validate(&model, ACME_TOKEN).await {
        assert_eq!(count(&issues, "MEMBER_NOT_FOUND"), 1, "{issues:?}");
        assert_eq!(count(&issues, "COMMAND_HAS_NO_TRIGGER"), 1, "{issues:?}");
        assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 0, "{issues:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn visible_untriggered_commands_remain_errors() {
    let mut fixture = Fixture::new().await;
    let model = fixture.model(vec![member(pb::EntityKind::Command, &fixture.own, "issue")]);
    fixture
        .seed(pb::entity::Kind::EventModel(model.clone()))
        .await;
    fixture
        .seed(pb::entity::Kind::Command(pb::Command {
            id: Some(id(&fixture.own, "issue")),
            title: "Issue".into(),
            ..Default::default()
        }))
        .await;
    for issues in fixture.validate(&model, ACME_TOKEN).await {
        assert_eq!(count(&issues, "MEMBER_NOT_FOUND"), 0, "{issues:?}");
        assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 0, "{issues:?}");
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "COMMAND_HAS_NO_TRIGGER"
                    && issue.severity == pb::validation_issue::Severity::Error as i32),
            "{issues:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn visible_strategic_defects_remain_errors() {
    for foreign_domain in [false, true] {
        let mut fixture = Fixture::new().await;
        let model = fixture.model(Vec::new());
        fixture
            .seed(pb::entity::Kind::EventModel(model.clone()))
            .await;
        let domain_namespace = if foreign_domain {
            &fixture.hidden
        } else {
            &fixture.own
        };
        fixture
            .seed(pb::entity::Kind::Subdomain(pb::Subdomain {
                id: Some(id(&fixture.own, "capability")),
                title: "Capability".into(),
                domain: Some(pb::DomainRef {
                    id: Some(id(domain_namespace, domain_namespace)),
                }),
                classification: pb::subdomain::Classification::Supporting as i32,
                ..Default::default()
            }))
            .await;
        for issues in fixture.validate(&model, ACME_TOKEN).await {
            let expected_code = if foreign_domain {
                "SUBDOMAIN_FOREIGN_DOMAIN"
            } else {
                "SUBDOMAIN_DANGLING_DOMAIN"
            };
            assert!(
                issues.iter().any(|issue| issue.code == expected_code
                    && issue.severity == pb::validation_issue::Severity::Error as i32),
                "{issues:?}"
            );
            assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 0, "{issues:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_member_references_remain_errors() {
    let mut fixture = Fixture::new().await;
    let model = fixture.model(vec![
        pb::EntityRef {
            kind: pb::EntityKind::Command as i32,
            id: None,
        },
        pb::EntityRef {
            kind: i32::MAX,
            id: Some(id(&fixture.hidden, "invalid")),
        },
    ]);
    fixture
        .seed(pb::entity::Kind::EventModel(model.clone()))
        .await;
    for issues in fixture.validate(&model, ACME_TOKEN).await {
        for code in ["MISSING_ID", "INVALID_KIND"] {
            assert!(
                issues.iter().any(|issue| issue.code == code
                    && issue.severity == pb::validation_issue::Severity::Error as i32),
                "{issues:?}"
            );
        }
        assert_eq!(count(&issues, "REF_OUTSIDE_VISIBLE_SCOPE"), 0, "{issues:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_scenario_references_are_not_dangling() {
    let mut fixture = Fixture::new().await;
    let own = fixture.own.clone();
    let hidden = fixture.hidden.clone();
    let command = || pb::CommandRef {
        id: Some(id(&own, "approve-review")),
    };
    fixture
        .seed(pb::entity::Kind::Command(pb::Command {
            id: Some(id(&own, "approve-review")),
            title: "Approve review".into(),
            ..Default::default()
        }))
        .await;
    fixture
        .seed(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(&own, "approve-review")),
            title: "Approve review".into(),
            command: Some(pb::CommandEdge {
                command: Some(command()),
                ..Default::default()
            }),
            scenarios: vec![pb::CommandScenario {
                id: "s1".into(),
                title: "submitted review is approved".into(),
                given: vec![pb::EventExample {
                    event: Some(pb::EventRef {
                        id: Some(id(&hidden, "review.submitted")),
                    }),
                    payload: None,
                }],
                when: Some(pb::CommandExample {
                    command: Some(command()),
                    payload: None,
                }),
                then: Some(pb::command_scenario::Then::Reject(pb::Rejection {
                    reason_code: "already_decided".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let model = fixture.model(vec![member(
        pb::EntityKind::CommandSlice,
        &own,
        "approve-review",
    )]);
    fixture
        .seed(pb::entity::Kind::EventModel(model.clone()))
        .await;

    for issues in fixture.validate(&model, ACME_TOKEN).await {
        assert_eq!(count(&issues, "SCENARIO_DANGLING_REF"), 0, "{issues:?}");
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "REF_OUTSIDE_VISIBLE_SCOPE"
                    && issue.subject_field.starts_with("scenarios")),
            "{issues:?}"
        );
    }
    for issues in fixture.validate(&model, ROOT_TOKEN).await {
        assert_eq!(count(&issues, "SCENARIO_DANGLING_REF"), 1, "{issues:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn validation_checks_stored_scenarios_against_the_current_lane() {
    let mut fixture = Fixture::new().await;
    let own = fixture.own.clone();
    let event = |slug: &str| pb::EventRef {
        id: Some(id(&own, slug)),
    };
    let example = |slug: &str| pb::EventExample {
        event: Some(event(slug)),
        payload: None,
    };
    for slug in ["review.submitted", "review.approved"] {
        fixture
            .seed(pb::entity::Kind::Event(pb::Event {
                id: Some(id(&own, slug)),
                title: slug.into(),
                swimlane: Some(pb::SwimlaneRef {
                    id: Some(id(&own, "review")),
                }),
                ..Default::default()
            }))
            .await;
    }
    fixture
        .seed(pb::entity::Kind::Swimlane(pb::Swimlane {
            id: Some(id(&own, "review")),
            title: "Review".into(),
            transitions: vec![pb::swimlane::Transition {
                after: Some(event("review.submitted")),
                next: vec![pb::swimlane::transition::Next {
                    event: Some(event("review.rejected")),
                    ..Default::default()
                }],
            }],
            ..Default::default()
        }))
        .await;
    fixture
        .seed(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(&own, "approve-review")),
            title: "Approve review".into(),
            scenarios: vec![pb::CommandScenario {
                id: "s1".into(),
                title: "submitted review is approved".into(),
                given: vec![example("review.submitted")],
                then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
                    events: vec![example("review.approved")],
                })),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let model = fixture.model(vec![member(
        pb::EntityKind::CommandSlice,
        &own,
        "approve-review",
    )]);
    fixture
        .seed(pb::entity::Kind::EventModel(model.clone()))
        .await;

    for issues in fixture.validate(&model, ACME_TOKEN).await {
        assert_eq!(
            count(&issues, "SCENARIO_TRANSITION_VIOLATION"),
            1,
            "{issues:?}"
        );
    }
    let coverage = fixture
        .client
        .validate_event_model(authed(
            pb::ValidateEventModelRequest {
                event_model: None,
                event_model_id: model.id.clone(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .coverage
        .unwrap();
    assert!(
        coverage.checks.iter().any(|c| c.check_id == "scenarios"
            && c.state == pb::validation_check_result::State::Completed as i32
            && c.subjects_examined == 1),
        "{coverage:?}"
    );
}
