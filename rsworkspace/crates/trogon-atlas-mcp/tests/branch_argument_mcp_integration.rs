//! The per-call `branch` tool argument, driven through the full MCP dispatch
//! over an in-process duplex pipe. Branch state is checked over plain gRPC so
//! the assertions do not depend on how a tool shapes its result.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use rmcp::{model::CallToolRequestParams, ServiceExt as _};
use tokio::net::TcpListener;
use tonic::Code;
use trogon_atlas_client::client::{connect_with_options, Client, ConnectOptions};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::service::EventModelServiceImpl;

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpService =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(svc))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

/// An MCP client with no connect-time branch, so only the `branch` argument
/// can move a call off baseline.
async fn mcp_client(grpc_endpoint: &str) -> (McpClient, McpService) {
    let server = support::mcp_server(grpc_endpoint, None);
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    let (service, client) = tokio::join!(
        server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client.unwrap(), service.unwrap())
}

async fn grpc(endpoint: &str, branch: Option<&str>) -> Client {
    connect_with_options(
        endpoint,
        None,
        30,
        ConnectOptions {
            branch: branch.map(str::to_owned),
            ..ConnectOptions::default()
        },
    )
    .await
    .unwrap()
}

fn call(name: &'static str, arguments: serde_json::Value) -> CallToolRequestParams {
    let serde_json::Value::Object(arguments) = arguments else {
        panic!("tool arguments must be a JSON object");
    };
    CallToolRequestParams::new(name).with_arguments(arguments)
}

fn event(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 1,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn get_event(ns: &str, slug: &str) -> pb::GetEntityRequest {
    pb::GetEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }),
    }
}

async fn create_branch(endpoint: &str, name: &str) {
    trogon_atlas_client::branch::create_branch(&mut grpc(endpoint, None).await, name, "")
        .await
        .unwrap();
}

/// Whether a tool call failed, as either a protocol error or an `isError`
/// result, so this holds however the adapter reports a failure.
fn failed(result: &Result<rmcp::model::CallToolResult, rmcp::ServiceError>) -> bool {
    match result {
        Ok(result) => result.is_error == Some(true),
        Err(_) => true,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_with_a_branch_argument_lands_on_that_branch_only() {
    let endpoint = start_server().await;
    create_branch(&endpoint, "arg-write").await;
    let (client, service) = mcp_client(&endpoint).await;

    let entity_json = serde_json::json!({
        "event": {
            "id": {"namespace": "branch-arg-write", "slug": "drafted", "version": 1},
            "title": "Drafted on a branch",
        }
    })
    .to_string();
    let put = client
        .call_tool(call(
            "put_entity_json",
            serde_json::json!({"entity_json": entity_json, "branch": "arg-write"}),
        ))
        .await;
    assert!(!failed(&put), "{put:?}");

    let on_branch = grpc(&endpoint, Some("arg-write"))
        .await
        .get_entity(get_event("branch-arg-write", "drafted"))
        .await;
    assert!(
        on_branch.is_ok(),
        "the branch holds the write: {on_branch:?}"
    );
    let on_baseline = grpc(&endpoint, None)
        .await
        .get_entity(get_event("branch-arg-write", "drafted"))
        .await
        .unwrap_err();
    assert_eq!(on_baseline.code(), Code::NotFound, "baseline is unchanged");

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_with_a_branch_argument_sees_the_branch_delta() {
    let endpoint = start_server().await;
    create_branch(&endpoint, "arg-read").await;
    grpc(&endpoint, Some("arg-read"))
        .await
        .put_entity(pb::PutEntityRequest {
            entity: Some(event("branch-arg-read", "drafted", "Only on arg-read")),
            create_only: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let (client, service) = mcp_client(&endpoint).await;
    let spec = serde_json::json!({
        "kind": "event",
        "namespace": "branch-arg-read",
        "slug": "drafted",
        "version": 1,
    });

    let mut on_branch = spec.clone();
    on_branch["branch"] = "arg-read".into();
    let read = client
        .call_tool(call("get_entity_json", on_branch))
        .await
        .unwrap();
    assert_ne!(read.is_error, Some(true), "{read:?}");
    let text = &read.content[0].as_text().unwrap().text;
    assert!(text.contains("Only on arg-read"), "{text}");

    // Without the argument the call still reads baseline, which never saw it.
    let baseline = client.call_tool(call("get_entity_json", spec)).await;
    assert!(failed(&baseline), "{baseline:?}");

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_without_a_branch_argument_still_refuse_one() {
    let endpoint = start_server().await;
    let (client, service) = mcp_client(&endpoint).await;

    let result = client
        .call_tool(call(
            "create_branch",
            serde_json::json!({"name": "arg-ignored", "branch": "elsewhere"}),
        ))
        .await;
    assert!(failed(&result), "{result:?}");

    service.cancel().await.unwrap();
}
