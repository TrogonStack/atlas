#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use prost::Message;
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

fn request<T>(body: T) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        "Bearer lean-code-fixture-user".parse().unwrap(),
    );
    request
}

async fn client() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(
            r#"
[principals.reviewer]
role = "admin"
tokens = ["lean-code-fixture-user"]
[principals.reader]
role = "reader"
tokens = ["lean-code-fixture-reader"]
[principals.limited]
role = "reader"
parent = "visible"
tokens = ["lean-code-fixture-limited"]
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

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
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

async fn put(client: &mut Client, entity: pb::Entity) -> pb::PutEntityResponse {
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(entity),
            create_only: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn source(client: &mut Client, namespace: &str, slug: &str) -> pb::PutEntityResponse {
    let anchor = id(namespace, &format!("{slug}-fact"));
    put(
        client,
        entity(pb::entity::Kind::Event(pb::Event {
            id: Some(anchor.clone()),
            title: "Source fact".into(),
            ..Default::default()
        })),
    )
    .await;
    put(
        client,
        entity(pb::entity::Kind::Document(pb::Document {
            id: Some(id(namespace, slug)),
            title: Some(pb::CatalogTitle {
                value: "Controlled source".into(),
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
                content: SOURCE.repeat(200),
                media_type: Some(pb::MediaType {
                    value: "text/plain".into(),
                }),
                digest: Some(trogon_atlas_core::specification::content_digest(
                    &SOURCE.repeat(200),
                )),
            })),
            ..Default::default()
        })),
    )
    .await
}

fn candidate(source: &pb::PutEntityResponse, slug: &str, evidence_count: usize) -> pb::Entity {
    let source = source.entity.as_ref().unwrap();
    let source_id = trogon_atlas_core::entity_id(source).unwrap();
    entity(pb::entity::Kind::Term(pb::Term {
        id: Some(id(&source_id.namespace, slug)),
        title: format!("Purchase {slug}"),
        doc: "A customer request accepted by the store.".into(),
        glossary: Some(pb::GlossaryEntry {
            evidence: (0..evidence_count)
                .map(|occurrence| pb::GlossaryEvidence {
                    document: Some(pb::DocumentRef {
                        id: Some(source_id.clone()),
                    }),
                    owner_uid: source.system.as_ref().unwrap().uid.clone(),
                    content_digest: Some(trogon_atlas_core::specification::content_digest(
                        &SOURCE.repeat(200),
                    )),
                    locator: format!("paragraph:1; occurrence:{occurrence}"),
                    quote: SOURCE.repeat(100),
                })
                .collect(),
            generation_revision: "missing-source-budget-v1".into(),
            audience: "customer".into(),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

fn selection(namespaces: &[&str]) -> pb::GlossarySelection {
    pb::GlossarySelection {
        namespaces: namespaces
            .iter()
            .map(|namespace| (*namespace).into())
            .collect(),
        audience: "customer".into(),
        include_unreviewed: true,
    }
}

fn reader<T>(body: T) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        "Bearer lean-code-fixture-reader".parse().unwrap(),
    );
    request
}

async fn accept(client: &mut Client, draft: &pb::PutEntityResponse) -> pb::GetGlossaryTermResponse {
    client
        .review_glossary_term(request(pb::ReviewGlossaryTermRequest {
            term: Some(pb::TermRef {
                id: trogon_atlas_core::entity_id(draft.entity.as_ref().unwrap()).cloned(),
            }),
            if_match: draft.etag.clone(),
            disposition: pb::GlossaryReviewDisposition::Accepted as i32,
            reason: "Controlled synthetic user review.".into(),
            validate_only: false,
        }))
        .await
        .unwrap()
        .into_inner()
}

fn definition_request(slug: &str) -> pb::GetGlossaryTermRequest {
    pb::GetGlossaryTermRequest {
        term: Some(pb::TermRef {
            id: Some(id("shop", slug)),
        }),
    }
}

#[tokio::test]
async fn lean_pointers_and_definitions_omit_payload_and_keep_publication_and_paging() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source").await;
    let mut first = candidate(&source, "first", 32);
    let Some(pb::entity::Kind::Term(term)) = &mut first.kind else {
        panic!("term");
    };
    let mut metadata = prost_types::Struct::default();
    metadata.fields.insert(
        "private-payload-marker".into(),
        prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue("x".repeat(100_000))),
        },
    );
    term.metadata.push(prost_types::Any {
        type_url: "type.googleapis.com/google.protobuf.Struct".into(),
        value: metadata.encode_to_vec(),
    });
    let first = put(&mut client, first).await;
    let accepted = accept(&mut client, &first).await;
    let second = put(&mut client, candidate(&source, "second", 1)).await;
    accept(&mut client, &second).await;
    let mut newer = candidate(&source, "first", 1);
    let Some(pb::entity::Kind::Term(term)) = &mut newer.kind else {
        panic!("term");
    };
    term.id.as_mut().unwrap().version = 2;
    put(&mut client, newer).await;
    let mut selected = selection(&["shop"]);
    selected.include_unreviewed = false;
    let page = client
        .list_glossary_pointers(reader(pb::ListGlossaryEntriesRequest {
            selection: Some(selected.clone()),
            page_size: 1,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page.entries.len(), 1);
    assert_ne!(page.next_page_token, "");
    assert_eq!(
        page.entries[0]
            .term
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        1
    );
    let next = client
        .list_glossary_pointers(reader(pb::ListGlossaryEntriesRequest {
            selection: Some(selected.clone()),
            page_size: 1,
            page_token: page.next_page_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(next.entries.len(), 1);
    assert_eq!(next.next_page_token, "");
    assert_eq!(next.snapshot_revision, page.snapshot_revision);
    assert_ne!(next.entries[0].term, page.entries[0].term);
    let lean = client
        .get_glossary_definition(reader(definition_request("first")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(lean.definition, "A customer request accepted by the store.");
    assert_eq!(lean.evidence.len(), 32);
    assert!(!lean.incomplete);
    assert_eq!(
        lean.owner_uid,
        accepted
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
    );
    assert_eq!(lean.etag, accepted.etag);
    assert_ne!(lean.snapshot_revision, "");
    assert_eq!(
        client
            .get_glossary_definition(reader(definition_request("first")))
            .await
            .unwrap()
            .into_inner(),
        lean
    );
    assert!(accepted.encoded_len() > lean.encoded_len() * 50);
    assert!(page.encoded_len() < 512);
    let mut changed_source = source.entity.clone().unwrap();
    let Some(pb::entity::Kind::Document(document)) = &mut changed_source.kind else {
        panic!("document");
    };
    let content = format!("{} A later source revision.", SOURCE.repeat(200));
    document.body = Some(pb::document::Body::Retained(pb::RetainedDocumentText {
        content: content.clone(),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        digest: Some(trogon_atlas_core::specification::content_digest(&content)),
    }));
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(changed_source),
            if_match: source.etag,
            ..Default::default()
        }))
        .await
        .unwrap();
    let changed = client
        .get_glossary_definition(reader(definition_request("first")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(changed.owner_uid, lean.owner_uid);
    assert_eq!(changed.etag, lean.etag);
    assert_ne!(changed.snapshot_revision, lean.snapshot_revision);
    assert_eq!(
        changed.entry.as_ref().unwrap().state,
        pb::GlossaryEntryState::SourceChanged as i32
    );
    assert_eq!(
        client
            .get_glossary_definition(reader(definition_request("first")))
            .await
            .unwrap()
            .into_inner(),
        changed
    );
    assert_eq!(
        client
            .list_glossary_pointers(reader(pb::ListGlossaryEntriesRequest {
                selection: Some(selected.clone()),
                page_size: 1,
                page_token: page.next_page_token
            }))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    selected.include_unreviewed = true;
    assert_eq!(
        client
            .list_glossary_pointers(reader(pb::ListGlossaryEntriesRequest {
                selection: Some(selected),
                ..Default::default()
            }))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

#[tokio::test]
async fn lean_definition_projects_before_the_full_record_json_gate() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source").await;
    let mut draft = candidate(&source, "large", 1);
    let Some(pb::entity::Kind::Term(term)) = &mut draft.kind else {
        panic!("term");
    };
    let mut metadata = prost_types::Struct::default();
    metadata.fields.insert(
        "payload".into(),
        prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue(
                "\u{0001}".repeat(600_000),
            )),
        },
    );
    term.metadata.push(prost_types::Any {
        type_url: "type.googleapis.com/google.protobuf.Struct".into(),
        value: metadata.encode_to_vec(),
    });
    put(&mut client, draft).await;
    assert_eq!(
        client
            .get_glossary_term(reader(definition_request("large")))
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
    let lean = client
        .get_glossary_definition(reader(definition_request("large")))
        .await
        .unwrap()
        .into_inner();
    assert!(lean.encoded_len() < 2048);
    assert_eq!(
        lean.entry.unwrap().state,
        pb::GlossaryEntryState::Draft as i32
    );
}

#[tokio::test]
async fn code_sensor_matches_identifier_segments_and_prose_keeps_word_boundaries() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source").await;
    let mut draft = candidate(&source, "legacy-booking", 1);
    let Some(pb::entity::Kind::Term(term)) = &mut draft.kind else {
        panic!("term");
    };
    term.title = "Legacy booking".into();
    term.glossary.as_mut().unwrap().policy = Some(pb::GlossaryUsagePolicy {
        usage: pb::GlossaryUsage::Prohibited as i32,
        rationale: "Use Parcel in this audience.".into(),
        ..Default::default()
    });
    let draft = put(&mut client, draft).await;
    accept(&mut client, &draft).await;
    let text = "é legacy_booking LegacyBooking legacyBooking LEGACY_BOOKING xlegacy_booking \"legacyBooking\"";
    let mut selected = selection(&["shop"]);
    selected.include_unreviewed = false;
    let input = pb::CheckGlossaryUsageRequest {
        selection: Some(selected),
        text: text.into(),
        text_revision: "code-exact-v1".into(),
        skip_quoted: true,
        ..Default::default()
    };
    let prose = client
        .check_glossary_usage(reader(input.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(prose.findings, Vec::<pb::GlossaryUsageFinding>::new());
    let code = client
        .check_glossary_code_usage(reader(input))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        code.findings
            .iter()
            .map(|finding| finding.matched.as_str())
            .collect::<Vec<_>>(),
        vec![
            "legacy_booking",
            "LegacyBooking",
            "legacyBooking",
            "LEGACY_BOOKING"
        ]
    );
    for finding in &code.findings {
        let range = finding.range.as_ref().unwrap();
        assert_eq!(
            &text[range.start as usize..range.end as usize],
            finding.matched
        );
    }
    assert_eq!(code.text_revision, "code-exact-v1");
}

#[tokio::test]
async fn term_usage_period_enforces_dates_and_binds_guarded_review() {
    let mut client = client().await;
    let source = source(&mut client, "shop", "source").await;
    let mut draft = candidate(&source, "period", 1);
    let Some(pb::entity::Kind::Term(term)) = &mut draft.kind else {
        panic!("term");
    };
    term.title = "Purchase".into();
    term.glossary.as_mut().unwrap().usage_period = Some(pb::GlossaryUsagePeriod {
        valid_from: Some(pb::CalendarDate {
            value: "2026-11-01".into(),
        }),
        valid_until: Some(pb::CalendarDate {
            value: "2026-12-01".into(),
        }),
    });
    let draft = put(&mut client, draft).await;
    let accepted = accept(&mut client, &draft).await;
    let mut selected = selection(&["shop"]);
    selected.include_unreviewed = false;
    for (on, count) in [
        ("2026-10-31", 1),
        ("2026-11-01", 0),
        ("2026-11-30", 0),
        ("2026-12-01", 1),
    ] {
        let input = pb::CheckGlossaryUsageRequest {
            selection: Some(selected.clone()),
            text: "Purchase".into(),
            text_revision: "period-copy-v1".into(),
            on: Some(pb::CalendarDate { value: on.into() }),
            ..Default::default()
        };
        let checked = client
            .check_glossary_usage(reader(input.clone()))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(checked.findings.len(), count, "{on}");
        let code = client
            .check_glossary_code_usage(reader(input))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(code.findings.len(), count, "{on}");
    }
    let lean = client
        .get_glossary_definition(reader(definition_request("period")))
        .await
        .unwrap()
        .into_inner();
    let mut changed = accepted.entity.clone().unwrap();
    let Some(pb::entity::Kind::Term(term)) = &mut changed.kind else {
        panic!("term");
    };
    assert_eq!(
        lean.usage_period,
        term.glossary.as_ref().unwrap().usage_period
    );
    term.glossary
        .as_mut()
        .unwrap()
        .usage_period
        .as_mut()
        .unwrap()
        .valid_until = Some(pb::CalendarDate {
        value: "2027-01-01".into(),
    });
    assert_eq!(
        client
            .put_entity(request(pb::PutEntityRequest {
                entity: Some(changed),
                if_match: accepted.etag.clone(),
                ..Default::default()
            }))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let mut invalid = candidate(&source, "invalid-period", 1);
    let Some(pb::entity::Kind::Term(term)) = &mut invalid.kind else {
        panic!("term");
    };
    term.glossary.as_mut().unwrap().usage_period = Some(pb::GlossaryUsagePeriod::default());
    assert_eq!(
        client
            .put_entity(request(pb::PutEntityRequest {
                entity: Some(invalid),
                create_only: true,
                ..Default::default()
            }))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let negative = client
        .review_glossary_term(request(pb::ReviewGlossaryTermRequest {
            term: Some(pb::TermRef {
                id: Some(id("shop", "period")),
            }),
            if_match: accepted.etag,
            disposition: pb::GlossaryReviewDisposition::NeedsChanges as i32,
            reason: "The declared use period requires expert reconsideration.".into(),
            validate_only: false,
        }))
        .await
        .unwrap()
        .into_inner();
    let mut corrected = negative.entity.unwrap();
    let Some(pb::entity::Kind::Term(term)) = &mut corrected.kind else {
        panic!("term");
    };
    term.glossary
        .as_mut()
        .unwrap()
        .usage_period
        .as_mut()
        .unwrap()
        .valid_until = Some(pb::CalendarDate {
        value: "2027-01-01".into(),
    });
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(corrected),
            if_match: negative.etag,
            ..Default::default()
        }))
        .await
        .unwrap();
    let stale = client
        .get_glossary_term(reader(definition_request("period")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        stale.entry.unwrap().state,
        pb::GlossaryEntryState::StaleReview as i32
    );
}

#[tokio::test]
async fn lean_reads_preserve_hidden_missing_equivalence_and_authenticated_reader_access() {
    let mut client = client().await;
    let namespace = client
        .register_namespace(request(pb::RegisterNamespaceRequest {
            name: "hidden-glossary".into(),
            parent: "hidden".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let source = source(&mut client, &namespace, "source").await;
    let draft = put(&mut client, candidate(&source, "private", 1)).await;
    accept(&mut client, &draft).await;
    let limited = |body| {
        let mut request = Request::new(body);
        request.metadata_mut().insert(
            "authorization",
            "Bearer lean-code-fixture-limited".parse().unwrap(),
        );
        request
    };
    let mut selected = selection(&[&namespace]);
    selected.include_unreviewed = false;
    let mut index = Request::new(pb::ListGlossaryEntriesRequest {
        selection: Some(selected),
        ..Default::default()
    });
    index.metadata_mut().insert(
        "authorization",
        "Bearer lean-code-fixture-limited".parse().unwrap(),
    );
    assert_eq!(
        client
            .list_glossary_pointers(index)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let mut errors = Vec::new();
    for slug in ["private", "missing"] {
        let error = client
            .get_glossary_definition(limited(pb::GetGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id(&namespace, slug)),
                }),
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::NotFound);
        errors.push(error.message().to_owned());
    }
    assert_eq!(errors[0], errors[1]);
    assert_eq!(
        client
            .get_glossary_definition(Request::new(pb::GetGlossaryTermRequest {
                term: Some(pb::TermRef {
                    id: Some(id(&namespace, "private"))
                })
            }))
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
}
