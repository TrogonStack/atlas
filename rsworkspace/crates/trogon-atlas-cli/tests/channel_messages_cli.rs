#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::client::{connect, connect_with_options, ConnectOptions};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "channel-cli-admin";
const READER_A: &str = "channel-cli-reader-a";
const READER_B: &str = "channel-cli-reader-b";
const HIDDEN: &str = "channel-cli-hidden";
const OWNER: &str = "channel-owner";
const BRANCH: &str = "@channel-owner:inspect";
const MESSAGE: &str = "11111111-1111-4111-8111-111111111111";
const CHANNEL_VERSION: u64 = u64::MAX;
const CHANNEL_TITLE: &str = "Orders Desk";

async fn server() -> (String, String) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "admin": {"role": "admin", "tokens": [ADMIN]},
            "reader-a": {"role": "reader", "tokens": [READER_A], "parent": OWNER},
            "reader-b": {"role": "reader", "tokens": [READER_B], "parent": OWNER},
            "hidden": {"role": "reader", "tokens": [HIDDEN], "parent": "other-owner"}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let secured = data_plane(
        EventModelServiceServer::new(service)
            .max_decoding_message_size(36 * 1024 * 1024)
            .max_encoding_message_size(36 * 1024 * 1024),
        BearerAuth::new(registry),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(secured)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let endpoint = format!("http://{address}");
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
    (endpoint, namespace)
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

fn command(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            endpoint,
            "--auth-token",
            token,
            "--branch",
            BRANCH,
            "--format",
            "json",
        ])
        .args(args)
        .output()
        .unwrap()
}

fn outcome(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn listed(endpoint: &str, token: &str, request: &Value) -> Value {
    let output = command(
        endpoint,
        token,
        &["list-channel-messages", "--request", &request.to_string()],
    );
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let body = outcome(&output);
    assert!(body["ok"].as_bool().unwrap_or(false), "{body}");
    body["data"].clone()
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

#[test]
fn channel_message_json_keeps_forward_compatible_authorities() {
    let response = pb::ListChannelMessagesResponse {
        records: vec![
            pb::ChannelMessageRecord {
                authority: pb::ChannelMessageAuthority::ClaimRoute as i32,
                owner_uid: "owner".into(),
                ..Default::default()
            },
            pb::ChannelMessageRecord {
                authority: pb::ChannelMessageAuthority::OperationBinding as i32,
                ..Default::default()
            },
            pb::ChannelMessageRecord {
                authority: 17,
                ..Default::default()
            },
        ],
        complete: true,
        scope_digest: "ab".into(),
        authorized_namespace_count: 2,
        ..Default::default()
    };
    let json = TranscodePool::builtin()
        .unwrap()
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListChannelMessagesResponse",
            &response,
        )
        .unwrap();
    assert_eq!(
        json["records"][0]["authority"],
        "CHANNEL_MESSAGE_AUTHORITY_CLAIM_ROUTE"
    );
    assert_eq!(
        json["records"][1]["authority"],
        "CHANNEL_MESSAGE_AUTHORITY_OPERATION_BINDING"
    );
    assert_eq!(json["records"][2]["authority"], 17);
    assert_eq!(json["complete"], true);
    assert!(json.get("incomplete").is_none(), "{json}");
    assert_eq!(json["authorizedNamespaceCount"], 2);
    assert!(json.get("nextPageToken").is_none(), "{json}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_lists_channel_messages_with_page_caller_and_hidden_parity() {
    let (endpoint, namespace) = server().await;
    let owner_uid = seed(&endpoint, &namespace).await;
    let full = listed(
        &endpoint,
        READER_A,
        &channel_request(&namespace, "orders", None, None),
    );
    assert_scope(&full);
    assert_eq!(full["complete"], true, "{full}");
    assert!(full.get("incomplete").is_none(), "{full}");
    assert!(full.get("nextPageToken").is_none(), "{full}");
    let records = full["records"].as_array().unwrap();
    assert_preserved_records(&records.iter().collect::<Vec<_>>(), &namespace, &owner_uid);

    let first = listed(
        &endpoint,
        READER_A,
        &channel_request(&namespace, "orders", Some(1), None),
    );
    assert_scope(&first);
    assert_eq!(first["records"].as_array().unwrap().len(), 1, "{first}");
    assert_ne!(first["complete"].as_bool(), Some(true), "{first}");
    assert_ne!(first["incomplete"].as_bool(), Some(true), "{first}");
    let token = first["nextPageToken"].as_str().unwrap();
    assert!(!token.is_empty(), "{first}");
    assert!(!token.contains(READER_A), "{token}");
    assert!(!token.contains("reader-a"), "{token}");

    let second = listed(
        &endpoint,
        READER_A,
        &channel_request(&namespace, "orders", Some(1), Some(token)),
    );
    assert_scope(&second);
    assert_eq!(second["complete"], true, "{second}");
    assert!(second.get("incomplete").is_none(), "{second}");
    assert!(second.get("nextPageToken").is_none(), "{second}");
    assert_eq!(second["records"].as_array().unwrap().len(), 1, "{second}");
    assert_ne!(
        first["records"][0]["connectionId"]["value"],
        second["records"][0]["connectionId"]["value"]
    );
    assert_preserved_records(
        &[&first["records"][0], &second["records"][0]],
        &namespace,
        &owner_uid,
    );

    let replay = command(
        &endpoint,
        READER_B,
        &[
            "list-channel-messages",
            "--request",
            &channel_request(&namespace, "orders", Some(1), Some(token)).to_string(),
        ],
    );
    assert_eq!(
        replay.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay = outcome(&replay);
    assert_eq!(replay["failure"]["category"], "validation", "{replay}");
    assert_eq!(
        replay["failure"]["message"],
        "page token does not match this query"
    );
    assert!(replay["failure"].get("code").is_none(), "{replay}");
    assert!(!replay.to_string().contains(CHANNEL_TITLE));

    let missing = command(
        &endpoint,
        READER_A,
        &[
            "list-channel-messages",
            "--request",
            &channel_request(&namespace, "absent", None, None).to_string(),
        ],
    );
    let hidden = command(
        &endpoint,
        HIDDEN,
        &[
            "list-channel-messages",
            "--request",
            &channel_request(&namespace, "orders", None, None).to_string(),
        ],
    );
    assert_eq!(
        missing.status.code(),
        Some(6),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
    assert_eq!(
        hidden.status.code(),
        Some(6),
        "{}",
        String::from_utf8_lossy(&hidden.stderr)
    );
    let missing = outcome(&missing);
    let hidden = outcome(&hidden);
    assert_eq!(missing["failure"]["category"], "not_found");
    assert_eq!(
        hidden["failure"]["category"],
        missing["failure"]["category"]
    );
    assert_eq!(missing["failure"]["message"], "resource unavailable");
    assert_eq!(hidden["failure"]["message"], missing["failure"]["message"]);
    assert!(!missing.to_string().contains(CHANNEL_TITLE));
    assert!(!hidden.to_string().contains(CHANNEL_TITLE));
}
