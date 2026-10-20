use trogon_atlas_core::{
    connection_persistence::prepare_entity,
    connections::{connection_occurrences, resolve_connection, ConnectionError, ConnectionId},
    entity_id, entity_kind, outbound_refs, retarget_refs,
};
use trogon_atlas_proto::{
    channel, entity, Channel, ChannelBinding, ComponentRef, ConnectionRef, DataStore, Entity,
    EntityKind, EntityRef, Id, ReadModelAttachment, ReadModelRef,
};

fn id(slug: &str) -> Id {
    Id {
        namespace: "shop".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn observation() -> ConnectionRef {
    ConnectionRef {
        owner: Some(EntityRef {
            kind: EntityKind::AutomationSlice as i32,
            id: Some(id("dispatch")),
        }),
        owner_uid: "observation-owner".into(),
        id: Some(ConnectionId::new().into()),
    }
}

fn channel(observations: Vec<ConnectionRef>) -> Entity {
    Entity {
        kind: Some(entity::Kind::Channel(Channel {
            id: Some(id("orders")),
            title: "Orders".into(),
            bindings: observations
                .into_iter()
                .map(|observation| ChannelBinding {
                    observation: Some(observation),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn store() -> Entity {
    Entity {
        kind: Some(entity::Kind::DataStore(DataStore {
            id: Some(id("orders")),
            title: "Orders".into(),
            attachments: ["ready", "sent"]
                .into_iter()
                .map(|slug| ReadModelAttachment {
                    read_model: Some(ReadModelRef { id: Some(id(slug)) }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn channel_associations_preserve_repeated_observation_identity_and_reorder(
) -> Result<(), ConnectionError> {
    let first = observation();
    let mut second = first.clone();
    second.id = Some(ConnectionId::new().into());
    let stored = prepare_entity(None, &channel(vec![first, second]))?;
    let old = connection_occurrences(&stored)?;
    assert_eq!(old.len(), 2);
    assert_ne!(old[0].id, old[1].id);
    assert_eq!(old[0].target, old[1].target);
    let mut edited = stored.clone();
    let Some(entity::Kind::Channel(resource)) = edited.kind.as_mut() else {
        panic!("channel")
    };
    resource.bindings.reverse();
    resource.bindings[0].doc = "Delivery intent".into();
    resource.bindings[0]
        .parameter_values
        .insert("region".into(), "west".into());
    let edited = prepare_entity(Some(&stored), &edited)?;
    let new = connection_occurrences(&edited)?;
    assert_eq!(old[0].id, new[1].id);
    assert_eq!(old[1].id, new[0].id);
    Ok(())
}

#[test]
fn channel_selector_occurrence_uid_kind_and_exact_owner_are_identity_endpoints(
) -> Result<(), ConnectionError> {
    let stored = prepare_entity(None, &channel(vec![observation()]))?;
    for change in 0..4 {
        let mut edited = stored.clone();
        let Some(entity::Kind::Channel(resource)) = edited.kind.as_mut() else {
            panic!("channel")
        };
        let selected = resource.bindings[0]
            .observation
            .as_mut()
            .ok_or(ConnectionError::MissingId)?;
        match change {
            0 => selected.id = Some(ConnectionId::new().into()),
            1 => selected.owner_uid.push_str("-recreated"),
            2 => {
                selected
                    .owner
                    .as_mut()
                    .ok_or(ConnectionError::WrongOwner)?
                    .kind = EntityKind::UiSlice as i32;
            }
            _ => {
                selected
                    .owner
                    .as_mut()
                    .ok_or(ConnectionError::WrongOwner)?
                    .id = Some(id("other-dispatch"));
            }
        }
        assert!(matches!(
            prepare_entity(Some(&stored), &edited),
            Err(ConnectionError::Retargeted(_))
        ));
    }
    Ok(())
}

#[test]
fn resource_association_ids_are_not_client_allocated_on_update() -> Result<(), ConnectionError> {
    for draft in [store(), channel(vec![observation()])] {
        let stored = prepare_entity(None, &draft)?;
        let mut changed = stored.clone();
        match changed.kind.as_mut() {
            Some(entity::Kind::Channel(channel)) => {
                channel.bindings[0].connection_id = Some(ConnectionId::new().into());
            }
            Some(entity::Kind::DataStore(store)) => {
                store.attachments[0].connection_id = Some(ConnectionId::new().into());
            }
            _ => panic!("resource"),
        }
        assert!(matches!(
            prepare_entity(Some(&stored), &changed),
            Err(ConnectionError::UnknownId(_))
        ));
        assert_eq!(prepare_entity(Some(&stored), &draft)?, stored);
    }
    Ok(())
}

#[test]
fn datastore_retarget_requires_new_occurrence_and_clone_has_new_owner_identity(
) -> Result<(), ConnectionError> {
    let stored = prepare_entity(None, &store())?;
    let old = connection_occurrences(&stored)?;
    let mut edited = stored.clone();
    let rewritten = retarget_refs(
        &mut edited,
        EntityKind::ReadModel,
        &id("ready"),
        &id("pending"),
    );
    assert_eq!(rewritten, ["attachments[0].read_model"]);
    assert!(matches!(
        prepare_entity(Some(&stored), &edited),
        Err(ConnectionError::Retargeted(_))
    ));
    assert_eq!(
        trogon_atlas_core::connections::renew_retargeted_connection_ids(&stored, &mut edited)?,
        1
    );
    let edited = prepare_entity(Some(&stored), &edited)?;
    let new = connection_occurrences(&edited)?;
    assert_ne!(new[0].id, old[0].id);
    assert_eq!(new[1].id, old[1].id);
    let reference = ConnectionRef {
        owner: Some(EntityRef {
            kind: EntityKind::DataStore as i32,
            id: entity_id(&stored).cloned(),
        }),
        owner_uid: stored
            .system
            .as_ref()
            .ok_or(ConnectionError::WrongOwner)?
            .uid
            .clone(),
        id: old[0].id.map(Into::into),
    };
    assert_eq!(resolve_connection(&stored, &reference)?.id, old[0].id);
    let clone = prepare_entity(None, &stored)?;
    assert_ne!(clone.system, stored.system);
    assert!(matches!(
        resolve_connection(&clone, &reference),
        Err(ConnectionError::WrongOwner)
    ));
    assert_eq!(entity_kind(&clone), Some(EntityKind::DataStore));
    Ok(())
}

#[test]
fn channel_impact_discovers_observation_owner_without_implicitly_retargeting_it() {
    let mut draft = channel(vec![observation()]);
    let Some(entity::Kind::Channel(resource)) = draft.kind.as_mut() else {
        panic!("channel")
    };
    resource.broker = Some(channel::Broker::Component(ComponentRef {
        id: Some(id("broker")),
    }));
    let refs = outbound_refs(&draft);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[1].to_kind, Some(EntityKind::AutomationSlice));
    assert_eq!(refs[1].to_id, id("dispatch"));
    assert_eq!(refs[1].from_field, "bindings[0].observation.owner");
    let previous = draft.clone();
    assert_eq!(
        retarget_refs(
            &mut draft,
            EntityKind::AutomationSlice,
            &id("dispatch"),
            &id("dispatch-v2")
        ),
        Vec::<String>::new()
    );
    assert_eq!(draft, previous);
    assert_eq!(
        retarget_refs(
            &mut draft,
            EntityKind::Component,
            &id("broker"),
            &id("broker-v2")
        ),
        ["component"]
    );
}
