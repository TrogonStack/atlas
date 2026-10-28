#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "claims-cli-admin";
const WRITER: &str = "claims-cli-writer";
const READER: &str = "claims-cli-reader";
const SPECIFICATION_VERSION: &str = "9007199254740993";
const CHANNEL_VERSION: &str = "18446744073709551615";

async fn server() -> (String, String) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "admin":{"role":"admin","tokens":[ADMIN]},
            "writer":{"role":"writer","tokens":[WRITER],"parent":"claims-cli-owner"},
            "reader":{"role":"reader","tokens":[READER],"parent":"claims-cli-owner"}
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
        EventModelServiceServer::new(service),
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
    let namespace = trogon_atlas_client::client::connect(&endpoint, Some(ADMIN), 30)
        .await
        .unwrap()
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "claims-cli".into(),
            parent: "claims-cli-owner".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    (endpoint, namespace)
}

fn run(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args(["--endpoint", endpoint, "--auth-token", token])
        .args(args)
        .output()
        .unwrap()
}
fn ok(endpoint: &str, token: &str, args: &[&str]) -> String {
    let output = run(endpoint, token, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn rejected(endpoint: &str, token: &str, args: &[&str]) -> String {
    let output = run(endpoint, token, args);
    assert!(!output.status.success(), "{args:?} was accepted");
    String::from_utf8_lossy(&output.stderr).into_owned()
}
fn write(dir: &Path, name: &str, value: &Value) -> String {
    let path = dir.join(name);
    std::fs::write(&path, value.to_string()).unwrap();
    path.to_str().unwrap().to_owned()
}
fn manifest(kind: &str, ns: &str, name: &str, version: Option<&str>, spec: Value) -> Value {
    let mut metadata = json!({"namespace": ns, "name": name});
    if let Some(version) = version {
        metadata["version"] = json!(version);
    }
    let mut document = json!({"apiVersion": trogon_atlas_client::manifest::API_VERSION, "kind": kind, "metadata": metadata});
    document["spec"] = spec;
    document
}
fn entity_id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}
async fn native_get(endpoint: &str, kind: pb::EntityKind, id: pb::Id) -> pb::GetEntityResponse {
    trogon_atlas_client::client::connect(endpoint, Some(READER), 30)
        .await
        .unwrap()
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id),
        })
        .await
        .unwrap()
        .into_inner()
}
fn declared(output: &str) -> pb::MessageClaims {
    let entity = trogon_atlas_client::manifest::parse_str(output, "read.yaml")
        .unwrap()
        .remove(0)
        .entity;
    trogon_atlas_core::message_claims::declarations(&entity)
        .unwrap()
        .clone()
}
fn identity(value: Option<&pb::ConnectionId>) -> String {
    value.unwrap().value.clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_claims_cli_authoring_uses_returned_ids_and_preserves_every_leaf_through_reapply() {
    let (endpoint, ns) = server().await;
    let ns = ns.as_str();
    let dir = tempfile::tempdir().unwrap();
    let apply = |value: &Value, name: &str| {
        ok(
            &endpoint,
            WRITER,
            &["apply", "-f", &write(dir.path(), name, value)],
        )
    };

    apply(
        &manifest(
            "Schema",
            ns,
            "order",
            None,
            json!({"fields":[{"name":"customer","type":{"object":{"fields":[{"name":"name","type":{"string":{}}}]}}}]}),
        ),
        "schema.yaml",
    );
    apply(
        &manifest(
            "Specification",
            ns,
            "messages",
            Some(SPECIFICATION_VERSION),
            json!({"title":"Messages","messages":[
                {"name":{"value":"order-placed"},"schemas":[{"role":"ROLE_PAYLOAD","schema":{"id":{"namespace":ns,"slug":"order","version":"1"}}}]},
                {"name":{"value":"order-shipped"}}
            ]}),
        ),
        "specification.yaml",
    );
    apply(
        &manifest(
            "ReadModel",
            ns,
            "order-feed",
            Some("2"),
            json!({"title":"Order feed"}),
        ),
        "read-model.yaml",
    );
    apply(
        &manifest(
            "Processor",
            ns,
            "order-intake",
            None,
            json!({"title":"Order intake"}),
        ),
        "processor.yaml",
    );
    apply(
        &manifest(
            "AutomationSlice",
            ns,
            "observe-orders",
            Some("3"),
            json!({"title":"Observe orders","sourceReadModels":["order-feed@2"],"processor":"order-intake"}),
        ),
        "slice.yaml",
    );
    let slice = native_get(
        &endpoint,
        pb::EntityKind::AutomationSlice,
        entity_id(ns, "observe-orders", 3),
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(automation)) = slice.kind.as_ref() else {
        panic!("AutomationSlice expected")
    };
    let observation = json!({
        "owner":{"kind":"ENTITY_KIND_AUTOMATION_SLICE","id":{"namespace":ns,"slug":"observe-orders","version":"3"}},
        "ownerUid":slice.system.as_ref().unwrap().uid,
        "id":{"value":identity(automation.source_read_models[0].connection_id.as_ref())}
    });
    apply(
        &manifest(
            "Channel",
            ns,
            "orders",
            Some(CHANNEL_VERSION),
            json!({"title":"Orders",
                "address":{"template":"orders/{tenant}","parameters":[{"name":{"value":"tenant"},"allowedValues":{"values":["","eu"]}}]},
                "bindings":[{"observation":observation,"parameterValues":{"tenant":"eu"}}]}),
        ),
        "channel.yaml",
    );
    let owner = |claims: Value| {
        manifest(
            "Component",
            ns,
            "checkout",
            None,
            json!({"title":"Checkout","messageClaims":claims}),
        )
    };
    apply(&owner(json!({})), "owner-empty.yaml");

    let specification = native_get(
        &endpoint,
        pb::EntityKind::Specification,
        entity_id(ns, "messages", 9_007_199_254_740_993),
    )
    .await;
    let specification_uid = specification
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone();
    let Some(pb::entity::Kind::Specification(contract)) =
        specification.entity.as_ref().unwrap().kind.as_ref()
    else {
        panic!("Specification expected")
    };
    let placed = &contract.messages[0].member_id.as_ref().unwrap().value;
    let shipped = &contract.messages[1].member_id.as_ref().unwrap().value;
    let binding_id = identity(contract.messages[0].schemas[0].connection_id.as_ref());
    let message = |member: &str| json!({"specification":{"id":{"namespace":ns,"slug":"messages","version":SPECIFICATION_VERSION}},"ownerUid":specification_uid,"kind":"CONTRACT_MEMBER_KIND_MESSAGE","memberId":{"value":member}});
    let send = |member: &str| json!({"direction":"COMMUNICATION_DIRECTION_SEND","message":message(member)});
    let receive = json!({"direction":"COMMUNICATION_DIRECTION_RECEIVE","message":message(shipped)});

    let staged = owner(json!({"claims":[send(placed), send(placed), receive]}));
    let staged_path = write(dir.path(), "owner-claims.yaml", &staged);
    ok(
        &endpoint,
        WRITER,
        &["apply", "-f", &staged_path, "--dry-run"],
    );
    let absent = ok(
        &endpoint,
        READER,
        &["get", &format!("component/{ns}/checkout@1")],
    );
    assert!(declared(&absent).claims.is_empty(), "dry run mutated");
    ok(&endpoint, WRITER, &["apply", "-f", &staged_path]);
    let first = ok(
        &endpoint,
        READER,
        &["get", &format!("component/{ns}/checkout@1")],
    );
    let claims = declared(&first).claims;
    assert_eq!(claims.len(), 3);
    let ids: Vec<_> = claims
        .iter()
        .map(|claim| identity(claim.connection_id.as_ref()))
        .collect();
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3,
        "equal claims must receive distinct returned IDs: {ids:?}"
    );

    let claim_with_id = |index: usize, value: Value| {
        let mut value = value;
        value["connectionId"] = json!({"value": ids[index]});
        value
    };
    let full = owner(json!({
        "claims":[claim_with_id(0, send(placed)), claim_with_id(1, send(placed)), claim_with_id(2, receive.clone())],
        "routes":[
            {"claimId":{"value":ids[0]},"channel":format!("orders@{CHANNEL_VERSION}"),"parameterValues":{"tenant":""}},
            {"claimId":{"value":ids[0]},"channel":format!("orders@{CHANNEL_VERSION}"),"parameterValues":{"tenant":"eu"},"deliveryMode":"CLAIM_DELIVERY_MODE_PUSH_PULL"}
        ],
        "triggers":[{"claimId":{"value":ids[2]},"message":message(placed),"condition":{"value":"when shipped"}}],
        "fieldSelections":[{"claimId":{"value":ids[0]},
            "binding":{"owner":{"kind":"ENTITY_KIND_SPECIFICATION","id":format!("messages@{SPECIFICATION_VERSION}")},"ownerUid":specification_uid,"id":{"value":binding_id}},
            "selectors":[{"path":{"segments":["customer","name"]}},{"path":{"segments":["customer"]}}]}]
    }));
    let full_path = write(dir.path(), "owner-full.yaml", &full);
    ok(&endpoint, WRITER, &["apply", "-f", &full_path, "--dry-run"]);
    let unchanged = ok(
        &endpoint,
        READER,
        &["get", &format!("component/{ns}/checkout@1")],
    );
    assert!(declared(&unchanged).routes.is_empty(), "dry run mutated");
    ok(&endpoint, WRITER, &["apply", "-f", &full_path]);
    let stored = ok(
        &endpoint,
        READER,
        &["get", &format!("component/{ns}/checkout@1")],
    );
    let value = declared(&stored);
    assert_eq!(
        value
            .claims
            .iter()
            .map(|claim| identity(claim.connection_id.as_ref()))
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(value.routes.len(), 2);
    assert_ne!(value.routes[0].connection_id, value.routes[1].connection_id);
    assert!(value
        .routes
        .iter()
        .all(|route| route.connection_id.is_some()));
    assert_eq!(value.routes[0].delivery_mode, None);
    assert_eq!(
        value.routes[1].delivery_mode,
        Some(pb::ClaimDeliveryMode::PushPull as i32)
    );
    assert_eq!(value.routes[0].parameter_values["tenant"], "");
    assert_eq!(value.routes[1].parameter_values["tenant"], "eu");
    assert_eq!(
        value.routes[0]
            .channel
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        u64::MAX
    );
    assert!(value
        .routes
        .iter()
        .all(|route| identity(route.claim_id.as_ref()) == ids[0]));
    assert_eq!(identity(value.triggers[0].claim_id.as_ref()), ids[2]);
    assert!(value.triggers[0].connection_id.is_some());
    let selection = &value.field_selections[0];
    assert!(selection.connection_id.is_some());
    assert_eq!(identity(selection.claim_id.as_ref()), ids[0]);
    let selected = selection.binding.as_ref().unwrap();
    assert_eq!(selected.owner_uid, specification_uid);
    assert_eq!(identity(selected.id.as_ref()), binding_id);
    assert_eq!(
        selected
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        9_007_199_254_740_993
    );
    assert_eq!(selection.selectors.len(), 2);

    let described = ok(
        &endpoint,
        READER,
        &["describe", &format!("component/{ns}/checkout@1")],
    );
    for field in [
        "message_claims.claims[0].message.specification",
        "message_claims.claims[1].message.specification",
        "message_claims.triggers[0].message.specification",
        "message_claims.routes[0].channel",
        "message_claims.routes[1].channel",
        "message_claims.field_selections[0].binding.owner",
    ] {
        assert!(described.contains(field), "{field} missing:\n{described}");
    }
    let before = native_get(
        &endpoint,
        pb::EntityKind::Component,
        entity_id(ns, "checkout", 1),
    )
    .await;
    let export = dir.path().join("export");
    ok(
        &endpoint,
        READER,
        &[
            "export",
            "--namespace",
            ns,
            "--output",
            export.to_str().unwrap(),
        ],
    );
    ok(&endpoint, READER, &["diff", "-f", export.to_str().unwrap()]);
    ok(
        &endpoint,
        WRITER,
        &["apply", "-f", export.to_str().unwrap()],
    );
    let after = native_get(
        &endpoint,
        pb::EntityKind::Component,
        entity_id(ns, "checkout", 1),
    )
    .await;
    assert_eq!(after, before, "no-op reapply changed UID, etag or IDs");
    assert_eq!(
        stored,
        ok(
            &endpoint,
            READER,
            &["get", &format!("component/{ns}/checkout@1")]
        )
    );

    let mutate = |change: &dyn Fn(&mut Value)| {
        let mut candidate = full.clone();
        change(&mut candidate["spec"]["messageClaims"]);
        candidate
    };
    let wrong_variant = manifest(
        "Team",
        ns,
        "claims-team",
        None,
        json!({"title":{"value":"Team"},"messageClaims":full["spec"]["messageClaims"]}),
    );
    let cases: Vec<(&str, Value)> = vec![
        (
            "claims on a variant that cannot declare them",
            wrong_variant,
        ),
        (
            "selector with no alternative",
            mutate(&|claims| claims["fieldSelections"][0]["selectors"] = json!([{}])),
        ),
        (
            "route to a claim this owner does not declare",
            mutate(&|claims| claims["routes"][0]["claimId"] = json!({"value":"not-a-claim"})),
        ),
        (
            "trigger on a SEND claim",
            mutate(&|claims| claims["triggers"][0]["claimId"] = json!({"value":ids[0]})),
        ),
        (
            "channel parameter value outside the allowed set",
            mutate(&|claims| claims["routes"][0]["parameterValues"]["tenant"] = json!("us")),
        ),
        (
            "field selection binding on a different owner",
            mutate(&|claims| {
                claims["fieldSelections"][0]["binding"]["owner"] =
                    json!({"kind":"ENTITY_KIND_CHANNEL","id":format!("orders@{CHANNEL_VERSION}")});
            }),
        ),
        (
            "native path that is not a field of the schema",
            mutate(&|claims| {
                claims["fieldSelections"][0]["selectors"] =
                    json!([{"path":{"segments":["customer","missing"]}}]);
            }),
        ),
    ];
    for (name, candidate) in cases {
        rejected(
            &endpoint,
            WRITER,
            &[
                "apply",
                "-f",
                &write(dir.path(), "rejected.yaml", &candidate),
            ],
        );
        assert_eq!(
            native_get(
                &endpoint,
                pb::EntityKind::Component,
                entity_id(ns, "checkout", 1)
            )
            .await,
            before,
            "{name} changed stored state"
        );
    }
}
