#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_core::{
    architectural_relationships,
    communication_intent::{self as intent, validate_values},
    connection_persistence::prepare_entity,
    connections::{self, ConnectionError, ConnectionId},
    decision_document, entity_id, entity_kind, outbound_refs, responsibility, retarget_refs,
    source_integration,
};
use trogon_atlas_proto::{self as pb, *};

fn id(slug: &str) -> Id {
    Id {
        namespace: "commerce".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn peer(external: bool) -> ArchitecturalEndpoint {
    ArchitecturalEndpoint {
        target: Some(if external {
            architectural_endpoint::Target::ExternalSystem(ExternalSystemRef {
                id: Some(id("provider")),
            })
        } else {
            architectural_endpoint::Target::Component(ComponentRef {
                id: Some(id("worker")),
            })
        }),
    }
}
fn declaration(external: bool) -> CommunicationDeclaration {
    CommunicationDeclaration {
        direction: CommunicationDirection::Send as i32,
        peer: Some(peer(external)),
        message: Some(ContractMemberRef {
            specification: Some(SpecificationRef {
                id: Some(id("notifications")),
            }),
            owner_uid: "01930000-0000-7000-8000-000000000001".into(),
            kind: ContractMemberKind::Message as i32,
            member_id: Some(ContractMemberId {
                value: "01930000-0000-7000-8000-000000000002".into(),
            }),
            ..Default::default()
        }),
        label: Some(CatalogTitle {
            value: "notifies".into(),
        }),
        condition: Some(CatalogText {
            value: "when ready".into(),
        }),
        ..Default::default()
    }
}
fn owner(external: bool, target_external: bool) -> Entity {
    let communications = vec![declaration(target_external), declaration(target_external)];
    let relationships = vec![ArchitecturalRelationship {
        target: Some(peer(target_external)),
        label: Some(CatalogTitle {
            value: "integrates".into(),
        }),
        ..Default::default()
    }];
    Entity {
        kind: Some(if external {
            entity::Kind::ExternalSystem(ExternalSystem {
                id: Some(id("checkout")),
                relationships,
                communications,
                ..Default::default()
            })
        } else {
            entity::Kind::Component(Component {
                id: Some(id("checkout")),
                relationships,
                communications,
                ..Default::default()
            })
        }),
        ..Default::default()
    }
}
fn values(owner: &mut Entity) -> &mut Vec<CommunicationDeclaration> {
    match owner.kind.as_mut().unwrap() {
        entity::Kind::Component(v) => &mut v.communications,
        entity::Kind::ExternalSystem(v) => &mut v.communications,
        _ => panic!("owner"),
    }
}
fn reference(owner: &Entity, index: usize) -> ConnectionRef {
    ConnectionRef {
        owner: Some(EntityRef {
            kind: entity_kind(owner).unwrap() as i32,
            id: entity_id(owner).cloned(),
        }),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id: intent::declarations(owner)[index].connection_id.clone(),
    }
}

#[test]
fn exact_parallel_communications_share_inventory_and_preserve_subjects() {
    for external in [false, true] {
        for target_external in [false, true] {
            for direction in [
                CommunicationDirection::Send,
                CommunicationDirection::Receive,
            ] {
                let mut proposed = owner(external, target_external);
                for value in values(&mut proposed) {
                    value.direction = direction as i32;
                }
                validate_values(&proposed).unwrap();
                let stored = prepare_entity(None, &proposed).unwrap();
                assert_eq!(
                    Entity::decode(stored.encode_to_vec().as_slice()).unwrap(),
                    stored
                );
                assert_eq!(
                    connections::connection_occurrences(&stored).unwrap().len(),
                    3
                );
                assert_ne!(reference(&stored, 0).id, reference(&stored, 1).id);
                let refs: Vec<_> = outbound_refs(&stored)
                    .into_iter()
                    .filter(|r| r.from_field.starts_with("communications["))
                    .collect();
                assert_eq!(refs.len(), 4);
                assert_eq!(
                    refs[1].from_field,
                    "communications[0].message.specification"
                );
                for index in 0..2 {
                    let selected = reference(&stored, index);
                    assert_eq!(
                        intent::resolve_connection_context(&stored, &selected).unwrap(),
                        architectural_relationships::endpoint_ref(&peer(target_external)).unwrap()
                    );
                    assert_eq!(
                        intent::message_selection(&stored, &selected).unwrap(),
                        declaration(target_external).message.unwrap()
                    );
                    responsibility::resolve_subject(
                        &ResponsibilitySubject {
                            selection: Some(responsibility_subject::Selection::Connection(
                                selected.clone(),
                            )),
                        },
                        &stored,
                    )
                    .unwrap();
                    decision_document::resolve_subject(
                        &CatalogSubject {
                            selection: Some(catalog_subject::Selection::Connection(selected)),
                        },
                        &stored,
                    )
                    .unwrap();
                }
                assert_eq!(prepare_entity(Some(&stored), &proposed).unwrap(), stored);
            }
        }
    }
}
#[test]
fn wording_condition_and_order_preserve_but_semantics_renew() {
    let stored = prepare_entity(None, &owner(false, true)).unwrap();
    let mut edited = stored.clone();
    values(&mut edited).reverse();
    values(&mut edited)[0].condition = None;
    values(&mut edited)[1].label.as_mut().unwrap().value = "different wording".into();
    let edited = prepare_entity(Some(&stored), &edited).unwrap();
    assert_eq!(reference(&stored, 0), reference(&edited, 1));
    for field in ["direction", "peer", "message", "uid", "member"] {
        let mut changed = edited.clone();
        let value = &mut values(&mut changed)[0];
        match field {
            "direction" => value.direction = CommunicationDirection::Receive as i32,
            "peer" => value.peer = Some(peer(false)),
            "message" => {
                value
                    .message
                    .as_mut()
                    .unwrap()
                    .specification
                    .as_mut()
                    .unwrap()
                    .id
                    .as_mut()
                    .unwrap()
                    .version = 2;
            }
            "uid" => {
                value.message.as_mut().unwrap().owner_uid =
                    "01930000-0000-7000-8000-000000000003".into();
            }
            _ => {
                value
                    .message
                    .as_mut()
                    .unwrap()
                    .member_id
                    .as_mut()
                    .unwrap()
                    .value = "01930000-0000-7000-8000-000000000004".into();
            }
        }
        assert!(matches!(
            prepare_entity(Some(&edited), &changed),
            Err(ConnectionError::Retargeted(_))
        ));
        values(&mut changed)[0].connection_id = None;
        let changed = prepare_entity(Some(&edited), &changed).unwrap();
        assert_ne!(reference(&edited, 0).id, reference(&changed, 0).id);
    }
    let mut stripped = edited.clone();
    values(&mut stripped)[0].connection_id = None;
    assert!(matches!(
        prepare_entity(Some(&edited), &stripped),
        Err(ConnectionError::StrippedId(_))
    ));
}
#[test]
fn shape_and_cross_collection_ids_reject() {
    for case in 0..15 {
        let mut invalid = owner(false, true);
        let value = &mut values(&mut invalid)[0];
        match case {
            0 => value.direction = 0,
            1 => value.direction = 999,
            2 => value.peer = None,
            3 => value.peer.as_mut().unwrap().target = None,
            4 => {
                value.peer = Some(ArchitecturalEndpoint {
                    target: Some(architectural_endpoint::Target::Component(ComponentRef {
                        id: Some(id("checkout")),
                    })),
                });
            }
            5 => value.message = None,
            6 => value.message.as_mut().unwrap().kind = ContractMemberKind::Schema as i32,
            7 => {
                value.message.as_mut().unwrap().expected_digest = Some(ContentDigest {
                    sha256: "a".repeat(64),
                });
            }
            8 => value.message.as_mut().unwrap().owner_uid.clear(),
            9 => value.message.as_mut().unwrap().member_id = None,
            10 => {
                value
                    .message
                    .as_mut()
                    .unwrap()
                    .specification
                    .as_mut()
                    .unwrap()
                    .id
                    .as_mut()
                    .unwrap()
                    .version = 0;
            }
            11 => {
                value.connection_id = Some(pb::ConnectionId {
                    value: String::new(),
                });
            }
            12 => value.label.as_mut().unwrap().value = " ".into(),
            13 => {
                value.condition = Some(CatalogText {
                    value: String::new(),
                });
            }
            _ => {
                value.doc = Some(CatalogText {
                    value: "a".repeat(16_385),
                });
            }
        }
        assert!(validate_values(&invalid).is_err(), "case {case}");
    }
    let mut duplicate = prepare_entity(None, &owner(false, true)).unwrap();
    let relation_id = architectural_relationships::relationships(&duplicate)[0]
        .connection_id
        .clone();
    values(&mut duplicate)[0].connection_id = relation_id;
    assert!(validate_values(&duplicate).is_err());
}
#[test]
fn selected_partial_peer_and_message_reject_without_sibling_repair() {
    let stored = prepare_entity(None, &owner(false, true)).unwrap();
    let selected = reference(&stored, 0);
    let mut invalid = stored.clone();
    values(&mut invalid)[0].peer = Some(ArchitecturalEndpoint {
        target: Some(architectural_endpoint::Target::ExternalSystem(
            ExternalSystemRef { id: None },
        )),
    });
    assert!(intent::resolve_connection_context(&invalid, &selected).is_err());
    assert!(responsibility::resolve_subject(
        &ResponsibilitySubject {
            selection: Some(responsibility_subject::Selection::Connection(
                selected.clone()
            ))
        },
        &invalid
    )
    .is_err());
    let mut invalid = stored.clone();
    values(&mut invalid)[0].message.as_mut().unwrap().member_id = None;
    assert!(intent::message_selection(&invalid, &selected).is_err());
    assert!(responsibility::resolve_subject(
        &ResponsibilitySubject {
            selection: Some(responsibility_subject::Selection::Connection(
                selected.clone()
            ))
        },
        &invalid
    )
    .is_err());
    assert!(decision_document::resolve_subject(
        &CatalogSubject {
            selection: Some(catalog_subject::Selection::Connection(selected.clone()))
        },
        &invalid
    )
    .is_err());
    let mut incomplete = stored.clone();
    values(&mut incomplete)[1].connection_id = None;
    assert!(intent::resolve_connection_context(&incomplete, &selected).is_err());
    let mut wrong = selected;
    wrong.owner_uid = "01930000-0000-7000-8000-000000000003".into();
    assert!(intent::message_selection(&stored, &wrong).is_err());
}
#[test]
fn retarget_only_peer_never_rebinds_message_and_source_is_native_only() {
    let stored = prepare_entity(None, &owner(false, true)).unwrap();
    let mut changed = stored.clone();
    let paths = retarget_refs(
        &mut changed,
        EntityKind::ExternalSystem,
        &id("provider"),
        &id("replacement"),
    );
    assert_eq!(paths.len(), 3);
    assert!(intent::declarations(&changed)
        .iter()
        .all(|v| v.connection_id.is_none()));
    let mut spec = stored.clone();
    assert_eq!(
        retarget_refs(
            &mut spec,
            EntityKind::Specification,
            &id("notifications"),
            &id("replacement")
        ),
        [] as [String; 0]
    );
    assert_eq!(spec, stored);
    let whole = SourceFieldOwnership {
        policy: Some(source_field_ownership::Policy::WholeOwner(
            SourceWholeOwnerOwnership {},
        )),
    };
    assert!(source_integration::validate_ownership(&stored, &whole).is_err());
    let mut selection = SourceSelection {
        selection: Some(source_selection::Selection::Connection(reference(
            &stored, 0,
        ))),
    };
    assert!(source_integration::validate_selection_ownership(&selection, &whole).is_err());
    selection.selection = Some(source_selection::Selection::Connection(ConnectionRef {
        id: Some(ConnectionId::new().into()),
        ..reference(&stored, 0)
    }));
    assert!(source_integration::validate_selection_ownership(&selection, &whole).is_err());
}
#[test]
fn rich_value_and_owner_bounds_apply_without_truncation() {
    let mut valid = owner(false, true);
    values(&mut valid)[0].label.as_mut().unwrap().value = "é".repeat(128);
    values(&mut valid)[0].condition.as_mut().unwrap().value = "é".repeat(8192);
    validate_values(&valid).unwrap();
    values(&mut valid)[0]
        .label
        .as_mut()
        .unwrap()
        .value
        .push('a');
    assert!(validate_values(&valid).is_err());
    let mut count = owner(false, true);
    values(&mut count).resize(intent::MAX_COMMUNICATIONS, declaration(true));
    validate_values(&count).unwrap();
    values(&mut count).push(declaration(true));
    assert!(validate_values(&count).is_err());
    let mut large = owner(false, true);
    let Some(entity::Kind::Component(value)) = &mut large.kind else {
        panic!("component")
    };
    value.doc = "x".repeat(intent::MAX_ENCODED_BYTES);
    assert!(validate_values(&large).is_err());
}
