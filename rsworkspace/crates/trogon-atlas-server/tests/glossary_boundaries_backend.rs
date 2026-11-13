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
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
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
