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

#[tokio::test(flavor = "multi_thread")]
async fn stable_reader_and_standby_lists_enumerate_last_per_subject_consumers() {
    use std::collections::{BTreeMap, BTreeSet};

    use trogon_atlas_core::WriterRole;
    use trogon_atlas_store::store::ListFilter;
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let writer = NatsStore::connect_with(config.clone()).await.unwrap();
    let event = |index: usize, title: &str| pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "catalog".into(),
                slug: format!("term-{index}"),
                version: 1,
            }),
            title: title.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    for index in 0..8 {
        writer
            .create(
                pb::EntityKind::Event,
                &event(index, "baseline"),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    writer.create_branch("catalog", "").await.unwrap();
    writer
        .put(
            pb::EntityKind::Event,
            &event(0, "overlaid"),
            false,
            WriteContext::branch("catalog"),
        )
        .await
        .unwrap();
    writer
        .delete(
            pb::EntityKind::Event,
            &pb::Id {
                namespace: "catalog".into(),
                slug: "term-1".into(),
                version: 1,
            },
            None,
            WriteContext::branch("catalog"),
        )
        .await
        .unwrap();
    writer
        .create(
            pb::EntityKind::Event,
            &event(8, "branch-only"),
            WriteContext::branch("catalog"),
        )
        .await
        .unwrap();
    let observer = async_nats::connect(&nats.url).await.unwrap();
    let context = async_nats::jetstream::new(observer.clone());
    let mut streams = BTreeMap::new();
    for name in [&config.bucket, &config.branches_bucket] {
        let bucket = context.get_key_value(name).await.unwrap();
        assert!(bucket.stream.get_info().await.unwrap().config.allow_direct);
        streams.insert(bucket.stream_name.clone(), format!("{}>", bucket.prefix));
    }
    let mut readers = Vec::new();
    for role in [WriterRole::Reader, WriterRole::Standby] {
        let mut configuration = config.clone();
        configuration.role = role;
        readers.push(NatsStore::connect_with(configuration).await.unwrap());
    }
    let mut requests = observer.subscribe("$JS.API.>").await.unwrap();
    observer.flush().await.unwrap();
    for reader in &readers {
        for branch in [None, Some("catalog")] {
            let before = reader.read_marker().await.unwrap();
            let rows = reader
                .list(ListFilter::default(), None, branch)
                .await
                .unwrap();
            assert_eq!(before, reader.read_marker().await.unwrap());
            let actual: BTreeMap<_, _> = rows
                .into_iter()
                .map(|row| {
                    let event = match row.entity.kind {
                        Some(pb::entity::Kind::Event(event)) => Some(event),
                        _ => None,
                    }
                    .expect("event");
                    (event.id.unwrap().slug, event.title)
                })
                .collect();
            let mut expected: BTreeMap<_, _> = (0..8)
                .map(|index| (format!("term-{index}"), "baseline".to_string()))
                .collect();
            if branch.is_some() {
                expected.insert("term-0".into(), "overlaid".into());
                expected.remove("term-1");
                expected.insert("term-8".into(), "branch-only".into());
            }
            assert_eq!(actual, expected);
        }
    }
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        let mut observed = BTreeSet::new();
        let mut count = 0;
        while count < 6 {
            let message = requests.next().await.unwrap();
            if !message.subject.contains(".CONSUMER.CREATE.") {
                continue;
            }
            let payload: serde_json::Value = serde_json::from_slice(&message.payload).unwrap();
            let Some(stream) = payload["stream_name"].as_str() else {
                continue;
            };
            let Some(filter) = streams.get(stream) else {
                continue;
            };
            assert_eq!(payload["config"]["filter_subject"], filter.as_str());
            assert_eq!(payload["config"]["deliver_policy"], "last_per_subject");
            assert_eq!(payload["config"]["replay_policy"], "instant");
            assert_eq!(payload["config"]["headers_only"], true);
            observed.insert(stream.to_string());
            count += 1;
        }
        observed
    })
    .await
    .expect("observe all baseline and branch key-enumeration consumers");
    assert_eq!(observed, streams.keys().cloned().collect());
}
