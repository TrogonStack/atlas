#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const NS: &str = "mcp-impact";

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServiceRunning =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn start() -> (McpClient, McpServiceRunning, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(svc))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let endpoint = format!("http://{addr}");
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (server, client) = tokio::join!(
        support::mcp_server(&endpoint, None).serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client.unwrap(), server.unwrap(), endpoint)
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

async fn classify(client: &McpClient, candidate: u64, max_depth: u32) -> serde_json::Value {
    let spec = |version: u64| serde_json::json!({"kind": "event", "namespace": NS, "slug": "placed", "version": version});
    let args = serde_json::json!({
        "current": spec(1),
        "candidate": spec(candidate),
        "max_depth": max_depth,
    });
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("classify_impact")
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn classify_impact_returns_every_verdict_as_proto_json() {
    let (client, service, endpoint) = start().await;
    seed(&endpoint).await;

    let bump = classify(&client, 2, 0).await;
    assert_eq!(bump["clientVerdict"], "bump_only");
    assert_eq!(bump["verdict"], "DEPENDENT_CLASSIFICATION_BUMP_ONLY");
    assert_eq!(bump["counts"]["bumpOnly"], 2);
    assert_eq!(bump["ops"][0]["path"], "title");
    assert_eq!(bump["clientSnapshotConsistent"], true);
    assert!(bump["snapshotId"].is_string());
    assert_ne!(bump["snapshotId"], "");

    let breaking = classify(&client, 3, 0).await;
    assert_eq!(breaking["clientVerdict"], "breaking");
    let slice = breaking["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["dependent"]["id"]["slug"] == "place-order")
        .unwrap();
    assert_eq!(slice["depth"], 1);
    assert_eq!(
        slice["occurrences"][0]["consumedPaths"],
        serde_json::json!(["schema.fields[order_id]"])
    );
    assert_eq!(
        slice["occurrences"][0]["causalOps"][0]["path"],
        "schema.fields[amount]"
    );
    let model = breaking["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["dependent"]["id"]["slug"] == "shop")
        .unwrap();
    assert_eq!(model["classification"], "DEPENDENT_CLASSIFICATION_UNKNOWN");
    assert_eq!(
        model["unknownReasons"],
        serde_json::json!(["CLASSIFICATION_UNKNOWN_REASON_TRANSITIVE_THROUGH_AFFECTED"])
    );
    assert_eq!(
        model["through"][0]["throughClassification"],
        "DEPENDENT_CLASSIFICATION_BREAKING"
    );

    let unknown = classify(&client, 2, 1).await;
    assert_eq!(unknown["clientVerdict"], "unknown");
    assert_eq!(
        unknown["incompleteness"],
        serde_json::json!(["IMPACT_INCOMPLETENESS_DEPTH_LIMITED"])
    );

    drop(client);
    service.cancel().await.unwrap();
}
