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
