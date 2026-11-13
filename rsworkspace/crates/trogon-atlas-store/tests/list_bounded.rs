#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message as _;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::ListFilter, EntityByteBudget, NatsStore, Store, StoreError, StoredEntity, WriteContext,
};

fn event(slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "shop".into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn encoded_bytes(rows: &[StoredEntity]) -> usize {
    rows.iter().map(|row| row.entity.encoded_len()).sum()
}

fn assert_same_rows(actual: &[StoredEntity], expected: &[StoredEntity]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.entity, expected.entity);
        assert_eq!(actual.etag, expected.etag);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_list_charges_all_fetched_versions_before_result_limit() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    for (slug, version) in [("b", 1), ("a", 2), ("a", 1)] {
        store
            .create(
                pb::EntityKind::Event,
                &event(slug, version, &"x".repeat(2048)),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    let all = store.list(ListFilter::default(), None, None).await.unwrap();
    let bytes = encoded_bytes(&all);
    let filter = ListFilter {
        latest_versions_only: true,
        ..Default::default()
    };
    let expected = store.list(filter.clone(), Some(1), None).await.unwrap();
    let bounded = store
        .list_bounded(filter.clone(), Some(1), None, EntityByteBudget::new(bytes))
        .await
        .unwrap();
    assert_same_rows(&bounded, &expected);
    assert!(matches!(
        store
            .list_bounded(filter, Some(1), None, EntityByteBudget::new(bytes - 1))
            .await,
        Err(StoreError::ResourceExhausted(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_list_shares_budget_with_branch_overlays_and_keeps_filters() {
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    let client = async_nats::connect(&nats.url).await.unwrap();
    let branches = async_nats::jetstream::new(client)
        .get_key_value(&config.branches_bucket)
        .await
        .unwrap();
    let mut bytes = 0;
    for slug in ["a", "b", "c"] {
        let written = store
            .create(
                pb::EntityKind::Event,
                &event(slug, 1, &"baseline".repeat(256)),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
        bytes += written.entity.encoded_len();
    }
    store.create_branch("bounded", "").await.unwrap();
    for (slug, version) in [("a", 2), ("b", 1), ("d", 1)] {
        store
            .put(
                pb::EntityKind::Event,
                &event(slug, version, &"branch".repeat(256)),
                true,
                WriteContext::branch("bounded"),
            )
            .await
            .unwrap();
    }
    store
        .delete(
            pb::EntityKind::Event,
            &pb::Id {
                namespace: "shop".into(),
                slug: "c".into(),
                version: 1,
            },
            None,
            WriteContext::branch("bounded"),
        )
        .await
        .unwrap();
    for ctx in [WriteContext::baseline(), WriteContext::branch("bounded")] {
        let mut excluded = event("excluded", 1, &"outside".repeat(4096));
        let Some(pb::entity::Kind::Event(inner)) = excluded.kind.as_mut() else {
            unreachable!()
        };
        inner.id.as_mut().unwrap().namespace = "other".into();
        store
            .put(pb::EntityKind::Event, &excluded, true, ctx)
            .await
            .unwrap();
    }
    let namespaces = ["shop".to_owned()];
    let filter = ListFilter {
        kinds: &[pb::EntityKind::Event],
        namespaces: &namespaces,
        latest_versions_only: true,
        ..Default::default()
    };
    for delta in store.list_branch_deltas("bounded").await.unwrap() {
        if delta.id.namespace == "shop" {
            let key = trogon_atlas_store::key::branch_delta_key("bounded", delta.kind, &delta.id);
            bytes += branches.get(key).await.unwrap().unwrap().len();
        }
    }
    let expected = store
        .list(filter.clone(), Some(2), Some("bounded"))
        .await
        .unwrap();
    let bounded = store
        .list_bounded(
            filter.clone(),
            Some(2),
            Some("bounded"),
            EntityByteBudget::new(bytes),
        )
        .await
        .unwrap();
    assert_same_rows(&bounded, &expected);
    assert!(matches!(
        store
            .list_bounded(
                filter,
                Some(2),
                Some("bounded"),
                EntityByteBudget::new(bytes - 1)
            )
            .await,
        Err(StoreError::ResourceExhausted(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_list_checks_wire_bytes_before_decoding_a_baseline_entity() {
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    let client = async_nats::connect(&nats.url).await.unwrap();
    let kv = async_nats::jetstream::new(client)
        .get_key_value(&config.bucket)
        .await
        .unwrap();
    let key = trogon_atlas_store::key::entity_key(
        pb::EntityKind::Event,
        &pb::Id {
            namespace: "shop".into(),
            slug: "oversized".into(),
            version: 1,
        },
    );
    kv.put(key, vec![0xff; 1024].into()).await.unwrap();
    assert!(matches!(
        store
            .list_bounded(
                ListFilter::default(),
                None,
                None,
                EntityByteBudget::new(1023)
            )
            .await,
        Err(StoreError::ResourceExhausted(_))
    ));
    assert!(matches!(
        store.list(ListFilter::default(), None, None).await,
        Err(StoreError::CorruptEntry { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_list_charges_retained_branch_tombstone_bytes_after_baseline_deletion() {
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    let client = async_nats::connect(&nats.url).await.unwrap();
    let branches = async_nats::jetstream::new(client)
        .get_key_value(&config.branches_bucket)
        .await
        .unwrap();
    let target = pb::Id {
        namespace: "shop".into(),
        slug: "retained-base".into(),
        version: 1,
    };
    store
        .create(
            pb::EntityKind::Event,
            &event("retained-base", 1, &"large baseline".repeat(4096)),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store.create_branch("tombstone", "").await.unwrap();
    store
        .delete(
            pb::EntityKind::Event,
            &target,
            None,
            WriteContext::branch("tombstone"),
        )
        .await
        .unwrap();
    store
        .delete(
            pb::EntityKind::Event,
            &target,
            None,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let key =
        trogon_atlas_store::key::branch_delta_key("tombstone", pb::EntityKind::Event, &target);
    let bytes = branches.get(key).await.unwrap().unwrap();
    let delta = pb::BranchDelta::decode(bytes.clone()).unwrap();
    assert!(delta.tombstone);
    assert!(delta.base.is_some());
    assert!(delta.ours.is_none());
    assert!(matches!(
        store
            .list_bounded(
                ListFilter::default(),
                None,
                Some("tombstone"),
                EntityByteBudget::new(0),
            )
            .await,
        Err(StoreError::ResourceExhausted(_))
    ));
    assert_eq!(
        store
            .list_bounded(
                ListFilter::default(),
                None,
                Some("tombstone"),
                EntityByteBudget::new(bytes.len()),
            )
            .await
            .unwrap()
            .len(),
        0
    );
    assert!(matches!(
        store
            .list_bounded(
                ListFilter::default(),
                None,
                Some("tombstone"),
                EntityByteBudget::new(bytes.len() - 1),
            )
            .await,
        Err(StoreError::ResourceExhausted(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_list_checks_branch_wire_bytes_before_decoding() {
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    store.create_branch("corrupt", "").await.unwrap();
    let client = async_nats::connect(&nats.url).await.unwrap();
    let branches = async_nats::jetstream::new(client)
        .get_key_value(&config.branches_bucket)
        .await
        .unwrap();
    let key = trogon_atlas_store::key::branch_delta_key(
        "corrupt",
        pb::EntityKind::Event,
        &pb::Id {
            namespace: "shop".into(),
            slug: "oversized".into(),
            version: 1,
        },
    );
    branches.put(key, vec![0xff; 1024].into()).await.unwrap();
    assert!(matches!(
        store
            .list_bounded(
                ListFilter::default(),
                None,
                Some("corrupt"),
                EntityByteBudget::new(1023),
            )
            .await,
        Err(StoreError::ResourceExhausted(_))
    ));
    assert!(matches!(
        store
            .list(ListFilter::default(), None, Some("corrupt"))
            .await,
        Err(StoreError::Backend(_))
    ));
}
