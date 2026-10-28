#![allow(clippy::unwrap_used)]
use prost::Message;
use trogon_atlas_core::agent_metadata::validate;
use trogon_atlas_proto::{entity, Component, Entity, Event};

fn annotation(name: &str, value: Vec<u8>) -> prost_types::Any {
    prost_types::Any { type_url: format!("type.googleapis.com/{name}"), value }
}
#[test]
fn agent_presence_is_component_only_and_singleton() {
    let any = annotation("trogonatlas.eventmodel.v1alpha1.AgentAnnotation", vec![]);
    let component = |metadata| Entity { kind: Some(entity::Kind::Component(Component { metadata, ..Default::default() })), ..Default::default() };
    assert!(validate(&component(vec![any.clone()])).is_ok());
    assert!(validate(&component(vec![any.clone(), any.clone()])).is_err());
    let event = Entity { kind: Some(entity::Kind::Event(Event { metadata: vec![any], ..Default::default() })), ..Default::default() };
    assert!(validate(&event).is_err());
}
#[test]
fn lifecycle_calendar_date_and_known_wire_are_validated() {
    use trogon_atlas_proto::{CalendarDate, LifecycleAnnotation};
    let owner = |value| Entity { kind: Some(entity::Kind::Component(Component { metadata: vec![annotation("trogonatlas.annotation.v1alpha1.LifecycleAnnotation", value)], ..Default::default() })), ..Default::default() };
    for (date, valid) in [("2024-02-29",true),("2023-02-29",false),("0000-01-01",false)] {
        let lifecycle = LifecycleAnnotation { status: "deprecated".into(), deprecated_on: Some(CalendarDate { value: date.into() }), ..Default::default() };
        assert_eq!(validate(&owner(lifecycle.encode_to_vec())).is_ok(),valid);
    }
    assert!(validate(&owner(vec![255])).is_err());
}
