#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{
    connection_persistence::prepare_entity,
    data_product::validate_values,
    source_integration,
    specification::{validate_specification, MemberId},
};
use trogon_atlas_proto::*;

fn id(slug: &str) -> Id {
    Id {
        namespace: "catalog".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn member() -> ContractMemberId {
    MemberId::new().into()
}

fn schema_binding(environments: &[&str], is_default: Option<bool>) -> SchemaBinding {
    SchemaBinding {
        role: schema_binding::Role::Payload as i32,
        r#type: Some(schema_binding::Type::Schema(SchemaRef {
            id: Some(id("order-schema")),
        })),
        environments: environments
            .iter()
            .map(|value| EnvironmentName {
                value: (*value).to_string(),
            })
            .collect(),
        is_default,
        ..Default::default()
    }
}

fn specification(bindings: Vec<SchemaBinding>, channels: Vec<MessageChannelDeclaration>) -> Specification {
    let schema = member();
    Specification {
        id: Some(id("orders")),
        title: "Orders".into(),
        schemas: vec![ContractSchema {
            member_id: Some(schema),
            name: Some(MemberName {
                value: "Order".into(),
            }),
            definition: Some(SchemaDefinition {
                target: Some(schema_definition::Target::Schema(SchemaRef {
                    id: Some(id("order-schema")),
                })),
                ..Default::default()
            }),
            ..Default::default()
        }],
        messages: vec![ContractMessage {
            member_id: Some(member()),
            name: Some(MemberName {
                value: "OrderPlaced".into(),
            }),
            schemas: bindings,
            ..Default::default()
        }],
        message_channels: channels,
        ..Default::default()
    }
}

fn stored_spec(bindings: Vec<SchemaBinding>, channels: Vec<MessageChannelDeclaration>) -> Specification {
    let entity = Entity {
        kind: Some(entity::Kind::Specification(specification(bindings, channels))),
        system: None,
    };
    let stored = prepare_entity(None, &entity).unwrap();
    let Some(entity::Kind::Specification(value)) = stored.kind else {
        panic!("expected specification");
    };
    value
}

fn product(associations: Vec<ProductResourceAssociation>) -> DataProduct {
    let input: DataPortId = trogon_atlas_core::data_product::DataPortId::new().into();
    let entity = Entity {
        kind: Some(entity::Kind::DataProduct(DataProduct {
            id: Some(id("orders")),
            title: "Order data".into(),
            purpose: Some("Receipts".into()),
            inputs: vec![DataInputPort {
                port_id: Some(input.clone()),
                name: Some(PortName {
                    value: "Order fact".into(),
                }),
                data: Some(InputDataSelection {
                    target: Some(input_data_selection::Target::Event(EventRef {
                        id: Some(id("placed")),
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            outputs: vec![DataOutputPort {
                name: Some(PortName {
                    value: "Receipt".into(),
                }),
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
            resource_associations: associations,
            ..Default::default()
        })),
        system: None,
    };
    let stored = prepare_entity(None, &entity).unwrap();
    let Some(entity::Kind::DataProduct(value)) = stored.kind else {
        panic!("expected data product");
    };
    value
}

fn event_association(direction: i32, contract: Option<ProductResourceContract>) -> ProductResourceAssociation {
    ProductResourceAssociation {
        direction,
        target: Some(product_resource_association::Target::Event(EventRef {
            id: Some(id("placed")),
        })),
        contract,
        ..Default::default()
    }
}

#[test]
fn overlapping_environments_for_one_message_role_are_rejected() {
    let value = stored_spec(
        vec![
            schema_binding(&["prod", "staging"], None),
            schema_binding(&["staging"], None),
        ],
        vec![],
    );
    assert!(
        validate_specification(&value).is_err(),
        "shared environment names on one message and role must be rejected"
    );
}

#[test]
fn two_defaults_for_one_message_role_are_rejected() {
    let value = stored_spec(
        vec![
            schema_binding(&[], Some(true)),
            schema_binding(&[], Some(true)),
        ],
        vec![],
    );
    assert!(validate_specification(&value).is_err());
}

#[test]
fn default_binding_cannot_name_environments() {
    let value = stored_spec(vec![schema_binding(&["prod"], Some(true))], vec![]);
    assert!(validate_specification(&value).is_err());
}

#[test]
fn environment_names_reject_65_bytes_controls_and_intra_binding_duplicates() {
    let long = "a".repeat(65);
    assert!(validate_specification(&stored_spec(vec![schema_binding(&[&long], None)], vec![])).is_err());
    assert!(validate_specification(&stored_spec(
        vec![schema_binding(&["prod\u{0001}"], None)],
        vec![]
    ))
    .is_err());
    assert!(validate_specification(&stored_spec(
        vec![schema_binding(&["prod", "prod"], None)],
        vec![]
    ))
    .is_err());
}

#[test]
fn explicit_unspecified_delivery_mode_is_rejected_and_absence_is_kept() {
    let message = member();
    let mut declared = stored_spec(
        vec![schema_binding(&[], None)],
        vec![MessageChannelDeclaration {
            member_id: Some(message),
            channel: Some(ChannelRef {
                id: Some(id("orders")),
            }),
            delivery_mode: Some(ClaimDeliveryMode::Unspecified as i32),
            ..Default::default()
        }],
    );
    // The stored member id is assigned by prepare; point the declaration at it.
    declared.message_channels[0].member_id = declared.messages[0].member_id.clone();
    assert!(
        validate_specification(&declared).is_err(),
        "explicit delivery mode 0 must be rejected"
    );
}

#[test]
fn association_direction_and_input_contract_pin_are_rejected() {
    assert!(validate_values(&product(vec![event_association(0, None)])).is_err());
    assert!(validate_values(&product(vec![event_association(
        DataPortDirection::Input as i32,
        Some(ProductResourceContract {
            r#type: Some(product_resource_contract::Type::Schema(SchemaRef {
                id: Some(id("order-schema")),
            })),
        }),
    )]))
    .is_err());
}

#[test]
fn whole_owner_rejects_resource_associations_and_message_channels() {
    let whole = SourceFieldOwnership {
        policy: Some(source_field_ownership::Policy::WholeOwner(
            SourceWholeOwnerOwnership {},
        )),
    };
    let product = Entity {
        kind: Some(entity::Kind::DataProduct(product(vec![event_association(
            DataPortDirection::Output as i32,
            None,
        )]))),
        system: None,
    };
    assert!(source_integration::validate_ownership(&product, &whole).is_err());
    let spec = Entity {
        kind: Some(entity::Kind::Specification(stored_spec(
            vec![schema_binding(&["prod"], None)],
            vec![MessageChannelDeclaration {
                member_id: Some(member()),
                channel: Some(ChannelRef {
                    id: Some(id("orders")),
                }),
                ..Default::default()
            }],
        ))),
        system: None,
    };
    assert!(source_integration::validate_ownership(&spec, &whole).is_err());
}

#[test]
fn title_only_ownership_does_not_blanket_reject_native_content() {
    let title = SourceFieldOwnership {
        policy: Some(source_field_ownership::Policy::Fields(SourceOwnedFields {
            fields: vec![SourceOwnedField::Title as i32],
        })),
    };
    let product = Entity {
        kind: Some(entity::Kind::DataProduct(product(vec![event_association(
            DataPortDirection::Output as i32,
            None,
        )]))),
        system: None,
    };
    assert!(source_integration::validate_ownership(&product, &title).is_ok());
}
