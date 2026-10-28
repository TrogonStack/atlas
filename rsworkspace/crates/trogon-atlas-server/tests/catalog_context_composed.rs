#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_core::document_catalog_context::bytes_content_digest;
use trogon_atlas_proto::{
    self as pb, event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "document-context-admin";

type Client = EventModelServiceClient<Channel>;

fn request<T>(body: T, branch: Option<&str>) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {ADMIN}").parse().unwrap());
    if let Some(branch) = branch {
        req.metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    req
}

async fn client() -> (Client, trogon_atlas_client::client::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        std::sync::Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let grpc = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(service)),
        BearerAuth::new(std::sync::Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (
        Client::connect(format!("http://{addr}")).await.unwrap(),
        trogon_atlas_client::client::connect(&format!("http://{addr}"), Some(ADMIN), 30)
            .await
            .unwrap(),
    )
}

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "document-context".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn subject() -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("receipt")),
        })),
    }
}

async fn put(client: &mut Client, entity: pb::Entity) -> pb::Entity {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

fn wrap(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn decision(slug: &str) -> pb::Decision {
    pb::Decision {
        id: Some(id(slug)),
        title: Some(pb::CatalogTitle { value: slug.into() }),
        subjects: vec![pb::SubjectAssociation {
            subject: Some(subject()),
            ..Default::default()
        }],
        status: pb::DecisionStatus::Proposed as i32,
        context: Some(pb::CatalogText {
            value: "Context".into(),
        }),
        rationale: Some(pb::CatalogText {
            value: "Rationale".into(),
        }),
        options: vec![pb::DecisionOption {
            title: Some(pb::CatalogTitle {
                value: "Option".into(),
            }),
            description: Some(pb::CatalogText {
                value: "Description".into(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}
#[tokio::test]
async fn composed_native_graph_preserves_store_and_manifest_identity() {
    let (mut client, mut manifest_client) = client().await;
    let lifecycle = pb::LifecycleAnnotation {
        status: "deprecated".into(),
        deprecated_on: Some(pb::CalendarDate {
            value: "2024-02-29".into(),
        }),
        ..Default::default()
    };
    let metadata = vec![prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation".into(),
        value: lifecycle.encode_to_vec(),
    }];
    let mut graph = vec![];
    for kind in [
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("receipt")),
            title: "Receipt".into(),
            ..Default::default()
        }),
        pb::entity::Kind::Component(pb::Component {
            id: Some(id("agent")),
            title: "Agent".into(),
            metadata,
            ..Default::default()
        }),
        pb::entity::Kind::Person(pb::Person {
            id: Some(id("person")),
            title: Some(pb::DirectoryDisplayName {
                value: "Person".into(),
            }),
            ..Default::default()
        }),
        pb::entity::Kind::Team(pb::Team {
            id: Some(id("team")),
            title: Some(pb::DirectoryDisplayName {
                value: "Team".into(),
            }),
            ..Default::default()
        }),
        pb::entity::Kind::Domain(pb::Domain {
            id: Some(pb::Id {
                version: 9007199254740993,
                ..id("domain")
            }),
            title: "Domain".into(),
            ..Default::default()
        }),
    ] {
        graph.push(put(&mut client, wrap(kind)).await);
    }
    graph.push(
        put(
            &mut client,
            wrap(pb::entity::Kind::Term(pb::Term {
                id: Some(id("word")),
                owning_domain: Some(pb::DomainRef {
                    id: Some(pb::Id {
                        version: 9007199254740993,
                        ..id("domain")
                    }),
                }),
                ..Default::default()
            })),
        )
        .await,
    );
    graph.push(
        put(
            &mut client,
            wrap(pb::entity::Kind::Decision(decision("original"))),
        )
        .await,
    );
    let mut amended = decision("amendment");
    amended.authored_on = Some(pb::CalendarDate {
        value: "2024-02-29".into(),
    });
    amended.decided_on = Some(pb::CalendarDate {
        value: "2024-03-01".into(),
    });
    amended.decision_makers = vec![
        pb::DecisionMaker {
            assignee: Some(pb::decision_maker::Assignee::Person(pb::PersonRef {
                id: Some(id("person")),
            })),
            ..Default::default()
        },
        pb::DecisionMaker {
            assignee: Some(pb::decision_maker::Assignee::Team(pb::TeamRef {
                id: Some(id("team")),
            })),
            ..Default::default()
        },
    ];
    amended.amends = vec![pb::DecisionAmendment {
        decision: Some(pb::DecisionRef {
            id: Some(id("original")),
        }),
        ..Default::default()
    }];
    graph.push(put(&mut client, wrap(pb::entity::Kind::Decision(amended))).await);
    let bytes = vec![0, 255, 128, 13, 10];
    let binary = pb::Document {
        id: Some(id("binary")),
        title: Some(pb::CatalogTitle {
            value: "Binary".into(),
        }),
        subjects: vec![pb::SubjectAssociation {
            subject: Some(subject()),
            ..Default::default()
        }],
        body: Some(pb::document::Body::RetainedContent(pb::RetainedContent {
            media_type: Some(pb::MediaType {
                value: "application/octet-stream".into(),
            }),
            digest: Some(bytes_content_digest(&bytes)),
            content: bytes,
        })),
        ..Default::default()
    };
    graph.push(put(&mut client, wrap(pb::entity::Kind::Document(binary))).await);
    let hub = pb::Document {
        id: Some(id("hub")),
        title: Some(pb::CatalogTitle {
            value: "Hub".into(),
        }),
        subjects: vec![pb::SubjectAssociation {
            subject: Some(subject()),
            ..Default::default()
        }],
        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
            value: "https://example.com/hub".into(),
        })),
        purpose: Some(pb::DocumentPurpose::General as i32),
        metadata: vec![prost_types::Any {
            type_url:
                "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DocumentCollectionAnnotation"
                    .into(),
            value: pb::DocumentCollectionAnnotation {
                category: Some(pb::CatalogTitle {
                    value: "Resources".into(),
                }),
                ..Default::default()
            }
            .encode_to_vec(),
        }],
        related_content: vec![pb::RelatedContent {
            document: Some(pb::DocumentSelection {
                selection: Some(pb::document_selection::Selection::Document(
                    pb::DocumentRef {
                        id: Some(id("binary")),
                    },
                )),
            }),
            role: pb::RelatedContentRole::Attachment as i32,
            ..Default::default()
        }],
        ..Default::default()
    };
    graph.push(put(&mut client, wrap(pb::entity::Kind::Document(hub))).await);
    for (kind, slug, expected) in [
        (pb::EntityKind::Person, "person", "decision_makers"),
        (pb::EntityKind::Decision, "original", "amends"),
        (pb::EntityKind::Document, "binary", "related_content"),
    ] {
        let incoming = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: kind as i32,
                    id: Some(id(slug)),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        let matching: Vec<_> = incoming
            .references
            .iter()
            .filter(|r| r.from_field.contains(expected))
            .collect();
        assert_eq!(matching.len(), 1);
        assert!(!matching[0].from_uid.is_empty());
        assert!(matching[0].connection_id.is_some());
        assert_eq!(matching[0].to.as_ref().unwrap().id, Some(id(slug)));
    }
    let domain_id = pb::Id {
        version: 9007199254740993,
        ..id("domain")
    };
    let declarations = client
        .list_term_domain_declarations(request(
            pb::ListTermDomainDeclarationsRequest {
                domain: Some(pb::DomainRef {
                    id: Some(domain_id),
                }),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(declarations.declarations.len(), 1);
    let word = wrap(pb::entity::Kind::Term(pb::Term {
        id: Some(id("batch-word")),
        owning_domain: Some(pb::DomainRef {
            id: Some(id("batch-domain")),
        }),
        ..Default::default()
    }));
    let domain = wrap(pb::entity::Kind::Domain(pb::Domain {
        id: Some(id("batch-domain")),
        ..Default::default()
    }));
    let batch = |values: Vec<pb::Entity>| pb::BatchMutateRequest {
        ops: values
            .into_iter()
            .map(|entity| pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: Some(entity),
                    create_only: true,
                    ..Default::default()
                })),
            })
            .collect(),
        ..Default::default()
    };
    let applied = client
        .batch_mutate(request(batch(vec![word, domain]), None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        applied.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    let mut invalid = decision("invalid-batch");
    invalid.authored_on = Some(pb::CalendarDate {
        value: "2023-02-29".into(),
    });
    let innocent = wrap(pb::entity::Kind::Domain(pb::Domain {
        id: Some(id("unpublished")),
        ..Default::default()
    }));
    let failed = client
        .batch_mutate(request(
            batch(vec![innocent, wrap(pb::entity::Kind::Decision(invalid))]),
            None,
        ))
        .await;
    if let Ok(result) = failed {
        assert_eq!(
            result.into_inner().status,
            pb::batch_mutate_response::Status::Failed as i32
        );
    }
    let missing = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: Some(id("unpublished")),
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound);
    let pool = trogon_atlas_core::transcode::TranscodePool::builtin().unwrap();
    let rendered = trogon_atlas_client::export::render_board(&graph, &[], &pool).unwrap();
    let loaded =
        trogon_atlas_client::manifest::parse_str(&rendered.manifests, "composed.yaml").unwrap();
    assert_eq!(loaded.len(), graph.len());
    let plan = trogon_atlas_client::apply::plan(&mut manifest_client, loaded.clone())
        .await
        .unwrap();
    assert!(plan
        .iter()
        .all(|change| change.action == trogon_atlas_client::apply::Action::Unchanged));
    trogon_atlas_client::apply::apply(&mut manifest_client, plan, false, None)
        .await
        .unwrap();
    for desired in loaded {
        let stored = graph
            .iter()
            .find(|entity| {
                trogon_atlas_core::entity_id(entity) == Some(&desired.id)
                    && trogon_atlas_core::entity_kind(entity) == Some(desired.kind)
            })
            .unwrap();
        assert_eq!(
            trogon_atlas_client::apply::classify(&desired.entity, Some(stored)),
            trogon_atlas_client::apply::Action::Unchanged
        );
        let fetched = client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: desired.kind as i32,
                    id: Some(desired.id),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        assert_eq!(&fetched, stored);
    }
}
