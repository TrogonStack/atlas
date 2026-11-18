#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const NS: &str = "cli-impact";

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn id(slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: NS.into(),
        slug: slug.into(),
        version,
    }
}

fn fields(names: &[&str]) -> prost_types::Any {
    trogon_atlas_core::schema::pack_fields(
        names
            .iter()
            .map(|name| pb::FieldSpec {
                name: (*name).into(),
                r#type: Some(pb::FieldType {
                    kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
                }),
                ..Default::default()
            })
            .collect(),
    )
}

fn event(version: u64, title: &str, names: &[&str]) -> pb::entity::Kind {
    pb::entity::Kind::Event(pb::Event {
        id: Some(id("placed", version)),
        title: title.into(),
        schema: Some(fields(names)),
        ..Default::default()
    })
}

async fn seed(endpoint: &str) {
    let mut grpc = trogon_atlas_client::client::connect(endpoint, None, 30)
        .await
        .unwrap();
    let entities = [
        pb::entity::Kind::Command(pb::Command {
            id: Some(id("place", 1)),
            title: "Place".into(),
            schema: Some(fields(&["order_id"])),
            ..Default::default()
        }),
        event(1, "Placed", &["order_id"]),
        event(2, "Order placed", &["order_id"]),
        event(3, "Placed", &["order_id", "amount"]),
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("place-order", 1)),
            title: "Place order".into(),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("place", 1)),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("placed", 1)),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
        pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id("shop", 1)),
            title: "Shop".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id("place-order", 1)),
            }],
            ..Default::default()
        }),
    ];
    for kind in entities {
        grpc.put_entity(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                system: None,
                kind: Some(kind),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    }
}

fn classify(endpoint: &str, candidate: u64, extra: &[&str]) -> (Option<i32>, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(["--format", "json", "classify-impact"])
        .arg(format!("event/{NS}/placed@1"))
        .arg(format!("event/{NS}/placed@{candidate}"))
        .args(extra)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    (output.status.code(), value)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn classify_impact_exits_by_verdict_and_prints_the_client_verdict() {
    let endpoint = start_server().await;
    seed(&endpoint).await;

    let (code, value) = classify(&endpoint, 2, &[]);
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(value["data"]["clientVerdict"], "bump_only");
    assert_eq!(
        value["data"]["verdict"],
        "DEPENDENT_CLASSIFICATION_BUMP_ONLY"
    );
    assert_eq!(value["data"]["counts"]["bumpOnly"], 2);
    assert_eq!(value["data"]["clientSnapshotConsistent"], true);
    assert!(value["data"]["snapshotId"].is_string());
    assert_ne!(value["data"]["snapshotId"], "");

    let (code, value) = classify(&endpoint, 3, &[]);
    assert_eq!(code, Some(1), "{value}");
    assert_eq!(value["data"]["clientVerdict"], "breaking");
    let slice = value["data"]["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["dependent"]["id"]["slug"] == "place-order")
        .unwrap();
    assert_eq!(slice["classification"], "DEPENDENT_CLASSIFICATION_BREAKING");
    assert_eq!(
        slice["occurrences"][0]["reference"]["fromField"],
        "emitted_events[0]"
    );
    assert_eq!(
        slice["occurrences"][0]["causalOps"][0]["path"],
        "schema.fields[amount]"
    );

    let (code, value) = classify(&endpoint, 2, &["--max-depth", "1"]);
    assert_eq!(code, Some(12), "{value}");
    assert_eq!(value["data"]["clientVerdict"], "unknown");
    assert_eq!(
        value["data"]["incompleteness"],
        serde_json::json!(["IMPACT_INCOMPLETENESS_DEPTH_LIMITED"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn classify_impact_requires_exact_versions() {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(["--endpoint", "http://127.0.0.1:1", "classify-impact"])
        .arg(format!("event/{NS}/placed"))
        .arg(format!("event/{NS}/placed@2"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}
