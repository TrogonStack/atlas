#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{Code, Request, service::interceptor::InterceptedService, transport::Channel};
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
    client_with_cap(0).await
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

fn term(slug: &str, domain: &str) -> pb::Entity {
    entity(pb::entity::Kind::Term(pb::Term {
        id: Some(id(slug)),
        owning_domain: Some(pb::DomainRef {
            id: Some(id(domain)),
        }),
        ..Default::default()
    }))
}

#[tokio::test]
async fn exact_domain_admission_and_paged_reader_query() {
    let mut client = client().await;
    let missing = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(term("word", "missing")),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::InvalidArgument);
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity(pb::entity::Kind::Domain(pb::Domain {
                    id: Some(id("domain")),
                    ..Default::default()
                }))),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    for slug in ["alpha", "beta"] {
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(term(slug, "domain")),
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap();
    }
    let query = pb::ListTermDomainDeclarationsRequest {
        domain: Some(pb::DomainRef {
            id: Some(id("domain")),
        }),
        page_size: 1,
        ..Default::default()
    };
    let first = client
        .list_term_domain_declarations(request(query.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.declarations.len(), 1);
    assert!(!first.incomplete);
    assert_ne!(first.next_page_token, "");
    let second = client
        .list_term_domain_declarations(request(
            pb::ListTermDomainDeclarationsRequest {
                page_token: first.next_page_token.clone(),
                ..query.clone()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.declarations.len(), 1);
    assert_ne!(first.declarations[0].owner, second.declarations[0].owner);
    assert_eq!(second.next_page_token, "");
    let changed_caller = client
        .list_term_domain_declarations(request(
            pb::ListTermDomainDeclarationsRequest {
                page_token: first.next_page_token,
                ..query
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(changed_caller.code(), Code::InvalidArgument);
    let denied = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(term("denied", "domain")),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn batch_target_and_persisted_reapply_preserve_exact_ownership() {
    let mut client = client().await;
    let domain = entity(pb::entity::Kind::Domain(pb::Domain {
        id: Some(id("batch-domain")),
        ..Default::default()
    }));
    let word = term("batch-word", "batch-domain");
    client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: [word.clone(), domain]
                    .into_iter()
                    .map(|entity| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(entity),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let first = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(word.clone()),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(first.no_op);
    let Some(pb::entity::Kind::Term(persisted)) = first.entity.unwrap().kind else {
        panic!("term expected")
    };
    assert_eq!(
        persisted.owning_domain,
        Some(pb::DomainRef {
            id: Some(id("batch-domain"))
        })
    );
    for invalid in [
        pb::DomainRef { id: None },
        pb::DomainRef {
            id: Some(pb::Id {
                version: 0,
                ..id("batch-domain")
            }),
        },
        pb::DomainRef {
            id: Some(pb::Id {
                namespace: "../unsafe".into(),
                ..id("batch-domain")
            }),
        },
    ] {
        let mut invalid_word = word.clone();
        if let Some(pb::entity::Kind::Term(term)) = &mut invalid_word.kind {
            term.owning_domain = Some(invalid);
        }
        assert_eq!(
            client
                .put_entity(request(
                    pb::PutEntityRequest {
                        entity: Some(invalid_word),
                        ..Default::default()
                    },
                    ADMIN,
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
    }
}

#[tokio::test]
async fn deletion_keeps_declarations_inspectable_and_plain_refs_follow_recreation() {
    let mut client = client().await;
    let domain = entity(pb::entity::Kind::Domain(pb::Domain {
        id: Some(id("lifecycle-domain")),
        ..Default::default()
    }));
    let create = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(domain.clone()),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let word = term("lifecycle-word", "lifecycle-domain");
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(word),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let incoming = client
        .get_incoming_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Domain as i32,
                id: Some(id("lifecycle-domain")),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.references.len(), 1);
    assert_eq!(incoming.references[0].from_field, "owning_domain");
    assert!(incoming.references[0].connection_id.is_none());
    let delete = pb::DeleteEntityRequest {
        kind: pb::EntityKind::Domain as i32,
        id: Some(id("lifecycle-domain")),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        ..Default::default()
    };
    assert_eq!(
        client
            .delete_entity(request(delete.clone(), ADMIN, None))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..delete
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let query = pb::ListTermDomainDeclarationsRequest {
        domain: Some(pb::DomainRef {
            id: Some(id("lifecycle-domain")),
        }),
        ..Default::default()
    };
    let dangling = client
        .list_term_domain_declarations(request(query.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(dangling.declarations.len(), 1);
    assert_eq!(
        dangling.declarations[0].state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    let recreated = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(domain),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(
        create.entity.unwrap().system.unwrap().uid,
        recreated.entity.unwrap().system.unwrap().uid
    );
    let rebound = client
        .list_term_domain_declarations(request(query, READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        rebound.declarations[0].state,
        pb::CatalogResolutionState::Resolved as i32
    );
    assert_eq!(
        rebound.declarations[0].domain,
        dangling.declarations[0].domain
    );
}

async fn put_native(client: &mut Client, value: pb::Entity, branch: Option<&str>) {
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
        .unwrap();
}
fn domain_query(id: pb::Id) -> pb::ListTermDomainDeclarationsRequest {
    pb::ListTermDomainDeclarationsRequest {
        domain: Some(pb::DomainRef { id: Some(id) }),
        page_size: 1,
        ..Default::default()
    }
}

#[tokio::test]
async fn tokens_bind_filter_branch_and_snapshot_and_reject_invalid_requests() {
    let mut client = client().await;
    for slug in ["token-domain", "other-domain"] {
        put_native(
            &mut client,
            entity(pb::entity::Kind::Domain(pb::Domain {
                id: Some(id(slug)),
                ..Default::default()
            })),
            None,
        )
        .await;
    }
    for slug in ["token-a", "token-b"] {
        put_native(&mut client, term(slug, "token-domain"), None).await;
    }
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "term-token-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let query = domain_query(id("token-domain"));
    let first = client
        .list_term_domain_declarations(request(query.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        first.declarations[0]
            .owner
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "token-a"
    );
    let continuation = pb::ListTermDomainDeclarationsRequest {
        page_token: first.next_page_token,
        ..query.clone()
    };
    for (query, branch) in [
        (continuation.clone(), Some("term-token-branch")),
        (
            pb::ListTermDomainDeclarationsRequest {
                domain: Some(pb::DomainRef {
                    id: Some(id("other-domain")),
                }),
                ..continuation.clone()
            },
            None,
        ),
    ] {
        assert_eq!(
            client
                .list_term_domain_declarations(request(query, READER, branch))
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
    }
    put_native(
        &mut client,
        term("branch-only", "token-domain"),
        Some("term-token-branch"),
    )
    .await;
    let branch = client
        .list_term_domain_declarations(request(query.clone(), READER, Some("term-token-branch")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        branch.declarations[0]
            .owner
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "branch-only"
    );
    let baseline = client
        .list_term_domain_declarations(request(query.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        baseline.declarations[0]
            .owner
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "token-a"
    );
    put_native(&mut client, term("token-c", "token-domain"), None).await;
    assert_eq!(
        client
            .list_term_domain_declarations(request(continuation, READER, None))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    for invalid in [
        pb::ListTermDomainDeclarationsRequest::default(),
        pb::ListTermDomainDeclarationsRequest {
            page_size: 501,
            ..query.clone()
        },
        pb::ListTermDomainDeclarationsRequest {
            page_token: "malformed".into(),
            ..query.clone()
        },
        domain_query(pb::Id {
            version: 0,
            ..id("token-domain")
        }),
        domain_query(pb::Id {
            namespace: "../unsafe".into(),
            ..id("token-domain")
        }),
    ] {
        assert_eq!(
            client
                .list_term_domain_declarations(request(invalid, READER, None))
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
    }
}

#[tokio::test]
async fn namespace_visibility_and_hidden_target_admission_are_independent_of_term_identity() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [("term-visible", "visible"), ("term-hidden", "hidden")] {
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
    let target_id = pb::Id {
        namespace: namespaces[0].clone(),
        ..id("shared-domain")
    };
    let hidden_id = pb::Id {
        namespace: namespaces[1].clone(),
        ..id("private-domain")
    };
    for target in [&target_id, &hidden_id] {
        put_native(
            &mut client,
            entity(pb::entity::Kind::Domain(pb::Domain {
                id: Some(target.clone()),
                title: "Private target title".into(),
                ..Default::default()
            })),
            None,
        )
        .await;
    }
    for namespace in &namespaces {
        for version in [1, 2] {
            put_native(
                &mut client,
                entity(pb::entity::Kind::Term(pb::Term {
                    id: Some(pb::Id {
                        namespace: namespace.clone(),
                        version,
                        ..id("equal-title")
                    }),
                    title: "Equal title".into(),
                    doc: "Native definition".into(),
                    owning_domain: Some(pb::DomainRef {
                        id: Some(target_id.clone()),
                    }),
                    ..Default::default()
                })),
                None,
            )
            .await;
        }
    }
    let query = pb::ListTermDomainDeclarationsRequest {
        page_size: 500,
        ..domain_query(target_id.clone())
    };
    let visible = client
        .list_term_domain_declarations(request(query.clone(), LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(visible.declarations.len(), 2);
    assert_ne!(visible.declarations[0].owner, visible.declarations[1].owner);
    assert!(visible.declarations.iter().all(|d| {
        d.owner
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .namespace
            == namespaces[0]
    }));
    assert_eq!(
        client
            .list_term_domain_declarations(request(query, ADMIN, None))
            .await
            .unwrap()
            .into_inner()
            .declarations
            .len(),
        4
    );
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Domain as i32,
                    id: Some(hidden_id.clone())
                },
                LIMITED,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let mut messages = Vec::new();
    for target in [
        hidden_id.clone(),
        pb::Id {
            slug: "missing".into(),
            ..hidden_id
        },
    ] {
        let value = entity(pb::entity::Kind::Term(pb::Term {
            id: Some(pb::Id {
                namespace: namespaces[0].clone(),
                ..id("invalid-owner")
            }),
            owning_domain: Some(pb::DomainRef { id: Some(target) }),
            ..Default::default()
        }));
        let error = client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(value),
                    ..Default::default()
                },
                LIMITED,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(!error.message().contains("Private target title"));
        messages.push(error.message().to_string());
    }
    assert_eq!(messages[0], messages[1]);
}

#[tokio::test]
async fn branch_force_delete_keeps_baseline_and_rejects_only_edited_dangling_owner() {
    let mut client = client().await;
    put_native(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id("branch-domain")),
            ..Default::default()
        })),
        None,
    )
    .await;
    let word = term("branch-word", "branch-domain");
    put_native(&mut client, word.clone(), None).await;
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "term-lifecycle".into(),
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
                kind: pb::EntityKind::Domain as i32,
                id: Some(id("branch-domain")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            Some("term-lifecycle"),
        ))
        .await
        .unwrap();
    for (branch, expected) in [
        (None, pb::CatalogResolutionState::Resolved),
        (
            Some("term-lifecycle"),
            pb::CatalogResolutionState::Unavailable,
        ),
    ] {
        let result = client
            .list_term_domain_declarations(request(
                domain_query(id("branch-domain")),
                READER,
                branch,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(result.declarations.len(), 1);
        assert_eq!(result.declarations[0].state, expected as i32);
    }
    put_native(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id("unrelated-domain")),
            ..Default::default()
        })),
        Some("term-lifecycle"),
    )
    .await;
    let mut edited = word;
    if let Some(pb::entity::Kind::Term(term)) = &mut edited.kind {
        term.title = "Edited dangling owner".into();
    }
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(edited),
                    ..Default::default()
                },
                ADMIN,
                Some("term-lifecycle")
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let persisted = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Term as i32,
                id: Some(id("branch-word")),
            },
            READER,
            Some("term-lifecycle"),
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let Some(pb::entity::Kind::Term(persisted)) = persisted.kind else {
        panic!("term expected")
    };
    assert_eq!(persisted.title, "");
    assert_eq!(
        persisted.owning_domain,
        Some(pb::DomainRef {
            id: Some(id("branch-domain"))
        })
    );
}
