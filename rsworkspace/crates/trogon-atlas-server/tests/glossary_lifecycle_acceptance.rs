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
const REVIEWER: &str = "glossary-lifecycle-reviewer";
const READER: &str = "glossary-lifecycle-reader";
const SOURCE: &str = "A purchase is a customer request accepted by the store.";
const CORRECTED_DEFINITION: &str = "A customer request accepted by the store.";

fn request<T>(body: T, identity: &str) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {identity}").parse().unwrap(),
    );
    request
}

async fn client() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.reviewer]
role = "admin"
tokens = ["{REVIEWER}"]
[principals.reader]
role = "reader"
tokens = ["{READER}"]
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

fn id(slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: "glossary-lifecycle".into(),
        slug: slug.into(),
        version,
    }
}

fn term(entity: &pb::Entity) -> &pb::Term {
    let Some(pb::entity::Kind::Term(term)) = &entity.kind else {
        panic!("term fixture required");
    };
    term
}

fn term_mut(entity: &mut pb::Entity) -> &mut pb::Term {
    let Some(pb::entity::Kind::Term(term)) = &mut entity.kind else {
        panic!("term fixture required");
    };
    term
}

async fn put(client: &mut Client, entity: pb::Entity, etag: &str) -> pb::PutEntityResponse {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: etag.into(),
                create_only: etag.is_empty(),
                ..Default::default()
            },
            REVIEWER,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn source(client: &mut Client, slug: &str, version: u64) -> pb::PutEntityResponse {
    let anchor = id(&format!("{slug}-{version}-fact"), 1);
    put(
        client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(anchor.clone()),
                title: "Source fact".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
        "",
    )
    .await;
    put(
        client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(id(slug, version)),
                title: Some(pb::CatalogTitle {
                    value: "Controlled glossary source".into(),
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
            ..Default::default()
        },
        "",
    )
    .await
}

fn candidate(source: &pb::PutEntityResponse, slug: &str) -> pb::Entity {
    let source = source.entity.as_ref().unwrap();
    pb::Entity {
        kind: Some(pb::entity::Kind::Term(pb::Term {
            id: Some(id(slug, 1)),
            title: "Purchase".into(),
            doc: "A purchase request.".into(),
            glossary: Some(pb::GlossaryEntry {
                audience: "customer".into(),
                generation_revision: "lifecycle-fixture/prompt-v1".into(),
                next_review_on: Some(pb::CalendarDate {
                    value: "9999-12-31".into(),
                }),
                evidence: vec![pb::GlossaryEvidence {
                    document: Some(pb::DocumentRef {
                        id: Some(trogon_atlas_core::entity_id(source).unwrap().clone()),
                    }),
                    owner_uid: source.system.as_ref().unwrap().uid.clone(),
                    content_digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
                    locator: "paragraph:1".into(),
                    quote: "a customer request accepted by the store".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn review(
    client: &mut Client,
    term_id: &pb::Id,
    etag: &str,
    disposition: pb::GlossaryReviewDisposition,
    reason: &str,
) -> pb::GetGlossaryTermResponse {
    client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(term_id.clone()),
                }),
                if_match: etag.into(),
                disposition: disposition as i32,
                reason: reason.into(),
                ..Default::default()
            },
            REVIEWER,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn detail(client: &mut Client) -> pb::GetGlossaryTermResponse {
    client
        .get_glossary_term(request(
            pb::GetGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("purchase", 1)),
                }),
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn selection(include_unreviewed: bool) -> pb::GlossarySelection {
    pb::GlossarySelection {
        namespaces: vec!["glossary-lifecycle".into()],
        audience: "customer".into(),
        include_unreviewed,
    }
}

async fn list(client: &mut Client, include_unreviewed: bool) -> pb::ListGlossaryEntriesResponse {
    client
        .list_glossary_entries(request(
            pb::ListGlossaryEntriesRequest {
                selection: Some(selection(include_unreviewed)),
                ..Default::default()
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn read(client: &mut Client, kind: pb::EntityKind, target: pb::Id) -> pb::GetEntityResponse {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(target),
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn delete(
    client: &mut Client,
    target: pb::Id,
    etag: &str,
) -> Result<pb::DeleteEntityResponse, tonic::Status> {
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Term as i32,
                id: Some(target),
                if_match: etag.into(),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            },
            REVIEWER,
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn incoming(client: &mut Client) -> pb::GetReferencesResponse {
    client
        .get_incoming_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Term as i32,
                id: Some(id("purchase", 1)),
                ..Default::default()
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn assert_current_publication(value: &pb::GetGlossaryTermResponse) {
    assert!(!value.incomplete);
    let entry = value.entry.as_ref().unwrap();
    assert!(entry.published);
    assert_eq!(entry.state, pb::GlossaryEntryState::Reviewed as i32);
    let term = term(value.entity.as_ref().unwrap());
    assert_eq!(term.doc, CORRECTED_DEFINITION);
    let review = term.glossary.as_ref().unwrap().review.as_ref().unwrap();
    assert_eq!(
        review.disposition,
        pb::GlossaryReviewDisposition::Accepted as i32
    );
    assert_eq!(review.actor, "reviewer");
    assert_eq!(value.evidence.len(), 1);
    assert_eq!(
        value.evidence[0].state,
        pb::GlossaryEvidenceState::Current as i32
    );
}

async fn expert_corrected_and_accepted(client: &mut Client) {
    let source = source(client, "source", 1).await;
    let draft = put(client, candidate(&source, "purchase"), "").await;
    let mut corrected = draft.entity.unwrap();
    term_mut(&mut corrected).doc = CORRECTED_DEFINITION.into();
    let corrected = put(client, corrected, &draft.etag).await;
    let accepted = review(
        client,
        &id("purchase", 1),
        &corrected.etag,
        pb::GlossaryReviewDisposition::Accepted,
        "Accept the exact definition after the reviewer corrected its wording.",
    )
    .await;
    assert_current_publication(&accepted);
}

#[tokio::test]
async fn explicit_rejection_is_recorded_but_never_published() {
    let mut client = client().await;
    let source = source(&mut client, "source", 1).await;
    let draft = put(&mut client, candidate(&source, "purchase"), "").await;
    let rejected = review(
        &mut client,
        &id("purchase", 1),
        &draft.etag,
        pb::GlossaryReviewDisposition::Rejected,
        "Reject this candidate because its wording needs a new proposal.",
    )
    .await;
    let entry = rejected.entry.as_ref().unwrap();
    assert!(!entry.published);
    assert_eq!(entry.state, pb::GlossaryEntryState::Rejected as i32);
    let review = term(rejected.entity.as_ref().unwrap())
        .glossary
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap();
    assert_eq!(
        review.disposition,
        pb::GlossaryReviewDisposition::Rejected as i32
    );
    assert_eq!(review.actor, "reviewer");
    assert_ne!(review.reviewed_at, "");
    assert_eq!(detail(&mut client).await, rejected);
    assert_eq!(
        list(&mut client, false).await.entries,
        Vec::<pb::GlossaryIndexEntry>::new()
    );
    let all = list(&mut client, true).await;
    assert!(!all.incomplete);
    assert_eq!(all.entries, vec![entry.clone()]);
}

#[tokio::test]
async fn explicit_retirement_allows_deletion_only_after_incoming_references_are_removed() {
    let mut client = client().await;
    expert_corrected_and_accepted(&mut client).await;
    let accepted = detail(&mut client).await;
    let source = read(&mut client, pb::EntityKind::Document, id("source", 1)).await;
    let mut referring = candidate(
        &pb::PutEntityResponse {
            entity: source.entity.clone(),
            etag: source.etag.clone(),
            ..Default::default()
        },
        "legacy-purchase",
    );
    term_mut(&mut referring).glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Deprecated as i32,
        preferred_term: Some(pb::TermRef {
            id: Some(id("purchase", 1)),
        }),
        rationale: "Prefer the reviewed Purchase definition.".into(),
        ..Default::default()
    });
    let referring = put(&mut client, referring, "").await;
    let retired = review(
        &mut client,
        &id("purchase", 1),
        &accepted.etag,
        pb::GlossaryReviewDisposition::Rejected,
        "Retire this formerly published definition; remove it after its referrers are repaired.",
    )
    .await;
    assert!(!retired.entry.as_ref().unwrap().published);
    assert_eq!(
        retired.entry.as_ref().unwrap().state,
        pb::GlossaryEntryState::Rejected as i32
    );
    assert_eq!(
        term(retired.entity.as_ref().unwrap()).doc,
        CORRECTED_DEFINITION
    );
    assert_eq!(
        list(&mut client, false).await.entries,
        Vec::<pb::GlossaryIndexEntry>::new()
    );
    let references = incoming(&mut client).await;
    assert!(references
        .references
        .iter()
        .any(|reference| { reference.from_field == "glossary.policy.preferred_term" }));
    assert_eq!(
        delete(&mut client, id("purchase", 1), &retired.etag)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(detail(&mut client).await, retired);
    delete(&mut client, id("legacy-purchase", 1), &referring.etag)
        .await
        .unwrap();
    assert_eq!(
        incoming(&mut client).await.references,
        Vec::<pb::Reference>::new()
    );
    let deleted = delete(&mut client, id("purchase", 1), &retired.etag)
        .await
        .unwrap();
    assert_eq!(deleted.referrers, Vec::<pb::Reference>::new());
    assert_eq!(
        client
            .get_glossary_term(request(
                pb::GetGlossaryTermRequest {
                    term: Some(pb::TermRef {
                        id: Some(id("purchase", 1)),
                    }),
                },
                READER,
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(
        list(&mut client, true).await.entries,
        Vec::<pb::GlossaryIndexEntry>::new()
    );
    assert_eq!(
        read(&mut client, pb::EntityKind::Document, id("source", 1))
            .await
            .etag,
        source.etag
    );
}

#[tokio::test]
async fn failed_refresh_preserves_exact_expert_edit_review_and_publication() {
    let mut client = client().await;
    expert_corrected_and_accepted(&mut client).await;
    source(&mut client, "source", 2).await;
    let before = detail(&mut client).await;
    assert_current_publication(&before);
    let before_listing = list(&mut client, false).await;
    let error = client
        .inspect_glossary_refresh(request(
            pb::InspectGlossaryRefreshRequest {
                selection: Some(selection(false)),
                documents: vec![
                    pb::DocumentRef {
                        id: Some(id("source", 1)),
                    },
                    pb::DocumentRef {
                        id: Some(id("source", 2)),
                    },
                ],
                generation_revision: "lifecycle-fixture/prompt-v2".into(),
            },
            READER,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains("one exact source version"));
    assert_eq!(detail(&mut client).await, before);
    assert_eq!(list(&mut client, false).await, before_listing);
}

#[tokio::test]
async fn partial_refresh_does_not_change_or_delete_a_definition_whose_source_was_omitted() {
    let mut client = client().await;
    expert_corrected_and_accepted(&mut client).await;
    source(&mut client, "other-source", 1).await;
    let before = detail(&mut client).await;
    assert_current_publication(&before);
    let before_listing = list(&mut client, false).await;
    let omitted_source = read(&mut client, pb::EntityKind::Document, id("source", 1)).await;
    let partial = client
        .inspect_glossary_refresh(request(
            pb::InspectGlossaryRefreshRequest {
                selection: Some(selection(false)),
                documents: vec![
                    pb::DocumentRef {
                        id: Some(id("other-source", 1)),
                    },
                    pb::DocumentRef {
                        id: Some(id("unavailable-source", 1)),
                    },
                ],
                generation_revision: "lifecycle-fixture/prompt-v2".into(),
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(partial.incomplete);
    assert_eq!(partial.sources.len(), 2);
    assert_eq!(
        partial.sources[0].state,
        pb::GlossaryEvidenceState::Current as i32
    );
    assert_eq!(
        partial.sources[1].state,
        pb::GlossaryEvidenceState::Unavailable as i32
    );
    assert_eq!(
        partial.sources[1].document.as_ref().unwrap().id,
        Some(id("unavailable-source", 1))
    );
    assert_eq!(partial.observations.len(), 1);
    let observation = &partial.observations[0];
    assert!(observation.generation_changed);
    assert_eq!(observation.entry, before.entry);
    assert_eq!(observation.evidence, before.evidence);
    assert_eq!(detail(&mut client).await, before);
    assert_eq!(list(&mut client, false).await, before_listing);
    assert_eq!(
        read(&mut client, pb::EntityKind::Document, id("source", 1)).await,
        omitted_source
    );
}

fn prefer(entity: &mut pb::Entity, preferred: pb::Id) {
    term_mut(entity).glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Deprecated as i32,
        preferred_term: Some(pb::TermRef {
            id: Some(preferred),
        }),
        rationale: "Use the preferred contextual definition instead.".into(),
        ..Default::default()
    });
}

async fn assert_preferred_update_refused(
    client: &mut Client,
    before: &pb::GetEntityResponse,
    preferred: pb::Id,
    expected_error: &str,
) {
    let mut proposed = before.entity.clone().unwrap();
    let term_id = term(&proposed).id.clone().unwrap();
    assert!(term(&proposed).glossary.as_ref().unwrap().review.is_none());
    let before_listing = list(client, true).await;
    prefer(&mut proposed, preferred);
    let error = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(proposed),
                if_match: before.etag.clone(),
                ..Default::default()
            },
            REVIEWER,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains(expected_error), "{error}");
    assert_eq!(read(client, pb::EntityKind::Term, term_id).await, *before);
    assert_eq!(list(client, true).await, before_listing);
}

#[tokio::test]
async fn self_replacement_is_refused_without_mutating_the_draft_or_etag() {
    let mut client = client().await;
    let source = source(&mut client, "source", 1).await;
    put(&mut client, candidate(&source, "purchase"), "").await;
    let before = read(&mut client, pb::EntityKind::Term, id("purchase", 1)).await;
    assert_preferred_update_refused(
        &mut client,
        &before,
        id("purchase", 1),
        "policy.self_replacement",
    )
    .await;
    assert_eq!(
        incoming(&mut client).await.references,
        Vec::<pb::Reference>::new()
    );
}

#[tokio::test]
async fn missing_preferred_target_is_refused_without_mutating_the_draft_or_etag() {
    let mut client = client().await;
    let source = source(&mut client, "source", 1).await;
    put(&mut client, candidate(&source, "purchase"), "").await;
    let before = read(&mut client, pb::EntityKind::Term, id("purchase", 1)).await;
    assert_preferred_update_refused(
        &mut client,
        &before,
        id("missing", 1),
        "glossary target is unavailable",
    )
    .await;
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Term as i32,
                    id: Some(id("missing", 1)),
                },
                READER,
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test]
async fn two_term_replacement_cycle_is_refused_without_mutating_either_draft_or_etag() {
    let mut client = client().await;
    let source = source(&mut client, "source", 1).await;
    let first = put(&mut client, candidate(&source, "purchase"), "").await;
    put(&mut client, candidate(&source, "replacement"), "").await;
    let mut first = first.entity.unwrap();
    prefer(&mut first, id("replacement", 1));
    let current = read(&mut client, pb::EntityKind::Term, id("purchase", 1)).await;
    put(&mut client, first, &current.etag).await;
    let first_before = read(&mut client, pb::EntityKind::Term, id("purchase", 1)).await;
    let second_before = read(&mut client, pb::EntityKind::Term, id("replacement", 1)).await;
    assert!(term(first_before.entity.as_ref().unwrap())
        .glossary
        .as_ref()
        .unwrap()
        .review
        .is_none());
    assert_eq!(
        term(first_before.entity.as_ref().unwrap())
            .glossary
            .as_ref()
            .unwrap()
            .policy
            .as_ref()
            .unwrap()
            .preferred_term,
        Some(pb::TermRef {
            id: Some(id("replacement", 1)),
        })
    );
    assert_preferred_update_refused(
        &mut client,
        &second_before,
        id("purchase", 1),
        "glossary preferred-term cycle",
    )
    .await;
    assert_eq!(
        read(&mut client, pb::EntityKind::Term, id("purchase", 1)).await,
        first_before
    );
    assert_eq!(
        incoming(&mut client).await.references,
        Vec::<pb::Reference>::new()
    );
}

async fn review_attempt(
    client: &mut Client,
    etag: &str,
    disposition: pb::GlossaryReviewDisposition,
    reason: &str,
) -> Result<pb::GetGlossaryTermResponse, tonic::Status> {
    client
        .review_glossary_term(request(
            pb::ReviewGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id("purchase", 1)),
                }),
                if_match: etag.into(),
                disposition: disposition as i32,
                reason: reason.into(),
                ..Default::default()
            },
            REVIEWER,
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn force_delete(
    client: &mut Client,
    kind: pb::EntityKind,
    target: pb::Id,
    etag: &str,
) -> pb::DeleteEntityResponse {
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: kind as i32,
                id: Some(target),
                if_match: etag.into(),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            REVIEWER,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn assert_negative_review_can_retire_unchanged_content(
    client: &mut Client,
    before: pb::GetGlossaryTermResponse,
    disposition: pb::GlossaryReviewDisposition,
    acceptance_error: &str,
) {
    assert!(before.entry.as_ref().unwrap().published);
    let original = term(before.entity.as_ref().unwrap());
    let original_review = original.glossary.as_ref().unwrap().review.as_ref().unwrap();
    assert_eq!(
        original_review.disposition,
        pb::GlossaryReviewDisposition::Accepted as i32
    );
    let positive = review_attempt(
        client,
        &before.etag,
        pb::GlossaryReviewDisposition::Accepted,
        "New acceptance must revalidate the exact external prerequisites.",
    )
    .await
    .unwrap_err();
    assert_eq!(positive.code(), Code::InvalidArgument);
    assert!(positive.message().contains(acceptance_error), "{positive}");
    assert_eq!(detail(client).await, before);
    assert_eq!(
        delete(client, id("purchase", 1), &before.etag)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(detail(client).await, before);
    let negative = review_attempt(
        client,
        &before.etag,
        disposition,
        "Withdraw the unchanged meaning while its external prerequisites require repair.",
    )
    .await
    .expect("negative review must retire unchanged content despite broken external prerequisites");
    assert_ne!(negative.etag, before.etag);
    assert!(!negative.entry.as_ref().unwrap().published);
    let expected_state = match disposition {
        pb::GlossaryReviewDisposition::Rejected => pb::GlossaryEntryState::Rejected,
        pb::GlossaryReviewDisposition::NeedsChanges => pb::GlossaryEntryState::NeedsChanges,
        _ => panic!("a negative review disposition is required"),
    };
    assert_eq!(
        negative.entry.as_ref().unwrap().state,
        expected_state as i32
    );
    assert_eq!(negative.evidence, before.evidence);
    let mut unchanged = term(negative.entity.as_ref().unwrap()).clone();
    let recorded = unchanged
        .glossary
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap();
    assert_eq!(recorded.disposition, disposition as i32);
    assert_eq!(recorded.actor, "reviewer");
    assert_eq!(recorded.content_digest, original_review.content_digest);
    assert_eq!(recorded.owner_uid, original_review.owner_uid);
    unchanged.glossary.as_mut().unwrap().review = Some(original_review.clone());
    assert_eq!(unchanged, *original);
    assert_eq!(detail(client).await, negative);
    assert_eq!(
        list(client, false).await.entries,
        Vec::<pb::GlossaryIndexEntry>::new()
    );
    let positive = review_attempt(
        client,
        &negative.etag,
        pb::GlossaryReviewDisposition::Accepted,
        "A negative decision must not authorize subsequent acceptance of broken inputs.",
    )
    .await
    .unwrap_err();
    assert_eq!(positive.code(), Code::InvalidArgument);
    assert!(positive.message().contains(acceptance_error), "{positive}");
    assert_eq!(detail(client).await, negative);
    let mut unsupported = negative.entity.clone().unwrap();
    term_mut(&mut unsupported)
        .glossary
        .as_mut()
        .unwrap()
        .evidence[0]
        .quote = "This unsupported passage is absent from the retained source.".into();
    let unsupported = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(unsupported),
                if_match: negative.etag.clone(),
                ..Default::default()
            },
            REVIEWER,
        ))
        .await
        .unwrap_err();
    assert_eq!(unsupported.code(), Code::InvalidArgument);
    assert_eq!(detail(client).await, negative);
    assert_eq!(
        incoming(client).await.references,
        Vec::<pb::Reference>::new()
    );
    assert_eq!(
        delete(client, id("purchase", 1), &negative.etag)
            .await
            .unwrap()
            .referrers,
        Vec::<pb::Reference>::new()
    );
    assert_eq!(
        client
            .get_glossary_term(request(
                pb::GetGlossaryTermRequest {
                    term: Some(pb::TermRef {
                        id: Some(id("purchase", 1)),
                    }),
                },
                READER,
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test]
async fn rejected_review_can_retire_accepted_content_after_source_bytes_change() {
    let mut client = client().await;
    expert_corrected_and_accepted(&mut client).await;
    let source = read(&mut client, pb::EntityKind::Document, id("source", 1)).await;
    let mut changed = source.entity.unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed.kind else {
        panic!("source document required");
    };
    let content = "This revised source no longer supports the published definition.";
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: content.into(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(content)),
    }));
    put(&mut client, changed, &source.etag).await;
    let before = detail(&mut client).await;
    assert_eq!(
        before.evidence[0].state,
        pb::GlossaryEvidenceState::Changed as i32
    );
    assert_negative_review_can_retire_unchanged_content(
        &mut client,
        before,
        pb::GlossaryReviewDisposition::Rejected,
        "evidence.content_digest",
    )
    .await;
}

#[tokio::test]
async fn needs_changes_review_can_retire_accepted_content_after_source_is_deleted() {
    let mut client = client().await;
    expert_corrected_and_accepted(&mut client).await;
    let source = read(&mut client, pb::EntityKind::Document, id("source", 1)).await;
    let deleted = force_delete(
        &mut client,
        pb::EntityKind::Document,
        id("source", 1),
        &source.etag,
    )
    .await;
    assert!(deleted
        .referrers
        .iter()
        .any(|reference| reference.from_field == "glossary.evidence[0].document"));
    let before = detail(&mut client).await;
    assert_eq!(
        before.evidence[0].state,
        pb::GlossaryEvidenceState::Unavailable as i32
    );
    assert_negative_review_can_retire_unchanged_content(
        &mut client,
        before,
        pb::GlossaryReviewDisposition::NeedsChanges,
        "glossary target is unavailable",
    )
    .await;
}

#[tokio::test]
async fn rejected_review_can_retire_accepted_content_after_preferred_target_is_deleted() {
    let mut client = client().await;
    let source = source(&mut client, "source", 1).await;
    let replacement = put(&mut client, candidate(&source, "replacement"), "").await;
    let mut proposed = candidate(&source, "purchase");
    term_mut(&mut proposed).doc = CORRECTED_DEFINITION.into();
    prefer(&mut proposed, id("replacement", 1));
    let draft = put(&mut client, proposed, "").await;
    let accepted = review(
        &mut client,
        &id("purchase", 1),
        &draft.etag,
        pb::GlossaryReviewDisposition::Accepted,
        "Accept this exact sourced definition with a currently available replacement.",
    )
    .await;
    assert_current_publication(&accepted);
    let deleted = force_delete(
        &mut client,
        pb::EntityKind::Term,
        id("replacement", 1),
        &replacement.etag,
    )
    .await;
    assert!(deleted
        .referrers
        .iter()
        .any(|reference| reference.from_field == "glossary.policy.preferred_term"));
    let before = detail(&mut client).await;
    assert_eq!(
        before.evidence[0].state,
        pb::GlossaryEvidenceState::Current as i32
    );
    assert_negative_review_can_retire_unchanged_content(
        &mut client,
        before,
        pb::GlossaryReviewDisposition::Rejected,
        "glossary target is unavailable",
    )
    .await;
}
