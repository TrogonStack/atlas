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
    fixture_client().await.0
}

async fn fixture_client() -> (Client, String) {
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
    let endpoint = format!("http://{address}");
    (Client::connect(endpoint.clone()).await.unwrap(), endpoint)
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
            embodied_by: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("purchase-recorded", 1)),
            }],
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

#[tokio::test]
#[ignore = "authenticated native glossary browser fixture, bounded to ten minutes"]
async fn glossary_native_fixture() {
    let (mut client, endpoint) = fixture_client().await;
    put(
        &mut client,
        pb::Entity {
            kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                id: Some(id("shop", 1)),
                title: "Shop".into(),
                doc: "Controlled glossary acceptance context.".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
        "",
    )
    .await;
    let candidate = candidate(&mut client, 1).await;
    let mut with_period = candidate.entity.clone().unwrap();
    let Some(pb::entity::Kind::Term(term)) = &mut with_period.kind else {
        panic!("fixture term");
    };
    term.glossary.as_mut().unwrap().usage_period = Some(pb::GlossaryUsagePeriod {
        valid_from: Some(pb::CalendarDate {
            value: "2026-11-01".into(),
        }),
        valid_until: Some(pb::CalendarDate {
            value: "2026-12-01".into(),
        }),
    });
    let candidate = put(&mut client, with_period, &candidate.etag).await;
    let published = client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("purchase", 1)),
                }),
                if_match: candidate.etag,
                disposition: pb::GlossaryReviewDisposition::Accepted as i32,
                reason: "Controlled expert fixture review.".into(),
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner();
    let mut draft = published.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Term(term)) = &mut draft.kind {
        term.id = Some(id("purchase", 2));
        term.doc = "Proposed definition awaiting expert correction.".into();
        term.glossary.as_mut().unwrap().review = None;
    }
    draft.system = None;
    put(&mut client, draft, "").await;
    let source = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("source", 1)),
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner();
    let mut stale_source = source.entity.unwrap();
    stale_source.system = None;
    if let Some(pb::entity::Kind::Document(document)) = &mut stale_source.kind {
        document.id = Some(id("stale-source", 1));
        for subject in &mut document.subjects {
            subject.connection_id = None;
        }
    }
    let stale_source = put(&mut client, stale_source, "").await;
    let mut stale = published.entity.unwrap();
    stale.system = None;
    if let Some(pb::entity::Kind::Term(term)) = &mut stale.kind {
        term.id = Some(id("stale", 1));
        term.title = "Archived vocabulary".into();
        term.glossary.as_mut().unwrap().review = None;
        let evidence = &mut term.glossary.as_mut().unwrap().evidence[0];
        evidence.document = Some(pb::DocumentRef {
            id: Some(id("stale-source", 1)),
        });
        evidence.owner_uid = stale_source
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
            .clone();
    }
    let stale = put(&mut client, stale, "").await;
    client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("stale", 1)),
                }),
                if_match: stale.etag,
                disposition: pb::GlossaryReviewDisposition::Accepted as i32,
                reason: "Controlled review before source changes.".into(),
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap();
    let mut changed = stale_source.entity.unwrap();
    if let Some(pb::entity::Kind::Document(document)) = &mut changed.kind {
        let content = "This source has changed since expert review.";
        document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            content: content.into(),
            media_type: Some(pb::MediaType {
                value: "text/plain".into(),
            }),
            digest: Some(trogon_atlas_core::specification::content_digest(content)),
        }));
    }
    put(&mut client, changed, &stale_source.etag).await;
    let receipt = serde_json::json!({
        "grpc_endpoint": endpoint,
        "namespace": "shop",
        "published": {"namespace":"shop", "slug":"purchase", "version":"1"},
        "draft": {"namespace":"shop", "slug":"purchase", "version":"2"},
        "stale": {"namespace":"shop", "slug":"stale", "version":"1"}
    });
    let path = std::env::var("ATLAS_BROWSER_FIXTURE_FILE").expect("fixture receipt path required");
    std::fs::write(path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    tokio::select! {
        () = tokio::time::sleep(std::time::Duration::from_secs(600)) => {},
        result = tokio::signal::ctrl_c() => result.unwrap(),
    }
}

async fn reviewed(client: &mut Client) -> pb::GetGlossaryTermResponse {
    let draft = candidate(client, 1).await;
    client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("purchase", 1)),
                }),
                if_match: draft.etag,
                disposition: pb::GlossaryReviewDisposition::Accepted as i32,
                reason: "Controlled expert review.".into(),
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn refresh_detects_a_new_selected_source_version_without_destroying_old_publication() {
    let mut client = client().await;
    reviewed(&mut client).await;
    let current = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("source", 1)),
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner();
    let mut next = current.entity.unwrap();
    next.system = None;
    if let Some(pb::entity::Kind::Document(document)) = &mut next.kind {
        document.id = Some(id("source", 2));
        for subject in &mut document.subjects {
            subject.connection_id = None;
        }
    }
    put(&mut client, next, "").await;
    let refreshed = client
        .inspect_glossary_refresh(request(
            pb::InspectGlossaryRefreshRequest {
                selection: Some(selection(false)),
                documents: vec![pb::DocumentRef {
                    id: Some(id("source", 2)),
                }],
                generation_revision: "fixture/prompt-v1".into(),
            },
            "glossary-fixture-reader",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!refreshed.incomplete);
    assert_eq!(refreshed.observations.len(), 1);
    assert_eq!(
        refreshed.observations[0].evidence[0].state,
        pb::GlossaryEvidenceState::Changed as i32
    );
    assert_eq!(
        refreshed.observations[0].entry.as_ref().unwrap().state,
        pb::GlossaryEntryState::SourceChanged as i32
    );
    assert!(refreshed.observations[0].entry.as_ref().unwrap().published);
    let old = client
        .get_glossary_term(request(
            pb::GetGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("purchase", 1)),
                }),
            },
            "glossary-fixture-reader",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(old.entry.unwrap().published);
    assert_eq!(
        old.evidence[0].state,
        pb::GlossaryEvidenceState::Current as i32
    );
}

#[tokio::test]
async fn source_projection_refuses_oversized_json_instead_of_breaking_connected_clients() {
    let mut client = client().await;
    candidate(&mut client, 1).await;
    let current = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("source", 1)),
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let mut references = Vec::new();
    for index in 0..10 {
        let source_id = id(&format!("escaped-source-{index}"), 1);
        let mut source = current.clone();
        source.system = None;
        if let Some(pb::entity::Kind::Document(document)) = &mut source.kind {
            document.id = Some(source_id.clone());
            for subject in &mut document.subjects {
                subject.connection_id = None;
            }
            let content = "\0".repeat(60_000);
            document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                digest: Some(trogon_atlas_core::specification::content_digest(&content)),
                content,
                media_type: Some(pb::MediaType {
                    value: "text/plain".into(),
                }),
            }));
        }
        put(&mut client, source, "").await;
        references.push(pb::DocumentRef {
            id: Some(source_id),
        });
    }
    let result = client
        .inspect_glossary_sources(request(
            pb::InspectGlossarySourcesRequest {
                documents: references,
            },
            "glossary-fixture-reader",
        ))
        .await;
    match result {
        Err(error) => assert_eq!(error.code(), Code::ResourceExhausted),
        Ok(_) => panic!("oversized JSON source projection must refuse before transport"),
    }
}

#[tokio::test]
async fn oversized_review_is_refused_before_any_publication_is_applied() {
    let mut client = client().await;
    let draft = candidate(&mut client, 1).await;
    let source = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("source", 1)),
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner();
    let content = "\0".repeat(60_000);
    let mut source_entity = source.entity.unwrap();
    source_entity.system = None;
    if let Some(pb::entity::Kind::Document(document)) = &mut source_entity.kind {
        document.id = Some(id("large-source", 1));
        for subject in &mut document.subjects {
            subject.connection_id = None;
        }
        document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            digest: Some(trogon_atlas_core::specification::content_digest(&content)),
            content: content.clone(),
            media_type: Some(pb::MediaType {
                value: "text/plain".into(),
            }),
        }));
    }
    let retained = put(&mut client, source_entity, "").await.entity.unwrap();
    let mut entity = draft.entity.unwrap();
    entity.system = None;
    if let Some(pb::entity::Kind::Term(term)) = &mut entity.kind {
        term.id = Some(id("oversized-review", 1));
        let glossary = term.glossary.as_mut().unwrap();
        glossary.evidence = (0..32)
            .map(|index| pb::GlossaryEvidence {
                document: Some(pb::DocumentRef {
                    id: Some(id("large-source", 1)),
                }),
                owner_uid: retained.system.as_ref().unwrap().uid.clone(),
                content_digest: Some(trogon_atlas_core::specification::content_digest(&content)),
                locator: format!("declared-passage:{index}"),
                quote: "\0".repeat(16_000),
            })
            .collect();
    }
    let candidate = put(&mut client, entity, "").await;
    let result = client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("oversized-review", 1)),
                }),
                if_match: candidate.etag.clone(),
                disposition: pb::GlossaryReviewDisposition::Accepted as i32,
                reason: "Must not persist an uninspectable publication.".into(),
                ..Default::default()
            },
            "glossary-fixture-expert",
        ))
        .await;
    match result {
        Err(error) => assert_eq!(error.code(), Code::ResourceExhausted),
        Ok(_) => panic!("an oversized review response must be refused before mutation"),
    }
    let after = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Term as i32,
                id: Some(id("oversized-review", 1)),
            },
            "glossary-fixture-expert",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after.etag, candidate.etag);
    let Some(pb::entity::Kind::Term(term)) = after.entity.unwrap().kind else {
        panic!("term")
    };
    assert!(term.glossary.unwrap().review.is_none());
}
