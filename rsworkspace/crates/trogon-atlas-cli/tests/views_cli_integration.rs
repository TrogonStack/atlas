#![allow(clippy::unwrap_used, clippy::expect_used)]

// `trogon-atlas get`, `describe` and `tree` against an in-process gRPC
// server, spawning the compiled binary like export_cli_integration.rs. A
// blocking Command::output on the current_thread runtime deadlocks the
// in-process server, so every test here uses the multi_thread flavor.

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::service::EventModelServiceImpl;

const NS: &str = "cli-views";

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: NS.into(),
        slug: slug.into(),
        version: 1,
    }
}

async fn put(client: &mut trogon_atlas_client::client::Client, kind: pb::entity::Kind) {
    client
        .put_entity(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                system: None,
                kind: Some(kind),
            }),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
            operation_id: String::new(),
        })
        .await
        .unwrap();
}

fn trogon_atlas(endpoint: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .expect("trogon-atlas must spawn")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_describe_and_tree_read_a_seeded_namespace() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    put(
        &mut client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id("order.placed")),
            title: "Order placed".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Command(pb::Command {
            id: Some(id("place-order")),
            title: "Place order".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("place-order-slice")),
            title: "Place order".into(),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("place-order")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("order.placed")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;

    let got = trogon_atlas(&endpoint, &["get", "event/cli-views/order.placed"]);
    assert!(got.status.success(), "{got:?}");
    let yaml = stdout(&got);
    assert!(yaml.contains("kind: Event\n"), "{yaml}");
    assert!(yaml.contains("name: order.placed\n"), "{yaml}");

    let described = trogon_atlas(
        &endpoint,
        &[
            "--format",
            "json",
            "describe",
            "event/cli-views/order.placed@1",
        ],
    );
    assert!(described.status.success(), "{described:?}");
    let outcome: serde_json::Value = serde_json::from_str(&stdout(&described)).unwrap();
    assert_eq!(outcome["ok"], true, "{outcome}");
    assert_eq!(
        outcome["data"]["incoming"][0],
        serde_json::json!({
            "field": "emitted_events[0]",
            "entity": "command_slice:cli-views/place-order-slice@1",
        }),
        "{outcome}"
    );

    let tree = trogon_atlas(&endpoint, &["tree", NS]);
    assert!(tree.status.success(), "{tree:?}");
    assert!(
        stdout(&tree).contains(
            "└── command_slice place-order-slice  Place order\n        ├── command place-order  Place order\n        └── event order.placed  Order placed\n"
        ),
        "{}",
        stdout(&tree)
    );

    let missing = trogon_atlas(&endpoint, &["get", "event/cli-views/nope"]);
    assert_eq!(missing.status.code(), Some(6), "{missing:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn namespaces_lists_counts_and_bare_tree_covers_every_namespace() {
    let endpoint = start_server().await;
    let mut client = trogon_atlas_client::client::connect(&endpoint, None, 30)
        .await
        .unwrap();
    let event = |namespace: &str, slug: &str| {
        pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: namespace.into(),
                slug: slug.into(),
                version: 1,
            }),
            ..Default::default()
        })
    };
    put(&mut client, event("cli-views-ns", "a")).await;
    put(&mut client, event("cli-views-ns", "b")).await;
    let registered = client
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "cli-views-registered".into(),
            parent: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    put(&mut client, event(&registered, "c")).await;

    let listed = trogon_atlas(&endpoint, &["--format", "json", "namespaces"]);
    assert!(listed.status.success(), "{listed:?}");
    let outcome: serde_json::Value = serde_json::from_str(&stdout(&listed)).unwrap();
    let entry = outcome["data"]["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "cli-views-ns")
        .cloned();
    assert_eq!(entry.unwrap()["entity_count"], 2, "{outcome}");
    let entry = outcome["data"]["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "cli-views-registered")
        .cloned();
    assert_eq!(entry.unwrap()["id"], registered.as_str(), "{outcome}");

    let text = trogon_atlas(&endpoint, &["namespaces"]);
    assert!(stdout(&text).starts_with("NAME"), "{text:?}");
    assert!(stdout(&text).contains("\ncli-views-ns "), "{text:?}");

    let tree = trogon_atlas(&endpoint, &["--format", "json", "tree"]);
    assert!(tree.status.success(), "{tree:?}");
    let outcome: serde_json::Value = serde_json::from_str(&stdout(&tree)).unwrap();
    let trees = outcome["data"]["trees"].as_array().unwrap();
    assert!(
        trees.iter().any(|t| t["namespace"] == "cli-views-ns"),
        "{outcome}"
    );
    assert!(
        trees.iter().any(|t| t["namespace"] == registered.as_str()),
        "{outcome}"
    );
}
