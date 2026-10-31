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

const SELECTION_OWNER: &str = "checkout";

struct Fixture {
    endpoint: String,
    ns: String,
    dir: tempfile::TempDir,
    specification_uid: String,
    placed: String,
    shipped: String,
    payload_binding: String,
    headers_binding: String,
}

impl Fixture {
    async fn new() -> Self {
        let (endpoint, ns) = server().await;
        let dir = tempfile::tempdir().unwrap();
        let apply = |value: &Value, name: &str| {
            ok(
                &endpoint,
                WRITER,
                &["apply", "-f", &write(dir.path(), name, value)],
            )
        };
        let ns_ref = ns.as_str();
        let fields = json!({"fields":[{"name":"customer","type":{"object":{"fields":[{"name":"name","type":{"string":{}}}]}}}]});
        apply(
            &manifest("Schema", ns_ref, "order", None, fields),
            "schema-order.yaml",
        );
        apply(
            &manifest(
                "Schema",
                ns_ref,
                "trace",
                None,
                json!({"fields":[{"name":"span","type":{"string":{}}}]}),
            ),
            "schema-trace.yaml",
        );
        apply(
            &manifest(
                "Specification",
                ns_ref,
                "messages",
                Some(SPECIFICATION_VERSION),
                json!({"title":"Messages","messages":[
                    {"name":{"value":"order-placed"},"schemas":[
                        {"role":"ROLE_PAYLOAD","schema":{"id":{"namespace":ns_ref,"slug":"order","version":"1"}}},
                        {"role":"ROLE_HEADERS","schema":{"id":{"namespace":ns_ref,"slug":"trace","version":"1"}}}
                    ]},
                    {"name":{"value":"order-shipped"}}
                ]}),
            ),
            "specification.yaml",
        );
        apply(
            &manifest(
                "ReadModel",
                ns_ref,
                "order-feed",
                Some("2"),
                json!({"title":"Order feed"}),
            ),
            "read-model.yaml",
        );
        apply(
            &manifest(
                "Processor",
                ns_ref,
                "order-intake",
                None,
                json!({"title":"Order intake"}),
            ),
            "processor.yaml",
        );
        apply(
            &manifest(
                "AutomationSlice",
                ns_ref,
                "observe-orders",
                Some("3"),
                json!({"title":"Observe orders","sourceReadModels":["order-feed@2"],"processor":"order-intake"}),
            ),
            "slice.yaml",
        );
        let slice = native_get(
            &endpoint,
            pb::EntityKind::AutomationSlice,
            entity_id(ns_ref, "observe-orders", 3),
        )
        .await
        .entity
        .unwrap();
        let Some(pb::entity::Kind::AutomationSlice(automation)) = slice.kind.as_ref() else {
            panic!("AutomationSlice expected")
        };
        let observation = json!({
            "owner":{"kind":"ENTITY_KIND_AUTOMATION_SLICE","id":{"namespace":ns_ref,"slug":"observe-orders","version":"3"}},
            "ownerUid":slice.system.as_ref().unwrap().uid,
            "id":{"value":identity(automation.source_read_models[0].connection_id.as_ref())}
        });
        for (slug, name) in [
            ("orders", "channel.yaml"),
            ("returns", "channel-returns.yaml"),
        ] {
            apply(
                &manifest(
                    "Channel",
                    ns_ref,
                    slug,
                    Some(CHANNEL_VERSION),
                    json!({"title":slug,
                        "address":{"template":format!("{slug}/{{tenant}}"),"parameters":[{"name":{"value":"tenant"},"allowedValues":{"values":["","eu"]}}]},
                        "bindings":[{"observation":observation,"parameterValues":{"tenant":"eu"}}]}),
                ),
                name,
            );
        }
        let specification = native_get(
            &endpoint,
            pb::EntityKind::Specification,
            entity_id(ns_ref, "messages", 9_007_199_254_740_993),
        )
        .await
        .entity
        .unwrap();
        let Some(pb::entity::Kind::Specification(contract)) = specification.kind.as_ref() else {
            panic!("Specification expected")
        };
        let placed = &contract.messages[0];
        let fixture = Self {
            endpoint: endpoint.clone(),
            ns: ns.clone(),
            specification_uid: specification.system.as_ref().unwrap().uid.clone(),
            placed: placed.member_id.as_ref().unwrap().value.clone(),
            shipped: contract.messages[1]
                .member_id
                .as_ref()
                .unwrap()
                .value
                .clone(),
            payload_binding: identity(placed.schemas[0].connection_id.as_ref()),
            headers_binding: identity(placed.schemas[1].connection_id.as_ref()),
            dir,
        };
        fixture
    }

    fn apply(&self, value: &Value, name: &str) -> String {
        ok(
            &self.endpoint,
            WRITER,
            &["apply", "-f", &write(self.dir.path(), name, value)],
        )
    }
    fn refuse(&self, value: &Value) -> String {
        rejected(
            &self.endpoint,
            WRITER,
            &[
                "apply",
                "-f",
                &write(self.dir.path(), "refused.yaml", value),
            ],
        )
    }
    fn owner(&self, kind: &str, name: &str, version: Option<&str>, claims: Value) -> Value {
        let mut spec = json!({"title":"Owner"});
        spec["messageClaims"] = claims;
        manifest(kind, &self.ns, name, version, spec)
    }
    fn message(&self, member: &str) -> Value {
        json!({"specification":{"id":{"namespace":self.ns,"slug":"messages","version":SPECIFICATION_VERSION}},"ownerUid":self.specification_uid,"kind":"CONTRACT_MEMBER_KIND_MESSAGE","memberId":{"value":member}})
    }
    fn send(&self, member: &str) -> Value {
        json!({"direction":"COMMUNICATION_DIRECTION_SEND","message":self.message(member)})
    }
    fn binding(&self, id: &str) -> Value {
        json!({"owner":{"kind":"ENTITY_KIND_SPECIFICATION","id":format!("messages@{SPECIFICATION_VERSION}")},"ownerUid":self.specification_uid,"id":{"value":id}})
    }
    async fn stored(
        &self,
        kind: pb::EntityKind,
        name: &str,
        version: u64,
    ) -> pb::GetEntityResponse {
        native_get(&self.endpoint, kind, entity_id(&self.ns, name, version)).await
    }
    async fn claims(&self, kind: pb::EntityKind, name: &str, version: u64) -> pb::MessageClaims {
        let entity = self.stored(kind, name, version).await.entity.unwrap();
        trogon_atlas_core::message_claims::declarations(&entity)
            .unwrap()
            .clone()
    }
}

fn claims_json(claims: &pb::MessageClaims) -> Value {
    trogon_atlas_core::transcode::TranscodePool::builtin()
        .unwrap()
        .message_json_value("trogonatlas.eventmodel.v1alpha1.MessageClaims", claims)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_claims_cli_selectors_stay_exact_and_headers_bindings_are_refused() {
    let fixture = Fixture::new().await;
    let placed = fixture.placed.as_str();
    fixture.apply(
        &fixture.owner(
            "Component",
            SELECTION_OWNER,
            None,
            json!({"claims":[fixture.send(placed), fixture.send(&fixture.shipped)]}),
        ),
        "owner-staged.yaml",
    );
    let staged = fixture
        .claims(pb::EntityKind::Component, SELECTION_OWNER, 1)
        .await;
    let claim =
        |index: usize| json!({"value": identity(staged.claims[index].connection_id.as_ref())});
    let claims = |selections: Value| {
        let mut value = claims_json(&staged);
        value["fieldSelections"] = selections;
        value
    };
    let selection = |binding: &str, selectors: Value| json!({"claimId":claim(0),"binding":fixture.binding(binding),"selectors":selectors});

    let selectors = json!([
        {"opaque":{"syntax":"json-pointer","value":"/customer/name"}},
        {"path":{"segments":["customer","name"]}},
        {"opaque":{"syntax":"json-pointer","value":"/customer/name"}},
        {"path":{"segments":["customer"]}}
    ]);
    fixture.apply(
        &fixture.owner(
            "Component",
            SELECTION_OWNER,
            None,
            claims(json!([selection(
                &fixture.payload_binding,
                selectors.clone()
            )])),
        ),
        "owner-selectors.yaml",
    );
    let stored = fixture
        .claims(pb::EntityKind::Component, SELECTION_OWNER, 1)
        .await;
    let kept = &stored.field_selections[0];
    assert_eq!(
        claims_json(&pb::MessageClaims {
            field_selections: vec![pb::ClaimFieldSelection {
                connection_id: None,
                ..kept.clone()
            }],
            ..Default::default()
        })["fieldSelections"][0]["selectors"],
        selectors,
        "selector kind, syntax, value and order must survive"
    );
    assert_eq!(kept.selectors.len(), 4);
    assert!(matches!(
        kept.selectors[0].selector,
        Some(pb::claim_field_selector::Selector::Opaque(_))
    ));
    assert!(matches!(
        kept.selectors[1].selector,
        Some(pb::claim_field_selector::Selector::Path(_))
    ));
    assert_eq!(kept.selectors[0], kept.selectors[2]);

    let before = fixture
        .stored(pb::EntityKind::Component, SELECTION_OWNER, 1)
        .await;
    let export = fixture.dir.path().join("export");
    ok(
        &fixture.endpoint,
        READER,
        &[
            "export",
            "--namespace",
            &fixture.ns,
            "--output",
            export.to_str().unwrap(),
        ],
    );
    ok(
        &fixture.endpoint,
        WRITER,
        &["apply", "-f", export.to_str().unwrap()],
    );
    assert_eq!(
        fixture
            .stored(pb::EntityKind::Component, SELECTION_OWNER, 1)
            .await,
        before,
        "exported selectors changed on reapply"
    );

    let keep_current = |extra: Value| {
        let mut value = claims_json(&stored);
        value["fieldSelections"].as_array_mut().unwrap().push(extra);
        value
    };
    let unknown = "01930000-0000-7000-8000-0000000000aa";
    let mut empty = selection(
        &fixture.payload_binding,
        json!([{"opaque":{"syntax":"json-pointer","value":""}}]),
    );
    empty["connectionId"] = json!({"value": identity(kept.connection_id.as_ref())});
    let target_invalid = "selected message claim target is unavailable or invalid";
    for (name, candidate, expected) in [
        (
            "HEADERS binding with a valid field path",
            keep_current(selection(
                &fixture.headers_binding,
                json!([{"path":{"segments":["span"]}}]),
            )),
            target_invalid,
        ),
        (
            "HEADERS binding with an opaque selector",
            keep_current(selection(
                &fixture.headers_binding,
                json!([{"opaque":{"syntax":"json-pointer","value":"/span"}}]),
            )),
            target_invalid,
        ),
        (
            "second selection for the same claim and binding",
            keep_current(selection(
                &fixture.payload_binding,
                json!([{"path":{"segments":["customer"]}}]),
            )),
            "duplicate claim binding selection",
        ),
        (
            "binding that is not a member of the Specification",
            keep_current(selection(
                unknown,
                json!([{"path":{"segments":["customer"]}}]),
            )),
            target_invalid,
        ),
        (
            "selection through a claim on another message binding",
            keep_current(
                json!({"claimId":claim(1),"binding":fixture.binding(&fixture.payload_binding),"selectors":[{"path":{"segments":["customer"]}}]}),
            ),
            target_invalid,
        ),
        (
            "empty opaque value on the existing selection",
            claims(json!([empty])),
            "invalid opaque selector",
        ),
    ] {
        let reason = fixture.refuse(&fixture.owner("Component", SELECTION_OWNER, None, candidate));
        assert!(
            reason.contains(expected),
            "{name} failed for another reason: {reason}"
        );
        assert_eq!(
            fixture
                .stored(pb::EntityKind::Component, SELECTION_OWNER, 1)
                .await,
            before,
            "{name} changed stored state"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_claims_cli_keeps_maximum_owner_versions_on_every_native_owner() {
    let fixture = Fixture::new().await;
    let placed = fixture.placed.as_str();
    for (kind, entity_kind, name) in [
        ("Component", pb::EntityKind::Component, "wide-component"),
        (
            "ExternalSystem",
            pb::EntityKind::ExternalSystem,
            "wide-external",
        ),
        ("Domain", pb::EntityKind::Domain, "wide-domain"),
    ] {
        let version = u64::MAX.to_string();
        fixture.apply(
            &fixture.owner(
                kind,
                name,
                Some(&version),
                json!({"claims":[fixture.send(placed), fixture.send(placed)]}),
            ),
            "wide-owner.yaml",
        );
        let stored = fixture.stored(entity_kind, name, u64::MAX).await;
        let entity = stored.entity.as_ref().unwrap();
        let claims = trogon_atlas_core::message_claims::declarations(entity).unwrap();
        assert_eq!(claims.claims.len(), 2, "{kind}");
        assert_ne!(
            claims.claims[0].connection_id, claims.claims[1].connection_id,
            "{kind}: equal claims must keep distinct IDs"
        );
        let message = claims.claims[0].message.as_ref().unwrap();
        assert_eq!(
            message
                .specification
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .version,
            9_007_199_254_740_993,
            "{kind}"
        );
        assert_eq!(message.owner_uid, fixture.specification_uid, "{kind}");

        let export = fixture.dir.path().join(format!("export-{name}"));
        ok(
            &fixture.endpoint,
            READER,
            &[
                "export",
                "--namespace",
                &fixture.ns,
                "--output",
                export.to_str().unwrap(),
            ],
        );
        ok(
            &fixture.endpoint,
            READER,
            &["diff", "-f", export.to_str().unwrap()],
        );
        ok(
            &fixture.endpoint,
            WRITER,
            &["apply", "-f", export.to_str().unwrap()],
        );
        assert_eq!(
            fixture.stored(entity_kind, name, u64::MAX).await,
            stored,
            "{kind}: exported reapply changed UID, etag or IDs"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_claims_cli_lifecycle_keeps_surviving_ids_and_refuses_referenced_removal() {
    let fixture = Fixture::new().await;
    let placed = fixture.placed.as_str();
    let channel = |slug: &str| format!("{slug}@{CHANNEL_VERSION}");
    let owner = |claims: &Value| fixture.owner("Component", SELECTION_OWNER, None, claims.clone());
    let current = || fixture.claims(pb::EntityKind::Component, SELECTION_OWNER, 1);
    let ids = |claims: &pb::MessageClaims| -> Vec<String> {
        claims
            .claims
            .iter()
            .map(|claim| identity(claim.connection_id.as_ref()))
            .collect()
    };

    fixture.apply(
        &owner(&json!({"claims":[fixture.send(placed), fixture.send(placed)]})),
        "lifecycle-staged.yaml",
    );
    let staged = current().await;
    let claim_ids = ids(&staged);
    let mut full = claims_json(&staged);
    full["routes"] = json!([
        {"claimId":{"value":claim_ids[0]},"channel":channel("orders"),"parameterValues":{"tenant":"eu"}},
        {"claimId":{"value":claim_ids[0]},"channel":channel("returns"),"parameterValues":{"tenant":""},"deliveryMode":"CLAIM_DELIVERY_MODE_PUSH"},
        {"claimId":{"value":claim_ids[1]},"channel":channel("orders"),"parameterValues":{"tenant":""}}
    ]);
    full["fieldSelections"] = json!([{"claimId":{"value":claim_ids[0]},"binding":fixture.binding(&fixture.payload_binding),"selectors":[{"path":{"segments":["customer"]}}]}]);
    fixture.apply(&owner(&full), "lifecycle-full.yaml");
    let applied = current().await;
    assert_eq!(ids(&applied), claim_ids);
    assert_eq!(applied.routes.len(), 3);
    let route_ids: Vec<_> = applied
        .routes
        .iter()
        .map(|route| identity(route.connection_id.as_ref()))
        .collect();
    let selection_id = identity(applied.field_selections[0].connection_id.as_ref());

    let applied_json = claims_json(&applied);
    let before = fixture
        .stored(pb::EntityKind::Component, SELECTION_OWNER, 1)
        .await;
    let mut dangling_route = applied_json.clone();
    dangling_route["claims"].as_array_mut().unwrap().remove(1);
    let mut dangling_selection = applied_json.clone();
    dangling_selection["claims"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    dangling_selection["routes"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    dangling_selection["routes"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    for (name, candidate) in [
        ("claim still named by a route", dangling_route),
        ("claim still named by a field selection", dangling_selection),
    ] {
        let reason = fixture.refuse(&owner(&candidate));
        assert!(
            reason.contains("local claim unavailable"),
            "{name} failed for another reason: {reason}"
        );
        assert_eq!(
            fixture
                .stored(pb::EntityKind::Component, SELECTION_OWNER, 1)
                .await,
            before,
            "{name} changed stored state"
        );
    }

    let mut without_first_route = applied_json.clone();
    without_first_route["routes"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    fixture.apply(&owner(&without_first_route), "lifecycle-route-removed.yaml");
    let after_route = current().await;
    assert_eq!(ids(&after_route), claim_ids);
    assert_eq!(
        after_route
            .routes
            .iter()
            .map(|route| identity(route.connection_id.as_ref()))
            .collect::<Vec<_>>(),
        route_ids[1..],
        "surviving routes must keep their returned IDs"
    );
    assert_eq!(
        identity(after_route.field_selections[0].connection_id.as_ref()),
        selection_id
    );

    let mut dependents_first = claims_json(&after_route);
    dependents_first["routes"]
        .as_array_mut()
        .unwrap()
        .retain(|route| route["claimId"]["value"] != json!(claim_ids[0]));
    dependents_first["fieldSelections"] = json!([]);
    fixture.apply(
        &owner(&dependents_first),
        "lifecycle-dependents-removed.yaml",
    );
    let after_dependents = current().await;
    assert_eq!(ids(&after_dependents), claim_ids);
    assert_eq!(after_dependents.routes.len(), 1);
    assert_eq!(
        identity(after_dependents.routes[0].connection_id.as_ref()),
        route_ids[2]
    );
    assert_eq!(after_dependents.field_selections, []);

    let mut claim_removed = claims_json(&after_dependents);
    claim_removed["claims"].as_array_mut().unwrap().remove(0);
    fixture.apply(&owner(&claim_removed), "lifecycle-claim-removed.yaml");
    let finished = current().await;
    assert_eq!(ids(&finished), claim_ids[1..]);
    assert_eq!(finished.claims[0], after_dependents.claims[1]);
    assert_eq!(
        identity(finished.routes[0].connection_id.as_ref()),
        route_ids[2]
    );
    assert_eq!(identity(finished.routes[0].claim_id.as_ref()), claim_ids[1]);

    let settled = fixture
        .stored(pb::EntityKind::Component, SELECTION_OWNER, 1)
        .await;
    let export = fixture.dir.path().join("export-lifecycle");
    ok(
        &fixture.endpoint,
        READER,
        &[
            "export",
            "--namespace",
            &fixture.ns,
            "--output",
            export.to_str().unwrap(),
        ],
    );
    ok(
        &fixture.endpoint,
        WRITER,
        &["apply", "-f", export.to_str().unwrap()],
    );
    assert_eq!(
        fixture
            .stored(pb::EntityKind::Component, SELECTION_OWNER, 1)
            .await,
        settled,
        "settled lifecycle state must be a no-op to reapply"
    );
}

fn cli_json(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            endpoint,
            "--auth-token",
            token,
            "--format",
            "json",
        ])
        .args(args)
        .output()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_claims_cli_source_native_owner_with_claims_is_refused_without_a_write() {
    let fixture = Fixture::new().await;
    let (endpoint, ns) = (fixture.endpoint.as_str(), fixture.ns.as_str());
    let registration = cli_json(
        endpoint,
        ADMIN,
        &[
            "create-source-registration",
            "--request",
            &json!({"registration":{"id":"claims-cli-source","administrationNamespace":ns,"destinationNamespaces":[ns],"profileRevision":1,"ownershipPolicyRevision":"policy","deletionPolicy":"SOURCE_DELETION_POLICY_RETAIN_NATIVE"}}).to_string(),
        ],
    );
    assert!(registration.status.success());
    let registration: Value = serde_json::from_slice(&registration.stdout).unwrap();
    let revision = registration["data"]["registration"]["revision"]
        .as_str()
        .unwrap();
    let inventory = write(
        fixture.dir.path(),
        "inventory.json",
        &json!({"profile":"supplied-native-v1","profile_revision":1,"registration":"claims-cli-source","source_version":"revision","partition":{"categories":["component"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL","resources":[{"key":{"category":"component","key":"claimant"},"file":"native/component.yaml","pointer":{"kind":"opaque","raw":"never-fetch"}}],"relations":[]}),
    );
    let payload = fixture.dir.path().join("component.yaml");
    std::fs::write(&payload, b"native: bytes\n").unwrap();
    let upload = cli_json(
        endpoint,
        WRITER,
        &[
            "upload-source-snapshot",
            "--registration-id",
            "claims-cli-source",
            "--registration-revision",
            revision,
            "--inventory",
            &inventory,
            "--file",
            &format!(
                "native/component.yaml=application/yaml={}",
                payload.display()
            ),
        ],
    );
    assert!(
        upload.status.success(),
        "{}",
        String::from_utf8_lossy(&upload.stderr)
    );
    let upload: Value = serde_json::from_slice(&upload.stdout).unwrap();
    let snapshot = upload["data"]["snapshot"]["id"].as_str().unwrap();
    let request = |run: &str| {
        json!({"registrationId":"claims-cli-source","registrationRevision":revision,"snapshotId":snapshot,"runId":run}).to_string()
    };
    let with_claims = write(
        fixture.dir.path(),
        "claimant-with-claims.yaml",
        &fixture.owner(
            "Component",
            "claimant",
            None,
            json!({"claims":[fixture.send(&fixture.placed)]}),
        ),
    );
    let plain = write(
        fixture.dir.path(),
        "claimant-plain.yaml",
        &manifest("Component", ns, "claimant", None, json!({"title":"Owner"})),
    );
    let preview = |run: &str, owner: &str| {
        cli_json(
            endpoint,
            WRITER,
            &[
                "preview-source-run",
                "--request",
                &request(run),
                "--native-owner",
                &format!("component=claimant={owner}"),
            ],
        )
    };

    let refused = preview("claims-run", &with_claims);
    assert!(
        !refused.status.success(),
        "a native owner carrying claims was previewed: {}",
        String::from_utf8_lossy(&refused.stdout)
    );
    assert!(
        String::from_utf8_lossy(&refused.stdout)
            .contains("message claims support native authoring only"),
        "refused for another reason: {}",
        String::from_utf8_lossy(&refused.stdout)
    );
    let missing_run = cli_json(
        endpoint,
        READER,
        &[
            "get-source-run",
            "--request",
            &json!({"registrationId":"claims-cli-source","runId":"claims-run"}).to_string(),
        ],
    );
    assert!(
        !missing_run.status.success(),
        "a refused preview left a run"
    );
    assert!(
        trogon_atlas_client::client::connect(endpoint, Some(READER), 30)
            .await
            .unwrap()
            .get_entity(pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(entity_id(ns, "claimant", 1)),
            })
            .await
            .is_err(),
        "a refused preview created the owner"
    );

    let control = preview("plain-run", &plain);
    assert!(
        control.status.success(),
        "the same owner without claims must preview: {}{}",
        String::from_utf8_lossy(&control.stdout),
        String::from_utf8_lossy(&control.stderr)
    );
    let control: Value = serde_json::from_slice(&control.stdout).unwrap();
    assert_eq!(
        control["data"]["run"]["status"],
        "SOURCE_RUN_STATUS_PREVIEWED"
    );
}
