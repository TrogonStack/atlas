#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_core::{
    architectural_relationships::{self as architecture, validate_values},
    connection_persistence::{prepare_entity, prepare_landing, prepare_resolution},
    connections::{self, connection_occurrences, ConnectionError, ConnectionId},
    decision_document, entity_id, entity_kind, outbound_refs, responsibility, retarget_refs,
    source_integration::{self, SourceError},
};
use trogon_atlas_proto::{self as pb, *};

fn id(slug: &str) -> Id {
    Id {
        namespace: "commerce".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn endpoint(external: bool, slug: &str) -> ArchitecturalEndpoint {
    ArchitecturalEndpoint {
        target: Some(if external {
            architectural_endpoint::Target::ExternalSystem(ExternalSystemRef { id: Some(id(slug)) })
        } else {
            architectural_endpoint::Target::Component(ComponentRef { id: Some(id(slug)) })
        }),
    }
}
fn relation(external: bool, slug: &str) -> ArchitecturalRelationship {
    ArchitecturalRelationship {
        target: Some(endpoint(external, slug)),
        label: Some(CatalogTitle {
            value: "submits orders".into(),
        }),
        doc: Some(CatalogText {
            value: "architectural intent".into(),
        }),
        ..Default::default()
    }
}
fn owner(external: bool) -> Entity {
    let relationships = vec![
        relation(true, "provider"),
        relation(true, "provider"),
        relation(false, "worker"),
    ];
    Entity {
        kind: Some(if external {
            entity::Kind::ExternalSystem(ExternalSystem {
                id: Some(id("checkout")),
                relationships,
                ..Default::default()
            })
        } else {
            entity::Kind::Component(Component {
                id: Some(id("checkout")),
                relationships,
                members: vec![EntityRef {
                    kind: EntityKind::Component as i32,
                    id: Some(id("worker")),
                }],
                ..Default::default()
            })
        }),
        ..Default::default()
    }
}
fn values(owner: &mut Entity) -> &mut Vec<ArchitecturalRelationship> {
    match owner.kind.as_mut().unwrap() {
        entity::Kind::Component(value) => &mut value.relationships,
        entity::Kind::ExternalSystem(value) => &mut value.relationships,
        _ => panic!("architectural owner"),
    }
}
fn reference(owner: &Entity, index: usize) -> ConnectionRef {
    ConnectionRef {
        owner: Some(EntityRef {
            kind: entity_kind(owner).unwrap() as i32,
            id: entity_id(owner).cloned(),
        }),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id: architecture::relationships(owner)[index]
            .connection_id
            .clone(),
    }
}

#[test]
fn typed_endpoints_preserve_parallel_occurrences_and_plain_members() {
    for external in [false, true] {
        let stored = prepare_entity(None, &owner(external)).unwrap();
        validate_values(&stored).unwrap();
        assert_eq!(
            Entity::decode(stored.encode_to_vec().as_slice()).unwrap(),
            stored
        );
        let occurrences = connection_occurrences(&stored).unwrap();
        assert_eq!(occurrences.len(), 3);
        assert_ne!(occurrences[0].id, occurrences[1].id);
        assert_eq!(occurrences[0].target, occurrences[1].target);
        let refs = outbound_refs(&stored);
        let declarations: Vec<_> = refs
            .iter()
            .filter(|r| r.from_field.starts_with("relationships["))
            .collect();
        assert_eq!(declarations.len(), 3);
        assert_eq!(
            declarations[0].from_field,
            "relationships[0].target.external_system"
        );
        assert_eq!(declarations[2].to_kind, Some(EntityKind::Component));
        assert_eq!(declarations[2].to_id, id("worker"));
        for (index, occurrence) in occurrences.iter().enumerate() {
            let selected = reference(&stored, index);
            let target = architecture::resolve_connection_context(&stored, &selected).unwrap();
            assert_eq!(Some(target), occurrence.target);
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
        let mut member_selection = reference(&stored, 0);
        member_selection.id = Some(ConnectionId::new().into());
        assert!(architecture::resolve_connection_context(&stored, &member_selection).is_err());
    }
}

#[test]
fn wording_and_order_preserve_identity_but_exact_targets_do_not() {
    let stored = prepare_entity(None, &owner(false)).unwrap();
    let mut edited = stored.clone();
    values(&mut edited).reverse();
    values(&mut edited)[2].label.as_mut().unwrap().value = "places requests".into();
    values(&mut edited)[2].doc = None;
    let edited = prepare_entity(Some(&stored), &edited).unwrap();
    assert_eq!(reference(&stored, 0), reference(&edited, 2));
    assert!(connections::resolve_connection(&edited, &reference(&stored, 0)).is_ok());
    let mut changed = edited.clone();
    values(&mut changed)[0].target = Some(endpoint(true, "worker"));
    assert!(matches!(
        prepare_entity(Some(&edited), &changed),
        Err(ConnectionError::Retargeted(_))
    ));
    values(&mut changed)[0].connection_id = None;
    let changed = prepare_entity(Some(&edited), &changed).unwrap();
    assert_ne!(reference(&changed, 0).id, reference(&edited, 0).id);
}

#[test]
fn exact_content_reapply_retains_ids_but_edited_stripping_and_preview_ids_reject() {
    let authored = owner(false);
    let stored = prepare_entity(None, &authored).unwrap();
    assert_eq!(prepare_entity(Some(&stored), &authored).unwrap(), stored);
    let mut edited = authored.clone();
    values(&mut edited)[0].label.as_mut().unwrap().value = "changed".into();
    assert!(matches!(
        prepare_entity(Some(&stored), &edited),
        Err(ConnectionError::StrippedId(_))
    ));
    let mut added = stored.clone();
    values(&mut added).push(relation(false, "worker"));
    let preview = prepare_entity(Some(&stored), &added).unwrap();
    assert!(matches!(
        prepare_entity(Some(&stored), &preview),
        Err(ConnectionError::UnknownId(_))
    ));
    let applied = prepare_entity(Some(&stored), &added).unwrap();
    assert_ne!(reference(&preview, 3).id, reference(&applied, 3).id);
}

#[test]
fn local_removal_restoration_and_recreation_keep_selectors_exact() {
    let stored = prepare_entity(None, &owner(false)).unwrap();
    let selected = reference(&stored, 0);
    let sibling = reference(&stored, 1);
    let mut removed = stored.clone();
    values(&mut removed).remove(0);
    let removed = prepare_entity(Some(&stored), &removed).unwrap();
    assert!(connections::resolve_connection(&removed, &selected).is_err());
    assert!(connections::resolve_connection(&removed, &sibling).is_ok());
    assert!(matches!(
        prepare_landing(Some(&stored), Some(&removed), &stored),
        Err(ConnectionError::RemovedFromBranchBase(_))
    ));
    let restored = prepare_resolution(Some(&stored), Some(&removed), &stored).unwrap();
    assert_ne!(reference(&restored, 0).id, selected.id);
    assert_eq!(reference(&restored, 1).id, sibling.id);
    assert_eq!(reference(&restored, 0).owner_uid, selected.owner_uid);
    let recreated = prepare_entity(None, &stored).unwrap();
    assert_ne!(reference(&recreated, 0).owner_uid, selected.owner_uid);
    assert!(connections::resolve_connection(&recreated, &selected).is_err());
    let mut readded = removed.clone();
    values(&mut readded).push(relation(true, "provider"));
    let readded = prepare_entity(Some(&removed), &readded).unwrap();
    assert_ne!(reference(&readded, 2).id, selected.id);
}

#[test]
fn retarget_renews_only_matching_occurrences_and_never_moves_subject_selectors() {
    let stored = prepare_entity(None, &owner(false)).unwrap();
    let selected = reference(&stored, 0);
    let mut changed = stored.clone();
    assert_eq!(
        retarget_refs(
            &mut changed,
            EntityKind::ExternalSystem,
            &id("provider"),
            &id("replacement")
        ),
        vec![
            "relationships[0].target.external_system",
            "relationships[1].target.external_system"
        ]
    );
    assert!(values(&mut changed)[0].connection_id.is_none());
    let changed = prepare_entity(Some(&stored), &changed).unwrap();
    assert!(connections::resolve_connection(&changed, &selected).is_err());
    assert_eq!(reference(&stored, 2), reference(&changed, 2));
    let mut team = Entity {
        kind: Some(entity::Kind::Team(Team {
            responsibilities: vec![Responsibility {
                subject: Some(ResponsibilitySubject {
                    selection: Some(responsibility_subject::Selection::Connection(
                        selected.clone(),
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    let before = team.clone();
    assert_eq!(
        retarget_refs(
            &mut team,
            EntityKind::Component,
            &id("checkout"),
            &id("replacement")
        ),
        Vec::<String>::new()
    );
    assert_eq!(team, before);
}

#[test]
fn complete_inventory_is_checked_before_selecting_readable_declaration() {
    let stored = prepare_entity(None, &owner(false)).unwrap();
    let selected = reference(&stored, 0);
    let mut corrupted = stored.clone();
    values(&mut corrupted)[2].connection_id = None;
    assert!(architecture::resolve_connection_context(&corrupted, &selected).is_err());
    values(&mut corrupted)[2].connection_id = selected.id.clone();
    assert!(architecture::resolve_connection_context(&corrupted, &selected).is_err());
    let mut malformed = stored.clone();
    values(&mut malformed)[2]
        .connection_id
        .as_mut()
        .unwrap()
        .value = "not-a-uuid".into();
    assert!(architecture::resolve_connection_context(&malformed, &selected).is_err());
    // Resolution uses retained identity and does not require target hydration.
    assert_eq!(
        architecture::resolve_connection_context(&stored, &selected)
            .unwrap()
            .id,
        Some(id("provider"))
    );
}

#[test]
fn exact_endpoint_and_rich_text_bounds_reject_without_truncation() {
    for external in [false, true] {
        let valid = owner(external);
        validate_values(&valid).unwrap();
        let mut bad = valid.clone();
        values(&mut bad)[0].target = None;
        assert!(validate_values(&bad).is_err());
        values(&mut bad)[0].target = Some(ArchitecturalEndpoint::default());
        assert!(validate_values(&bad).is_err());
        values(&mut bad)[0].target = Some(endpoint(external, "checkout"));
        assert!(validate_values(&bad).is_err());
        let target = values(&mut bad)[0].target.as_mut().unwrap();
        match target.target.as_mut().unwrap() {
            architectural_endpoint::Target::Component(v) => v.id.as_mut().unwrap().version = 2,
            architectural_endpoint::Target::ExternalSystem(v) => v.id.as_mut().unwrap().version = 2,
        }
        validate_values(&bad).unwrap();
        for invalid in [
            None,
            Some(Id {
                version: 0,
                ..id("target")
            }),
            Some(Id {
                namespace: "../bad".into(),
                ..id("target")
            }),
        ] {
            values(&mut bad)[0].target = Some(ArchitecturalEndpoint {
                target: Some(architectural_endpoint::Target::Component(ComponentRef {
                    id: invalid,
                })),
            });
            assert!(validate_values(&bad).is_err());
        }
        for text in [String::new(), " \n".into(), "é".repeat(129)] {
            let mut bad = valid.clone();
            values(&mut bad)[0].label = Some(CatalogTitle { value: text });
            assert!(validate_values(&bad).is_err());
        }
        let mut bad = valid.clone();
        values(&mut bad)[0].label = None;
        assert!(validate_values(&bad).is_err());
        for text in [String::new(), " \n".into(), "é".repeat(8193)] {
            let mut bad = valid.clone();
            values(&mut bad)[0].doc = Some(CatalogText { value: text });
            assert!(validate_values(&bad).is_err());
        }
        let mut limit = valid;
        values(&mut limit)[0].label = Some(CatalogTitle {
            value: "é".repeat(128),
        });
        values(&mut limit)[0].doc = Some(CatalogText {
            value: "é".repeat(8192),
        });
        validate_values(&limit).unwrap();
    }
    let mut limit = owner(false);
    *values(&mut limit) = vec![relation(true, "provider"); architecture::MAX_RELATIONSHIPS];
    validate_values(&limit).unwrap();
    values(&mut limit).push(relation(true, "provider"));
    assert!(validate_values(&limit).is_err());
    assert_eq!(
        values(&mut limit).len(),
        architecture::MAX_RELATIONSHIPS + 1
    );
    *values(&mut limit) = vec![relation(true, "provider"); 100];
    for value in values(&mut limit) {
        value.doc = Some(CatalogText {
            value: "d".repeat(16_384),
        });
    }
    assert!(limit.encoded_len() > architecture::MAX_ENCODED_BYTES);
    assert!(validate_values(&limit).is_err());
}

#[test]
fn supplied_identity_is_validated_and_source_architecture_is_native_only() {
    for external in [false, true] {
        let mut authored = owner(external);
        let supplied: pb::ConnectionId = ConnectionId::new().into();
        values(&mut authored)[0].connection_id = Some(supplied.clone());
        let stored = prepare_entity(None, &authored).unwrap();
        assert_eq!(reference(&stored, 0).id, Some(supplied.clone()));
        values(&mut authored)[1].connection_id = Some(supplied);
        assert!(validate_values(&authored).is_err());
        assert!(prepare_entity(None, &authored).is_err());
        let selection = SourceSelection {
            selection: Some(source_selection::Selection::Connection(reference(
                &stored, 0,
            ))),
        };
        let none = SourceFieldOwnership {
            policy: Some(source_field_ownership::Policy::None(SourceNoOwnership {})),
        };
        assert_eq!(
            source_integration::validate_selection_ownership(&selection, &none),
            Err(SourceError::UnsupportedUnit)
        );
        let fields = SourceFieldOwnership {
            policy: Some(source_field_ownership::Policy::Fields(SourceOwnedFields {
                fields: vec![SourceOwnedField::Relationships as i32],
            })),
        };
        assert_eq!(
            source_integration::validate_ownership(&stored, &fields),
            Err(SourceError::UnsupportedUnit)
        );
        let whole = SourceFieldOwnership {
            policy: Some(source_field_ownership::Policy::WholeOwner(
                SourceWholeOwnerOwnership {},
            )),
        };
        assert_eq!(
            source_integration::validate_ownership(&stored, &whole),
            Err(SourceError::UnsupportedUnit)
        );
        let title = SourceFieldOwnership {
            policy: Some(source_field_ownership::Policy::Fields(SourceOwnedFields {
                fields: vec![SourceOwnedField::Title as i32],
            })),
        };
        let unit = source_integration::validate_ownership(&stored, &title).unwrap()[0];
        let selected = source_integration::extract_unit(&stored, unit).unwrap();
        let mut merged = stored.clone();
        source_integration::replace_unit(&mut merged, unit, &selected).unwrap();
        assert_eq!(
            architecture::relationships(&merged),
            architecture::relationships(&stored)
        );
    }
}

#[test]
fn architectural_subject_resolvers_reject_non_exact_selected_endpoints() {
    let invalid_ids = [
        None,
        Some(Id {
            version: 0,
            ..id("target")
        }),
        Some(Id {
            namespace: "../unsafe".into(),
            ..id("target")
        }),
        Some(Id {
            slug: "../unsafe".into(),
            ..id("target")
        }),
    ];
    for external_owner in [false, true] {
        let stored = prepare_entity(None, &owner(external_owner)).unwrap();
        for external_target in [false, true] {
            for invalid_id in &invalid_ids {
                let mut malformed = stored.clone();
                values(&mut malformed)[0].target = Some(ArchitecturalEndpoint {
                    target: Some(if external_target {
                        architectural_endpoint::Target::ExternalSystem(ExternalSystemRef {
                            id: invalid_id.clone(),
                        })
                    } else {
                        architectural_endpoint::Target::Component(ComponentRef {
                            id: invalid_id.clone(),
                        })
                    }),
                });
                let selected = reference(&malformed, 0);
                assert!(architecture::resolve_connection_context(&malformed, &selected).is_err(), "owner external={external_owner}, target external={external_target}, id={invalid_id:?}");
                assert!(responsibility::resolve_subject(
                    &ResponsibilitySubject {
                        selection: Some(responsibility_subject::Selection::Connection(
                            selected.clone()
                        )),
                    },
                    &malformed
                )
                .is_err());
                assert!(decision_document::resolve_subject(
                    &CatalogSubject {
                        selection: Some(catalog_subject::Selection::Connection(selected)),
                    },
                    &malformed
                )
                .is_err());
            }
        }
    }
}

#[test]
fn architectural_values_reject_explicit_empty_connection_id_but_allow_omission() {
    for external_owner in [false, true] {
        let mut authored = owner(external_owner);
        assert!(values(&mut authored)[0].connection_id.is_none());
        validate_values(&authored).unwrap();
        values(&mut authored)[0].connection_id = Some(pb::ConnectionId {
            value: String::new(),
        });
        assert!(
            validate_values(&authored).is_err(),
            "explicit empty connection ID is malformed on external={external_owner}"
        );
    }
}
