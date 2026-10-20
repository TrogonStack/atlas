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
    let targets = [
        pb::EntityKey::new(pb::EntityKind::CommandSlice, &slice_id),
        pb::EntityKey::new(pb::EntityKind::AutomationSlice, &slice_id),
    ];
    assert_eq!(
        store
            .referenced_targets(&targets, Some("untyped"))
            .await
            .unwrap(),
        std::collections::HashSet::from([targets[0].clone()])
    );
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
    assert_eq!(
        store
            .referenced_targets(&targets, Some("untyped"))
            .await
            .unwrap(),
        std::collections::HashSet::from([targets[1].clone()])
    );
    assert!(store
        .has_incoming_reference(pb::EntityKind::AutomationSlice, &slice_id, Some("untyped"))
        .await
        .unwrap());
    assert!(!store
        .has_incoming_reference(pb::EntityKind::AutomationSlice, &slice_id, None)
        .await
        .unwrap());
}

#[tokio::test]
async fn multi_target_reference_guard_uses_one_merged_world() {
    use std::collections::HashSet;

    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let key = |slug| pb::EntityKey::new(pb::EntityKind::ExternalSystem, &id(slug));
    for (target, owner) in [("old", "replacement"), ("removed", "tombstone")] {
        store
            .create(
                pb::EntityKind::Processor,
                &processor(target, owner),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    store.create_branch("multi", "").await.unwrap();
    store
        .put(
            pb::EntityKind::Processor,
            &processor("new", "replacement"),
            false,
            WriteContext::branch("multi"),
        )
        .await
        .unwrap();
    store
        .delete(
            pb::EntityKind::Processor,
            &id("tombstone"),
            None,
            WriteContext::branch("multi"),
        )
        .await
        .unwrap();
    store
        .create(
            pb::EntityKind::Processor,
            &processor("added", "branch-only"),
            WriteContext::branch("multi"),
        )
        .await
        .unwrap();
    let targets = [
        key("old"),
        key("removed"),
        key("new"),
        key("added"),
        key("absent"),
        key("new"),
    ];
    assert_eq!(
        store
            .referenced_targets(&targets, Some("multi"))
            .await
            .unwrap(),
        HashSet::from([key("new"), key("added")])
    );
    assert_eq!(
        store.referenced_targets(&targets, None).await.unwrap(),
        HashSet::from([key("old"), key("removed")])
    );
    assert!(store
        .referenced_targets(&[], None)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        store.referenced_targets(&targets, Some("missing")).await,
        Err(StoreError::NotFound)
    ));
}
