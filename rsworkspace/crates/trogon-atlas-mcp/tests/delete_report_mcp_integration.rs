#![allow(clippy::unwrap_used)]

use std::convert::Infallible;

use base64::Engine as _;
use prost::Message as _;
use rmcp::ServiceExt as _;
use trogon_atlas_client::client::{ConnectOptions, Transport};
use trogon_atlas_mcp::server::McpServer;
use trogon_atlas_proto as pb;

fn grpc_response(message: &impl prost::Message) -> http::Response<axum::body::Body> {
    let encoded = message.encode_to_vec();
    let mut frame = vec![0];
    frame.extend_from_slice(&u32::try_from(encoded.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(&encoded);
    http::Response::builder()
        .header("content-type", "application/grpc")
        .header("grpc-status", "0")
        .body(axum::body::Body::from(frame))
        .unwrap()
}

#[tokio::test]
async fn incomplete_delete_report_retains_typed_flag_and_warns_without_claiming_empty_coverage() {
    let transport = Transport::new(tower::service_fn(
        |request: http::Request<tonic::body::Body>| async move {
            let response = if request.uri().path().ends_with("/GetServerInfo") {
                grpc_response(&pb::GetServerInfoResponse {
                    schema_version: pb::SCHEMA_VERSION.into(),
                    contract_revision: pb::CONTRACT_REVISION,
                    min_client_contract_revision: pb::MIN_CLIENT_CONTRACT_REVISION,
                    features: Some(pb::get_server_info_response::Features {
                        mutations: true,
                        validate_only: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            } else {
                assert!(request.uri().path().ends_with("/DeleteEntity"));
                grpc_response(&pb::DeleteEntityResponse {
                    operation_receipt: None,
                    referrers: vec![],
                    referrers_incomplete: true,
                })
            };
            Ok::<_, Infallible>(response)
        },
    ));
    let server = McpServer::new(transport, ConnectOptions::default()).unwrap();
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    let (server, client) = tokio::join!(
        server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    let server = server.unwrap();
    let client = client.unwrap();
    let result = client.call_tool(rmcp::model::CallToolRequestParams::new("delete_entity").with_arguments(
        serde_json::json!({"kind":"event","namespace":"catalog-report","slug":"receipt","version":1,"mode":"dry_run"}).as_object().unwrap().clone()
    )).await.unwrap();
    let envelope: serde_json::Value =
        serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(envelope["binpb_base64"].as_str().unwrap())
        .unwrap();
    let decoded = pb::DeleteEntityResponse::decode(bytes.as_slice()).unwrap();
    assert!(decoded.referrers_incomplete);
    assert_eq!(decoded.referrers, []);
    assert!(result
        .content
        .iter()
        .filter_map(|item| item.as_text())
        .any(|text| text
            .text
            .contains("Incoming-reference report is incomplete")));
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
