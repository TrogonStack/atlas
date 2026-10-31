#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tonic::Request;
use trogon_atlas_proto::{
    self as pb,
    event_model_service_server::{EventModelService, EventModelServiceServer},
};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

const ADMIN: &str = "claims-mcp-admin";
const WRITER: &str = "claims-mcp-writer";
const READER: &str = "claims-mcp-reader";
const HIDDEN: &str = "claims-mcp-hidden";
const SPECIFICATION_VERSION: &str = "9007199254740993";
const CHANNEL_VERSION: &str = "18446744073709551615";

async fn server() -> (String, tokio::task::JoinHandle<()>, String) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "admin":{"role":"admin","tokens":[ADMIN]},
            "writer":{"role":"writer","tokens":[WRITER],"parent":"claims-mcp-owner"},
            "reader":{"role":"reader","tokens":[READER],"parent":"claims-mcp-owner"},
            "hidden":{"role":"reader","tokens":[HIDDEN],"parent":"other-owner"}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "claims-mcp".into(),
            parent: "claims-mcp-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let service = EventModelServiceServer::new(service)
        .max_decoding_message_size(36 * 1024 * 1024)
        .max_encoding_message_size(36 * 1024 * 1024);
    let (drain, _) = tokio::sync::watch::channel(0i64);
    let app = mcp_router(
        data_plane(service, BearerAuth::new(registry)),
        RpcMetricsLayer::new(ServerMetrics::default()),
        InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
        Some(Duration::from_secs(30)),
        36 * 1024 * 1024,
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/mcp"), task, namespace)
}

async fn connect(uri: &str, token: &str) -> McpClient {
    ().serve(StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri.to_owned()).auth_header(token),
    ))
    .await
    .unwrap()
}

async fn invoke(client: &McpClient, name: &'static str, arguments: Value) -> Result<Value, String> {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .map_err(|error| format!("{error:?}"))?;
    let text = result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.clone())
        .unwrap_or_default();
    if result.is_error.unwrap_or(false) {
        return Err(text);
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}
async fn succeed(client: &McpClient, name: &'static str, arguments: Value) -> Value {
    invoke(client, name, arguments)
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}
async fn source(client: &McpClient, name: &'static str, request: Value) -> Result<Value, String> {
    invoke(client, name, json!({"request_json": request})).await
}

fn decode<M: prost::Message + Default>(body: &Value) -> M {
    prost::Message::decode(
        STANDARD
            .decode(body["binpb_base64"].as_str().unwrap())
            .unwrap()
            .as_slice(),
    )
    .unwrap()
}

fn manifest(ns: &str, kind: &str, name: &str, version: Option<&str>, spec: Value) -> String {
    let mut metadata = json!({"namespace": ns, "name": name});
    if let Some(version) = version {
        metadata["version"] = json!(version);
    }
    let mut document = json!({"apiVersion": trogon_atlas_client::manifest::API_VERSION, "kind": kind, "metadata": metadata});
    document["spec"] = spec;
    document.to_string()
}

async fn fetch(client: &McpClient, kind: &str, ns: &str, slug: &str, version: u64) -> Value {
    succeed(
        client,
        "get_entity_json",
        json!({"kind":kind,"namespace":ns,"slug":slug,"version":version}),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_message_claims_mcp_exposes_leaf_provenance_and_protects_targets() {
    let (uri, task, ns) = server().await;
    let ns = ns.as_str();
    let admin = connect(&uri, ADMIN).await;
    let writer = connect(&uri, WRITER).await;
    let reader = connect(&uri, READER).await;
    let hidden = connect(&uri, HIDDEN).await;

    let apply = |documents: Vec<String>| {
        succeed(
            &writer,
            "apply_manifests",
            json!({"manifests_yaml": documents.join("\n---\n")}),
        )
    };
    apply(vec![
        manifest(
            ns,
            "Schema",
            "order",
            None,
            json!({"fields":[{"name":"customer","type":{"object":{"fields":[{"name":"name","type":{"string":{}}}]}}}]}),
        ),
        manifest(
            ns,
            "Specification",
            "messages",
            Some(SPECIFICATION_VERSION),
            json!({"title":"Messages","messages":[
                {"name":{"value":"order-placed"},"schemas":[{"role":"ROLE_PAYLOAD","schema":{"id":{"namespace":ns,"slug":"order","version":"1"}}}]},
                {"name":{"value":"order-shipped"}}
            ]}),
        ),
        manifest(ns, "ReadModel", "order-feed", Some("2"), json!({"title":"Order feed"})),
        manifest(ns, "Processor", "order-intake", None, json!({"title":"Order intake"})),
        manifest(
            ns,
            "AutomationSlice",
            "observe-orders",
            Some("3"),
            json!({"title":"Observe orders","sourceReadModels":["order-feed@2"],"processor":"order-intake"}),
        ),
    ])
    .await;
    let slice = fetch(&writer, "automation_slice", ns, "observe-orders", 3).await;
    let observation = json!({
        "owner":{"kind":"ENTITY_KIND_AUTOMATION_SLICE","id":{"namespace":ns,"slug":"observe-orders","version":"3"}},
        "ownerUid":slice["json"]["system"]["uid"],
        "id":slice["json"]["automationSlice"]["sourceReadModels"][0]["connectionId"]
    });
    apply(vec![manifest(
        ns,
        "Channel",
        "orders",
        Some(CHANNEL_VERSION),
        json!({"title":"Orders",
            "address":{"template":"orders/{tenant}","parameters":[{"name":{"value":"tenant"},"allowedValues":{"values":["","eu"]}}]},
            "bindings":[{"observation":observation,"parameterValues":{"tenant":"eu"}}]}),
    )])
    .await;
    let specification = fetch(
        &writer,
        "specification",
        ns,
        "messages",
        9_007_199_254_740_993,
    )
    .await;
    let contract = &specification["json"]["specification"];
    let message = |member: &Value| json!({"specification":{"id":{"namespace":ns,"slug":"messages","version":SPECIFICATION_VERSION}},"ownerUid":specification["json"]["system"]["uid"],"kind":"CONTRACT_MEMBER_KIND_MESSAGE","memberId":member});
    let placed = &contract["messages"][0]["memberId"];
    let send = json!({"direction":"COMMUNICATION_DIRECTION_SEND","message":message(placed)});
    let owner = |claims: Value| json!({"component":{"id":{"namespace":ns,"slug":"checkout","version":"1"},"title":"Checkout","messageClaims":claims}});

    put(&writer, &owner(json!({"claims":[send, send]})), false)
        .await
        .unwrap();
    let staged = fetch(&writer, "component", ns, "checkout", 1).await;
    let ids: Vec<Value> = staged["json"]["component"]["messageClaims"]["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|claim| claim["connectionId"].clone())
        .collect();
    let channel = json!({"id":{"namespace":ns,"slug":"orders","version":CHANNEL_VERSION}});
    let claims = json!({
        "claims":[{"connectionId":ids[0],"direction":"COMMUNICATION_DIRECTION_SEND","message":message(placed)},{"connectionId":ids[1],"direction":"COMMUNICATION_DIRECTION_SEND","message":message(placed)}],
        "routes":[
            {"claimId":ids[0],"channel":channel,"parameterValues":{"tenant":""}},
            {"claimId":ids[1],"channel":channel,"parameterValues":{"tenant":"eu"}}
        ],
        "fieldSelections":[{"claimId":ids[0],
            "binding":{"owner":{"kind":"ENTITY_KIND_SPECIFICATION","id":{"namespace":ns,"slug":"messages","version":SPECIFICATION_VERSION}},"ownerUid":specification["json"]["system"]["uid"],"id":contract["messages"][0]["schemas"][0]["connectionId"]},
            "selectors":[{"path":{"segments":["customer","name"]}}]}]
    });
    put(&writer, &owner(claims), false).await.unwrap();
    let owner_uid =
        fetch(&writer, "component", ns, "checkout", 1).await["json"]["system"]["uid"].clone();

    let channel_version = 18_446_744_073_709_551_615u64;
    let specification_version = 9_007_199_254_740_993u64;
    let leaf = |references: &pb::GetReferencesResponse| -> Vec<String> {
        references
            .references
            .iter()
            .map(|reference| {
                let from = reference.from.as_ref().unwrap();
                assert_eq!(from.kind, pb::EntityKind::Component as i32);
                assert_eq!(from.id.as_ref().unwrap().slug, "checkout");
                assert_eq!(reference.from_uid, owner_uid.as_str().unwrap());
                assert!(reference.connection_id.is_some(), "{reference:?}");
                reference.from_field.clone()
            })
            .collect()
    };
    for (name, client) in [("reader", &reader), ("writer", &writer)] {
        let channel_refs: pb::GetReferencesResponse = decode(
            &succeed(
                client,
                "get_incoming_references",
                json!({"kind":"channel","namespace":ns,"slug":"orders","version":channel_version}),
            )
            .await,
        );
        assert_eq!(
            leaf(&channel_refs),
            [
                "message_claims.routes[0].channel",
                "message_claims.routes[1].channel"
            ],
            "{name}: each route is its own leaf, in storyboard order"
        );
        let specification_refs: pb::GetReferencesResponse = decode(
            &succeed(
                client,
                "get_incoming_references",
                json!({"kind":"specification","namespace":ns,"slug":"messages","version":specification_version}),
            )
            .await,
        );
        assert_eq!(
            leaf(&specification_refs),
            [
                "message_claims.claims[0].message.specification",
                "message_claims.claims[1].message.specification",
                "message_claims.field_selections[0].binding.owner"
            ],
            "{name}: equal sibling claims stay separate leaves"
        );
        let impact: pb::GetImpactResponse = decode(
            &succeed(
                client,
                "get_impact",
                json!({"kind":"channel","namespace":ns,"slug":"orders","version":channel_version}),
            )
            .await,
        );
        let reached = |kind: pb::EntityKind, slug: &str| {
            impact.nodes.iter().find(|node| {
                let entity = node.entity.as_ref().unwrap();
                entity.kind == kind as i32 && entity.id.as_ref().unwrap().slug == slug
            })
        };
        let component = reached(pb::EntityKind::Component, "checkout")
            .unwrap_or_else(|| panic!("{name}: impact missed the claim owner"));
        assert_eq!(component.depth, 1);
        assert_eq!(
            component.path.last().unwrap().from_field,
            "message_claims.routes[0].channel"
        );
        let specification_node = reached(pb::EntityKind::Specification, "messages")
            .unwrap_or_else(|| panic!("{name}: impact missed the claimed Specification"));
        assert_eq!(specification_node.depth, 2);
        assert!(specification_node
            .path
            .iter()
            .any(|hop| hop.from_field.starts_with("message_claims.claims[")));
    }

    let hidden_view = invoke(
        &hidden,
        "get_incoming_references",
        json!({"kind":"channel","namespace":ns,"slug":"orders","version":channel_version}),
    )
    .await
    .unwrap_err();
    assert!(hidden_view.contains("entity not found"), "{hidden_view}");
    assert!(!hidden_view.contains("checkout"));
    let denied = put(&reader, &owner(json!({})), false).await.unwrap_err();
    assert!(denied.contains("unauthorized"), "{denied}");

    let before = (
        fetch(&writer, "channel", ns, "orders", channel_version).await,
        fetch(
            &writer,
            "specification",
            ns,
            "messages",
            specification_version,
        )
        .await,
        fetch(&writer, "component", ns, "checkout", 1).await,
    );
    for (kind, slug, version) in [
        ("channel", "orders", channel_version),
        ("specification", "messages", specification_version),
    ] {
        let refused = invoke(
            &writer,
            "delete_entity",
            json!({"kind":kind,"namespace":ns,"slug":slug,"version":version}),
        )
        .await
        .unwrap_err();
        assert!(
            refused.contains("entity is referenced"),
            "{kind}: {refused}"
        );
    }
    assert_eq!(
        before,
        (
            fetch(&writer, "channel", ns, "orders", channel_version).await,
            fetch(
                &writer,
                "specification",
                ns,
                "messages",
                specification_version
            )
            .await,
            fetch(&writer, "component", ns, "checkout", 1).await,
        ),
        "a refused delete changed stored state"
    );

    let registration = source(
        &admin,
        "create_source_registration",
        json!({"registration":{"id":"claims-mcp-source","administrationNamespace":ns,"destinationNamespaces":[ns],"profileRevision":1,"ownershipPolicyRevision":"policy","deletionPolicy":"SOURCE_DELETION_POLICY_RETAIN_NATIVE"}}),
    )
    .await
    .unwrap();
    let revision = registration["registration"]["revision"].as_str().unwrap();
    let inventory = serde_json::to_vec(&json!({"profile":"supplied-native-v1","profile_revision":1,"registration":"claims-mcp-source","source_version":"revision","partition":{"categories":["component"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL","resources":[{"key":{"category":"component","key":"claimant"},"file":"native/component.yaml","pointer":{"kind":"opaque","raw":"never-fetch"}}],"relations":[]})).unwrap();
    let upload = pb::AcceptSourceSnapshotRequest {
        registration_id: "claims-mcp-source".into(),
        registration_revision: revision.into(),
        inventory_bytes: inventory,
        files: vec![pb::SourceFile {
            logical_name: "native/component.yaml".into(),
            media_type: "application/yaml".into(),
            content: b"native: bytes\n".to_vec(),
            ..Default::default()
        }],
    };
    let upload = trogon_atlas_core::transcode::TranscodePool::builtin()
        .unwrap()
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.AcceptSourceSnapshotRequest",
            &upload,
        )
        .unwrap();
    let snapshot = source(&writer, "accept_source_snapshot", upload)
        .await
        .unwrap();
    let snapshot = snapshot["snapshot"]["id"].as_str().unwrap();
    let preview = |run: &str, claims: Option<Value>| {
        let mut component =
            json!({"id":{"namespace":ns,"slug":"claimant","version":"1"},"title":"Imported"});
        if let Some(claims) = claims {
            component["messageClaims"] = claims;
        }
        json!({"registrationId":"claims-mcp-source","registrationRevision":revision,"snapshotId":snapshot,"runId":run,
            "nativePayloads":[{"source":{"registrationId":"claims-mcp-source","category":"component","key":"claimant"},"entity":{"component":component}}]})
    };
    let refused = source(
        &writer,
        "preview_source_run",
        preview(
            "claims-run",
            Some(json!({"claims":[{"direction":"COMMUNICATION_DIRECTION_SEND","message":message(placed)}]})),
        ),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("message claims support native authoring only"),
        "refused for another reason: {refused}"
    );
    assert!(source(
        &reader,
        "get_source_run",
        json!({"registrationId":"claims-mcp-source","runId":"claims-run"})
    )
    .await
    .is_err());
    assert!(fetch_err(&reader, "component", ns, "claimant", 1).await);
    let control = source(&writer, "preview_source_run", preview("plain-run", None))
        .await
        .unwrap();
    assert_eq!(control["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    admin.cancel().await.unwrap();
    writer.cancel().await.unwrap();
    reader.cancel().await.unwrap();
    hidden.cancel().await.unwrap();
    task.abort();
}

async fn fetch_err(client: &McpClient, kind: &str, ns: &str, slug: &str, version: u64) -> bool {
    invoke(
        client,
        "get_entity_json",
        json!({"kind":kind,"namespace":ns,"slug":slug,"version":version}),
    )
    .await
    .is_err()
}

async fn put(client: &McpClient, entity: &Value, validate_only: bool) -> Result<Value, String> {
    let mut arguments = json!({"entity_json": entity.to_string()});
    if validate_only {
        arguments["validate_only"] = json!(true);
    }
    invoke(client, "put_entity_json", arguments).await
}
