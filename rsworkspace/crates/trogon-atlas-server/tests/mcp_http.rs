//! The server's `/mcp` endpoint, built from the same `mcp::data_plane` and
//! `mcp::mcp_router` as `main.rs` and driven over real Streamable HTTP,
//! merged onto the same gRPC listener: each MCP request acts as its own
//! `Authorization: Bearer` token, `list_tools` only offers what that
//! caller's role can invoke, and the server's own authentication and
//! authorization refuse everything else.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashSet,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use tokio::net::TcpListener;
use tonic::transport::{server::TcpIncoming, Server};
use tonic_health::pb::{health_client::HealthClient, HealthCheckRequest};
use trogon_atlas_mcp::{
    profile::{ToolProfile, PROFILE_HEADER},
    server::{McpServer, ToolRole, PROFILE_META_KEY, TOOL_POLICIES},
};
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{
    auth::{BearerAuth, RegistryFile, TokenRegistry},
    health::{Health, StoreStatus},
    mcp::{data_plane, mcp_router, AllowedHosts, MCP_PATH},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

const READER_TOKEN: &str = "role-filter-reader-aaaa";
const WRITER_TOKEN: &str = "role-filter-writer-bbbb";
const ADMIN_TOKEN: &str = "role-filter-admin-cccc";

fn authenticated_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.reader]
role = "reader"
tokens = ["{READER_TOKEN}"]

[principals.writer]
role = "writer"
tokens = ["{WRITER_TOKEN}"]

[principals.admin]
role = "admin"
tokens = ["{ADMIN_TOKEN}"]
"#
    );
    let file: RegistryFile = toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

/// The server's gRPC listener with `/mcp` merged onto the same port, exactly
/// as `main.rs` serves them when `--mcp` is set.
struct McpHttp {
    addr: std::net::SocketAddr,
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl McpHttp {
    async fn start(registry: TokenRegistry) -> Self {
        Self::start_with_hosts(registry, "127.0.0.1".parse().unwrap()).await
    }

    async fn start_with_hosts(registry: TokenRegistry, allowed_hosts: AllowedHosts) -> Self {
        Self::start_with(registry, allowed_hosts, None).await
    }

    async fn start_with_profile(registry: TokenRegistry, profile: ToolProfile) -> Self {
        Self::start_with(registry, "127.0.0.1".parse().unwrap(), Some(profile)).await
    }

    async fn start_with(
        registry: TokenRegistry,
        allowed_hosts: AllowedHosts,
        profile: Option<ToolProfile>,
    ) -> Self {
        let store = trogon_atlas_testsupport::shared().await.store().await;
        let service = EventModelServiceServer::new(
            EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap(),
        );
        let data_plane = data_plane(service, BearerAuth::new(registry));
        let health = Health::new().await;
        health.set_store(StoreStatus::Reachable).await;
        let (drain_tx, _drain_rx) = tokio::sync::watch::channel(0i64);
        let inflight = InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain_tx));

        let mut routes = tonic::service::Routes::builder();
        routes.add_service(health.grpc_service());
        routes.add_service(data_plane.clone());
        let mut routes = routes.routes();
        let mcp = mcp_router(
            data_plane,
            RpcMetricsLayer::new(ServerMetrics::default()),
            inflight,
            Some(Duration::from_secs(30)),
            4 * 1024 * 1024,
            allowed_hosts,
            profile,
        )
        .unwrap();
        let merged = routes
            .axum_router_mut()
            .clone()
            .merge(health.router())
            .merge(mcp);
        *routes.axum_router_mut() = merged;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from(listener);
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        health.mark_live().await;
        let task = tokio::spawn(
            Server::builder()
                .accept_http1(true)
                .add_routes(routes)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                }),
        );
        Self {
            addr,
            shutdown,
            task,
        }
    }

    /// An MCP client whose every request carries `auth_token` as its own
    /// `Authorization: Bearer` header.
    async fn client(&self, auth_token: Option<&str>) -> McpClient {
        self.try_client(auth_token, None).await.unwrap()
    }

    /// An MCP client whose every request also names `profile` in
    /// `x-trogon-atlas-profile`.
    async fn try_client(
        &self,
        auth_token: Option<&str>,
        profile: Option<&str>,
    ) -> Result<McpClient, rmcp::service::ClientInitializeError> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(format!(
            "http://{}{MCP_PATH}",
            self.addr
        ));
        if let Some(token) = auth_token {
            config = config.auth_header(token);
        }
        if let Some(profile) = profile {
            config = config.custom_headers(std::collections::HashMap::from([(
                http::HeaderName::from_static(PROFILE_HEADER),
                http::HeaderValue::from_str(profile).unwrap(),
            )]));
        }
        ().serve(StreamableHttpClientTransport::from_config(config))
            .await
    }

    /// Checks gRPC health on the same port `/mcp` is served from.
    async fn grpc_health(&self) -> tonic_health::pb::health_check_response::ServingStatus {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", self.addr))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let response = HealthClient::new(channel)
            .check(HealthCheckRequest {
                service: String::new(),
            })
            .await
            .unwrap();
        response.into_inner().status.try_into().unwrap()
    }

    async fn shut_down(self) {
        let _ = self.shutdown.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("the gRPC listener drains on shutdown")
            .unwrap()
            .unwrap();
    }
}

fn tool_names(tools: &[rmcp::model::Tool]) -> HashSet<String> {
    tools.iter().map(|t| t.name.to_string()).collect()
}

fn who_am_i_body(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    assert!(
        !result.is_error.unwrap_or(false),
        "who_am_i must succeed: {result:?}"
    );
    serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap()
}

fn tool_call(name: &'static str, arguments: serde_json::Value) -> CallToolRequestParams {
    let serde_json::Value::Object(arguments) = arguments else {
        panic!("tool arguments must be a JSON object");
    };
    CallToolRequestParams::new(name).with_arguments(arguments)
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_sees_only_reader_min_role_tools() {
    let http = McpHttp::start(authenticated_registry()).await;
    let client = http.client(Some(READER_TOKEN)).await;

    let names = tool_names(&client.list_all_tools().await.unwrap());
    for policy in TOOL_POLICIES {
        assert_eq!(
            names.contains(policy.tool),
            policy.min_role == ToolRole::Reader,
            "reader's tool list disagrees with the policy table for {}",
            policy.tool
        );
    }
    assert!(names.contains("who_am_i"));
    assert!(!names.contains("put_entity"));

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn writer_sees_no_admin_only_tools() {
    let http = McpHttp::start(authenticated_registry()).await;
    let client = http.client(Some(WRITER_TOKEN)).await;

    let names = tool_names(&client.list_all_tools().await.unwrap());
    assert!(TOOL_POLICIES.iter().any(|p| p.min_role == ToolRole::Admin));
    for policy in TOOL_POLICIES {
        assert_eq!(
            names.contains(policy.tool),
            policy.min_role <= ToolRole::Writer,
            "writer's tool list disagrees with the policy table for {}",
            policy.tool
        );
    }

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_sees_every_tool() {
    let http = McpHttp::start(authenticated_registry()).await;
    let client = http.client(Some(ADMIN_TOKEN)).await;

    let names = tool_names(&client.list_all_tools().await.unwrap());
    assert_eq!(names.len(), TOOL_POLICIES.len());
    for policy in TOOL_POLICIES {
        assert!(
            names.contains(policy.tool),
            "admin is missing {}",
            policy.tool
        );
    }

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_callers_each_act_as_themselves() {
    let http = McpHttp::start(authenticated_registry()).await;
    let reader = http.client(Some(READER_TOKEN)).await;
    let admin = http.client(Some(ADMIN_TOKEN)).await;

    let (reader_tools, admin_tools, reader_me, admin_me) = tokio::join!(
        reader.list_all_tools(),
        admin.list_all_tools(),
        reader.call_tool(CallToolRequestParams::new("who_am_i")),
        admin.call_tool(CallToolRequestParams::new("who_am_i")),
    );
    let reader_tools = tool_names(&reader_tools.unwrap());
    let admin_tools = tool_names(&admin_tools.unwrap());
    assert!(!reader_tools.contains("merge_branch"));
    assert!(admin_tools.contains("merge_branch"));
    assert!(reader_tools.len() < admin_tools.len());

    let reader_me = who_am_i_body(&reader_me.unwrap());
    assert_eq!(reader_me["role"], "reader");
    assert_eq!(reader_me["name"], "reader");
    assert_eq!(reader_me["anonymous"], false);
    assert_eq!(who_am_i_body(&admin_me.unwrap())["role"], "admin");

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn readers_can_compile_and_resolve_types() {
    let http = McpHttp::start(authenticated_registry()).await;
    let client = http.client(Some(READER_TOKEN)).await;

    let library = serde_json::json!({
        "id": {"namespace": "role-filter-types", "slug": "acme.orders", "version": 1},
        "files": [{
            "path": "acme/orders/v1/orders.proto",
            "content": "syntax = \"proto3\";\npackage acme.orders.v1;\nmessage OrderPlaced { strin id = 1; }\n",
        }],
    });
    let result = client
        .call_tool(tool_call(
            "compile_type_library",
            serde_json::json!({ "library_json": library.to_string() }),
        ))
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "a reader may compile without writing: {result:?}"
    );

    let result = client
        .call_tool(tool_call(
            "resolve_type",
            serde_json::json!({
                "namespace": "role-filter-types",
                "type_url": "type.googleapis.com/google.protobuf.Timestamp",
            }),
        ))
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "a reader may resolve a type: {result:?}"
    );

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_denies_a_reader_a_tool_it_was_never_offered() {
    let http = McpHttp::start(authenticated_registry()).await;
    let reader = http.client(Some(READER_TOKEN)).await;
    let admin = http.client(Some(ADMIN_TOKEN)).await;
    let branch = "reader/should-not-exist";

    assert_eq!(
        who_am_i_body(
            &reader
                .call_tool(CallToolRequestParams::new("who_am_i"))
                .await
                .unwrap()
        )["role"],
        "reader"
    );
    let err = reader
        .call_tool(tool_call(
            "create_branch",
            serde_json::json!({ "name": branch }),
        ))
        .await
        .expect_err("authorization must refuse a reader creating a branch");
    assert!(
        err.to_string().contains("unauthorized"),
        "expected an authorization error, got: {err}"
    );

    let branches = admin
        .call_tool(CallToolRequestParams::new("list_branches"))
        .await
        .unwrap();
    assert!(
        !format!("{branches:?}").contains(branch),
        "the refused branch must not exist: {branches:?}"
    );

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_authenticated_server_refuses_a_caller_without_a_token() {
    let http = McpHttp::start(authenticated_registry()).await;
    let client = http.client(None).await;

    let err = client
        .call_tool(CallToolRequestParams::new("who_am_i"))
        .await
        .expect_err("an anonymous caller must be refused by an authenticated server");
    assert!(
        err.to_string().contains("unauthorized"),
        "expected an unauthorized error, got: {err}"
    );

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_anonymous_server_lets_a_caller_without_a_token_in() {
    let http = McpHttp::start(TokenRegistry::anonymous()).await;
    let client = http.client(None).await;

    let names = tool_names(&client.list_all_tools().await.unwrap());
    assert_eq!(names.len(), TOOL_POLICIES.len());
    let me = who_am_i_body(
        &client
            .call_tool(CallToolRequestParams::new("who_am_i"))
            .await
            .unwrap(),
    );
    assert_eq!(me["anonymous"], true);

    http.shut_down().await;
}

/// Sends one raw HTTP/1.1 request and returns the status code and the
/// whole response text.
async fn raw_http(addr: std::net::SocketAddr, request: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("an HTTP status line");
    (status, response)
}

fn tools_list(host: &str, authorization: &str) -> String {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: {host}\r\nAuthorization: {authorization}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         MCP-Protocol-Version: 2025-06-18\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_bearer_authorization_header_is_rejected() {
    let http = McpHttp::start(authenticated_registry()).await;

    let (_, response) = raw_http(http.addr, &tools_list("127.0.0.1", "Basic dXNlcjpwYXNz")).await;
    assert!(
        response.contains("Bearer <token>"),
        "a non-bearer Authorization header must be rejected, got: {response}"
    );

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_outside_the_allow_list_is_refused() {
    let http = McpHttp::start(authenticated_registry()).await;

    let authorization = format!("Bearer {ADMIN_TOKEN}");
    let (status, response) =
        raw_http(http.addr, &tools_list("rebound.example", &authorization)).await;
    assert_eq!(status, 403, "got: {response}");
    assert_eq!(
        http.grpc_health().await,
        tonic_health::pb::health_check_response::ServingStatus::Serving,
        "the Host guard must not reach gRPC on the same port"
    );
    let (status, response) = raw_http(http.addr, &tools_list("127.0.0.1", &authorization)).await;
    assert_eq!(status, 200, "got: {response}");

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bracketed_ipv6_loopback_host_matches_the_default_allow_list() {
    let http = McpHttp::start_with_hosts(
        authenticated_registry(),
        "localhost,127.0.0.1,::1".parse().unwrap(),
    )
    .await;

    let authorization = format!("Bearer {ADMIN_TOKEN}");
    let host = format!("[::1]:{}", http.addr.port());
    let (status, response) = raw_http(http.addr, &tools_list(&host, &authorization)).await;
    assert_eq!(status, 200, "got: {response}");

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn grpc_and_mcp_share_the_same_port() {
    let http = McpHttp::start(authenticated_registry()).await;
    let client = http.client(Some(ADMIN_TOKEN)).await;

    assert_eq!(
        http.grpc_health().await,
        tonic_health::pb::health_check_response::ServingStatus::Serving
    );
    let names = tool_names(&client.list_all_tools().await.unwrap());
    assert_eq!(names.len(), TOOL_POLICIES.len());
    let who_am_i = who_am_i_body(
        &client
            .call_tool(CallToolRequestParams::new("who_am_i"))
            .await
            .unwrap(),
    );
    assert_eq!(who_am_i["role"], "admin");

    http.shut_down().await;
}

/// The tools a caller of `role` should see under `profile`: the role's
/// tools intersected with the profile's.
fn expected_tools(role: ToolRole, profile: ToolProfile) -> HashSet<String> {
    McpServer::registered_tools()
        .into_iter()
        .filter(|tool| profile.admits(tool))
        .filter(|tool| {
            TOOL_POLICIES
                .iter()
                .any(|p| p.tool == tool.name && p.min_role <= role)
        })
        .map(|tool| tool.name.to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profile_header_lists_the_role_and_profile_intersection() {
    let http = McpHttp::start(authenticated_registry()).await;

    for (token, role) in [
        (READER_TOKEN, ToolRole::Reader),
        (WRITER_TOKEN, ToolRole::Writer),
    ] {
        for &profile in ToolProfile::ALL {
            let client = http
                .try_client(Some(token), Some(profile.as_str()))
                .await
                .unwrap();
            let names = tool_names(&client.list_all_tools().await.unwrap());
            assert_eq!(
                names,
                expected_tools(role, profile),
                "{role:?} under {profile}"
            );
        }
    }
    let reader_authoring = expected_tools(ToolRole::Reader, ToolProfile::Authoring);
    assert!(!reader_authoring.contains("put_entity_json"));
    assert!(reader_authoring.contains("get_entity_schema"));

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_outside_the_profile_is_refused_at_dispatch() {
    let http = McpHttp::start(authenticated_registry()).await;
    let client = http
        .try_client(Some(ADMIN_TOKEN), Some("read-only"))
        .await
        .unwrap();

    let err = client
        .call_tool(tool_call(
            "register_namespace",
            serde_json::json!({ "namespace": "profile-hidden" }),
        ))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("not in the `read-only` tool profile"),
        "{err}"
    );
    let me = who_am_i_body(
        &client
            .call_tool(CallToolRequestParams::new("who_am_i"))
            .await
            .unwrap(),
    );
    assert_eq!(me["role"], "admin");

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_profile_is_refused() {
    let http = McpHttp::start(authenticated_registry()).await;

    let err = http.try_client(Some(ADMIN_TOKEN), Some("everything")).await;
    let Err(err) = err else {
        panic!("an unknown profile must not connect");
    };
    assert!(
        format!("{err:?}").contains("unknown tool profile"),
        "{err:?}"
    );

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_operator_profile_fixes_every_session_and_is_reported() {
    let http = McpHttp::start_with_profile(authenticated_registry(), ToolProfile::Review).await;

    let admin = http.client(Some(ADMIN_TOKEN)).await;
    let names = tool_names(&admin.list_all_tools().await.unwrap());
    assert_eq!(names, expected_tools(ToolRole::Admin, ToolProfile::Review));
    let info = admin.peer_info().unwrap();
    assert!(
        info.instructions
            .as_deref()
            .unwrap()
            .contains("Active tool profile: review."),
        "{info:?}"
    );
    assert_eq!(
        info.meta.as_ref().unwrap().0[PROFILE_META_KEY],
        serde_json::json!("review")
    );

    let same = http
        .try_client(Some(ADMIN_TOKEN), Some("review"))
        .await
        .unwrap();
    assert_eq!(tool_names(&same.list_all_tools().await.unwrap()), names);
    let err = http.try_client(Some(ADMIN_TOKEN), Some("authoring")).await;
    let Err(err) = err else {
        panic!("a request cannot swap the operator's profile");
    };
    assert!(
        format!("{err:?}").contains("runs the `review` tool profile"),
        "{err:?}"
    );

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_sessions_keep_their_own_profile() {
    let http = McpHttp::start(authenticated_registry()).await;
    let read_only = http
        .try_client(Some(WRITER_TOKEN), Some("read-only"))
        .await
        .unwrap();
    let authoring = http
        .try_client(Some(WRITER_TOKEN), Some("authoring"))
        .await
        .unwrap();
    let unprofiled = http.client(Some(WRITER_TOKEN)).await;

    let (read_only_tools, authoring_tools, unprofiled_tools) = tokio::join!(
        read_only.list_all_tools(),
        authoring.list_all_tools(),
        unprofiled.list_all_tools(),
    );
    assert_eq!(
        tool_names(&read_only_tools.unwrap()),
        expected_tools(ToolRole::Writer, ToolProfile::ReadOnly)
    );
    let authoring_tools = tool_names(&authoring_tools.unwrap());
    assert_eq!(
        authoring_tools,
        expected_tools(ToolRole::Writer, ToolProfile::Authoring)
    );
    assert!(authoring_tools.contains("apply_manifests"));
    let unprofiled_tools = tool_names(&unprofiled_tools.unwrap());
    assert!(authoring_tools.is_subset(&unprofiled_tools));
    assert!(unprofiled_tools.contains("delete_entity"));
    assert!(read_only.peer_info().unwrap().meta.is_some());
    assert!(unprofiled.peer_info().unwrap().meta.is_none());

    http.shut_down().await;
}
