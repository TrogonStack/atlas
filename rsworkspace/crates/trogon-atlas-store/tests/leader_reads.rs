#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use futures::StreamExt as _;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{NatsStore, Store, WriteContext};

#[tokio::test(flavor = "multi_thread")]
async fn entity_point_reads_use_leader_stream_api_even_when_direct_kv_is_enabled() {
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    let id = pb::Id {
        namespace: "shop".into(),
        slug: "leader-read".into(),
        version: 1,
    };
    let supplied = pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id.clone()),
            title: "Leader-backed read".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let written = store
        .create(pb::EntityKind::Event, &supplied, WriteContext::baseline())
        .await
        .unwrap();
    let observer = async_nats::connect(&nats.url).await.unwrap();
    let bucket = async_nats::jetstream::new(observer.clone())
        .get_key_value(&config.bucket)
        .await
        .unwrap();
    assert!(bucket.status().await.unwrap().info.config.allow_direct);
    let mut requests = observer.subscribe("$JS.API.>").await.unwrap();
    observer.flush().await.unwrap();
    let read = store.get(pb::EntityKind::Event, &id, None).await.unwrap();
    assert_eq!(read.entity, written.entity);
    assert_eq!(read.etag, written.etag);
    let expected_api = format!("$JS.API.STREAM.MSG.GET.{}", bucket.stream_name);
    let direct_api = format!("$JS.API.DIRECT.GET.{}", bucket.stream_name);
    let request = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let request = requests.next().await.unwrap();
            if request.subject.as_str() == expected_api
                || request.subject.as_str() == direct_api
                || request.subject.starts_with(&format!("{direct_api}."))
            {
                return request;
            }
        }
    })
    .await
    .expect("observe the successful entity read request");
    assert_eq!(request.subject.as_str(), expected_api);
    let payload: serde_json::Value = serde_json::from_slice(&request.payload).unwrap();
    assert_eq!(
        payload["last_by_subj"],
        format!(
            "{}{}",
            bucket.prefix,
            trogon_atlas_store::key::entity_key(pb::EntityKind::Event, &id)
        )
    );
}
