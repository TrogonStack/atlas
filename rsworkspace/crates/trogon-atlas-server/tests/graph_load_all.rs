#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use trogon_atlas_proto as pb;
use trogon_atlas_server::graph::load_all_with_cap;
use trogon_atlas_store::{Store, WriteContext};

fn event(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 1,
            }),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn load_all_flags_truncation_at_cap_boundary() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store: Arc<dyn Store> = Arc::new(nats.store().await);

    for slug in ["a", "b", "c"] {
        store
            .create(
                pb::EntityKind::Event,
                &event("shop", slug),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }

    let at_cap = load_all_with_cap(&store, 3, None).await.unwrap();
    assert_eq!(at_cap.entities.len(), 3);
    assert!(at_cap.truncated);

    let below_cap = load_all_with_cap(&store, 10, None).await.unwrap();
    assert_eq!(below_cap.entities.len(), 3);
    assert!(!below_cap.truncated);
}
