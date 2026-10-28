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
