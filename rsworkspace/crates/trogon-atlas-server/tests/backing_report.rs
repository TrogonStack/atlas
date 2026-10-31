#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer, BackingIncompleteReason as Reason,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "backing-admin";
const LIMITED: &str = "backing-limited";
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

async fn client(cap: Option<usize>) -> Client {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.limited]
role = "reader"
parent = "visible"
tokens = ["{LIMITED}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let mut service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    if let Some(cap) = cap {
        service = service.with_snapshot_cap(cap);
    }
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

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
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

fn event(namespace: &str, slug: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
    entity(pb::entity::Kind::Event(pb::Event {
        id: Some(id(namespace, slug)),
        title: slug.into(),
        metadata,
        ..Default::default()
    }))
}

fn code_ref(repo: &str, path: &str) -> prost_types::Any {
    prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation".into(),
        value: prost::Message::encode_to_vec(&pb::CodeRefAnnotation {
            repo: repo.into(),
            path: path.into(),
            ..Default::default()
        }),
    }
}

fn subject(target: &pb::Entity) -> pb::SubjectAssociation {
    pb::SubjectAssociation {
        subject: Some(pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Entity(
                trogon_atlas_server::conv::entity_to_ref(target).unwrap(),
            )),
        }),
        ..Default::default()
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

fn document(namespace: &str, slug: &str, targets: &[&pb::Entity]) -> pb::Entity {
    let content = "# Note\n";
    entity(pb::entity::Kind::Document(pb::Document {
        id: Some(id(namespace, slug)),
        title: Some(title(slug)),
        subjects: targets.iter().map(|target| subject(target)).collect(),
        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            media_type: Some(pb::MediaType {
                value: "text/markdown".into(),
            }),
            content: content.into(),
            digest: Some(trogon_atlas_core::specification::content_digest(content)),
        })),
        ..Default::default()
    }))
}

fn decision(
    namespace: &str,
    slug: &str,
    target: &pb::Entity,
    status: pb::DecisionStatus,
) -> pb::Entity {
    let adopted = matches!(
        status,
        pb::DecisionStatus::Accepted | pb::DecisionStatus::Superseded
    );
    let option_id = pb::LocalValueId {
        value: uuid::Uuid::now_v7().to_string(),
    };
    entity(pb::entity::Kind::Decision(pb::Decision {
        id: Some(id(namespace, slug)),
        title: Some(title(slug)),
        subjects: vec![subject(target)],
        status: status as i32,
        context: Some(text("Context")),
        rationale: Some(text("Rationale")),
        options: vec![pb::DecisionOption {
            option_id: Some(option_id.clone()),
            title: Some(title("Option")),
            description: Some(text("Option text")),
        }],
        selected_option: adopted.then_some(option_id),
        consequences: if adopted {
            vec![pb::DecisionConsequence {
                kind: pb::decision_consequence::Kind::Risk as i32,
                text: Some(text("Consequence")),
            }]
        } else {
            Vec::new()
        },
        ..Default::default()
    }))
}

fn superseded(namespace: &str, slug: &str, target: &pb::Entity, by: &pb::Entity) -> pb::Entity {
    let mut value = decision(namespace, slug, target, pb::DecisionStatus::Superseded);
    if let Some(pb::entity::Kind::Decision(decision)) = &mut value.kind {
        decision.replaced_by = Some(pb::DecisionReplacement {
            decision: Some(pb::DecisionRef {
                id: trogon_atlas_server::conv::entity_to_ref(by).unwrap().id,
            }),
            ..Default::default()
        });
    }
    value
}

async fn put(client: &mut Client, value: pb::Entity, branch: Option<&str>) -> pb::Entity {
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
        .entity
        .unwrap()
}

async fn report(
    client: &mut Client,
    namespaces: &[&str],
    findings: bool,
    token: &str,
    branch: Option<&str>,
) -> pb::GetBackingReportResponse {
    client
        .get_backing_report(request(
            pb::GetBackingReportRequest {
                namespaces: namespaces.iter().map(|ns| (*ns).to_owned()).collect(),
                include_unbacked_findings: findings,
            },
            token,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn reasons(response: &pb::GetBackingReportResponse) -> Vec<Reason> {
    response
        .incomplete_reasons
        .iter()
        .map(|reason| Reason::try_from(*reason).unwrap())
        .collect()
}

fn counts(totals: Option<&pb::BackingTotals>) -> (u32, u32, u32, u32) {
    let totals = totals.unwrap();
    (totals.total, totals.backed, totals.unbacked, totals.unknown)
}

#[tokio::test(flavor = "multi_thread")]
async fn backing_counts_each_subject_once_and_ignores_unadopted_decisions() {
    let mut client = client(None).await;
    let ns = "backing";
    let rich = put(
        &mut client,
        event(ns, "rich", vec![code_ref("repo", "src/a.rs")]),
        None,
    )
    .await;
    let rejected = put(&mut client, event(ns, "rejected", Vec::new()), None).await;
    let proposed = put(&mut client, event(ns, "proposed", Vec::new()), None).await;
    let replaced = put(&mut client, event(ns, "replaced", Vec::new()), None).await;
    let current = put(&mut client, event(ns, "current", Vec::new()), None).await;
    for slug in ["first", "second"] {
        put(
            &mut client,
            decision(ns, slug, &rich, pb::DecisionStatus::Accepted),
            None,
        )
        .await;
    }
    put(&mut client, document(ns, "note", &[&rich]), None).await;
    put(&mut client, document(ns, "again", &[&rich]), None).await;
    put(
        &mut client,
        decision(ns, "no", &rejected, pb::DecisionStatus::Rejected),
        None,
    )
    .await;
    put(
        &mut client,
        decision(ns, "maybe", &proposed, pb::DecisionStatus::Proposed),
        None,
    )
    .await;
    let successor = put(
        &mut client,
        decision(ns, "successor", &current, pb::DecisionStatus::Accepted),
        None,
    )
    .await;
    put(
        &mut client,
        superseded(ns, "old", &replaced, &successor),
        None,
    )
    .await;

    let quiet = report(&mut client, &[ns], false, ADMIN, None).await;
    assert!(quiet.findings.is_empty(), "findings are opt-in");
    let response = report(&mut client, &[ns], true, ADMIN, None).await;
    assert_eq!(counts(response.totals.as_ref()), (5, 2, 3, 0));
    let totals = response.totals.unwrap();
    assert_eq!(totals.decision.unwrap().present, 2);
    assert_eq!(totals.document.unwrap().present, 1);
    assert_eq!(totals.code_ref.unwrap().present, 1);
    assert_eq!(totals.unbacked_rate, Some(0.6));
    assert_eq!(response.incomplete_reasons, Vec::<i32>::new());
    assert_eq!(response.kinds.len(), 1);
    assert_eq!(response.kinds[0].kind, pb::EntityKind::Event as i32);
    assert_eq!(response.namespaces, vec![ns.to_owned()]);
    let flagged: Vec<&str> = response
        .findings
        .iter()
        .map(|finding| {
            assert_eq!(finding.code, "ENTITY_UNBACKED");
            assert_eq!(finding.rule_category, "backing");
            finding
                .subject
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .slug
                .as_str()
        })
        .collect();
    assert_eq!(flagged, ["proposed", "rejected", "replaced"]);

    let snapshot = client
        .get_snapshot_id(request(pb::GetSnapshotIdRequest::default(), ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.snapshot_id, snapshot.snapshot_id);
    assert!(!response.snapshot_truncated);
    assert!(!response.visibility.unwrap().restricted);
}

#[tokio::test(flavor = "multi_thread")]
async fn incomplete_code_reference_and_empty_scope_have_no_rate() {
    let mut client = client(None).await;
    put(
        &mut client,
        event("partial", "bare", vec![code_ref("repo", "")]),
        None,
    )
    .await;
    let partial = report(&mut client, &["partial"], true, ADMIN, None).await;
    assert_eq!(counts(partial.totals.as_ref()), (1, 0, 0, 1));
    assert_eq!(partial.totals.unwrap().unbacked_rate, None);
    assert_eq!(reasons(&partial), vec![Reason::UnresolvedCodeRef]);
    assert!(
        partial.findings.is_empty(),
        "unknown is never reported as unbacked"
    );

    let empty = report(&mut client, &["nothing-here"], false, ADMIN, None).await;
    assert_eq!(counts(empty.totals.as_ref()), (0, 0, 0, 0));
    assert_eq!(empty.totals.unwrap().unbacked_rate, None);
    assert_eq!(empty.kinds, Vec::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_documents_make_backing_unknown_for_restricted_callers() {
    let mut client = client(None).await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("backing-visible", "visible"),
        ("backing-private", "hidden"),
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
    let (open, private) = (namespaces[0].as_str(), namespaces[1].as_str());
    let target = put(&mut client, event(open, "target", Vec::new()), None).await;
    put(&mut client, document(private, "secret", &[&target]), None).await;

    let admin = report(&mut client, &[open], false, ADMIN, None).await;
    assert_eq!(counts(admin.totals.as_ref()), (1, 1, 0, 0));

    let limited = report(&mut client, &[open], true, LIMITED, None).await;
    assert_eq!(counts(limited.totals.as_ref()), (1, 0, 0, 1));
    assert_eq!(limited.totals.unwrap().unbacked_rate, None);
    assert_eq!(reasons(&limited), vec![Reason::VisibilityRestricted]);
    assert_eq!(limited.findings, Vec::new());
    let visibility = limited.visibility.unwrap();
    assert!(visibility.restricted);
    assert!(visibility.authorized_namespaces.contains(&open.to_owned()));
    assert!(!visibility
        .authorized_namespaces
        .contains(&private.to_owned()));

    let refused = client
        .get_backing_report(request(
            pb::GetBackingReportRequest {
                namespaces: vec![private.to_owned()],
                include_unbacked_findings: false,
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn truncated_snapshot_never_proves_absence() {
    let mut client = client(Some(1)).await;
    put(&mut client, event("cut", "one", Vec::new()), None).await;
    put(&mut client, event("cut", "two", Vec::new()), None).await;
    let response = report(&mut client, &[], true, ADMIN, None).await;
    assert!(response.snapshot_truncated);
    let totals = response.totals.as_ref().unwrap();
    assert_eq!(totals.unbacked, 0);
    assert_eq!(totals.unknown, totals.total);
    assert_eq!(totals.unbacked_rate, None);
    assert_eq!(reasons(&response), vec![Reason::SnapshotTruncated]);
    assert_eq!(response.findings, Vec::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn report_is_bound_to_the_requested_branch() {
    let mut client = client(None).await;
    let target = put(&mut client, event("branchy", "target", Vec::new()), None).await;
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "backing-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    put(
        &mut client,
        document("branchy", "note", &[&target]),
        Some("backing-branch"),
    )
    .await;

    let main = report(&mut client, &["branchy"], false, ADMIN, None).await;
    assert_eq!(counts(main.totals.as_ref()), (1, 0, 1, 0));
    assert_eq!(main.branch, "");
    let branch = report(
        &mut client,
        &["branchy"],
        false,
        ADMIN,
        Some("backing-branch"),
    )
    .await;
    assert_eq!(counts(branch.totals.as_ref()), (1, 1, 0, 0));
    assert_eq!(branch.branch, "backing-branch");
    assert_ne!(main.snapshot_id, branch.snapshot_id);
}
