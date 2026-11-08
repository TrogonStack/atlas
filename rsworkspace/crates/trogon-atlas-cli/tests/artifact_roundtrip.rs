#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    process::{Command, Output},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use prost::Message as _;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::{
    artifacts::{self, sha256_hex},
    client::{connect, connect_with_options, ConnectOptions},
};
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
    completed: Arc<Mutex<Option<pb::Artifact>>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn start() -> Self {
        Self::start_with_completion_loss(false).await
    }

    async fn start_with_completion_loss(drop_response: bool) -> Self {
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
        let store = Arc::new(
            trogon_atlas_store::NatsStore::connect_with(config)
                .await
                .unwrap(),
        );
        let artifacts = ArtifactService::new(
            Arc::new(blobs),
            catalog,
            8 * 1024 * 1024,
            Duration::from_secs(300),
            trogon_atlas_store::Store::artifact_publication_authority(store.as_ref()).unwrap(),
        )
        .unwrap();
        let service = EventModelServiceImpl::try_new(store, ServerMetrics::default())
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
        let lose_response = Arc::new(AtomicBool::new(drop_response));
        let completed = Arc::new(Mutex::new(None));
        let completed_reply = completed.clone();
        *routes.axum_router_mut() =
            routes
                .axum_router_mut()
                .clone()
                .layer(axum::middleware::from_fn(
                    move |request: axum::extract::Request, next: axum::middleware::Next| {
                        let lose_response = lose_response.clone();
                        let completed = completed_reply.clone();
                        async move {
                            let is_completion =
                                request.uri().path().ends_with("/CompleteArtifactUpload");
                            let response = next.run(request).await;
                            if is_completion && lose_response.swap(false, Ordering::SeqCst) {
                                let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                                    .await
                                    .unwrap();
                                let reply =
                                    pb::CompleteArtifactUploadResponse::decode(&body[5..]).unwrap();
                                *completed.lock().unwrap() = Some(
                                    reply.artifact.expect("real completion must publish first"),
                                );
                                tonic::Status::unavailable("completion response lost")
                                    .into_http::<axum::body::Body>()
                            } else {
                                response
                            }
                        }
                    },
                ));
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
            completed,
        }
    }

    async fn cli(&self, token: &str, arguments: &[&str]) -> Output {
        self.cli_with_format(token, arguments, "json").await
    }

    async fn cli_with_format(&self, token: &str, arguments: &[&str], format: &str) -> Output {
        let endpoint = self.endpoint.clone();
        let token = token.to_owned();
        let format = format.to_owned();
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
                    &format,
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

async fn assert_completion_recovery(fixture: &Fixture, details: &Value) {
    let accepted = fixture
        .completed
        .lock()
        .unwrap()
        .clone()
        .expect("server published before losing its reply");
    assert_eq!(details["artifact_upload"]["namespace"], fixture.namespace);
    assert_eq!(
        details["artifact_upload"]["upload_id"],
        accepted.artifact_id
    );
    let mut client = connect(&fixture.endpoint, Some(WRITER), 30).await.unwrap();
    let recovered = client
        .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
            namespace: details["artifact_upload"]["namespace"]
                .as_str()
                .unwrap()
                .to_owned(),
            upload_id: details["artifact_upload"]["upload_id"]
                .as_str()
                .unwrap()
                .to_owned(),
        })
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    assert_eq!(recovered, accepted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sdk_completion_failure_retains_recovery_for_an_already_published_artifact() {
    let fixture = Fixture::start_with_completion_loss(true).await;
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("capture.bin");
    std::fs::write(&input, b"published capture").unwrap();
    let mut client = connect(&fixture.endpoint, Some(WRITER), 30).await.unwrap();
    let error = artifacts::upload_file(
        &client,
        &fixture.namespace,
        &input,
        None,
        Duration::from_secs(30),
    )
    .await
    .unwrap_err();
    let completion = error
        .downcast_ref::<artifacts::UploadCompletionError>()
        .expect("SDK callers must have typed completion recovery");
    let typed_retry = client
        .complete_artifact_upload(completion.recovery.completion_request())
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    assert_eq!(
        typed_retry,
        fixture.completed.lock().unwrap().clone().unwrap()
    );
    let failure = trogon_atlas_client::failure::Failure::classify(&error);
    assert_eq!(
        failure.category,
        trogon_atlas_client::failure::FailureCategory::Unavailable
    );
    assert_eq!(
        failure.next_action,
        trogon_atlas_client::failure::NextAction::Retry
    );
    assert_completion_recovery(&fixture, &failure.details.into_json()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cli_json_completion_failure_retains_recovery_for_an_already_published_artifact() {
    let fixture = Fixture::start_with_completion_loss(true).await;
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("capture.bin");
    std::fs::write(&input, b"published capture").unwrap();
    let output = fixture
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
        .await;
    assert_eq!(output.status.code(), Some(75));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["failure"]["category"], "unavailable");
    assert_eq!(body["failure"]["next_action"], "retry");
    assert_completion_recovery(&fixture, &body["failure"]["details"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cli_text_completion_failure_retains_recovery_for_an_already_published_artifact() {
    let fixture = Fixture::start_with_completion_loss(true).await;
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("capture.bin");
    std::fs::write(&input, b"published capture").unwrap();
    let output = fixture
        .cli_with_format(
            WRITER,
            &[
                "artifacts",
                "upload",
                "--namespace",
                &fixture.namespace,
                "--file",
                input.to_str().unwrap(),
            ],
            "text",
        )
        .await;
    assert_eq!(output.status.code(), Some(75));
    let accepted = fixture
        .completed
        .lock()
        .unwrap()
        .clone()
        .expect("server published before losing its reply");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&fixture.namespace));
    assert!(stderr.contains(&accepted.artifact_id));
    assert!(!stderr.contains("upload_url"));
    assert!(!stderr.contains("X-Amz-"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_sdk_client_uploads_supported_artifacts_larger_than_four_mebibytes() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.bin");
    let content = vec![0x61; 5 * 1024 * 1024];
    std::fs::write(&path, &content).unwrap();
    let mut client = connect(&fixture.endpoint, Some(WRITER), 30).await.unwrap();
    let artifact = artifacts::upload_file(
        &client,
        &fixture.namespace,
        &path,
        None,
        Duration::from_secs(30),
    )
    .await
    .expect("default SDK clients must support the advertised artifact upload limit");
    assert_eq!(artifact.sha256, sha256_hex(&content));
    let unchanged_default = client
        .get_artifact_content(pb::GetArtifactContentRequest {
            namespace: fixture.namespace.clone(),
            artifact_id: artifact.artifact_id,
        })
        .await
        .expect_err("artifact helpers must not widen ordinary caller RPC limits");
    assert_eq!(unchanged_default.code(), tonic::Code::OutOfRange);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_sdk_client_downloads_supported_artifacts_larger_than_four_mebibytes() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("large.bin");
    let content = vec![0x62; 5 * 1024 * 1024];
    std::fs::write(&input, &content).unwrap();
    let writer = connect_with_options(
        &fixture.endpoint,
        Some(WRITER),
        30,
        ConnectOptions {
            max_message_bytes: Some(artifacts::MAX_MESSAGE_BYTES),
            ..ConnectOptions::default()
        },
    )
    .await
    .unwrap();
    let artifact = artifacts::upload_file(
        &writer,
        &fixture.namespace,
        &input,
        None,
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    let reader = connect(&fixture.endpoint, Some(READER), 30).await.unwrap();
    let output = directory.path().join("download.bin");
    artifacts::download_file(&reader, &fixture.namespace, &artifact.artifact_id, &output)
        .await
        .expect("default SDK clients must support the advertised artifact download limit");
    assert_eq!(std::fs::read(output).unwrap(), content);
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
