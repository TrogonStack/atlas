#![allow(clippy::unwrap_used)]
use prost::Message;
use trogon_atlas_core::agent_metadata::validate;
use trogon_atlas_proto::{entity, Component, Entity, Event};

fn annotation(name: &str, value: Vec<u8>) -> prost_types::Any {
    prost_types::Any {
        type_url: format!("type.googleapis.com/{name}"),
        value,
    }
}
#[test]
fn agent_presence_is_component_only_and_singleton() {
    let any = annotation("trogonatlas.eventmodel.v1alpha1.AgentAnnotation", vec![]);
    let component = |metadata| Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata,
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(validate(&component(vec![any.clone()])).is_ok());
    assert!(validate(&component(vec![any.clone(), any.clone()])).is_err());
    let event = Entity {
        kind: Some(entity::Kind::Event(Event {
            metadata: vec![any],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(validate(&event).is_err());
}
#[test]
fn lifecycle_calendar_date_and_known_wire_are_validated() {
    use trogon_atlas_proto::{CalendarDate, LifecycleAnnotation};
    let owner = |value| Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(
                "trogonatlas.annotation.v1alpha1.LifecycleAnnotation",
                value,
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    for (date, valid) in [
        ("2024-02-29", true),
        ("2023-02-29", false),
        ("0000-01-01", false),
    ] {
        let lifecycle = LifecycleAnnotation {
            status: "deprecated".into(),
            deprecated_on: Some(CalendarDate { value: date.into() }),
            ..Default::default()
        };
        assert_eq!(validate(&owner(lifecycle.encode_to_vec())).is_ok(), valid);
    }
    assert!(validate(&owner(vec![255])).is_err());
}

#[test]
fn values_and_resource_occurrences_are_native() {
    use trogon_atlas_core::connections::{assign_connection_ids, connection_occurrences};
    use trogon_atlas_proto::{
        AgentAnnotation, AgentModel, ResourceGroup, ResourceGroupItem, ResourceGroupsAnnotation,
    };
    let owner = |name, bytes| Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(name, bytes)],
            ..Default::default()
        })),
        ..Default::default()
    };
    let empty_model = AgentAnnotation {
        model: Some(AgentModel::default()),
        ..Default::default()
    };
    assert!(validate(&owner(
        "trogonatlas.eventmodel.v1alpha1.AgentAnnotation",
        empty_model.encode_to_vec()
    ))
    .is_err());
    let groups = ResourceGroupsAnnotation {
        groups: vec![ResourceGroup {
            items: vec![ResourceGroupItem::default()],
            ..Default::default()
        }],
    };
    let mut entity = owner(
        "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
        groups.encode_to_vec(),
    );
    assert!(validate(&entity).is_err());
    assert_eq!(assign_connection_ids(&mut entity).unwrap(), 2);
    assert_eq!(connection_occurrences(&entity).unwrap().len(), 2);
}

#[test]
fn group_refs_retarget_and_malformed_payloads_are_accounted_for() {
    use trogon_atlas_core::{outbound_refs, retarget_refs};
    use trogon_atlas_proto::*;
    let target = Id {
        namespace: "example".into(),
        slug: "target".into(),
        version: 1,
    };
    let groups = ResourceGroupsAnnotation {
        groups: vec![ResourceGroup {
            items: vec![
                ResourceGroupItem {
                    target: Some(EntityRef {
                        kind: EntityKind::Component as i32,
                        id: Some(target.clone())
                    }),
                    ..Default::default()
                };
                2
            ],
            ..Default::default()
        }],
    };
    let mut owner = Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(
                "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
                groups.encode_to_vec(),
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    trogon_atlas_core::connections::assign_connection_ids(&mut owner).unwrap();
    assert_eq!(outbound_refs(&owner).len(), 2);
    let new = Id {
        slug: "new".into(),
        ..target.clone()
    };
    assert_eq!(
        retarget_refs(&mut owner, EntityKind::Component, &target, &new).len(),
        2
    );
    assert!(outbound_refs(&owner).iter().all(|r| r.to_id == new));
}
