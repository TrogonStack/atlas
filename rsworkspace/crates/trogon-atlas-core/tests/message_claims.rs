#![allow(clippy::unwrap_used)]
use trogon_atlas_core::{
    connections, outbound_refs,
    source_integration::{self, OwnershipUnit},
};
use trogon_atlas_proto::*;
fn id(s: &str) -> Id {
    Id {
        namespace: "claims".into(),
        slug: s.into(),
        version: 1,
    }
}
fn owner() -> Entity {
    Entity {
        kind: Some(entity::Kind::Domain(Domain {
            id: Some(id("owner")),
            message_claims: Some(MessageClaims {
                claims: vec![MessageClaim {
                    direction: CommunicationDirection::Receive as i32,
                    message: Some(ContractMemberRef {
                        specification: Some(SpecificationRef {
                            id: Some(id("messages")),
                        }),
                        owner_uid: "01930000-0000-7000-8000-000000000001".into(),
                        kind: ContractMemberKind::Message as i32,
                        member_id: Some(ContractMemberId {
                            value: "01930000-0000-7000-8000-000000000002".into(),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}
#[test]
fn claim_description_preserves_identity_and_independent_domain_reference() {
    let mut old = owner();
    connections::assign_connection_ids(&mut old).unwrap();
    let mut next = old.clone();
    if let Some(entity::Kind::Domain(d)) = &mut next.kind {
        d.message_claims.as_mut().unwrap().claims[0].group = Some(CatalogTitle {
            value: "incoming".into(),
        });
    }
    assert_eq!(
        connections::reconcile_connection_ids(&old, &mut next).unwrap(),
        0
    );
    let refs = outbound_refs(&next);
    assert!(refs
        .iter()
        .any(|r| r.from_field == "message_claims.claims[0].message.specification"));
}
#[test]
fn source_whole_owner_rejects_native_claims() {
    assert!(source_integration::extract_unit(&owner(), OwnershipUnit::WholeOwner).is_err());
}
#[test]
fn explicitly_empty_claim_identity_is_invalid() {
    let mut e = owner();
    if let Some(entity::Kind::Domain(d)) = &mut e.kind {
        d.message_claims.as_mut().unwrap().claims[0].connection_id = Some(ConnectionId {
            value: String::new(),
        });
    }
    assert!(connections::assign_connection_ids(&mut e).is_err());
}

fn with_route() -> Entity {
    let first = trogon_atlas_core::connection_persistence::prepare_entity(None, &owner()).unwrap();
    let mut next = first.clone();
    let v = trogon_atlas_core::message_claims::declarations_mut(&mut next).unwrap();
    v.routes.push(ClaimRoute { claim_id:v.claims[0].connection_id.clone(), channel:Some(ChannelRef{id:Some(id("channel"))}), ..Default::default() });
    trogon_atlas_core::connection_persistence::prepare_entity(Some(&first), &next).unwrap()
}
#[test]
fn explicit_branch_restoration_rewrites_only_selected_local_claim_dependencies() {
    use trogon_atlas_core::{connection_persistence as persistence, message_claims as claims};
    let base=with_route(); let mut opposing=base.clone();
    *claims::declarations_mut(&mut opposing).unwrap()=MessageClaims::default();
    let opposing=persistence::prepare_entity(Some(&base),&opposing).unwrap();
    assert!(persistence::prepare_landing(Some(&base),Some(&opposing),&base).is_err());
    let restored=persistence::prepare_resolution(Some(&base),Some(&opposing),&base).unwrap();
    let v=claims::declarations(&restored).unwrap();
    assert_ne!(v.claims[0].connection_id,claims::declarations(&base).unwrap().claims[0].connection_id);
    assert_eq!(v.routes[0].claim_id,v.claims[0].connection_id);
    assert_ne!(v.routes[0].connection_id,claims::declarations(&base).unwrap().routes[0].connection_id);
}
#[test]
fn partial_selected_claim_does_not_resolve_context() {
    let mut v=with_route();
    let r=ConnectionRef { owner:Some(EntityRef{kind:EntityKind::Domain as i32,id:Some(id("owner"))}), owner_uid:v.system.as_ref().unwrap().uid.clone(),id:trogon_atlas_core::message_claims::declarations(&v).unwrap().claims[0].connection_id.clone() };
    trogon_atlas_core::message_claims::declarations_mut(&mut v).unwrap().claims[0].message.as_mut().unwrap().member_id=None;
    assert!(trogon_atlas_core::message_claims::resolve_connection_context(&v,&r).is_err());
}

#[test]
fn route_parameters_require_authored_defaults_or_explicit_values() {
    use trogon_atlas_core::message_claims::validate_route_target;
    let channel=Channel{id:Some(id("channel")), address:Some(ChannelAddress{template:"{tenant}".into(),parameters:vec![AddressParameter{name:Some(ParameterName{value:"tenant".into()}),..Default::default()}]}),..Default::default()};
    let mut route=ClaimRoute{channel:Some(ChannelRef{id:channel.id.clone()}),..Default::default()};
    assert!(validate_route_target(&route,&channel).is_err());
    route.parameter_values.insert("tenant".into(),"".into());
    validate_route_target(&route,&channel).unwrap();
    let mut defaults=channel.clone();defaults.address.as_mut().unwrap().parameters[0].default=Some("default".into());
    route.parameter_values.clear(); validate_route_target(&route,&defaults).unwrap();
    route.parameter_values.insert("unknown".into(),"".into()); assert!(validate_route_target(&route,&defaults).is_err());
    defaults.address=None;assert!(validate_route_target(&route,&defaults).is_err());
    route.parameter_values.clear();defaults.id=Some(id("different"));assert!(validate_route_target(&route,&defaults).is_err());
}
#[test]
fn title_only_source_create_refuses_native_content_but_existing_title_preserves_it() {
    use trogon_atlas_core::message_claims::validate_source_write;
    let e=with_route();let title=OwnershipUnit::Field(SourceOwnedField::Title);
    assert!(validate_source_write(None,&e,title).is_err());
    validate_source_write(Some(&e),&e,title).unwrap();
    let mut cleared=e.clone(); *trogon_atlas_core::message_claims::declarations_mut(&mut cleared).unwrap()=MessageClaims::default();
    assert!(validate_source_write(Some(&e),&cleared,OwnershipUnit::WholeOwner).is_err());
}
#[test]
fn complete_values_limits_and_staged_local_ids_are_enforced_without_mutation() {
    use trogon_atlas_core::{connection_persistence::prepare_entity,message_claims as claims};
    let base=with_route();let mut wrong=base.clone();
    claims::declarations_mut(&mut wrong).unwrap().triggers.push(ClaimTrigger{claim_id:claims::declarations(&base).unwrap().claims[0].connection_id.clone(),message:claims::declarations(&base).unwrap().claims[0].message.clone(),..Default::default()});
    assert!(prepare_entity(Some(&base),&wrong).is_err());
    let mut new=base.clone();let v=claims::declarations_mut(&mut new).unwrap();let mut c=v.claims[0].clone();c.connection_id=Some(ConnectionId{value:"01930000-0000-7000-8000-000000000030".into()});v.claims.push(c.clone());v.routes[0].claim_id=c.connection_id;v.routes[0].connection_id=None;
    assert!(prepare_entity(Some(&base),&new).is_err());
    let mut many=owner();let v=claims::declarations_mut(&mut many).unwrap();v.claims=vec![v.claims[0].clone();1024];prepare_entity(None,&many).unwrap();
    let c=claims::declarations(&many).unwrap().claims[0].clone();claims::declarations_mut(&mut many).unwrap().claims.push(c);assert!(prepare_entity(None,&many).is_err());
    let mut large=owner();if let Some(entity::Kind::Domain(d))=&mut large.kind{d.doc="x".repeat(1_048_576);}assert!(prepare_entity(None,&large).is_err());
}

fn field_fixture() -> (Entity,ClaimFieldSelection,Entity,Entity) {
    use trogon_atlas_core::{connection_persistence::prepare_entity,message_claims as claims};
    let spec=prepare_entity(None,&Entity{kind:Some(entity::Kind::Specification(Specification{id:Some(id("messages")),messages:vec![ContractMessage{schemas:vec![SchemaBinding{role:schema_binding::Role::Payload as i32,r#type:Some(schema_binding::Type::Schema(SchemaRef{id:Some(id("payload"))})),..Default::default()}],..Default::default()}],..Default::default()})),..Default::default()}).unwrap();
    let Some(entity::Kind::Specification(s))=&spec.kind else{panic!()};
    let mut e=owner();let v=claims::declarations_mut(&mut e).unwrap();let m=v.claims[0].message.as_mut().unwrap();m.owner_uid=spec.system.as_ref().unwrap().uid.clone();m.member_id=s.messages[0].member_id.clone();
    let e=prepare_entity(None,&e).unwrap();
    let selection=ClaimFieldSelection{claim_id:claims::declarations(&e).unwrap().claims[0].connection_id.clone(),binding:Some(ConnectionRef{owner:Some(EntityRef{kind:EntityKind::Specification as i32,id:s.id.clone()}),owner_uid:spec.system.as_ref().unwrap().uid.clone(),id:s.messages[0].schemas[0].connection_id.clone()}),selectors:vec![ClaimFieldSelector{selector:Some(claim_field_selector::Selector::Path(FieldPath{segments:vec!["customer".into(),"name".into()]}))}],..Default::default()};
    let schema=Entity{kind:Some(entity::Kind::Schema(Schema{id:Some(id("payload")),fields:vec![FieldSpec{name:"customer".into(),r#type:Some(FieldType{kind:Some(field_type::Kind::Object(field_type::ObjectType{fields:vec![FieldSpec{name:"name".into(),..Default::default()}]}))}),..Default::default()}],..Default::default()})),..Default::default()};
    (e,selection,spec,schema)
}
#[test]
fn payload_selection_resolves_exact_message_binding_and_inline_paths_only() {
    use trogon_atlas_core::message_claims::{validate_field_selection,FieldSelectorVerification};
    let (e,mut selection,mut spec,schema)=field_fixture();
    assert_eq!(validate_field_selection(&e,&selection,&spec,Some(&schema)).unwrap(),vec![FieldSelectorVerification::NativePath]);
    selection.selectors[0].selector=Some(claim_field_selector::Selector::Path(FieldPath{segments:vec!["missing".into()]}));assert!(validate_field_selection(&e,&selection,&spec,Some(&schema)).is_err());
    selection.selectors[0].selector=Some(claim_field_selector::Selector::Opaque(OpaqueTypeSelector{syntax:"json-pointer".into(),value:"/unknown".into()}));
    assert_eq!(validate_field_selection(&e,&selection,&spec,None).unwrap(),vec![FieldSelectorVerification::OpaqueUnverified]);
    if let Some(entity::Kind::Specification(s))=&mut spec.kind{s.messages[0].schemas[0].role=schema_binding::Role::Headers as i32;}
    assert!(validate_field_selection(&e,&selection,&spec,None).is_err());
}
