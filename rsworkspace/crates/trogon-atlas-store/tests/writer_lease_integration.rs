//! Two `NatsStore` processes sharing one NATS `JetStream` instance,
//! proving single-writer fencing end to end: only one of them writes,
//! the other is fenced, and a standby takes over once the writer's lease
//! expires. See `docs/explanation/single-writer.md`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use trogon_atlas_core::WriterRole;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{NatsStore, NatsStoreConfig, Store, StoreError, WriteContext};

fn event(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

/// Both processes share every bucket/stream name, so they really are
/// contending for the same lease over the same data, not just connected
/// to the same server. Durations are shrunk well below the production
/// defaults so the takeover-after-expiry half of the test does not need
/// to wait out a real fifteen-second lease.
fn config(url: &str, suffix: &str, role: WriterRole) -> NatsStoreConfig {
    let mut config = NatsStoreConfig::new(url);
    config.bucket = format!("test-lease-{suffix}");
    config.branches_bucket = format!("test-lease-branches-{suffix}");
    config.changesets_bucket = format!("test-lease-changesets-{suffix}");
    config.revisions_bucket = format!("test-lease-revisions-{suffix}");
    config.batches_bucket = format!("test-lease-batches-{suffix}");
    config.namespaces_bucket = format!("test-lease-namespaces-{suffix}");
    config.lease_bucket = format!("test-lease-lease-{suffix}");
    config.stream = format!("TEST_LEASE_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.lease.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    config.role = role;
    config.lease_duration = Duration::from_millis(600);
    config.lease_renew_interval = Duration::from_millis(100);
    config
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_lease_holder_writes_and_a_standby_takes_over_after_expiry() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();

    let writer = NatsStore::connect_with(config(&nats.url, &suffix, WriterRole::Writer))
        .await
        .expect("writer connects");
    let standby = NatsStore::connect_with(config(&nats.url, &suffix, WriterRole::Standby))
        .await
        .expect("standby connects");

    // Give both lease loops a few ticks to settle: the writer should have
    // won the (uncontested) lease, and the standby should have observed it
    // without competing for it.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(writer.is_writer(), "writer must hold the lease");
    assert!(!standby.is_writer(), "standby must not hold the lease");

    let e = event("lease", "order-placed", 1, "Order placed");
    writer
        .create(pb::EntityKind::Event, &e, WriteContext::baseline())
        .await
        .expect("the lease holder may write");

    let fenced = standby
        .create(pb::EntityKind::Event, &e, WriteContext::baseline())
        .await
        .expect_err("a non-holder must be fenced");
    assert!(
        matches!(
            fenced,
            StoreError::NotWriter {
                role: WriterRole::Standby,
                ..
            }
        ),
        "unexpected error: {fenced:?}"
    );

    // Drop the writer: its background lease loop stops renewing, so its
    // lease goes stale and is eligible for takeover once it expires.
    drop(writer);
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    assert!(
        standby.is_writer(),
        "standby must take over once the old writer's lease expires"
    );
    let status = standby.writer_status();
    assert_eq!(status.epoch.get(), 2, "takeover must bump the epoch");

    standby
        .create(
            pb::EntityKind::Event,
            &event("lease", "order-shipped", 1, "Order shipped"),
            WriteContext::baseline(),
        )
        .await
        .expect("the new lease holder may write");
}
