#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
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

async fn read_native(
    client: &mut Client,
    kind: pb::EntityKind,
    id: pb::Id,
    branch: Option<&str>,
) -> pb::GetEntityResponse {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(id),
            },
            ADMIN,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
fn native_ref(kind: pb::EntityKind, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(slug)),
    }
}
async fn create_branch(client: &mut Client, name: &str) {
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: name.into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
}
async fn merge(
    client: &mut Client,
    name: &str,
    dry_run: bool,
) -> Result<pb::MergeBranchResponse, tonic::Status> {
    client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: name.into(),
                dry_run,
                keep_branch: true,
                auto_merge: true,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}
#[tokio::test]
async fn hidden_term_referrer_blocks_safe_delete_without_disclosing_identity() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("term-delete-visible", "visible"),
        ("term-delete-private", "hidden"),
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
    let target = pb::Id {
        namespace: namespaces[0].clone(),
        ..id("visible-domain")
    };
    put_native(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(target.clone()),
            ..Default::default()
        })),
        None,
    )
    .await;
    let hidden = entity(pb::entity::Kind::Term(pb::Term {
        id: Some(pb::Id {
            namespace: namespaces[1].clone(),
            ..id("private-word")
        }),
        title: "Secret vocabulary title".into(),
        owning_domain: Some(pb::DomainRef {
            id: Some(target.clone()),
        }),
        ..Default::default()
    }));
    put_native(&mut client, hidden.clone(), None).await;
    let before = read_native(&mut client, pb::EntityKind::Domain, target.clone(), None).await;
    let error = client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: Some(target.clone()),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    for secret in [
        "private-word",
        "Secret vocabulary title",
        namespaces[1].as_str(),
    ] {
        assert!(!error.message().contains(secret), "{error}");
    }
    assert_eq!(
        read_native(&mut client, pb::EntityKind::Domain, target, None).await,
        before
    );
    let hidden_id = trogon_atlas_core::refs::entity_id(&hidden).unwrap().clone();
    let persisted = read_native(&mut client, pb::EntityKind::Term, hidden_id, None).await;
    assert!(trogon_atlas_core::semantically_equal(
        persisted.entity.as_ref().unwrap(),
        &hidden
    ));
}
#[tokio::test]
async fn exact_impact_and_guarded_retarget_preserve_plain_reference_and_owner_uid() {
    let mut client = client().await;
    for slug in ["impact-old", "impact-new"] {
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
    put_native(&mut client, term("impact-word", "impact-old"), None).await;
    let before = read_native(&mut client, pb::EntityKind::Term, id("impact-word"), None).await;
    let target = read_native(&mut client, pb::EntityKind::Domain, id("impact-new"), None).await;
    let version_two = pb::Id {
        version: 2,
        ..id("impact-old")
    };
    put_native(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(version_two.clone()),
            ..Default::default()
        })),
        None,
    )
    .await;
    put_native(
        &mut client,
        entity(pb::entity::Kind::Term(pb::Term {
            id: Some(id("other-version-word")),
            owning_domain: Some(pb::DomainRef {
                id: Some(version_two),
            }),
            ..Default::default()
        })),
        None,
    )
    .await;
    let other_version = read_native(
        &mut client,
        pb::EntityKind::Term,
        id("other-version-word"),
        None,
    )
    .await;
    let impact = client
        .get_impact(request(
            pb::GetImpactRequest {
                root: Some(native_ref(pb::EntityKind::Domain, "impact-old")),
                max_depth: 1,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(impact.nodes.len(), 2);
    let owner = impact
        .nodes
        .iter()
        .find(|node| node.entity.as_ref() == Some(&native_ref(pb::EntityKind::Term, "impact-word")))
        .unwrap();
    assert_eq!(owner.depth, 1);
    assert_eq!(owner.path.len(), 1);
    assert_eq!(owner.path[0].from_field, "owning_domain");
    assert_eq!(
        owner.path[0].to,
        Some(native_ref(pb::EntityKind::Domain, "impact-old"))
    );
    assert_eq!(
        owner.path[0].from_uid,
        before.entity.as_ref().unwrap().system.as_ref().unwrap().uid
    );
    assert!(owner.path[0].connection_id.is_none());
    let retarget = pb::RetargetReferencesRequest {
        from: Some(native_ref(pb::EntityKind::Domain, "impact-old")),
        to: Some(native_ref(pb::EntityKind::Domain, "impact-new")),
        filter_kinds: vec![pb::EntityKind::Term as i32],
        ..Default::default()
    };
    assert_eq!(
        client
            .retarget_references(request(retarget.clone(), READER, None))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    for dry_run in [true, false] {
        let missing = pb::RetargetReferencesRequest {
            to: Some(native_ref(pb::EntityKind::Domain, "absent-target")),
            dry_run,
            ..retarget.clone()
        };
        assert_eq!(
            client
                .retarget_references(request(missing, ADMIN, None))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
        assert_eq!(
            read_native(&mut client, pb::EntityKind::Term, id("impact-word"), None).await,
            before
        );
    }
    let preview = client
        .retarget_references(request(
            pb::RetargetReferencesRequest {
                dry_run: true,
                ..retarget.clone()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.rewritten, 1);
    assert_eq!(preview.failed, 0);
    assert_eq!(preview.results[0].fields, vec!["owning_domain"]);
    assert_eq!(
        read_native(&mut client, pb::EntityKind::Term, id("impact-word"), None).await,
        before
    );
    let applied = client
        .retarget_references(request(retarget, ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(applied.rewritten, 1);
    assert_eq!(applied.failed, 0);
    let after = read_native(&mut client, pb::EntityKind::Term, id("impact-word"), None).await;
    let mut expected = before.entity.unwrap();
    if let Some(pb::entity::Kind::Term(word)) = &mut expected.kind {
        word.owning_domain = Some(pb::DomainRef {
            id: Some(id("impact-new")),
        });
    }
    assert_eq!(after.entity, Some(expected));
    assert_ne!(after.etag, before.etag);
    assert_eq!(
        read_native(&mut client, pb::EntityKind::Domain, id("impact-new"), None).await,
        target
    );
    assert_eq!(
        read_native(
            &mut client,
            pb::EntityKind::Term,
            id("other-version-word"),
            None
        )
        .await,
        other_version
    );
}
#[tokio::test]
async fn branch_lands_term_and_domain_then_rebases_edits_without_rebinding_owner_identity() {
    let mut client = client().await;
    create_branch(&mut client, "term-land").await;
    put_native(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id("land-domain")),
            ..Default::default()
        })),
        Some("term-land"),
    )
    .await;
    put_native(
        &mut client,
        term("land-word", "land-domain"),
        Some("term-land"),
    )
    .await;
    let branch_word = read_native(
        &mut client,
        pb::EntityKind::Term,
        id("land-word"),
        Some("term-land"),
    )
    .await;
    let branch_domain = read_native(
        &mut client,
        pb::EntityKind::Domain,
        id("land-domain"),
        Some("term-land"),
    )
    .await;
    assert_eq!(
        merge(&mut client, "term-land", true).await.unwrap().status,
        pb::merge_branch_response::Status::Validated as i32
    );
    assert_eq!(
        merge(&mut client, "term-land", false).await.unwrap().status,
        pb::merge_branch_response::Status::Applied as i32
    );
    let baseline_word = read_native(&mut client, pb::EntityKind::Term, id("land-word"), None).await;
    let baseline_domain =
        read_native(&mut client, pb::EntityKind::Domain, id("land-domain"), None).await;
    assert_eq!(baseline_word.entity, branch_word.entity);
    assert_eq!(baseline_domain.entity, branch_domain.entity);
    let mut edited_word = baseline_word.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Term(word)) = &mut edited_word.kind {
        word.doc = "Branch definition".into();
    }
    put_native(&mut client, edited_word.clone(), Some("term-land")).await;
    let mut edited_domain = baseline_domain.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Domain(domain)) = &mut edited_domain.kind {
        domain.title = "Baseline domain title".into();
    }
    put_native(&mut client, edited_domain.clone(), None).await;
    let update = client
        .update_branch(request(
            pb::UpdateBranchRequest {
                name: "term-land".into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(update.conflicts.len(), 0);
    assert_eq!(
        read_native(
            &mut client,
            pb::EntityKind::Term,
            id("land-word"),
            Some("term-land")
        )
        .await
        .entity,
        Some(edited_word.clone())
    );
    assert_eq!(
        merge(&mut client, "term-land", false).await.unwrap().status,
        pb::merge_branch_response::Status::Applied as i32
    );
    let landed_word = read_native(&mut client, pb::EntityKind::Term, id("land-word"), None).await;
    assert_eq!(landed_word.entity, Some(edited_word));
    assert_ne!(landed_word.etag, baseline_word.etag);
    assert_eq!(
        read_native(&mut client, pb::EntityKind::Domain, id("land-domain"), None)
            .await
            .entity,
        Some(edited_domain)
    );
}

#[tokio::test]
async fn explicit_conflict_restoration_of_term_or_domain_preserves_trusted_branch_incarnation() {
    for kind in [pb::EntityKind::Term, pb::EntityKind::Domain] {
        let mut client = client().await;
        put_native(
            &mut client,
            entity(pb::entity::Kind::Domain(pb::Domain {
                id: Some(id("restore-domain")),
                ..Default::default()
            })),
            None,
        )
        .await;
        put_native(&mut client, term("restore-word", "restore-domain"), None).await;
        let slug = if kind == pb::EntityKind::Term {
            "restore-word"
        } else {
            "restore-domain"
        };
        let original = read_native(&mut client, kind, id(slug), None).await;
        let other_kind = if kind == pb::EntityKind::Term {
            pb::EntityKind::Domain
        } else {
            pb::EntityKind::Term
        };
        let other_slug = if kind == pb::EntityKind::Term {
            "restore-domain"
        } else {
            "restore-word"
        };
        let other = read_native(&mut client, other_kind, id(other_slug), None).await;
        create_branch(&mut client, "term-restore").await;
        let mut edited = original.entity.clone().unwrap();
        match &mut edited.kind {
            Some(pb::entity::Kind::Term(word)) => word.title = "Branch word".into(),
            Some(pb::entity::Kind::Domain(domain)) => domain.title = "Branch domain".into(),
            _ => unreachable!(),
        }
        put_native(&mut client, edited.clone(), Some("term-restore")).await;
        let branch = read_native(&mut client, kind, id(slug), Some("term-restore")).await;
        client
            .delete_entity(request(
                pb::DeleteEntityRequest {
                    kind: kind as i32,
                    id: Some(id(slug)),
                    mode: pb::delete_entity_request::Mode::Force as i32,
                    if_match: original.etag.clone(),
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap();
        let refused = merge(&mut client, "term-restore", false).await.unwrap();
        assert_eq!(
            refused.status,
            pb::merge_branch_response::Status::Conflicts as i32
        );
        assert_eq!(
            client
                .get_entity(request(
                    pb::GetEntityRequest {
                        kind: kind as i32,
                        id: Some(id(slug))
                    },
                    ADMIN,
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
        assert_eq!(
            read_native(&mut client, other_kind, id(other_slug), None).await,
            other
        );
        let update = client
            .update_branch(request(
                pb::UpdateBranchRequest {
                    name: "term-restore".into(),
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(update.conflicts.len(), 1);
        assert_eq!(
            read_native(&mut client, kind, id(slug), Some("term-restore")).await,
            branch
        );
        let state = update.conflicts[0].state.clone();
        assert!(state.is_some());
        let mut stale = state.clone().unwrap();
        stale.theirs_etag = original.etag.clone();
        assert_eq!(
            client
                .resolve_branch_entry(request(
                    pb::ResolveBranchEntryRequest {
                        name: "term-restore".into(),
                        r#ref: Some(native_ref(kind, slug)),
                        resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
                        expected_state: Some(stale),
                        ..Default::default()
                    },
                    ADMIN,
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::Aborted
        );
        assert_eq!(
            read_native(&mut client, kind, id(slug), Some("term-restore")).await,
            branch
        );
        client
            .resolve_branch_entry(request(
                pb::ResolveBranchEntryRequest {
                    name: "term-restore".into(),
                    r#ref: Some(native_ref(kind, slug)),
                    resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
                    expected_state: state,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            merge(&mut client, "term-restore", true)
                .await
                .unwrap()
                .status,
            pb::merge_branch_response::Status::Validated as i32
        );
        assert_eq!(
            merge(&mut client, "term-restore", false)
                .await
                .unwrap()
                .status,
            pb::merge_branch_response::Status::Applied as i32
        );
        let restored = read_native(&mut client, kind, id(slug), None).await;
        assert_eq!(restored.entity, Some(edited));
        assert_ne!(restored.etag, original.etag);
        assert_eq!(
            restored.entity.as_ref().unwrap().system,
            original.entity.as_ref().unwrap().system
        );
        assert_eq!(
            read_native(&mut client, other_kind, id(other_slug), None).await,
            other
        );
        let declaration = client
            .list_term_domain_declarations(request(
                domain_query(id("restore-domain")),
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(declaration.declarations.len(), 1);
        assert_eq!(
            declaration.declarations[0].state,
            pb::CatalogResolutionState::Resolved as i32
        );
        assert_eq!(
            declaration.declarations[0].domain,
            Some(pb::DomainRef {
                id: Some(id("restore-domain"))
            })
        );
    }
}

#[tokio::test]
async fn branch_term_landing_follows_recreated_exact_domain_key_without_uid_pin() {
    let mut client = client().await;
    let domain = entity(pb::entity::Kind::Domain(pb::Domain {
        id: Some(id("recreated-domain")),
        ..Default::default()
    }));
    put_native(&mut client, domain.clone(), None).await;
    put_native(
        &mut client,
        term("recreated-word", "recreated-domain"),
        None,
    )
    .await;
    let old_domain = read_native(
        &mut client,
        pb::EntityKind::Domain,
        id("recreated-domain"),
        None,
    )
    .await;
    let original_word = read_native(
        &mut client,
        pb::EntityKind::Term,
        id("recreated-word"),
        None,
    )
    .await;
    create_branch(&mut client, "term-recreated").await;
    let mut edited = original_word.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Term(word)) = &mut edited.kind {
        word.doc = "Reviewed definition".into();
    }
    put_native(&mut client, edited.clone(), Some("term-recreated")).await;
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: Some(id("recreated-domain")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let refusal = merge(&mut client, "term-recreated", true).await;
    assert_eq!(refusal.unwrap_err().code(), Code::InvalidArgument);
    assert_eq!(
        read_native(
            &mut client,
            pb::EntityKind::Term,
            id("recreated-word"),
            None
        )
        .await,
        original_word
    );
    put_native(&mut client, domain, None).await;
    let recreated = read_native(
        &mut client,
        pb::EntityKind::Domain,
        id("recreated-domain"),
        None,
    )
    .await;
    assert_ne!(
        recreated
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        old_domain
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
    );
    assert_ne!(recreated.etag, old_domain.etag);
    client
        .update_branch(request(
            pb::UpdateBranchRequest {
                name: "term-recreated".into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        merge(&mut client, "term-recreated", false)
            .await
            .unwrap()
            .status,
        pb::merge_branch_response::Status::Applied as i32
    );
    let landed = read_native(
        &mut client,
        pb::EntityKind::Term,
        id("recreated-word"),
        None,
    )
    .await;
    assert_eq!(landed.entity, Some(edited));
    assert_ne!(landed.etag, original_word.etag);
    assert_eq!(
        read_native(
            &mut client,
            pb::EntityKind::Domain,
            id("recreated-domain"),
            None
        )
        .await,
        recreated
    );
}

#[tokio::test]
async fn structural_merge_preserves_explicit_domain_clear_from_either_side() {
    for clear_on_branch in [true, false] {
        let mut client = client().await;
        put_native(
            &mut client,
            entity(pb::entity::Kind::Domain(pb::Domain {
                id: Some(id("clear-domain")),
                ..Default::default()
            })),
            None,
        )
        .await;
        put_native(&mut client, term("clear-word", "clear-domain"), None).await;
        let original = read_native(&mut client, pb::EntityKind::Term, id("clear-word"), None).await;
        let target = read_native(
            &mut client,
            pb::EntityKind::Domain,
            id("clear-domain"),
            None,
        )
        .await;
        create_branch(&mut client, "term-clear").await;
        let mut cleared = original.entity.clone().unwrap();
        if let Some(pb::entity::Kind::Term(word)) = &mut cleared.kind {
            word.owning_domain = None;
        }
        let mut documented = original.entity.clone().unwrap();
        if let Some(pb::entity::Kind::Term(word)) = &mut documented.kind {
            word.doc = "Reviewed definition".into();
        }
        if clear_on_branch {
            put_native(&mut client, cleared, Some("term-clear")).await;
            put_native(&mut client, documented, None).await;
        } else {
            put_native(&mut client, documented, Some("term-clear")).await;
            put_native(&mut client, cleared, None).await;
        }
        let captured = client
            .diff_branch(request(
                pb::DiffBranchRequest {
                    name: "term-clear".into(),
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            captured.entries[0].base, original.entity,
            "branch must capture the common ancestor before baseline changes; clear_on_branch={clear_on_branch}"
        );
        assert_eq!(
            merge(&mut client, "term-clear", true).await.unwrap().status,
            pb::merge_branch_response::Status::Validated as i32
        );
        assert_eq!(
            merge(&mut client, "term-clear", false)
                .await
                .unwrap()
                .status,
            pb::merge_branch_response::Status::Applied as i32
        );
        let after = read_native(&mut client, pb::EntityKind::Term, id("clear-word"), None).await;
        let mut expected = original.entity.unwrap();
        if let Some(pb::entity::Kind::Term(word)) = &mut expected.kind {
            word.doc = "Reviewed definition".into();
            word.owning_domain = None;
        }
        assert_eq!(after.entity, Some(expected));
        assert_ne!(after.etag, original.etag);
        assert_eq!(
            read_native(
                &mut client,
                pb::EntityKind::Domain,
                id("clear-domain"),
                None
            )
            .await,
            target
        );
        let declarations = client
            .list_term_domain_declarations(request(domain_query(id("clear-domain")), READER, None))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(declarations.declarations.len(), 0);
    }
}
#[tokio::test]
async fn domain_declaration_conflict_requires_guarded_choice_between_clear_and_exact_retarget() {
    for keep_ours in [true, false] {
        let mut client = client().await;
        for slug in ["choice-old", "choice-new"] {
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
        put_native(&mut client, term("choice-word", "choice-old"), None).await;
        let original =
            read_native(&mut client, pb::EntityKind::Term, id("choice-word"), None).await;
        let target = read_native(&mut client, pb::EntityKind::Domain, id("choice-new"), None).await;
        create_branch(&mut client, "term-choice").await;
        let mut retargeted = original.entity.clone().unwrap();
        if let Some(pb::entity::Kind::Term(word)) = &mut retargeted.kind {
            word.owning_domain = Some(pb::DomainRef {
                id: Some(id("choice-new")),
            });
        }
        put_native(&mut client, retargeted.clone(), Some("term-choice")).await;
        let mut cleared = original.entity.clone().unwrap();
        if let Some(pb::entity::Kind::Term(word)) = &mut cleared.kind {
            word.owning_domain = None;
        }
        put_native(&mut client, cleared.clone(), None).await;
        let baseline =
            read_native(&mut client, pb::EntityKind::Term, id("choice-word"), None).await;
        let refused = merge(&mut client, "term-choice", false).await.unwrap();
        assert_eq!(
            refused.status,
            pb::merge_branch_response::Status::Conflicts as i32
        );
        assert_eq!(
            read_native(&mut client, pb::EntityKind::Term, id("choice-word"), None).await,
            baseline
        );
        assert_eq!(refused.conflicts.len(), 1);
        client
            .resolve_branch_entry(request(
                pb::ResolveBranchEntryRequest {
                    name: "term-choice".into(),
                    r#ref: Some(native_ref(pb::EntityKind::Term, "choice-word")),
                    resolution: if keep_ours {
                        pb::resolve_branch_entry_request::Resolution::KeepOurs
                    } else {
                        pb::resolve_branch_entry_request::Resolution::TakeTheirs
                    } as i32,
                    expected_state: refused.conflicts[0].state.clone(),
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap();
        let resolved = read_native(
            &mut client,
            pb::EntityKind::Term,
            id("choice-word"),
            Some("term-choice"),
        )
        .await;
        assert_eq!(
            resolved.entity,
            Some(if keep_ours {
                retargeted.clone()
            } else {
                cleared.clone()
            })
        );
        assert_eq!(
            resolved.entity.as_ref().unwrap().system,
            original.entity.as_ref().unwrap().system
        );
        if keep_ours {
            assert_eq!(
                merge(&mut client, "term-choice", false)
                    .await
                    .unwrap()
                    .status,
                pb::merge_branch_response::Status::Applied as i32
            );
            let landed =
                read_native(&mut client, pb::EntityKind::Term, id("choice-word"), None).await;
            assert_eq!(landed.entity, Some(retargeted));
            assert_ne!(landed.etag, baseline.etag);
        } else {
            assert_eq!(
                read_native(&mut client, pb::EntityKind::Term, id("choice-word"), None).await,
                baseline
            );
        }
        assert_eq!(
            read_native(&mut client, pb::EntityKind::Domain, id("choice-new"), None).await,
            target
        );
    }
}

async fn seed_native_rows(store: &trogon_atlas_store::NatsStore, rows: Vec<pb::Entity>) {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    for chunk in rows.chunks(128) {
        let ops: Vec<_> = chunk
            .iter()
            .map(|value| MutationOp::Create {
                kind: trogon_atlas_core::refs::entity_kind(value).unwrap(),
                entity: value.clone(),
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn term_listing_row_boundary_pages_exhaust_exact_snapshot_and_rejects_overflow() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 10_000).await;
    put_native(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id("rows-domain")),
            ..Default::default()
        })),
        None,
    )
    .await;
    seed_native_rows(
        &store,
        (0..4_999)
            .map(|i| term(&format!("row-{i:04}"), "rows-domain"))
            .collect(),
    )
    .await;
    let mut query = pb::ListTermDomainDeclarationsRequest {
        page_size: 500,
        ..domain_query(id("rows-domain"))
    };
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = client
            .list_term_domain_declarations(request(query.clone(), READER, None))
            .await
            .unwrap()
            .into_inner();
        assert!(!page.incomplete);
        assert_eq!(page.declarations.len(), (4_999 - seen.len()).min(500));
        for entry in page.declarations {
            assert_eq!(entry.domain, query.domain);
            let owner = entry.owner.unwrap();
            assert_ne!(owner.uid, "");
            assert_ne!(owner.etag, "");
            assert!(seen.insert(owner.owner.unwrap().id.unwrap().slug));
        }
        if page.next_page_token.is_empty() {
            break;
        }
        query.page_token = page.next_page_token;
    }
    assert_eq!(seen.len() + 1, 5_000);
    seed_native_rows(&store, vec![term("row-overflow", "rows-domain")]).await;
    query.page_token.clear();
    let overflow = client
        .list_term_domain_declarations(request(query, READER, None))
        .await
        .unwrap_err();
    assert_eq!(overflow.code(), Code::ResourceExhausted);
    assert!(overflow.message().contains("entity budget"), "{overflow}");
}
#[tokio::test(flavor = "multi_thread")]
async fn term_listing_work_boundary_shares_real_directory_occurrences() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 10_000).await;
    put_native(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id("work-domain")),
            ..Default::default()
        })),
        None,
    )
    .await;
    seed_native_rows(
        &store,
        (0..1_000)
            .map(|i| {
                entity(pb::entity::Kind::Person(pb::Person {
                    id: Some(id(&format!("person-{i}"))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Person".into(),
                    }),
                    ..Default::default()
                }))
            })
            .collect(),
    )
    .await;
    seed_native_rows(
        &store,
        (0..10)
            .map(|i| {
                entity(pb::entity::Kind::Team(pb::Team {
                    id: Some(id(&format!("team-{i}"))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Team".into(),
                    }),
                    members: (0..if i == 9 { 999 } else { 1000 })
                        .map(|j| pb::TeamMember {
                            person: Some(pb::PersonRef {
                                id: Some(id(&format!("person-{j}"))),
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }))
            })
            .collect(),
    )
    .await;
    put_native(&mut client, term("work-word", "work-domain"), None).await;
    let query = domain_query(id("work-domain"));
    let boundary = client
        .list_term_domain_declarations(request(query.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(boundary.declarations.len(), 1);
    assert!(!boundary.incomplete);
    put_native(&mut client, term("work-overflow", "work-domain"), None).await;
    let overflow = client
        .list_term_domain_declarations(request(query, READER, None))
        .await
        .unwrap_err();
    assert_eq!(overflow.code(), Code::ResourceExhausted);
    assert!(overflow.message().contains("work"), "{overflow}");
}
#[tokio::test(flavor = "multi_thread")]
async fn term_subject_checked_scope_records_share_output_budget() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 10_000).await;
    let rows = (0..4_095)
        .map(|i| {
            entity(pb::entity::Kind::Term(pb::Term {
                id: Some(id(&format!("checked-{i}"))),
                ..Default::default()
            }))
        })
        .collect();
    seed_native_rows(&store, rows).await;
    let query = pb::ListSubjectDocumentsRequest {
        subject: Some(pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Entity(native_ref(
                pb::EntityKind::Term,
                "checked-0",
            ))),
        }),
    };
    let boundary = client
        .list_subject_documents(request(query.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    let scope = boundary.scope.unwrap();
    assert_eq!(
        boundary.declarations.len()
            + scope.visited.len()
            + scope.visited_subjects.len()
            + scope.visited_documents.len(),
        4_096
    );
    seed_native_rows(
        &store,
        vec![entity(pb::entity::Kind::Term(pb::Term {
            id: Some(id("checked-overflow")),
            ..Default::default()
        }))],
    )
    .await;
    let overflow = client
        .list_subject_documents(request(query, READER, None))
        .await
        .unwrap_err();
    assert_eq!(overflow.code(), Code::ResourceExhausted);
    assert!(overflow.message().contains("output"), "{overflow}");
}
