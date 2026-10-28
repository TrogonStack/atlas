#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_core::connection_persistence::prepare_entity;
use trogon_atlas_proto as pb;

fn custom() -> pb::TopologyCustomStep {
    pb::TopologyCustomStep {
        title: Some(pb::CatalogTitle {
            value: "Custom step".into(),
        }),
        properties: (0..16)
            .map(|index| (format!("key-{index}"), format!("value-{index}")))
            .collect(),
        ..Default::default()
    }
}

#[test]
fn equal_custom_properties_cannot_hide_stripped_step_identity() {
    let original = custom();
    let reordered = (0..128)
        .map(|_| custom())
        .find(|candidate| candidate.encode_to_vec() != original.encode_to_vec())
        .expect("equivalent maps with distinct wire ordering");
    assert_eq!(original, reordered);
    let entity = pb::Entity {
        kind: Some(pb::entity::Kind::DesignGraph(pb::DesignGraph {
            id: Some(pb::Id {
                namespace: "catalog".into(),
                slug: "custom-flow".into(),
                version: 1,
            }),
            title: Some(pb::CatalogTitle {
                value: "Original graph".into(),
            }),
            profile: pb::DesignGraphProfile::Flow as i32,
            steps: vec![pb::TopologyStep {
                title: Some(pb::CatalogTitle {
                    value: "Step".into(),
                }),
                target: Some(pb::TopologyStepTarget {
                    selection: Some(pb::topology_step_target::Selection::Custom(original)),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    let previous = prepare_entity(None, &entity).unwrap();
    let mut proposed = previous.clone();
    let Some(pb::entity::Kind::DesignGraph(graph)) = proposed.kind.as_mut() else {
        panic!("expected design graph");
    };
    graph.title.as_mut().unwrap().value = "Renamed graph".into();
    graph.steps[0].step_id = None;
    graph.steps[0].target.as_mut().unwrap().selection =
        Some(pb::topology_step_target::Selection::Custom(reordered));

    let error = prepare_entity(Some(&previous), &proposed).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("removed step reintroduced without its ID")
    );

    let Some(pb::entity::Kind::DesignGraph(graph)) = proposed.kind.as_mut() else {
        panic!("expected design graph");
    };
    let Some(pb::topology_step_target::Selection::Custom(custom)) =
        graph.steps[0].target.as_mut().unwrap().selection.as_mut()
    else {
        panic!("expected custom target");
    };
    custom.properties.insert("key-0".into(), "changed".into());
    let replacement = prepare_entity(Some(&previous), &proposed).unwrap();
    let Some(pb::entity::Kind::DesignGraph(replacement)) = replacement.kind else {
        panic!("expected design graph");
    };
    let Some(pb::entity::Kind::DesignGraph(previous)) = previous.kind else {
        panic!("expected design graph");
    };
    assert_ne!(replacement.steps[0].step_id, previous.steps[0].step_id);
}
