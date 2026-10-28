#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

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

const ADMIN: &str = "doc-auth-admin";
const READER: &str = "doc-auth-reader";
const LIMITED: &str = "doc-auth-limited";

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
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
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

fn subject(namespace: &str) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(namespace, "receipt")),
        })),
    }
}

async fn register_pair(client: &mut Client) -> (String, String) {
    let mut namespaces = Vec::new();
    for (name, parent) in [("doc-visible", "visible"), ("doc-private", "hidden")] {
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
    (namespaces[0].clone(), namespaces[1].clone())
}

async fn put_receipt(client: &mut Client, visible_ns: &str) {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                        id: Some(id(visible_ns, "receipt")),
                        title: "Receipt".into(),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
}

fn binary_document(
    namespace: &str,
    slug: &str,
    title: &str,
    visible_ns: &str,
    bytes: &[u8],
    media: &str,
) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Document(pb::Document {
            id: Some(id(namespace, slug)),
            title: Some(pb::CatalogTitle {
                value: title.into(),
            }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(subject(visible_ns)),
                ..Default::default()
            }],
            body: Some(pb::document::Body::RetainedContent(pb::RetainedContent {
                media_type: Some(pb::MediaType {
                    value: media.into(),
                }),
                content: bytes.to_vec(),
                digest: Some(bytes_content_digest(bytes)),
            })),
            purpose: Some(pb::DocumentPurpose::Attachment as i32),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn whole_selection(namespace: &str, slug: &str) -> pb::DocumentSelection {
    pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: Some(id(namespace, slug)),
            },
        )),
    }
}

fn assert_no_binary_leak(message: &str, title: &str, media: &str, digest: &str) {
    let lower = message.to_ascii_lowercase();
    assert!(!lower.contains(&title.to_ascii_lowercase()));
    assert!(!lower.contains(&media.to_ascii_lowercase()));
    assert!(!lower.contains(&digest.to_ascii_lowercase()));
    assert!(!lower.contains("0xff"));
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_hidden_detail_is_generic_not_found_without_content_leaks() {
    let mut client = client().await;
    let (visible_ns, hidden_ns) = register_pair(&mut client).await;
    put_receipt(&mut client, &visible_ns).await;
    let secret_title = "PrivateBinaryTitleLeakProbe";
    let media = "image/svg+xml";
    let bytes = b"\xff<svg secret=\"1\"/>";
    let digest = bytes_content_digest(bytes).sha256;
    let hidden = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(binary_document(
                    &hidden_ns,
                    "hidden-binary",
                    secret_title,
                    &visible_ns,
                    bytes,
                    media,
                )),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let selection = whole_selection(&hidden_ns, "hidden-binary");
    let err = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(selection),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    assert_no_binary_leak(err.message(), secret_title, media, &digest);
    let write_err = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(hidden),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(write_err.code(), Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn visible_owner_related_content_to_hidden_binary_is_unavailable_without_leak() {
    let mut client = client().await;
    let (visible_ns, hidden_ns) = register_pair(&mut client).await;
    put_receipt(&mut client, &visible_ns).await;
    let secret_title = "HiddenRelatedTargetTitle";
    let media = "application/octet-stream";
    let bytes = b"\xff\x00secret-bytes";
    let digest = bytes_content_digest(bytes).sha256;
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(binary_document(
                    &hidden_ns,
                    "hidden-target",
                    secret_title,
                    &visible_ns,
                    bytes,
                    media,
                )),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let hub = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "visible-hub")),
                        title: Some(pb::CatalogTitle {
                            value: "Visible hub".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/hub".into(),
                        })),
                        related_content: vec![pb::RelatedContent {
                            role: pb::RelatedContentRole::Attachment as i32,
                            document: Some(whole_selection(&hidden_ns, "hidden-target")),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let detail = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(whole_selection(&visible_ns, "visible-hub")),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let related = detail
        .declarations
        .iter()
        .find(|d| d.field.starts_with("related_content["))
        .expect("related_content declaration must be listed");
    assert_eq!(
        related.state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    let Some(pb::catalog_declaration::Declaration::RelatedContent(assoc)) = &related.declaration
    else {
        panic!("related content declaration");
    };
    assert_eq!(
        assoc.document.as_ref(),
        Some(&whole_selection(&hidden_ns, "hidden-target"))
    );
    assert!(detail.scope.unwrap().visited.iter().all(|owner| owner
        .owner
        .as_ref()
        .and_then(|owner| owner.id.as_ref())
        .is_none_or(|id| id.namespace != hidden_ns)));
    let err = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(whole_selection(&hidden_ns, "hidden-target")),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    assert_no_binary_leak(err.message(), secret_title, media, &digest);
    let _ = hub;
}

#[tokio::test(flavor = "multi_thread")]
async fn incoming_to_missing_target_keeps_declaration_unavailable() {
    let mut client = client().await;
    let (visible_ns, _) = register_pair(&mut client).await;
    put_receipt(&mut client, &visible_ns).await;
    let target = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "incoming-target")),
                        title: Some(pb::CatalogTitle {
                            value: "Incoming target".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/target".into(),
                        })),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let owner = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "incoming-owner")),
                        title: Some(pb::CatalogTitle {
                            value: "Incoming owner".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/owner".into(),
                        })),
                        related_content: vec![pb::RelatedContent {
                            role: pb::RelatedContentRole::Diagram as i32,
                            document: Some(whole_selection(&visible_ns, "incoming-target")),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let selection = whole_selection(&visible_ns, "incoming-target");
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&visible_ns, "incoming-target")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let incoming = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(selection.clone()),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.declarations.len(), 1);
    assert_eq!(
        incoming.declarations[0].state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    let Some(pb::catalog_declaration::Declaration::RelatedContent(related)) =
        &incoming.declarations[0].declaration
    else {
        panic!("related incoming");
    };
    assert_eq!(related.document.as_ref(), Some(&selection));
    assert_eq!(
        incoming.declarations[0].owner.as_ref().unwrap().uid,
        owner.system.as_ref().unwrap().uid
    );
    let _ = target;
}

#[tokio::test(flavor = "multi_thread")]
async fn safe_delete_visible_target_blocked_by_hidden_referrer_without_leak() {
    let mut client = client().await;
    let (visible_ns, hidden_ns) = register_pair(&mut client).await;
    put_receipt(&mut client, &visible_ns).await;
    let hub_title = "VisibleHubSafeDeleteProbe";
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "safe-hub")),
                        title: Some(pb::CatalogTitle {
                            value: hub_title.into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/safe-hub".into(),
                        })),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&hidden_ns, "hidden-referrer")),
                        title: Some(pb::CatalogTitle {
                            value: "Hidden referrer".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/hidden".into(),
                        })),
                        related_content: vec![pb::RelatedContent {
                            role: pb::RelatedContentRole::Example as i32,
                            document: Some(whole_selection(&visible_ns, "safe-hub")),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let err = client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&visible_ns, "safe-hub")),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(!err.message().contains(hub_title));
    assert!(!err.message().contains("hidden-referrer"));
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&visible_ns, "safe-hub")),
            },
            READER,
            None,
        ))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn force_delete_target_preserves_exact_supporting_declaration_on_decision_detail() {
    let mut client = client().await;
    let (visible_ns, _) = register_pair(&mut client).await;
    put_receipt(&mut client, &visible_ns).await;
    let selection = whole_selection(&visible_ns, "force-target");
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "force-target")),
                        title: Some(pb::CatalogTitle {
                            value: "Force target".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/force-target".into(),
                        })),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let choice = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Decision(pb::Decision {
                        id: Some(id(&visible_ns, "force-choice")),
                        title: Some(pb::CatalogTitle {
                            value: "Force choice".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
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
                            document: Some(selection.clone()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&visible_ns, "force-target")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let detail = client
        .get_decision_detail(request(
            pb::GetDecisionDetailRequest {
                decision: Some(pb::DecisionRef {
                    id: Some(id(&visible_ns, "force-choice")),
                }),
                follow_replacement: false,
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let supporting = detail
        .declarations
        .iter()
        .find(|d| d.field.starts_with("documents["))
        .expect("supporting declaration");
    assert_eq!(
        supporting.state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    let Some(pb::catalog_declaration::Declaration::Supporting(doc)) = &supporting.declaration
    else {
        panic!("supporting");
    };
    assert_eq!(doc.document.as_ref(), Some(&selection));
    assert_eq!(detail.entity.as_ref(), Some(&choice));
}

#[tokio::test(flavor = "multi_thread")]
async fn document_whole_ref_stable_while_anchor_pin_recreates_ids_on_rerender() {
    let mut client = client().await;
    let (visible_ns, _) = register_pair(&mut client).await;
    put_receipt(&mut client, &visible_ns).await;
    let content = "# Delivery\n";
    let note = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "anchor-note")),
                        title: Some(pb::CatalogTitle {
                            value: "Anchor note".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                            media_type: Some(pb::MediaType {
                                value: "text/markdown".into(),
                            }),
                            content: content.into(),
                            digest: Some(trogon_atlas_core::specification::content_digest(content)),
                        })),
                        anchors: vec![pb::DocumentAnchor {
                            fragment: Some(pb::AnchorFragment {
                                value: "delivery".into(),
                            }),
                            label: Some(pb::CatalogTitle {
                                value: "Delivery".into(),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let entity = note.entity.as_ref().unwrap();
    let Some(pb::entity::Kind::Document(document)) = &entity.kind else {
        panic!("document");
    };
    let anchor_id = document.anchors[0].anchor_id.clone();
    let Some(pb::document::Body::Retained(body)) = &document.body else {
        panic!("retained");
    };
    let pinned = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Anchor(
            pb::DocumentAnchorRef {
                document: Some(pb::DocumentRef {
                    id: document.id.clone(),
                }),
                owner_uid: entity.system.as_ref().unwrap().uid.clone(),
                anchor_id: anchor_id.clone(),
                expected_digest: Some(body.digest.clone().unwrap()),
            },
        )),
    };
    let whole = whole_selection(&visible_ns, "anchor-note");
    let incoming_whole = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(whole.clone()),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(incoming_whole.declarations.is_empty());
    let revised = "# Revised delivery\n";
    let mut edited = entity.clone();
    if let Some(pb::entity::Kind::Document(document)) = &mut edited.kind {
        document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            media_type: Some(pb::MediaType {
                value: "text/markdown".into(),
            }),
            content: revised.into(),
            digest: Some(trogon_atlas_core::specification::content_digest(revised)),
        }));
        document.anchors = vec![pb::DocumentAnchor {
            fragment: Some(pb::AnchorFragment {
                value: "revised".into(),
            }),
            label: Some(pb::CatalogTitle {
                value: "Revised".into(),
            }),
            ..Default::default()
        }];
    }
    let updated = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(edited),
                if_match: note.etag,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let updated_entity = updated.entity.unwrap();
    let Some(pb::entity::Kind::Document(updated_doc)) = &updated_entity.kind else {
        panic!("document");
    };
    assert_ne!(updated_doc.anchors[0].anchor_id, anchor_id);
    assert_eq!(
        updated_entity.system.as_ref().unwrap().uid,
        entity.system.as_ref().unwrap().uid
    );
    let stale_pin = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(pinned),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        stale_pin.selection_state,
        pb::CatalogResolutionState::Unresolved as i32
    );
    assert!(stale_pin.anchor.is_none());
    let whole_detail = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(whole),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        whole_detail.selection_state,
        pb::CatalogResolutionState::Resolved as i32
    );
    assert_eq!(
        whole_detail.entity.as_ref().unwrap().encode_to_vec(),
        updated_entity.encode_to_vec()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_related_content_survives_target_removal_on_branch_without_baseline_mutation() {
    let mut client = client().await;
    let (visible_ns, _) = register_pair(&mut client).await;
    put_receipt(&mut client, &visible_ns).await;
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "branch-target")),
                        title: Some(pb::CatalogTitle {
                            value: "Branch target".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/branch-target".into(),
                        })),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let owner = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&visible_ns, "branch-owner")),
                        title: Some(pb::CatalogTitle {
                            value: "Branch owner".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(subject(&visible_ns)),
                            ..Default::default()
                        }],
                        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                            value: "https://example.com/branch-owner".into(),
                        })),
                        related_content: vec![pb::RelatedContent {
                            role: pb::RelatedContentRole::Diagram as i32,
                            document: Some(whole_selection(&visible_ns, "branch-target")),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let baseline_detail = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(whole_selection(&visible_ns, "branch-owner")),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        baseline_detail.declarations[0].state,
        pb::CatalogResolutionState::Resolved as i32
    );
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "doc-related-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&visible_ns, "branch-target")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            Some("doc-related-branch"),
        ))
        .await
        .unwrap();
    let branch_detail = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(whole_selection(&visible_ns, "branch-owner")),
            },
            READER,
            Some("doc-related-branch"),
        ))
        .await
        .unwrap()
        .into_inner();
    let related = branch_detail
        .declarations
        .iter()
        .find(|d| d.field.starts_with("related_content["))
        .expect("related_content declaration");
    assert_eq!(
        related.state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    let merged = client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: "doc-related-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Document as i32,
                    id: Some(id(&visible_ns, "branch-owner")),
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap()
            .encode_to_vec(),
        owner.encode_to_vec()
    );
    let foreign = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(whole_selection(&visible_ns, "branch-target")),
            },
            READER,
            Some("missing-foreign-branch"),
        ))
        .await
        .unwrap_err();
    assert_eq!(foreign.code(), Code::NotFound);
}
