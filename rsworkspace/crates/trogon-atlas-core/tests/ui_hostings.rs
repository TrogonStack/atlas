use prost::Message;
use trogon_atlas_core::{connection_persistence::prepare_entity, connections::connection_occurrences};
use trogon_atlas_proto::{entity, Component, Entity, Id, UiRef, CatalogTitle};

#[derive(Clone, PartialEq, Message)]
struct HostingWire {
    #[prost(message, optional, tag="2")]
    ui: Option<UiRef>,
    #[prost(message, optional, tag="3")]
    label: Option<CatalogTitle>,
}
#[derive(Clone, PartialEq, Message)]
struct ComponentWire {
    #[prost(message, optional, tag="1")]
    id: Option<Id>,
    #[prost(message, repeated, tag="10")]
    hostings: Vec<HostingWire>,
}
#[test]
fn native_preparation_preserves_equal_hostings_as_distinct_occurrences() -> Result<(), Box<dyn std::error::Error>> {
    let id = |slug: &str| Id { namespace: "shop".into(), slug: slug.into(), version: 1 };
    let hosting = HostingWire { ui: Some(UiRef { id: Some(id("review")) }), label: Some(CatalogTitle { value: "Review".into() }) };
    let component = Component::decode(ComponentWire { id: Some(id("portal")), hostings: vec![hosting.clone(), hosting] }.encode_to_vec().as_slice())?;
    let prepared = prepare_entity(None, &Entity { kind: Some(entity::Kind::Component(component)), ..Default::default() })?;
    let occurrences = connection_occurrences(&prepared)?;
    assert_eq!(occurrences.len(), 2, "native hosting occurrences must survive preparation");
    assert_ne!(occurrences[0].id, occurrences[1].id);
    assert_eq!(occurrences[0].target, occurrences[1].target);
    Ok(())
}
