#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{
    connection_persistence::{prepare_entity, prepare_resolution},
    connections::{ConnectionError, ConnectionId as Cid},
    data_product, outbound_refs, retarget_refs,
    storage_claims::{self, validate_values, MAX_CLAIMS},
};
use trogon_atlas_proto::*;

const OWNERS: [&str; 3] = ["component", "external_system", "data_product"];

fn id(slug: &str) -> Id {
    Id {
        namespace: "platform".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn attachment(store: &str) -> ConnectionRef {
    ConnectionRef {
        owner: Some(EntityRef {
            kind: EntityKind::DataStore as i32,
            id: Some(id(store)),
        }),
        owner_uid: "01930000-0000-7000-8000-000000000001".into(),
        id: Some(Cid::new().into()),
    }
}
fn claim(store: &str, direction: StorageDirection) -> StorageAccessClaim {
    StorageAccessClaim {
        store: Some(DataStoreRef {
            id: Some(id(store)),
        }),
        direction: direction as i32,
        doc: Some(CatalogText {
            value: "reads receipts".into(),
        }),
        ..Default::default()
    }
}
fn owner(kind: &str, claims: Vec<StorageAccessClaim>) -> Entity {
    let input: DataPortId = data_product::DataPortId::new().into();
    Entity {
        kind: Some(match kind {
            "component" => entity::Kind::Component(Component {
                id: Some(id("checkout")),
                storage_claims: claims,
                ..Default::default()
            }),
            "external_system" => entity::Kind::ExternalSystem(ExternalSystem {
                id: Some(id("checkout")),
                storage_claims: claims,
                ..Default::default()
            }),
            _ => entity::Kind::DataProduct(DataProduct {
                id: Some(id("checkout")),
                title: "Order data".into(),
                inputs: vec![DataInputPort {
                    port_id: Some(input.clone()),
                    name: Some(PortName {
                        value: "Order fact".into(),
                    }),
                    ..Default::default()
                }],
                outputs: vec![DataOutputPort {
                    name: Some(PortName {
                        value: "Receipt".into(),
                    }),
                    ..Default::default()
                }],
                storage_claims: claims,
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}
fn claims_mut(entity: &mut Entity) -> &mut Vec<StorageAccessClaim> {
    match entity.kind.as_mut().unwrap() {
        entity::Kind::Component(v) => &mut v.storage_claims,
        entity::Kind::ExternalSystem(v) => &mut v.storage_claims,
        entity::Kind::DataProduct(v) => &mut v.storage_claims,
        _ => panic!("owner"),
    }
}
fn product(entity: &Entity) -> &DataProduct {
    let Some(entity::Kind::DataProduct(value)) = &entity.kind else {
        panic!("product")
    };
    value
}
fn trio(kind: &str) -> Entity {
    owner(
        kind,
        vec![
            claim("receipts", StorageDirection::Read),
            claim("receipts", StorageDirection::Write),
            claim("receipts", StorageDirection::Read),
        ],
    )
}

#[test]
fn every_owner_kind_keeps_read_write_and_equal_claims_as_distinct_occurrences() {
    for kind in OWNERS {
        let proposed = trio(kind);
        validate_values(&proposed).unwrap();
        let stored = prepare_entity(None, &proposed).unwrap();
        let ids: Vec<_> = storage_claims::claims(&stored)
            .iter()
            .map(|v| v.connection_id.clone().unwrap())
            .collect();
        assert!(
            ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2],
            "{kind}"
        );
        let refs: Vec<_> = outbound_refs(&stored)
            .into_iter()
            .filter(|r| r.from_field.starts_with("storage_claims["))
            .collect();
        assert_eq!(refs.len(), 3, "{kind}");
        assert_eq!(refs[1].from_field, "storage_claims[1].store");
        assert_eq!(refs[1].to_kind, Some(EntityKind::DataStore));

        let mut edited = stored.clone();
        claims_mut(&mut edited)[0].doc = None;
        claims_mut(&mut edited).reverse();
        let edited = prepare_entity(Some(&stored), &edited).unwrap();
        let kept: Vec<_> = storage_claims::claims(&edited)
            .iter()
            .map(|v| v.connection_id.clone().unwrap())
            .collect();
        assert_eq!(kept, [ids[2].clone(), ids[1].clone(), ids[0].clone()]);
        assert_eq!(prepare_entity(Some(&stored), &stored).unwrap(), stored);

        let mut direction = stored.clone();
        claims_mut(&mut direction)[0].direction = StorageDirection::Write as i32;
        let mut store = stored.clone();
        claims_mut(&mut store)[0].store = Some(DataStoreRef {
            id: Some(id("other")),
        });
        let mut attached = stored.clone();
        claims_mut(&mut attached)[0].attachment = Some(attachment("receipts"));
        for changed in [direction, store, attached] {
            assert!(
                matches!(
                    prepare_entity(Some(&stored), &changed),
                    Err(ConnectionError::Retargeted(_))
                ),
                "{kind}"
            );
        }
    }
}

#[test]
fn claim_shape_rules_reject_incomplete_selectors() {
    for kind in OWNERS {
        for case in 0..11 {
            let mut invalid = owner(kind, vec![claim("receipts", StorageDirection::Read)]);
            claims_mut(&mut invalid)[0].attachment = Some(attachment("receipts"));
            let value = &mut claims_mut(&mut invalid)[0];
            match case {
                0 => value.direction = 0,
                1 => value.direction = 99,
                2 => value.store = None,
                3 => value.store.as_mut().unwrap().id.as_mut().unwrap().version = 0,
                4 => {
                    value
                        .attachment
                        .as_mut()
                        .unwrap()
                        .owner
                        .as_mut()
                        .unwrap()
                        .kind = EntityKind::Channel as i32
                }
                5 => {
                    value
                        .attachment
                        .as_mut()
                        .unwrap()
                        .owner
                        .as_mut()
                        .unwrap()
                        .id = Some(id("other"))
                }
                6 => value.attachment.as_mut().unwrap().owner_uid.clear(),
                7 => {
                    value.attachment.as_mut().unwrap().id = Some(ConnectionId {
                        value: "bad".into(),
                    })
                }
                8 => value.attachment.as_mut().unwrap().owner = None,
                9 => {
                    value.doc = Some(CatalogText {
                        value: String::new(),
                    })
                }
                _ => {
                    value.connection_id = Some(ConnectionId {
                        value: String::new(),
                    })
                }
            }
            assert!(validate_values(&invalid).is_err(), "{kind} case {case}");
        }
        let mut valid = owner(kind, vec![claim("receipts", StorageDirection::Write)]);
        claims_mut(&mut valid)[0].attachment = Some(attachment("receipts"));
        validate_values(&valid).unwrap();
    }
}

#[test]
fn local_port_is_valid_on_data_product_only_and_must_exist() {
    for kind in ["component", "external_system"] {
        let mut invalid = owner(kind, vec![claim("receipts", StorageDirection::Read)]);
        claims_mut(&mut invalid)[0].port_id = Some(data_product::DataPortId::new().into());
        assert!(validate_values(&invalid).is_err(), "{kind}");
    }
    let stored = prepare_entity(
        None,
        &owner(
            "data_product",
            vec![claim("receipts", StorageDirection::Read)],
        ),
    )
    .unwrap();
    for port in [
        product(&stored).inputs[0].port_id.clone(),
        product(&stored).outputs[0].port_id.clone(),
    ] {
        let mut selected = stored.clone();
        claims_mut(&mut selected)[0].port_id = port;
        data_product::validate_values(product(&selected)).unwrap();
        validate_values(&selected).unwrap();
    }
    for port in [
        Some(data_product::DataPortId::new().into()),
        Some(DataPortId {
            value: "not-a-port".into(),
        }),
    ] {
        let mut missing = stored.clone();
        claims_mut(&mut missing)[0].port_id = port;
        assert!(data_product::validate_values(product(&missing)).is_err());
        assert!(validate_values(&missing).is_err());
    }
}

#[test]
fn retarget_rewrites_unattached_stores_and_pins_attached_ones() {
    for kind in OWNERS {
        let mut plain = owner(
            kind,
            vec![
                claim("receipts", StorageDirection::Read),
                claim("audit", StorageDirection::Read),
            ],
        );
        let mut attached = owner(kind, vec![claim("receipts", StorageDirection::Write)]);
        claims_mut(&mut attached)[0].attachment = Some(attachment("receipts"));
        plain = prepare_entity(None, &plain).unwrap();
        attached = prepare_entity(None, &attached).unwrap();
        let paths = retarget_refs(
            &mut plain,
            EntityKind::DataStore,
            &id("receipts"),
            &id("fresh"),
        );
        assert_eq!(paths, ["storage_claims[0].store"], "{kind}");
        let claims = storage_claims::claims(&plain);
        assert!(claims[0].connection_id.is_none() && claims[1].connection_id.is_some());
        let before = attached.clone();
        assert!(retarget_refs(
            &mut attached,
            EntityKind::DataStore,
            &id("receipts"),
            &id("fresh")
        )
        .is_empty());
        assert_eq!(attached, before);
        let refs: Vec<_> = outbound_refs(&before)
            .into_iter()
            .filter(|r| r.from_field.starts_with("storage_claims["))
            .map(|r| r.from_field)
            .collect();
        assert_eq!(
            refs,
            [
                "storage_claims[0].store",
                "storage_claims[0].attachment.owner"
            ],
            "{kind}"
        );
    }
}

#[test]
fn claim_count_and_owner_size_are_bounded() {
    for kind in ["component", "external_system"] {
        let mut value = owner(kind, vec![]);
        claims_mut(&mut value).resize(MAX_CLAIMS, claim("receipts", StorageDirection::Read));
        validate_values(&value).unwrap();
        claims_mut(&mut value).push(claim("receipts", StorageDirection::Read));
        assert!(validate_values(&value).is_err(), "{kind}");
    }
    let mut large = owner("component", vec![claim("receipts", StorageDirection::Read)]);
    let Some(entity::Kind::Component(value)) = &mut large.kind else {
        panic!("component")
    };
    value.doc = "x".repeat(storage_claims::MAX_ENCODED_BYTES);
    assert!(validate_values(&large).is_err());
}

#[test]
fn restoring_a_removed_port_remaps_the_claim_and_renews_its_identity() {
    let mut authored = owner(
        "data_product",
        vec![claim("receipts", StorageDirection::Read)],
    );
    let stored = prepare_entity(None, &authored).unwrap();
    claims_mut(&mut authored)[0].port_id = product(&stored).inputs[0].port_id.clone();
    let base = {
        let mut with_port = stored.clone();
        claims_mut(&mut with_port)[0].port_id = product(&stored).inputs[0].port_id.clone();
        claims_mut(&mut with_port)[0].connection_id = None;
        prepare_entity(Some(&stored), &with_port).unwrap()
    };
    let mut opposing = base.clone();
    let Some(entity::Kind::DataProduct(value)) = &mut opposing.kind else {
        panic!("product")
    };
    value.inputs.clear();
    value.storage_claims.clear();
    let opposing = prepare_entity(Some(&base), &opposing).unwrap();
    let restored = prepare_resolution(Some(&base), Some(&opposing), &base).unwrap();
    data_product::validate_values(product(&restored)).unwrap();
    assert_ne!(
        storage_claims::claims(&restored)[0].port_id,
        storage_claims::claims(&base)[0].port_id
    );
    assert_eq!(
        storage_claims::claims(&restored)[0].port_id,
        product(&restored).inputs[0].port_id
    );
    assert_ne!(
        storage_claims::claims(&restored)[0].connection_id,
        storage_claims::claims(&base)[0].connection_id
    );
    let mut dangling = base.clone();
    let Some(entity::Kind::DataProduct(value)) = &mut dangling.kind else {
        panic!("product")
    };
    value.inputs.clear();
    let outcome = prepare_entity(Some(&base), &dangling);
    assert!(outcome.is_err() || data_product::validate_values(product(&outcome.unwrap())).is_err());
}
