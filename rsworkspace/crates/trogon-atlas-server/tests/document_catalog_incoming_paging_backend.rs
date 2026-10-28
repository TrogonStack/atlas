#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use prost::Message;
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto::{
    self as pb, event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "document-paging-admin";

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

async fn client() -> Client {
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
    Client::connect(format!("http://{addr}")).await.unwrap()
}

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "document-paging".into(),
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

async fn batch_put_referrers(
    client: &mut Client,
    hub: pb::DocumentSelection,
    start: usize,
    count: usize,
) {
    const CHUNK: usize = 200;
    let mut index = start;
    while index < start + count {
        let end = (index + CHUNK).min(start + count);
        let ops = (index..end)
            .map(|n| {
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(pb::Entity {
                            kind: Some(pb::entity::Kind::Document(pb::Document {
                                id: Some(id(&format!("referrer-{n}"))),
                                title: Some(pb::CatalogTitle {
                                    value: format!("referrer-{n}"),
                                }),
                                subjects: vec![pb::SubjectAssociation {
                                    subject: Some(subject()),
                                    ..Default::default()
                                }],
                                body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                                    value: format!("https://example.com/referrer-{n}"),
                                })),
                                related_content: vec![pb::RelatedContent {
                                    role: pb::RelatedContentRole::Diagram as i32,
                                    document: Some(hub.clone()),
                                    ..Default::default()
                                }],
                                ..Default::default()
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })),
                }
            })
            .collect();
        let result = client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    ops,
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            result.status,
            pb::batch_mutate_response::Status::Applied as i32
        );
        index = end;
    }
}

async fn batch_put_filler_documents(client: &mut Client, start: usize, count: usize) {
    const CHUNK: usize = 200;
    let mut index = start;
    while index < start + count {
        let end = (index + CHUNK).min(start + count);
        let ops = (index..end)
            .map(|n| {
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(pb::Entity {
                            kind: Some(pb::entity::Kind::Document(pb::Document {
                                id: Some(id(&format!("filler-{n}"))),
                                title: Some(pb::CatalogTitle {
                                    value: format!("filler-{n}"),
                                }),
                                subjects: vec![pb::SubjectAssociation {
                                    subject: Some(subject()),
                                    ..Default::default()
                                }],
                                body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                                    value: format!("https://example.com/filler-{n}"),
                                })),
                                ..Default::default()
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })),
                }
            })
            .collect();
        let result = client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    ops,
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            result.status,
            pb::batch_mutate_response::Status::Applied as i32
        );
        index = end;
    }
}

fn hub_selection(hub_id: pb::Id) -> pb::DocumentSelection {
    pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef { id: Some(hub_id) },
        )),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn incoming_document_paging_surpasses_output_ceiling_with_multipage_continuity() {
    let mut client = client().await;
    let _ = put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id("receipt")),
                title: "Receipt".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let hub = put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(id("incoming-hub")),
                title: Some(pb::CatalogTitle {
                    value: "Incoming hub".into(),
                }),
                subjects: vec![pb::SubjectAssociation {
                    subject: Some(subject()),
                    ..Default::default()
                }],
                body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                    value: "https://example.com/hub".into(),
                })),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let hub_id = trogon_atlas_core::entity_id(&hub).unwrap().clone();
    let whole = hub_selection(hub_id);
    const REFERRERS: usize = 4_097;
    const FILLER: usize = 902;
    batch_put_filler_documents(&mut client, 0, FILLER).await;
    batch_put_referrers(&mut client, whole.clone(), 0, REFERRERS).await;
    let mut token = String::new();
    let mut seen = HashSet::new();
    let page_size = 500;
    let mut pages = 0;
    loop {
        let page = client
            .list_incoming_document_declarations(request(
                pb::ListIncomingDocumentDeclarationsRequest {
                    document: Some(whole.clone()),
                    page_size: page_size as i32,
                    page_token: token.clone(),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(!page.incomplete);
        assert!(page.failures.is_empty());
        for declaration in &page.declarations {
            let owner = declaration.owner.as_ref().expect("owner");
            assert!(seen.insert(owner.uid.clone()));
            assert_eq!(
                declaration.state,
                pb::CatalogResolutionState::Resolved as i32
            );
        }
        pages += 1;
        if page.next_page_token.is_empty() {
            break;
        }
        let err = client
            .list_incoming_document_declarations(request(
                pb::ListIncomingDocumentDeclarationsRequest {
                    document: Some(whole.clone()),
                    page_size: page_size as i32,
                    page_token: page.next_page_token.clone(),
                    related_content_role: Some(pb::RelatedContentRole::Example as i32),
                    ..Default::default()
                },
                None,
            ))
            .await
            .err()
            .expect("role filter mismatch must refuse stale token");
        assert_eq!(err.code(), Code::InvalidArgument);
        token = page.next_page_token;
    }
    assert_eq!(seen.len(), REFERRERS);
    assert_eq!(pages, (REFERRERS + page_size - 1) / page_size);
}

#[tokio::test(flavor = "multi_thread")]
async fn incoming_document_page_token_rejects_malformed_stale_and_out_of_range_offsets() {
    let mut client = client().await;
    let _ = put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id("receipt")),
                title: "Receipt".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let hub = put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(id("token-hub")),
                title: Some(pb::CatalogTitle {
                    value: "Token hub".into(),
                }),
                subjects: vec![pb::SubjectAssociation {
                    subject: Some(subject()),
                    ..Default::default()
                }],
                body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                    value: "https://example.com/token-hub".into(),
                })),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let whole = hub_selection(trogon_atlas_core::entity_id(&hub).unwrap().clone());
    batch_put_referrers(&mut client, whole.clone(), 0, 3).await;
    let page1 = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let scope = page1.scope.clone().expect("scope");
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let stale = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({
            "revision": "catalog-checked-v1:stale",
            "role": serde_json::Value::Null,
            "selection": URL_SAFE_NO_PAD.encode(whole.encode_to_vec()),
            "index": 0,
        }))
        .unwrap(),
    );
    let err = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                page_token: stale,
                ..Default::default()
            },
            None,
        ))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("stale page token"));
    let forged = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({
            "revision": scope.revision,
            "role": serde_json::Value::Null,
            "selection": URL_SAFE_NO_PAD.encode(whole.encode_to_vec()),
            "index": 99,
        }))
        .unwrap(),
    );
    let err = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                page_token: forged,
                ..Default::default()
            },
            None,
        ))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("page token offset out of range"));
    let mut bad = page1.next_page_token.clone();
    bad.push('!');
    let err = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                page_token: bad,
                ..Default::default()
            },
            None,
        ))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("invalid page token"));

    assert!(!page1.next_page_token.is_empty());
    assert_eq!(page1.declarations.len(), 1);
    let continued = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                page_token: page1.next_page_token.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(continued.declarations.len(), 1);
    assert_eq!(continued.scope.unwrap().revision, scope.revision);
    assert_ne!(continued.declarations, page1.declarations);

    batch_put_filler_documents(&mut client, 0, 1).await;
    let stale = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                page_token: page1.next_page_token,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(stale.code(), Code::InvalidArgument);
    assert!(stale.message().contains("stale page token"));

    let refreshed = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole),
                page_size: 1,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(refreshed.scope.unwrap().revision, scope.revision);
    assert_eq!(refreshed.declarations, page1.declarations);
}
