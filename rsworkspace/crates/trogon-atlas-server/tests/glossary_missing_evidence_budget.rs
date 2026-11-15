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
        "Bearer missing-evidence-fixture-user".parse().unwrap(),
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
tokens = ["missing-evidence-fixture-user"]
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
                content: SOURCE.into(),
                media_type: Some(pb::MediaType {
                    value: "text/plain".into(),
                }),
                digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
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
                    content_digest: Some(trogon_atlas_core::specification::content_digest(SOURCE)),
                    locator: format!("paragraph:1; occurrence:{occurrence}"),
                    quote: "a customer request accepted by the store".into(),
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

#[tokio::test]
async fn missing_source_lookups_share_the_request_examination_budget() {
    let mut client = client().await;
    let mut base_sources = Vec::new();
    for index in 0..3 {
        base_sources.push(source(&mut client, "budget-base", &format!("source-{index}")).await);
    }
    let extra_source = source(&mut client, "budget-extra", "source").await;
    assert!(base_sources
        .iter()
        .all(|source| source.entity.as_ref().unwrap().encoded_len() * 10_000 < 32 * 1024 * 1024));
    for index in 0..312 {
        put(
            &mut client,
            candidate(&base_sources[index % 3], &format!("term-{index}"), 32),
        )
        .await;
    }
    put(
        &mut client,
        candidate(&base_sources[0], "term-remainder", 16),
    )
    .await;
    put(&mut client, candidate(&extra_source, "term-overflow", 1)).await;
    for source in base_sources.iter().chain([&extra_source]) {
        client
            .delete_entity(request(pb::DeleteEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(
                    trogon_atlas_core::entity_id(source.entity.as_ref().unwrap())
                        .unwrap()
                        .clone(),
                ),
                if_match: source.etag.clone(),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            }))
            .await
            .unwrap();
    }
    let at_limit = client
        .list_glossary_entries(request(pb::ListGlossaryEntriesRequest {
            selection: Some(selection(&["budget-base"])),
            page_size: 500,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(at_limit.entries.len(), 313);
    assert!(at_limit.incomplete);
    assert_eq!(at_limit.next_page_token, "");
    assert!(at_limit.entries.iter().all(|entry| !entry.published));
    let overflow = client
        .list_glossary_entries(request(pb::ListGlossaryEntriesRequest {
            selection: Some(selection(&["budget-base", "budget-extra"])),
            page_size: 500,
            ..Default::default()
        }))
        .await;
    assert!(
        overflow.is_err(),
        "10,001 missing source examinations must refuse"
    );
    assert_eq!(overflow.unwrap_err().code(), Code::ResourceExhausted);
}
