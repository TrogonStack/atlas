#![allow(clippy::unwrap_used, clippy::expect_used)]

// ExportEventModelManifests: a board rendered as manifests someone else can
// apply. Pins what the board carries (its own closure, foreign entities it
// references once, nothing it does not reach) and that the output loads
// back with no drift.

use std::{collections::BTreeSet, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_client::{apply, client, manifest};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
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

const PAYMENTS: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Swimlane
metadata: { namespace: board-pay, name: payments }
spec: { title: Payments }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: board-pay, name: payment.captured }
spec: { title: Payment captured, swimlane: payments }
";

const SHOP: &str = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Persona
metadata: { namespace: board-src, name: buyer }
spec: { title: Buyer }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Ui
metadata: { namespace: board-src, name: checkout }
spec: { title: Checkout }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Swimlane
metadata: { namespace: board-src, name: order }
spec: { title: Order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: board-src, name: order.placed }
spec:
  title: Order placed
  swimlane: order
  metadata:
    - '@type': type.googleapis.com/trogonatlas.annotation.v1alpha1.TagAnnotation
      tags: ['context:shop']
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: board-src, name: not-on-the-board }
spec: { title: Not on the board }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: board-src, name: place-order }
spec: { title: Place order }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: board-src, name: place-order-slice }
spec:
  title: Place order
  persona: buyer
  ui: checkout
  command: place-order
  emittedEvents: [order.placed]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModel
metadata: { namespace: board-src, name: paid-orders }
spec:
  title: Paid orders
  sourceEvents: [board-pay/payment.captured]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ReadModelSlice
metadata: { namespace: board-src, name: paid-orders-slice }
spec:
  title: Paid orders
  readModel: paid-orders
  sourceEvents: [board-pay/payment.captured]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Storyboard
metadata: { namespace: board-src, name: buy-flow }
spec:
  title: Buy flow
  entry:
    human:
      persona: buyer
      ui: checkout
  slices:
    - id: place-order-slice
    - id: paid-orders-slice
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: board-src, name: shop }
spec:
  title: Shop
  members:
    - { kind: storyboard, id: buy-flow }
";

async fn seed(client: &mut client::Client, yaml: &str) {
    let docs = manifest::parse_str(yaml, "seed").unwrap();
    let plan = apply::plan(client, docs).await.unwrap();
    apply::apply(client, plan, false, None).await.unwrap();
}

async fn export(
    client: &mut client::Client,
    namespace: &str,
) -> pb::ExportEventModelManifestsResponse {
    client
        .export_event_model_manifests(pb::ExportEventModelManifestsRequest {
            id: Some(pb::Id {
                namespace: namespace.into(),
                slug: "shop".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner()
}

fn names(yaml: &str) -> BTreeSet<String> {
    manifest::parse_str(yaml, "export")
        .unwrap()
        .iter()
        .map(|doc| format!("{}/{}", doc.id.namespace, doc.id.slug))
        .collect()
}

#[tokio::test]
async fn board_manifests_carry_the_board_and_load_back_without_drift() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    seed(&mut client, PAYMENTS).await;
    seed(&mut client, SHOP).await;

    let out = export(&mut client, "board-src").await;
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);
    assert!(!out.truncated);

    let own: BTreeSet<String> = [
        "buy-flow",
        "buyer",
        "checkout",
        "order",
        "order.placed",
        "paid-orders",
        "paid-orders-slice",
        "place-order",
        "place-order-slice",
        "shop",
    ]
    .iter()
    .map(|slug| format!("board-src/{slug}"))
    .collect();
    assert_eq!(
        names(&out.manifests),
        own,
        "an entity the board does not reach stays out"
    );
    assert_eq!(
        names(&out.external_manifests),
        BTreeSet::from(["board-pay/payment.captured".to_string()]),
        "a foreign entity comes once, without its own swimlane",
    );
    assert!(
        out.manifests.contains("TagAnnotation") && out.manifests.contains("context:shop"),
        "annotations render as typed values:\n{}",
        out.manifests
    );

    let mut docs = manifest::parse_str(&out.manifests, "own").unwrap();
    docs.extend(manifest::parse_str(&out.external_manifests, "external").unwrap());
    for change in apply::plan(&mut client, docs).await.unwrap() {
        assert_eq!(
            change.action,
            apply::Action::Unchanged,
            "{} drifted from the store it was exported from",
            change.manifest.source
        );
    }

    // Loading the board into another namespace reproduces it exactly.
    seed(
        &mut client,
        &out.manifests.replace("board-src", "board-dst"),
    )
    .await;
    let copied = export(&mut client, "board-dst").await;
    assert_eq!(
        copied.manifests,
        out.manifests.replace("board-src", "board-dst")
    );
    assert_eq!(copied.external_manifests, out.external_manifests);
}

#[tokio::test]
async fn a_missing_event_model_is_not_found() {
    let endpoint = start_server().await;
    let mut client = client::connect(&endpoint, None, 30).await.unwrap();
    let status = client
        .export_event_model_manifests(pb::ExportEventModelManifestsRequest {
            id: Some(pb::Id {
                namespace: "board-none".into(),
                slug: "shop".into(),
                version: 1,
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
}
