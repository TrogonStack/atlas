#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use rmcp::ServiceExt;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

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

async fn start_client() -> McpClient {
    let endpoint = start_server().await;
    let server = support::mcp_server(&endpoint, None);
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    let (service, client) = tokio::join!(
        server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    std::mem::forget(service.unwrap());
    client.unwrap()
}

async fn call(
    client: &McpClient,
    tool: &'static str,
    arguments: Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(
            rmcp::model::CallToolRequestParams::new(tool)
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

fn body(result: &rmcp::model::CallToolResult) -> Value {
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

const MANIFESTS: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Component
metadata: { namespace: mcp-groups, name: worker }
spec: { title: Worker }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: SystemGroup
metadata: { namespace: mcp-groups, name: platform }
spec:
  title: Platform
  scope: SYSTEM_GROUP_SCOPE_INTERNAL
  members:
    - member: { component: worker@1 }
    - member: { component: worker@1 }
";

#[tokio::test(flavor = "multi_thread")]
async fn mcp_authors_reads_and_references_native_groups_with_parallel_members() {
    let client = start_client().await;
    let applied = call(
        &client,
        "apply_manifests",
        json!({"manifests_yaml": MANIFESTS}),
    )
    .await;
    assert!(!applied.is_error.unwrap_or(false), "{applied:?}");
    assert_eq!(
        body(&applied)["lines"],
        json!([
            "component:mcp-groups/worker@1 created",
            "system_group:mcp-groups/platform@1 created"
        ])
    );
    let reapplied = call(
        &client,
        "apply_manifests",
        json!({"manifests_yaml": MANIFESTS}),
    )
    .await;
    assert_eq!(
        body(&reapplied)["lines"],
        json!([
            "component:mcp-groups/worker@1 unchanged",
            "system_group:mcp-groups/platform@1 unchanged"
        ])
    );

    let fetched = call(
        &client,
        "get_entity",
        json!({"kind":"system_group","namespace":"mcp-groups","slug":"platform","version":1}),
    )
    .await;
    assert!(!fetched.is_error.unwrap_or(false), "{fetched:?}");
    let entity = {
        use base64::Engine as _;
        use prost::Message as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(body(&fetched)["binpb_base64"].as_str().unwrap())
            .unwrap();
        trogon_atlas_proto::GetEntityResponse::decode(raw.as_slice())
            .unwrap()
            .entity
            .unwrap()
    };
    let Some(trogon_atlas_proto::entity::Kind::SystemGroup(group)) = entity.kind else {
        panic!("expected system group");
    };
    let ids: Vec<_> = group
        .members
        .iter()
        .map(|member| member.connection_id.clone().unwrap().value)
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);

    let incoming = call(
        &client,
        "get_incoming_references",
        json!({"kind":"component","namespace":"mcp-groups","slug":"worker","version":1,"filter_kinds":["system_group"]}),
    )
    .await;
    assert!(!incoming.is_error.unwrap_or(false), "{incoming:?}");
    let references = {
        use base64::Engine as _;
        use prost::Message as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(body(&incoming)["binpb_base64"].as_str().unwrap())
            .unwrap();
        trogon_atlas_proto::GetReferencesResponse::decode(raw.as_slice())
            .unwrap()
            .references
    };
    let fields: Vec<_> = references
        .iter()
        .map(|hit| hit.from_field.clone())
        .collect();
    assert_eq!(fields, ["members[0].component", "members[1].component"]);

    let kinds = call(&client, "list_entity_kinds", json!({})).await;
    let catalog = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(body(&kinds)["binpb_base64"].as_str().unwrap())
            .unwrap()
    };
    let catalog = String::from_utf8_lossy(&catalog);
    assert!(catalog.contains("ENTITY_KIND_SYSTEM_GROUP"));
    assert!(catalog.contains("systemGroup"));

    let protected = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("delete_entity").with_arguments(
                json!({"kind":"component","namespace":"mcp-groups","slug":"worker","version":1})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    let error = protected.expect_err("referenced component must not be deleted");
    assert!(
        error.to_string().contains("entity is referenced"),
        "{error}"
    );
    let still_there = call(
        &client,
        "get_entity",
        json!({"kind":"component","namespace":"mcp-groups","slug":"worker","version":1}),
    )
    .await;
    assert!(!still_there.is_error.unwrap_or(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_refuses_self_membership_and_unresolved_targets_without_writing() {
    let client = start_client().await;
    for member in [
        "systemGroup: loop@1",
        "component: ghost@1",
        "designGraph: graph@1",
    ] {
        let manifest = format!(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: SystemGroup\nmetadata: {{ namespace: mcp-refused, name: loop }}\nspec:\n  title: Loop\n  scope: SYSTEM_GROUP_SCOPE_INTERNAL\n  members:\n    - member: {{ {member} }}\n"
        );
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("apply_manifests").with_arguments(
                    json!({"manifests_yaml": manifest})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await;
        let refused = match result {
            Ok(result) => result.is_error.unwrap_or(false),
            Err(_) => true,
        };
        assert!(refused, "{member}");
    }
    let absent = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity").with_arguments(
                json!({"kind":"system_group","namespace":"mcp-refused","slug":"loop","version":1})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(absent.map_or(true, |result| result.is_error.unwrap_or(false)));
}
