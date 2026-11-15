#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    path::Path,
    process::Command,
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
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

const WRITER: &str = "glossary-lean-writer-public-fixture";
const READER: &str = "glossary-lean-reader-public-fixture";
const NAMESPACE: &str = "glossary-lean";
const BRANCH: &str = "glossary-lean-proposal";
const DEFINITION: &str = "An old reservation label that current internal code must replace.";
const CODE: &str =
    "let é = legacy_booking + LegacyBooking + legacyBooking + LEGACY_BOOKING + xlegacy_booking;";
type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn serve() -> String {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "writer": {"role": "writer", "tokens": [WRITER]},
            "reader": {"role": "reader", "tokens": [READER]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceServer::new(
        EventModelServiceImpl::try_new(
            Arc::new(trogon_atlas_testsupport::shared().await.store().await),
            ServerMetrics::default(),
        )
        .unwrap(),
    );
    let plane = data_plane(service, BearerAuth::new(registry));
    let (drain, _) = tokio::sync::watch::channel(0i64);
    let mcp = mcp_router(
        plane.clone(),
        RpcMetricsLayer::new(ServerMetrics::default()),
        InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
        Some(Duration::from_secs(30)),
        4 * 1024 * 1024,
        "127.0.0.1".parse().unwrap(),
        None,
    )
    .unwrap();
    let mut routes = tonic::service::Routes::builder();
    routes.add_service(plane);
    let mut routes = routes.routes();
    let merged = routes.axum_router_mut().clone().merge(mcp);
    *routes.axum_router_mut() = merged;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .accept_http1(true)
            .add_routes(routes)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{address}")
}

fn command(endpoint: &str, token: &str, branch: Option<&str>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"));
    command
        .args(["--endpoint", endpoint])
        .env("TROGON_ATLAS_AUTH_TOKEN", token);
    if let Some(branch) = branch {
        command.args(["--branch", branch]);
    }
    command
}

fn successful(endpoint: &str, token: &str, branch: Option<&str>, arguments: &[&str]) -> String {
    let output = command(endpoint, token, branch)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn glossary(
    endpoint: &str,
    token: &str,
    branch: Option<&str>,
    action: &str,
    request: &Value,
) -> Value {
    serde_json::from_str(&successful(
        endpoint,
        token,
        branch,
        &["glossary", action, "--request", &request.to_string()],
    ))
    .unwrap()
}

fn reference(slug: &str) -> Value {
    json!({"id": {"namespace": NAMESPACE, "slug": slug, "version": "1"}})
}

fn selection() -> Value {
    json!({"namespaces": [NAMESPACE], "audience": "internal"})
}

fn apply(endpoint: &str, branch: Option<&str>, path: &Path, kind: &str, slug: &str, spec: &Value) {
    std::fs::write(
        path,
        json!({"apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": kind, "metadata": {"namespace": NAMESPACE, "name": slug}, "spec": spec})
        .to_string(),
    )
    .unwrap();
    successful(
        endpoint,
        WRITER,
        branch,
        &["apply", "-f", path.to_str().unwrap()],
    );
}

fn author(
    endpoint: &str,
    branch: Option<&str>,
    path: &Path,
    source: &Value,
    slug: &str,
    title: &str,
    policy: &Value,
) {
    apply(
        endpoint,
        branch,
        path,
        "Term",
        slug,
        &json!({"title": title, "doc": DEFINITION,
        "glossary": {"evidence": [{"document": source["document"], "ownerUid": source["ownerUid"],
            "contentDigest": source["contentDigest"], "locator": "paragraph:1", "quote": source["text"]}],
            "generationRevision": "lean-public-readers-v1", "audience": "internal",
            "nextReviewOn": {"value": "9999-12-31"}, "policy": policy,
            "usagePeriod": {"validFrom": {"value": "2026-11-01"}, "validUntil": {"value": "2026-12-01"}}}}),
    );
    let draft = glossary(
        endpoint,
        WRITER,
        branch,
        "get",
        &json!({"term": reference(slug)}),
    );
    glossary(
        endpoint,
        WRITER,
        branch,
        "review",
        &json!({"term": reference(slug), "ifMatch": draft["etag"],
        "disposition": "GLOSSARY_REVIEW_DISPOSITION_ACCEPTED", "reason": "Synthetic User protocol acceptance for reader tests."}),
    );
}

fn seed(endpoint: &str, path: &Path) -> Value {
    apply(
        endpoint,
        None,
        path,
        "Event",
        "context",
        &json!({"title": "Retained language context"}),
    );
    let content =
        "Legacy booking is the old reservation label. Pickup slot is its current replacement. Branch marker is an old reservation label proposed only in a branch. "
            .repeat(64);
    apply(
        endpoint,
        None,
        path,
        "Document",
        "source",
        &json!({
            "title": {"value": "Retained reservation wording"}, "subjects": [{"subject": {"entity": {
                "kind": "ENTITY_KIND_EVENT", "id": reference("context")["id"]}}}],
            "retained": {"mediaType": {"value": "text/plain"}, "content": content,
                "digest": {"sha256": trogon_atlas_core::specification::content_digest(&content).sha256}}
        }),
    );
    let sources = glossary(
        endpoint,
        WRITER,
        None,
        "sources",
        &json!({"documents": [reference("source")]}),
    );
    let source = sources["sources"][0].clone();
    author(
        endpoint,
        None,
        path,
        &source,
        "pickup-slot",
        "Pickup slot",
        &json!({"usage": "GLOSSARY_USAGE_ALLOWED"}),
    );
    author(
        endpoint,
        None,
        path,
        &source,
        "legacy-booking",
        "Legacy booking",
        &json!({
        "usage": "GLOSSARY_USAGE_PROHIBITED", "preferredTerm": reference("pickup-slot"), "rationale": "Use pickup slot."}),
    );
    source
}

fn assert_pointer(pointer: &Value) {
    let object = pointer.as_object().unwrap();
    assert_eq!(object.len(), 4);
    for key in ["term", "title", "audience", "state"] {
        assert!(object.contains_key(key));
    }
    assert_eq!(pointer["audience"], "internal");
    assert_eq!(pointer["state"], "GLOSSARY_ENTRY_STATE_REVIEWED");
    assert_eq!(pointer["term"]["id"]["version"], "1");
}

fn assert_definition(lean: &Value, full: &Value) {
    assert_pointer(&lean["entry"]);
    assert_eq!(lean["entry"]["term"], reference("legacy-booking"));
    assert_eq!(lean["definition"], DEFINITION);
    assert_eq!(lean["ownerUid"], full["entity"]["system"]["uid"]);
    assert_eq!(lean["etag"], full["etag"]);
    assert_ne!(lean["snapshotRevision"].as_str().unwrap(), "");
    assert_eq!(lean["policy"], full["entity"]["term"]["glossary"]["policy"]);
    assert_eq!(lean["evidence"], full["evidence"]);
    assert_eq!(lean.get("incomplete"), full.get("incomplete"));
    assert_eq!(
        lean["usagePeriod"],
        full["entity"]["term"]["glossary"]["usagePeriod"]
    );
    assert_eq!(lean["usagePeriod"]["validFrom"]["value"], "2026-11-01");
    assert_eq!(lean["usagePeriod"]["validUntil"]["value"], "2026-12-01");
    for key in lean.as_object().unwrap().keys() {
        assert!(
            [
                "entry",
                "definition",
                "policy",
                "evidence",
                "incomplete",
                "usagePeriod",
                "ownerUid",
                "etag",
                "snapshotRevision"
            ]
            .contains(&key.as_str()),
            "unexpected {key}"
        );
    }
    assert!(lean.to_string().len() * 2 < full.to_string().len());
}

fn usage_request(text: &str) -> Value {
    json!({"selection": selection(), "text": text, "textRevision": "code-source-v1", "on": {"value": "2026-11-13"}})
}

fn assert_period_boundary(response: &Value, prohibited: bool) {
    let findings = response
        .get("findings")
        .cloned()
        .unwrap_or_else(|| json!([]));
    assert_eq!(findings.as_array().unwrap().len(), usize::from(prohibited));
    if prohibited {
        assert_eq!(findings[0]["term"], reference("pickup-slot"));
        assert_eq!(findings[0]["matched"], "pickup_slot");
        assert_eq!(findings[0]["usage"], "GLOSSARY_USAGE_PROHIBITED");
    }
}

fn assert_code_findings(response: &Value, text: &str, expected: &[&str]) {
    assert_eq!(response["textRevision"], "code-source-v1");
    let findings = response["findings"].as_array().unwrap();
    assert_eq!(findings.len(), expected.len());
    let mut actual = Vec::new();
    for finding in findings {
        let start = usize::try_from(finding["range"]["start"].as_u64().unwrap()).unwrap();
        let end = usize::try_from(finding["range"]["end"].as_u64().unwrap()).unwrap();
        assert_eq!(&text[start..end], finding["matched"].as_str().unwrap());
        assert_eq!(finding["term"], reference("legacy-booking"));
        assert_eq!(finding["preferredTerm"], reference("pickup-slot"));
        actual.push((start, finding["matched"].as_str().unwrap()));
    }
    actual.sort_unstable();
    assert_eq!(
        actual
            .iter()
            .map(|(_, matched)| *matched)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        actual,
        expected
            .iter()
            .map(|matched| (text.find(matched).unwrap(), *matched))
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_pointers_definitions_and_code_checks_are_lean_scoped_and_read_only() {
    let endpoint = serve().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("language.yaml");
    let source = seed(&endpoint, &path);
    let full = glossary(
        &endpoint,
        READER,
        None,
        "get",
        &json!({"term": reference("legacy-booking")}),
    );
    let rich_index = glossary(
        &endpoint,
        READER,
        None,
        "list",
        &json!({"selection": selection()}),
    );
    let pointers = glossary(
        &endpoint,
        READER,
        None,
        "pointers",
        &json!({"selection": selection()}),
    );
    assert_eq!(pointers["entries"].as_array().unwrap().len(), 2);
    assert_ne!(pointers["snapshotRevision"].as_str().unwrap(), "");
    for pointer in pointers["entries"].as_array().unwrap() {
        assert_pointer(pointer);
    }
    assert!(pointers.to_string().len() < rich_index.to_string().len());
    let first = glossary(
        &endpoint,
        READER,
        None,
        "pointers",
        &json!({"selection": selection(), "pageSize": 1}),
    );
    let second = glossary(
        &endpoint,
        READER,
        None,
        "pointers",
        &json!({"selection": selection(), "pageSize": 1,
        "pageToken": first["nextPageToken"]}),
    );
    assert_eq!(first["entries"].as_array().unwrap().len(), 1);
    assert_eq!(second["entries"].as_array().unwrap().len(), 1);
    assert_ne!(first["nextPageToken"].as_str().unwrap(), "");
    assert_eq!(first["snapshotRevision"], second["snapshotRevision"]);
    assert_ne!(first["entries"][0]["term"], second["entries"][0]["term"]);
    assert_eq!(second.get("nextPageToken"), None);
    let lean = glossary(
        &endpoint,
        READER,
        None,
        "definition",
        &json!({"term": reference("legacy-booking")}),
    );
    assert_definition(&lean, &full);
    let prose = glossary(&endpoint, READER, None, "check", &usage_request(CODE));
    assert!(prose["findings"].as_array().is_none_or(Vec::is_empty));
    let checked = glossary(&endpoint, READER, None, "check-code", &usage_request(CODE));
    assert_code_findings(
        &checked,
        CODE,
        &[
            "legacy_booking",
            "LegacyBooking",
            "legacyBooking",
            "LEGACY_BOOKING",
        ],
    );
    for (on, prohibited) in [
        ("2026-10-31", true),
        ("2026-11-01", false),
        ("2026-11-30", false),
        ("2026-12-01", true),
    ] {
        let mut request = usage_request("pickup_slot");
        request["on"]["value"] = json!(on);
        assert_period_boundary(
            &glossary(&endpoint, READER, None, "check-code", &request),
            prohibited,
        );
    }
    let quoted = format!("{CODE} \"legacy_booking\"");
    let start = quoted.find("LegacyBooking").unwrap();
    let mut excluded = usage_request(&quoted);
    excluded["skipQuoted"] = json!(true);
    excluded["exclusions"] = json!([{"start": start, "end": start + "LegacyBooking".len()}]);
    assert_code_findings(
        &glossary(&endpoint, READER, None, "check-code", &excluded),
        &quoted,
        &["legacy_booking", "legacyBooking", "LEGACY_BOOKING"],
    );
    let denied = command(&endpoint, READER, None)
        .args([
            "glossary",
            "pointers",
            "--request",
            &json!({"selection": {"namespaces": [NAMESPACE], "includeUnreviewed": true}})
                .to_string(),
        ])
        .output()
        .unwrap();
    assert_eq!(denied.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&denied.stderr)
        .contains("glossary pointers require published selection"));
    assert_eq!(
        glossary(
            &endpoint,
            READER,
            None,
            "get",
            &json!({"term": reference("legacy-booking")})
        ),
        full
    );
    successful(&endpoint, WRITER, None, &["branch", "create", BRANCH]);
    author(
        &endpoint,
        Some(BRANCH),
        &path,
        &source,
        "branch-only",
        "Branch marker",
        &json!({"usage": "GLOSSARY_USAGE_PROHIBITED", "preferredTerm": reference("pickup-slot"),
            "rationale": "Use pickup slot instead of the proposed branch-only label."}),
    );
    let branch = glossary(
        &endpoint,
        READER,
        Some(BRANCH),
        "pointers",
        &json!({"selection": selection()}),
    );
    assert_eq!(branch["entries"].as_array().unwrap().len(), 3);
    assert_eq!(
        glossary(
            &endpoint,
            READER,
            None,
            "pointers",
            &json!({"selection": selection()})
        ),
        pointers
    );
    assert_eq!(
        glossary(
            &endpoint,
            READER,
            Some(BRANCH),
            "definition",
            &json!({"term": reference("branch-only")})
        )["definition"],
        DEFINITION
    );
    assert!(!command(&endpoint, READER, None)
        .args([
            "glossary",
            "definition",
            "--request",
            &json!({"term": reference("branch-only")}).to_string()
        ])
        .output()
        .unwrap()
        .status
        .success());
    let baseline_code = glossary(
        &endpoint,
        READER,
        None,
        "check-code",
        &usage_request("branch_marker"),
    );
    assert!(baseline_code["findings"]
        .as_array()
        .is_none_or(Vec::is_empty));
    let branch_code = glossary(
        &endpoint,
        READER,
        Some(BRANCH),
        "check-code",
        &usage_request("branch_marker"),
    );
    assert_eq!(branch_code["findings"].as_array().unwrap().len(), 1);
    assert_eq!(branch_code["findings"][0]["term"], reference("branch-only"));
    assert_eq!(branch_code["findings"][0]["matched"], "branch_marker");
    println!("CLI glossary canonical UTF-8 bytes: rich-index={}, pointers={}, full-term={}, definition={}", rich_index.to_string().len(), pointers.to_string().len(), full.to_string().len(), lean.to_string().len());
}

async fn connect(endpoint: &str, profile: &str, branch: Option<&str>) -> McpClient {
    let mut headers = HashMap::new();
    headers.insert(
        "x-trogon-atlas-profile".parse().unwrap(),
        profile.parse().unwrap(),
    );
    if let Some(branch) = branch {
        headers.insert(
            "x-trogon-atlas-branch".parse().unwrap(),
            branch.parse().unwrap(),
        );
    }
    let config = StreamableHttpClientTransportConfig::with_uri(format!("{endpoint}/mcp"))
        .auth_header(READER)
        .custom_headers(headers);
    ().serve(StreamableHttpClientTransport::from_config(config))
        .await
        .unwrap()
}

async fn call(client: &McpClient, name: &'static str, request: &Value) -> Value {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(
                json!({"request_json": request})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{name}: {result:?}");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_mcp_lean_readers_preserve_profiles_branch_scope_and_code_spans() {
    let endpoint = serve().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("language.yaml");
    let source = seed(&endpoint, &path);
    let client = connect(&endpoint, "read-only", None).await;
    let pointers = call(
        &client,
        "list_glossary_pointers",
        &json!({"selection": selection()}),
    )
    .await;
    assert_eq!(pointers["entries"].as_array().unwrap().len(), 2);
    assert_ne!(pointers["snapshotRevision"].as_str().unwrap(), "");
    let rich = call(
        &client,
        "list_glossary_entries",
        &json!({"selection": selection()}),
    )
    .await;
    assert!(pointers.to_string().len() < rich.to_string().len());
    for pointer in pointers["entries"].as_array().unwrap() {
        assert_pointer(pointer);
    }
    let full = call(
        &client,
        "get_glossary_term",
        &json!({"term": reference("legacy-booking")}),
    )
    .await;
    let lean = call(
        &client,
        "get_glossary_definition",
        &json!({"term": reference("legacy-booking")}),
    )
    .await;
    assert_definition(&lean, &full);
    let prose = call(&client, "check_glossary_usage", &usage_request(CODE)).await;
    assert!(prose["findings"].as_array().is_none_or(Vec::is_empty));
    assert_code_findings(
        &call(&client, "check_glossary_code_usage", &usage_request(CODE)).await,
        CODE,
        &[
            "legacy_booking",
            "LegacyBooking",
            "legacyBooking",
            "LEGACY_BOOKING",
        ],
    );
    for (on, prohibited) in [
        ("2026-10-31", true),
        ("2026-11-01", false),
        ("2026-11-30", false),
        ("2026-12-01", true),
    ] {
        let mut request = usage_request("pickup_slot");
        request["on"]["value"] = json!(on);
        assert_period_boundary(
            &call(&client, "check_glossary_code_usage", &request).await,
            prohibited,
        );
    }
    assert_eq!(
        call(
            &client,
            "get_glossary_term",
            &json!({"term": reference("legacy-booking")})
        )
        .await,
        full
    );
    let malformed = client
        .call_tool(
            CallToolRequestParams::new("get_glossary_definition").with_arguments(
                json!({"request_json": {"unknown": true}})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(matches!(malformed, Err(rmcp::ServiceError::McpError(error))
        if error.code == rmcp::model::ErrorCode::INVALID_PARAMS && error.message.contains("unknown")));
    let denied = client
        .call_tool(
            CallToolRequestParams::new("list_glossary_pointers").with_arguments(
                json!({"request_json": {"selection": {"includeUnreviewed": true}}})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(matches!(denied, Err(rmcp::ServiceError::McpError(error))
        if error.code == rmcp::model::ErrorCode::INVALID_PARAMS
            && error.message.contains("glossary pointers require published selection")));
    client.cancel().await.unwrap();
    for profile in [
        "authoring",
        "repair",
        "review",
        "source-integration",
        "read-only",
    ] {
        let client = connect(&endpoint, profile, None).await;
        let tools = client.list_all_tools().await.unwrap();
        for name in [
            "list_glossary_pointers",
            "get_glossary_definition",
            "check_glossary_code_usage",
        ] {
            let tool = tools.iter().find(|tool| tool.name == name).unwrap();
            assert_eq!(
                tool.annotations.as_ref().unwrap().read_only_hint,
                Some(true)
            );
        }
        assert!(!tools
            .iter()
            .any(|tool| tool.name == "review_glossary_term" || tool.name == "put_entity_json"));
        client.cancel().await.unwrap();
    }
    successful(&endpoint, WRITER, None, &["branch", "create", BRANCH]);
    author(
        &endpoint,
        Some(BRANCH),
        &path,
        &source,
        "branch-only",
        "Branch marker",
        &json!({"usage": "GLOSSARY_USAGE_PROHIBITED", "preferredTerm": reference("pickup-slot"),
            "rationale": "Use pickup slot instead of the proposed branch-only label."}),
    );
    let branch = connect(&endpoint, "read-only", Some(BRANCH)).await;
    let listed = call(
        &branch,
        "list_glossary_pointers",
        &json!({"selection": selection()}),
    )
    .await;
    assert_eq!(listed["entries"].as_array().unwrap().len(), 3);
    assert_eq!(
        call(
            &branch,
            "get_glossary_definition",
            &json!({"term": reference("branch-only")})
        )
        .await["definition"],
        DEFINITION
    );
    let branch_code = call(
        &branch,
        "check_glossary_code_usage",
        &usage_request("branch_marker"),
    )
    .await;
    assert_eq!(branch_code["findings"].as_array().unwrap().len(), 1);
    assert_eq!(branch_code["findings"][0]["term"], reference("branch-only"));
    assert_eq!(branch_code["findings"][0]["matched"], "branch_marker");
    branch.cancel().await.unwrap();
    let baseline = connect(&endpoint, "read-only", None).await;
    let baseline_code = call(
        &baseline,
        "check_glossary_code_usage",
        &usage_request("branch_marker"),
    )
    .await;
    assert!(baseline_code["findings"]
        .as_array()
        .is_none_or(Vec::is_empty));
    baseline.cancel().await.unwrap();
    println!("HTTP MCP glossary canonical UTF-8 bytes: rich-index={}, pointers={}, full-term={}, definition={}", rich.to_string().len(), pointers.to_string().len(), full.to_string().len(), lean.to_string().len());
}

#[test]
fn new_cli_readers_reject_unknown_fields_before_connecting() {
    for action in ["pointers", "definition", "check-code"] {
        let output = command("http://127.0.0.1:1", READER, None)
            .args(["glossary", action, "--request", "{\"unknown\":true}"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(5), "{action}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown"));
    }
}
