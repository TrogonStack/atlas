#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_core::{
    connection_persistence::{prepare_entity, prepare_landing, prepare_resolution},
    connections::{self, connection_occurrences, ConnectionError, ConnectionId},
    decision_document, entity_id, entity_kind, outbound_refs, responsibility, retarget_refs,
    system_group::{self, validate_values},
};
use trogon_atlas_proto::{self as pb, *};

const BEYOND_JS: u64 = (1 << 53) + 7;

fn id(slug: &str) -> Id {
    Id {
        namespace: "commerce".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn versioned(slug: &str, version: u64) -> Id {
    Id {
        version,
        ..id(slug)
    }
}
fn title(value: &str) -> CatalogTitle {
    CatalogTitle {
        value: value.into(),
    }
}
fn text(value: &str) -> CatalogText {
    CatalogText {
        value: value.into(),
    }
}

fn member(target: system_group_member::Target) -> SystemGroupMember {
    SystemGroupMember {
        target: Some(target),
    }
}
fn component(slug: &str) -> system_group_member::Target {
    system_group_member::Target::Component(ComponentRef { id: Some(id(slug)) })
}
fn group_ref(slug: &str) -> system_group_member::Target {
    system_group_member::Target::SystemGroup(SystemGroupRef { id: Some(id(slug)) })
}
fn membership(target: system_group_member::Target) -> SystemGroupMembership {
    SystemGroupMembership {
        member: Some(member(target)),
        doc: Some(text("curated")),
        ..Default::default()
    }
}
fn relationship(slug: &str, label: Option<&str>) -> SystemGroupRelationship {
    SystemGroupRelationship {
        target: Some(SystemGroupRef { id: Some(id(slug)) }),
        label: label.map(title),
        ..Default::default()
    }
}
fn actor(target: system_actor_target::Target, direction: SystemActorDirection) -> SystemActorLink {
    SystemActorLink {
        target: Some(SystemActorTarget {
            target: Some(target),
        }),
        direction: direction as i32,
        label: Some(title("uses")),
        ..Default::default()
    }
}
fn persona(slug: &str) -> system_actor_target::Target {
    system_actor_target::Target::Persona(PersonaRef { id: Some(id(slug)) })
}

fn platform() -> Entity {
    Entity {
        kind: Some(entity::Kind::SystemGroup(SystemGroup {
            id: Some(id("platform")),
            title: "Platform".into(),
            scope: SystemGroupScope::Internal as i32,
            members: vec![
                membership(component("checkout")),
                membership(component("checkout")),
                membership(system_group_member::Target::Component(ComponentRef {
                    id: Some(versioned("checkout", 2)),
                })),
                membership(system_group_member::Target::DataStore(DataStoreRef {
                    id: Some(id("orders-db")),
                })),
                membership(group_ref("storefront")),
                membership(system_group_member::Target::ExternalSystem(
                    ExternalSystemRef {
                        id: Some(id("provider")),
                    },
                )),
                membership(system_group_member::Target::Schema(SchemaRef {
                    id: Some(id("order")),
                })),
            ],
            relationships: vec![
                relationship("payments-vendor", Some("pays through")),
                relationship("payments-vendor", None),
            ],
            actors: vec![
                actor(persona("shopper"), SystemActorDirection::Inbound),
                actor(persona("shopper"), SystemActorDirection::Outbound),
                actor(
                    system_actor_target::Target::Component(ComponentRef {
                        id: Some(id("worker")),
                    }),
                    SystemActorDirection::Inbound,
                ),
                actor(
                    system_actor_target::Target::ExternalSystem(ExternalSystemRef {
                        id: Some(id("provider")),
                    }),
                    SystemActorDirection::Outbound,
                ),
                actor(
                    system_actor_target::Target::SystemGroup(SystemGroupRef {
                        id: Some(id("storefront")),
                    }),
                    SystemActorDirection::Inbound,
                ),
            ],
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn group_with(
    slug: &str,
    members: Vec<SystemGroupMembership>,
    relationships: Vec<SystemGroupRelationship>,
) -> Entity {
    Entity {
        kind: Some(entity::Kind::SystemGroup(SystemGroup {
            id: Some(id(slug)),
            scope: SystemGroupScope::Internal as i32,
            members,
            relationships,
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn domain_member(target: domain_member::Target) -> DomainMembership {
    DomainMembership {
        member: Some(DomainMember {
            target: Some(target),
        }),
        ..Default::default()
    }
}
fn domain_with(slug: &str, members: Vec<DomainMembership>) -> Entity {
    Entity {
        kind: Some(entity::Kind::Domain(Domain {
            id: Some(id(slug)),
            title: "Retail".into(),
            members,
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn retail() -> Entity {
    domain_with(
        "retail",
        vec![
            domain_member(domain_member::Target::SystemGroup(SystemGroupRef {
                id: Some(id("platform")),
            })),
            domain_member(domain_member::Target::Component(ComponentRef {
                id: Some(id("worker")),
            })),
            domain_member(domain_member::Target::Domain(DomainRef {
                id: Some(id("fulfilment")),
            })),
            domain_member(domain_member::Target::DataProduct(DataProductRef {
                id: Some(id("receipts")),
            })),
            domain_member(domain_member::Target::Schema(SchemaRef {
                id: Some(id("order")),
            })),
            domain_member(domain_member::Target::ExternalSystem(ExternalSystemRef {
                id: Some(id("provider")),
            })),
        ],
    )
}
fn group_mut(entity: &mut Entity) -> &mut SystemGroup {
    match entity.kind.as_mut().unwrap() {
        entity::Kind::SystemGroup(value) => value,
        _ => panic!("system group"),
    }
}
fn domain_mut(entity: &mut Entity) -> &mut Domain {
    match entity.kind.as_mut().unwrap() {
        entity::Kind::Domain(value) => value,
        _ => panic!("domain"),
    }
}
fn selector(owner: &Entity, id: Option<pb::ConnectionId>) -> ConnectionRef {
    ConnectionRef {
        owner: Some(EntityRef {
            kind: entity_kind(owner).unwrap() as i32,
            id: entity_id(owner).cloned(),
        }),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id,
    }
}
fn member_selector(owner: &Entity, index: usize) -> ConnectionRef {
    let id = match owner.kind.as_ref().unwrap() {
        entity::Kind::SystemGroup(value) => value.members[index].connection_id.clone(),
        entity::Kind::Domain(value) => value.members[index].connection_id.clone(),
        _ => panic!("owner"),
    };
    selector(owner, id)
}

#[test]
fn entities_round_trip_and_every_occurrence_has_a_distinct_identity() {
    for owner in [platform(), retail()] {
        let stored = prepare_entity(None, &owner).unwrap();
        validate_values(&stored).unwrap();
        assert_eq!(
            Entity::decode(stored.encode_to_vec().as_slice()).unwrap(),
            stored
        );
        let ids: Vec<_> = connection_occurrences(&stored)
            .unwrap()
            .into_iter()
            .map(|item| item.id.unwrap())
            .collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len());
    }
    let stored = prepare_entity(None, &platform()).unwrap();
    let occurrences = connection_occurrences(&stored).unwrap();
    assert_eq!(occurrences.len(), 7 + 2 + 5);
    assert_ne!(occurrences[0].id, occurrences[1].id);
    assert_eq!(occurrences[0].target, occurrences[1].target);
    assert_ne!(occurrences[0].target, occurrences[2].target);
}

#[test]
fn typed_values_reject_without_coercion() {
    validate_values(&platform()).unwrap();
    validate_values(&retail()).unwrap();
    for scope in [SystemGroupScope::Unspecified as i32, 99] {
        let mut bad = platform();
        group_mut(&mut bad).scope = scope;
        assert!(validate_values(&bad).is_err());
    }
    for direction in [SystemActorDirection::Unspecified as i32, 99] {
        let mut bad = platform();
        group_mut(&mut bad).actors[0].direction = direction;
        assert!(validate_values(&bad).is_err());
    }
    let invalid_ids = [
        None,
        Some(versioned("target", 0)),
        Some(Id {
            namespace: "../bad".into(),
            ..id("target")
        }),
        Some(Id {
            slug: String::new(),
            ..id("target")
        }),
    ];
    for invalid in invalid_ids {
        let mut bad = platform();
        group_mut(&mut bad).members[0].member = Some(member(
            system_group_member::Target::Component(ComponentRef {
                id: invalid.clone(),
            }),
        ));
        assert!(validate_values(&bad).is_err());
        let mut bad = platform();
        group_mut(&mut bad).relationships[0].target = Some(SystemGroupRef {
            id: invalid.clone(),
        });
        assert!(validate_values(&bad).is_err());
        let mut bad = platform();
        group_mut(&mut bad).actors[0].target = Some(SystemActorTarget {
            target: Some(system_actor_target::Target::Persona(PersonaRef {
                id: invalid.clone(),
            })),
        });
        assert!(validate_values(&bad).is_err());
        let mut bad = retail();
        domain_mut(&mut bad).members[0].member = Some(DomainMember {
            target: Some(domain_member::Target::Domain(DomainRef { id: invalid })),
        });
        assert!(validate_values(&bad).is_err());
    }
    let mut bad = platform();
    group_mut(&mut bad).members[0].member = None;
    assert!(validate_values(&bad).is_err());
    group_mut(&mut bad).members[0].member = Some(SystemGroupMember::default());
    assert!(validate_values(&bad).is_err());
    let mut bad = platform();
    group_mut(&mut bad).relationships[0].target = None;
    assert!(validate_values(&bad).is_err());
    let mut bad = platform();
    group_mut(&mut bad).actors[0].target = Some(SystemActorTarget::default());
    assert!(validate_values(&bad).is_err());
    let mut bad = retail();
    domain_mut(&mut bad).members[0].member = Some(DomainMember::default());
    assert!(validate_values(&bad).is_err());
}

#[test]
fn versions_beyond_javascript_precision_are_exact_members() {
    let mut owner = platform();
    group_mut(&mut owner).members[0].member = Some(member(system_group_member::Target::Component(
        ComponentRef {
            id: Some(versioned("checkout", BEYOND_JS)),
        },
    )));
    let stored = prepare_entity(None, &owner).unwrap();
    validate_values(&stored).unwrap();
    let decoded = Entity::decode(stored.encode_to_vec().as_slice()).unwrap();
    let target = connection_occurrences(&decoded).unwrap()[0].target.clone();
    assert_eq!(target.unwrap().id.unwrap().version, BEYOND_JS);
}

#[test]
fn self_membership_and_self_relationship_reject_but_other_versions_and_kinds_do_not() {
    let mut owner = platform();
    group_mut(&mut owner)
        .members
        .push(membership(group_ref("platform")));
    assert!(validate_values(&owner).is_err());
    let mut owner = platform();
    group_mut(&mut owner)
        .relationships
        .push(relationship("platform", None));
    assert!(validate_values(&owner).is_err());
    let mut owner = platform();
    group_mut(&mut owner)
        .members
        .push(membership(system_group_member::Target::SystemGroup(
            SystemGroupRef {
                id: Some(versioned("platform", 2)),
            },
        )));
    group_mut(&mut owner)
        .members
        .push(membership(component("platform")));
    group_mut(&mut owner)
        .relationships
        .push(SystemGroupRelationship {
            target: Some(SystemGroupRef {
                id: Some(versioned("platform", 2)),
            }),
            ..Default::default()
        });
    validate_values(&owner).unwrap();
    let mut domain = retail();
    domain_mut(&mut domain)
        .members
        .push(domain_member(domain_member::Target::Domain(DomainRef {
            id: Some(id("retail")),
        })));
    assert!(validate_values(&domain).is_err());
    let mut domain = retail();
    domain_mut(&mut domain)
        .members
        .push(domain_member(domain_member::Target::Domain(DomainRef {
            id: Some(versioned("retail", 2)),
        })));
    domain_mut(&mut domain)
        .members
        .push(domain_member(domain_member::Target::SystemGroup(
            SystemGroupRef {
                id: Some(id("retail")),
            },
        )));
    validate_values(&domain).unwrap();
    let mut actor_self = platform();
    group_mut(&mut actor_self).actors.push(actor(
        system_actor_target::Target::SystemGroup(SystemGroupRef {
            id: Some(id("platform")),
        }),
        SystemActorDirection::Inbound,
    ));
    validate_values(&actor_self).unwrap();
}

#[test]
fn wording_bounds_and_optional_presence_are_enforced() {
    let mut owner = platform();
    assert!(group_mut(&mut owner).relationships[1].label.is_none());
    validate_values(&owner).unwrap();
    for blank in [String::new(), " \n".into(), "é".repeat(129)] {
        let mut bad = platform();
        group_mut(&mut bad).relationships[0].label = Some(CatalogTitle {
            value: blank.clone(),
        });
        assert!(validate_values(&bad).is_err());
        let mut bad = platform();
        group_mut(&mut bad).actors[0].label = Some(CatalogTitle { value: blank });
        assert!(validate_values(&bad).is_err());
    }
    group_mut(&mut owner).actors[0].label = None;
    validate_values(&owner).unwrap();
    for blank in [String::new(), " \n".into(), "é".repeat(8193)] {
        let mut bad = platform();
        group_mut(&mut bad).members[0].doc = Some(CatalogText {
            value: blank.clone(),
        });
        assert!(validate_values(&bad).is_err());
        let mut bad = retail();
        domain_mut(&mut bad).members[0].doc = Some(CatalogText { value: blank });
        assert!(validate_values(&bad).is_err());
    }
}

#[test]
fn collection_and_entity_ceilings_reject_without_truncation() {
    let mut owner = platform();
    group_mut(&mut owner).members = (0..1_024)
        .map(|_| membership(component("checkout")))
        .collect();
    group_mut(&mut owner).relationships = (0..1_024)
        .map(|_| relationship("payments-vendor", None))
        .collect();
    group_mut(&mut owner).actors = (0..1_024)
        .map(|_| actor(persona("shopper"), SystemActorDirection::Inbound))
        .collect();
    validate_values(&owner).unwrap();
    let mut over = owner.clone();
    group_mut(&mut over)
        .members
        .push(membership(component("checkout")));
    assert!(validate_values(&over).is_err());
    let mut over = owner.clone();
    group_mut(&mut over)
        .relationships
        .push(relationship("payments-vendor", None));
    assert!(validate_values(&over).is_err());
    let mut over = owner;
    group_mut(&mut over)
        .actors
        .push(actor(persona("shopper"), SystemActorDirection::Inbound));
    assert!(validate_values(&over).is_err());
    let mut domain = retail();
    domain_mut(&mut domain).members = (0..1_024)
        .map(|_| {
            domain_member(domain_member::Target::Domain(DomainRef {
                id: Some(id("x")),
            }))
        })
        .collect();
    validate_values(&domain).unwrap();
    domain_mut(&mut domain)
        .members
        .push(domain_member(domain_member::Target::Domain(DomainRef {
            id: Some(id("x")),
        })));
    assert!(validate_values(&domain).is_err());
    let mut large = platform();
    group_mut(&mut large).doc = "x".repeat(1_048_577);
    assert!(validate_values(&large).is_err());
}

#[test]
fn supplied_connection_ids_must_be_valid_and_unique_across_roles() {
    let mut owner = platform();
    group_mut(&mut owner).members[0].connection_id = Some(pb::ConnectionId {
        value: "not-a-uuid".into(),
    });
    assert!(validate_values(&owner).is_err());
    group_mut(&mut owner).members[0].connection_id = Some(pb::ConnectionId::default());
    assert!(validate_values(&owner).is_err());
    let shared: pb::ConnectionId = ConnectionId::new().into();
    let mut owner = platform();
    group_mut(&mut owner).members[0].connection_id = Some(shared.clone());
    group_mut(&mut owner).actors[0].connection_id = Some(shared.clone());
    assert!(validate_values(&owner).is_err());
    let mut owner = platform();
    group_mut(&mut owner).members[0].connection_id = Some(shared.clone());
    group_mut(&mut owner).relationships[0].connection_id = Some(shared);
    assert!(validate_values(&owner).is_err());
    let mut domain = retail();
    let first: pb::ConnectionId = ConnectionId::new().into();
    domain_mut(&mut domain).members[0].connection_id = Some(first.clone());
    domain_mut(&mut domain).members[1].connection_id = Some(first);
    assert!(validate_values(&domain).is_err());
}

#[test]
fn extraction_reports_every_typed_occurrence_with_its_diagnostic_path() {
    let stored = prepare_entity(None, &platform()).unwrap();
    let refs: Vec<_> = outbound_refs(&stored)
        .into_iter()
        .map(|r| (r.from_field, r.to_kind, r.to_id))
        .collect();
    let expect = |field: &str, kind: EntityKind, to: Id| {
        assert!(
            refs.contains(&(field.to_owned(), Some(kind), to.clone())),
            "missing {field}"
        );
    };
    expect(
        "members[0].component",
        EntityKind::Component,
        id("checkout"),
    );
    expect(
        "members[1].component",
        EntityKind::Component,
        id("checkout"),
    );
    expect(
        "members[2].component",
        EntityKind::Component,
        versioned("checkout", 2),
    );
    expect(
        "members[3].data_store",
        EntityKind::DataStore,
        id("orders-db"),
    );
    expect(
        "members[4].system_group",
        EntityKind::SystemGroup,
        id("storefront"),
    );
    expect(
        "members[5].external_system",
        EntityKind::ExternalSystem,
        id("provider"),
    );
    expect("members[6].schema", EntityKind::Schema, id("order"));
    expect(
        "relationships[0].target",
        EntityKind::SystemGroup,
        id("payments-vendor"),
    );
    expect(
        "relationships[1].target",
        EntityKind::SystemGroup,
        id("payments-vendor"),
    );
    expect(
        "actors[0].target.persona",
        EntityKind::Persona,
        id("shopper"),
    );
    expect(
        "actors[1].target.persona",
        EntityKind::Persona,
        id("shopper"),
    );
    expect(
        "actors[2].target.component",
        EntityKind::Component,
        id("worker"),
    );
    expect(
        "actors[3].target.external_system",
        EntityKind::ExternalSystem,
        id("provider"),
    );
    expect(
        "actors[4].target.system_group",
        EntityKind::SystemGroup,
        id("storefront"),
    );
    assert_eq!(refs.len(), 14);
    let stored = prepare_entity(None, &retail()).unwrap();
    let paths: Vec<_> = outbound_refs(&stored)
        .into_iter()
        .map(|r| (r.from_field, r.to_kind))
        .collect();
    assert_eq!(
        paths,
        vec![
            (
                "members[0].system_group".to_owned(),
                Some(EntityKind::SystemGroup)
            ),
            (
                "members[1].component".to_owned(),
                Some(EntityKind::Component)
            ),
            ("members[2].domain".to_owned(), Some(EntityKind::Domain)),
            (
                "members[3].data_product".to_owned(),
                Some(EntityKind::DataProduct)
            ),
            ("members[4].schema".to_owned(), Some(EntityKind::Schema)),
            (
                "members[5].external_system".to_owned(),
                Some(EntityKind::ExternalSystem)
            ),
        ]
    );
}

#[test]
fn design_graph_members_are_typed_but_wait_for_the_kind_to_land() {
    assert_eq!(system_group::design_graph_kind(), None);
    let mut owner = platform();
    group_mut(&mut owner)
        .members
        .push(membership(system_group_member::Target::DesignGraph(
            DesignGraphRef {
                id: Some(id("checkout-flow")),
            },
        )));
    validate_values(&owner).unwrap();
    let stored = prepare_entity(None, &owner).unwrap();
    let flow = outbound_refs(&stored)
        .into_iter()
        .find(|r| r.from_field == "members[7].design_graph")
        .expect("flow membership is reported");
    assert_eq!(flow.to_kind, system_group::design_graph_kind());
    assert_eq!(flow.to_id, id("checkout-flow"));
    let mut bad = owner;
    group_mut(&mut bad).members[7].member = Some(member(system_group_member::Target::DesignGraph(
        DesignGraphRef {
            id: Some(versioned("checkout-flow", 0)),
        },
    )));
    assert!(validate_values(&bad).is_err());
    let mut domain = retail();
    domain_mut(&mut domain)
        .members
        .push(domain_member(domain_member::Target::DesignGraph(
            DesignGraphRef {
                id: Some(id("checkout-flow")),
            },
        )));
    validate_values(&domain).unwrap();
}

#[test]
fn component_raw_members_stay_raw_and_accept_known_system_groups() {
    let component = Entity {
        kind: Some(entity::Kind::Component(Component {
            id: Some(id("checkout")),
            members: vec![EntityRef {
                kind: EntityKind::SystemGroup as i32,
                id: Some(id("platform")),
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    let stored = prepare_entity(None, &component).unwrap();
    assert_eq!(
        connection_occurrences(&stored).unwrap(),
        [] as [trogon_atlas_core::connections::ConnectionOccurrence; 0]
    );
    let refs = outbound_refs(&stored);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].from_field, "members[0]");
    assert_eq!(refs[0].to_kind, Some(EntityKind::SystemGroup));
}

#[test]
fn resolver_checks_complete_inventory_and_role_before_selecting() {
    for owner in [platform(), retail()] {
        let stored = prepare_entity(None, &owner).unwrap();
        for (index, occurrence) in connection_occurrences(&stored)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            let selected = ConnectionRef {
                id: occurrence.id.map(Into::into),
                ..selector(&stored, None)
            };
            let target = system_group::resolve_connection_context(&stored, &selected).unwrap();
            assert_eq!(target, occurrence.target, "occurrence {index}");
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
        let unknown = selector(&stored, Some(ConnectionId::new().into()));
        assert!(system_group::resolve_connection_context(&stored, &unknown).is_err());
        let mut wrong_uid = member_selector(&stored, 0);
        wrong_uid.owner_uid = uuid::Uuid::now_v7().to_string();
        assert!(system_group::resolve_connection_context(&stored, &wrong_uid).is_err());
    }
    let stored = prepare_entity(None, &platform()).unwrap();
    let selected = member_selector(&stored, 0);
    let mut corrupted = stored.clone();
    group_mut(&mut corrupted).members[3].connection_id = None;
    assert!(system_group::resolve_connection_context(&corrupted, &selected).is_err());
    let component = Entity {
        kind: Some(entity::Kind::Component(Component {
            id: Some(id("checkout")),
            ..Default::default()
        })),
        ..Default::default()
    };
    let component = prepare_entity(None, &component).unwrap();
    assert!(system_group::resolve_connection_context(&component, &selected).is_err());
}

#[test]
fn both_owner_kinds_are_admitted_as_connection_subject_owners() {
    assert!(responsibility::connection_subject_owner(
        EntityKind::SystemGroup
    ));
    assert!(responsibility::connection_subject_owner(EntityKind::Domain));
    assert!(!responsibility::connection_subject_owner(
        EntityKind::Subdomain
    ));
    assert!(!responsibility::connection_subject_owner(
        EntityKind::BoundedContext
    ));
}

#[test]
fn subject_selection_on_domain_is_limited_to_authored_members() {
    let stored = prepare_entity(None, &retail()).unwrap();
    let mut with_metadata = stored.clone();
    domain_mut(&mut with_metadata).project = Some(ProjectRef {
        id: Some(id("storefront-project")),
    });
    let selected = member_selector(&stored, 0);
    let target = system_group::resolve_connection_context(&stored, &selected)
        .unwrap()
        .unwrap();
    assert_eq!(target.kind, EntityKind::SystemGroup as i32);
    assert_eq!(target.id, Some(id("platform")));
}

#[test]
fn wording_and_order_preserve_identity_but_exact_targets_and_direction_do_not() {
    let stored = prepare_entity(None, &platform()).unwrap();
    let mut edited = stored.clone();
    group_mut(&mut edited).members.reverse();
    group_mut(&mut edited).members[6].doc = Some(text("reworded"));
    group_mut(&mut edited).relationships[0].label = Some(title("settles with"));
    group_mut(&mut edited).actors[0].label = None;
    group_mut(&mut edited).actors[0].doc = Some(text("notes"));
    let edited = prepare_entity(Some(&stored), &edited).unwrap();
    assert_eq!(member_selector(&stored, 0), member_selector(&edited, 6));
    assert!(connections::resolve_connection(&edited, &member_selector(&stored, 0)).is_ok());
    assert_eq!(
        group_mut(&mut edited.clone()).actors[0].connection_id,
        group_mut(&mut stored.clone()).actors[0].connection_id
    );

    let mut retargeted = edited.clone();
    group_mut(&mut retargeted).members[6].member = Some(member(component("worker")));
    assert!(matches!(
        prepare_entity(Some(&edited), &retargeted),
        Err(ConnectionError::Retargeted(_))
    ));
    group_mut(&mut retargeted).members[6].connection_id = None;
    let retargeted = prepare_entity(Some(&edited), &retargeted).unwrap();
    assert_ne!(
        member_selector(&retargeted, 6).id,
        member_selector(&stored, 0).id
    );
    assert!(connections::resolve_connection(&retargeted, &member_selector(&stored, 0)).is_err());

    let mut flipped = stored.clone();
    group_mut(&mut flipped).actors[0].direction = SystemActorDirection::Outbound as i32;
    assert!(matches!(
        prepare_entity(Some(&stored), &flipped),
        Err(ConnectionError::Retargeted(_))
    ));
    group_mut(&mut flipped).actors[0].connection_id = None;
    let flipped = prepare_entity(Some(&stored), &flipped).unwrap();
    assert_ne!(
        group_mut(&mut flipped.clone()).actors[0].connection_id,
        group_mut(&mut stored.clone()).actors[0].connection_id
    );
    assert_eq!(
        group_mut(&mut flipped.clone()).actors[1].connection_id,
        group_mut(&mut stored.clone()).actors[1].connection_id
    );

    let mut other_role = stored.clone();
    let moved = group_mut(&mut other_role).members.remove(4);
    group_mut(&mut other_role).members.insert(
        4,
        SystemGroupMembership {
            member: moved.member.clone(),
            ..moved.clone()
        },
    );
    assert!(prepare_entity(Some(&stored), &other_role).is_ok());
}

#[test]
fn exact_reapply_is_a_noop_and_edits_without_ids_are_rejected() {
    let authored = platform();
    let stored = prepare_entity(None, &authored).unwrap();
    assert_eq!(prepare_entity(Some(&stored), &authored).unwrap(), stored);
    let mut edited = authored;
    group_mut(&mut edited).members[0].doc = Some(text("changed"));
    assert!(matches!(
        prepare_entity(Some(&stored), &edited),
        Err(ConnectionError::StrippedId(_))
    ));
    let mut added = stored.clone();
    group_mut(&mut added)
        .members
        .push(membership(component("checkout")));
    let preview = prepare_entity(Some(&stored), &added).unwrap();
    assert!(matches!(
        prepare_entity(Some(&stored), &preview),
        Err(ConnectionError::UnknownId(_))
    ));
}

#[test]
fn removal_recreation_and_branch_restoration_keep_selectors_exact() {
    let stored = prepare_entity(None, &platform()).unwrap();
    let selected = member_selector(&stored, 0);
    let sibling = member_selector(&stored, 1);
    let mut removed = stored.clone();
    group_mut(&mut removed).members.remove(0);
    let removed = prepare_entity(Some(&stored), &removed).unwrap();
    assert!(connections::resolve_connection(&removed, &selected).is_err());
    assert!(connections::resolve_connection(&removed, &sibling).is_ok());
    assert!(matches!(
        prepare_landing(Some(&stored), Some(&removed), &stored),
        Err(ConnectionError::RemovedFromBranchBase(_))
    ));
    let restored = prepare_resolution(Some(&stored), Some(&removed), &stored).unwrap();
    assert_ne!(member_selector(&restored, 0).id, selected.id);
    assert_eq!(member_selector(&restored, 1).id, sibling.id);
    let recreated = prepare_entity(None, &stored).unwrap();
    assert_ne!(member_selector(&recreated, 0).owner_uid, selected.owner_uid);
    assert!(connections::resolve_connection(&recreated, &selected).is_err());
    let mut readded = removed.clone();
    group_mut(&mut readded)
        .members
        .push(membership(component("checkout")));
    let readded = prepare_entity(Some(&removed), &readded).unwrap();
    assert_ne!(member_selector(&readded, 6).id, selected.id);
}

#[test]
fn retarget_renews_only_matching_occurrences_and_leaves_siblings_and_other_kinds() {
    let stored = prepare_entity(None, &platform()).unwrap();
    let selected = member_selector(&stored, 0);
    let mut changed = stored.clone();
    assert_eq!(
        retarget_refs(
            &mut changed,
            EntityKind::Component,
            &id("checkout"),
            &id("replacement")
        ),
        vec!["members[0].component", "members[1].component"]
    );
    assert!(group_mut(&mut changed).members[0].connection_id.is_none());
    assert!(group_mut(&mut changed).members[2].connection_id.is_some());
    let changed = prepare_entity(Some(&stored), &changed).unwrap();
    assert!(connections::resolve_connection(&changed, &selected).is_err());
    assert_eq!(member_selector(&stored, 3), member_selector(&changed, 3));

    let mut actors = stored.clone();
    assert_eq!(
        retarget_refs(
            &mut actors,
            EntityKind::Persona,
            &id("shopper"),
            &id("buyer")
        ),
        vec!["actors[0].target.persona", "actors[1].target.persona"]
    );
    let mut wrong_kind = stored.clone();
    assert_eq!(
        retarget_refs(
            &mut wrong_kind,
            EntityKind::DataStore,
            &id("checkout"),
            &id("replacement")
        ),
        [] as [std::string::String; 0]
    );
    assert_eq!(wrong_kind, stored);

    let mut relationships = stored.clone();
    assert_eq!(
        retarget_refs(
            &mut relationships,
            EntityKind::SystemGroup,
            &id("payments-vendor"),
            &id("settlement")
        ),
        vec!["relationships[0].target", "relationships[1].target"]
    );
    let mut nested = stored.clone();
    assert_eq!(
        retarget_refs(
            &mut nested,
            EntityKind::SystemGroup,
            &id("storefront"),
            &id("web")
        ),
        vec!["members[4].system_group", "actors[4].target.system_group"]
    );

    let mut domain = prepare_entity(None, &retail()).unwrap();
    assert_eq!(
        retarget_refs(
            &mut domain,
            EntityKind::Domain,
            &id("fulfilment"),
            &id("ops")
        ),
        vec!["members[2].domain"]
    );
    assert!(domain_mut(&mut domain).members[2].connection_id.is_none());
}

#[test]
fn advisories_report_cycles_and_exact_repeats_without_blocking_anything() {
    let platform = prepare_entity(
        None,
        &group_with(
            "platform",
            vec![
                membership(component("checkout")),
                membership(component("checkout")),
                membership(group_ref("storefront")),
            ],
            vec![],
        ),
    )
    .unwrap();
    let storefront = prepare_entity(
        None,
        &group_with(
            "storefront",
            vec![
                membership(component("checkout")),
                membership(group_ref("platform")),
            ],
            vec![],
        ),
    )
    .unwrap();
    let retail = prepare_entity(None, &retail()).unwrap();
    let fulfilment = prepare_entity(
        None,
        &domain_with(
            "fulfilment",
            vec![domain_member(domain_member::Target::Domain(DomainRef {
                id: Some(id("retail")),
            }))],
        ),
    )
    .unwrap();
    let owners = vec![
        platform.clone(),
        storefront.clone(),
        retail.clone(),
        fulfilment.clone(),
    ];
    let findings = system_group::advisories(&owners);
    let by_rule = |rule: &str| {
        findings
            .iter()
            .filter(|f| f.rule == rule)
            .collect::<Vec<_>>()
    };
    let group_cycles = by_rule(system_group::SYSTEM_GROUP_MEMBERSHIP_CYCLE);
    assert_eq!(group_cycles.len(), 1);
    assert_eq!(group_cycles[0].path.len(), 2);
    let hop_ids: Vec<_> = group_cycles[0]
        .path
        .iter()
        .map(|hop| (hop.owner.id.clone().unwrap().slug, hop.connection_id))
        .collect();
    assert!(hop_ids.contains(&("platform".into(), platform_id(&platform, 2))));
    assert!(hop_ids.contains(&("storefront".into(), platform_id(&storefront, 1))));
    assert_eq!(
        group_cycles[0].path[0].owner_uid,
        owner_uid(&group_cycles[0].path[0].owner, &owners)
    );
    let domain_cycles = by_rule(system_group::DOMAIN_MEMBERSHIP_CYCLE);
    assert_eq!(domain_cycles.len(), 1);
    assert_eq!(domain_cycles[0].path.len(), 2);
    let repeats = by_rule(system_group::SYSTEM_GROUP_MEMBERSHIP_REPEAT);
    assert_eq!(repeats.len(), 1);
    assert_eq!(repeats[0].path.len(), 2);
    assert_eq!(repeats[0].path[0].target, repeats[0].path[1].target);
    assert_ne!(
        repeats[0].path[0].connection_id,
        repeats[0].path[1].connection_id
    );
    assert_eq!(
        by_rule(system_group::DOMAIN_MEMBERSHIP_REPEAT),
        [] as [&trogon_atlas_core::system_group::Advisory; 0]
    );
    // The advisory run changes nothing and a hidden participant produces no cycle.
    assert_eq!(system_group::advisories(&owners), findings);
    let visible = system_group::advisories(std::slice::from_ref(&platform));
    assert!(visible
        .iter()
        .all(|f| f.rule != system_group::SYSTEM_GROUP_MEMBERSHIP_CYCLE));
    assert!(visible
        .iter()
        .any(|f| f.rule == system_group::SYSTEM_GROUP_MEMBERSHIP_REPEAT));
    assert_eq!(validate_values(&platform).ok(), Some(()));
}

fn platform_id(owner: &Entity, index: usize) -> ConnectionId {
    let id = match owner.kind.as_ref().unwrap() {
        entity::Kind::SystemGroup(value) => value.members[index].connection_id.clone(),
        _ => panic!("group"),
    };
    id.unwrap().value.parse().unwrap()
}
fn owner_uid(owner: &EntityRef, owners: &[Entity]) -> String {
    owners
        .iter()
        .find(|entity| {
            entity_id(entity) == owner.id.as_ref()
                && entity_kind(entity).map(|kind| kind as i32) == Some(owner.kind)
        })
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone()
}

#[test]
fn long_cycles_are_reported_once_and_work_is_bounded() {
    let owners: Vec<_> = (0..40)
        .map(|index| {
            prepare_entity(
                None,
                &group_with(
                    &format!("g{index}"),
                    vec![membership(group_ref(&format!("g{}", (index + 1) % 40)))],
                    vec![],
                ),
            )
            .unwrap()
        })
        .collect();
    let findings = system_group::advisories(&owners);
    let cycles: Vec<_> = findings
        .iter()
        .filter(|f| f.rule == system_group::SYSTEM_GROUP_MEMBERSHIP_CYCLE)
        .collect();
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].path.len(), 40);
}
