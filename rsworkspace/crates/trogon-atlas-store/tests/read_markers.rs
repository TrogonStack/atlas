#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{NamespaceName, OwnerId, WriterRole};
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::MutationOp, ChangesetRecord, ChangesetRef, ChangesetValidation, NatsStore, Store,
    StoreError, WriteContext,
};

fn event(slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "marker".into(),
                slug: slug.into(),
                version: 1,
            }),
            title: title.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn read_markers_follow_foreign_model_and_namespace_mutations() {
    for role in [WriterRole::Reader, WriterRole::Standby] {
        let nats = trogon_atlas_testsupport::shared().await;
        let config = nats.store_config();
        let writer = NatsStore::connect_with(config.clone()).await.unwrap();
        writer
            .create(
                pb::EntityKind::Event,
                &event("one", "before"),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
        let mut read_config = config;
        read_config.role = role;
        let reader = NatsStore::connect_with(read_config).await.unwrap();
        let before = reader.read_marker().await.unwrap();
        assert_eq!(before, reader.read_marker().await.unwrap());
        writer
            .put(
                pb::EntityKind::Event,
                &event("one", "after"),
                false,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
        let after = reader.read_marker().await.unwrap();
        assert_ne!(before, after);
        assert_eq!(before.namespace_marker(), after.namespace_marker());
        writer.create_branch("marker-branch", "").await.unwrap();
        let branch = reader.read_marker().await.unwrap();
        assert_ne!(branch, after);
        writer
            .put(
                pb::EntityKind::Event,
                &event("one", "branch-value"),
                false,
                WriteContext::branch("marker-branch"),
            )
            .await
            .unwrap();
        let delta = reader.read_marker().await.unwrap();
        assert_ne!(branch, delta);
        assert_eq!(branch.namespace_marker(), delta.namespace_marker());
        writer
            .register_namespace(
                &NamespaceName::parse("registered").unwrap(),
                &OwnerId::parse("team").unwrap(),
                "marker-test",
            )
            .await
            .unwrap();
        let registered = reader.read_marker().await.unwrap();
        assert_ne!(registered.namespace_marker(), branch.namespace_marker());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn read_markers_refuse_published_branch_journal_until_changeset_append_releases_it() {
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let writer = NatsStore::connect_with(config.clone()).await.unwrap();
    writer.create_branch("pending", "").await.unwrap();
    let mut reader_config = config.clone();
    reader_config.role = WriterRole::Reader;
    let reader = NatsStore::connect_with(reader_config).await.unwrap();
    let before = reader.read_marker().await.unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let changeset = ChangesetRef {
        id: &id,
        author: "marker-test",
        rpc: "BatchMutate",
        operation_id: None,
    };
    writer
        .batch_apply(
            &[MutationOp::Create {
                kind: pb::EntityKind::Event,
                entity: event("branch-one", "candidate"),
            }],
            WriteContext::branch("pending").attributed(changeset),
        )
        .await
        .unwrap();
    assert!(matches!(
        reader.read_marker().await,
        Err(StoreError::Unavailable(_))
    ));
    let client = async_nats::connect(&nats.url).await.unwrap();
    let journal = async_nats::jetstream::new(client)
        .get_key_value(&config.batches_bucket)
        .await
        .unwrap();
    let published = journal.stream.get_info().await.unwrap();
    assert!(published.state.messages > 0);
    writer
        .append_changeset(&ChangesetRecord {
            id,
            author: "marker-test".into(),
            at: chrono::Utc::now().to_rfc3339(),
            message: "branch mutation".into(),
            rpc: "BatchMutate".into(),
            branch: Some("pending".into()),
            operation_id: None,
            validation: ChangesetValidation::NotRecorded,
            ops: Vec::new(),
        })
        .await
        .unwrap();
    let released = journal.stream.get_info().await.unwrap();
    assert_eq!(released.state.messages, 0);
    assert_eq!(released.state.last_sequence, published.state.last_sequence);
    assert_eq!(released.created, published.created);
    let after = reader.read_marker().await.unwrap();
    assert_ne!(before, after);
    assert_eq!(after, reader.read_marker().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn read_markers_ignore_lease_heartbeat_revisions() {
    use std::time::Duration;

    use futures::StreamExt as _;
    let nats = trogon_atlas_testsupport::shared().await;
    let mut config = nats.store_config();
    config.lease_duration = Duration::from_millis(600);
    config.lease_renew_interval = Duration::from_millis(50);
    let writer = NatsStore::connect_with(config.clone()).await.unwrap();
    let client = async_nats::connect(&nats.url).await.unwrap();
    let lease = async_nats::jetstream::new(client)
        .get_key_value(&config.lease_bucket)
        .await
        .unwrap();
    let mut watch = lease.watch("writer-lease").await.unwrap();
    let before = writer.read_marker().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), watch.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(writer.read_marker().await.unwrap(), before);
}
