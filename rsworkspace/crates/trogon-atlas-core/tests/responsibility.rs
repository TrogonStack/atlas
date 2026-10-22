#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{
    connection_persistence::{prepare_entity, prepare_landing, prepare_resolution},
    connections::{
        connection_occurrences, renew_retargeted_connection_ids, validate_stored_connection_ids,
        ConnectionId,
    },
    entity_id, entity_kind, outbound_refs,
    responsibility::{self, resolve_subject, validate_subject, validate_values},
    retarget_refs,
};
use trogon_atlas_proto::*;

fn id(slug: &str) -> Id {
    Id {
        namespace: "catalog".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn entity_ref(kind: EntityKind, slug: &str) -> EntityRef {
    EntityRef {
        kind: kind as i32,
        id: Some(id(slug)),
    }
}
fn subject(kind: EntityKind, slug: &str) -> ResponsibilitySubject {
    ResponsibilitySubject {
        selection: Some(responsibility_subject::Selection::Entity(entity_ref(
            kind, slug,
        ))),
    }
}
fn assignment(subject: ResponsibilitySubject) -> Responsibility {
    Responsibility {
        kind: ResponsibilityKind::Owner as i32,
        subject: Some(subject),
        ..Default::default()
    }
}
fn authored() -> Entity {
    Entity {
        kind: Some(entity::Kind::Team(Team {
            id: Some(id("fulfillment")),
            title: Some(DirectoryDisplayName {
                value: "Fulfillment".into(),
            }),
            members: vec![TeamMember {
                person: Some(PersonRef {
                    id: Some(id("pat")),
                }),
                ..Default::default()
            }],
            responsibilities: vec![assignment(subject(EntityKind::Event, "order-placed"))],
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn team(entity: &Entity) -> &Team {
    let Some(entity::Kind::Team(value)) = &entity.kind else {
        panic!("expected Team")
    };
    value
}
fn team_mut(entity: &mut Entity) -> &mut Team {
    let Some(entity::Kind::Team(value)) = &mut entity.kind else {
        panic!("expected Team")
    };
    value
}
fn stored() -> Entity {
    prepare_entity(None, &authored()).unwrap()
}
fn ids(entity: &Entity) -> Vec<Option<ConnectionId>> {
    connection_occurrences(entity)
        .unwrap()
        .into_iter()
        .map(|o| o.id)
        .collect()
}
fn selected_connection(owner: &Entity, index: usize) -> ResponsibilitySubject {
    ResponsibilitySubject {
        selection: Some(responsibility_subject::Selection::Connection(
            ConnectionRef {
                owner: Some(EntityRef {
                    kind: entity_kind(owner).unwrap() as i32,
                    id: entity_id(owner).cloned(),
                }),
                owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                id: connection_occurrences(owner).unwrap()[index]
                    .id
                    .map(Into::into),
            },
        )),
    }
}
fn behavioral_owner() -> Entity {
    prepare_entity(
        None,
        &Entity {
            kind: Some(entity::Kind::AutomationSlice(AutomationSlice {
                id: Some(id("react")),
                source_read_models: (0..2)
                    .map(|_| ReadModelEdge {
                        read_model: Some(ReadModelRef {
                            id: Some(id("receipt")),
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn wire_allocations_revision_and_presence_are_native() {
    assert_eq!(
        (EntityKind::Team as i32, EntityKind::Person as i32),
        (34, 35)
    );
    assert_eq!(
        (
            CONTRACT_REVISION,
            MIN_CLIENT_CONTRACT_REVISION,
            MIN_SERVER_CONTRACT_REVISION
        ),
        (14, 14, 14)
    );
    assert_eq!(canonical::parse_kind("team"), Some(EntityKind::Team));
    assert_eq!(canonical::parse_kind("person"), Some(EntityKind::Person));
    let pool = prost_reflect::DescriptorPool::decode(FILE_DESCRIPTOR_SET).unwrap();
    let entity = pool
        .get_message_by_name("trogonatlas.eventmodel.v1alpha1.Entity")
        .unwrap();
    assert_eq!(entity.get_field_by_name("team").unwrap().number(), 35);
    assert_eq!(entity.get_field_by_name("person").unwrap().number(), 36);
    for name in [
        "ListSubjectAssignments",
        "ListAssigneeResponsibilities",
        "ListTeamMembers",
        "ListPersonTeams",
        "GetTeamContacts",
    ] {
        assert!(pool
            .services()
            .any(|s| s.methods().any(|m| m.name() == name)));
    }
}

#[test]
fn directory_profiles_contacts_and_optional_presence_validate() {
    validate_values(&authored()).unwrap();
    let email = ContactPoint {
        channel: Some(contact_point::Channel::Email(EmailAddress {
            value: "pat@example.test".into(),
        })),
        label: None,
    };
    responsibility::validate_contact(&email).unwrap();
    for address in ["", "a@@b", "@b", "a@", "a b@c", "é@b", "a\u{7f}@b"] {
        let contact = ContactPoint {
            channel: Some(contact_point::Channel::Email(EmailAddress {
                value: address.into(),
            })),
            label: None,
        };
        assert!(
            responsibility::validate_contact(&contact).is_err(),
            "{address:?}"
        );
    }
    for address in [
        "https://example.test/contact",
        "https://example.test:8443/a?q=b#c",
    ] {
        responsibility::validate_contact(&ContactPoint {
            channel: Some(contact_point::Channel::Web(ContactUrl {
                value: address.into(),
            })),
            label: None,
        })
        .unwrap();
    }
    for address in [
        "http://example.test",
        "https:///",
        "https://a:b@example.test",
        "https://@example.test",
        "https:example.test",
        "relative",
        "https://example.test/\n",
    ] {
        assert!(
            responsibility::validate_contact(&ContactPoint {
                channel: Some(contact_point::Channel::Web(ContactUrl {
                    value: address.into()
                })),
                label: None
            })
            .is_err(),
            "{address:?}"
        );
    }
    let mut entity = authored();
    team_mut(&mut entity).contacts = vec![email.clone(), email];
    assert!(validate_values(&entity).is_err());
    team_mut(&mut entity).contacts[1].label = Some("work".into());
    validate_values(&entity).unwrap();
    team_mut(&mut entity).contacts[1].label = Some(" ".into());
    assert!(validate_values(&entity).is_err());
    let mut entity = authored();
    team_mut(&mut entity).title.as_mut().unwrap().value = "é".repeat(128);
    validate_values(&entity).unwrap();
    team_mut(&mut entity)
        .title
        .as_mut()
        .unwrap()
        .value
        .push('é');
    assert!(validate_values(&entity).is_err());
    team_mut(&mut entity).title = Some(DirectoryDisplayName::default());
    assert!(validate_values(&entity).is_err());
    let mut person = Entity {
        kind: Some(entity::Kind::Person(Person {
            id: Some(id("pat")),
            title: Some(DirectoryDisplayName {
                value: "Pat".into(),
            }),
            directory_pointer: Some(ExternalDirectoryPointer {
                provider: Some(DirectoryProvider {
                    value: "directory.example".into(),
                }),
                key: Some(OpaqueDirectoryKey {
                    value: "Exact Key".into(),
                }),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    validate_values(&person).unwrap();
    if let Some(entity::Kind::Person(p)) = &mut person.kind {
        p.role = Some(DirectoryRoleText::default());
    }
    assert!(validate_values(&person).is_err());
    if let Some(entity::Kind::Person(p)) = &mut person.kind {
        p.role = None;
        p.directory_pointer = Some(ExternalDirectoryPointer::default());
    }
    assert!(validate_values(&person).is_err());
}

#[test]
fn structural_limits_duplicates_and_self_targets_reject() {
    let mut entity = authored();
    let member = team(&entity).members[0].clone();
    team_mut(&mut entity).members.push(member);
    assert!(validate_values(&entity).is_err());
    let mut entity = authored();
    let responsibility = team(&entity).responsibilities[0].clone();
    team_mut(&mut entity).responsibilities.push(responsibility);
    assert!(validate_values(&entity).is_err());
    team_mut(&mut entity).responsibilities[1].kind = ResponsibilityKind::Contact as i32;
    validate_values(&entity).unwrap();
    for kind in [0, 123] {
        team_mut(&mut entity).responsibilities[1].kind = kind;
        assert!(validate_values(&entity).is_err());
    }
    let mut entity = authored();
    team_mut(&mut entity).responsibilities =
        vec![assignment(subject(EntityKind::Team, "fulfillment"))];
    assert!(validate_values(&entity).is_err());
    team_mut(&mut entity).responsibilities = vec![assignment(subject(EntityKind::Person, "pat"))];
    validate_values(&entity).unwrap();
    team_mut(&mut entity).doc = Some("x".repeat(responsibility::MAX_ENCODED_BYTES));
    assert!(validate_values(&entity).is_err());
    let mut entity = authored();
    team_mut(&mut entity).members = (0..=responsibility::MAX_MEMBERS)
        .map(|i| TeamMember {
            person: Some(PersonRef {
                id: Some(id(&format!("person-{i}"))),
            }),
            ..Default::default()
        })
        .collect();
    assert!(validate_values(&entity).is_err());
    team_mut(&mut entity).members.pop();
    validate_values(&entity).unwrap();
    team_mut(&mut entity).responsibilities = (0..=responsibility::MAX_RESPONSIBILITIES)
        .map(|i| assignment(subject(EntityKind::Event, &format!("event-{i}"))))
        .collect();
    assert!(validate_values(&entity).is_err());
    team_mut(&mut entity).responsibilities.pop();
    validate_values(&entity).unwrap();
    team_mut(&mut entity).contacts = (0..=responsibility::MAX_CONTACTS)
        .map(|i| ContactPoint {
            channel: Some(contact_point::Channel::Email(EmailAddress {
                value: format!("p{i}@example.test"),
            })),
            label: None,
        })
        .collect();
    assert!(validate_values(&entity).is_err());
    team_mut(&mut entity).contacts.pop();
    validate_values(&entity).unwrap();
}

#[test]
fn assignments_accept_all_exact_entity_kinds_and_no_unknown_kind() {
    for kind in canonical::ALL_KINDS {
        validate_subject(&subject(*kind, "target")).unwrap();
    }
    for kind in [0, 1000] {
        let mut value = subject(EntityKind::Event, "target");
        let Some(responsibility_subject::Selection::Entity(r)) = &mut value.selection else {
            unreachable!()
        };
        r.kind = kind;
        assert!(validate_subject(&value).is_err());
    }
    assert!(validate_subject(&ResponsibilitySubject::default()).is_err());
    let mut value = subject(EntityKind::Event, "target");
    let Some(responsibility_subject::Selection::Entity(r)) = &mut value.selection else {
        unreachable!()
    };
    r.id.as_mut().unwrap().version = 0;
    assert!(validate_subject(&value).is_err());
}

#[test]
fn identity_reapply_reorder_edit_add_remove_and_retarg_are_native() {
    let previous = stored();
    validate_stored_connection_ids(&previous).unwrap();
    assert_eq!(ids(&previous).len(), 2);
    assert!(ids(&previous).iter().all(Option::is_some));
    let reapplied = prepare_entity(Some(&previous), &authored()).unwrap();
    assert_eq!(reapplied, previous);
    let mut proposed = previous.clone();
    team_mut(&mut proposed).title.as_mut().unwrap().value = "A new label".into();
    team_mut(&mut proposed).members[0].doc = Some("Notes".into());
    team_mut(&mut proposed).responsibilities[0].doc = Some("Notes".into());
    assert_eq!(
        ids(&prepare_entity(Some(&previous), &proposed).unwrap()),
        ids(&previous)
    );
    team_mut(&mut proposed).members.push(TeamMember {
        person: Some(PersonRef {
            id: Some(id("sam")),
        }),
        ..Default::default()
    });
    let added = prepare_entity(Some(&previous), &proposed).unwrap();
    let mut proposed = added.clone();
    team_mut(&mut proposed).members.reverse();
    let reordered = prepare_entity(Some(&added), &proposed).unwrap();
    assert_eq!(
        team(&reordered).members[0].connection_id,
        team(&added).members[1].connection_id
    );
    let mut stripped = reordered.clone();
    team_mut(&mut stripped).members[0].connection_id = None;
    assert!(prepare_entity(Some(&reordered), &stripped).is_err());
    let mut supplied = previous.clone();
    team_mut(&mut supplied).members.push(TeamMember {
        person: Some(PersonRef {
            id: Some(id("sam")),
        }),
        connection_id: Some(ConnectionId::new().into()),
        ..Default::default()
    });
    assert!(prepare_entity(Some(&previous), &supplied).is_err());
    let mut changed = previous.clone();
    team_mut(&mut changed).responsibilities[0].kind = ResponsibilityKind::Contact as i32;
    assert!(prepare_entity(Some(&previous), &changed).is_err());
    team_mut(&mut changed).responsibilities[0].connection_id = None;
    let changed = prepare_entity(Some(&previous), &changed).unwrap();
    assert_ne!(
        team(&changed).responsibilities[0].connection_id,
        team(&previous).responsibilities[0].connection_id
    );
    let mut removed = previous.clone();
    team_mut(&mut removed).members.clear();
    let removed = prepare_entity(Some(&previous), &removed).unwrap();
    let mut readded = removed.clone();
    team_mut(&mut readded).members = team(&authored()).members.clone();
    let readded = prepare_entity(Some(&removed), &readded).unwrap();
    assert_ne!(
        team(&readded).members[0].connection_id,
        team(&previous).members[0].connection_id
    );
    let mut duplicate = previous.clone();
    team_mut(&mut duplicate).members[0].connection_id =
        team(&previous).responsibilities[0].connection_id.clone();
    assert!(validate_values(&duplicate).is_err());
}

#[test]
fn branch_lifecycle_and_clone_keep_identity_scopes() {
    let base = stored();
    let mut branch = base.clone();
    team_mut(&mut branch).doc = Some("branch documentation".into());
    let branch = prepare_entity(Some(&base), &branch).unwrap();
    let landed = prepare_landing(Some(&base), Some(&base), &branch).unwrap();
    assert_eq!(
        landed.system.as_ref().unwrap().uid,
        base.system.as_ref().unwrap().uid
    );
    assert_eq!(ids(&landed), ids(&base));
    let mut opposing = base.clone();
    team_mut(&mut opposing).members.clear();
    let opposing = prepare_entity(Some(&base), &opposing).unwrap();
    assert!(prepare_landing(Some(&base), Some(&opposing), &branch).is_err());
    let resolved = prepare_resolution(Some(&base), Some(&opposing), &branch).unwrap();
    assert_ne!(
        team(&resolved).members[0].connection_id,
        team(&base).members[0].connection_id
    );
    assert_eq!(
        team(&resolved).responsibilities[0].connection_id,
        team(&base).responsibilities[0].connection_id
    );
    let mut clone = base.clone();
    team_mut(&mut clone).id.as_mut().unwrap().version = 2;
    let clone = prepare_entity(None, &clone).unwrap();
    assert_ne!(
        clone.system.as_ref().unwrap().uid,
        base.system.as_ref().unwrap().uid
    );
    assert_eq!(ids(&clone), ids(&base));
    assert_eq!(team(&clone).responsibilities, team(&base).responsibilities);
}

#[test]
fn connections_resolve_exact_occurrences_validate_all_ids_and_exclude_directory_associations() {
    let owner = behavioral_owner();
    let selection = selected_connection(&owner, 0);
    resolve_subject(&selection, &owner).unwrap();
    let mut reordered = owner.clone();
    if let Some(entity::Kind::AutomationSlice(v)) = &mut reordered.kind {
        v.source_read_models.reverse();
    }
    resolve_subject(&selection, &reordered).unwrap();
    if let Some(entity::Kind::AutomationSlice(v)) = &mut reordered.kind {
        v.source_read_models[0].connection_id = v.source_read_models[1].connection_id.clone();
    }
    assert!(resolve_subject(&selection, &reordered).is_err());
    let mut removed = owner.clone();
    if let Some(entity::Kind::AutomationSlice(v)) = &mut removed.kind {
        v.source_read_models.remove(0);
    }
    assert!(resolve_subject(&selection, &removed).is_err());
    let mut malformed = owner.clone();
    if let Some(entity::Kind::AutomationSlice(v)) = &mut malformed.kind {
        v.source_read_models[1].connection_id = Some(trogon_atlas_proto::ConnectionId {
            value: "broken".into(),
        });
    }
    assert!(resolve_subject(&selection, &malformed).is_err());
    let mut missing = owner.clone();
    if let Some(entity::Kind::AutomationSlice(v)) = &mut missing.kind {
        v.source_read_models[1].connection_id = None;
    }
    assert!(resolve_subject(&selection, &missing).is_err());
    let recreated = prepare_entity(None, &owner).unwrap();
    assert!(resolve_subject(&selection, &recreated).is_err());
    let directory = stored();
    assert!(validate_subject(&selected_connection(&directory, 0)).is_err());
    assert!(validate_subject(&selected_connection(&directory, 1)).is_err());
    for kind in canonical::ALL_KINDS {
        let selection = ResponsibilitySubject {
            selection: Some(responsibility_subject::Selection::Connection(
                ConnectionRef {
                    owner: Some(entity_ref(*kind, "target")),
                    owner_uid: uuid::Uuid::now_v7().to_string(),
                    id: Some(ConnectionId::new().into()),
                },
            )),
        };
        assert_eq!(
            validate_subject(&selection).is_ok(),
            responsibility::connection_subject_owner(*kind)
        );
    }
}

#[test]
fn traversal_includes_local_owners_and_generic_retarget_excludes_them() {
    let mut entity = authored();
    let behavior = behavioral_owner();
    team_mut(&mut entity)
        .responsibilities
        .push(assignment(selected_connection(&behavior, 0)));
    team_mut(&mut entity)
        .responsibilities
        .push(assignment(ResponsibilitySubject {
            selection: Some(responsibility_subject::Selection::Member(
                ContractMemberRef {
                    specification: Some(SpecificationRef {
                        id: Some(id("api")),
                    }),
                    owner_uid: uuid::Uuid::now_v7().to_string(),
                    kind: ContractMemberKind::Operation as i32,
                    member_id: Some(trogon_atlas_core::specification::MemberId::new().into()),
                    expected_digest: None,
                },
            )),
        }));
    team_mut(&mut entity)
        .responsibilities
        .push(assignment(ResponsibilitySubject {
            selection: Some(responsibility_subject::Selection::Port(DataPortRef {
                product: Some(DataProductRef {
                    id: Some(id("data")),
                }),
                owner_uid: uuid::Uuid::now_v7().to_string(),
                direction: DataPortDirection::Output as i32,
                port_id: Some(trogon_atlas_core::data_product::DataPortId::new().into()),
            })),
        }));
    let previous = prepare_entity(None, &entity).unwrap();
    validate_values(&previous).unwrap();
    let refs = outbound_refs(&previous);
    assert_eq!(refs.len(), 5);
    for expected in [
        "members[0].person",
        "responsibilities[0].subject.entity",
        "responsibilities[1].subject.connection.owner",
        "responsibilities[2].subject.member.specification",
        "responsibilities[3].subject.port.product",
    ] {
        assert!(refs.iter().any(|r| r.from_field == expected));
    }
    let mut proposed = previous.clone();
    for (kind, slug) in [
        (EntityKind::AutomationSlice, "react"),
        (EntityKind::Specification, "api"),
        (EntityKind::DataProduct, "data"),
    ] {
        assert_eq!(
            retarget_refs(&mut proposed, kind, &id(slug), &id("replacement")),
            [] as [String; 0]
        );
    }
    assert_eq!(proposed, previous);
    assert_eq!(
        retarget_refs(&mut proposed, EntityKind::Person, &id("pat"), &id("sam")).len(),
        1
    );
    assert_eq!(
        retarget_refs(
            &mut proposed,
            EntityKind::Event,
            &id("order-placed"),
            &id("order-confirmed")
        )
        .len(),
        1
    );
    assert!(prepare_entity(Some(&previous), &proposed).is_err());
    assert_eq!(
        renew_retargeted_connection_ids(&previous, &mut proposed).unwrap(),
        2
    );
    let updated = prepare_entity(Some(&previous), &proposed).unwrap();
    assert_ne!(
        team(&updated).members[0].connection_id,
        team(&previous).members[0].connection_id
    );
    assert_ne!(
        team(&updated).responsibilities[0].connection_id,
        team(&previous).responsibilities[0].connection_id
    );
    assert_eq!(
        team(&updated).responsibilities[1..],
        team(&previous).responsibilities[1..]
    );
}

fn specification_owner() -> Entity {
    prepare_entity(
        None,
        &Entity {
            kind: Some(entity::Kind::Specification(Specification {
                id: Some(id("http-api")),
                artifacts: vec![ContractArtifact {
                    content: "retained bytes".into(),
                    digest: Some(trogon_atlas_core::specification::content_digest(
                        "retained bytes",
                    )),
                    ..Default::default()
                }],
                schemas: vec![ContractSchema::default()],
                messages: vec![ContractMessage::default()],
                operations: vec![ContractOperation::default()],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .unwrap()
}
fn member_selection(owner: &Entity, kind: ContractMemberKind) -> ResponsibilitySubject {
    let Some(entity::Kind::Specification(value)) = &owner.kind else {
        panic!("expected Specification")
    };
    let member_id = match kind {
        ContractMemberKind::Artifact => value.artifacts[0].member_id.clone(),
        ContractMemberKind::Schema => value.schemas[0].member_id.clone(),
        ContractMemberKind::Message => value.messages[0].member_id.clone(),
        ContractMemberKind::Operation => value.operations[0].member_id.clone(),
        ContractMemberKind::Unspecified => None,
    };
    ResponsibilitySubject {
        selection: Some(responsibility_subject::Selection::Member(
            ContractMemberRef {
                specification: Some(SpecificationRef {
                    id: entity_id(owner).cloned(),
                }),
                owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                kind: kind as i32,
                member_id,
                expected_digest: None,
            },
        )),
    }
}

#[test]
fn members_validate_complete_local_identity_and_optional_artifact_digest() {
    let owner = specification_owner();
    for kind in [
        ContractMemberKind::Artifact,
        ContractMemberKind::Schema,
        ContractMemberKind::Message,
        ContractMemberKind::Operation,
    ] {
        let selection = member_selection(&owner, kind);
        resolve_subject(&selection, &owner).unwrap();
        let recreated = prepare_entity(None, &owner).unwrap();
        assert!(resolve_subject(&selection, &recreated).is_err());
    }
    let selection = member_selection(&owner, ContractMemberKind::Operation);
    let mut corrupt = owner.clone();
    if let Some(entity::Kind::Specification(v)) = &mut corrupt.kind {
        v.schemas[0].member_id = None;
    }
    assert!(resolve_subject(&selection, &corrupt).is_err());
    if let Some(entity::Kind::Specification(v)) = &mut corrupt.kind {
        v.schemas[0].member_id = v.operations[0].member_id.clone();
    }
    assert!(resolve_subject(&selection, &corrupt).is_err());
    if let Some(entity::Kind::Specification(v)) = &mut corrupt.kind {
        v.schemas[0].member_id = Some(ContractMemberId {
            value: "malformed".into(),
        });
    }
    assert!(resolve_subject(&selection, &corrupt).is_err());
    let mut removed = owner.clone();
    if let Some(entity::Kind::Specification(v)) = &mut removed.kind {
        v.operations.clear();
    }
    assert!(resolve_subject(&selection, &removed).is_err());
    let mut pinned = member_selection(&owner, ContractMemberKind::Artifact);
    if let Some(responsibility_subject::Selection::Member(r)) = &mut pinned.selection {
        r.expected_digest = Some(trogon_atlas_core::specification::content_digest(
            "retained bytes",
        ));
    }
    resolve_subject(&pinned, &owner).unwrap();
    let mut different_bytes = owner.clone();
    if let Some(entity::Kind::Specification(v)) = &mut different_bytes.kind {
        v.artifacts[0].content = "changed bytes".into();
    }
    assert!(resolve_subject(&pinned, &different_bytes).is_err());
    if let Some(responsibility_subject::Selection::Member(r)) = &mut pinned.selection {
        r.kind = ContractMemberKind::Schema as i32;
    }
    assert!(validate_subject(&pinned).is_err());
    if let Some(responsibility_subject::Selection::Member(r)) = &mut pinned.selection {
        r.kind = ContractMemberKind::Artifact as i32;
        r.expected_digest = Some(ContentDigest {
            sha256: "malformed".into(),
        });
    }
    assert!(validate_subject(&pinned).is_err());
    let mut entity = authored();
    team_mut(&mut entity).responsibilities = vec![assignment(member_selection(
        &owner,
        ContractMemberKind::Artifact,
    ))];
    let previous = prepare_entity(None, &entity).unwrap();
    let mut next = previous.clone();
    if let Some(responsibility_subject::Selection::Member(r)) = team_mut(&mut next).responsibilities
        [0]
    .subject
    .as_mut()
    .and_then(|s| s.selection.as_mut())
    {
        r.expected_digest = Some(trogon_atlas_core::specification::content_digest(
            "retained bytes",
        ));
    }
    assert!(prepare_entity(Some(&previous), &next).is_err());
    team_mut(&mut next).responsibilities[0].connection_id = None;
    let next = prepare_entity(Some(&previous), &next).unwrap();
    assert_ne!(
        team(&next).responsibilities[0].connection_id,
        team(&previous).responsibilities[0].connection_id
    );
}

fn port_owner() -> Entity {
    prepare_entity(
        None,
        &Entity {
            kind: Some(entity::Kind::DataProduct(DataProduct {
                id: Some(id("orders")),
                inputs: vec![DataInputPort {
                    name: Some(PortName { value: "In".into() }),
                    ..Default::default()
                }],
                outputs: vec![DataOutputPort {
                    name: Some(PortName {
                        value: "Out".into(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .unwrap()
}
fn port_selection(owner: &Entity, direction: DataPortDirection) -> ResponsibilitySubject {
    let Some(entity::Kind::DataProduct(value)) = &owner.kind else {
        panic!("expected product")
    };
    ResponsibilitySubject {
        selection: Some(responsibility_subject::Selection::Port(DataPortRef {
            product: Some(DataProductRef {
                id: entity_id(owner).cloned(),
            }),
            owner_uid: owner.system.as_ref().unwrap().uid.clone(),
            direction: direction as i32,
            port_id: if direction == DataPortDirection::Input {
                value.inputs[0].port_id.clone()
            } else {
                value.outputs[0].port_id.clone()
            },
        })),
    }
}

#[test]
fn ports_validate_all_identities_direction_and_incarnation_without_data_proof() {
    let owner = port_owner();
    for direction in [DataPortDirection::Input, DataPortDirection::Output] {
        let selection = port_selection(&owner, direction);
        resolve_subject(&selection, &owner).unwrap();
        let recreated = prepare_entity(None, &owner).unwrap();
        assert!(resolve_subject(&selection, &recreated).is_err());
    }
    let selection = port_selection(&owner, DataPortDirection::Output);
    let mut modified = owner.clone();
    if let Some(entity::Kind::DataProduct(v)) = &mut modified.kind {
        v.outputs[0].name = Some(PortName {
            value: "New name".into(),
        });
    }
    resolve_subject(&selection, &modified).unwrap();
    if let Some(entity::Kind::DataProduct(v)) = &mut modified.kind {
        v.inputs[0].port_id = None;
    }
    assert!(resolve_subject(&selection, &modified).is_err());
    if let Some(entity::Kind::DataProduct(v)) = &mut modified.kind {
        v.inputs[0].port_id = v.outputs[0].port_id.clone();
    }
    assert!(resolve_subject(&selection, &modified).is_err());
    if let Some(entity::Kind::DataProduct(v)) = &mut modified.kind {
        v.inputs[0].port_id = Some(DataPortId {
            value: "malformed".into(),
        });
    }
    assert!(resolve_subject(&selection, &modified).is_err());
    let mut removed = owner.clone();
    if let Some(entity::Kind::DataProduct(v)) = &mut removed.kind {
        v.outputs.clear();
    }
    assert!(resolve_subject(&selection, &removed).is_err());
    let mut wrong = selection;
    if let Some(responsibility_subject::Selection::Port(r)) = &mut wrong.selection {
        r.direction = DataPortDirection::Input as i32;
    }
    assert!(resolve_subject(&wrong, &owner).is_err());
    if let Some(responsibility_subject::Selection::Port(r)) = &mut wrong.selection {
        r.direction = 0;
    }
    assert!(validate_subject(&wrong).is_err());
}

#[test]
fn typed_subject_fingerprint_changes_require_new_assignment_identity() {
    let mut entity = authored();
    team_mut(&mut entity).responsibilities = vec![assignment(port_selection(
        &port_owner(),
        DataPortDirection::Output,
    ))];
    let previous = prepare_entity(None, &entity).unwrap();
    let mut alternatives = Vec::new();
    let mut selection = team(&previous).responsibilities[0].subject.clone().unwrap();
    if let Some(responsibility_subject::Selection::Port(r)) = &mut selection.selection {
        r.owner_uid = uuid::Uuid::now_v7().to_string();
    }
    alternatives.push(selection);
    let mut selection = team(&previous).responsibilities[0].subject.clone().unwrap();
    if let Some(responsibility_subject::Selection::Port(r)) = &mut selection.selection {
        r.port_id = Some(trogon_atlas_core::data_product::DataPortId::new().into());
    }
    alternatives.push(selection);
    let mut selection = team(&previous).responsibilities[0].subject.clone().unwrap();
    if let Some(responsibility_subject::Selection::Port(r)) = &mut selection.selection {
        r.direction = DataPortDirection::Input as i32;
    }
    alternatives.push(selection);
    let mut selection = team(&previous).responsibilities[0].subject.clone().unwrap();
    if let Some(responsibility_subject::Selection::Port(r)) = &mut selection.selection {
        r.product.as_mut().unwrap().id.as_mut().unwrap().version = 2;
    }
    alternatives.push(selection);
    alternatives.push(subject(EntityKind::DataProduct, "orders"));
    for alternative in alternatives {
        let mut next = previous.clone();
        team_mut(&mut next).responsibilities[0].subject = Some(alternative);
        assert!(prepare_entity(Some(&previous), &next).is_err());
        team_mut(&mut next).responsibilities[0].connection_id = None;
        let next = prepare_entity(Some(&previous), &next).unwrap();
        assert_ne!(
            team(&next).responsibilities[0].connection_id,
            team(&previous).responsibilities[0].connection_id
        );
    }
}

#[test]
fn directory_identity_errors_do_not_mutate_proposed_or_prior_values() {
    let previous = stored();
    for malformed in ["", "00000000-0000-0000-0000-000000000000", "not-an-id"] {
        let mut proposed = previous.clone();
        team_mut(&mut proposed).members[0].connection_id = Some(trogon_atlas_proto::ConnectionId {
            value: malformed.into(),
        });
        let before = proposed.clone();
        assert!(prepare_entity(Some(&previous), &proposed).is_err());
        assert_eq!(proposed, before);
        validate_stored_connection_ids(&previous).unwrap();
    }
    let mut new_owner = authored();
    let supplied: trogon_atlas_proto::ConnectionId = ConnectionId::new().into();
    team_mut(&mut new_owner).members[0].connection_id = Some(supplied.clone());
    let new_owner = prepare_entity(None, &new_owner).unwrap();
    assert_eq!(team(&new_owner).members[0].connection_id, Some(supplied));
    let mut proposed = previous.clone();
    team_mut(&mut proposed).members[0].person = Some(PersonRef {
        id: Some(id("sam")),
    });
    assert!(prepare_entity(Some(&previous), &proposed).is_err());
    team_mut(&mut proposed).members[0].connection_id = None;
    let updated = prepare_entity(Some(&previous), &proposed).unwrap();
    assert_ne!(
        team(&updated).members[0].connection_id,
        team(&previous).members[0].connection_id
    );
}

#[test]
fn plain_entity_subject_follows_same_exact_key_recreation_only() {
    let owner = stored();
    let selection = subject(EntityKind::Team, "fulfillment");
    resolve_subject(&selection, &owner).unwrap();
    let recreated = prepare_entity(None, &owner).unwrap();
    resolve_subject(&selection, &recreated).unwrap();
    let mut another_version = recreated.clone();
    team_mut(&mut another_version).id.as_mut().unwrap().version = 2;
    assert!(resolve_subject(&selection, &another_version).is_err());
    let mut person = Entity {
        kind: Some(entity::Kind::Person(Person {
            id: Some(id("pat")),
            title: Some(DirectoryDisplayName {
                value: "Pat".into(),
            }),
            responsibilities: vec![assignment(selection)],
            ..Default::default()
        })),
        ..Default::default()
    };
    validate_values(&person).unwrap();
    let mut team_owner = authored();
    team_mut(&mut team_owner).responsibilities =
        vec![assignment(subject(EntityKind::Person, "pat"))];
    validate_values(&team_owner).unwrap();
    if let Some(entity::Kind::Person(value)) = &mut person.kind {
        value.responsibilities = vec![assignment(subject(EntityKind::Person, "pat"))];
    }
    assert!(validate_values(&person).is_err());
}

#[test]
fn directory_value_byte_boundaries_and_provider_grammar_are_enforced() {
    let email = |value: String| ContactPoint {
        channel: Some(contact_point::Channel::Email(EmailAddress { value })),
        label: None,
    };
    responsibility::validate_contact(&email(format!("{}@b", "a".repeat(252)))).unwrap();
    assert!(responsibility::validate_contact(&email(format!("{}@b", "a".repeat(253)))).is_err());
    let web = |value: String| ContactPoint {
        channel: Some(contact_point::Channel::Web(ContactUrl { value })),
        label: None,
    };
    let prefix = "https://example.test/";
    responsibility::validate_contact(&web(format!("{prefix}{}", "a".repeat(2048 - prefix.len()))))
        .unwrap();
    assert!(responsibility::validate_contact(&web(format!(
        "{prefix}{}",
        "a".repeat(2049 - prefix.len())
    )))
    .is_err());
    let chat = |service: String, handle: String| ContactPoint {
        channel: Some(contact_point::Channel::Chat(ChatContact {
            service: Some(DirectoryProvider { value: service }),
            handle: Some(OpaqueDirectoryKey { value: handle }),
        })),
        label: None,
    };
    responsibility::validate_contact(&chat("a".repeat(64), "é".repeat(256))).unwrap();
    for provider in [
        "",
        "UPPER",
        "9start",
        "under_score",
        "white space",
        &"a".repeat(65),
    ] {
        assert!(responsibility::validate_contact(&chat(provider.into(), "key".into())).is_err());
    }
    for key in [
        String::new(),
        " \t".into(),
        "control\n".into(),
        "é".repeat(257),
    ] {
        assert!(responsibility::validate_contact(&chat("chat".into(), key)).is_err());
    }
    let mut contact = email("a@b".into());
    contact.label = Some("é".repeat(64));
    responsibility::validate_contact(&contact).unwrap();
    contact.label = Some("é".repeat(65));
    assert!(responsibility::validate_contact(&contact).is_err());
    let mut person = Entity {
        kind: Some(entity::Kind::Person(Person {
            id: Some(id("pat")),
            title: Some(DirectoryDisplayName {
                value: "Pat".into(),
            }),
            role: Some(DirectoryRoleText {
                value: "é".repeat(256),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    validate_values(&person).unwrap();
    if let Some(entity::Kind::Person(value)) = &mut person.kind {
        value.role.as_mut().unwrap().value.push('a');
    }
    assert!(validate_values(&person).is_err());
}

#[test]
fn generic_references_preserve_full_typed_assignment_and_membership_provenance() {
    use prost::Message;

    let directory = stored();
    let owner_uid = directory.system.as_ref().unwrap().uid.clone();
    let from = Some(entity_ref(EntityKind::Team, "fulfillment"));
    let legacy = Reference {
        from: from.clone(),
        from_field: "supersedes".into(),
        to: Some(entity_ref(EntityKind::Team, "prior-version")),
        ..Default::default()
    };
    let decoded = Reference::decode(legacy.encode_to_vec().as_slice()).unwrap();
    assert_eq!(decoded, legacy);
    assert_eq!(decoded.responsibility, None);
    assert_eq!(decoded.membership, None);
    assert_eq!(decoded.from_uid, "");
    let member_owner = specification_owner();
    let mut member = member_selection(&member_owner, ContractMemberKind::Artifact);
    if let Some(responsibility_subject::Selection::Member(r)) = &mut member.selection {
        r.expected_digest = Some(trogon_atlas_core::specification::content_digest(
            "retained bytes",
        ));
    }
    for subject in [
        subject(EntityKind::Event, "placed"),
        selected_connection(&behavioral_owner(), 1),
        member,
        port_selection(&port_owner(), DataPortDirection::Output),
    ] {
        let selected_owner = responsibility::subject_owner(&subject).unwrap();
        let assignment = Responsibility {
            connection_id: Some(ConnectionId::new().into()),
            kind: ResponsibilityKind::Steward as i32,
            subject: Some(subject),
            doc: Some("Catalog stewardship".into()),
        };
        let reference = Reference {
            catalog_subject: None,
            document_selection: None,
            connection_id: None,
            from: from.clone(),
            from_field: "responsibilities[0].subject".into(),
            to: Some(selected_owner),
            responsibility: Some(assignment),
            membership: None,
            from_uid: owner_uid.clone(),
        };
        assert_eq!(
            Reference::decode(reference.encode_to_vec().as_slice()).unwrap(),
            reference
        );
        let impact = ImpactNode {
            entity: reference.to.clone(),
            depth: 1,
            path: vec![reference],
        };
        assert_eq!(
            ImpactNode::decode(impact.encode_to_vec().as_slice()).unwrap(),
            impact
        );
    }
    let reference = Reference {
        catalog_subject: None,
        document_selection: None,
        connection_id: None,
        from,
        from_field: "members[0].person".into(),
        to: Some(entity_ref(EntityKind::Person, "pat")),
        membership: Some(team(&directory).members[0].clone()),
        from_uid: owner_uid,
        ..Default::default()
    };
    assert_eq!(
        Reference::decode(reference.encode_to_vec().as_slice()).unwrap(),
        reference
    );
}
