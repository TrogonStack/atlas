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
const LIMITED: &str = "document-paging-reader";

type Client = EventModelServiceClient<Channel>;

fn request<T>(body: T, branch: Option<&str>) -> Request<T> {
    request_as(body, ADMIN, branch)
}

fn request_as<T>(body: T, token: &str, branch: Option<&str>) -> Request<T> {
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
    client_with_store(std::sync::Arc::new(
        trogon_atlas_testsupport::shared().await.store().await,
    ))
    .await
}

async fn client_with_store(store: std::sync::Arc<dyn trogon_atlas_store::Store>) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.peer]
role = "admin"
tokens = ["document-paging-peer"]
[principals.limited]
role = "reader"
parent = "document-paging"
tokens = ["{LIMITED}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
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
async fn incoming_document_cursor_is_bound_to_caller_visibility() {
    let mut client = client().await;
    let namespace = client
        .register_namespace(request(
            pb::RegisterNamespaceRequest {
                name: "paging-visible".into(),
                parent: "document-paging".into(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let scoped_id = |slug: &str| pb::Id {
        namespace: namespace.clone(),
        slug: slug.into(),
        version: 1,
    };
    put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(scoped_id("receipt")),
                title: "Receipt".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let document = |slug: &str| pb::Entity {
        kind: Some(pb::entity::Kind::Document(pb::Document {
            id: Some(scoped_id(slug)),
            title: Some(pb::CatalogTitle { value: slug.into() }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                        kind: pb::EntityKind::ReadModel as i32,
                        id: Some(scoped_id("receipt")),
                    })),
                }),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/document".into(),
            })),
            ..Default::default()
        })),
        ..Default::default()
    };
    put(&mut client, document("hub")).await;
    let selection = hub_selection(scoped_id("hub"));
    for slug in ["first", "second"] {
        let mut owner = document(slug);
        let Some(pb::entity::Kind::Document(body)) = owner.kind.as_mut() else {
            panic!("expected document");
        };
        body.related_content.push(pb::RelatedContent {
            role: pb::RelatedContentRole::Example as i32,
            document: Some(selection.clone()),
            ..Default::default()
        });
        put(&mut client, owner).await;
    }
    let mut hidden = document("private-owner");
    let Some(pb::entity::Kind::Document(body)) = hidden.kind.as_mut() else {
        panic!("expected document");
    };
    body.id.as_mut().unwrap().namespace = "document-private".into();
    body.title = Some(pb::CatalogTitle {
        value: "Private owner detail".into(),
    });
    let hidden = put(&mut client, hidden).await;
    let hidden_uid = hidden.system.unwrap().uid;
    assert!(!hidden_uid.is_empty());
    let query = |page_token: String| pb::ListIncomingDocumentDeclarationsRequest {
        document: Some(selection.clone()),
        page_size: 1,
        page_token,
        ..Default::default()
    };
    let unrestricted = client
        .list_incoming_document_declarations(request(query(String::new()), None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(unrestricted.declarations.len(), 1);
    assert!(!unrestricted.next_page_token.is_empty());
    let restricted = client
        .list_incoming_document_declarations(request_as(query(String::new()), LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(restricted.declarations, unrestricted.declarations);
    let restricted_scope = restricted.scope.as_ref().unwrap();
    assert_eq!(restricted_scope.authorized_namespaces, [namespace]);
    let visible_receipt = format!("{restricted:?}");
    assert!(!visible_receipt.contains("document-private"));
    assert!(!visible_receipt.contains("Private owner detail"));
    assert!(!visible_receipt.contains(&hidden_uid));
    assert_ne!(
        restricted_scope.revision,
        unrestricted.scope.unwrap().revision
    );
    let refused = client
        .list_incoming_document_declarations(request_as(
            query(unrestricted.next_page_token),
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains("stale page token"));
    assert!(!refused.message().contains(&hidden_uid));
    assert!(!refused.message().contains("document-private"));
    let next = client
        .list_incoming_document_declarations(request_as(
            query(restricted.next_page_token),
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(next.declarations.len(), 1);
    assert_ne!(next.declarations, restricted.declarations);
    assert!(next.next_page_token.is_empty());
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
        let scope = page.scope.as_ref().unwrap();
        let receipt = scope.page_receipt.as_ref().expect("explicit page receipt");
        assert_eq!(receipt.checked_owner_count, (REFERRERS + FILLER + 1) as u64);
        assert_eq!(receipt.total_declarations, REFERRERS as u64);
        assert_eq!(receipt.start_offset, seen.len() as u64);
        assert_eq!(
            receipt.end_offset,
            (seen.len() + page.declarations.len()) as u64
        );
        assert!(scope.visited.len() <= page.declarations.len() + 1);
        assert!(
            page.declarations.len()
                + scope.visited.len()
                + scope.visited_subjects.len()
                + scope.visited_documents.len()
                + 1
                <= 4_096
        );
        for owner in &scope.visited {
            assert!(
                owner.owner.as_ref().unwrap().id.as_ref().unwrap().slug == "incoming-hub"
                    || page
                        .declarations
                        .iter()
                        .any(|declaration| declaration.owner.as_ref() == Some(owner))
            );
        }
        if !token.is_empty() {
            let mut replay = request(
                pb::ListIncomingDocumentDeclarationsRequest {
                    document: Some(whole.clone()),
                    page_size: page_size as i32,
                    page_token: token.clone(),
                    ..Default::default()
                },
                None,
            );
            replay.metadata_mut().insert(
                "authorization",
                "Bearer document-paging-peer".parse().unwrap(),
            );
            assert_eq!(
                client
                    .list_incoming_document_declarations(replay)
                    .await
                    .unwrap_err()
                    .code(),
                Code::InvalidArgument
            );
        }
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
    let first_request = pb::ListIncomingDocumentDeclarationsRequest {
        document: Some(whole.clone()),
        page_size: 1,
        ..Default::default()
    };
    let first = client
        .list_incoming_document_declarations(request(first_request.clone(), None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        first
            .scope
            .as_ref()
            .unwrap()
            .page_receipt
            .as_ref()
            .unwrap()
            .checked_owner_count,
        5_000
    );
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "paging-context".into(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap();
    let continuation = pb::ListIncomingDocumentDeclarationsRequest {
        page_token: first.next_page_token.clone(),
        ..first_request
    };
    assert_eq!(
        client
            .list_incoming_document_declarations(request(
                continuation.clone(),
                Some("paging-context")
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let mut filler = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("filler-0")),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    if let Some(pb::entity::Kind::Document(document)) = &mut filler.kind {
        document.title.as_mut().unwrap().value = "Unreturned scan owner changed".into();
    }
    put(&mut client, filler).await;
    assert_eq!(
        client
            .list_incoming_document_declarations(request(continuation, None))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
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

async fn seed(store: &trogon_atlas_store::NatsStore, entities: Vec<pb::Entity>) {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    for chunk in entities.chunks(128) {
        let ops: Vec<_> = chunk
            .iter()
            .map(|entity| MutationOp::Create {
                kind: trogon_atlas_core::entity_kind(entity).unwrap(),
                entity: entity.clone(),
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn incoming_document_small_pages_do_not_bypass_full_scan_row_and_work_limits() {
    let store = std::sync::Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone()).await;
    let query = pb::ListIncomingDocumentDeclarationsRequest {
        document: Some(hub_selection(id("absent-target"))),
        page_size: 1,
        ..Default::default()
    };
    seed(
        &store,
        (0..5_001)
            .map(|index| pb::Entity {
                kind: Some(pb::entity::Kind::Person(pb::Person {
                    id: Some(id(&format!("row-{index}"))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Person".into(),
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            })
            .collect(),
    )
    .await;
    let row_error = client
        .list_incoming_document_declarations(request(query.clone(), None))
        .await
        .unwrap_err();
    assert_eq!(row_error.code(), Code::ResourceExhausted);
    assert!(row_error.message().contains("entity budget"));

    let store = std::sync::Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone()).await;
    seed(
        &store,
        (0..1_000)
            .map(|index| pb::Entity {
                kind: Some(pb::entity::Kind::Person(pb::Person {
                    id: Some(id(&format!("member-{index}"))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Person".into(),
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            })
            .collect(),
    )
    .await;
    seed(
        &store,
        (0..10)
            .map(|index| pb::Entity {
                kind: Some(pb::entity::Kind::Team(pb::Team {
                    id: Some(id(&format!("team-{index}"))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Team".into(),
                    }),
                    members: (0..1_000)
                        .map(|member| pb::TeamMember {
                            person: Some(pb::PersonRef {
                                id: Some(id(&format!("member-{member}"))),
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })),
                ..Default::default()
            })
            .collect(),
    )
    .await;
    let boundary = client
        .list_incoming_document_declarations(request(query.clone(), None))
        .await
        .unwrap()
        .into_inner();
    assert!(boundary.declarations.is_empty());
    assert!(!boundary.incomplete);
    seed(
        &store,
        vec![pb::Entity {
            kind: Some(pb::entity::Kind::Team(pb::Team {
                id: Some(id("work-overflow")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Overflow".into(),
                }),
                members: vec![pb::TeamMember {
                    person: Some(pb::PersonRef {
                        id: Some(id("member-0")),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        }],
    )
    .await;
    let work_error = client
        .list_incoming_document_declarations(request(query, None))
        .await
        .unwrap_err();
    assert_eq!(work_error.code(), Code::ResourceExhausted);
    assert!(work_error.message().contains("work budget"));
}
