#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeSet, sync::Arc};

use trogon_atlas_core::NamespaceId;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{ownership::Lens, service::get_latest_version_with_cap};
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

/// A bound caller's own cap budget must not be consumed by entities in
/// namespaces it cannot even see. Three decoys in a namespace that sorts
/// before the caller's own exhaust a cap of two before the caller's entity
/// is ever reached, unless the store is only ever asked to list the
/// caller's own visible namespaces.
#[tokio::test(flavor = "multi_thread")]
async fn get_latest_version_does_not_truncate_a_visible_entity_behind_invisible_ones() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store: Arc<dyn Store> = Arc::new(nats.store().await);

    for slug in ["a", "b", "c"] {
        store
            .create(
                pb::EntityKind::Event,
                &event("aaa-attacker", slug),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    store
        .create(
            pb::EntityKind::Event,
            &event("zzz-victim", "needle"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let mut visible = BTreeSet::new();
    visible.insert(NamespaceId::parse("zzz-victim").unwrap());
    let lens = Lens::Only(Arc::new(visible));

    let found =
        get_latest_version_with_cap(&store, &lens, pb::EntityKind::Event, "", "needle", 2, None)
            .await
            .unwrap();

    assert!(
        found.is_some(),
        "the caller's own entity must not be truncated away by decoys it cannot see",
    );
}
