#![allow(clippy::unwrap_used)]

use trogon_atlas_proto as pb;
use trogon_atlas_store::{Store, StoreError, WriteContext};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "directory".into(),
        slug: slug.into(),
        version: 1,
    }
}

#[tokio::test]
async fn team_person_crud_routes_exact_kinds_and_preserves_branch_reference_guards() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let person = pb::Entity {
        kind: Some(pb::entity::Kind::Person(pb::Person {
            id: Some(id("person")),
            title: Some(pb::DirectoryDisplayName {
                value: "Person".into(),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    store
        .create(pb::EntityKind::Person, &person, WriteContext::baseline())
        .await
        .unwrap();
    let subjects = [
        pb::responsibility_subject::Selection::Entity(pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("event")),
        }),
        pb::responsibility_subject::Selection::Connection(pb::ConnectionRef {
            owner: Some(pb::EntityRef {
                kind: pb::EntityKind::AutomationSlice as i32,
                id: Some(id("slice")),
            }),
            owner_uid: "slice-uid".into(),
            id: Some(pb::ConnectionId {
                value: "edge-id".into(),
            }),
        }),
        pb::responsibility_subject::Selection::Member(pb::ContractMemberRef {
            specification: Some(pb::SpecificationRef {
                id: Some(id("spec")),
            }),
            owner_uid: "spec-uid".into(),
            kind: pb::ContractMemberKind::Schema as i32,
            member_id: Some(pb::ContractMemberId {
                value: "member-id".into(),
            }),
            ..Default::default()
        }),
        pb::responsibility_subject::Selection::Port(pb::DataPortRef {
            product: Some(pb::DataProductRef {
                id: Some(id("product")),
            }),
            owner_uid: "product-uid".into(),
            direction: pb::DataPortDirection::Output as i32,
            port_id: Some(pb::DataPortId {
                value: "port-id".into(),
            }),
        }),
    ];
    let mut team = pb::Entity {
        kind: Some(pb::entity::Kind::Team(pb::Team {
            id: Some(id("team")),
            title: Some(pb::DirectoryDisplayName {
                value: "Team".into(),
            }),
            members: vec![pb::TeamMember {
                person: Some(pb::PersonRef {
                    id: Some(id("person")),
                }),
                ..Default::default()
            }],
            responsibilities: subjects
                .into_iter()
                .map(|selection| pb::Responsibility {
                    kind: pb::ResponsibilityKind::Owner as i32,
                    subject: Some(pb::ResponsibilitySubject {
                        selection: Some(selection),
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    };
    store
        .create(pb::EntityKind::Team, &team, WriteContext::baseline())
        .await
        .unwrap();
    let stored = store
        .get(pb::EntityKind::Team, &id("team"), None)
        .await
        .unwrap();
    assert!(matches!(
        stored.entity.kind,
        Some(pb::entity::Kind::Team(_))
    ));
    assert!(matches!(
        store.get(pb::EntityKind::Person, &id("team"), None).await,
        Err(StoreError::NotFound)
    ));
    let targets = [
        (pb::EntityKind::Person, "person"),
        (pb::EntityKind::Event, "event"),
        (pb::EntityKind::AutomationSlice, "slice"),
        (pb::EntityKind::Specification, "spec"),
        (pb::EntityKind::DataProduct, "product"),
    ];
    for (kind, slug) in targets {
        assert!(
            store
                .has_incoming_reference(kind, &id(slug), None)
                .await
                .unwrap(),
            "{kind:?}"
        );
    }
    store.create_branch("directory-edit", "").await.unwrap();
    let Some(pb::entity::Kind::Team(value)) = team.kind.as_mut() else {
        panic!("team missing")
    };
    value.members.clear();
    value.responsibilities.clear();
    store
        .put(
            pb::EntityKind::Team,
            &team,
            false,
            WriteContext::branch("directory-edit"),
        )
        .await
        .unwrap();
    for (kind, slug) in targets {
        assert!(!store
            .has_incoming_reference(kind, &id(slug), Some("directory-edit"))
            .await
            .unwrap());
        assert!(store
            .has_incoming_reference(kind, &id(slug), None)
            .await
            .unwrap());
    }
    store
        .delete(
            pb::EntityKind::Team,
            &id("team"),
            None,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    assert!(store
        .get(pb::EntityKind::Person, &id("person"), None)
        .await
        .is_ok());
    assert!(!store
        .has_incoming_reference(pb::EntityKind::Person, &id("person"), None)
        .await
        .unwrap());
    store
        .delete(
            pb::EntityKind::Person,
            &id("person"),
            None,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    assert!(matches!(
        store.get(pb::EntityKind::Person, &id("person"), None).await,
        Err(StoreError::NotFound)
    ));
}
