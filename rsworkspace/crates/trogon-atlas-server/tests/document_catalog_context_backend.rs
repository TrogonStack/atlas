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

#[tokio::test]
async fn binary_retained_content_round_trips_exact_bytes_on_grpc() {
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
    let bytes = b"\xffSVG<svg onload=\"alert(1)\"></svg>\x00";
    let stored = put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(id("attachment")),
                title: Some(pb::CatalogTitle {
                    value: "Attachment".into(),
                }),
                subjects: vec![pb::SubjectAssociation {
                    subject: Some(subject()),
                    ..Default::default()
                }],
                body: Some(pb::document::Body::RetainedContent(pb::RetainedContent {
                    media_type: Some(pb::MediaType {
                        value: "image/svg+xml".into(),
                    }),
                    content: bytes.to_vec(),
                    digest: Some(bytes_content_digest(bytes)),
                })),
                purpose: Some(pb::DocumentPurpose::Attachment as i32),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let stored_id = trogon_atlas_core::entity_id(&stored).unwrap().clone();
    let loaded = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(stored_id),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let Some(pb::entity::Kind::Document(document)) = loaded.kind else {
        panic!("document required")
    };
    let Some(pb::document::Body::RetainedContent(body)) = document.body else {
        panic!("retained content required")
    };
    assert_eq!(body.content, bytes);
    assert_eq!(
        body.digest.as_ref().map(|d| d.sha256.as_str()),
        Some(bytes_content_digest(bytes).sha256.as_str())
    );
}

#[tokio::test]
async fn incoming_document_page_token_binds_selection_and_paginates() {
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
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let hub_id = trogon_atlas_core::entity_id(&hub).unwrap().clone();
    let whole = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef { id: Some(hub_id) },
        )),
    };
    for slug in ["alpha", "beta"] {
        let _ = put(
            &mut client,
            pb::Entity {
                kind: Some(pb::entity::Kind::Decision(pb::Decision {
                    id: Some(id(slug)),
                    title: Some(pb::CatalogTitle { value: slug.into() }),
                    subjects: vec![pb::SubjectAssociation {
                        subject: Some(subject()),
                        ..Default::default()
                    }],
                    status: pb::DecisionStatus::Proposed as i32,
                    context: Some(pb::CatalogText {
                        value: "ctx".into(),
                    }),
                    rationale: Some(pb::CatalogText {
                        value: "rat".into(),
                    }),
                    options: vec![pb::DecisionOption {
                        title: Some(pb::CatalogTitle {
                            value: "opt".into(),
                        }),
                        description: Some(pb::CatalogText {
                            value: "desc".into(),
                        }),
                        ..Default::default()
                    }],
                    documents: vec![pb::SupportingDocument {
                        document: Some(whole.clone()),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await;
    }
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
    assert_eq!(page1.declarations.len(), 1);
    let continue_token = page1.next_page_token.clone();
    assert_ne!(continue_token, "");
    let page2 = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                page_token: continue_token,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page2.declarations.len(), 1);
    assert_eq!(page2.next_page_token, "");
    let mismatched = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: Some(id("missing")),
            },
        )),
    };
    let err = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(mismatched),
                page_size: 1,
                page_token: page1.next_page_token.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), Code::InvalidArgument);
    let mut bad_token = page1.next_page_token;
    bad_token.push('!');
    let err = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: 1,
                page_token: bad_token,
                ..Default::default()
            },
            None,
        ))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), Code::InvalidArgument);
    let scope = page1.scope.expect("checked scope");
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
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
                document: Some(whole),
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
}
