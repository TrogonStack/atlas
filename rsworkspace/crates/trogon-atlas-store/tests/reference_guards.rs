#![allow(clippy::unwrap_used)]

use trogon_atlas_proto as pb;
use trogon_atlas_store::{Store, StoreError, WriteContext};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "commerce".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn processor(target: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id(slug)),
            title: slug.into(),
            calls: vec![pb::ExternalSystemRef {
                id: Some(id(target)),
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[tokio::test]
async fn reference_guard_uses_merged_replacements_tombstones_and_branch_only_owners() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    store
        .create(
            pb::EntityKind::Processor,
            &processor("old", "consumer"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store.create_branch("references", "").await.unwrap();
    assert!(store
        .has_incoming_reference(
            pb::EntityKind::ExternalSystem,
            &id("old"),
            Some("references")
        )
        .await
        .unwrap());
    store
        .put(
            pb::EntityKind::Processor,
            &processor("new", "consumer"),
            false,
            WriteContext::branch("references"),
        )
        .await
        .unwrap();
    assert!(!store
        .has_incoming_reference(
            pb::EntityKind::ExternalSystem,
            &id("old"),
            Some("references")
        )
        .await
        .unwrap());
    assert!(store
        .has_incoming_reference(
            pb::EntityKind::ExternalSystem,
            &id("new"),
            Some("references")
        )
        .await
        .unwrap());
    assert!(store
        .has_incoming_reference(pb::EntityKind::ExternalSystem, &id("old"), None)
        .await
        .unwrap());
    store
        .delete(
            pb::EntityKind::Processor,
            &id("consumer"),
            None,
            WriteContext::branch("references"),
        )
        .await
        .unwrap();
    assert!(!store
        .has_incoming_reference(
            pb::EntityKind::ExternalSystem,
            &id("old"),
            Some("references")
        )
        .await
        .unwrap());
    assert!(!store
        .has_incoming_reference(
            pb::EntityKind::ExternalSystem,
            &id("new"),
            Some("references")
        )
        .await
        .unwrap());
    store
        .create(
            pb::EntityKind::Processor,
            &processor("branch-only", "added"),
            WriteContext::branch("references"),
        )
        .await
        .unwrap();
    assert!(store
        .has_incoming_reference(
            pb::EntityKind::ExternalSystem,
            &id("branch-only"),
            Some("references")
        )
        .await
        .unwrap());
    assert!(!store
        .has_incoming_reference(pb::EntityKind::ExternalSystem, &id("branch-only"), None)
        .await
        .unwrap());
    assert!(matches!(
        store
            .has_incoming_reference(pb::EntityKind::ExternalSystem, &id("old"), Some("missing"))
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn untyped_reference_guard_preserves_first_existing_slice_kind_precedence() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let slice_id = id("shared-key");
    for (kind, entity) in [
        (
            pb::EntityKind::CommandSlice,
            pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(slice_id.clone()),
                title: "Command moment".into(),
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::AutomationSlice,
            pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(slice_id.clone()),
                title: "Automation moment".into(),
                ..Default::default()
            }),
        ),
    ] {
        store
            .create(
                kind,
                &pb::Entity {
                    kind: Some(entity),
                    ..Default::default()
                },
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    store
        .create(
            pb::EntityKind::Storyboard,
            &pb::Entity {
                kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                    id: Some(id("journey")),
                    title: "Journey".into(),
                    slices: vec![pb::SliceRef {
                        id: Some(slice_id.clone()),
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            },
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store.create_branch("untyped", "").await.unwrap();
    assert!(store
        .has_incoming_reference(pb::EntityKind::CommandSlice, &slice_id, Some("untyped"))
        .await
        .unwrap());
    assert!(!store
        .has_incoming_reference(pb::EntityKind::AutomationSlice, &slice_id, Some("untyped"))
        .await
        .unwrap());
    store
        .delete(
            pb::EntityKind::CommandSlice,
            &slice_id,
            None,
            WriteContext::branch("untyped"),
        )
        .await
        .unwrap();
    assert!(store
        .has_incoming_reference(pb::EntityKind::AutomationSlice, &slice_id, Some("untyped"))
        .await
        .unwrap());
    assert!(!store
        .has_incoming_reference(pb::EntityKind::AutomationSlice, &slice_id, None)
        .await
        .unwrap());
}
