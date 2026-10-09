#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bytes::Bytes;
use prost::Message as _;
use trogon_atlas_core::{
    schema::{entity_fields, pack_fields},
    WriterRole,
};
use trogon_atlas_proto::{
    entity, BranchDelta, Entity, EntityKind, Event, FieldSpec, Id, ReadModel,
};

use super::{
    ConflictResolution, LegacyEntity, LegacyFieldsLocation, LegacyFieldsReport, LegacyFieldsShape,
    MigrationMode,
};
use crate::{
    error::StoreError,
    key::{branch_delta_key, entity_key},
    nats::{NatsStore, NatsStoreConfig, StoreOpenMode, SCHEMA_VERSION_KEY, STORE_SCHEMA_MARKER},
    StoreResult,
};

async fn isolated_config() -> NatsStoreConfig {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("lf-{suffix}");
    config.branches_bucket = format!("lf-branches-{suffix}");
    config.changesets_bucket = format!("lf-changesets-{suffix}");
    config.revisions_bucket = format!("lf-revisions-{suffix}");
    config.batches_bucket = format!("lf-batches-{suffix}");
    config.namespaces_bucket = format!("lf-namespaces-{suffix}");
    config.lease_bucket = format!("lf-writer-lease-{suffix}");
    config.stream = format!("LF_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("lf.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    config
}

async fn isolated_store() -> NatsStore {
    NatsStore::connect_with(isolated_config().await)
        .await
        .expect("connect isolated store")
}

async fn reopen(
    config: &NatsStoreConfig,
    role: WriterRole,
    open_mode: StoreOpenMode,
) -> StoreResult<NatsStore> {
    let mut config = config.clone();
    config.role = role;
    config.open_mode = open_mode;
    NatsStore::connect_with(config).await
}

async fn marker(store: &NatsStore) -> String {
    let entry = store.kv.entry(SCHEMA_VERSION_KEY).await.unwrap().unwrap();
    String::from_utf8(entry.value.to_vec()).unwrap()
}

async fn mark_legacy(store: &NatsStore) {
    store
        .kv
        .put(
            SCHEMA_VERSION_KEY,
            Bytes::from_static(trogon_atlas_proto::SCHEMA_VERSION.as_bytes()),
        )
        .await
        .unwrap();
}

fn unmigrated_rows(result: StoreResult<NatsStore>) -> usize {
    match result {
        Err(StoreError::LegacyFieldsUnmigrated { rows }) => rows,
        Err(other) => panic!("expected LegacyFieldsUnmigrated, got {other}"),
        Ok(_) => panic!("expected LegacyFieldsUnmigrated, the store opened"),
    }
}

fn id(slug: &str) -> Id {
    Id {
        namespace: "legacy".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn named(name: &str) -> FieldSpec {
    FieldSpec {
        name: name.into(),
        ..Default::default()
    }
}

fn event(slug: &str, schema: Option<Vec<FieldSpec>>) -> Event {
    Event {
        id: Some(id(slug)),
        title: slug.into(),
        schema: schema.map(pack_fields),
        ..Default::default()
    }
}

/// An Entity as a pre-migration server wrote it: the current Event bytes
/// with the retired `fields = 5` entries appended.
fn legacy_event_bytes(event: &Event, legacy: &[FieldSpec]) -> Vec<u8> {
    let mut inner = event.encode_to_vec();
    for field in legacy {
        prost::encoding::message::encode(5, field, &mut inner);
    }
    let mut outer = Vec::new();
    prost::encoding::bytes::encode(1, &inner, &mut outer);
    outer
}

fn legacy_fields_of(bytes: &[u8]) -> Vec<FieldSpec> {
    LegacyEntity::decode(bytes).unwrap().into_fields()
}

async fn put_raw(store: &NatsStore, slug: &str, bytes: Vec<u8>) -> (String, u64) {
    let key = entity_key(EntityKind::Event, &id(slug));
    let revision = store.kv.put(&key, Bytes::from(bytes)).await.unwrap();
    (key, revision)
}

async fn raw(store: &NatsStore, key: &str) -> (Bytes, u64) {
    let entry = store.kv.entry(key).await.unwrap().unwrap();
    (entry.value, entry.revision)
}

fn shape_at(report: &LegacyFieldsReport, key: &str) -> Option<LegacyFieldsShape> {
    report
        .findings
        .iter()
        .find(|f| matches!(&f.location, LegacyFieldsLocation::Baseline { key: k } if k == key))
        .map(|f| f.shape)
}

#[test]
fn read_model_fields_live_at_tag_four() {
    let mut inner = ReadModel {
        id: Some(id("orders")),
        ..Default::default()
    }
    .encode_to_vec();
    prost::encoding::message::encode(4, &named("total"), &mut inner);
    let mut bytes = Vec::new();
    prost::encoding::bytes::encode(3, &inner, &mut bytes);

    let (shape, migrated) = super::migrate_entity_bytes("rm", &bytes).unwrap();

    assert_eq!(shape, LegacyFieldsShape::FieldsOnly);
    assert_eq!(entity_fields(&migrated), vec![named("total")]);
}

#[test]
fn kinds_without_fields_are_not_field_bearing() {
    let bytes = Entity {
        kind: Some(entity::Kind::Persona(trogon_atlas_proto::Persona {
            id: Some(id("buyer")),
            ..Default::default()
        })),
        ..Default::default()
    }
    .encode_to_vec();

    let (shape, _) = super::migrate_entity_bytes("persona", &bytes).unwrap();

    assert_eq!(shape, LegacyFieldsShape::NotFieldBearing);
}

#[tokio::test(flavor = "multi_thread")]
async fn fields_only_moves_into_schema() {
    let store = isolated_store().await;
    let legacy = vec![named("order_id"), named("amount")];
    let (key, before) = put_raw(
        &store,
        "placed",
        legacy_event_bytes(&event("placed", None), &legacy),
    )
    .await;

    let dry = store
        .migrate_legacy_fields(MigrationMode::DryRun, ConflictResolution::Refuse)
        .await
        .unwrap();
    assert_eq!(shape_at(&dry, &key), Some(LegacyFieldsShape::FieldsOnly));
    assert_eq!(
        raw(&store, &key).await.1,
        before,
        "a dry run must not write"
    );

    let applied = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();
    assert_eq!(
        shape_at(&applied, &key),
        Some(LegacyFieldsShape::FieldsOnly)
    );

    let (bytes, _) = raw(&store, &key).await;
    assert_eq!(legacy_fields_of(&bytes), Vec::new());
    assert_eq!(entity_fields(&Entity::decode(bytes).unwrap()), legacy);
}

#[tokio::test(flavor = "multi_thread")]
async fn schema_only_is_left_alone() {
    let store = isolated_store().await;
    let bytes = Entity {
        kind: Some(entity::Kind::Event(event(
            "shipped",
            Some(vec![named("carrier")]),
        ))),
        ..Default::default()
    }
    .encode_to_vec();
    let (key, before) = put_raw(&store, "shipped", bytes).await;

    let report = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();

    assert_eq!(shape_at(&report, &key), None);
    assert_eq!(report.count(LegacyFieldsShape::SchemaOnly), 1);
    assert_eq!(raw(&store, &key).await.1, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn both_equal_drops_the_legacy_list() {
    let store = isolated_store().await;
    let fields = vec![named("sku")];
    let (key, _) = put_raw(
        &store,
        "added",
        legacy_event_bytes(&event("added", Some(fields.clone())), &fields),
    )
    .await;

    let report = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();

    assert_eq!(shape_at(&report, &key), Some(LegacyFieldsShape::BothEqual));
    let (bytes, _) = raw(&store, &key).await;
    assert_eq!(legacy_fields_of(&bytes), Vec::new());
    assert_eq!(entity_fields(&Entity::decode(bytes).unwrap()), fields);
}

#[tokio::test(flavor = "multi_thread")]
async fn both_different_is_reported_and_kept() {
    let store = isolated_store().await;
    let original = legacy_event_bytes(
        &event("refunded", Some(vec![named("reason")])),
        &[named("amount")],
    );
    let (key, before) = put_raw(&store, "refunded", original.clone()).await;

    let report = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();

    assert_eq!(
        shape_at(&report, &key),
        Some(LegacyFieldsShape::BothDifferent)
    );
    assert!(report.has_conflicts());
    let (bytes, revision) = raw(&store, &key).await;
    assert_eq!(revision, before);
    assert_eq!(bytes.as_ref(), original.as_slice());
}

#[tokio::test(flavor = "multi_thread")]
async fn rerun_is_a_no_op() {
    let store = isolated_store().await;
    let fields = vec![named("sku")];
    let (only, _) = put_raw(
        &store,
        "only",
        legacy_event_bytes(&event("only", None), &fields),
    )
    .await;
    let (both, _) = put_raw(
        &store,
        "both",
        legacy_event_bytes(&event("both", Some(fields.clone())), &fields),
    )
    .await;
    store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();
    let after_first = (raw(&store, &only).await.1, raw(&store, &both).await.1);

    let second = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();

    assert!(second.findings.is_empty(), "{:?}", second.findings);
    assert_eq!(second.count(LegacyFieldsShape::SchemaOnly), 2);
    assert_eq!(
        (raw(&store, &only).await.1, raw(&store, &both).await.1),
        after_first
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_deltas_migrate_and_follow_the_baseline_revision() {
    let store = isolated_store().await;
    let fields = vec![named("order_id")];
    let legacy = legacy_event_bytes(&event("placed", None), &fields);
    let (key, base_revision) = put_raw(&store, "placed", legacy.clone()).await;

    let mut delta = Vec::new();
    prost::encoding::bytes::encode(1, &legacy, &mut delta);
    prost::encoding::string::encode(2, &base_revision.to_string(), &mut delta);
    prost::encoding::bytes::encode(3, &legacy, &mut delta);
    let delta_key = branch_delta_key("feature", EntityKind::Event, &id("placed"));
    store
        .branches_kv
        .put(&delta_key, Bytes::from(delta))
        .await
        .unwrap();

    let report = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();

    assert_eq!(report.count(LegacyFieldsShape::FieldsOnly), 3);
    let new_revision = raw(&store, &key).await.1;
    let stored = store.branches_kv.entry(&delta_key).await.unwrap().unwrap();
    let migrated = BranchDelta::decode(stored.value).unwrap();
    assert_eq!(migrated.base_etag, new_revision.to_string());
    assert_eq!(entity_fields(migrated.base.as_ref().unwrap()), fields);
    assert_eq!(entity_fields(migrated.ours.as_ref().unwrap()), fields);
}

#[tokio::test(flavor = "multi_thread")]
async fn fresh_store_is_stamped_without_migration() {
    let store = isolated_store().await;

    assert_eq!(marker(&store).await, STORE_SCHEMA_MARKER);
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_store_with_retired_rows_is_refused_for_every_role() {
    let config = isolated_config().await;
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    put_raw(
        &store,
        "placed",
        legacy_event_bytes(&event("placed", None), &[named("order_id")]),
    )
    .await;
    mark_legacy(&store).await;

    let writer = reopen(&config, WriterRole::Writer, StoreOpenMode::Serve).await;
    let reader = reopen(&config, WriterRole::Reader, StoreOpenMode::Serve).await;

    let Err(error) = writer else {
        panic!("a writer must not open an unmigrated store");
    };
    assert!(
        error.to_string().contains("migrate-legacy-fields --apply"),
        "{error}"
    );
    assert_eq!(unmigrated_rows(Err(error)), 1);
    assert_eq!(unmigrated_rows(reader), 1);
    assert_eq!(marker(&store).await, trogon_atlas_proto::SCHEMA_VERSION);
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_marks_the_store_and_lets_servers_open_it() {
    let config = isolated_config().await;
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    put_raw(
        &store,
        "placed",
        legacy_event_bytes(&event("placed", None), &[named("order_id")]),
    )
    .await;
    mark_legacy(&store).await;

    let migrator = reopen(
        &config,
        WriterRole::Reader,
        StoreOpenMode::MigrateLegacyFields,
    )
    .await
    .expect("the migrate command must open a legacy store");
    let dry = migrator
        .migrate_legacy_fields(MigrationMode::DryRun, ConflictResolution::Refuse)
        .await
        .unwrap();
    assert_eq!(dry.remaining(), 1);
    assert_eq!(marker(&store).await, trogon_atlas_proto::SCHEMA_VERSION);

    let applied = migrator
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();
    assert_eq!(applied.remaining(), 0);
    assert_eq!(marker(&store).await, STORE_SCHEMA_MARKER);
    reopen(&config, WriterRole::Reader, StoreOpenMode::Serve)
        .await
        .expect("a migrated store must open");

    let rerun = migrator
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();
    assert!(rerun.findings.is_empty(), "{:?}", rerun.findings);
    assert_eq!(marker(&store).await, STORE_SCHEMA_MARKER);
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_store_without_retired_rows_is_stamped_on_open() {
    let config = isolated_config().await;
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    put_raw(
        &store,
        "shipped",
        Entity {
            kind: Some(entity::Kind::Event(event(
                "shipped",
                Some(vec![named("carrier")]),
            ))),
            ..Default::default()
        }
        .encode_to_vec(),
    )
    .await;
    mark_legacy(&store).await;

    reopen(&config, WriterRole::Reader, StoreOpenMode::Serve)
        .await
        .expect("a store with no retired rows must open");

    assert_eq!(marker(&store).await, STORE_SCHEMA_MARKER);
}

#[tokio::test(flavor = "multi_thread")]
async fn conflicts_keep_the_store_refused_until_schema_is_chosen() {
    let config = isolated_config().await;
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    let (key, _) = put_raw(
        &store,
        "refunded",
        legacy_event_bytes(
            &event("refunded", Some(vec![named("reason")])),
            &[named("amount")],
        ),
    )
    .await;
    mark_legacy(&store).await;

    let refused = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::Refuse)
        .await
        .unwrap();
    assert_eq!(refused.unresolved_conflicts(), 1);
    assert_eq!(marker(&store).await, trogon_atlas_proto::SCHEMA_VERSION);
    assert_eq!(
        unmigrated_rows(reopen(&config, WriterRole::Reader, StoreOpenMode::Serve).await),
        1
    );

    let kept = store
        .migrate_legacy_fields(MigrationMode::Apply, ConflictResolution::KeepSchema)
        .await
        .unwrap();
    assert_eq!(kept.remaining(), 0);
    assert_eq!(marker(&store).await, STORE_SCHEMA_MARKER);
    let (bytes, _) = raw(&store, &key).await;
    assert_eq!(legacy_fields_of(&bytes), Vec::new());
    assert_eq!(
        entity_fields(&Entity::decode(bytes).unwrap()),
        vec![named("reason")]
    );
}
