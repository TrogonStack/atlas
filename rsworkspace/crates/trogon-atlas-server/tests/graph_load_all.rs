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

async fn store_with(slugs: &[&str]) -> Arc<dyn Store> {
    let nats = trogon_atlas_testsupport::shared().await;
    let store: Arc<dyn Store> = Arc::new(nats.store().await);
    for slug in slugs {
        store
            .create(
                pb::EntityKind::Event,
                &event("shop", slug),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    store
}

#[tokio::test(flavor = "multi_thread")]
async fn load_all_flags_truncation_only_above_the_cap() {
    let store = store_with(&["a", "b", "c"]).await;

    let below_cap = load_all_with_cap(&store, 4, None).await.unwrap();
    assert_eq!(below_cap.entities.len(), 3);
    assert!(!below_cap.truncated);

    let at_cap = load_all_with_cap(&store, 3, None).await.unwrap();
    assert_eq!(at_cap.entities.len(), 3);
    assert!(
        !at_cap.truncated,
        "a view holding exactly the cap is complete"
    );

    let above_cap = load_all_with_cap(&store, 2, None).await.unwrap();
    assert_eq!(above_cap.entities.len(), 2, "never more than the cap");
    assert!(above_cap.truncated);
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_overlay_load_flags_truncation_only_above_the_cap() {
    let store = store_with(&["a", "b"]).await;
    store.create_branch("overlay", "").await.unwrap();
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "c"),
            WriteContext::branch("overlay"),
        )
        .await
        .unwrap();

    let at_cap = load_all_with_cap(&store, 3, Some("overlay")).await.unwrap();
    assert_eq!(at_cap.entities.len(), 3);
    assert!(!at_cap.truncated);

    let above_cap = load_all_with_cap(&store, 2, Some("overlay")).await.unwrap();
    assert_eq!(above_cap.entities.len(), 2);
    assert!(above_cap.truncated);

    let baseline = load_all_with_cap(&store, 2, None).await.unwrap();
    assert!(!baseline.truncated, "the overlay entity is not on baseline");
}
