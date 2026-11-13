#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

#[path = "support/observed_glossary_store.rs"]
mod observed_glossary_store;

type Client = EventModelServiceClient<Channel>;
const ADMIN: &str = "glossary-boundary-admin";
const READER: &str = "glossary-boundary-reader";
const LIMITED: &str = "glossary-boundary-limited";
const SOURCE: &str = "A purchase is a customer request accepted by the store.";

fn request<T>(body: T, identity: &str, branch: Option<&str>) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {identity}").parse().unwrap(),
    );
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    request
}

async fn client() -> Client {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    client_with_store(Arc::new(store)).await
}

async fn client_with_store(store: Arc<dyn trogon_atlas_store::Store>) -> Client {
    client_with_store_and_directory(
        store,
        Arc::new(trogon_atlas_server::ownership::NamespaceDirectory::new()),
    )
    .await
}

async fn client_with_store_and_directory(
    store: Arc<dyn trogon_atlas_store::Store>,
    directory: Arc<trogon_atlas_server::ownership::NamespaceDirectory>,
) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.expert]
role = "admin"
tokens = ["{ADMIN}"]
[principals.reader]
role = "reader"
tokens = ["{READER}"]
[principals.limited]
role = "writer"
parent = "visible"
tokens = ["{LIMITED}"]
"#,
        ))
        .unwrap(),
    )
    .unwrap();
    let service =
        EventModelServiceImpl::try_new_with_directory(store, directory, ServerMetrics::default())
            .unwrap();
    let grpc = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(pb::event_model_service_server::EventModelServiceServer::new(service)),
        BearerAuth::new(Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Client::connect(format!("http://{address}")).await.unwrap()
}

fn id(namespace: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version,
    }
}

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

fn term(value: &pb::Entity) -> &pb::Term {
    let Some(pb::entity::Kind::Term(term)) = value.kind.as_ref() else {
        panic!("term fixture required");
    };
    term
}

fn term_mut(value: &mut pb::Entity) -> &mut pb::Term {
    let Some(pb::entity::Kind::Term(term)) = value.kind.as_mut() else {
        panic!("term fixture required");
    };
    term
}

async fn put(
    client: &mut Client,
    entity: pb::Entity,
    etag: &str,
    branch: Option<&str>,
) -> pb::PutEntityResponse {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: etag.into(),
                create_only: etag.is_empty(),
                ..Default::default()
            },
            ADMIN,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn read(
    client: &mut Client,
    kind: pb::EntityKind,
    id: &pb::Id,
    branch: Option<&str>,
) -> pb::GetEntityResponse {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(id.clone()),
            },
            ADMIN,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn source(
    client: &mut Client,
    namespace: &str,
    slug: &str,
    branch: Option<&str>,
) -> pb::PutEntityResponse {
    let anchor = id(namespace, &format!("{slug}-fact"), 1);
    put(
        client,
        entity(pb::entity::Kind::Event(pb::Event {
            id: Some(anchor.clone()),
            title: "Source fact".into(),
            ..Default::default()
        })),
        "",
        branch,
    )
    .await;
    put(
        client,
        entity(pb::entity::Kind::Document(pb::Document {
            id: Some(id(namespace, slug, 1)),
            title: Some(pb::CatalogTitle {
                value: "Controlled vocabulary source".into(),
            }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                        kind: pb::EntityKind::Event as i32,
                        id: Some(anchor),
                    })),
                }),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                content: SOURCE.into(),
                media_type: Some(pb::MediaType {
                    value: "text/plain".into(),
                }),
                digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
            })),
            ..Default::default()
        })),
        "",
        branch,
    )
    .await
}

fn candidate(source: &pb::PutEntityResponse, slug: &str, version: u64) -> pb::Entity {
    let source = source.entity.as_ref().unwrap();
    let source_id = trogon_atlas_core::entity_id(source).unwrap();
    entity(pb::entity::Kind::Term(pb::Term {
        id: Some(id(&source_id.namespace, slug, version)),
        title: format!("Purchase {slug}"),
        doc: "A customer request accepted by the store.".into(),
        glossary: Some(pb::GlossaryEntry {
            evidence: vec![pb::GlossaryEvidence {
                document: Some(pb::DocumentRef {
                    id: Some(source_id.clone()),
                }),
                owner_uid: source.system.as_ref().unwrap().uid.clone(),
                content_digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
                locator: "paragraph:1".into(),
                quote: "a customer request accepted by the store".into(),
            }],
            generation_revision: "boundary-fixture/prompt-v1".into(),
            audience: "customer".into(),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

fn selection(namespace: &str, include_unreviewed: bool) -> pb::GlossarySelection {
    pb::GlossarySelection {
        namespaces: vec![namespace.into()],
        audience: "customer".into(),
        include_unreviewed,
    }
}

async fn review(
    client: &mut Client,
    term_id: &pb::Id,
    etag: &str,
    validate_only: bool,
    branch: Option<&str>,
) -> Result<pb::GetGlossaryTermResponse, tonic::Status> {
    client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(term_id.clone()),
                }),
                if_match: etag.into(),
                disposition: pb::GlossaryReviewDisposition::Accepted as i32,
                reason: "Controlled authenticated review of the exact source and definition."
                    .into(),
                validate_only,
            },
            ADMIN,
            branch,
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn detail(client: &mut Client, term_id: &pb::Id) -> pb::GetGlossaryTermResponse {
    client
        .get_glossary_term(request(
            pb::GetGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(term_id.clone()),
                }),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn batch_cannot_forge_a_valid_review_or_publish_sibling_operations() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    let before = read(
        &mut client,
        pb::EntityKind::Term,
        &id("shop", "purchase", 1),
        None,
    )
    .await;
    let mut forged = draft.entity.unwrap();
    let owner_uid = forged.system.as_ref().unwrap().uid.clone();
    let content_digest = trogon_atlas_core::glossary::candidate_digest(term(&forged)).unwrap();
    term_mut(&mut forged).glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
        disposition: pb::GlossaryReviewDisposition::Accepted as i32,
        actor: "expert".into(),
        reviewed_at: "2026-11-12T12:00:00Z".into(),
        content_digest: Some(content_digest),
        owner_uid,
        reason: "Forged but structurally valid review.".into(),
    });
    let sibling = entity(pb::entity::Kind::Event(pb::Event {
        id: Some(id("shop", "sibling", 1)),
        title: "Must not persist".into(),
        ..Default::default()
    }));
    let result = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: vec![
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(sibling),
                            create_only: true,
                            ..Default::default()
                        })),
                    },
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(forged),
                            if_match: draft.etag,
                            ..Default::default()
                        })),
                    },
                ],
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await;
    assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
    let after = read(
        &mut client,
        pb::EntityKind::Term,
        &id("shop", "purchase", 1),
        None,
    )
    .await;
    assert_eq!(after, before);
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("shop", "sibling", 1)),
                },
                ADMIN,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test]
async fn accepted_meaning_is_immutable_but_schedule_changes_preserve_review() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    let accepted = review(
        &mut client,
        &id("shop", "purchase", 1),
        &draft.etag,
        false,
        None,
    )
    .await
    .unwrap();
    let mut changed = accepted.entity.clone().unwrap();
    term_mut(&mut changed).doc = "An unrelated unsupported meaning.".into();
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(changed),
                    if_match: accepted.etag.clone(),
                    ..Default::default()
                },
                ADMIN,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let mut scheduled = accepted.entity.clone().unwrap();
    term_mut(&mut scheduled)
        .glossary
        .as_mut()
        .unwrap()
        .next_review_on = Some(pb::CalendarDate {
        value: "9999-12-31".into(),
    });
    let scheduled = put(&mut client, scheduled, &accepted.etag, None).await;
    assert_eq!(
        term(scheduled.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .review,
        term(accepted.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .review
    );
    let current = detail(&mut client, &id("shop", "purchase", 1)).await;
    assert!(current.entry.unwrap().published);
    assert_eq!(
        current.evidence[0].state,
        pb::GlossaryEvidenceState::Current as i32
    );
}

#[tokio::test]
async fn authenticated_review_preview_is_not_a_persisted_review() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    let before = read(
        &mut client,
        pb::EntityKind::Term,
        &id("shop", "purchase", 1),
        None,
    )
    .await;
    let preview = review(
        &mut client,
        &id("shop", "purchase", 1),
        &draft.etag,
        true,
        None,
    )
    .await
    .unwrap();
    assert!(!preview.entry.unwrap().published);
    assert_eq!(
        read(
            &mut client,
            pb::EntityKind::Term,
            &id("shop", "purchase", 1),
            None
        )
        .await,
        before
    );
    let listing = client
        .list_glossary_entries(request(
            pb::ListGlossaryEntriesRequest {
                selection: Some(selection("shop", false)),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listing.entries, Vec::new());
    let accepted = review(
        &mut client,
        &id("shop", "purchase", 1),
        &draft.etag,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(accepted.entry.unwrap().published);
}

#[tokio::test]
async fn omitted_audience_preserves_distinct_published_audience_versions() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    for (version, audience) in [(1, "customer"), (2, "internal")] {
        let mut value = candidate(&source, "purchase", version);
        term_mut(&mut value).glossary.as_mut().unwrap().audience = audience.into();
        let draft = put(&mut client, value, "", None).await;
        review(
            &mut client,
            &id("shop", "purchase", version),
            &draft.etag,
            false,
            None,
        )
        .await
        .unwrap();
    }
    let all = client
        .list_glossary_entries(request(
            pb::ListGlossaryEntriesRequest {
                selection: Some(pb::GlossarySelection {
                    audience: String::new(),
                    ..selection("shop", false)
                }),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(all.entries.len(), 2);
    let audiences = all
        .entries
        .iter()
        .map(|entry| entry.audience.as_str())
        .collect::<Vec<_>>();
    assert!(audiences.contains(&"customer") && audiences.contains(&"internal"));
    for (audience, version) in [("customer", 1), ("internal", 2)] {
        let listing = client
            .list_glossary_entries(request(
                pb::ListGlossaryEntriesRequest {
                    selection: Some(pb::GlossarySelection {
                        audience: audience.into(),
                        ..selection("shop", false)
                    }),
                    ..Default::default()
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(
            listing.entries[0]
                .term
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .version,
            version
        );
    }
}

async fn list(
    client: &mut Client,
    selection: pb::GlossarySelection,
    page_token: &str,
    identity: &str,
    branch: Option<&str>,
) -> Result<pb::ListGlossaryEntriesResponse, tonic::Status> {
    client
        .list_glossary_entries(request(
            pb::ListGlossaryEntriesRequest {
                selection: Some(selection),
                page_size: 1,
                page_token: page_token.into(),
            },
            identity,
            branch,
        ))
        .await
        .map(tonic::Response::into_inner)
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

async fn delete(
    client: &mut Client,
    kind: pb::EntityKind,
    id: &pb::Id,
    etag: &str,
    mode: pb::delete_entity_request::Mode,
) -> Result<pb::DeleteEntityResponse, tonic::Status> {
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: kind as i32,
                id: Some(id.clone()),
                if_match: etag.into(),
                mode: mode as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test]
async fn stale_review_etag_and_page_tokens_cannot_authorize_moved_scope() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let original = put(&mut client, candidate(&source, "alpha", 1), "", None).await;
    put(&mut client, candidate(&source, "beta", 1), "", None).await;
    let mut corrected = original.entity.unwrap();
    term_mut(&mut corrected).doc = "A corrected customer request definition.".into();
    let corrected = put(&mut client, corrected, &original.etag, None).await;
    assert_eq!(
        review(
            &mut client,
            &id("shop", "alpha", 1),
            &original.etag,
            false,
            None
        )
        .await
        .unwrap_err()
        .code(),
        Code::Aborted
    );
    assert_eq!(
        read(
            &mut client,
            pb::EntityKind::Term,
            &id("shop", "alpha", 1),
            None
        )
        .await
        .etag,
        corrected.etag
    );
    let first = list(&mut client, selection("shop", true), "", READER, None)
        .await
        .unwrap();
    assert_ne!(first.next_page_token, "");
    let second = list(
        &mut client,
        selection("shop", true),
        &first.next_page_token,
        READER,
        None,
    )
    .await
    .unwrap();
    assert_ne!(first.entries[0].term, second.entries[0].term);
    assert_eq!(first.snapshot_revision, second.snapshot_revision);
    for changed in [
        pb::GlossarySelection {
            audience: "internal".into(),
            ..selection("shop", true)
        },
        selection("shop", false),
    ] {
        assert_eq!(
            list(&mut client, changed, &first.next_page_token, READER, None)
                .await
                .unwrap_err()
                .code(),
            Code::FailedPrecondition
        );
    }
    create_branch(&mut client, "glossary-pages").await;
    assert_eq!(
        list(
            &mut client,
            selection("shop", true),
            &first.next_page_token,
            READER,
            Some("glossary-pages")
        )
        .await
        .unwrap_err()
        .code(),
        Code::FailedPrecondition
    );
    let mut changed_source = source.entity.unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed_source.kind else {
        panic!("document required");
    };
    let text = "The source material changed after catalog pagination began.";
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: text.into(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(text)),
    }));
    put(&mut client, changed_source, &source.etag, None).await;
    assert_eq!(
        list(
            &mut client,
            selection("shop", true),
            &first.next_page_token,
            READER,
            None
        )
        .await
        .unwrap_err()
        .code(),
        Code::FailedPrecondition
    );
}

#[tokio::test]
async fn restricted_readers_cannot_distinguish_hidden_and_missing_terms_or_source_bytes() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("glossary-visible", "visible"),
        ("glossary-hidden", "hidden"),
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
    let visible = &namespaces[0];
    let hidden = &namespaces[1];
    let visible_source = source(&mut client, visible, "source", None).await;
    let hidden_source = source(&mut client, hidden, "source", None).await;
    put(&mut client, candidate(&visible_source, "word", 1), "", None).await;
    put(
        &mut client,
        candidate(&visible_source, "other", 1),
        "",
        None,
    )
    .await;
    put(
        &mut client,
        candidate(&hidden_source, "secret-word", 1),
        "",
        None,
    )
    .await;
    let visible_page = list(&mut client, selection(visible, true), "", ADMIN, None)
        .await
        .unwrap();
    assert_eq!(
        list(
            &mut client,
            selection(visible, true),
            &visible_page.next_page_token,
            LIMITED,
            None
        )
        .await
        .unwrap_err()
        .code(),
        Code::FailedPrecondition
    );
    let limited_page = list(&mut client, selection(visible, true), "", LIMITED, None)
        .await
        .unwrap();
    assert!(limited_page.entries.iter().all(|entry| entry
        .term
        .as_ref()
        .unwrap()
        .id
        .as_ref()
        .unwrap()
        .namespace
        == *visible));
    let mut errors = Vec::new();
    let mut unavailable_sources = Vec::new();
    for slug in ["secret-word", "never-existed"] {
        let error = client
            .get_glossary_term(request(
                pb::GetGlossaryTermRequest {
                    term: Some(pb::TermRef {
                        id: Some(id(hidden, slug, 1)),
                    }),
                },
                LIMITED,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::NotFound);
        errors.push(error.message().to_owned());
        let material = client
            .inspect_glossary_sources(request(
                pb::InspectGlossarySourcesRequest {
                    documents: vec![pb::DocumentRef {
                        id: Some(id(
                            hidden,
                            if slug == "secret-word" {
                                "source"
                            } else {
                                "missing"
                            },
                            1,
                        )),
                    }],
                },
                LIMITED,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(material.incomplete);
        let mut unavailable = material.sources.into_iter().next().unwrap();
        assert_eq!(
            unavailable.state,
            pb::GlossaryEvidenceState::Unavailable as i32
        );
        assert_eq!(unavailable.text, "");
        assert_eq!(unavailable.etag, "");
        assert_eq!(unavailable.owner_uid, "");
        assert!(unavailable.content_digest.is_none());
        unavailable.document = None;
        unavailable_sources.push(unavailable);
    }
    assert_eq!(errors[0], errors[1]);
    assert_eq!(unavailable_sources[0], unavailable_sources[1]);
}

#[tokio::test]
async fn source_and_preferred_term_references_block_safe_deletion() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let preferred = put(
        &mut client,
        entity(pb::entity::Kind::Term(pb::Term {
            id: Some(id("shop", "order", 1)),
            title: "Order".into(),
            doc: "A customer order.".into(),
            ..Default::default()
        })),
        "",
        None,
    )
    .await;
    let mut proposed = candidate(&source, "purchase", 1);
    term_mut(&mut proposed).glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Deprecated as i32,
        preferred_term: Some(pb::TermRef {
            id: Some(id("shop", "order", 1)),
        }),
        rationale: "Use the customer-facing term Order.".into(),
        ..Default::default()
    });
    let declared = put(&mut client, proposed, "", None).await;
    for (kind, target, etag, expected_path) in [
        (
            pb::EntityKind::Document,
            id("shop", "source", 1),
            source.etag.as_str(),
            "glossary.evidence[0].document",
        ),
        (
            pb::EntityKind::Term,
            id("shop", "order", 1),
            preferred.etag.as_str(),
            "glossary.policy.preferred_term",
        ),
    ] {
        let incoming = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: kind as i32,
                    id: Some(target.clone()),
                    ..Default::default()
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(incoming
            .references
            .iter()
            .any(|reference| reference.from_field == expected_path));
        assert_eq!(
            delete(
                &mut client,
                kind,
                &target,
                etag,
                pb::delete_entity_request::Mode::FailIfReferenced
            )
            .await
            .unwrap_err()
            .code(),
            Code::FailedPrecondition
        );
        assert_eq!(read(&mut client, kind, &target, None).await.etag, etag);
    }
    assert_eq!(
        read(
            &mut client,
            pb::EntityKind::Term,
            &id("shop", "purchase", 1),
            None
        )
        .await
        .entity,
        declared.entity
    );
}

#[tokio::test]
async fn branch_review_and_landing_preserve_exact_term_and_source_incarnations() {
    let mut client = client().await;
    let branch = "glossary-reviewed-branch";
    create_branch(&mut client, branch).await;
    let source = source(&mut client, "shop", "source", Some(branch)).await;
    let draft = put(
        &mut client,
        candidate(&source, "purchase", 1),
        "",
        Some(branch),
    )
    .await;
    let accepted = review(
        &mut client,
        &id("shop", "purchase", 1),
        &draft.etag,
        false,
        Some(branch),
    )
    .await
    .unwrap();
    assert!(accepted.entry.as_ref().unwrap().published);
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Term as i32,
                    id: Some(id("shop", "purchase", 1)),
                },
                ADMIN,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    for dry_run in [true, false] {
        let merged = client
            .merge_branch(request(
                pb::MergeBranchRequest {
                    name: branch.into(),
                    dry_run,
                    keep_branch: true,
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
            if dry_run {
                pb::merge_branch_response::Status::Validated as i32
            } else {
                pb::merge_branch_response::Status::Applied as i32
            },
            "{merged:?}"
        );
    }
    let landed = detail(&mut client, &id("shop", "purchase", 1)).await;
    assert_eq!(landed.entity, accepted.entity);
    assert_eq!(
        read(
            &mut client,
            pb::EntityKind::Document,
            &id("shop", "source", 1),
            None
        )
        .await
        .entity,
        source.entity
    );
    assert!(landed.entry.unwrap().published);
    assert_eq!(
        landed.evidence[0].state,
        pb::GlossaryEvidenceState::Current as i32
    );
}

#[tokio::test]
async fn branch_negative_review_lands_with_unavailable_original_evidence() {
    let mut client = client().await;
    let target = id("shop", "purchase", 1);
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    let accepted = review(&mut client, &target, &draft.etag, false, None)
        .await
        .unwrap();
    delete(
        &mut client,
        pb::EntityKind::Document,
        &id("shop", "source", 1),
        &source.etag,
        pb::delete_entity_request::Mode::Force,
    )
    .await
    .unwrap();
    let baseline = detail(&mut client, &target).await;
    assert!(baseline.entry.as_ref().unwrap().published);
    assert!(baseline.incomplete);
    let branch = "glossary-retirement-branch";
    create_branch(&mut client, branch).await;
    let rejected = client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(target.clone()),
                }),
                if_match: accepted.etag,
                disposition: pb::GlossaryReviewDisposition::Rejected as i32,
                reason: "Retire the exact definition whose original source was removed.".into(),
                validate_only: false,
            },
            ADMIN,
            Some(branch),
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!rejected.entry.as_ref().unwrap().published);
    assert_eq!(
        rejected.entry.as_ref().unwrap().state,
        pb::GlossaryEntryState::Rejected as i32
    );
    assert_eq!(detail(&mut client, &target).await, baseline);
    let rejected_term = term(rejected.entity.as_ref().unwrap());
    let accepted_term = term(accepted.entity.as_ref().unwrap());
    assert_eq!(
        trogon_atlas_core::glossary::candidate_digest(rejected_term).unwrap(),
        trogon_atlas_core::glossary::candidate_digest(accepted_term).unwrap()
    );
    assert_eq!(
        rejected_term.glossary.as_ref().unwrap().evidence,
        accepted_term.glossary.as_ref().unwrap().evidence
    );
    let rejected_review = rejected_term
        .glossary
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap();
    let accepted_review = accepted_term
        .glossary
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap();
    assert_eq!(
        rejected_review.content_digest,
        accepted_review.content_digest
    );
    assert_eq!(rejected_review.owner_uid, accepted_review.owner_uid);
    assert_eq!(
        rejected
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        accepted
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
    );
    for dry_run in [true, false] {
        let merged = client
            .merge_branch(request(
                pb::MergeBranchRequest {
                    name: branch.into(),
                    dry_run,
                    keep_branch: true,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .expect("exact negative review must land despite unavailable original evidence")
            .into_inner();
        assert_eq!(
            merged.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated as i32
            } else {
                pb::merge_branch_response::Status::Applied as i32
            },
            "{merged:?}"
        );
        if dry_run {
            assert_eq!(detail(&mut client, &target).await, baseline);
        }
    }
    let landed = detail(&mut client, &target).await;
    assert_eq!(landed.entity, rejected.entity);
    assert!(!landed.entry.as_ref().unwrap().published);
    assert_eq!(
        landed.entry.unwrap().state,
        pb::GlossaryEntryState::Rejected as i32
    );
    assert_eq!(landed.evidence, rejected.evidence);
    assert!(landed.incomplete);
}

#[tokio::test]
async fn source_changes_deletion_and_recreation_do_not_rebind_published_evidence() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    let accepted = review(
        &mut client,
        &id("shop", "purchase", 1),
        &draft.etag,
        false,
        None,
    )
    .await
    .unwrap();
    let initial = detail(&mut client, &id("shop", "purchase", 1)).await;
    assert_eq!(
        initial.evidence[0].state,
        pb::GlossaryEvidenceState::Current as i32
    );
    let mut changed = source.entity.clone().unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed.kind else {
        panic!("document");
    };
    let text = "This source no longer contains the reviewed evidence.";
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: text.into(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(text)),
    }));
    let changed = put(&mut client, changed, &source.etag, None).await;
    let stale = detail(&mut client, &id("shop", "purchase", 1)).await;
    assert!(stale.entry.as_ref().unwrap().published);
    assert_eq!(
        stale.entry.unwrap().state,
        pb::GlossaryEntryState::SourceChanged as i32
    );
    assert_eq!(
        stale.evidence[0].state,
        pb::GlossaryEvidenceState::Changed as i32
    );
    delete(
        &mut client,
        pb::EntityKind::Document,
        &id("shop", "source", 1),
        &changed.etag,
        pb::delete_entity_request::Mode::Force,
    )
    .await
    .unwrap();
    let missing = detail(&mut client, &id("shop", "purchase", 1)).await;
    assert!(missing.incomplete);
    assert!(missing.entry.as_ref().unwrap().published);
    assert_eq!(
        missing.entry.unwrap().state,
        pb::GlossaryEntryState::SourceUnavailable as i32
    );
    assert_eq!(
        missing.evidence[0].state,
        pb::GlossaryEvidenceState::Unavailable as i32
    );
    let mut recreated = source.entity.unwrap();
    let original_uid = recreated.system.take().unwrap().uid;
    let Some(pb::entity::Kind::Document(document)) = &mut recreated.kind else {
        panic!("document");
    };
    for subject in &mut document.subjects {
        subject.connection_id = None;
    }
    let recreated = put(&mut client, recreated, "", None).await;
    assert_ne!(
        recreated
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        original_uid
    );
    let unresolved_pin = detail(&mut client, &id("shop", "purchase", 1)).await;
    assert!(!unresolved_pin.incomplete);
    assert_eq!(
        unresolved_pin.evidence[0].state,
        pb::GlossaryEvidenceState::Changed as i32
    );
    assert_eq!(unresolved_pin.entity, accepted.entity);
}

#[tokio::test]
async fn source_change_refuses_review_without_persisting_a_new_decision() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    let before = read(
        &mut client,
        pb::EntityKind::Term,
        &id("shop", "purchase", 1),
        None,
    )
    .await;
    let mut changed = source.entity.unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed.kind else {
        panic!("document");
    };
    let text = "Different source content at the same exact key.";
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: text.into(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(text)),
    }));
    put(&mut client, changed, &source.etag, None).await;
    assert_eq!(
        review(
            &mut client,
            &id("shop", "purchase", 1),
            &draft.etag,
            false,
            None
        )
        .await
        .unwrap_err()
        .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        read(
            &mut client,
            pb::EntityKind::Term,
            &id("shop", "purchase", 1),
            None
        )
        .await,
        before
    );
}

#[tokio::test]
async fn published_term_deletion_requires_explicit_retirement() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    let accepted = review(
        &mut client,
        &id("shop", "purchase", 1),
        &draft.etag,
        false,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        delete(
            &mut client,
            pb::EntityKind::Term,
            &id("shop", "purchase", 1),
            &accepted.etag,
            pb::delete_entity_request::Mode::Force
        )
        .await
        .unwrap_err()
        .code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        read(
            &mut client,
            pb::EntityKind::Term,
            &id("shop", "purchase", 1),
            None
        )
        .await
        .entity,
        accepted.entity
    );
}

#[tokio::test]
async fn glossary_native_input_limits_refuse_instead_of_claiming_empty_success() {
    let mut client = client().await;
    let source_ref = pb::DocumentRef {
        id: Some(id("shop", "source", 1)),
    };
    assert_eq!(
        client
            .inspect_glossary_sources(request(
                pb::InspectGlossarySourcesRequest {
                    documents: vec![source_ref; 33],
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        client
            .list_glossary_entries(request(
                pb::ListGlossaryEntriesRequest {
                    selection: Some(pb::GlossarySelection {
                        namespaces: vec!["shop".into(); 33],
                        ..selection("shop", true)
                    }),
                    ..Default::default()
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        client
            .list_glossary_entries(request(
                pb::ListGlossaryEntriesRequest {
                    selection: Some(selection("shop", true)),
                    page_size: 501,
                    ..Default::default()
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        client
            .check_glossary_usage(request(
                pb::CheckGlossaryUsageRequest {
                    selection: Some(selection("shop", false)),
                    text: "a".repeat(trogon_atlas_core::glossary::MAX_TEXT_BYTES + 1),
                    text_revision: "controlled-code-revision".into(),
                    on: Some(pb::CalendarDate {
                        value: "2026-11-12".into()
                    }),
                    ..Default::default()
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        client
            .check_glossary_usage(request(
                pb::CheckGlossaryUsageRequest {
                    selection: Some(selection("shop", false)),
                    text: "é".into(),
                    text_revision: "controlled-code-revision".into(),
                    on: Some(pb::CalendarDate {
                        value: "2026-11-12".into()
                    }),
                    exclusions: vec![pb::GlossaryTextRange { start: 1, end: 2 }],
                    ..Default::default()
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

#[tokio::test]
async fn source_output_at_the_byte_budget_succeeds_and_overflow_refuses() {
    let mut client = client().await;
    let seed = source(&mut client, "shop", "source", None).await;
    let content = "a".repeat(trogon_atlas_core::glossary::MAX_TEXT_BYTES);
    let mut documents = Vec::new();
    for index in 0..4 {
        let mut supplied = seed.entity.clone().unwrap();
        supplied.system = None;
        let Some(pb::entity::Kind::Document(document)) = &mut supplied.kind else {
            panic!("document required");
        };
        let target = id("shop", &format!("bounded-source-{index}"), 1);
        document.id = Some(target.clone());
        for subject in &mut document.subjects {
            subject.connection_id = None;
        }
        document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            content: content.clone(),
            media_type: Some(pb::MediaType {
                value: "text/plain".into(),
            }),
            digest: Some(trogon_atlas_core::specification::content_digest(&content)),
        }));
        put(&mut client, supplied, "", None).await;
        documents.push(pb::DocumentRef { id: Some(target) });
    }
    let complete = client
        .inspect_glossary_sources(request(
            pb::InspectGlossarySourcesRequest {
                documents: documents.clone(),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!complete.incomplete);
    assert_eq!(complete.sources.len(), 4);
    assert_eq!(
        complete
            .sources
            .iter()
            .map(|source| source.text.len())
            .sum::<usize>(),
        1_048_576
    );
    assert!(complete
        .sources
        .iter()
        .all(|source| source.state == pb::GlossaryEvidenceState::Current as i32));
    documents.push(pb::DocumentRef {
        id: Some(id("shop", "source", 1)),
    });
    assert_eq!(
        client
            .inspect_glossary_sources(request(
                pb::InspectGlossarySourcesRequest { documents },
                READER,
                None,
            ))
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
}

#[tokio::test]
async fn review_reply_budget_refuses_before_persisting_the_review() {
    use prost::Message;

    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let mut proposed = candidate(&source, "purchase", 1);
    term_mut(&mut proposed).metadata.push(prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.NoteAnnotation".into(),
        value: pb::NoteAnnotation {
            text: "\0".repeat(400_000),
            ..Default::default()
        }
        .encode_to_vec(),
    });
    let draft = put(&mut client, proposed, "", None).await;
    let before = read(
        &mut client,
        pb::EntityKind::Term,
        &id("shop", "purchase", 1),
        None,
    )
    .await;
    assert_eq!(
        review(
            &mut client,
            &id("shop", "purchase", 1),
            &draft.etag,
            false,
            None
        )
        .await
        .unwrap_err()
        .code(),
        Code::ResourceExhausted
    );
    assert_eq!(
        read(
            &mut client,
            pb::EntityKind::Term,
            &id("shop", "purchase", 1),
            None
        )
        .await,
        before
    );
    assert!(term(before.entity.as_ref().unwrap())
        .glossary
        .as_ref()
        .unwrap()
        .review
        .is_none());
}

#[tokio::test]
async fn comparison_charges_each_distinct_source_once_across_both_selections() {
    use prost::Message;

    let mut client = client().await;
    let text = format!("{SOURCE}\n{}", "a".repeat(230 * 1024 - SOURCE.len() - 1));
    let source_digest = trogon_atlas_core::specification::content_digest(&text);
    let request_byte_limit = 32 * 1024 * 1024;
    let mut examined_bytes = Vec::new();
    for namespace in ["glossary-left", "glossary-right"] {
        let supplied = source(&mut client, namespace, "source", None).await;
        let mut expanded = supplied.entity.unwrap();
        let Some(pb::entity::Kind::Document(document)) = &mut expanded.kind else {
            panic!("document required");
        };
        document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            content: text.clone(),
            media_type: Some(pb::MediaType {
                value: "text/plain".into(),
            }),
            digest: Some(source_digest.clone()),
        }));
        let supplied = put(&mut client, expanded, &supplied.etag, None).await;
        let catalog_bytes = 3 * 32 * supplied.entity.as_ref().unwrap().encoded_len();
        assert!(catalog_bytes < request_byte_limit);
        examined_bytes.push(catalog_bytes);
        for index in 0..3 {
            let slug = format!("word-{index}");
            let mut proposed = candidate(&supplied, &slug, 1);
            let glossary = term_mut(&mut proposed).glossary.as_mut().unwrap();
            let mut citation = glossary.evidence[0].clone();
            citation.content_digest = Some(source_digest.clone());
            glossary.evidence = (0..32)
                .map(|index| pb::GlossaryEvidence {
                    locator: format!("paragraph:{index}"),
                    ..citation.clone()
                })
                .collect();
            let draft = put(&mut client, proposed, "", None).await;
            let accepted = review(
                &mut client,
                &id(namespace, &slug, 1),
                &draft.etag,
                false,
                None,
            )
            .await
            .unwrap();
            assert!(accepted.entry.unwrap().published);
        }
        let independently_valid = client
            .list_glossary_entries(request(
                pb::ListGlossaryEntriesRequest {
                    selection: Some(selection(namespace, false)),
                    ..Default::default()
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(!independently_valid.incomplete);
        assert_eq!(independently_valid.entries.len(), 3);
    }
    assert!(examined_bytes.iter().sum::<usize>() > request_byte_limit);
    let compared = client
        .compare_glossaries(request(
            pb::CompareGlossariesRequest {
                left: Some(selection("glossary-left", false)),
                right: Some(selection("glossary-right", false)),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!compared.incomplete);
}

#[tokio::test]
async fn missing_replacement_preserves_the_prohibited_usage_finding() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let preferred_id = id("shop", "order", 1);
    let preferred = put(
        &mut client,
        entity(pb::entity::Kind::Term(pb::Term {
            id: Some(preferred_id.clone()),
            title: "Order".into(),
            doc: "A customer order.".into(),
            ..Default::default()
        })),
        "",
        None,
    )
    .await;
    let mut proposed = candidate(&source, "purchase", 1);
    term_mut(&mut proposed).title = "Purchase".into();
    term_mut(&mut proposed).glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Prohibited as i32,
        preferred_term: Some(pb::TermRef {
            id: Some(preferred_id.clone()),
        }),
        rationale: "Use Order in customer text.".into(),
        ..Default::default()
    });
    let draft = put(&mut client, proposed, "", None).await;
    review(
        &mut client,
        &id("shop", "purchase", 1),
        &draft.etag,
        false,
        None,
    )
    .await
    .unwrap();
    delete(
        &mut client,
        pb::EntityKind::Term,
        &preferred_id,
        &preferred.etag,
        pb::delete_entity_request::Mode::Force,
    )
    .await
    .unwrap();
    let checked = client
        .check_glossary_usage(request(
            pb::CheckGlossaryUsageRequest {
                selection: Some(selection("shop", false)),
                text: "Complete your purchase.".into(),
                text_revision: "customer-copy-v1".into(),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(checked.incomplete);
    assert_eq!(
        checked.findings.len(),
        1,
        "a missing replacement must not hide prohibited usage"
    );
    assert_eq!(checked.findings[0].matched, "purchase");
    assert_eq!(
        checked.findings[0].usage,
        pb::GlossaryUsage::Prohibited as i32
    );
    assert!(checked.findings[0].preferred_term.is_none());
}

#[tokio::test]
async fn bulk_delete_requires_glossary_retirement_in_preview_and_execution() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let draft = put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    put(&mut client, candidate(&source, "purchase", 2), "", None).await;
    let target = id("shop", "purchase", 1);
    let sibling = id("shop", "purchase", 2);
    review(&mut client, &target, &draft.etag, false, None)
        .await
        .unwrap();
    let before = read(&mut client, pb::EntityKind::Term, &target, None).await;
    let sibling_before = read(&mut client, pb::EntityKind::Term, &sibling, None).await;
    let query = pb::DeleteByQueryRequest {
        kind: pb::EntityKind::Term as i32,
        namespace: "shop".into(),
        slug: "purchase".into(),
        max_deletes: 2,
        ..Default::default()
    };
    for mode in [
        pb::delete_entity_request::Mode::DryRun,
        pb::delete_entity_request::Mode::Force,
    ] {
        let result = client
            .delete_by_query(request(
                pb::DeleteByQueryRequest {
                    mode: mode as i32,
                    ..query.clone()
                },
                ADMIN,
                None,
            ))
            .await;
        assert_eq!(result.unwrap_err().code(), Code::FailedPrecondition);
        assert_eq!(
            read(&mut client, pb::EntityKind::Term, &target, None).await,
            before
        );
        assert_eq!(
            read(&mut client, pb::EntityKind::Term, &sibling, None).await,
            sibling_before
        );
    }
    client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(target.clone()),
                }),
                if_match: before.etag,
                disposition: pb::GlossaryReviewDisposition::Rejected as i32,
                reason: "Retire the published version before cleanup.".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let retired = read(&mut client, pb::EntityKind::Term, &target, None).await;
    let preview = client
        .delete_by_query(request(
            pb::DeleteByQueryRequest {
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                ..query.clone()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(preview.dry_run);
    assert_eq!(preview.deleted_count, 2);
    assert_eq!(
        read(&mut client, pb::EntityKind::Term, &target, None).await,
        retired
    );
    assert_eq!(
        read(&mut client, pb::EntityKind::Term, &sibling, None).await,
        sibling_before
    );
    let deleted = client
        .delete_by_query(request(
            pb::DeleteByQueryRequest {
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..query
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!deleted.dry_run);
    assert_eq!(deleted.deleted_count, 2);
    for target in [target, sibling] {
        assert_eq!(
            client
                .get_entity(request(
                    pb::GetEntityRequest {
                        kind: pb::EntityKind::Term as i32,
                        id: Some(target),
                    },
                    ADMIN,
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
    }
}

#[tokio::test]
async fn duplicate_evidence_is_fetched_once_per_request() {
    use std::sync::atomic::Ordering;
    let store = Arc::new(observed_glossary_store::ObservedGlossaryStore::new(
        trogon_atlas_testsupport::shared().await.store().await,
    ));
    let mut client = client_with_store(store.clone()).await;
    let source = source(&mut client, "shop", "source", None).await;
    let mut proposed = candidate(&source, "purchase", 1);
    let glossary = term_mut(&mut proposed).glossary.as_mut().unwrap();
    glossary.evidence = (0..32)
        .map(|index| pb::GlossaryEvidence {
            locator: format!("paragraph:{index}"),
            ..glossary.evidence[0].clone()
        })
        .collect();
    put(&mut client, proposed, "", None).await;
    store.source_reads.store(0, Ordering::SeqCst);
    let listed = client
        .list_glossary_entries(request(
            pb::ListGlossaryEntriesRequest {
                selection: Some(selection("shop", true)),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!listed.incomplete);
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(store.source_reads.load(Ordering::SeqCst), 1);
}

async fn concurrent_glossary_read(branch: Option<&str>, recreate: bool) {
    use std::sync::atomic::Ordering;
    let store = Arc::new(observed_glossary_store::ObservedGlossaryStore::new(
        trogon_atlas_testsupport::shared().await.store().await,
    ));
    let mut client = client_with_store(store.clone()).await;
    let source = source(&mut client, "shop", "source", None).await;
    put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    if let Some(branch) = branch {
        create_branch(&mut client, branch).await;
    }
    store.pause_source.store(true, Ordering::SeqCst);
    let mut reader = client.clone();
    let read_branch = branch.map(str::to_owned);
    let read = tokio::spawn(async move {
        reader
            .list_glossary_entries(request(
                pb::ListGlossaryEntriesRequest {
                    selection: Some(selection("shop", true)),
                    ..Default::default()
                },
                READER,
                read_branch.as_deref(),
            ))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), store.reached.notified())
        .await
        .unwrap();
    let write_branch = branch.map(str::to_owned);
    let mut write = tokio::spawn(async move {
        if let Some(branch) = write_branch {
            client
                .delete_branch(request(
                    pb::DeleteBranchRequest {
                        name: branch.clone(),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap();
            if recreate {
                create_branch(&mut client, &branch).await;
            }
        } else {
            put(
                &mut client,
                entity(pb::entity::Kind::Event(pb::Event {
                    id: Some(id("shop", "concurrent-event", 1)),
                    title: "Concurrent write".into(),
                    ..Default::default()
                })),
                "",
                None,
            )
            .await;
        }
    });
    let progressed = tokio::time::timeout(std::time::Duration::from_secs(5), &mut write).await;
    store.release.notify_one();
    let write_completed = if let Ok(result) = progressed {
        result.unwrap();
        true
    } else {
        write.await.unwrap();
        false
    };
    let result = read.await.unwrap();
    assert!(
        write_completed,
        "a blocked evidence read must not hold the global mutation lock"
    );
    assert_eq!(
        result.unwrap_err().code(),
        if branch.is_some() && !recreate {
            Code::NotFound
        } else {
            Code::Aborted
        }
    );
}

#[tokio::test]
async fn glossary_read_allows_writes_and_refuses_a_mixed_baseline() {
    concurrent_glossary_read(None, false).await;
}

#[tokio::test]
async fn glossary_read_refuses_a_deleted_branch_instead_of_baseline_fallback() {
    concurrent_glossary_read(Some("glossary-concurrent-delete"), false).await;
}

#[tokio::test]
async fn glossary_read_refuses_a_recreated_branch() {
    concurrent_glossary_read(Some("glossary-concurrent-recreate"), true).await;
}

#[tokio::test]
async fn retarget_draft_evidence_binds_the_exact_target_document() {
    let mut client = client().await;
    let from = source(&mut client, "shop", "source-one", None).await;
    let to = source(&mut client, "shop", "source-two", None).await;
    let target_text = format!("{SOURCE} Additional retained context.");
    let mut changed = to.entity.clone().unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed.kind else {
        panic!("document required");
    };
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: target_text.clone(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(
            &target_text,
        )),
    }));
    let to = put(&mut client, changed, &to.etag, None).await;
    let target = id("shop", "purchase", 1);
    put(&mut client, candidate(&from, "purchase", 1), "", None).await;
    let from_ref = pb::EntityRef {
        kind: pb::EntityKind::Document as i32,
        id: Some(id("shop", "source-one", 1)),
    };
    let to_ref = pb::EntityRef {
        kind: pb::EntityKind::Document as i32,
        id: Some(id("shop", "source-two", 1)),
    };
    for dry_run in [true, false] {
        let result = client
            .retarget_references(request(
                pb::RetargetReferencesRequest {
                    from: Some(from_ref.clone()),
                    to: Some(to_ref.clone()),
                    dry_run,
                    filter_kinds: vec![pb::EntityKind::Term as i32],
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(result.rewritten, 1);
        assert_eq!(result.failed, 0);
        let current = detail(&mut client, &target).await;
        assert_eq!(
            current.evidence[0].state,
            pb::GlossaryEvidenceState::Current as i32
        );
        let evidence = &term(current.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .evidence[0];
        assert_eq!(
            evidence.owner_uid,
            if dry_run {
                from.entity.as_ref().unwrap()
            } else {
                to.entity.as_ref().unwrap()
            }
            .system
            .as_ref()
            .unwrap()
            .uid
        );
        assert_eq!(
            evidence.document.as_ref().unwrap().id.as_ref(),
            if dry_run {
                from_ref.id.as_ref()
            } else {
                to_ref.id.as_ref()
            }
        );
    }
    let after = read(&mut client, pb::EntityKind::Term, &target, None).await;
    review(&mut client, &target, &after.etag, false, None)
        .await
        .unwrap();
    let accepted = read(&mut client, pb::EntityKind::Term, &target, None).await;
    let result = client
        .retarget_references(request(
            pb::RetargetReferencesRequest {
                from: Some(to_ref.clone()),
                to: Some(from_ref.clone()),
                dry_run: false,
                filter_kinds: vec![pb::EntityKind::Term as i32],
            },
            ADMIN,
            None,
        ))
        .await;
    assert_eq!(result.unwrap_err().code(), Code::FailedPrecondition);
    assert_eq!(
        read(&mut client, pb::EntityKind::Term, &target, None).await,
        accepted
    );
    let negative = client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(target.clone()),
                }),
                if_match: accepted.etag,
                disposition: pb::GlossaryReviewDisposition::NeedsChanges as i32,
                reason: "Revise the original evidence selection.".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    client
        .retarget_references(request(
            pb::RetargetReferencesRequest {
                from: Some(to_ref),
                to: Some(from_ref),
                dry_run: false,
                filter_kinds: vec![pb::EntityKind::Term as i32],
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let corrected = detail(&mut client, &target).await;
    assert_eq!(
        corrected.entry.unwrap().state,
        pb::GlossaryEntryState::StaleReview as i32
    );
    assert_eq!(
        term(corrected.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .review,
        term(negative.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .review,
    );
}

#[tokio::test]
async fn comparison_keeps_one_byte_budget_for_distinct_source_documents() {
    use prost::Message;
    let mut client = client().await;
    let mut total_bytes = 0;
    for namespace in ["distinct-left", "distinct-right"] {
        let seed = source(&mut client, namespace, "source", None).await;
        let mut proposed = candidate(&seed, "purchase", 1);
        term_mut(&mut proposed)
            .glossary
            .as_mut()
            .unwrap()
            .evidence
            .clear();
        let mut side_bytes = 0;
        for index in 0..18 {
            let mut document = seed.entity.clone().unwrap();
            document.system = None;
            let Some(pb::entity::Kind::Document(value)) = &mut document.kind else {
                panic!("document required")
            };
            value.id = Some(id(namespace, &format!("source-{index}"), 1));
            for subject in &mut value.subjects {
                subject.connection_id = None;
            }
            value.metadata.push(prost_types::Any {
                type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.NoteAnnotation"
                    .into(),
                value: pb::NoteAnnotation {
                    text: "a".repeat(960 * 1024),
                    ..Default::default()
                }
                .encode_to_vec(),
            });
            let source = put(&mut client, document, "", None).await;
            side_bytes += source.entity.as_ref().unwrap().encoded_len();
            let citation = term(&candidate(&source, "unused", 1))
                .glossary
                .as_ref()
                .unwrap()
                .evidence[0]
                .clone();
            term_mut(&mut proposed)
                .glossary
                .as_mut()
                .unwrap()
                .evidence
                .push(citation);
        }
        assert!(side_bytes < 32 * 1024 * 1024);
        total_bytes += side_bytes;
        let draft = put(&mut client, proposed, "", None).await;
        review(
            &mut client,
            &id(namespace, "purchase", 1),
            &draft.etag,
            false,
            None,
        )
        .await
        .unwrap();
        let listed = client
            .list_glossary_entries(request(
                pb::ListGlossaryEntriesRequest {
                    selection: Some(selection(namespace, false)),
                    ..Default::default()
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(!listed.incomplete);
        assert_eq!(listed.entries.len(), 1);
    }
    assert!(total_bytes > 32 * 1024 * 1024);
    let result = client
        .compare_glossaries(request(
            pb::CompareGlossariesRequest {
                left: Some(selection("distinct-left", false)),
                right: Some(selection("distinct-right", false)),
            },
            READER,
            None,
        ))
        .await;
    assert_eq!(result.unwrap_err().code(), Code::ResourceExhausted);
}

#[tokio::test]
async fn glossary_read_refuses_changed_namespace_authority() {
    use std::sync::atomic::Ordering;
    let store = Arc::new(observed_glossary_store::ObservedGlossaryStore::new(
        trogon_atlas_testsupport::shared().await.store().await,
    ));
    let mut client = client_with_store(store.clone()).await;
    let namespace = client
        .register_namespace(request(
            pb::RegisterNamespaceRequest {
                name: "glossary-authority".into(),
                parent: "visible".into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let source = source(&mut client, &namespace, "source", None).await;
    put(&mut client, candidate(&source, "purchase", 1), "", None).await;
    store.pause_source.store(true, Ordering::SeqCst);
    let mut reader = client.clone();
    let read_namespace = namespace.clone();
    let read = tokio::spawn(async move {
        reader
            .list_glossary_entries(request(
                pb::ListGlossaryEntriesRequest {
                    selection: Some(selection(&read_namespace, true)),
                    ..Default::default()
                },
                LIMITED,
                None,
            ))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), store.reached.notified())
        .await
        .unwrap();
    let mut write = tokio::spawn(async move {
        client
            .move_namespace(request(
                pb::MoveNamespaceRequest {
                    id: namespace,
                    parent: "hidden".into(),
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap();
    });
    let progressed = tokio::time::timeout(std::time::Duration::from_secs(5), &mut write).await;
    store.release.notify_one();
    let write_completed = if let Ok(result) = progressed {
        result.unwrap();
        true
    } else {
        write.await.unwrap();
        false
    };
    let result = read.await.unwrap();
    assert!(
        write_completed,
        "namespace changes must not wait for an evidence scan"
    );
    assert_eq!(result.unwrap_err().code(), Code::Aborted);
}

#[tokio::test]
async fn duplicate_batch_glossary_puts_cannot_hide_a_final_replacement_cycle() {
    for branch in [None, Some("glossary-final-cycle")] {
        let mut client = client().await;
        let source = source(&mut client, "shop", "source", None).await;
        if let Some(branch) = branch {
            create_branch(&mut client, branch).await;
        }
        let first = candidate(&source, "first", 1);
        let second = candidate(&source, "second", 1);
        let mut first_cycle = first.clone();
        let mut second_cycle = second.clone();
        for (entity, target) in [
            (&mut first_cycle, id("shop", "second", 1)),
            (&mut second_cycle, id("shop", "first", 1)),
        ] {
            term_mut(entity).glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
                usage: pb::GlossaryUsage::Deprecated as i32,
                preferred_term: Some(pb::TermRef { id: Some(target) }),
                rationale: "Use the final preferred definition.".into(),
                ..Default::default()
            });
        }
        let ops = [first, second, first_cycle, second_cycle]
            .into_iter()
            .map(|entity| pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: Some(entity),
                    ..Default::default()
                })),
            })
            .collect::<Vec<_>>();
        for validate_only in [true, false] {
            let error = client
                .batch_mutate(request(
                    pb::BatchMutateRequest {
                        ops: ops.clone(),
                        validate_only,
                        ..Default::default()
                    },
                    ADMIN,
                    branch,
                ))
                .await
                .unwrap_err();
            assert_eq!(error.code(), Code::InvalidArgument);
            assert!(error.message().contains("glossary preferred-term cycle"));
            for slug in ["first", "second"] {
                assert_eq!(
                    client
                        .get_entity(request(
                            pb::GetEntityRequest {
                                kind: pb::EntityKind::Term as i32,
                                id: Some(id("shop", slug, 1)),
                            },
                            ADMIN,
                            branch,
                        ))
                        .await
                        .unwrap_err()
                        .code(),
                    Code::NotFound
                );
            }
        }
    }
}

#[tokio::test]
async fn duplicate_batch_glossary_targets_use_the_final_acyclic_put() {
    for branch in [None, Some("glossary-final-acyclic")] {
        let mut client = client().await;
        let source = source(&mut client, "shop", "source", None).await;
        if let Some(branch) = branch {
            create_branch(&mut client, branch).await;
        }
        let first = candidate(&source, "first", 1);
        let second = candidate(&source, "second", 1);
        let mut first_cycle = first.clone();
        let mut second_cycle = second.clone();
        for (entity, target) in [
            (&mut first_cycle, id("shop", "second", 1)),
            (&mut second_cycle, id("shop", "first", 1)),
        ] {
            term_mut(entity).glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
                usage: pb::GlossaryUsage::Deprecated as i32,
                preferred_term: Some(pb::TermRef { id: Some(target) }),
                rationale: "Use the final preferred definition.".into(),
                ..Default::default()
            });
        }
        let ops = [first_cycle, second_cycle, first, second]
            .into_iter()
            .map(|entity| pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: Some(entity),
                    ..Default::default()
                })),
            })
            .collect::<Vec<_>>();
        for validate_only in [true, false] {
            let response = client
                .batch_mutate(request(
                    pb::BatchMutateRequest {
                        ops: ops.clone(),
                        validate_only,
                        ..Default::default()
                    },
                    ADMIN,
                    branch,
                ))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(
                response.status,
                if validate_only {
                    pb::batch_mutate_response::Status::Validated as i32
                } else {
                    pb::batch_mutate_response::Status::Applied as i32
                }
            );
        }
        for slug in ["first", "second"] {
            let stored = read(
                &mut client,
                pb::EntityKind::Term,
                &id("shop", slug, 1),
                branch,
            )
            .await;
            assert!(term(stored.entity.as_ref().unwrap())
                .glossary
                .as_ref()
                .unwrap()
                .policy
                .is_none());
        }
    }
}

#[tokio::test]
async fn duplicate_batch_glossary_target_deleted_last_is_unavailable() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source", None).await;
    let replacement = candidate(&source, "replacement", 1);
    let mut proposed = candidate(&source, "obsolete", 1);
    term_mut(&mut proposed).glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Deprecated as i32,
        preferred_term: Some(pb::TermRef {
            id: Some(id("shop", "replacement", 1)),
        }),
        rationale: "Use the final preferred definition.".into(),
        ..Default::default()
    });
    let mut ops = [replacement, proposed]
        .into_iter()
        .map(|entity| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            })),
        })
        .collect::<Vec<_>>();
    ops.push(pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Term as i32,
            id: Some(id("shop", "replacement", 1)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        })),
    });
    for validate_only in [true, false] {
        let error = client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    ops: ops.clone(),
                    validate_only,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(error.message().contains("glossary target is unavailable"));
        for slug in ["replacement", "obsolete"] {
            assert_eq!(
                client
                    .get_entity(request(
                        pb::GetEntityRequest {
                            kind: pb::EntityKind::Term as i32,
                            id: Some(id("shop", slug, 1)),
                        },
                        ADMIN,
                        None,
                    ))
                    .await
                    .unwrap_err()
                    .code(),
                Code::NotFound
            );
        }
    }
}

#[tokio::test]
async fn cached_source_provenance_keeps_repeat_revisions_and_tracks_metadata_changes() {
    use std::sync::atomic::Ordering;

    let store = Arc::new(observed_glossary_store::ObservedGlossaryStore::new(
        trogon_atlas_testsupport::shared().await.store().await,
    ));
    let mut client = client_with_store(store.clone()).await;
    let source = source(&mut client, "shop", "source", None).await;
    let content = format!("{SOURCE}\n{}", "a".repeat(230 * 1024));
    let mut expanded = source.entity.unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut expanded.kind else {
        panic!("document required");
    };
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: content.clone(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(&content)),
    }));
    let source = put(&mut client, expanded, &source.etag, None).await;
    for slug in ["alpha", "beta"] {
        let mut proposed = candidate(&source, slug, 1);
        let glossary = term_mut(&mut proposed).glossary.as_mut().unwrap();
        let mut citation = glossary.evidence[0].clone();
        citation.content_digest = Some(trogon_atlas_core::specification::content_digest(&content));
        glossary.evidence = (0..32)
            .map(|index| pb::GlossaryEvidence {
                locator: format!("paragraph:{index}"),
                ..citation.clone()
            })
            .collect();
        put(&mut client, proposed, "", None).await;
    }
    store.source_reads.store(0, Ordering::SeqCst);
    let first = list(&mut client, selection("shop", true), "", READER, None)
        .await
        .unwrap();
    assert!(!first.incomplete);
    assert_ne!(first.next_page_token, "");
    assert_eq!(store.source_reads.load(Ordering::SeqCst), 1);
    let repeated = list(&mut client, selection("shop", true), "", READER, None)
        .await
        .unwrap();
    assert_eq!(repeated, first);
    assert_eq!(store.source_reads.load(Ordering::SeqCst), 2);
    let exact = pb::GetGlossaryTermRequest {
        term: Some(pb::TermRef {
            id: Some(id("shop", "alpha", 1)),
        }),
    };
    let before = client
        .get_glossary_definition(request(exact.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    let mut changed = source.entity.unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed.kind else {
        panic!("document required");
    };
    document.title.as_mut().unwrap().value = "Updated source metadata".into();
    put(&mut client, changed, &source.etag, None).await;
    let after = list(&mut client, selection("shop", true), "", READER, None)
        .await
        .unwrap();
    assert!(!after.incomplete);
    assert_ne!(after.snapshot_revision, first.snapshot_revision);
    assert_eq!(
        list(
            &mut client,
            selection("shop", true),
            &first.next_page_token,
            READER,
            None,
        )
        .await
        .unwrap_err()
        .code(),
        Code::FailedPrecondition
    );
    let after = client
        .get_glossary_definition(request(exact, READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(before.definition, after.definition);
    assert_eq!(before.etag, after.etag);
    assert_ne!(before.snapshot_revision, after.snapshot_revision);
    assert!(after
        .evidence
        .iter()
        .all(|item| item.state == pb::GlossaryEvidenceState::Current as i32));
}

async fn glossary_refuses_a_commit_from_another_service(
    role: trogon_atlas_core::WriterRole,
    branch: Option<&str>,
) {
    use std::sync::atomic::Ordering;

    use trogon_atlas_store::Store as _;

    let config = trogon_atlas_testsupport::shared().await.store_config();
    let writer_store = trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .unwrap();
    assert!(writer_store.is_writer());
    let mut reader_config = config;
    reader_config.role = role;
    let reader_store = trogon_atlas_store::NatsStore::connect_with(reader_config)
        .await
        .unwrap();
    assert!(!reader_store.is_writer());
    let observed = Arc::new(observed_glossary_store::ObservedGlossaryStore::new(
        reader_store,
    ));
    let mut writer = client_with_store(Arc::new(writer_store)).await;
    let mut reader = client_with_store(observed.clone()).await;
    let source = source(&mut writer, "shop", "source", None).await;
    let original = put(&mut writer, candidate(&source, "purchase", 1), "", None).await;
    if let Some(branch) = branch {
        create_branch(&mut writer, branch).await;
    }
    observed.pause_source.store(true, Ordering::SeqCst);
    let read_branch = branch.map(str::to_owned);
    let read = tokio::spawn(async move {
        reader
            .get_glossary_term(request(
                pb::GetGlossaryTermRequest {
                    term: Some(pb::TermRef {
                        id: Some(id("shop", "purchase", 1)),
                    }),
                },
                READER,
                read_branch.as_deref(),
            ))
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        observed.reached.notified(),
    )
    .await
    .unwrap();
    let content = format!("{SOURCE} The source has a new accepted context.");
    let mut changed_source = source.entity.unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed_source.kind else {
        panic!("document required");
    };
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: content.clone(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(&content)),
    }));
    let mut changed_term = original.entity.unwrap();
    let term = term_mut(&mut changed_term);
    term.doc = "A customer request under the newly retained context.".into();
    term.glossary.as_mut().unwrap().evidence[0].content_digest =
        Some(trogon_atlas_core::specification::content_digest(&content));
    let result = writer
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: [(changed_source, source.etag), (changed_term, original.etag)]
                    .into_iter()
                    .map(|(entity, etag)| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(entity),
                            if_match: etag,
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            branch,
        ))
        .await;
    observed.release.notify_one();
    assert_eq!(
        result.unwrap().into_inner().status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    assert_eq!(read.await.unwrap().unwrap_err().code(), Code::Aborted);
}

#[tokio::test]
async fn glossary_reader_refuses_a_mixed_commit_from_another_service() {
    glossary_refuses_a_commit_from_another_service(trogon_atlas_core::WriterRole::Reader, None)
        .await;
}

#[tokio::test]
async fn glossary_standby_refuses_a_mixed_commit_from_another_service() {
    glossary_refuses_a_commit_from_another_service(trogon_atlas_core::WriterRole::Standby, None)
        .await;
}

#[tokio::test]
async fn glossary_reader_refuses_a_mixed_branch_commit_from_another_service() {
    glossary_refuses_a_commit_from_another_service(
        trogon_atlas_core::WriterRole::Reader,
        Some("glossary-cross-service"),
    )
    .await;
}

#[tokio::test]
async fn glossary_reader_refreshes_cached_ownership_after_a_remote_move() {
    let config = trogon_atlas_testsupport::shared().await.store_config();
    let writer_store = trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .unwrap();
    let mut reader_config = config;
    reader_config.role = trogon_atlas_core::WriterRole::Reader;
    let reader_store = trogon_atlas_store::NatsStore::connect_with(reader_config)
        .await
        .unwrap();
    let mut writer = client_with_store(Arc::new(writer_store)).await;
    let directory = Arc::new(
        trogon_atlas_server::ownership::NamespaceDirectory::with_freshness(
            trogon_atlas_server::token_reload::ReloadInterval::from_secs(0),
        ),
    );
    let mut reader = client_with_store_and_directory(Arc::new(reader_store), directory).await;
    let source = source(&mut writer, "shop", "source", None).await;
    put(&mut writer, candidate(&source, "purchase", 1), "", None).await;
    writer
        .move_namespace(request(
            pb::MoveNamespaceRequest {
                id: "shop".into(),
                parent: "visible".into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let query = pb::GetGlossaryTermRequest {
        term: Some(pb::TermRef {
            id: Some(id("shop", "purchase", 1)),
        }),
    };
    reader
        .get_glossary_term(request(query.clone(), LIMITED, None))
        .await
        .unwrap();
    writer
        .move_namespace(request(
            pb::MoveNamespaceRequest {
                id: "shop".into(),
                parent: "hidden".into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        reader
            .get_glossary_term(request(query, LIMITED, None))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}
