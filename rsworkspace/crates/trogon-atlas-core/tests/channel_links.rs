#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{
    channel_links::{self, validate_values, MAX_LINKS},
    connection_persistence::prepare_entity,
    connections::{self, ConnectionError},
    outbound_refs, retarget_refs,
};
use trogon_atlas_proto::*;

fn id(slug: &str) -> Id {
    Id {
        namespace: "messaging".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn link(kind: ChannelLinkKind, target: &str) -> ChannelLink {
    ChannelLink {
        kind: kind as i32,
        target: Some(ChannelRef {
            id: Some(id(target)),
        }),
        parameter_values: [("tenant".to_string(), "acme".to_string())].into(),
        doc: Some(CatalogText {
            value: "routes orders".into(),
        }),
        ..Default::default()
    }
}
fn channel(slug: &str, links: Vec<ChannelLink>) -> Entity {
    Entity {
        kind: Some(entity::Kind::Channel(Channel {
            id: Some(id(slug)),
            title: "Orders".into(),
            links,
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn links_mut(entity: &mut Entity) -> &mut Vec<ChannelLink> {
    let Some(entity::Kind::Channel(value)) = &mut entity.kind else {
        panic!("channel")
    };
    &mut value.links
}

#[test]
fn equal_links_repeat_with_distinct_ids_and_descriptive_values_keep_identity() {
    let proposed = channel(
        "orders",
        vec![
            link(ChannelLinkKind::Composes, "billing"),
            link(ChannelLinkKind::Composes, "billing"),
            link(ChannelLinkKind::RoutesTo, "billing"),
        ],
    );
    validate_values(&proposed).unwrap();
    let stored = prepare_entity(None, &proposed).unwrap();
    assert_eq!(
        connections::connection_occurrences(&stored).unwrap().len(),
        3
    );
    let ids: Vec<_> = channel_links::links(&stored)
        .iter()
        .map(|v| v.connection_id.clone().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);

    let mut edited = stored.clone();
    links_mut(&mut edited)[0].parameter_values.clear();
    links_mut(&mut edited)[1].doc = None;
    links_mut(&mut edited)[2]
        .parameter_values
        .insert("region".into(), "eu".into());
    links_mut(&mut edited).reverse();
    let edited = prepare_entity(Some(&stored), &edited).unwrap();
    let kept: Vec<_> = channel_links::links(&edited)
        .iter()
        .map(|v| v.connection_id.clone().unwrap())
        .collect();
    assert_eq!(kept, [ids[2].clone(), ids[1].clone(), ids[0].clone()]);
    assert_eq!(prepare_entity(Some(&stored), &stored).unwrap(), stored);
}

#[test]
fn kind_or_target_change_requires_a_fresh_id() {
    let stored = prepare_entity(
        None,
        &channel("orders", vec![link(ChannelLinkKind::Composes, "billing")]),
    )
    .unwrap();
    let mut kind = stored.clone();
    links_mut(&mut kind)[0].kind = ChannelLinkKind::RoutesTo as i32;
    assert!(matches!(
        prepare_entity(Some(&stored), &kind),
        Err(ConnectionError::Retargeted(_))
    ));
    let mut target = stored.clone();
    links_mut(&mut target)[0].target = Some(ChannelRef {
        id: Some(id("shipping")),
    });
    assert!(matches!(
        prepare_entity(Some(&stored), &target),
        Err(ConnectionError::Retargeted(_))
    ));
    links_mut(&mut target)[0].connection_id = None;
    let renewed = prepare_entity(Some(&stored), &target).unwrap();
    assert_ne!(
        channel_links::links(&renewed)[0].connection_id,
        channel_links::links(&stored)[0].connection_id
    );
    let mut stripped = stored.clone();
    links_mut(&mut stripped)[0].connection_id = None;
    links_mut(&mut stripped)[0].doc = None;
    assert!(matches!(
        prepare_entity(Some(&stored), &stripped),
        Err(ConnectionError::StrippedId(_))
    ));
}

#[test]
fn outbound_and_retarget_cover_every_link_target() {
    let stored = prepare_entity(
        None,
        &channel(
            "orders",
            vec![
                link(ChannelLinkKind::Composes, "billing"),
                link(ChannelLinkKind::RoutesTo, "shipping"),
                link(ChannelLinkKind::RoutesTo, "billing"),
            ],
        ),
    )
    .unwrap();
    let refs: Vec<_> = outbound_refs(&stored)
        .into_iter()
        .filter(|r| r.from_field.starts_with("links["))
        .collect();
    assert_eq!(refs.len(), 3);
    assert_eq!(refs[0].from_field, "links[0].target");
    assert_eq!(refs[0].to_kind, Some(EntityKind::Channel));
    assert_eq!(refs[1].to_id, id("shipping"));
    let mut changed = stored.clone();
    let paths = retarget_refs(
        &mut changed,
        EntityKind::Channel,
        &id("billing"),
        &id("fresh"),
    );
    assert_eq!(paths, ["links[0].target", "links[2].target"]);
    let links = channel_links::links(&changed);
    assert!(links[0].connection_id.is_none() && links[2].connection_id.is_none());
    assert!(links[1].connection_id.is_some());
    assert_eq!(links[0].target.as_ref().unwrap().id, Some(id("fresh")));
    assert_eq!(links[0].parameter_values.len(), 1);
}

#[test]
fn value_rules_allow_cross_key_cycles_and_reject_self_key_and_bad_shapes() {
    validate_values(&channel("a", vec![link(ChannelLinkKind::Composes, "b")])).unwrap();
    validate_values(&channel("b", vec![link(ChannelLinkKind::RoutesTo, "a")])).unwrap();
    let mut newer = channel("a", vec![link(ChannelLinkKind::Composes, "a")]);
    links_mut(&mut newer)[0]
        .target
        .as_mut()
        .unwrap()
        .id
        .as_mut()
        .unwrap()
        .version = 2;
    assert!(validate_values(&newer).is_err(), "same key is a self link");
    for case in 0..8 {
        let mut invalid = channel("a", vec![link(ChannelLinkKind::Composes, "b")]);
        let value = &mut links_mut(&mut invalid)[0];
        match case {
            0 => value.kind = 0,
            1 => value.kind = 99,
            2 => value.target = None,
            3 => value.target.as_mut().unwrap().id = None,
            4 => value.target.as_mut().unwrap().id.as_mut().unwrap().version = 0,
            5 => value.target.as_mut().unwrap().id.as_mut().unwrap().slug = "../x".into(),
            6 => {
                value.doc = Some(CatalogText {
                    value: String::new(),
                });
            }
            _ => {
                value.connection_id = Some(ConnectionId {
                    value: "not-a-uuid".into(),
                });
            }
        }
        assert!(validate_values(&invalid).is_err(), "case {case}");
    }
}

#[test]
fn link_count_and_owner_size_are_bounded() {
    let mut owner = channel("a", vec![]);
    links_mut(&mut owner).resize(MAX_LINKS, link(ChannelLinkKind::Composes, "b"));
    validate_values(&owner).unwrap();
    links_mut(&mut owner).push(link(ChannelLinkKind::Composes, "b"));
    assert!(validate_values(&owner).is_err());
    let mut large = channel("a", vec![link(ChannelLinkKind::Composes, "b")]);
    let Some(entity::Kind::Channel(value)) = &mut large.kind else {
        panic!("channel")
    };
    value.doc = "x".repeat(channel_links::MAX_ENCODED_BYTES);
    assert!(validate_values(&large).is_err());
}
