#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use http::{HeaderName, HeaderValue};
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::client::{connect, connect_with_options, ConnectOptions};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

const ADMIN: &str = "channel-mcp-admin";
const READER_A: &str = "channel-mcp-reader-a";
const READER_B: &str = "channel-mcp-reader-b";
const HIDDEN: &str = "channel-mcp-hidden";
const OWNER: &str = "channel-owner";
const BRANCH: &str = "@channel-owner:inspect";
const MESSAGE: &str = "11111111-1111-4111-8111-111111111111";
const CHANNEL_VERSION: u64 = u64::MAX;
const CHANNEL_TITLE: &str = "Orders Desk";

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn server() -> (String, String, String) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "admin": {"role": "admin", "tokens": [ADMIN]},
            "reader-a": {"role": "reader", "kind": "agent", "tokens": [READER_A], "parent": OWNER},
            "reader-b": {"role": "reader", "kind": "agent", "tokens": [READER_B], "parent": OWNER},
            "hidden": {"role": "reader", "kind": "agent", "tokens": [HIDDEN], "parent": "other-owner"}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let secured = EventModelServiceServer::new(service)
        .max_decoding_message_size(36 * 1024 * 1024)
        .max_encoding_message_size(36 * 1024 * 1024);
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_address = grpc_listener.local_addr().unwrap();
    let grpc = data_plane(secured.clone(), BearerAuth::new(registry.clone()));
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                grpc_listener,
            ))
            .await
            .unwrap();
    });
    let (drain, _) = tokio::sync::watch::channel(0i64);
    let app = mcp_router(
        data_plane(secured, BearerAuth::new(registry)),
        RpcMetricsLayer::new(ServerMetrics::default()),
        InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
        Some(Duration::from_secs(30)),
        36 * 1024 * 1024,
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let endpoint = format!("http://{grpc_address}");
    let mut admin = connect(&endpoint, Some(ADMIN), 30).await.unwrap();
    let namespace = admin
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "channel-messages".into(),
            parent: OWNER.into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    admin
        .create_branch(pb::CreateBranchRequest {
            name: BRANCH.into(),
            doc: "inspect".into(),
        })
        .await
        .unwrap();
    (format!("http://{address}/mcp"), endpoint, namespace)
}

fn id(namespace: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version,
    }
}

fn member(value: &str) -> pb::ContractMemberId {
    pb::ContractMemberId {
        value: value.into(),
    }
}

fn name(value: &str) -> pb::MemberName {
    pb::MemberName {
        value: value.into(),
    }
}

async fn put(
    client: &mut trogon_atlas_client::client::Client,
    kind: pb::entity::Kind,
) -> pb::Entity {
    client
        .put_entity(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(kind),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap_or_else(|status| panic!("put failed: {status}"))
        .into_inner()
        .entity
        .unwrap()
}

async fn seed(endpoint: &str, namespace: &str) -> String {
    let mut client = connect_with_options(
        endpoint,
        Some(ADMIN),
        30,
        ConnectOptions {
            branch: Some(BRANCH.into()),
            ..ConnectOptions::default()
        },
    )
    .await
    .unwrap();
    let read_model = id(namespace, "receipt", 1);
    let processor = id(namespace, "worker", 1);
    let slice = id(namespace, "observe", 1);
    let channel = id(namespace, "orders", CHANNEL_VERSION);
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(read_model.clone()),
            title: "Receipt".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(processor.clone()),
            title: "Worker".into(),
            ..Default::default()
        }),
    )
    .await;
    let automation = put(
        &mut client,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(slice.clone()),
            title: "Observe".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(read_model),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(processor),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::AutomationSlice(slice_value) = automation.kind.as_ref().unwrap() else {
        panic!("automation");
    };
    let observation = pb::ConnectionRef {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(slice),
        }),
        owner_uid: automation.system.as_ref().unwrap().uid.clone(),
        id: slice_value.source_read_models[0].connection_id.clone(),
    };
    put(
        &mut client,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(channel.clone()),
            title: CHANNEL_TITLE.into(),
            address: Some(pb::ChannelAddress {
                template: "orders.{topic}".into(),
                parameters: vec![pb::AddressParameter {
                    name: Some(pb::ParameterName {
                        value: "topic".into(),
                    }),
                    ..Default::default()
                }],
            }),
            bindings: vec![pb::ChannelBinding {
                observation: Some(observation),
                parameter_values: [("topic".into(), "orders".into())].into(),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let specification = put(
        &mut client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(id(namespace, "contract", 1)),
            title: "Contract".into(),
            messages: vec![pb::ContractMessage {
                member_id: Some(member(MESSAGE)),
                name: Some(name("Order")),
                ..Default::default()
            }],
            message_channels: vec![
                pb::MessageChannelDeclaration {
                    member_id: Some(member(MESSAGE)),
                    channel: Some(pb::ChannelRef {
                        id: Some(channel.clone()),
                    }),
                    parameter_values: [("topic".into(), "orders".into())].into(),
                    doc: Some("carried".into()),
                    ..Default::default()
                },
                pb::MessageChannelDeclaration {
                    member_id: Some(member(MESSAGE)),
                    channel: Some(pb::ChannelRef { id: Some(channel) }),
                    parameter_values: [("topic".into(), "orders".into())].into(),
                    delivery_mode: Some(pb::ClaimDeliveryMode::Push as i32),
                    ..Default::default()
                },
            ],
            operations: vec![pb::ContractOperation {
                member_id: Some(member("55555555-5555-4555-8555-555555555555")),
                name: Some(name("Send")),
                messages: vec![pb::OperationMessage {
                    message_id: Some(member(MESSAGE)),
                    role: pb::operation_message::Role::Request as i32,
                    ..Default::default()
                }],
                binding: Some(pb::OperationBinding {
                    kind: Some(pb::operation_binding::Kind::Channel(
                        pb::ChannelOperationBinding {
                            channel: Some(pb::ChannelRef {
                                id: Some(id(namespace, "orders", CHANNEL_VERSION)),
                            }),
                            ..Default::default()
                        },
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    specification.system.unwrap().uid
}

async fn connect_mcp(uri: &str, token: &str) -> McpClient {
    let mut headers = HashMap::new();
    headers.insert(
        HeaderName::from_static("x-trogon-atlas-branch"),
        HeaderValue::from_static(BRANCH),
    );
    ().serve(StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri.to_owned())
            .auth_header(token)
            .custom_headers(headers),
    ))
    .await
    .unwrap()
}

fn channel_request(
    namespace: &str,
    slug: &str,
    page_size: Option<u32>,
    page_token: Option<&str>,
) -> Value {
    let mut request = json!({
        "channel": {
            "id": {
                "namespace": namespace,
                "slug": slug,
                "version": CHANNEL_VERSION.to_string()
            }
        }
    });
    if let Some(page_size) = page_size {
        request["pageSize"] = json!(page_size);
    }
    if let Some(page_token) = page_token {
        request["pageToken"] = json!(page_token);
    }
    request
}

async fn invoke(client: &McpClient, request: Value) -> Result<Value, String> {
    match client
        .call_tool(
            CallToolRequestParams::new("list_channel_messages").with_arguments(
                json!({"request_json": request})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
    {
        Ok(result) if result.is_error.unwrap_or(false) => Err(format!("{result:?}")),
        Ok(result) => {
            let content = result.content.into_iter().next().unwrap();
            Ok(serde_json::from_str(&content.as_text().unwrap().text).unwrap())
        }
        Err(error) => Err(error.to_string()),
    }
}

async fn listed(client: &McpClient, request: Value) -> Value {
    invoke(client, request)
        .await
        .unwrap_or_else(|error| panic!("list_channel_messages failed: {error}"))
}

fn assert_scope(page: &Value) {
    let digest = page["scopeDigest"].as_str().unwrap_or("");
    assert_eq!(digest.len(), 64, "{page}");
    assert!(digest.chars().all(|ch| ch.is_ascii_hexdigit()), "{digest}");
    assert_eq!(page["authorizedNamespaceCount"], 1, "{page}");
    assert_eq!(page["branch"], BRANCH, "{page}");
}

fn assert_channel_id(record: &Value, namespace: &str) {
    let id = &record["channel"]["id"];
    assert_eq!(id["namespace"], namespace, "{record}");
    assert_eq!(id["slug"], "orders", "{record}");
    assert_eq!(id["version"], CHANNEL_VERSION.to_string(), "{record}");
}

fn assert_operation_record(record: &Value, independent: &Value, namespace: &str, owner_uid: &str) {
    assert_eq!(
        record["authority"],
        "CHANNEL_MESSAGE_AUTHORITY_OPERATION_BINDING"
    );
    assert_channel_id(record, namespace);
    assert_eq!(record["owner"], independent["owner"]);
    assert_eq!(record["ownerUid"], owner_uid);
    assert_eq!(record["message"], independent["message"]);
    assert_eq!(
        record["operation"]["kind"],
        "CONTRACT_MEMBER_KIND_OPERATION"
    );
    assert_eq!(
        record["operation"]["specification"],
        record["message"]["specification"]
    );
    assert_eq!(record["operation"]["ownerUid"], owner_uid);
    assert_eq!(
        record["operation"]["memberId"]["value"],
        "55555555-5555-4555-8555-555555555555"
    );
    assert_eq!(
        record["operationMessage"]["messageId"],
        record["message"]["memberId"]
    );
    assert_eq!(record["operationMessage"]["role"], "ROLE_REQUEST");
    let binding_id = record["connectionId"]["value"].as_str().unwrap();
    let message_id = record["operationMessage"]["connectionId"]["value"]
        .as_str()
        .unwrap();
    assert_ne!(binding_id, "");
    assert_ne!(message_id, "");
    assert_ne!(binding_id, message_id);
    assert_eq!(record["state"], "CATALOG_RESOLUTION_STATE_RESOLVED");
    assert!(record.get("deliveryMode").is_none());
}

fn assert_preserved_records(records: &[&Value], namespace: &str, owner_uid: &str) {
    assert_eq!(records.len(), 2, "{records:?}");
    for record in records {
        assert_eq!(
            record["authority"], "CHANNEL_MESSAGE_AUTHORITY_SPECIFICATION_DECLARATION",
            "{record}"
        );
        assert_eq!(record["ownerUid"], owner_uid, "{record}");
        assert_eq!(
            record["owner"]["kind"], "ENTITY_KIND_SPECIFICATION",
            "{record}"
        );
        assert_eq!(record["owner"]["id"]["slug"], "contract");
        assert_eq!(record["owner"]["id"]["version"], "1");
        assert_eq!(record["message"]["kind"], "CONTRACT_MEMBER_KIND_MESSAGE");
        assert_eq!(record["message"]["memberId"]["value"], MESSAGE);
        assert_eq!(record["message"]["ownerUid"], owner_uid);
        assert_eq!(record["parameterValues"]["topic"], "orders");
        assert!(record["connectionId"]["value"]
            .as_str()
            .is_some_and(|id| !id.is_empty()));
        assert_channel_id(record, namespace);
    }
    let connection_ids = records
        .iter()
        .map(|record| record["connectionId"]["value"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(connection_ids.len(), 2);
    let documented = records
        .iter()
        .filter(|record| record.get("doc").is_some())
        .collect::<Vec<_>>();
    let delivered = records
        .iter()
        .filter(|record| record.get("deliveryMode").is_some())
        .collect::<Vec<_>>();
    assert_eq!(documented.len(), 1, "{records:?}");
    assert_eq!(documented[0]["doc"], "carried");
    assert!(
        documented[0].get("deliveryMode").is_none(),
        "{documented:?}"
    );
    assert_eq!(delivered.len(), 1, "{records:?}");
    assert_eq!(delivered[0]["deliveryMode"], "CLAIM_DELIVERY_MODE_PUSH");
    assert!(delivered[0].get("doc").is_none(), "{delivered:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_lists_channel_messages_with_page_caller_and_hidden_parity() {
    let (uri, endpoint, namespace) = server().await;
    let owner_uid = seed(&endpoint, &namespace).await;
    let reader = connect_mcp(&uri, READER_A).await;
    let other = connect_mcp(&uri, READER_B).await;
    let hidden = connect_mcp(&uri, HIDDEN).await;
    let tools = reader.list_all_tools().await.unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool.name == "list_channel_messages")
        .unwrap_or_else(|| panic!("reader tools omit list_channel_messages: {tools:?}"));
    assert_eq!(
        tool.annotations
            .as_ref()
            .and_then(|notes| notes.read_only_hint),
        Some(true)
    );
    assert_eq!(
        tool.annotations
            .as_ref()
            .and_then(|notes| notes.destructive_hint),
        Some(false)
    );

    let full = listed(&reader, channel_request(&namespace, "orders", None, None)).await;
    assert_scope(&full);
    assert_eq!(full["complete"], true, "{full}");
    assert!(full.get("incomplete").is_none(), "{full}");
    assert!(full.get("nextPageToken").is_none(), "{full}");
    let records = full["records"].as_array().unwrap();
    assert_eq!(records.len(), 3, "{full}");
    let independent: Vec<_> = records
        .iter()
        .filter(|record| {
            record["authority"] == "CHANNEL_MESSAGE_AUTHORITY_SPECIFICATION_DECLARATION"
        })
        .collect();
    assert_preserved_records(&independent, &namespace, &owner_uid);
    let operation = records
        .iter()
        .find(|record| record["authority"] == "CHANNEL_MESSAGE_AUTHORITY_OPERATION_BINDING")
        .unwrap();
    assert_operation_record(operation, &records[0], &namespace, &owner_uid);

    let first = listed(
        &reader,
        channel_request(&namespace, "orders", Some(1), None),
    )
    .await;
    assert_scope(&first);
    assert_eq!(first["records"].as_array().unwrap().len(), 1, "{first}");
    assert_ne!(first["complete"].as_bool(), Some(true), "{first}");
    assert_ne!(first["incomplete"].as_bool(), Some(true), "{first}");
    let token = first["nextPageToken"].as_str().unwrap();
    assert!(!token.is_empty(), "{first}");
    assert!(!token.contains(READER_A), "{token}");
    assert!(!token.contains("reader-a"), "{token}");

    let second = listed(
        &reader,
        channel_request(&namespace, "orders", Some(1), Some(token)),
    )
    .await;
    assert_scope(&second);
    assert_ne!(second["complete"].as_bool(), Some(true), "{second}");
    assert!(second.get("incomplete").is_none(), "{second}");
    let next = second["nextPageToken"].as_str().unwrap();
    assert!(!next.is_empty(), "{second}");
    assert_ne!(
        first["records"][0]["connectionId"]["value"],
        second["records"][0]["connectionId"]["value"]
    );
    assert_preserved_records(
        &[&first["records"][0], &second["records"][0]],
        &namespace,
        &owner_uid,
    );

    let third = listed(
        &reader,
        channel_request(&namespace, "orders", Some(1), Some(next)),
    )
    .await;
    assert_scope(&third);
    assert_eq!(third["complete"], true, "{third}");
    assert!(third.get("incomplete").is_none(), "{third}");
    assert!(third.get("nextPageToken").is_none(), "{third}");
    assert_eq!(third["records"].as_array().unwrap().len(), 1, "{third}");
    assert_operation_record(
        &third["records"][0],
        &first["records"][0],
        &namespace,
        &owner_uid,
    );

    let replay = invoke(
        &other,
        channel_request(&namespace, "orders", Some(1), Some(token)),
    )
    .await
    .unwrap_err();
    assert!(
        replay.contains("page token does not match this query"),
        "{replay}"
    );
    assert!(!replay.contains(CHANNEL_TITLE), "{replay}");

    let missing = invoke(&reader, channel_request(&namespace, "absent", None, None))
        .await
        .unwrap_err();
    let concealed = invoke(&hidden, channel_request(&namespace, "orders", None, None))
        .await
        .unwrap_err();
    assert!(missing.contains("resource unavailable"), "{missing}");
    assert!(concealed.contains("resource unavailable"), "{concealed}");
    assert!(!missing.contains(CHANNEL_TITLE), "{missing}");
    assert!(!concealed.contains(CHANNEL_TITLE), "{concealed}");
}
