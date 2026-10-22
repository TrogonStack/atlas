#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "catalog-admin";
const READER: &str = "catalog-reader";
const LIMITED: &str = "catalog-limited";
type Client = EventModelServiceClient<Channel>;
fn request<T>(body: T, token: &str, branch: Option<&str>) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(branch) = branch {
        req.metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    req
}
async fn client() -> Client {
    client_with_cap(5_000).await
}
async fn client_with_cap(cap: usize) -> Client {
    client_with_store(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        cap,
    )
    .await
}
async fn client_with_store(store: Arc<dyn trogon_atlas_store::Store>, cap: usize) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.reader]
role = "reader"
tokens = ["{READER}"]
[principals.limited]
role = "writer"
parent = "visible"
tokens = ["{LIMITED}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(cap);
    let grpc = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(service)),
        BearerAuth::new(Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Client::connect(format!("http://{addr}")).await.unwrap()
}

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "catalog".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}
fn subject(reference: pb::EntityRef) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Entity(reference)),
    }
}
fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}
fn text(value: &str) -> pb::CatalogText {
    pb::CatalogText {
        value: value.into(),
    }
}
fn read_model(slug: &str) -> pb::Entity {
    entity(pb::entity::Kind::ReadModel(pb::ReadModel {
        id: Some(id(slug)),
        title: slug.into(),
        ..Default::default()
    }))
}
fn decision(slug: &str, selected: pb::CatalogSubject, adopted: bool) -> pb::Entity {
    let option_id = pb::LocalValueId {
        value: uuid::Uuid::now_v7().to_string(),
    };
    entity(pb::entity::Kind::Decision(pb::Decision {
        id: Some(id(slug)),
        title: Some(title(slug)),
        subjects: vec![pb::SubjectAssociation {
            subject: Some(selected),
            ..Default::default()
        }],
        status: if adopted {
            pb::DecisionStatus::Accepted
        } else {
            pb::DecisionStatus::Proposed
        } as i32,
        context: Some(text("Choose delivery")),
        rationale: Some(text("Keep delivery explicit")),
        options: vec![pb::DecisionOption {
            option_id: Some(option_id.clone()),
            title: Some(title("Native delivery")),
            description: Some(text("Retain the selected choice")),
        }],
        selected_option: adopted.then_some(option_id),
        consequences: if adopted {
            vec![pb::DecisionConsequence {
                kind: pb::decision_consequence::Kind::Risk as i32,
                text: Some(text("Delivery may need repair")),
            }]
        } else {
            Vec::new()
        },
        ..Default::default()
    }))
}
async fn put(
    client: &mut Client,
    value: pb::Entity,
    branch: Option<&str>,
) -> pb::PutEntityResponse {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(value),
                ..Default::default()
            },
            ADMIN,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn superseded(slug: &str, selected: pb::CatalogSubject, replacement: &pb::Entity) -> pb::Entity {
    let mut owner = decision(slug, selected, true);
    let Some(pb::entity::Kind::Decision(value)) = &mut owner.kind else {
        unreachable!()
    };
    value.status = pb::DecisionStatus::Superseded as i32;
    value.replaced_by = Some(pb::DecisionReplacement {
        decision: Some(pb::DecisionRef {
            id: reference(replacement).id,
        }),
        ..Default::default()
    });
    owner
}
async fn findings(client: &mut Client, branch: Option<&str>) -> Vec<pb::ValidationIssue> {
    client
        .validate_project(request(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    "catalog".into(),
                )),
            },
            READER,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .reports
        .into_iter()
        .flat_map(|report| report.issues)
        .filter(|issue| issue.code == "CATALOG_REFERENCE_UNRESOLVED")
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_replacement_chains_report_changed_terminal_status_in_baseline_and_branch() {
    let mut client = client().await;
    let model = put(&mut client, read_model("model"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&model));
    let terminal = put(
        &mut client,
        decision("terminal", selected.clone(), true),
        None,
    )
    .await
    .entity
    .unwrap();
    let middle = put(
        &mut client,
        superseded("middle", selected.clone(), &terminal),
        None,
    )
    .await
    .entity
    .unwrap();
    let first = put(&mut client, superseded("first", selected, &middle), None)
        .await
        .entity
        .unwrap();
    assert_eq!(findings(&mut client, None).await, []);
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "replacement-change".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let mut changed = terminal.clone();
    let Some(pb::entity::Kind::Decision(value)) = &mut changed.kind else {
        unreachable!()
    };
    value.status = pb::DecisionStatus::Withdrawn as i32;
    value.selected_option = None;
    value.consequences.clear();
    for branch in [Some("replacement-change"), None] {
        put(&mut client, changed.clone(), branch).await;
        let issues = findings(&mut client, branch).await;
        for owner in [&middle, &first] {
            let issue = issues
                .iter()
                .find(|issue| {
                    issue.subject.as_ref() == Some(&reference(owner))
                        && issue.subject_field == "replaced_by.decision"
                })
                .unwrap_or_else(|| {
                    panic!(
                        "missing replacement diagnostic for {:?} on {branch:?}: {issues:?}",
                        reference(owner)
                    )
                });
            assert_ne!(issue.rule_title, "");
            assert_eq!(issue.severity, pb::validation_issue::Severity::Error as i32);
        }
        if branch.is_some() {
            assert_eq!(findings(&mut client, None).await, []);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn transitive_replacement_hydration_is_exact_beyond_the_snapshot_cap() {
    let mut client = client_with_cap(2).await;
    let model = put(&mut client, read_model("model"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&model));
    let terminal = put(
        &mut client,
        decision("z-terminal", selected.clone(), true),
        None,
    )
    .await
    .entity
    .unwrap();
    let middle = put(
        &mut client,
        superseded("y-middle", selected.clone(), &terminal),
        None,
    )
    .await
    .entity
    .unwrap();
    let first = put(&mut client, superseded("a-first", selected, &middle), None)
        .await
        .entity
        .unwrap();
    assert_eq!(findings(&mut client, None).await, []);
    let mut changed = terminal;
    let Some(pb::entity::Kind::Decision(value)) = &mut changed.kind else {
        unreachable!()
    };
    value.status = pb::DecisionStatus::Withdrawn as i32;
    value.selected_option = None;
    value.consequences.clear();
    put(&mut client, changed, None).await;
    let issues = findings(&mut client, None).await;
    assert!(
        issues
            .iter()
            .any(|issue| issue.subject.as_ref() == Some(&reference(&first))
                && issue.subject_field == "replaced_by.decision"),
        "{issues:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_replacement_terminal_remains_generic_for_visible_predecessors() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("catalog-visible", "visible"),
        ("catalog-private", "hidden"),
    ] {
        namespaces.push(
            client
                .register_namespace(request(
                    pb::RegisterNamespaceRequest {
                        name: name.into(),
                        parent: parent.into(),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    let mut model = read_model("model");
    if let Some(pb::entity::Kind::ReadModel(value)) = &mut model.kind {
        value
            .id
            .as_mut()
            .unwrap()
            .namespace
            .clone_from(&namespaces[0]);
    }
    let model = put(&mut client, model, None).await.entity.unwrap();
    let selected = subject(reference(&model));
    let mut terminal = decision("private-terminal", selected.clone(), true);
    if let Some(pb::entity::Kind::Decision(value)) = &mut terminal.kind {
        value
            .id
            .as_mut()
            .unwrap()
            .namespace
            .clone_from(&namespaces[1]);
    }
    let terminal = put(&mut client, terminal, None).await.entity.unwrap();
    let mut middle = superseded("middle", selected.clone(), &terminal);
    if let Some(pb::entity::Kind::Decision(value)) = &mut middle.kind {
        value
            .id
            .as_mut()
            .unwrap()
            .namespace
            .clone_from(&namespaces[0]);
    }
    let middle = put(&mut client, middle, None).await.entity.unwrap();
    let mut first = superseded("first", selected, &middle);
    if let Some(pb::entity::Kind::Decision(value)) = &mut first.kind {
        value
            .id
            .as_mut()
            .unwrap()
            .namespace
            .clone_from(&namespaces[0]);
    }
    let first = put(&mut client, first, None).await.entity.unwrap();
    let report = client
        .validate_project(request(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    namespaces[0].clone(),
                )),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    for owner in [&first, &middle] {
        let issue = report
            .reports
            .iter()
            .flat_map(|report| &report.issues)
            .find(|issue| {
                issue.code == "CATALOG_REFERENCE_UNRESOLVED"
                    && issue.subject.as_ref() == Some(&reference(owner))
                    && issue.subject_field == "replaced_by.decision"
            })
            .expect("hidden terminal prevents confirmation");
        assert_eq!(
            issue.message,
            "Replacement chain is absent, stale or unavailable"
        );
    }
    let report_json = format!("{report:?}");
    assert!(!report_json.contains("private-terminal"));
    assert!(!report_json.contains(&namespaces[1]));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_replacement_emits_one_selection_diagnostic() {
    let mut client = client().await;
    let model = put(&mut client, read_model("model"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&model));
    let terminal = put(
        &mut client,
        decision("terminal", selected.clone(), true),
        None,
    )
    .await
    .entity
    .unwrap();
    let first = put(&mut client, superseded("first", selected, &terminal), None)
        .await
        .entity
        .unwrap();
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Decision as i32,
                id: reference(&terminal).id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let issues = findings(&mut client, None).await;
    assert_eq!(
        issues
            .iter()
            .filter(|issue| issue.subject.as_ref() == Some(&reference(&first))
                && issue.subject_field == "replaced_by.decision")
            .count(),
        1,
        "{issues:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_replacement_cycle_is_a_bounded_diagnostic() {
    use trogon_atlas_store::{Store, WriteContext};
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 5_000).await;
    let model = put(&mut client, read_model("model"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&model));
    let terminal = put(
        &mut client,
        decision("terminal", selected.clone(), true),
        None,
    )
    .await
    .entity
    .unwrap();
    let first = put(&mut client, superseded("first", selected, &terminal), None)
        .await
        .entity
        .unwrap();
    let mut cyclic = terminal;
    let Some(pb::entity::Kind::Decision(value)) = &mut cyclic.kind else {
        unreachable!()
    };
    value.status = pb::DecisionStatus::Superseded as i32;
    value.replaced_by = Some(pb::DecisionReplacement {
        decision: Some(pb::DecisionRef {
            id: reference(&first).id,
        }),
        ..Default::default()
    });
    store
        .put(
            pb::EntityKind::Decision,
            &cyclic,
            false,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let issues = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        findings(&mut client, None),
    )
    .await
    .unwrap();
    assert!(
        issues
            .iter()
            .any(|issue| issue.subject.as_ref() == Some(&reference(&first))
                && issue.subject_field == "replaced_by.decision"),
        "{issues:?}"
    );
}
