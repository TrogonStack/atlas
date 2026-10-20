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

#[tokio::test]
async fn specification_nested_references_guard_the_complete_target_set_and_branch_tombstones() {
    use std::collections::HashSet;

    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let targets = [
        (pb::EntityKind::ReadModel, "receipt"),
        (pb::EntityKind::Schema, "native-shape"),
        (pb::EntityKind::TypeLibrary, "types"),
        (pb::EntityKind::Specification, "shared-contract"),
        (pb::EntityKind::Channel, "transport"),
        (pb::EntityKind::Component, "provider"),
        (pb::EntityKind::ExternalSystem, "caller"),
    ]
    .map(|(kind, slug)| pb::EntityKey::new(kind, &id(slug)));
    let entity = pb::Entity {
        kind: Some(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id("order-contract")),
            title: "Order contract".into(),
            schemas: vec![pb::ContractSchema {
                definition: Some(pb::SchemaDefinition {
                    target: Some(pb::schema_definition::Target::Schema(pb::SchemaRef {
                        id: Some(id("native-shape")),
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            messages: vec![pb::ContractMessage {
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id("receipt")),
                    }),
                    ..Default::default()
                }),
                schemas: vec![
                    pb::SchemaBinding {
                        r#type: Some(pb::schema_binding::Type::LibraryType(
                            pb::TypeLibraryTypeRef {
                                library: Some(pb::TypeLibraryRef {
                                    id: Some(id("types")),
                                }),
                                message: Some(pb::QualifiedProtoName {
                                    value: "orders.Receipt".into(),
                                }),
                            },
                        )),
                        ..Default::default()
                    },
                    pb::SchemaBinding {
                        r#type: Some(pb::schema_binding::Type::ContractType(
                            pb::ContractTypeRef {
                                schema: Some(pb::ContractMemberRef {
                                    specification: Some(pb::SpecificationRef {
                                        id: Some(id("shared-contract")),
                                    }),
                                    owner_uid: "selected-incarnation".into(),
                                    kind: pb::ContractMemberKind::Schema as i32,
                                    member_id: Some(pb::ContractMemberId {
                                        value: "11111111-1111-4111-8111-111111111111".into(),
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        )),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            operations: vec![pb::ContractOperation {
                binding: Some(pb::OperationBinding {
                    kind: Some(pb::operation_binding::Kind::Channel(
                        pb::ChannelOperationBinding {
                            channel: Some(pb::ChannelRef {
                                id: Some(id("transport")),
                            }),
                            ..Default::default()
                        },
                    )),
                }),
                ..Default::default()
            }],
            exposures: vec![pb::ProviderExposure {
                provider: Some(pb::ProviderTarget {
                    target: Some(pb::provider_target::Target::Component(pb::ComponentRef {
                        id: Some(id("provider")),
                    })),
                }),
                ..Default::default()
            }],
            usages: vec![pb::ApiUsage {
                caller: Some(pb::ProviderTarget {
                    target: Some(pb::provider_target::Target::ExternalSystem(
                        pb::ExternalSystemRef {
                            id: Some(id("caller")),
                        },
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    store
        .create(
            pb::EntityKind::Specification,
            &entity,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    assert_eq!(
        store.referenced_targets(&targets, None).await.unwrap(),
        HashSet::from(targets.clone())
    );
    store.create_branch("contract-removal", "").await.unwrap();
    assert_eq!(
        store
            .referenced_targets(&targets, Some("contract-removal"))
            .await
            .unwrap(),
        HashSet::from(targets.clone())
    );
    store
        .delete(
            pb::EntityKind::Specification,
            &id("order-contract"),
            None,
            WriteContext::branch("contract-removal"),
        )
        .await
        .unwrap();
    assert!(store
        .referenced_targets(&targets, Some("contract-removal"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store.referenced_targets(&targets, None).await.unwrap(),
        HashSet::from(targets)
    );
}
