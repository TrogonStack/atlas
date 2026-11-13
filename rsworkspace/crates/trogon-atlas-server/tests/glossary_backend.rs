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
const SOURCE: &str = "A purchase is a customer request accepted by the store.";

fn request<T>(body: T, identity: &str) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        format!("Bearer {identity}").parse().unwrap(),
    );
    req
}

async fn client() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(
            r#"
[principals.expert]
role = "admin"
tokens = ["glossary-fixture-expert"]
[principals.reader]
role = "reader"
tokens = ["glossary-fixture-reader"]
[principals.worker]
role = "writer"
kind = "agent"
parent = "expert"
tokens = ["glossary-fixture-worker"]
"#,
        )
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

fn id(slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: "shop".into(),
        slug: slug.into(),
        version,
    }
}

async fn put(client: &mut Client, entity: pb::Entity, etag: &str) -> pb::PutEntityResponse {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: etag.into(),
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn candidate(client: &mut Client, version: u64) -> pb::PutEntityResponse {
    put(
        client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id("purchase-recorded", 1)),
                title: "Purchase recorded".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
        "",
    )
    .await;
    let existing_source = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("source", 1)),
            },
            "glossary-fixture-expert",
        ))
        .await;
    let source = if let Ok(existing) = existing_source {
        let source = existing.into_inner();
        pb::PutEntityResponse {
            entity: source.entity,
            etag: source.etag,
            ..Default::default()
        }
    } else {
        put(
            client,
            pb::Entity {
                kind: Some(pb::entity::Kind::Document(pb::Document {
                    id: Some(id("source", 1)),
                    title: Some(pb::CatalogTitle {
                        value: "Fixture source".into(),
                    }),
                    subjects: vec![pb::SubjectAssociation {
                        subject: Some(pb::CatalogSubject {
                            selection: Some(pb::catalog_subject::Selection::Entity(
                                pb::EntityRef {
                                    kind: pb::EntityKind::Event as i32,
                                    id: Some(id("purchase-recorded", 1)),
                                },
                            )),
                        }),
                        ..Default::default()
                    }],
                    body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                        content: SOURCE.into(),
                        media_type: Some(pb::MediaType {
                            value: "text/plain".into(),
                        }),
                        digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
                        ..Default::default()
                    })),
                    ..Default::default()
                })),
                ..Default::default()
            },
            "",
        )
        .await
    };
    let source_uid = source.entity.unwrap().system.unwrap().uid;
    let entity = pb::Entity {
        kind: Some(pb::entity::Kind::Term(pb::Term {
            id: Some(id("purchase", version)),
            title: "Purchase".into(),
            doc: "A customer request accepted by the store.".into(),
            glossary: Some(pb::GlossaryEntry {
                generation_revision: "fixture/prompt-v1".into(),
                audience: "customer".into(),
                evidence: vec![pb::GlossaryEvidence {
                    document: Some(pb::DocumentRef {
                        id: Some(id("source", 1)),
                    }),
                    owner_uid: source_uid,
                    content_digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
                    locator: "paragraph:1".into(),
                    quote: "a customer request accepted by the store".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    put(client, entity, "").await
}

fn selection(unreviewed: bool) -> pb::GlossarySelection {
    pb::GlossarySelection {
        namespaces: vec!["shop".into()],
        audience: "customer".into(),
        include_unreviewed: unreviewed,
    }
}

#[tokio::test]
async fn reviewed_version_remains_published_while_a_newer_candidate_is_draft() {
    let mut client = client().await;
    let draft = candidate(&mut client, 1).await;
    let review = client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("purchase", 1)),
                }),
                if_match: draft.etag,
                disposition: pb::GlossaryReviewDisposition::Accepted as i32,
                reason: "The fixture expert corrected and accepted this contextual definition."
                    .into(),
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(review.entry.unwrap().published);
    candidate(&mut client, 2).await;
    let listed = client
        .list_glossary_entries(request(
            pb::ListGlossaryEntriesRequest {
                selection: Some(selection(false)),
                ..Default::default()
            },
            "glossary-fixture-reader",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!listed.incomplete);
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(
        listed.entries[0]
            .term
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        1
    );
    assert_eq!(
        listed.entries[0].state,
        pb::GlossaryEntryState::Reviewed as i32
    );
    let drafts = client
        .list_glossary_entries(request(
            pb::ListGlossaryEntriesRequest {
                selection: Some(selection(true)),
                ..Default::default()
            },
            "glossary-fixture-reader",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(drafts.entries.len(), 2);
}

#[tokio::test]
async fn ordinary_native_writes_cannot_forge_an_expert_review() {
    let mut client = client().await;
    let draft = candidate(&mut client, 1).await;
    let mut entity = draft.entity.unwrap();
    let uid = entity.system.as_ref().unwrap().uid.clone();
    let Some(pb::entity::Kind::Term(term)) = &mut entity.kind else {
        panic!("term")
    };
    let digest = trogon_atlas_core::glossary::candidate_digest(term).unwrap();
    term.glossary.as_mut().unwrap().review = Some(pb::GlossaryReview {
        disposition: pb::GlossaryReviewDisposition::Accepted as i32,
        actor: "expert".into(),
        reviewed_at: "2026-11-12T00:00:00Z".into(),
        owner_uid: uid,
        reason: "forged fixture".into(),
        content_digest: Some(digest),
    });
    let result = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: draft.etag,
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await;
    assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
}

#[tokio::test]
async fn agent_credentials_cannot_record_user_review() {
    let mut client = client().await;
    let result = client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("purchase", 1)),
                }),
                if_match: "not-current".into(),
                disposition: pb::GlossaryReviewDisposition::Accepted as i32,
                reason: "agent cannot assert expert identity".into(),
                ..Default::default()
            },
            "glossary-fixture-worker",
        ))
        .await;
    assert_eq!(result.unwrap_err().code(), Code::PermissionDenied);
}

#[tokio::test]
async fn generated_definition_requires_the_exact_retained_source_incarnation() {
    let mut client = client().await;
    let draft = candidate(&mut client, 1).await;
    let mut entity = draft.entity.unwrap();
    let Some(pb::entity::Kind::Term(term)) = &mut entity.kind else {
        panic!("term")
    };
    term.glossary.as_mut().unwrap().evidence[0].owner_uid =
        "019a0000-0000-7000-8000-000000000099".into();
    let result = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: draft.etag,
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await;
    assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
}
