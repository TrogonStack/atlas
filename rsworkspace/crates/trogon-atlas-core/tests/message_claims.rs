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
