#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use http::{HeaderName, HeaderValue};
use prost::Message;
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

const WRITER: &str = "mcp-accept-writer";
const READER: &str = "mcp-accept-reader";
const VERSION: u64 = 1;

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn server() -> (String, tokio::task::JoinHandle<()>) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "writer": {"role": "writer", "tokens": [WRITER]},
            "reader": {"role": "reader", "tokens": [READER]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
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
    (format!("http://{address}/mcp"), task)
}

async fn connect(uri: &str, token: &str) -> McpClient {
    connect_branch(uri, token, None).await
}

async fn connect_branch(uri: &str, token: &str, branch: Option<&str>) -> McpClient {
    let mut config =
        StreamableHttpClientTransportConfig::with_uri(uri.to_owned()).auth_header(token);
    if let Some(branch) = branch {
        let mut headers = HashMap::new();
        headers.insert(
            HeaderName::from_static("x-trogon-atlas-branch"),
            HeaderValue::from_str(branch).unwrap(),
        );
        config = config.custom_headers(headers);
    }
    ().serve(StreamableHttpClientTransport::from_config(config))
        .await
        .unwrap()
}

async fn call(client: &McpClient, name: &str, args: Value) -> Value {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{name}: {result:?}");
    let content = result.content.into_iter().next().unwrap();
    serde_json::from_str(&content.as_text().unwrap().text).unwrap()
}

fn decode_binpb<M: Message + Default>(body: &Value) -> M {
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(body["binpb_base64"].as_str().unwrap())
        .unwrap();
    M::decode(raw.as_slice()).unwrap()
}

fn schema_of(response: &pb::GetEntityResponse) -> pb::Schema {
    let pb::entity::Kind::Schema(schema) = response.entity.as_ref().unwrap().kind.as_ref().unwrap()
    else {
        panic!("schema");
    };
    schema.clone()
}

fn order_documents(yaml: &str) -> (String, String) {
    let mut first = None;
    let mut second = None;
    for document in yaml.split("\n---\n") {
        if !document.contains("name: order") {
            continue;
        }
        if document.contains("version:") {
            second = Some(document.to_string());
        } else {
            first = Some(document.to_string());
        }
    }
    (
        first.expect("order version 1"),
        second.expect("order version 2"),
    )
}

fn strip_connection_ids(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("connectionId");
            for child in map.values_mut() {
                strip_connection_ids(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_connection_ids(item);
            }
        }
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_mcp_absent_aggregate_reapply_isolates_versions() {
    let (uri, task) = server().await;
    let writer = connect(&uri, WRITER).await;
    let reader = connect(&uri, READER).await;
    let namespace = "mcp-accept";
    let product = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "product", "version": "1"},
            "title": "Product",
            "fields": [{"name": "sku", "type": {"string": {}}}]
        }
    });
    let order = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "order", "version": "1"},
            "title": "Order",
            "fields": [
                {"name": "customer", "type": {"object": {"fields": [
                    {"name": "id", "type": {"string": {}}}
                ]}}},
                {"name": "product_ref", "type": {"ref": {"ref": {"schema": {"id": {
                    "namespace": namespace, "slug": "product", "version": "1"
                }}}}}}
            ],
            "identifiers": [{
                "property": {"segments": [
                    {"name": "customer", "repeated": false},
                    {"name": "id", "repeated": false}
                ]},
                "role": "BUSINESS_IDENTIFIER_ROLE_PRIMARY"
            }],
            "propertyRelationships": [{
                "source": {"segments": [{"name": "product_ref", "repeated": false}]},
                "targetEntity": {"id": {"namespace": namespace, "slug": "product", "version": "1"}},
                "label": {"value": "hasMany"}
            }]
        }
    });
    let second = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "order", "version": "2"},
            "title": "Order v2",
            "aggregateRoot": true,
            "fields": [{"name": "id", "type": {"string": {}}}]
        }
    });
    let note = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "note", "version": "1"},
            "title": "Note",
            "fields": [{"name": "body", "type": {"string": {}}}]
        }
    });
    for entity in [product, order, second, note] {
        call(
            &writer,
            "put_entity_json",
            json!({"entity_json": entity.to_string()}),
        )
        .await;
    }

    let v1_json = call(
        &reader,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
    )
    .await;
    assert!(
        v1_json["json"]["schema"].get("aggregateRoot").is_none(),
        "{v1_json}"
    );
    let v2_json = call(
        &reader,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 2}),
    )
    .await;
    assert_eq!(v2_json["json"]["schema"]["aggregateRoot"], true);
    let v1 = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &reader,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
        )
        .await,
    );
    let v2 = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &reader,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 2}),
        )
        .await,
    );
    assert_eq!(schema_of(&v1).aggregate_root, None);
    assert_eq!(schema_of(&v2).aggregate_root, Some(true));
    assert_ne!(
        v1.entity.as_ref().unwrap().system.as_ref().unwrap().uid,
        v2.entity.as_ref().unwrap().system.as_ref().unwrap().uid
    );
    let relationship_id = schema_of(&v1).property_relationships[0]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let listed = decode_binpb::<pb::ListEntitiesResponse>(
        &call(
            &reader,
            "list_entities",
            json!({"kind": "schema", "namespace": namespace, "page_size": 100}),
        )
        .await,
    );
    assert!(
        listed.entities.len() >= 4,
        "product, note, and both order versions: {listed:?}"
    );
    let outgoing = decode_binpb::<pb::GetReferencesResponse>(
        &call(
            &reader,
            "get_outgoing_references",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
        )
        .await,
    );
    assert!(outgoing.references.iter().any(|reference| {
        reference.from_field.contains("property_relationships")
            && reference.connection_id.as_ref().map(|id| id.value.as_str())
                == Some(relationship_id.as_str())
    }));
    assert!(!outgoing
        .references
        .iter()
        .any(|reference| reference.from_field.contains("identifiers")));

    let mut edited = v1_json["json"].clone();
    edited["schema"]["title"] = json!("Order edited");
    edited["schema"]["propertyRelationships"][0]["label"] = json!({"value": "belongsTo"});
    call(
        &writer,
        "put_entity_json",
        json!({"entity_json": edited.to_string(), "if_match": v1.etag}),
    )
    .await;
    let edited_get = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &reader,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
        )
        .await,
    );
    assert_ne!(edited_get.etag, v1.etag);
    assert_eq!(schema_of(&edited_get).aggregate_root, None);
    assert_eq!(
        schema_of(&edited_get).property_relationships[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        relationship_id
    );
    assert_eq!(
        schema_of(&edited_get).property_relationships[0]
            .label
            .as_ref()
            .unwrap()
            .value,
        "belongsTo"
    );

    let exported = call(&writer, "export_namespace", json!({"namespace": namespace})).await;
    let files = exported["files"].as_object().unwrap();
    let schema_yaml = files
        .values()
        .find_map(|value| value.as_str().filter(|text| text.contains("name: order")))
        .expect("schema export");
    let (first, second) = order_documents(schema_yaml);
    assert!(!first.contains("aggregateRoot"), "{first}");
    assert!(first.contains("belongsTo"), "{first}");
    assert!(second.contains("aggregateRoot: true"), "{second}");
    let yaml = files
        .values()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join("\n---\n");
    let diff = call(&writer, "diff_manifests", json!({"manifests_yaml": yaml})).await;
    assert_eq!(diff["changed"], false, "{diff}");
    call(&writer, "apply_manifests", json!({"manifests_yaml": yaml})).await;
    let reapplied = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &reader,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
        )
        .await,
    );
    assert_eq!(reapplied, edited_get);

    let mut stripped = call(
        &reader,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
    )
    .await["json"]
        .clone();
    strip_connection_ids(&mut stripped);
    let manifest = json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": namespace, "name": "order"},
        "spec": {
            "title": stripped["schema"]["title"],
            "fields": stripped["schema"]["fields"],
            "identifiers": stripped["schema"]["identifiers"],
            "propertyRelationships": stripped["schema"]["propertyRelationships"]
        }
    });
    assert!(manifest["spec"].get("aggregateRoot").is_none());
    call(
        &writer,
        "apply_manifests",
        json!({"manifests_yaml": manifest.to_string()}),
    )
    .await;
    let kept = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &reader,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
        )
        .await,
    );
    assert_eq!(kept, edited_get);

    let denied = reader
        .call_tool(
            CallToolRequestParams::new("put_entity_json").with_arguments(
                json!({"entity_json": json!({
                    "schema": {
                        "id": {"namespace": namespace, "slug": "order", "version": "1"},
                        "title": "Denied"
                    }
                }).to_string()})
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;
    match denied {
        Ok(result) => assert!(result.is_error.unwrap_or(false), "{result:?}"),
        Err(error) => assert_ne!(error.to_string(), ""),
    }
    let still = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": VERSION}),
        )
        .await,
    );
    assert_eq!(still, edited_get);

    call(
        &writer,
        "delete_entity",
        json!({"kind": "schema", "namespace": namespace, "slug": "note", "version": VERSION, "mode": "dry_run"}),
    )
    .await;
    call(
        &reader,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "note", "version": VERSION}),
    )
    .await;
    call(
        &writer,
        "delete_entity",
        json!({"kind": "schema", "namespace": namespace, "slug": "note", "version": VERSION}),
    )
    .await;
    let gone = reader
        .call_tool(
            CallToolRequestParams::new("get_entity_json").with_arguments(
                json!({"kind": "schema", "namespace": namespace, "slug": "note", "version": VERSION})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    if let Ok(result) = gone {
        assert!(result.is_error.unwrap_or(false), "{result:?}");
    }
    task.abort();
}

fn schema_uid(response: &pb::GetEntityResponse) -> String {
    response
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone()
}

fn occurrence_ids(response: &pb::GetEntityResponse) -> (String, String) {
    let schema = schema_of(response);
    (
        schema.identifiers[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value
            .clone(),
        schema.property_relationships[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value
            .clone(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn business_mcp_impact_retarget_delete_and_branch_keep_identity() {
    let (uri, task) = server().await;
    let writer = connect(&uri, WRITER).await;
    let namespace = "mcp-boundary";
    let customer = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "customer-record", "version": "1"},
            "title": "Customer",
            "fields": [{"name": "id", "type": {"string": {}}}]
        }
    });
    let product = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "product", "version": "1"},
            "title": "Product",
            "fields": [
                {"name": "sku", "type": {"string": {}}},
                {"name": "customer_ref", "type": {"ref": {"ref": {"schema": {"id": {
                    "namespace": namespace, "slug": "customer-record", "version": "1"
                }}}}}}
            ]
        }
    });
    let product_v2 = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "product", "version": "2"},
            "title": "Product v2",
            "fields": [{"name": "sku", "type": {"string": {}}}]
        }
    });
    let order = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "order", "version": "1"},
            "title": "Order",
            "fields": [
                {"name": "customer", "type": {"object": {"fields": [
                    {"name": "id", "type": {"string": {}}}
                ]}}},
                {"name": "product_ref", "type": {"string": {}}}
            ],
            "identifiers": [{
                "property": {"segments": [
                    {"name": "customer", "repeated": false},
                    {"name": "id", "repeated": false}
                ]},
                "role": "BUSINESS_IDENTIFIER_ROLE_PRIMARY"
            }],
            "propertyRelationships": [{
                "source": {"segments": [{"name": "product_ref", "repeated": false}]},
                "targetEntity": {"id": {"namespace": namespace, "slug": "product", "version": "1"}},
                "targetProperty": {"segments": [{"name": "sku", "repeated": false}]},
                "label": {"value": "hasMany"}
            }]
        }
    });
    let solo = json!({
        "schema": {
            "id": {"namespace": namespace, "slug": "solo", "version": "1"},
            "title": "Solo",
            "fields": [{"name": "sku", "type": {"string": {}}}],
            "propertyRelationships": [{
                "source": {"segments": [{"name": "sku", "repeated": false}]},
                "targetEntity": {"id": {"namespace": namespace, "slug": "solo", "version": "1"}},
                "targetProperty": {"segments": [{"name": "sku", "repeated": false}]}
            }]
        }
    });
    for entity in [customer, product, product_v2, order, solo] {
        call(
            &writer,
            "put_entity_json",
            json!({"entity_json": entity.to_string()}),
        )
        .await;
    }
    let before = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    let uid = schema_uid(&before);
    let (identifier_id, relationship_id) = occurrence_ids(&before);
    assert_ne!(uid, "");
    assert_ne!(identifier_id, relationship_id);
    let impact = decode_binpb::<pb::GetImpactResponse>(
        &call(
            &writer,
            "get_impact",
            json!({
                "kind": "schema",
                "namespace": namespace,
                "slug": "order",
                "version": 1,
                "max_depth": 5
            }),
        )
        .await,
    );
    let product_node = impact
        .nodes
        .iter()
        .find(|node| {
            node.entity
                .as_ref()
                .and_then(|entity| entity.id.as_ref())
                .is_some_and(|id| id.slug == "product" && id.version == 1)
        })
        .unwrap();
    assert!(product_node.path.iter().any(|reference| {
        reference.from_uid == uid
            && reference.connection_id.as_ref().map(|id| id.value.as_str())
                == Some(relationship_id.as_str())
            && reference.from_field.contains("property_relationships")
    }));
    assert!(!impact.nodes.iter().any(|node| {
        node.entity
            .as_ref()
            .and_then(|entity| entity.id.as_ref())
            .is_some_and(|id| id.slug == "customer-record")
    }));
    let mut typed = call(
        &writer,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
    )
    .await["json"]
        .clone();
    typed["schema"]["fields"][1] = json!({
        "name": "product_ref",
        "type": {"ref": {"ref": {"schema": {"id": {
            "namespace": namespace, "slug": "product", "version": "1"
        }}}}}
    });
    call(
        &writer,
        "put_entity_json",
        json!({"entity_json": typed.to_string()}),
    )
    .await;
    let before = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    assert_eq!(schema_uid(&before), uid);
    assert_eq!(
        occurrence_ids(&before),
        (identifier_id.clone(), relationship_id.clone())
    );
    match &schema_of(&before).fields[1].r#type.as_ref().unwrap().kind {
        Some(pb::field_type::Kind::Ref(value)) => match &value.r#ref.as_ref().unwrap().r#ref {
            Some(pb::type_ref::Ref::Schema(schema_ref)) => {
                assert_eq!(schema_ref.id.as_ref().unwrap().version, 1);
            }
            other => panic!("schema ref, got {other:?}"),
        },
        other => panic!("ref, got {other:?}"),
    }
    let preview = decode_binpb::<pb::RetargetReferencesResponse>(
        &call(
            &writer,
            "retarget_references",
            json!({
                "from_kind": "schema",
                "from_namespace": namespace,
                "from_slug": "product",
                "from_version": 1,
                "to_version": 2,
                "dry_run": true
            }),
        )
        .await,
    );
    assert!(preview.rewritten >= 1, "{preview:?}");
    assert_eq!(preview.failed, 0);
    let dry = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    assert_eq!(dry, before);
    let retargeted = decode_binpb::<pb::RetargetReferencesResponse>(
        &call(
            &writer,
            "retarget_references",
            json!({
                "from_kind": "schema",
                "from_namespace": namespace,
                "from_slug": "product",
                "from_version": 1,
                "to_version": 2
            }),
        )
        .await,
    );
    assert!(retargeted.rewritten >= 1, "{retargeted:?}");
    assert_eq!(retargeted.failed, 0);
    let moved = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    let moved_schema = schema_of(&moved);
    assert_eq!(schema_uid(&moved), uid);
    assert_eq!(
        moved_schema.identifiers[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        identifier_id
    );
    let renewed = moved_schema.property_relationships[0]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert_ne!(renewed, relationship_id);
    assert_eq!(
        moved_schema.property_relationships[0]
            .target_entity
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        2
    );
    match &moved_schema.fields[1].r#type.as_ref().unwrap().kind {
        Some(pb::field_type::Kind::Ref(value)) => match &value.r#ref.as_ref().unwrap().r#ref {
            Some(pb::type_ref::Ref::Schema(schema_ref)) => {
                assert_eq!(schema_ref.id.as_ref().unwrap().version, 2);
            }
            other => panic!("schema ref, got {other:?}"),
        },
        other => panic!("ref, got {other:?}"),
    }
    let mut relationship_only = call(
        &writer,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
    )
    .await["json"]
        .clone();
    relationship_only["schema"]["fields"][1] =
        json!({"name": "product_ref", "type": {"string": {}}});
    call(
        &writer,
        "put_entity_json",
        json!({"entity_json": relationship_only.to_string()}),
    )
    .await;
    let moved = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    assert_eq!(
        occurrence_ids(&moved),
        (identifier_id.clone(), renewed.clone())
    );
    let product_v2_before = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "product", "version": 2}),
        )
        .await,
    );
    let dry_delete = decode_binpb::<pb::DeleteEntityResponse>(
        &call(
            &writer,
            "delete_entity",
            json!({
                "kind": "schema",
                "namespace": namespace,
                "slug": "product",
                "version": 2,
                "mode": "dry_run"
            }),
        )
        .await,
    );
    assert!(dry_delete.referrers.iter().any(|reference| {
        reference.from_uid == uid
            && reference.connection_id.as_ref().map(|id| id.value.as_str())
                == Some(renewed.as_str())
            && reference.from_field.contains("property_relationships")
    }));
    let blocked_force = writer
        .call_tool(
            CallToolRequestParams::new("delete_entity").with_arguments(
                json!({
                    "kind": "schema",
                    "namespace": namespace,
                    "slug": "product",
                    "version": 2
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;
    match blocked_force {
        Ok(result) => {
            assert!(result.is_error.unwrap_or(false), "{result:?}");
            assert!(format!("{result:?}").contains("referenced"), "{result:?}");
        }
        Err(error) => assert!(error.to_string().contains("referenced"), "{error}"),
    }
    let product_v2_after = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "product", "version": 2}),
        )
        .await,
    );
    assert_eq!(product_v2_after, product_v2_before);
    let still = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    assert_eq!(still, moved);
    call(
        &writer,
        "delete_entity",
        json!({"kind": "schema", "namespace": namespace, "slug": "solo", "version": 1}),
    )
    .await;
    let solo_gone = writer
        .call_tool(
            CallToolRequestParams::new("get_entity").with_arguments(
                json!({"kind": "schema", "namespace": namespace, "slug": "solo", "version": 1})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    match solo_gone {
        Ok(result) => {
            assert!(result.is_error.unwrap_or(false), "{result:?}");
            let rendered = format!("{result:?}").to_lowercase();
            assert!(
                rendered.contains("not_found") || rendered.contains("not found"),
                "{result:?}"
            );
        }
        Err(error) => {
            let rendered = error.to_string().to_lowercase();
            assert!(
                rendered.contains("not_found") || rendered.contains("not found"),
                "{error}"
            );
        }
    }
    assert_eq!(
        decode_binpb::<pb::GetEntityResponse>(
            &call(
                &writer,
                "get_entity",
                json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
            )
            .await,
        ),
        moved
    );
    let created = writer
        .call_tool(
            CallToolRequestParams::new("create_branch").with_arguments(
                json!({"name": "mcp-bd-keep", "doc": "business ids"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(!created.is_error.unwrap_or(false), "{created:?}");
    assert!(
        created.content[0]
            .as_text()
            .unwrap()
            .text
            .contains("mcp-bd-keep"),
        "{created:?}"
    );
    let branched = connect_branch(&uri, WRITER, Some("mcp-bd-keep")).await;
    let mut edited = call(
        &branched,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
    )
    .await["json"]
        .clone();
    edited["schema"]["title"] = json!("Branch title");
    call(
        &branched,
        "put_entity_json",
        json!({"entity_json": edited.to_string()}),
    )
    .await;
    let on_branch = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &branched,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    assert_eq!(schema_of(&on_branch).title, "Branch title");
    assert_eq!(schema_uid(&on_branch), uid);
    assert_eq!(
        schema_of(&on_branch).identifiers[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        identifier_id
    );
    assert_eq!(
        schema_of(&on_branch).property_relationships[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        renewed
    );
    let baseline = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    assert_eq!(baseline, moved);
    call(&writer, "merge_branch", json!({"name": "mcp-bd-keep"})).await;
    let landed = decode_binpb::<pb::GetEntityResponse>(
        &call(
            &writer,
            "get_entity",
            json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
        )
        .await,
    );
    assert_eq!(schema_of(&landed).title, "Branch title");
    assert_eq!(schema_uid(&landed), uid);
    assert_eq!(
        schema_of(&landed).identifiers[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        identifier_id
    );
    assert_eq!(
        schema_of(&landed).property_relationships[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        renewed
    );
    assert_ne!(landed.etag, moved.etag);
    let dropped = writer
        .call_tool(
            CallToolRequestParams::new("create_branch").with_arguments(
                json!({"name": "mcp-bd-drop", "doc": "discard"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(!dropped.is_error.unwrap_or(false), "{dropped:?}");
    let dropping = connect_branch(&uri, WRITER, Some("mcp-bd-drop")).await;
    let mut discarded = call(
        &dropping,
        "get_entity_json",
        json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
    )
    .await["json"]
        .clone();
    discarded["schema"]["title"] = json!("Discarded");
    call(
        &dropping,
        "put_entity_json",
        json!({"entity_json": discarded.to_string()}),
    )
    .await;
    call(
        &writer,
        "merge_branch",
        json!({"name": "mcp-bd-drop", "dry_run": true}),
    )
    .await;
    assert_eq!(
        decode_binpb::<pb::GetEntityResponse>(
            &call(
                &writer,
                "get_entity",
                json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
            )
            .await,
        ),
        landed
    );
    let deleted = writer
        .call_tool(
            CallToolRequestParams::new("delete_branch")
                .with_arguments(json!({"name": "mcp-bd-drop"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert!(!deleted.is_error.unwrap_or(false), "{deleted:?}");
    assert_eq!(
        decode_binpb::<pb::GetEntityResponse>(
            &call(
                &writer,
                "get_entity",
                json!({"kind": "schema", "namespace": namespace, "slug": "order", "version": 1}),
            )
            .await,
        ),
        landed
    );
    task.abort();
}
