use prost::Message;
use trogon_atlas_core::{
    connection_persistence::prepare_entity, connections::connection_occurrences,
};
use trogon_atlas_proto::{entity, CatalogTitle, Component, Entity, Id, UiRef};

#[derive(Clone, PartialEq, Message)]
struct HostingWire {
    #[prost(message, optional, tag = "2")]
    ui: Option<UiRef>,
    #[prost(message, optional, tag = "3")]
    label: Option<CatalogTitle>,
}
#[derive(Clone, PartialEq, Message)]
struct ComponentWire {
    #[prost(message, optional, tag = "1")]
    id: Option<Id>,
    #[prost(message, repeated, tag = "10")]
    hostings: Vec<HostingWire>,
}
#[test]
fn native_preparation_preserves_equal_hostings_as_distinct_occurrences(
) -> Result<(), Box<dyn std::error::Error>> {
    let id = |slug: &str| Id {
        namespace: "shop".into(),
        slug: slug.into(),
        version: 1,
    };
    let hosting = HostingWire {
        ui: Some(UiRef {
            id: Some(id("review")),
        }),
        label: Some(CatalogTitle {
            value: "Review".into(),
        }),
    };
    let component = Component::decode(
        ComponentWire {
            id: Some(id("portal")),
            hostings: vec![hosting.clone(), hosting],
        }
        .encode_to_vec()
        .as_slice(),
    )?;
    let prepared = prepare_entity(
        None,
        &Entity {
            kind: Some(entity::Kind::Component(component)),
            ..Default::default()
        },
    )?;
    let occurrences = connection_occurrences(&prepared)?;
    assert_eq!(
        occurrences.len(),
        2,
        "native hosting occurrences must survive preparation"
    );
    assert_ne!(occurrences[0].id, occurrences[1].id);
    assert_eq!(occurrences[0].target, occurrences[1].target);
    Ok(())
}

fn owner() -> Entity {
    Entity {
        kind: Some(entity::Kind::Component(Component {
            id: Some(Id {
                namespace: "shop".into(),
                slug: "portal".into(),
                version: 1,
            }),
            ui_hostings: vec![trogon_atlas_proto::UiHosting {
                ui: Some(UiRef {
                    id: Some(Id {
                        namespace: "shop".into(),
                        slug: "review".into(),
                        version: 1,
                    }),
                }),
                label: Some(CatalogTitle {
                    value: "Review".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn preparation_rejects_invalid_hosting_values_before_identity_assignment() {
    for case in 0..8 {
        let mut candidate = owner();
        let Some(entity::Kind::Component(component)) = candidate.kind.as_mut() else {
            panic!("component")
        };
        match case {
            0 => {
                component.ui_hostings[0].connection_id = Some(trogon_atlas_proto::ConnectionId {
                    value: String::new(),
                })
            }
            1 => component.ui_hostings[0].ui = None,
            2 => {
                component.ui_hostings[0]
                    .ui
                    .as_mut()
                    .unwrap()
                    .id
                    .as_mut()
                    .unwrap()
                    .version = 0
            }
            3 => component.ui_hostings[0].label = None,
            4 => component.ui_hostings[0].label.as_mut().unwrap().value = " ".into(),
            5 => component.ui_hostings[0].label.as_mut().unwrap().value = "a".repeat(257),
            6 => {
                component.ui_hostings[0].doc = Some(trogon_atlas_proto::CatalogText {
                    value: "a".repeat(16_385),
                })
            }
            _ => component.ui_hostings = vec![component.ui_hostings[0].clone(); 1_025],
        }
        assert!(
            prepare_entity(None, &candidate).is_err(),
            "invalid hosting case {case} published"
        );
    }
}

#[test]
fn hosting_reference_lifecycle_preserves_occurrence_provenance() -> Result<(), Box<dyn std::error::Error>> {
    use trogon_atlas_core::{outbound_refs, retarget_refs, responsibility};
    use trogon_atlas_proto::{ConnectionRef, EntityKind, EntityRef, ResponsibilitySubject, responsibility_subject};
    let stored = prepare_entity(None, &owner())?;
    let occurrence = &connection_occurrences(&stored)?[0];
    let selector = ConnectionRef { owner: Some(EntityRef { kind: EntityKind::Component as i32, id: trogon_atlas_core::entity_id(&stored).cloned() }), owner_uid: stored.system.as_ref().unwrap().uid.clone(), id: occurrence.id.clone().map(Into::into) };
    let refs = outbound_refs(&stored);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].from_field, "ui_hostings[0].ui");
    assert_eq!(refs[0].to_kind, Some(EntityKind::Ui));
    let subject = ResponsibilitySubject { selection: Some(responsibility_subject::Selection::Connection(selector)), };
    responsibility::resolve_subject(&subject, &stored)?;
    let mut changed = stored.clone();
    let old = refs[0].to_id.clone();
    let mut new = old.clone(); new.version = 2;
    let paths = retarget_refs(&mut changed, EntityKind::Ui, &old, &new);
    assert_eq!(paths, vec!["ui_hostings[0].ui"]);
    let changed = prepare_entity(Some(&stored), &changed)?;
    assert_ne!(connection_occurrences(&changed)?[0].id, occurrence.id);
    assert!(responsibility::resolve_subject(&subject, &changed).is_err());
    Ok(())
}
