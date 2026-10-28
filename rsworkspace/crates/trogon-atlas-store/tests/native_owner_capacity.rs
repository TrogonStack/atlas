#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    key::{branch_delta_key, entity_key},
    store::BranchLandOp,
    NatsStore, NatsStoreConfig, Store, StoreMetrics, WriteContext,
};

const LIMIT: usize = 1_048_576;
const KIND: pb::EntityKind = pb::EntityKind::Document;

fn id(slug: &str) -> pb::Id {
    pb::Id { namespace: "capacity".into(), slug: slug.into(), version: 1 }
}

fn owner() -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Document(pb::Document {
            id: Some(id("owner")), title: Some(pb::CatalogTitle { value: "Owner".into() }),
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink { value: "https://example.com/doc".into() })),
            subjects: (0..64).map(|index| pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject { selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32, id: Some(id(&format!("subject-{index}"))),
                })) }),
                doc: Some(pb::CatalogText { value: "x".repeat(15_500) }),
                ..Default::default()
            }).collect(),
            ..Default::default()
        })),
    }
}

fn resize(entity: &mut pb::Entity, target: usize) {
    loop {
        let size = entity.encoded_len();
        if size == target { return; }
        let Some(pb::entity::Kind::Document(document)) = entity.kind.as_mut() else { panic!("document") };
        let doc = document.doc.get_or_insert_default();
        if size < target { doc.push_str(&"x".repeat(target - size)); }
        else { doc.truncate(doc.len() - (size - target)); }
        assert!(doc.len() <= 65_536);
    }
}

#[tokio::test]
async fn branch_publication_checks_complete_prepared_owner() {
    let nats = trogon_atlas_testsupport::TestNats::start().await;
    let config = NatsStoreConfig::new(&nats.url, StoreMetrics::default());
    let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    for index in 0..64 {
        store.create(pb::EntityKind::ReadModel, &pb::Entity {
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(&format!("subject-{index}"))), title: "Subject".into(), ..Default::default()
            })), ..Default::default()
        }, WriteContext::baseline()).await.unwrap();
    }
    let mut exact = store.create(KIND, &owner(), WriteContext::baseline()).await.unwrap().entity;
    resize(&mut exact, LIMIT);
    trogon_atlas_core::decision_document::validate_values(&exact).unwrap();
    store.put(KIND, &exact, true, WriteContext::baseline()).await.unwrap();
    assert_eq!(store.get(KIND, &id("owner"), None).await.unwrap().entity.encoded_len(), LIMIT);
    let mut overflow = exact.clone();
    resize(&mut overflow, LIMIT + 1);
    let entities = js.get_key_value(&config.bucket).await.unwrap();
    let branches = js.get_key_value(&config.branches_bucket).await.unwrap();
    let key = entity_key(KIND, &id("owner"));
    for route in ["active", "merge", "land", "rebase", "ours", "theirs"] {
        let branch = format!("capacity-{route}");
        store.create_branch(&branch, "capacity proof").await.unwrap();
        store.put(KIND, &exact, true, WriteContext::branch(&branch)).await.unwrap();
        assert_eq!(store.get(KIND, &id("owner"), Some(&branch)).await.unwrap().entity.encoded_len(), LIMIT);
        store.resolve_branch_entry(&branch, KIND, &id("owner"), false).await.unwrap();
        store.resolve_branch_entry(&branch, KIND, &id("owner"), true).await.unwrap();
        store.put(KIND, &exact, true, WriteContext::branch(&branch)).await.unwrap();
        let delta_key = branch_delta_key(&branch, KIND, &id("owner"));
        // Model snapshots persisted before the final-owner guard existed.
        if matches!(route, "rebase" | "ours") {
            branches.put(&delta_key, pb::BranchDelta {
                base: Some(exact.clone()), base_etag: store.get(KIND, &id("owner"), None).await.unwrap().etag,
                ours: Some(overflow.clone()), tombstone: false,
            }.encode_to_vec().into()).await.unwrap();
        }
        if route == "theirs" {
            entities.put(&key, overflow.encode_to_vec().into()).await.unwrap();
        }
        let before = entities.entry(&key).await.unwrap().unwrap();
        let delta_before = branches.entry(&delta_key).await.unwrap().unwrap();
        let seq = store.current_change_seq().await.unwrap();
        let result = match route {
            "active" => store.put(KIND, &overflow, true, WriteContext::branch(&branch)).await.map(|_| ()),
            "merge" => store.put(KIND, &overflow, true, WriteContext::branch(&branch).with_branch_merge_base(&exact)).await.map(|_| ()),
            "land" => store.land_branch_merge(&branch, &[BranchLandOp::Put {
                kind: KIND, ours: Box::new(overflow.clone()), base: Some(Box::new(exact.clone())),
                expected_baseline_etag: Some(before.revision.to_string()),
            }], WriteContext::baseline()).await.map(|_| ()),
            "rebase" => store.rebase_branch_deltas(&branch, &[(KIND, id("owner"), Some((exact.clone(), before.revision.to_string())))]).await.map(|_| ()),
            "ours" => store.resolve_branch_entry(&branch, KIND, &id("owner"), false).await,
            "theirs" => store.resolve_branch_entry(&branch, KIND, &id("owner"), true).await,
            _ => unreachable!(),
        };
        assert!(result.is_err(), "{route} published an oversized prepared owner");
        let after = entities.entry(&key).await.unwrap().unwrap();
        let delta_after = branches.entry(&delta_key).await.unwrap().unwrap();
        assert_eq!((after.value, after.revision), (before.value, before.revision), "{route}");
        assert_eq!((delta_after.value, delta_after.revision), (delta_before.value, delta_before.revision), "{route}");
        assert_eq!(store.current_change_seq().await.unwrap(), seq, "{route}");
        if route == "theirs" { entities.put(&key, exact.encode_to_vec().into()).await.unwrap(); }
    }
}
