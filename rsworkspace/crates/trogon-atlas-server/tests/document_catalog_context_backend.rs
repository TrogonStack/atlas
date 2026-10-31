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
    client_with_store().await.0
}

async fn client_with_store() -> (Client, std::sync::Arc<trogon_atlas_store::NatsStore>) {
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
    let store = std::sync::Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
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
        store,
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
    put_on_branch(client, entity, None).await
}

async fn put_on_branch(
    client: &mut Client,
    entity: pb::Entity,
    branch: Option<&str>,
) -> pb::Entity {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            },
            branch,
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
                page_size: Some(1),
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
                page_size: Some(1),
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
                page_size: Some(1),
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
                page_size: Some(1),
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
                page_size: Some(1),
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

fn document_ref(slug: &str) -> pb::DocumentRef {
    pb::DocumentRef { id: Some(id(slug)) }
}

fn target_document(slug: &str, purpose: pb::DocumentPurpose) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Document(pb::Document {
            id: Some(id(slug)),
            title: Some(pb::CatalogTitle { value: slug.into() }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(subject()),
                ..Default::default()
            }],
            purpose: Some(purpose as i32),
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/target".into(),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn owner_with_related(slug: &str, role: pb::RelatedContentRole, target: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Document(pb::Document {
            id: Some(id(slug)),
            title: Some(pb::CatalogTitle { value: slug.into() }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(subject()),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/owner".into(),
            })),
            related_content: vec![pb::RelatedContent {
                role: role as i32,
                document: Some(pb::DocumentSelection {
                    selection: Some(pb::document_selection::Selection::Document(document_ref(
                        target,
                    ))),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn get_doc(client: &mut Client, slug: &str) -> pb::Entity {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(slug)),
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
async fn related_content_purpose_agreement_rejects_conflicting_target_edit_in_batch() {
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
    put(
        &mut client,
        target_document("diagram-target", pb::DocumentPurpose::Diagram),
    )
    .await;
    put(
        &mut client,
        owner_with_related(
            "diagram-owner",
            pb::RelatedContentRole::Diagram,
            "diagram-target",
        ),
    )
    .await;
    let before = get_doc(&mut client, "diagram-owner").await;
    let mut edited_target = get_doc(&mut client, "diagram-target").await;
    if let Some(pb::entity::Kind::Document(document)) = &mut edited_target.kind {
        document.purpose = Some(pb::DocumentPurpose::Attachment as i32);
    }
    let response = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: vec![
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(edited_target),
                            ..Default::default()
                        })),
                    },
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(before.clone()),
                            ..Default::default()
                        })),
                    },
                ],
                ..Default::default()
            },
            None,
        ))
        .await;
    match response {
        Err(error) => assert_eq!(error.code(), Code::InvalidArgument, "{error}"),
        Ok(response) => assert_ne!(
            response.into_inner().status,
            pb::batch_mutate_response::Status::Applied as i32
        ),
    }
    assert_eq!(get_doc(&mut client, "diagram-owner").await, before);
    let target_after = get_doc(&mut client, "diagram-target").await;
    if let Some(pb::entity::Kind::Document(document)) = &target_after.kind {
        assert_eq!(
            document.purpose,
            Some(pb::DocumentPurpose::Diagram as i32),
            "target purpose must not partially publish"
        );
    }
}

#[tokio::test]
async fn related_content_purpose_agreement_rejects_removed_target_in_batch() {
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
    put(
        &mut client,
        target_document("drop-target", pb::DocumentPurpose::Diagram),
    )
    .await;
    put(
        &mut client,
        owner_with_related("drop-owner", pb::RelatedContentRole::Diagram, "drop-target"),
    )
    .await;
    let before = get_doc(&mut client, "drop-owner").await;
    let response = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: vec![
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(before.clone()),
                            ..Default::default()
                        })),
                    },
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                            kind: pb::EntityKind::Document as i32,
                            id: Some(id("drop-target")),
                            mode: pb::delete_entity_request::Mode::Force as i32,
                            ..Default::default()
                        })),
                    },
                ],
                ..Default::default()
            },
            None,
        ))
        .await;
    match response {
        Err(error) => assert_eq!(error.code(), Code::InvalidArgument, "{error}"),
        Ok(response) => assert_ne!(
            response.into_inner().status,
            pb::batch_mutate_response::Status::Applied as i32
        ),
    }
    assert_eq!(get_doc(&mut client, "drop-owner").await, before);
    get_doc(&mut client, "drop-target").await;
}

#[tokio::test]
async fn related_content_purpose_agreement_uses_branch_overlay_in_batch() {
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
    put(
        &mut client,
        target_document("branch-target", pb::DocumentPurpose::Diagram),
    )
    .await;
    put(
        &mut client,
        owner_with_related(
            "branch-owner",
            pb::RelatedContentRole::Diagram,
            "branch-target",
        ),
    )
    .await;
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "purpose-branch".into(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap();
    let mut branch_target = get_doc(&mut client, "branch-target").await;
    if let Some(pb::entity::Kind::Document(document)) = &mut branch_target.kind {
        document.purpose = Some(pb::DocumentPurpose::Attachment as i32);
    }
    put_on_branch(&mut client, branch_target, Some("purpose-branch")).await;
    let mut branch_owner = get_doc(&mut client, "branch-owner").await;
    if let Some(pb::entity::Kind::Document(document)) = &mut branch_owner.kind {
        document.title = Some(pb::CatalogTitle {
            value: "Branch owner title".into(),
        });
    }
    let before = get_doc(&mut client, "branch-owner").await;
    let response = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: vec![pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(branch_owner),
                        ..Default::default()
                    })),
                }],
                ..Default::default()
            },
            Some("purpose-branch"),
        ))
        .await;
    match response {
        Err(error) => assert_eq!(error.code(), Code::InvalidArgument, "{error}"),
        Ok(response) => assert_ne!(
            response.into_inner().status,
            pb::batch_mutate_response::Status::Applied as i32
        ),
    }
    assert_eq!(get_doc(&mut client, "branch-owner").await, before);
}

#[tokio::test]
async fn incoming_document_pages_order_versions_numerically() {
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
    for version in [10, 2] {
        let slug = "same-owner";
        let _ = put(
            &mut client,
            pb::Entity {
                kind: Some(pb::entity::Kind::Decision(pb::Decision {
                    id: Some(pb::Id {
                        version,
                        ..id(slug)
                    }),
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
                page_size: Some(1),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page1.declarations.len(), 1);
    assert_eq!(
        page1.declarations[0]
            .owner
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        2
    );
    let continue_token = page1.next_page_token.clone();
    assert_ne!(continue_token, "");
    let page2 = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                page_size: Some(1),
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
                page_size: Some(1),
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
                page_size: Some(1),
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
                page_size: Some(1),
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

fn resize_complete_owner(entity: &mut pb::Entity, target: usize) {
    loop {
        let size = entity.encoded_len();
        if size == target {
            return;
        }
        let Some(pb::entity::Kind::Document(document)) = entity.kind.as_mut() else {
            panic!("document")
        };
        let doc = document.doc.get_or_insert_default();
        if size < target {
            doc.push_str(&"x".repeat(target - size));
        } else {
            doc.truncate(doc.len() - (size - target));
        }
    }
}

#[tokio::test]
async fn complete_native_owner_capacity_is_atomic_on_public_service_and_store() {
    use trogon_atlas_store::{Store, WriteContext};
    let (mut client, store) = client_with_store().await;
    let mut subjects = Vec::new();
    for index in 0..64 {
        let subject_id = id(&format!("capacity-subject-{index}"));
        put(
            &mut client,
            pb::Entity {
                kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                    id: Some(subject_id.clone()),
                    title: "Receipt".into(),
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await;
        subjects.push(pb::SubjectAssociation {
            subject: Some(pb::CatalogSubject {
                selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32,
                    id: Some(subject_id),
                })),
            }),
            doc: Some(pb::CatalogText {
                value: "x".repeat(15_500),
            }),
            ..Default::default()
        });
    }
    let mut owner = put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(id("capacity")),
                title: Some(pb::CatalogTitle {
                    value: "Capacity".into(),
                }),
                subjects,
                body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                    value: "https://example.com/doc".into(),
                })),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    resize_complete_owner(&mut owner, 1_048_576);
    trogon_atlas_core::decision_document::validate_values(&owner).unwrap();
    let exact = put(&mut client, owner.clone()).await;
    assert_eq!(exact.encoded_len(), 1_048_576);
    trogon_atlas_core::decision_document::validate_values(&exact).unwrap();
    let reference = id("capacity");
    let kind = pb::EntityKind::Document;
    let read = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(reference.clone()),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.entity.as_ref().unwrap(), &exact);
    assert_eq!(put(&mut client, exact.clone()).await, exact);
    let before = store.get(kind, &reference, None).await.unwrap();
    let sequence = store.current_change_seq().await.unwrap();
    let mut overflow = exact.clone();
    resize_complete_owner(&mut overflow, 1_048_577);
    overflow.system = None;
    assert_eq!(
        trogon_atlas_core::stamp_system(Some(&exact), &overflow).encoded_len(),
        1_048_577
    );
    assert!(overflow.encoded_len() < 1_048_576);
    let error = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(overflow.clone()),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        store.get(kind, &reference, None).await.unwrap().entity,
        before.entity
    );
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
    assert!(matches!(
        store
            .update(
                kind,
                &overflow,
                &before.etag,
                true,
                WriteContext::baseline()
            )
            .await,
        Err(trogon_atlas_store::StoreError::InvalidArgument(_))
    ));
    let unchanged = store.get(kind, &reference, None).await.unwrap();
    assert_eq!(unchanged.entity, before.entity);
    assert_eq!(unchanged.etag, before.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
}

fn resize_graph_owner(entity: &mut pb::Entity, target: usize) {
    loop {
        let size = entity.encoded_len();
        if size == target {
            return;
        }
        let Some(pb::entity::Kind::DesignGraph(graph)) = &mut entity.kind else {
            panic!("graph")
        };
        let doc = graph.doc.get_or_insert_default();
        if size < target {
            doc.push_str(&"x".repeat(target - size));
        } else {
            doc.truncate(doc.len() - (size - target));
        }
        assert!(doc.len() <= 16_384);
    }
}

#[tokio::test]
async fn complete_prepared_graph_capacity_persists_exact_boundary_and_refuses_one_byte_more() {
    use trogon_atlas_store::{Store, WriteContext};
    let (mut client, store) = client_with_store().await;
    let mut subjects = Vec::new();
    for index in 0..64 {
        let selected = id(&format!("graph-capacity-subject-{index}"));
        put(
            &mut client,
            pb::Entity {
                kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                    id: Some(selected.clone()),
                    title: "Subject".into(),
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await;
        subjects.push(pb::SubjectAssociation {
            subject: Some(pb::CatalogSubject {
                selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32,
                    id: Some(selected),
                })),
            }),
            doc: Some(pb::CatalogText {
                value: "x".repeat(16_100),
            }),
            ..Default::default()
        });
    }
    let mut owner = put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::DesignGraph(pb::DesignGraph {
                id: Some(id("graph-capacity")),
                title: Some(pb::CatalogTitle {
                    value: "Capacity".into(),
                }),
                profile: pb::DesignGraphProfile::Diagram as i32,
                steps: vec![pb::TopologyStep {
                    title: Some(pb::CatalogTitle {
                        value: "Actor".into(),
                    }),
                    target: Some(pb::TopologyStepTarget {
                        selection: Some(pb::topology_step_target::Selection::Actor(
                            pb::TopologyActor {
                                name: Some(pb::CatalogTitle {
                                    value: "Actor".into(),
                                }),
                                ..Default::default()
                            },
                        )),
                    }),
                    ..Default::default()
                }],
                subjects,
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    resize_graph_owner(&mut owner, 1_048_576);
    trogon_atlas_core::design_topology::validate_values(&owner).unwrap();
    let exact = put(&mut client, owner).await;
    assert_eq!(exact.encoded_len(), 1_048_576);
    let kind = pb::EntityKind::DesignGraph;
    let key = id("graph-capacity");
    let before = store.get(kind, &key, None).await.unwrap();
    assert_eq!(before.entity, exact);
    let fetched = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(key.clone()),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched.entity.as_ref(), Some(&exact));
    assert_eq!(fetched.etag, before.etag);
    let sequence = store.current_change_seq().await.unwrap();
    assert_eq!(put(&mut client, exact.clone()).await, exact);
    assert_eq!(store.get(kind, &key, None).await.unwrap().etag, before.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
    let mut overflow = exact.clone();
    resize_graph_owner(&mut overflow, 1_048_577);
    overflow.system = None;
    assert_eq!(
        trogon_atlas_core::stamp_system(Some(&exact), &overflow).encoded_len(),
        1_048_577
    );
    assert!(overflow.encoded_len() < 1_048_576);
    let error = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(overflow.clone()),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(matches!(
        store
            .update(
                kind,
                &overflow,
                &before.etag,
                true,
                WriteContext::baseline()
            )
            .await,
        Err(trogon_atlas_store::StoreError::InvalidArgument(_))
    ));
    let after = store.get(kind, &key, None).await.unwrap();
    assert_eq!(after.entity, before.entity);
    assert_eq!(after.etag, before.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
}
