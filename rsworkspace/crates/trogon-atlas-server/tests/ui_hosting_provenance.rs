#![allow(clippy::unwrap_used)]

use trogon_atlas_core::connection_persistence::prepare_entity;
use trogon_atlas_proto as pb;
use trogon_atlas_server::graph::{impact, outgoing_of, ReverseIndex};
use trogon_atlas_store::StoredEntity;

#[test]
fn hosting_provenance_retains_distinct_occurrences_across_public_graph_surfaces() {
    let id = |slug: &str| pb::Id {
        namespace: "hosting-provenance".into(),
        slug: slug.into(),
        version: 1,
    };
    let target = id("review");
    let hosting = pb::UiHosting {
        ui: Some(pb::UiRef {
            id: Some(target.clone()),
        }),
        label: Some(pb::CatalogTitle {
            value: "Review".into(),
        }),
        ..Default::default()
    };
    let owner = prepare_entity(
        None,
        &pb::Entity {
            kind: Some(pb::entity::Kind::Component(pb::Component {
                id: Some(id("portal")),
                title: "Portal".into(),
                ui_hostings: vec![hosting.clone(), hosting],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .unwrap();
    let Some(pb::entity::Kind::Component(component)) = &owner.kind else {
        unreachable!()
    };
    let refs = outgoing_of(&owner).unwrap();
    assert_eq!(refs.len(), 2);
    for (index, reference) in refs.iter().enumerate() {
        assert_eq!(reference.from_uid, owner.system.as_ref().unwrap().uid);
        assert_eq!(
            reference.connection_id, component.ui_hostings[index].connection_id,
            "equal hosting targets require distinct provenance"
        );
        assert_eq!(reference.from_field, format!("ui_hostings[{index}].ui"));
    }
    assert_ne!(refs[0].connection_id, refs[1].connection_id);
    let ui = prepare_entity(
        None,
        &pb::Entity {
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(target.clone()),
                title: "Review".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .unwrap();
    let rows = [
        StoredEntity {
            entity: owner.clone(),
            etag: "owner-etag".into(),
        },
        StoredEntity {
            entity: ui,
            etag: "ui-etag".into(),
        },
    ];
    let index = ReverseIndex::build(&rows).unwrap();
    let incoming = index.incoming(pb::EntityKind::Ui, &target);
    assert_eq!(incoming, refs.as_slice());
    let nodes = impact(&rows, pb::EntityKind::Component, &id("portal"), 1, false).unwrap();
    let selected = nodes
        .iter()
        .find(|node| {
            node.entity
                .as_ref()
                .is_some_and(|entity| entity.kind == pb::EntityKind::Ui as i32)
        })
        .unwrap();
    assert_eq!(selected.depth, 1);
    assert_eq!(selected.path.len(), 1);
    assert!(refs.contains(&selected.path[0]));
    assert!(selected.path[0].connection_id.is_some());
}
