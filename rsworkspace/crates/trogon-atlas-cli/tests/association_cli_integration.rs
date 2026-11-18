#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::manifest;
use trogon_atlas_core::{channel_links, storage_claims};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const NS: &str = "cli-association";

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn run(endpoint: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .unwrap()
}

fn cli(endpoint: &str, args: &[&str]) -> String {
    let result = run(endpoint, args);
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

fn apply(endpoint: &str, dir: &Path, name: &str, document: &Value) {
    let path = dir.join(name);
    std::fs::write(&path, document.to_string()).unwrap();
    cli(endpoint, &["apply", "-f", path.to_str().unwrap()]);
}

#[allow(clippy::needless_pass_by_value)]
fn document(kind: &str, name: &str, spec: Value) -> Value {
    json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":NS,"name":name},"spec":spec})
}

async fn native(
    endpoint: &str,
    kind: pb::EntityKind,
    slug: &str,
) -> (
    pb::event_model_service_client::EventModelServiceClient<tonic::transport::Channel>,
    pb::Entity,
) {
    let mut client =
        pb::event_model_service_client::EventModelServiceClient::connect(endpoint.to_string())
            .await
            .unwrap();
    let entity = client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(pb::Id {
                namespace: NS.into(),
                slug: slug.into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    (client, entity)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_link_and_storage_claim_cli_authoring_inspect_export_and_reapply_keep_identity() {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    apply(
        &endpoint,
        dir.path(),
        "model.yaml",
        &document("ReadModel", "result", json!({"title":"Result"})),
    );
    apply(
        &endpoint,
        dir.path(),
        "worker.yaml",
        &document("Processor", "worker", json!({"title":"Worker"})),
    );
    apply(
        &endpoint,
        dir.path(),
        "observe.yaml",
        &document(
            "AutomationSlice",
            "observe",
            json!({"title":"Observe","sourceReadModels":[{"readModel":format!("{NS}/result")}],"processor":{"processor":format!("{NS}/worker")}}),
        ),
    );
    let (_, slice) = native(&endpoint, pb::EntityKind::AutomationSlice, "observe").await;
    let pb::entity::Kind::AutomationSlice(body) = slice.kind.as_ref().unwrap() else {
        panic!("AutomationSlice expected")
    };
    let observation = json!({"owner":{"kind":"ENTITY_KIND_AUTOMATION_SLICE","id":{"namespace":NS,"slug":"observe","version":"1"}},"ownerUid":slice.system.as_ref().unwrap().uid,"id":{"value":body.source_read_models[0].connection_id.as_ref().unwrap().value}});
    let channel = |name: &str, links: Value| {
        document(
            "Channel",
            name,
            json!({"title":name,"bindings":[{"observation":observation}],"links":links}),
        )
    };
    apply(
        &endpoint,
        dir.path(),
        "inner.yaml",
        &channel("inner", json!([])),
    );
    let link = |kind: &str, values: Value| json!({"kind":kind,"target":format!("{NS}/inner@1"),"parameterValues":values});
    let hub = channel(
        "hub",
        json!([
            link("CHANNEL_LINK_KIND_COMPOSES", json!({})),
            link("CHANNEL_LINK_KIND_COMPOSES", json!({})),
            link("CHANNEL_LINK_KIND_ROUTES_TO", json!({}))
        ]),
    );
    let hub_path = dir.path().join("hub.yaml");
    std::fs::write(&hub_path, hub.to_string()).unwrap();
    cli(
        &endpoint,
        &["apply", "-f", hub_path.to_str().unwrap(), "--dry-run"],
    );
    assert!(!run(&endpoint, &["get", &format!("channel/{NS}/hub@1")])
        .status
        .success());
    cli(&endpoint, &["apply", "-f", hub_path.to_str().unwrap()]);

    apply(
        &endpoint,
        dir.path(),
        "store.yaml",
        &document(
            "DataStore",
            "warehouse",
            json!({"title":"Warehouse","storeClass":{"known":"KNOWN_CACHE"},"accessMode":"ACCESS_MODE_READ_WRITE","attachments":[{"readModel":format!("{NS}/result")}]}),
        ),
    );
    let claims = |port: bool| {
        let mut read =
            json!({"store":format!("{NS}/warehouse@1"),"direction":"STORAGE_DIRECTION_READ"});
        if port {
            read["portId"] = json!({"value":"01930000-0000-7000-8000-0000000000aa"});
        }
        json!([read, {"store":format!("{NS}/warehouse@1"),"direction":"STORAGE_DIRECTION_WRITE"}, {"store":format!("{NS}/warehouse@1"),"direction":"STORAGE_DIRECTION_WRITE"}])
    };
    apply(
        &endpoint,
        dir.path(),
        "component.yaml",
        &document(
            "Component",
            "worker-app",
            json!({"title":"App","storageClaims":claims(false)}),
        ),
    );
    apply(
        &endpoint,
        dir.path(),
        "external.yaml",
        &document(
            "ExternalSystem",
            "vendor",
            json!({"title":"Vendor","storageClaims":claims(false)}),
        ),
    );
    apply(
        &endpoint,
        dir.path(),
        "product.yaml",
        &document(
            "DataProduct",
            "offering",
            json!({"title":"Offering","outputs":[{"portId":{"value":"01930000-0000-7000-8000-0000000000aa"},"name":{"value":"Result"}}],"storageClaims":claims(true)}),
        ),
    );

    let subjects = [
        (pb::EntityKind::Channel, "channel", "hub", 3usize),
        (pb::EntityKind::Component, "component", "worker-app", 3),
        (
            pb::EntityKind::ExternalSystem,
            "external-system",
            "vendor",
            3,
        ),
        (pb::EntityKind::DataProduct, "data-product", "offering", 3),
    ];
    let mut before = Vec::new();
    for (kind, noun, slug, count) in subjects {
        let text = cli(&endpoint, &["get", &format!("{noun}/{NS}/{slug}@1")]);
        let entity = manifest::parse_str(&text, "read.yaml")
            .unwrap()
            .remove(0)
            .entity;
        let ids: Vec<String> = if kind == pb::EntityKind::Channel {
            channel_links::links(&entity)
                .iter()
                .map(|item| item.connection_id.as_ref().unwrap().value.clone())
                .collect()
        } else {
            storage_claims::claims(&entity)
                .iter()
                .map(|item| item.connection_id.as_ref().unwrap().value.clone())
                .collect()
        };
        assert_eq!(ids.len(), count, "{slug}");
        assert!(ids[0] != ids[1] && ids[1] != ids[2], "{slug}: {ids:?}");
        let described = cli(&endpoint, &["describe", &format!("{noun}/{NS}/{slug}@1")]);
        for id in &ids {
            assert!(described.contains(id), "{slug}: {id}");
        }
        let (_, stored) = native(&endpoint, kind, slug).await;
        before.push((kind, slug, text, stored));
    }

    let export = dir.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            NS,
            "--output",
            export.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["diff", "-f", export.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]);
    for (kind, slug, text, stored) in before {
        let noun = match kind {
            pb::EntityKind::Channel => "channel",
            pb::EntityKind::Component => "component",
            pb::EntityKind::ExternalSystem => "external-system",
            _ => "data-product",
        };
        assert_eq!(
            text,
            cli(&endpoint, &["get", &format!("{noun}/{NS}/{slug}@1")]),
            "{slug}"
        );
        let (_, after) = native(&endpoint, kind, slug).await;
        assert_eq!(
            after, stored,
            "{slug}: uid, etag and ids survive a no-op reapply"
        );
    }
}

const DIFF_NS: &str = "cli-association-diff";
const MAX: u64 = u64::MAX;

#[allow(clippy::needless_pass_by_value)]
fn document_at(kind: &str, name: &str, version: u64, spec: Value) -> Value {
    json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":DIFF_NS,"name":name,"version":version},"spec":spec})
}

async fn native_at(endpoint: &str, kind: pb::EntityKind, slug: &str, version: u64) -> pb::Entity {
    let mut client =
        pb::event_model_service_client::EventModelServiceClient::connect(endpoint.to_string())
            .await
            .unwrap();
    client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(pb::Id {
                namespace: DIFF_NS.into(),
                slug: slug.into(),
                version,
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

fn apply_value(endpoint: &str, dir: &Path, name: &str, document: &Value) -> std::process::Output {
    let path = dir.join(name);
    std::fs::write(&path, document.to_string()).unwrap();
    run(endpoint, &["apply", "-f", path.to_str().unwrap()])
}

fn parsed(endpoint: &str, noun: &str, slug: &str, version: u64) -> pb::Entity {
    let text = cli(
        endpoint,
        &["get", &format!("{noun}/{DIFF_NS}/{slug}@{version}")],
    );
    manifest::parse_str(&text, "read.yaml")
        .unwrap()
        .remove(0)
        .entity
}

fn link_ids(entity: &pb::Entity) -> Vec<String> {
    channel_links::links(entity)
        .iter()
        .map(|item| item.connection_id.as_ref().unwrap().value.clone())
        .collect()
}

fn claim_ids(entity: &pb::Entity) -> Vec<String> {
    storage_claims::claims(entity)
        .iter()
        .map(|item| item.connection_id.as_ref().unwrap().value.clone())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn association_cli_authors_exact_u64_attachments_and_parameter_maps_diffs_real_changes_and_refuses_invalid_input(
) {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let ok = |name: &str, doc: &Value| {
        let output = apply_value(&endpoint, dir.path(), name, doc);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    ok(
        "model.yaml",
        &document_at("ReadModel", "result", 1, json!({"title":"Result"})),
    );
    ok(
        "worker.yaml",
        &document_at("Processor", "worker", 1, json!({"title":"Worker"})),
    );
    ok(
        "observe.yaml",
        &document_at(
            "AutomationSlice",
            "observe",
            1,
            json!({"title":"Observe","sourceReadModels":[{"readModel":format!("{DIFF_NS}/result")}],"processor":{"processor":format!("{DIFF_NS}/worker")}}),
        ),
    );
    let slice = native_at(&endpoint, pb::EntityKind::AutomationSlice, "observe", 1).await;
    let pb::entity::Kind::AutomationSlice(body) = slice.kind.as_ref().unwrap() else {
        panic!("AutomationSlice expected")
    };
    let observation = json!({"owner":{"kind":"ENTITY_KIND_AUTOMATION_SLICE","id":{"namespace":DIFF_NS,"slug":"observe","version":"1"}},"ownerUid":slice.system.as_ref().unwrap().uid,"id":{"value":body.source_read_models[0].connection_id.as_ref().unwrap().value}});
    let address = json!({"template":"orders.{tenant}","parameters":[{"name":{"value":"tenant"},"allowedValues":{"values":["acme","globex"]}}]});
    ok(
        "inner.yaml",
        &document_at(
            "Channel",
            "inner",
            MAX,
            json!({"title":"inner","address":address,"bindings":[{"observation":observation,"parameterValues":{"tenant":"acme"}}],"links":[]}),
        ),
    );
    let target = format!("{DIFF_NS}/inner@{MAX}");
    let hub = |links: Value| {
        document_at(
            "Channel",
            "hub",
            1,
            json!({"title":"hub","bindings":[{"observation":observation}],"links":links}),
        )
    };
    let authored_links = |second: &str| {
        json!([
            {"kind":"CHANNEL_LINK_KIND_COMPOSES","target":target,"parameterValues":{"tenant":"acme"}},
            {"kind":"CHANNEL_LINK_KIND_COMPOSES","target":target,"parameterValues":{"tenant":second}},
            {"kind":"CHANNEL_LINK_KIND_ROUTES_TO","target":target,"parameterValues":{"tenant":"acme"}}
        ])
    };
    ok("hub.yaml", &hub(authored_links("globex")));

    ok(
        "store.yaml",
        &document_at(
            "DataStore",
            "warehouse",
            1,
            json!({"title":"Warehouse","storeClass":{"known":"KNOWN_CACHE"},"accessMode":"ACCESS_MODE_READ_WRITE","attachments":[{"readModel":format!("{DIFF_NS}/result")}]}),
        ),
    );
    let store = native_at(&endpoint, pb::EntityKind::DataStore, "warehouse", 1).await;
    let pb::entity::Kind::DataStore(store_body) = store.kind.as_ref().unwrap() else {
        panic!("DataStore expected")
    };
    let attachment = json!({"owner":{"kind":"ENTITY_KIND_DATA_STORE","id":{"namespace":DIFF_NS,"slug":"warehouse","version":"1"}},"ownerUid":store.system.as_ref().unwrap().uid,"id":{"value":store_body.attachments[0].connection_id.as_ref().unwrap().value}});
    let warehouse = format!("{DIFF_NS}/warehouse@1");
    let port = "01930000-0000-7000-8000-0000000000aa";
    let component = |claims: Value| {
        document_at(
            "Component",
            "app",
            1,
            json!({"title":"App","storageClaims":claims}),
        )
    };
    let component_claims = |doc: Option<&str>| {
        let mut read =
            json!({"store":warehouse,"direction":"STORAGE_DIRECTION_READ","attachment":attachment});
        if let Some(doc) = doc {
            read["doc"] = json!({"value":doc});
        }
        json!([read, {"store":warehouse,"direction":"STORAGE_DIRECTION_WRITE"}])
    };
    ok("component.yaml", &component(component_claims(None)));
    ok(
        "product.yaml",
        &document_at(
            "DataProduct",
            "offering",
            1,
            json!({"title":"Offering","outputs":[{"portId":{"value":port},"name":{"value":"Result"}}],"storageClaims":[
                {"store":warehouse,"direction":"STORAGE_DIRECTION_READ","portId":{"value":port},"attachment":attachment}]}),
        ),
    );

    let stored_hub = parsed(&endpoint, "channel", "hub", 1);
    let links = channel_links::links(&stored_hub);
    assert_eq!(links.len(), 3);
    for item in links {
        assert_eq!(
            item.target.as_ref().unwrap().id.as_ref().unwrap().version,
            MAX
        );
    }
    let tenants: Vec<_> = links
        .iter()
        .map(|item| item.parameter_values.get("tenant").cloned().unwrap())
        .collect();
    assert_eq!(tenants, ["acme", "globex", "acme"]);
    for (noun, kind, slug) in [
        ("component", pb::EntityKind::Component, "app"),
        ("data-product", pb::EntityKind::DataProduct, "offering"),
    ] {
        let stored = parsed(&endpoint, noun, slug, 1);
        let claim = &storage_claims::claims(&stored)[0];
        let attached = claim.attachment.as_ref().unwrap();
        assert_eq!(
            attached.id.as_ref().unwrap().value,
            store_body.attachments[0]
                .connection_id
                .as_ref()
                .unwrap()
                .value,
            "{slug}"
        );
        assert_eq!(
            attached.owner_uid,
            store.system.as_ref().unwrap().uid,
            "{slug}"
        );
        let native = native_at(&endpoint, kind, slug, 1).await;
        assert_eq!(
            storage_claims::claims(&native)[0].attachment,
            claim.attachment.clone()
        );
    }
    assert_eq!(
        storage_claims::claims(&parsed(&endpoint, "data-product", "offering", 1))[0].port_id,
        Some(pb::DataPortId { value: port.into() })
    );

    let hub_before = native_at(&endpoint, pb::EntityKind::Channel, "hub", 1).await;
    let app_before = native_at(&endpoint, pb::EntityKind::Component, "app", 1).await;
    let changed_hub = dir.path().join("changed-hub.yaml");
    let exported_hub = cli(&endpoint, &["get", &format!("channel/{DIFF_NS}/hub@1")]);
    assert_eq!(exported_hub.matches("globex").count(), 1);
    std::fs::write(&changed_hub, exported_hub.replace("globex", "acme")).unwrap();
    let changed_app = dir.path().join("changed-app.yaml");
    let exported_app = cli(&endpoint, &["get", &format!("component/{DIFF_NS}/app@1")]);
    let read_claim = "    direction: STORAGE_DIRECTION_READ\n";
    assert_eq!(exported_app.matches(read_claim).count(), 1);
    std::fs::write(
        &changed_app,
        exported_app.replace(
            read_claim,
            &format!("{read_claim}    doc:\n      value: reads the cache\n"),
        ),
    )
    .unwrap();
    for path in [&changed_hub, &changed_app] {
        let diff = run(&endpoint, &["diff", "-f", path.to_str().unwrap()]);
        assert_eq!(
            diff.status.code(),
            Some(1),
            "{path:?}: a real change is a finding"
        );
        let text = String::from_utf8_lossy(&diff.stdout);
        assert!(!text.trim().is_empty(), "{path:?}: diff names the change");
    }
    let hub_diff =
        String::from_utf8(run(&endpoint, &["diff", "-f", changed_hub.to_str().unwrap()]).stdout)
            .unwrap();
    assert!(hub_diff.contains("globex"), "{hub_diff}");
    assert_eq!(
        native_at(&endpoint, pb::EntityKind::Channel, "hub", 1).await,
        hub_before,
        "diff never mutates"
    );

    cli(&endpoint, &["apply", "-f", changed_hub.to_str().unwrap()]);
    cli(&endpoint, &["apply", "-f", changed_app.to_str().unwrap()]);
    let hub_after = native_at(&endpoint, pb::EntityKind::Channel, "hub", 1).await;
    let app_after = native_at(&endpoint, pb::EntityKind::Component, "app", 1).await;
    assert_eq!(
        hub_after.system.as_ref().unwrap().uid,
        hub_before.system.as_ref().unwrap().uid
    );
    assert_ne!(hub_after, hub_before);
    assert_ne!(app_after, app_before);
    assert_eq!(
        link_ids(&hub_after),
        link_ids(&hub_before),
        "edited descriptive fields keep occurrence ids"
    );
    assert_eq!(claim_ids(&app_after), claim_ids(&app_before));
    assert_eq!(
        channel_links::links(&hub_after)[1]
            .parameter_values
            .get("tenant")
            .map(String::as_str),
        Some("acme")
    );
    assert_eq!(
        storage_claims::claims(&app_after)[0]
            .doc
            .as_ref()
            .map(|text| text.value.as_str()),
        Some("reads the cache")
    );
    for path in [&changed_hub, &changed_app] {
        cli(&endpoint, &["diff", "-f", path.to_str().unwrap()]);
    }

    let refusals = [
        (
            "self link",
            hub(json!([{"kind":"CHANNEL_LINK_KIND_COMPOSES","target":format!("{DIFF_NS}/hub@1")}])),
        ),
        (
            "missing target",
            hub(
                json!([{"kind":"CHANNEL_LINK_KIND_COMPOSES","target":format!("{DIFF_NS}/ghost@1")}]),
            ),
        ),
        (
            "undeclared parameter",
            hub(
                json!([{"kind":"CHANNEL_LINK_KIND_COMPOSES","target":target,"parameterValues":{"region":"eu"}}]),
            ),
        ),
        (
            "value outside the allowed set",
            hub(
                json!([{"kind":"CHANNEL_LINK_KIND_COMPOSES","target":target,"parameterValues":{"tenant":"initech"}}]),
            ),
        ),
        (
            "unspecified link kind",
            hub(
                json!([{"kind":"CHANNEL_LINK_KIND_UNSPECIFIED","target":target,"parameterValues":{"tenant":"acme"}}]),
            ),
        ),
        (
            "claim on a missing store",
            component(
                json!([{"store":format!("{DIFF_NS}/ghost-store@1"),"direction":"STORAGE_DIRECTION_READ"}]),
            ),
        ),
        (
            "port on a Component claim",
            component(
                json!([{"store":warehouse,"direction":"STORAGE_DIRECTION_READ","portId":{"value":port}}]),
            ),
        ),
        (
            "attachment the store never declared",
            component(
                json!([{"store":warehouse,"direction":"STORAGE_DIRECTION_READ","attachment":{"owner":attachment["owner"],"ownerUid":attachment["ownerUid"],"id":{"value":"01930000-0000-7000-8000-0000000000bb"}}}]),
            ),
        ),
        (
            "unspecified direction",
            component(json!([{"store":warehouse,"direction":"STORAGE_DIRECTION_UNSPECIFIED"}])),
        ),
    ];
    for (label, document) in refusals {
        let output = apply_value(&endpoint, dir.path(), "refused.yaml", &document);
        assert!(!output.status.success(), "{label} must be refused");
        assert!(
            !String::from_utf8_lossy(&output.stderr).trim().is_empty(),
            "{label}: refusal explains itself"
        );
    }
    assert_eq!(
        native_at(&endpoint, pb::EntityKind::Channel, "hub", 1).await,
        hub_after,
        "refusals are atomic"
    );
    assert_eq!(
        native_at(&endpoint, pb::EntityKind::Component, "app", 1).await,
        app_after,
        "refusals are atomic"
    );
}
