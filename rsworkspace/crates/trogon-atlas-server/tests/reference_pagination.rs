#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashSet, sync::Arc};

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{transport::Channel, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{store::WriteContext, Store};

const OWNERS: usize = 4_097;
type Client = EventModelServiceClient<Channel>;

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "reference-pages".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn screen(slug: &str, transitions: usize) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Ui(pb::Ui {
            id: Some(id(slug)),
            transitions: (0..transitions)
                .map(|_| pb::ui::Transition {
                    to: Some(pb::UiRef {
                        id: Some(id("target")),
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn references(slug: &str, token: String) -> pb::GetReferencesRequest {
    pb::GetReferencesRequest {
        kind: pb::EntityKind::Ui as i32,
        id: Some(id(slug)),
        page_size: i32::MAX,
        page_token: token,
        ..Default::default()
    }
}

fn impact(token: String) -> pb::GetImpactRequest {
    pb::GetImpactRequest {
        root: Some(pb::EntityRef {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("target")),
        }),
        max_depth: 1,
        page_size: i32::MAX,
        page_token: token,
        ..Default::default()
    }
}

async fn seeded_client() -> Client {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    store
        .put(
            pb::EntityKind::Ui,
            &screen("target", 0),
            false,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .put(
            pb::EntityKind::Ui,
            &screen("many-outgoing", OWNERS),
            false,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    for index in 0..OWNERS {
        store
            .put(
                pb::EntityKind::Ui,
                &screen(&format!("owner-{index:04}"), 1),
                false,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Client::connect(format!("http://{addr}")).await.unwrap()
}

#[tokio::test]
async fn reference_and_impact_pages_do_not_reject_large_result_totals() {
    let mut client = seeded_client().await;
    let incoming = client
        .get_incoming_references(Request::new(references("target", String::new())))
        .await;
    let outgoing = client
        .get_outgoing_references(Request::new(references("many-outgoing", String::new())))
        .await;
    let impacted = client.get_impact(Request::new(impact(String::new()))).await;
    let statuses = [
        incoming.as_ref().err().map(tonic::Status::code),
        outgoing.as_ref().err().map(tonic::Status::code),
        impacted.as_ref().err().map(tonic::Status::code),
    ];
    assert_eq!(
        statuses,
        [None, None, None],
        "bounded pages must not be refused because candidate totals exceed 4096"
    );

    let mut response = incoming.unwrap().into_inner();
    let mut incoming_occurrences = HashSet::new();
    loop {
        assert_ne!(response.references, []);
        assert!(response.references.len() <= 500);
        for reference in response.references {
            let owner = reference.from.unwrap().id.unwrap();
            assert!(incoming_occurrences.insert((owner.slug, reference.from_field)));
        }
        if response.next_page_token.is_empty() {
            break;
        }
        response = client
            .get_incoming_references(Request::new(references("target", response.next_page_token)))
            .await
            .unwrap()
            .into_inner();
    }
    assert_eq!(incoming_occurrences.len(), OWNERS * 2);

    let mut response = outgoing.unwrap().into_inner();
    let mut outgoing_occurrences = HashSet::new();
    loop {
        assert_ne!(response.references, []);
        assert!(response.references.len() <= 500);
        for reference in response.references {
            assert_eq!(reference.to.unwrap().id, Some(id("target")));
            assert!(outgoing_occurrences.insert(reference.from_field));
        }
        if response.next_page_token.is_empty() {
            break;
        }
        response = client
            .get_outgoing_references(Request::new(references(
                "many-outgoing",
                response.next_page_token,
            )))
            .await
            .unwrap()
            .into_inner();
    }
    assert_eq!(outgoing_occurrences.len(), OWNERS);

    let mut response = impacted.unwrap().into_inner();
    let mut impacted_owners = HashSet::new();
    loop {
        assert_ne!(response.nodes, []);
        assert!(response.nodes.len() <= 500);
        for node in response.nodes {
            assert!(impacted_owners.insert(node.entity.unwrap().id.unwrap().slug));
        }
        if response.next_page_token.is_empty() {
            break;
        }
        response = client
            .get_impact(Request::new(impact(response.next_page_token)))
            .await
            .unwrap()
            .into_inner();
    }
    assert_eq!(impacted_owners.len(), OWNERS + 2);
}
