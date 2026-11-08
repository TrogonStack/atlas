#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    process::{Command, Output},
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::{artifacts::sha256_hex, client::connect};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    artifacts::{storage::NatsBlobStorage, ArtifactService},
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

const ADMIN: &str = "artifact-roundtrip-admin";
const WRITER: &str = "artifact-roundtrip-writer";
const READER: &str = "artifact-roundtrip-reader";
const HIDDEN: &str = "artifact-roundtrip-hidden";
const OWNER: &str = "artifact-roundtrip-owner";

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

struct Fixture {
    endpoint: String,
    namespace: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn start() -> Self {
        let nats = trogon_atlas_testsupport::shared().await;
        let config = nats.store_config();
        let connection = async_nats::connect(&nats.url).await.unwrap();
        let jetstream = async_nats::jetstream::new(connection);
        let blobs = NatsBlobStorage::open(
            jetstream.clone(),
            async_nats::jetstream::object_store::Config {
                bucket: format!("{}-artifacts", config.bucket),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let catalog = jetstream
            .create_key_value(async_nats::jetstream::kv::Config {
                bucket: format!("{}-artifact-catalog", config.bucket),
                ..Default::default()
            })
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}");
        let artifacts = ArtifactService::new(
            Arc::new(blobs),
            catalog,
            8 * 1024 * 1024,
            Duration::from_secs(300),
        )
        .unwrap();
        let service = EventModelServiceImpl::try_new(
            Arc::new(
                trogon_atlas_store::NatsStore::connect_with(config)
                    .await
                    .unwrap(),
            ),
            ServerMetrics::default(),
        )
        .unwrap()
        .with_artifacts(Arc::new(artifacts));
        let registry = TokenRegistry::from_file_contents(
            serde_json::from_value(json!({"principals": {
                "admin": {"role": "admin", "tokens": [ADMIN]},
                "writer": {"role": "writer", "kind": "agent", "parent": OWNER, "tokens": [WRITER]},
                "reader": {"role": "reader", "kind": "agent", "parent": OWNER, "tokens": [READER]},
                "hidden": {"role": "reader", "kind": "agent", "parent": "outside-owner", "tokens": [HIDDEN]}
            }}))
            .unwrap(),
        )
        .unwrap();
        let secured = data_plane(
            EventModelServiceServer::new(service)
                .max_decoding_message_size(36 * 1024 * 1024)
                .max_encoding_message_size(36 * 1024 * 1024),
            BearerAuth::new(registry),
        );
        let (drain, _) = tokio::sync::watch::channel(0i64);
        let mcp = mcp_router(
            secured.clone(),
            RpcMetricsLayer::new(ServerMetrics::default()),
            InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
            Some(Duration::from_secs(30)),
            36 * 1024 * 1024,
            "127.0.0.1".parse().unwrap(),
            None,
        )
        .unwrap();
        let mut routes = tonic::service::Routes::builder();
        routes.add_service(secured);
        let mut routes = routes.routes();
        *routes.axum_router_mut() = routes.axum_router_mut().clone().merge(mcp);
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .accept_http1(true)
                .add_routes(routes)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let mut admin = connect(&endpoint, Some(ADMIN), 30).await.unwrap();
        let namespace = admin
            .register_namespace(pb::RegisterNamespaceRequest {
                name: "artifact-roundtrip".into(),
                parent: OWNER.into(),
            })
            .await
            .unwrap()
            .into_inner()
            .namespace
            .unwrap()
            .id;
        Self {
            endpoint,
            namespace,
            task,
        }
    }

    async fn cli(&self, token: &str, arguments: &[&str]) -> Output {
        let endpoint = self.endpoint.clone();
        let token = token.to_owned();
        let arguments = arguments
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        tokio::task::spawn_blocking(move || {
            Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
                .env("TROGON_ATLAS_AUTH_TOKEN", token)
                .args([
                    "--endpoint",
                    &endpoint,
                    "--rpc-timeout-secs",
                    "30",
                    "--format",
                    "json",
                ])
                .args(arguments)
                .output()
                .unwrap()
        })
        .await
        .unwrap()
    }

    async fn mcp(&self, token: &str) -> McpClient {
        ().serve(StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", self.endpoint))
                .auth_header(token),
        ))
        .await
        .unwrap()
    }
}

fn cli_result(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], true);
    body["data"].clone()
}

async fn tool(client: &McpClient, name: &'static str, request: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(
                json!({"request_json": request})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap()
}

async fn refused(client: &McpClient, name: &'static str, request: Value) -> bool {
    match client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(
                json!({"request_json": request})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
    {
        Err(_) => true,
        Ok(result) => result.is_error.unwrap_or(false),
    }
}

fn tool_result(result: &CallToolResult) -> Value {
    assert!(!result.is_error.unwrap_or(false), "MCP operation failed");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cli_uploads_discovers_and_downloads_verified_nats_artifacts() {
    let fixture = Fixture::start().await;
    let info = cli_result(&fixture.cli(READER, &["artifacts", "info"]).await);
    assert_eq!(info["backend"], "nats");
    assert_eq!(info["enabled"], true);
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.bin");
    let content = vec![0x61; 5 * 1024 * 1024];
    std::fs::write(&input, &content).unwrap();
    let artifact = cli_result(
        &fixture
            .cli(
                WRITER,
                &[
                    "artifacts",
                    "upload",
                    "--namespace",
                    &fixture.namespace,
                    "--file",
                    input.to_str().unwrap(),
                ],
            )
            .await,
    );
    assert_eq!(artifact["sha256"], sha256_hex(&content));
    assert_eq!(artifact["size"], content.len().to_string());
    assert!(artifact.get("uploadUrl").is_none());
    assert!(artifact.get("contentUrl").is_none());
    let id = artifact["artifactId"].as_str().unwrap();
    let metadata = cli_result(
        &fixture
            .cli(
                READER,
                &[
                    "artifacts",
                    "get",
                    "--namespace",
                    &fixture.namespace,
                    "--id",
                    id,
                ],
            )
            .await,
    );
    assert_eq!(metadata["artifact"], artifact);
    let output = directory.path().join("download.bin");
    cli_result(
        &fixture
            .cli(
                READER,
                &[
                    "artifacts",
                    "download",
                    "--namespace",
                    &fixture.namespace,
                    "--id",
                    id,
                    "--output",
                    output.to_str().unwrap(),
                ],
            )
            .await,
    );
    assert_eq!(std::fs::read(&output).unwrap(), content);
    let hidden = fixture
        .cli(
            HIDDEN,
            &[
                "artifacts",
                "get",
                "--namespace",
                &fixture.namespace,
                "--id",
                id,
            ],
        )
        .await;
    assert!(!hidden.status.success());
    let denied = fixture
        .cli(
            READER,
            &[
                "artifacts",
                "upload",
                "--namespace",
                &fixture.namespace,
                "--file",
                input.to_str().unwrap(),
            ],
        )
        .await;
    assert!(!denied.status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_coordinates_verified_uploads_and_enforces_reader_visibility() {
    let fixture = Fixture::start().await;
    let writer = fixture.mcp(WRITER).await;
    let reader = fixture.mcp(READER).await;
    let hidden = fixture.mcp(HIDDEN).await;
    let writer_tools = writer.list_all_tools().await.unwrap();
    assert!(writer_tools
        .iter()
        .any(|tool| tool.name == "begin_artifact_upload"));
    let reader_tools = reader.list_all_tools().await.unwrap();
    assert!(!reader_tools
        .iter()
        .any(|tool| tool.name == "begin_artifact_upload"));
    assert!(reader_tools.iter().any(|tool| tool.name == "get_artifact"));
    let content = b"MCP uploaded capture";
    let request = json!({
        "namespace": fixture.namespace, "filename": "capture.bin",
        "mediaType": "application/octet-stream", "size": content.len().to_string(),
        "sha256": sha256_hex(content),
    });
    let upload = tool_result(&tool(&writer, "begin_artifact_upload", request.clone()).await);
    assert_ne!(upload["uploadId"].as_str().unwrap(), "");
    assert!(upload
        .get("directUpload")
        .is_none_or(|value| value == false));
    assert!(upload.get("uploadUrl").is_none_or(|value| value == ""));
    assert!(upload
        .get("headers")
        .is_none_or(|value| value.as_object().is_some_and(serde_json::Map::is_empty)));
    let id = upload["uploadId"].as_str().unwrap();
    let pending = refused(
        &reader,
        "get_artifact",
        json!({"namespace": fixture.namespace, "artifactId": id}),
    )
    .await;
    assert!(pending);
    let mut client = connect(&fixture.endpoint, Some(WRITER), 30).await.unwrap();
    client
        .upload_artifact_content(pb::UploadArtifactContentRequest {
            namespace: fixture.namespace.clone(),
            upload_id: id.to_owned(),
            content: content.to_vec(),
        })
        .await
        .unwrap();
    let completed = tool_result(
        &tool(
            &writer,
            "complete_artifact_upload",
            json!({
                "namespace": fixture.namespace, "uploadId": id,
            }),
        )
        .await,
    );
    assert_eq!(completed["artifact"]["sha256"], sha256_hex(content));
    assert!(completed["artifact"].get("contentUrl").is_none());
    let artifact = tool_result(
        &tool(
            &reader,
            "get_artifact",
            json!({
                "namespace": fixture.namespace, "artifactId": id,
            }),
        )
        .await,
    );
    assert_eq!(artifact, completed);
    let mut reader_client = connect(&fixture.endpoint, Some(READER), 30).await.unwrap();
    let download = reader_client
        .get_artifact_download(pb::GetArtifactDownloadRequest {
            namespace: fixture.namespace.clone(),
            artifact_id: id.to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(download.code(), tonic::Code::FailedPrecondition);
    assert!(download.message().contains("GetArtifactContent"));
    assert!(
        refused(
            &reader,
            "get_artifact_download",
            json!({"namespace": fixture.namespace, "artifactId": id}),
        )
        .await
    );
    let recovered = reader_client
        .get_artifact_content(pb::GetArtifactContentRequest {
            namespace: fixture.namespace.clone(),
            artifact_id: id.to_owned(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(recovered.content, content);
    let hidden_result = refused(
        &hidden,
        "get_artifact",
        json!({
            "namespace": fixture.namespace, "artifactId": id,
        }),
    )
    .await;
    assert!(hidden_result);
    let denied = reader
        .call_tool(
            CallToolRequestParams::new("begin_artifact_upload").with_arguments(
                json!({"request_json": request})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(denied.is_err() || denied.unwrap().is_error.unwrap_or(false));
    writer.cancel().await.unwrap();
    reader.cancel().await.unwrap();
    hidden.cancel().await.unwrap();
}
