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
