#![allow(clippy::unwrap_used, clippy::expect_used)]

// Smoke test proving trogon-atlas's dependency wiring on trogon-atlas-client works
// end to end. The full apply/plan/export test suite lives in
// trogon-atlas-client/tests/apply_integration.rs.

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_client::{apply, client, manifest};
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

#[tokio::test]
async fn create_then_unchanged_cycle() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();

    let yaml = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: trogon-atlas-smoke, name: thing.happened }
spec: { title: Thing happened }
";

    let manifests = manifest::parse_str(yaml, "smoke.yaml").unwrap();
    let plan = apply::plan(&mut client, manifests).await.unwrap();
    let outcome = apply::apply(&mut client, plan, false, None).await.unwrap();
    assert_eq!(
        outcome.lines,
        vec!["event:trogon-atlas-smoke/thing.happened@1 created"]
    );

    let manifests = manifest::parse_str(yaml, "smoke.yaml").unwrap();
    let plan = apply::plan(&mut client, manifests).await.unwrap();
    let outcome = apply::apply(&mut client, plan, false, None).await.unwrap();
    assert_eq!(
        outcome.lines,
        vec!["event:trogon-atlas-smoke/thing.happened@1 unchanged"]
    );
}
