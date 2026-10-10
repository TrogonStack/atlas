#![allow(clippy::unwrap_used, clippy::expect_used)]

// export_namespace_openslo: live store -> OpenSLO v1 YAML. Covers the one
// branch the pure-conversion golden test (openslo_conversion.rs) cannot:
// `fetch_latest_namespace_entities`'s own ListEntities paging/filtering
// against a real server, end to end through `trogon-atlas`'s connect path.

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_client::{client, openslo, openslo_bindings::BindingsConfig};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::service::EventModelServiceImpl;

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn put_req(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    }
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn bindings() -> BindingsConfig {
    BindingsConfig::parse(
        r#"
bindings:
  - signal:
      source: telemetry
      kind: span
      name: checkout.duration
    dataSource:
      name: tracing-backend
      type: Datadog
      connectionDetails:
        site: datadoghq.com
    metricSource:
      threshold:
        query: "p95:trace.checkout.duration{service:checkout}"
"#,
    )
    .unwrap()
}

#[tokio::test]
async fn export_namespace_openslo_fetches_live_entities_and_renders_yaml() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    client
        .put_entity(put_req(pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ServiceLevelIndicator(
                pb::ServiceLevelIndicator {
                    id: Some(id("openslo-live", "checkout-latency")),
                    title: "Checkout latency".into(),
                    connection: Some(pb::Connection {
                        kind: Some(pb::connection::Kind::CommandHandling(
                            pb::connection::CommandHandling {
                                command_slice: Some(pb::SliceRef {
                                    id: Some(id("openslo-live", "place-order")),
                                }),
                                events: vec![],
                            },
                        )),
                    }),
                    measure: Some(pb::service_level_indicator::Measure::Latency(
                        pb::service_level_indicator::Latency {},
                    )),
                    signal: Some(pb::Signal {
                        source: Some(pb::signal::Source::Telemetry(pb::signal::Telemetry {
                            kind: pb::TelemetryKind::Span as i32,
                            name: "checkout.duration".into(),
                        })),
                    }),
                    ..Default::default()
                },
            )),
        }))
        .await
        .unwrap();

    let outcome =
        openslo::export_namespace_openslo(&mut client, "openslo-live", &bindings(), false)
            .await
            .unwrap();
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert!(outcome.yaml.contains("name: openslo-live-checkout-latency"));
    assert!(outcome.yaml.contains("metricSourceRef: tracing-backend"));
    assert!(outcome.yaml.contains("kind: DataSource"));
}

#[tokio::test]
async fn export_namespace_openslo_empty_namespace_errors() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let err =
        openslo::export_namespace_openslo(&mut client, "openslo-empty-ns", &bindings(), false)
            .await
            .unwrap_err();
    assert!(format!("{err:#}").contains("no entities"), "{err:#}");
}
