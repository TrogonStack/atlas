#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_core::{
    connection_persistence::{prepare_entity, prepare_landing, prepare_resolution},
    connections::{connection_occurrences, resolve_connection, ConnectionId},
    data_product::{
        resolve_port, validate_values, DataPortId as PortId, MAX_ASSOCIATIONS, MAX_ENCODED_BYTES,
        MAX_PORTS,
    },
    outbound_refs, retarget_refs,
};
use trogon_atlas_proto::*;

fn id(slug: &str) -> Id {
    Id {
        namespace: "catalog".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn name(value: &str) -> PortName {
    PortName {
        value: value.into(),
    }
}
fn product(entity: &Entity) -> &DataProduct {
    let Some(entity::Kind::DataProduct(value)) = &entity.kind else {
        panic!("expected product")
    };
    value
}
fn product_mut(entity: &mut Entity) -> &mut DataProduct {
    let Some(entity::Kind::DataProduct(value)) = &mut entity.kind else {
        panic!("expected product")
    };
    value
}
fn authored() -> Entity {
    let input: DataPortId = PortId::new().into();
    Entity {
        kind: Some(entity::Kind::DataProduct(DataProduct {
            id: Some(id("orders")),
            title: "Order data".into(),
            purpose: Some("Receipts for readers".into()),
            inputs: vec![DataInputPort {
                port_id: Some(input.clone()),
                name: Some(name("Order fact")),
                data: Some(InputDataSelection {
                    target: Some(input_data_selection::Target::Event(EventRef {
                        id: Some(id("placed")),
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            outputs: vec![DataOutputPort {
                name: Some(name("Receipt")),
                data: Some(OutputDataSelection {
                    read_model: Some(ReadModelRef {
                        id: Some(id("receipt")),
                    }),
                    ..Default::default()
                }),
                derived_from: Some(DeclaredLineage {
                    inputs: vec![LineageInput {
                        input_port_id: Some(input),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        system: None,
    }
}
fn stored() -> Entity {
    prepare_entity(None, &authored()).unwrap()
}
fn port_ref(
    entity: &Entity,
    direction: DataPortDirection,
    port_id: Option<DataPortId>,
) -> DataPortRef {
    DataPortRef {
        product: Some(DataProductRef {
            id: product(entity).id.clone(),
        }),
        owner_uid: entity.system.as_ref().unwrap().uid.clone(),
        direction: direction as i32,
        port_id,
    }
}
fn connection_ref(kind: EntityKind, owner: &str) -> ConnectionRef {
    ConnectionRef {
        owner: Some(EntityRef {
            kind: kind as i32,
            id: Some(id(owner)),
        }),
        owner_uid: "selected-owner".into(),
        id: Some(ConnectionId::new().into()),
    }
}
fn operation_access(provider: bool) -> OutputAccess {
    OutputAccess {
        selection: Some(output_access::Selection::Operation(OperationAccess {
            operation: Some(ContractMemberRef {
                specification: Some(SpecificationRef {
                    id: Some(id("http")),
                }),
                owner_uid: "selected-owner".into(),
                kind: ContractMemberKind::Operation as i32,
                member_id: Some(trogon_atlas_core::specification::MemberId::new().into()),
                expected_digest: None,
            }),
            response: Some(connection_ref(EntityKind::Specification, "http")),
            provider_exposure: provider.then(|| connection_ref(EntityKind::Specification, "http")),
        })),
        ..Default::default()
    }
}
fn with_access() -> Entity {
    let mut value = authored();
    product_mut(&mut value).outputs[0].access = vec![operation_access(false)];
    prepare_entity(None, &value).unwrap()
}

#[test]
fn new_kind_roundtrips_and_revision_floors_are_coherent() {
    assert_eq!(EntityKind::DataProduct as i32, 33);
    assert_eq!(
        canonical::parse_kind("DATA_PRODUCT"),
        Some(EntityKind::DataProduct)
    );
    assert_eq!(CONTRACT_REVISION, 13);
    assert_eq!(MIN_CLIENT_CONTRACT_REVISION, CONTRACT_REVISION);
    assert_eq!(MIN_SERVER_CONTRACT_REVISION, CONTRACT_REVISION);
    let value = stored();
    assert_eq!(
        Entity::decode(value.encode_to_vec().as_slice()).unwrap(),
        value
    );
    assert_eq!(
        trogon_atlas_core::entity_kind(&value),
        Some(EntityKind::DataProduct)
    );
}

#[test]
fn strict_port_uuid_syntax_and_complete_stored_set_before_resolution() {
    for invalid in [
        "",
        "00000000-0000-0000-0000-000000000000",
        "ABCDEFAB-CDEF-ABCD-EFAB-CDEFABCDEFAB",
        "abcdefabcdefabcdefabcdefabcdefab",
    ] {
        assert!(invalid.parse::<PortId>().is_err());
    }
    let value = stored();
    let reference = port_ref(
        &value,
        DataPortDirection::Output,
        product(&value).outputs[0].port_id.clone(),
    );
    assert!(resolve_port(&value, &reference).is_ok());
    for malformed in [
        None,
        Some(DataPortId {
            value: "invalid".into(),
        }),
        reference.port_id.clone(),
    ] {
        let mut invalid = value.clone();
        product_mut(&mut invalid).inputs[0].port_id = malformed;
        assert!(resolve_port(&invalid, &reference).is_err());
    }
}

#[test]
fn port_reference_rejects_stale_incarnation_wrong_direction_key_or_owner_kind() {
    let value = stored();
    let reference = port_ref(
        &value,
        DataPortDirection::Output,
        product(&value).outputs[0].port_id.clone(),
    );
    for candidate in [
        DataPortRef {
            owner_uid: "stale".into(),
            ..reference.clone()
        },
        DataPortRef {
            direction: DataPortDirection::Input as i32,
            ..reference.clone()
        },
        DataPortRef {
            direction: 999,
            ..reference.clone()
        },
        DataPortRef {
            product: Some(DataProductRef {
                id: Some(id("other")),
            }),
            ..reference.clone()
        },
    ] {
        assert!(resolve_port(&value, &candidate).is_err());
    }
    let other = Entity {
        kind: Some(entity::Kind::ReadModel(ReadModel {
            id: Some(id("orders")),
            ..Default::default()
        })),
        system: value.system.clone(),
    };
    assert!(resolve_port(&other, &reference).is_err());
    let recreated = prepare_entity(None, &value).unwrap();
    assert!(resolve_port(&recreated, &reference).is_err());
}

#[test]
fn independent_port_and_connection_selectors_do_not_interchange() {
    let value = stored();
    let occurrence = connection_occurrences(&value).unwrap()[0].clone();
    let port = product(&value).inputs[0].port_id.clone().unwrap();
    assert_ne!(port.value, occurrence.id.unwrap().to_string());
    let wrong_port = port_ref(
        &value,
        DataPortDirection::Input,
        Some(DataPortId {
            value: occurrence.id.unwrap().to_string(),
        }),
    );
    assert!(resolve_port(&value, &wrong_port).is_err());
    let wrong_connection = ConnectionRef {
        owner: Some(EntityRef {
            kind: EntityKind::DataProduct as i32,
            id: product(&value).id.clone(),
        }),
        owner_uid: value.system.as_ref().unwrap().uid.clone(),
        id: Some(trogon_atlas_proto::ConnectionId { value: port.value }),
    };
    assert!(resolve_connection(&value, &wrong_connection).is_err());
}

#[test]
fn repeated_occurrences_reorder_rename_and_native_export_preserve_identity_and_presence() {
    let mut value = authored();
    let mut repeated = product(&value).outputs[0].clone();
    repeated.name = Some(name("Receipt alternate"));
    repeated.derived_from = Some(DeclaredLineage::default());
    product_mut(&mut value).outputs.push(repeated);
    let first = operation_access(false);
    product_mut(&mut value).outputs[0].access = vec![first.clone(), first];
    let value = prepare_entity(None, &value).unwrap();
    validate_values(product(&value)).unwrap();
    let output = &product(&value).outputs;
    assert_ne!(output[0].port_id, output[1].port_id);
    assert_ne!(
        output[0].access[0].connection_id,
        output[0].access[1].connection_id
    );
    let mut reordered = value.clone();
    product_mut(&mut reordered).outputs.reverse();
    product_mut(&mut reordered).outputs[1].access.reverse();
    product_mut(&mut reordered).outputs[1].name = Some(name("Renamed"));
    product_mut(&mut reordered).outputs[1].doc = Some("documented".into());
    let edited = prepare_entity(Some(&value), &reordered).unwrap();
    assert_eq!(product(&edited), product(&reordered));
    let json = trogon_atlas_core::transcode::entity_json_value(&edited).unwrap();
    let roundtrip = trogon_atlas_core::transcode::message_from_json::<Entity>(
        "trogonatlas.eventmodel.v1alpha1.Entity",
        &json.to_string(),
    )
    .unwrap();
    assert_eq!(prepare_entity(Some(&edited), &roundtrip).unwrap(), edited);
    assert_eq!(
        product(&edited).outputs[0]
            .derived_from
            .as_ref()
            .unwrap()
            .inputs,
        []
    );
}

#[test]
fn unchanged_idless_manifest_is_noop_but_edited_id_stripping_is_rejected() {
    let mut value = authored();
    product_mut(&mut value).inputs.clear();
    product_mut(&mut value).outputs[0].derived_from = None;
    let persisted = prepare_entity(None, &value).unwrap();
    assert_eq!(prepare_entity(Some(&persisted), &value).unwrap(), persisted);
    product_mut(&mut value).title = "Edited".into();
    assert!(prepare_entity(Some(&persisted), &value).is_err());
    let mut stripped = persisted.clone();
    product_mut(&mut stripped).outputs[0].port_id = None;
    product_mut(&mut stripped).title = "Edited".into();
    assert!(prepare_entity(Some(&persisted), &stripped).is_err());
}

#[test]
fn port_move_direction_and_unknown_caller_ids_are_rejected() {
    let value = stored();
    let mut moved = value.clone();
    product_mut(&mut moved).outputs[0].port_id = product(&value).inputs[0].port_id.clone();
    product_mut(&mut moved).inputs.clear();
    product_mut(&mut moved).outputs[0].derived_from = None;
    assert!(prepare_entity(Some(&value), &moved).is_err());
    let mut unknown = value.clone();
    product_mut(&mut unknown).outputs[0].port_id = Some(PortId::new().into());
    assert!(prepare_entity(Some(&value), &unknown).is_err());
}

#[test]
fn association_fingerprints_include_port_kind_key_type_pin_and_provider_presence() {
    let value = with_access();
    let mut moved = value.clone();
    let mut other = product(&value).outputs[0].clone();
    other.port_id = None;
    other.name = Some(name("Other"));
    other.access.clear();
    other.data = None;
    other.derived_from = None;
    product_mut(&mut moved).outputs.push(other);
    let moved = prepare_entity(Some(&value), &moved).unwrap();
    let mut proposed = moved.clone();
    let association = product_mut(&mut proposed).outputs[0].access.remove(0);
    product_mut(&mut proposed).outputs[1]
        .access
        .push(association);
    assert!(prepare_entity(Some(&moved), &proposed).is_err());
    let mut retargeted = value.clone();
    product_mut(&mut retargeted).inputs[0]
        .data
        .as_mut()
        .unwrap()
        .target = Some(input_data_selection::Target::ReadModel(ReadModelRef {
        id: Some(id("placed")),
    }));
    assert!(prepare_entity(Some(&value), &retargeted).is_err());
    let mut provider = value.clone();
    let Some(output_access::Selection::Operation(operation)) =
        product_mut(&mut provider).outputs[0].access[0]
            .selection
            .as_mut()
    else {
        panic!("operation")
    };
    operation.provider_exposure = Some(connection_ref(EntityKind::Specification, "http"));
    assert!(prepare_entity(Some(&value), &provider).is_err());
    product_mut(&mut provider).outputs[0].access[0].connection_id = None;
    let provider = prepare_entity(Some(&value), &provider).unwrap();
    assert_eq!(
        product(&provider).outputs[0].port_id,
        product(&value).outputs[0].port_id
    );
    assert_ne!(
        product(&provider).outputs[0].access[0].connection_id,
        product(&value).outputs[0].access[0].connection_id
    );
}

#[test]
fn retarget_input_with_fresh_association_preserves_local_lineage() {
    let value = stored();
    let mut proposed = value.clone();
    let data = product_mut(&mut proposed).inputs[0].data.as_mut().unwrap();
    data.target = Some(input_data_selection::Target::ReadModel(ReadModelRef {
        id: Some(id("request")),
    }));
    data.connection_id = None;
    let edited = prepare_entity(Some(&value), &proposed).unwrap();
    validate_values(product(&edited)).unwrap();
    assert_eq!(
        product(&edited).inputs[0].port_id,
        product(&value).inputs[0].port_id
    );
    assert_eq!(
        product(&edited).outputs[0].derived_from,
        product(&value).outputs[0].derived_from
    );
}

#[test]
fn removal_and_readdition_do_not_revive_port_or_association_identity() {
    let value = stored();
    let mut pruned = value.clone();
    product_mut(&mut pruned).inputs.clear();
    product_mut(&mut pruned).outputs[0].derived_from = None;
    let pruned = prepare_entity(Some(&value), &pruned).unwrap();
    assert!(prepare_entity(Some(&pruned), &value).is_err());
    let mut readded = pruned.clone();
    let mut input = product(&value).inputs[0].clone();
    input.port_id = None;
    input.data.as_mut().unwrap().connection_id = None;
    product_mut(&mut readded).inputs.push(input);
    let readded = prepare_entity(Some(&pruned), &readded).unwrap();
    assert_ne!(
        product(&value).inputs[0].port_id,
        product(&readded).inputs[0].port_id
    );
    assert_ne!(
        product(&value).inputs[0]
            .data
            .as_ref()
            .unwrap()
            .connection_id,
        product(&readded).inputs[0]
            .data
            .as_ref()
            .unwrap()
            .connection_id
    );
}

#[test]
fn branch_removal_blocks_revival_and_explicit_restoration_rewrites_local_lineage() {
    let base = stored();
    let mut opposing = base.clone();
    product_mut(&mut opposing).inputs.clear();
    product_mut(&mut opposing).outputs[0].derived_from = None;
    let opposing = prepare_entity(Some(&base), &opposing).unwrap();
    assert!(prepare_landing(Some(&base), Some(&opposing), &base).is_err());
    let restored = prepare_resolution(Some(&base), Some(&opposing), &base).unwrap();
    validate_values(product(&restored)).unwrap();
    assert_eq!(
        restored.system.as_ref().unwrap().uid,
        base.system.as_ref().unwrap().uid
    );
    assert_ne!(
        product(&restored).inputs[0].port_id,
        product(&base).inputs[0].port_id
    );
    assert_eq!(
        product(&restored).inputs[0].port_id,
        product(&restored).outputs[0]
            .derived_from
            .as_ref()
            .unwrap()
            .inputs[0]
            .input_port_id
    );
    assert_ne!(
        product(&restored).inputs[0]
            .data
            .as_ref()
            .unwrap()
            .connection_id,
        product(&base).inputs[0]
            .data
            .as_ref()
            .unwrap()
            .connection_id
    );
    assert_ne!(
        product(&restored).outputs[0]
            .derived_from
            .as_ref()
            .unwrap()
            .inputs[0]
            .connection_id,
        product(&base).outputs[0]
            .derived_from
            .as_ref()
            .unwrap()
            .inputs[0]
            .connection_id
    );
    assert_eq!(
        product(&restored).outputs[0].data,
        product(&base).outputs[0].data
    );
    let old = port_ref(
        &base,
        DataPortDirection::Input,
        product(&base).inputs[0].port_id.clone(),
    );
    assert!(resolve_port(&restored, &old).is_err());
}

#[test]
fn branch_only_addition_and_whole_owner_keep_ours_preserve_identities() {
    let base = stored();
    let mut branch = base.clone();
    product_mut(&mut branch).outputs.push(DataOutputPort {
        name: Some(name("Draft")),
        ..Default::default()
    });
    let branch = prepare_entity(Some(&base), &branch).unwrap();
    let landed = prepare_landing(Some(&base), Some(&base), &branch).unwrap();
    assert_eq!(product(&landed), product(&branch));
    assert_eq!(
        prepare_resolution(Some(&base), None, &branch).unwrap(),
        branch
    );
}

#[test]
fn clone_keeps_local_selections_and_external_pins_but_changes_owner_incarnation() {
    let value = with_access();
    let mut copy = value.clone();
    product_mut(&mut copy).id = Some(id("orders-copy"));
    let copy = prepare_entity(None, &copy).unwrap();
    validate_values(product(&copy)).unwrap();
    assert_eq!(product(&copy).inputs, product(&value).inputs);
    assert_eq!(product(&copy).outputs, product(&value).outputs);
    assert_ne!(
        copy.system.as_ref().unwrap().uid,
        value.system.as_ref().unwrap().uid
    );
    let original = port_ref(
        &value,
        DataPortDirection::Output,
        product(&value).outputs[0].port_id.clone(),
    );
    assert!(resolve_port(&copy, &original).is_err());
}

#[test]
fn invalid_local_lineage_and_access_without_data_fail() {
    let value = stored();
    for selection in [
        None,
        Some(PortId::new().into()),
        product(&value).outputs[0].port_id.clone(),
    ] {
        let mut invalid = value.clone();
        product_mut(&mut invalid).outputs[0]
            .derived_from
            .as_mut()
            .unwrap()
            .inputs[0]
            .input_port_id = selection;
        assert!(validate_values(product(&invalid)).is_err());
    }
    let mut duplicate = value.clone();
    let lineage = product_mut(&mut duplicate).outputs[0]
        .derived_from
        .as_mut()
        .unwrap();
    let mut input = lineage.inputs[0].clone();
    input.connection_id = Some(ConnectionId::new().into());
    lineage.inputs.push(input);
    assert!(validate_values(product(&duplicate)).is_err());
    let mut invalid = with_access();
    product_mut(&mut invalid).outputs[0].data = None;
    assert!(validate_values(product(&invalid)).is_err());
    let mut missing_input = value.clone();
    product_mut(&mut missing_input).inputs.clear();
    assert!(validate_values(product(&missing_input)).is_err());
}

#[test]
fn undeclared_ports_preserve_absent_empty_and_nonempty_lineage_without_inference() {
    let mut value = stored();
    product_mut(&mut value).inputs[0].data = None;
    product_mut(&mut value).outputs[0].data = None;
    validate_values(product(&value)).unwrap();
    for declaration in [
        None,
        Some(DeclaredLineage::default()),
        product(&value).outputs[0].derived_from.clone(),
    ] {
        let mut proposed = value.clone();
        product_mut(&mut proposed).outputs[0].derived_from = declaration.clone();
        let persisted = prepare_entity(Some(&value), &proposed).unwrap();
        validate_values(product(&persisted)).unwrap();
        assert_eq!(product(&persisted).outputs[0].derived_from, declaration);
    }
}

#[test]
fn value_validation_rejects_blank_values_duplicate_names_ids_missing_oneofs_and_unknown_kinds() {
    let value = stored();
    let mut cases = Vec::new();
    let mut bad = value.clone();
    product_mut(&mut bad).title = " ".into();
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).purpose = Some(" ".into());
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).outputs.clear();
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).inputs[0].name = None;
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).inputs[0]
        .data
        .as_mut()
        .unwrap()
        .target = None;
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).outputs[0].contract = Some(OutputContract {
        connection_id: Some(ConnectionId::new().into()),
        r#type: None,
    });
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).outputs[0].access.push(OutputAccess {
        connection_id: Some(ConnectionId::new().into()),
        ..Default::default()
    });
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).outputs[0]
        .data
        .as_mut()
        .unwrap()
        .read_model = None;
    cases.push(bad);
    let mut bad = value.clone();
    product_mut(&mut bad).outputs[0]
        .data
        .as_mut()
        .unwrap()
        .connection_id = product(&value).inputs[0]
        .data
        .as_ref()
        .unwrap()
        .connection_id
        .clone();
    cases.push(bad);
    for bad in cases {
        assert!(validate_values(product(&bad)).is_err());
    }
    let mut repeated_name = value.clone();
    let mut output = product(&value).outputs[0].clone();
    output.port_id = None;
    output.data = None;
    output.derived_from = None;
    product_mut(&mut repeated_name).outputs.push(output);
    let repeated_name = prepare_entity(None, &repeated_name).unwrap();
    assert!(validate_values(product(&repeated_name)).is_err());
    let mut allowed = value.clone();
    product_mut(&mut allowed).purpose = None;
    product_mut(&mut allowed).outputs[0].name = product(&allowed).inputs[0].name.clone();
    validate_values(product(&allowed)).unwrap();
    let mut unknown = with_access();
    let Some(output_access::Selection::Operation(access)) = product_mut(&mut unknown).outputs[0]
        .access[0]
        .selection
        .as_mut()
    else {
        panic!("operation")
    };
    access.operation.as_mut().unwrap().kind = 999;
    assert!(validate_values(product(&unknown)).is_err());
}

#[test]
fn independent_limits_accept_boundary_and_reject_excess() {
    let mut value = stored();
    product_mut(&mut value).inputs.clear();
    product_mut(&mut value).outputs.clear();
    for index in 0..MAX_PORTS {
        product_mut(&mut value).outputs.push(DataOutputPort {
            port_id: Some(PortId::new().into()),
            name: Some(name(&format!("Port {index}"))),
            ..Default::default()
        });
    }
    validate_values(product(&value)).unwrap();
    product_mut(&mut value).outputs.push(DataOutputPort {
        port_id: Some(PortId::new().into()),
        name: Some(name("excess")),
        ..Default::default()
    });
    assert!(validate_values(product(&value))
        .unwrap_err()
        .0
        .contains("port limit"));
    let mut value = stored();
    let template = OutputAccess {
        selection: Some(output_access::Selection::Store(StoreAccess {
            store: Some(DataStoreRef {
                id: Some(id("cache")),
            }),
            attachment: Some(connection_ref(EntityKind::DataStore, "cache")),
        })),
        ..Default::default()
    };
    for _ in 0..MAX_ASSOCIATIONS - 3 {
        let mut access = template.clone();
        access.connection_id = Some(ConnectionId::new().into());
        product_mut(&mut value).outputs[0].access.push(access);
    }
    assert!(product(&value).encoded_len() < MAX_ENCODED_BYTES);
    validate_values(product(&value)).unwrap();
    let mut extra = template;
    extra.connection_id = Some(ConnectionId::new().into());
    product_mut(&mut value).outputs[0].access.push(extra);
    assert!(validate_values(product(&value))
        .unwrap_err()
        .0
        .contains("association limit"));
    let mut value = stored();
    product_mut(&mut value).doc = Some("x".repeat(MAX_ENCODED_BYTES));
    let over = product(&value).encoded_len() - MAX_ENCODED_BYTES;
    product_mut(&mut value)
        .doc
        .as_mut()
        .unwrap()
        .truncate(MAX_ENCODED_BYTES - over);
    assert_eq!(product(&value).encoded_len(), MAX_ENCODED_BYTES);
    validate_values(product(&value)).unwrap();
    product_mut(&mut value).doc.as_mut().unwrap().push('x');
    assert!(validate_values(product(&value))
        .unwrap_err()
        .0
        .contains("encoded size"));
}

#[test]
fn nested_reference_walk_preserves_occurrences_and_guards_pinned_selection_retarget() {
    let mut value = authored();
    let output = &mut product_mut(&mut value).outputs[0];
    output.contract = Some(OutputContract {
        r#type: Some(output_contract::Type::ContractType(ContractTypeRef {
            schema: Some(ContractMemberRef {
                specification: Some(SpecificationRef {
                    id: Some(id("types")),
                }),
                owner_uid: "type-owner".into(),
                kind: ContractMemberKind::Schema as i32,
                member_id: Some(trogon_atlas_core::specification::MemberId::new().into()),
                expected_digest: None,
            }),
            expected_artifact_id: Some(trogon_atlas_core::specification::MemberId::new().into()),
            expected_digest: Some(ContentDigest {
                sha256: "a".repeat(64),
            }),
            expected_selector: None,
        })),
        ..Default::default()
    });
    output.access = vec![
        operation_access(true),
        OutputAccess {
            selection: Some(output_access::Selection::Store(StoreAccess {
                store: Some(DataStoreRef {
                    id: Some(id("cache")),
                }),
                attachment: Some(connection_ref(EntityKind::DataStore, "cache")),
            })),
            ..Default::default()
        },
        OutputAccess {
            selection: Some(output_access::Selection::Channel(ChannelAccess {
                channel: Some(ChannelRef {
                    id: Some(id("bus")),
                }),
                binding: Some(connection_ref(EntityKind::Channel, "bus")),
                observation: Some(connection_ref(EntityKind::AutomationSlice, "observer")),
            })),
            ..Default::default()
        },
    ];
    let mut value = prepare_entity(None, &value).unwrap();
    validate_values(product(&value)).unwrap();
    let refs = outbound_refs(&value);
    for key in [
        "placed", "receipt", "types", "http", "cache", "bus", "observer",
    ] {
        assert!(refs.iter().any(|r| r.to_id.slug == key), "missing {key}");
    }
    assert_eq!(refs.iter().filter(|r| r.to_id.slug == "http").count(), 3);
    assert_eq!(refs.iter().filter(|r| r.to_id.slug == "cache").count(), 2);
    for (kind, key) in [
        (EntityKind::Specification, "types"),
        (EntityKind::Specification, "http"),
        (EntityKind::DataStore, "cache"),
        (EntityKind::Channel, "bus"),
        (EntityKind::AutomationSlice, "observer"),
    ] {
        assert_eq!(
            retarget_refs(&mut value, kind, &id(key), &id("replacement")),
            Vec::<String>::new()
        );
    }
    assert_eq!(
        retarget_refs(
            &mut value,
            EntityKind::ReadModel,
            &id("receipt"),
            &id("receipt-v2")
        )
        .len(),
        1
    );
    assert_eq!(
        product(&value).outputs[0]
            .data
            .as_ref()
            .unwrap()
            .read_model
            .as_ref()
            .unwrap()
            .id,
        Some(id("receipt-v2"))
    );
}

#[test]
fn contract_association_fingerprint_preserves_full_pin_and_selector_presence() {
    let mut value = authored();
    product_mut(&mut value).outputs[0].contract = Some(OutputContract {
        r#type: Some(output_contract::Type::ContractType(ContractTypeRef {
            schema: Some(ContractMemberRef {
                specification: Some(SpecificationRef {
                    id: Some(id("types")),
                }),
                owner_uid: "type-owner".into(),
                kind: ContractMemberKind::Schema as i32,
                member_id: Some(trogon_atlas_core::specification::MemberId::new().into()),
                expected_digest: None,
            }),
            expected_artifact_id: Some(trogon_atlas_core::specification::MemberId::new().into()),
            expected_digest: Some(ContentDigest {
                sha256: "b".repeat(64),
            }),
            expected_selector: None,
        })),
        ..Default::default()
    });
    let value = prepare_entity(None, &value).unwrap();
    for change in 0..5 {
        let mut proposed = value.clone();
        let Some(output_contract::Type::ContractType(pin)) = product_mut(&mut proposed).outputs[0]
            .contract
            .as_mut()
            .unwrap()
            .r#type
            .as_mut()
        else {
            panic!("type pin")
        };
        match change {
            0 => pin.schema.as_mut().unwrap().owner_uid = "recreated".into(),
            1 => {
                pin.expected_artifact_id =
                    Some(trogon_atlas_core::specification::MemberId::new().into());
            }
            2 => pin.expected_digest.as_mut().unwrap().sha256 = "c".repeat(64),
            3 => {
                pin.expected_selector = Some(OpaqueTypeSelector {
                    syntax: "pointer".into(),
                    value: "/Receipt".into(),
                });
            }
            _ => {
                pin.schema
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
        }
        assert!(prepare_entity(Some(&value), &proposed).is_err());
    }
}

#[test]
fn persisted_identity_corruption_cannot_bypass_noop_or_branch_preparation() {
    let value = stored();
    let mut incomplete = value.clone();
    product_mut(&mut incomplete).inputs[0].port_id = None;
    assert!(prepare_entity(Some(&incomplete), &incomplete).is_err());
    assert!(prepare_landing(Some(&value), Some(&incomplete), &value).is_err());
    assert!(prepare_landing(Some(&incomplete), Some(&value), &value).is_err());
    assert!(prepare_resolution(Some(&value), Some(&value), &incomplete).is_err());
}

#[test]
fn lineage_endpoint_change_requires_fresh_association_while_port_survives() {
    let mut value = authored();
    let mut repeated = product(&value).inputs[0].clone();
    repeated.port_id = Some(PortId::new().into());
    repeated.name = Some(name("Repeated fact"));
    product_mut(&mut value).inputs.push(repeated);
    let value = prepare_entity(None, &value).unwrap();
    let mut changed = value.clone();
    let selected = product(&value).inputs[1].port_id.clone();
    let lineage = &mut product_mut(&mut changed).outputs[0]
        .derived_from
        .as_mut()
        .unwrap()
        .inputs[0];
    lineage.input_port_id = selected;
    assert!(prepare_entity(Some(&value), &changed).is_err());
    product_mut(&mut changed).outputs[0]
        .derived_from
        .as_mut()
        .unwrap()
        .inputs[0]
        .connection_id = None;
    let updated = prepare_entity(Some(&value), &changed).unwrap();
    validate_values(product(&updated)).unwrap();
    assert_eq!(
        product(&updated).outputs[0].port_id,
        product(&value).outputs[0].port_id
    );
    assert_ne!(
        product(&updated).outputs[0].derived_from,
        product(&value).outputs[0].derived_from
    );
}

#[test]
fn selected_library_type_and_connection_owner_pins_are_identity_significant() {
    let mut value = authored();
    product_mut(&mut value).outputs[0].contract = Some(OutputContract {
        r#type: Some(output_contract::Type::LibraryType(TypeLibraryTypeRef {
            library: Some(TypeLibraryRef {
                id: Some(id("receipts")),
            }),
            message: Some(QualifiedProtoName {
                value: "receipts.Receipt".into(),
            }),
        })),
        ..Default::default()
    });
    product_mut(&mut value).outputs[0].access = vec![operation_access(true)];
    let value = prepare_entity(None, &value).unwrap();
    validate_values(product(&value)).unwrap();
    for change in 0..4 {
        let mut proposed = value.clone();
        if change == 0 {
            let Some(output_contract::Type::LibraryType(pin)) = product_mut(&mut proposed).outputs
                [0]
            .contract
            .as_mut()
            .unwrap()
            .r#type
            .as_mut() else {
                panic!("library")
            };
            pin.message.as_mut().unwrap().value = "receipts.Other".into();
        } else {
            let Some(output_access::Selection::Operation(access)) =
                product_mut(&mut proposed).outputs[0].access[0]
                    .selection
                    .as_mut()
            else {
                panic!("operation")
            };
            match change {
                1 => access.response.as_mut().unwrap().id = Some(ConnectionId::new().into()),
                2 => access.response.as_mut().unwrap().owner_uid = "recreated".into(),
                _ => access.provider_exposure = None,
            }
        }
        assert!(prepare_entity(Some(&value), &proposed).is_err());
    }
}
