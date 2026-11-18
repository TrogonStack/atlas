#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

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
fn cli(endpoint: &str, args: &[&str]) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_product_actual_cli_authoring_inspection_and_native_reapply_preserve_presence_and_identity(
) {
    let endpoint = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("offering.yaml");
    std::fs::write(
        &path,
        r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModel
metadata: {namespace: cli-product, name: receipt, version: 2}
spec: {title: Receipt}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DataProduct
metadata: {namespace: cli-product, name: offering}
spec:
  title: Offering
  inputs:
    - portId: {value: 11111111-1111-4111-8111-111111111111}
      name: {value: Receipt input}
      data: {readModel: receipt@2}
    - name: {value: Repeated input}
      data: {readModel: receipt@2}
    - name: {value: Undeclared input}
  outputs:
    - name: {value: Receipt}
      data: {readModel: receipt@2}
      derivedFrom:
        inputs:
          - inputPortId: {value: 11111111-1111-4111-8111-111111111111}
    - name: {value: Explicit empty}
      derivedFrom: {}
    - name: {value: Unknown lineage}
",
    )
    .unwrap();
    cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    let before = cli(&endpoint, &["get", "data_product/cli-product/offering@1"]);
    let parsed = trogon_atlas_client::manifest::parse_str(&before, "get.yaml").unwrap();
    let Some(trogon_atlas_proto::entity::Kind::DataProduct(product)) = &parsed[0].entity.kind
    else {
        panic!("product missing")
    };
    assert_eq!(
        product.inputs[0].port_id.as_ref().unwrap().value,
        "11111111-1111-4111-8111-111111111111"
    );
    assert_ne!(
        product.inputs[0].data.as_ref().unwrap().connection_id,
        product.inputs[1].data.as_ref().unwrap().connection_id
    );
    assert!(product.outputs[0].derived_from.is_some());
    assert_eq!(
        product.outputs[1].derived_from.as_ref().unwrap().inputs,
        Vec::<trogon_atlas_proto::LineageInput>::new()
    );
    assert!(product.outputs[2].derived_from.is_none());
    let inspection = cli(
        &endpoint,
        &[
            "inspect-data-product",
            "data_product/cli-product/offering@1",
        ],
    );
    let inspection: serde_json::Value = serde_json::from_str(&inspection).unwrap();
    assert_eq!(inspection["ports"].as_array().unwrap().len(), 6);
    assert!(inspection["scope"].is_object());
    let export = dir.path().join("export");
    cli(
        &endpoint,
        &[
            "export",
            "--namespace",
            "cli-product",
            "--output",
            export.to_str().unwrap(),
        ],
    );
    cli(&endpoint, &["apply", "-f", export.to_str().unwrap()]);
    assert_eq!(
        before,
        cli(&endpoint, &["get", "data_product/cli-product/offering@1"])
    );
}
