#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{
    connection_persistence::{prepare_entity, prepare_landing, prepare_resolution},
    connections::{connection_occurrences, resolve_connection},
    outbound_refs, retarget_refs,
    specification::{
        content_digest, is_api, resolve_member, resolve_type, validate_specification, MemberId,
    },
};
use trogon_atlas_proto::*;

fn id(slug: &str) -> Id {
    Id {
        namespace: "contracts".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn name(value: &str) -> MemberName {
    MemberName {
        value: value.into(),
    }
}
fn spec(entity: &Entity) -> &Specification {
    let Some(entity::Kind::Specification(s)) = &entity.kind else {
        panic!("expected Specification")
    };
    s
}
fn spec_mut(entity: &mut Entity) -> &mut Specification {
    let Some(entity::Kind::Specification(s)) = &mut entity.kind else {
        panic!("expected Specification")
    };
    s
}
fn member() -> ContractMemberId {
    MemberId::new().into()
}

fn fixture() -> Entity {
    let artifact = Some(member());
    let schema = member();
    let message = Some(member());
    let operation = Some(member());
    let digest = content_digest("{\r\n  \"type\": \"object\"\r\n}\r\n");
    let entity = Entity {
        kind: Some(entity::Kind::Specification(Specification {
            id: Some(id("orders")),
            title: "Orders contract".into(),
            artifacts: vec![ContractArtifact {
                member_id: artifact.clone(),
                name: Some(name("source")),
                path: Some(ArtifactPath {
                    value: "contracts/orders.json".into(),
                }),
                format: Some(ContractFormat {
                    kind: Some(contract_format::Kind::Known(
                        contract_format::Known::JsonSchema as i32,
                    )),
                }),
                media_type: Some(MediaType {
                    value: "application/schema+json".into(),
                }),
                content: "{\r\n  \"type\": \"object\"\r\n}\r\n".into(),
                digest: Some(digest.clone()),
                ..Default::default()
            }],
            schemas: vec![ContractSchema {
                member_id: Some(schema.clone()),
                name: Some(name("Order")),
                definition: Some(SchemaDefinition {
                    target: Some(schema_definition::Target::Artifact(
                        LocalArtifactSelection {
                            artifact_id: artifact,
                            expected_digest: Some(digest),
                        },
                    )),
                    selector: Some(OpaqueTypeSelector {
                        syntax: "json-pointer".into(),
                        value: "/Order".into(),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            messages: vec![ContractMessage {
                member_id: message.clone(),
                name: Some(name("receipt")),
                schemas: vec![SchemaBinding {
                    role: schema_binding::Role::Payload as i32,
                    r#type: Some(schema_binding::Type::LocalSchema(schema)),
                    ..Default::default()
                }],
                mapping: Some(MessageReadModelMapping {
                    read_model: Some(ReadModelRef {
                        id: Some(id("receipt")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            operations: vec![ContractOperation {
                member_id: operation.clone(),
                name: Some(name("submit")),
                messages: vec![OperationMessage {
                    message_id: message,
                    role: operation_message::Role::Response as i32,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            exposures: vec![ProviderExposure {
                provider: Some(ProviderTarget {
                    target: Some(provider_target::Target::Component(ComponentRef {
                        id: Some(id("gateway")),
                    })),
                }),
                operation_id: operation.clone(),
                ..Default::default()
            }],
            usages: vec![ApiUsage {
                caller: Some(ProviderTarget {
                    target: Some(provider_target::Target::Component(ComponentRef {
                        id: Some(id("checkout")),
                    })),
                }),
                operation_id: operation,
                ..Default::default()
            }],
            ..Default::default()
        })),
        system: None,
    };
    let mut entity = prepare_entity(None, &entity).unwrap();
    let exposure_id = spec(&entity).exposures[0].connection_id.clone();
    spec_mut(&mut entity).usages[0].exposure_id = exposure_id;
    spec_mut(&mut entity).usages[0].connection_id = None;
    let entity = prepare_entity(None, &entity).unwrap();
    validate_specification(spec(&entity)).unwrap();
    entity
}

fn type_pin(entity: &Entity) -> ContractTypeRef {
    let s = spec(entity);
    ContractTypeRef {
        schema: Some(ContractMemberRef {
            specification: Some(SpecificationRef { id: s.id.clone() }),
            owner_uid: entity.system.as_ref().unwrap().uid.clone(),
            kind: ContractMemberKind::Schema as i32,
            member_id: s.schemas[0].member_id.clone(),
            expected_digest: None,
        }),
        expected_artifact_id: s.artifacts[0].member_id.clone(),
        expected_digest: s.artifacts[0].digest.clone(),
        expected_selector: s.schemas[0].definition.as_ref().unwrap().selector.clone(),
    }
}

#[test]
fn generated_contract_revision_and_canonical_kind_are_complete() {
    assert_eq!(
        (
            CONTRACT_REVISION,
            MIN_SERVER_CONTRACT_REVISION,
            MIN_CLIENT_CONTRACT_REVISION
        ),
        (19, 19, 19)
    );
    assert_eq!(
        canonical::parse_kind("specification"),
        Some(EntityKind::Specification)
    );
    assert!(canonical::ALL_KINDS.contains(&EntityKind::Specification));
}

#[test]
fn local_graph_is_native_and_classification_is_derived() {
    let entity = fixture();
    assert!(is_api(spec(&entity)));
    assert_eq!(connection_occurrences(&entity).unwrap().len(), 6);
    for occurrence in connection_occurrences(&entity).unwrap() {
        let selected = ConnectionRef {
            owner: Some(EntityRef {
                kind: EntityKind::Specification as i32,
                id: spec(&entity).id.clone(),
            }),
            owner_uid: entity.system.as_ref().unwrap().uid.clone(),
            id: occurrence.id.map(Into::into),
        };
        assert_eq!(resolve_connection(&entity, &selected).unwrap(), occurrence);
    }
    let mut undeclared = entity.clone();
    spec_mut(&mut undeclared).exposures.clear();
    spec_mut(&mut undeclared).usages[0].exposure_id = None;
    spec_mut(&mut undeclared).usages[0].connection_id = None;
    let undeclared = prepare_entity(Some(&entity), &undeclared).unwrap();
    validate_specification(spec(&undeclared)).unwrap();
    assert!(!is_api(spec(&undeclared)));
    let mut schema_only = undeclared;
    let s = spec_mut(&mut schema_only);
    s.usages.clear();
    s.operations.clear();
    s.messages.clear();
    validate_specification(s).unwrap();
}

#[test]
fn member_identity_survives_edits_and_is_bounded_to_active_state() {
    let original = fixture();
    let mut renamed = original.clone();
    spec_mut(&mut renamed).messages[0].name = Some(name("renamed"));
    spec_mut(&mut renamed).messages[0].doc = Some("documentation".into());
    let renamed = prepare_entity(Some(&original), &renamed).unwrap();
    assert_eq!(
        spec(&renamed).messages[0].member_id,
        spec(&original).messages[0].member_id
    );
    let mut removed = renamed.clone();
    let s = spec_mut(&mut removed);
    s.messages.clear();
    s.operations.clear();
    s.exposures.clear();
    s.usages.clear();
    let removed = prepare_entity(Some(&renamed), &removed).unwrap();
    assert!(prepare_entity(Some(&removed), &renamed).is_err());
    let mut addition = removed.clone();
    spec_mut(&mut addition).messages.push(ContractMessage {
        name: Some(name("renamed")),
        ..Default::default()
    });
    let added = prepare_entity(Some(&removed), &addition).unwrap();
    assert_ne!(
        spec(&added).messages[0].member_id,
        spec(&renamed).messages[0].member_id
    );
    validate_specification(spec(&added)).unwrap();
}

#[test]
fn immutable_artifacts_preserve_exact_utf8_bytes_and_pins() {
    let original = fixture();
    let pin = type_pin(&original);
    resolve_type(&original, &pin).unwrap();
    let mut changed = original.clone();
    spec_mut(&mut changed).artifacts[0].content = "different".into();
    assert!(validate_specification(spec(&changed)).is_err());
    spec_mut(&mut changed).artifacts[0].digest = Some(content_digest("different"));
    assert!(prepare_entity(Some(&original), &changed).is_err());
    assert_ne!(content_digest("a\r\n"), content_digest("a\n"));
    for mutation in 0..5 {
        let mut broken = pin.clone();
        match mutation {
            0 => broken.expected_selector = None,
            1 => broken.expected_selector.as_mut().unwrap().value = "/different".into(),
            2 => broken.expected_digest = Some(content_digest("different")),
            3 => broken.expected_artifact_id = Some(member()),
            _ => broken.schema.as_mut().unwrap().owner_uid = "different".into(),
        }
        assert!(resolve_type(&original, &broken).is_err());
    }
    let mut wrong_kind = pin.schema.unwrap();
    wrong_kind.kind = ContractMemberKind::Artifact as i32;
    assert!(resolve_member(&original, &wrong_kind).is_err());
}

#[test]
fn all_association_endpoints_and_containing_members_are_identity_significant() {
    let original = fixture();
    for mutation in 0..7 {
        let mut changed = original.clone();
        let s = spec_mut(&mut changed);
        match mutation {
            0 => s.schemas[0].definition.as_mut().unwrap().selector = None,
            1 => s.messages[0].schemas[0].role = schema_binding::Role::Headers as i32,
            2 => s.operations[0].messages[0].role = operation_message::Role::Request as i32,
            3 => s.usages[0].exposure_id = None,
            4 => s.exposures[0].provider = s.usages[0].caller.clone(),
            5 => {
                s.messages[0]
                    .mapping
                    .as_mut()
                    .unwrap()
                    .read_model
                    .as_mut()
                    .unwrap()
                    .id = Some(id("different"));
            }
            _ => {
                let mut duplicate = s.messages[0].clone();
                duplicate.member_id = None;
                duplicate.name = Some(name("different"));
                s.messages.clear();
                s.messages.push(duplicate);
            }
        }
        assert!(
            prepare_entity(Some(&original), &changed).is_err(),
            "mutation {mutation}"
        );
    }
    let mut docs = original.clone();
    spec_mut(&mut docs).schemas[0].doc = Some("schema documentation".into());
    spec_mut(&mut docs).operations[0].messages[0].doc = Some("occurrence documentation".into());
    let edited = prepare_entity(Some(&original), &docs).unwrap();
    assert_eq!(
        connection_occurrences(&original).unwrap(),
        connection_occurrences(&edited).unwrap()
    );
}

#[test]
fn member_removal_requires_local_selection_repair_and_duplicate_ids_fail() {
    let entity = fixture();
    for mutation in 0..6 {
        let mut changed = entity.clone();
        let s = spec_mut(&mut changed);
        match mutation {
            0 => s.schemas.clear(),
            1 => s.artifacts.clear(),
            2 => s.exposures.clear(),
            3 => s.messages[0].member_id = s.schemas[0].member_id.clone(),
            4 => {
                s.messages[0].mapping.as_mut().unwrap().connection_id = s.schemas[0]
                    .definition
                    .as_ref()
                    .unwrap()
                    .connection_id
                    .clone();
            }
            _ => s.operations[0].messages[0].message_id = s.schemas[0].member_id.clone(),
        }
        assert!(validate_specification(s).is_err(), "mutation {mutation}");
    }
}

#[test]
fn concurrent_member_removal_blocks_land_and_explicit_restore_rewrites_local_graph() {
    let base = fixture();
    let mut opposing = base.clone();
    let s = spec_mut(&mut opposing);
    s.schemas.clear();
    s.messages.clear();
    s.operations.clear();
    s.exposures.clear();
    s.usages.clear();
    let opposing = prepare_entity(Some(&base), &opposing).unwrap();
    assert!(prepare_landing(Some(&base), Some(&opposing), &base).is_err());
    let restored = prepare_resolution(Some(&base), Some(&opposing), &base).unwrap();
    validate_specification(spec(&restored)).unwrap();
    assert_ne!(
        spec(&base).schemas[0].member_id,
        spec(&restored).schemas[0].member_id
    );
    assert_ne!(
        spec(&base).operations[0].member_id,
        spec(&restored).operations[0].member_id
    );
    assert_eq!(
        spec(&restored).usages[0].exposure_id,
        spec(&restored).exposures[0].connection_id
    );
    assert!(prepare_landing(Some(&opposing), Some(&opposing), &restored).is_ok());
    assert!(resolve_type(&restored, &type_pin(&base)).is_err());
}

#[test]
fn owner_clone_keeps_local_ids_and_regenerates_incarnation() {
    let original = fixture();
    let cloned = prepare_entity(None, &original).unwrap();
    assert_ne!(original.system, cloned.system);
    assert_eq!(spec(&original), spec(&cloned));
    assert!(resolve_type(&cloned, &type_pin(&original)).is_err());
    let restored = prepare_resolution(Some(&original), None, &original).unwrap();
    assert_eq!(restored, original);
}

#[test]
fn every_nested_external_target_is_discovered_but_pins_are_not_retargeted() {
    let mut entity = fixture();
    let pin = type_pin(&entity);
    let s = spec_mut(&mut entity);
    s.messages[0].schemas.extend([
        SchemaBinding {
            r#type: Some(schema_binding::Type::Schema(SchemaRef {
                id: Some(id("native")),
            })),
            ..Default::default()
        },
        SchemaBinding {
            r#type: Some(schema_binding::Type::LibraryType(TypeLibraryTypeRef {
                library: Some(TypeLibraryRef {
                    id: Some(id("types")),
                }),
                message: Some(QualifiedProtoName {
                    value: "types.Message".into(),
                }),
            })),
            ..Default::default()
        },
        SchemaBinding {
            r#type: Some(schema_binding::Type::ContractType(pin)),
            ..Default::default()
        },
    ]);
    s.dependencies.push(ContractDependency {
        target: Some(contract_dependency::Target::Specification(
            SpecificationRef {
                id: Some(id("dependency")),
            },
        )),
        ..Default::default()
    });
    s.dependencies.push(ContractDependency {
        target: Some(contract_dependency::Target::TypeLibrary(TypeLibraryRef {
            id: Some(id("types")),
        })),
        ..Default::default()
    });
    s.operations[0].binding = Some(OperationBinding {
        kind: Some(operation_binding::Kind::Channel(ChannelOperationBinding {
            channel: Some(ChannelRef {
                id: Some(id("transport")),
            }),
            ..Default::default()
        })),
    });
    let refs = outbound_refs(&entity);
    assert_eq!(refs.len(), 9);
    for kind in [
        EntityKind::ReadModel,
        EntityKind::Schema,
        EntityKind::TypeLibrary,
        EntityKind::Specification,
        EntityKind::Channel,
        EntityKind::Component,
    ] {
        assert!(refs.iter().any(|r| r.to_kind == Some(kind)), "{kind:?}");
    }
    assert!(refs
        .iter()
        .all(|r| r.to_kind != Some(EntityKind::Command) && r.to_kind != Some(EntityKind::Event)));
    assert_eq!(
        retarget_refs(
            &mut entity,
            EntityKind::Specification,
            &id("orders"),
            &id("replacement")
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        retarget_refs(
            &mut entity,
            EntityKind::TypeLibrary,
            &id("types"),
            &id("replacement")
        )
        .len(),
        2
    );
    assert_eq!(
        retarget_refs(
            &mut entity,
            EntityKind::Channel,
            &id("transport"),
            &id("replacement")
        )
        .len(),
        1
    );
}

#[test]
fn idless_noop_manifest_keeps_authoritative_member_and_connection_ids() {
    let original = fixture();
    let mut manifest = original.clone();
    let s = spec_mut(&mut manifest);
    for a in &mut s.artifacts {
        a.member_id = None;
    }
    for a in &mut s.schemas {
        a.member_id = None;
        a.definition.as_mut().unwrap().connection_id = None;
    }
    for a in &mut s.messages {
        a.member_id = None;
        a.mapping.as_mut().unwrap().connection_id = None;
        for b in &mut a.schemas {
            b.connection_id = None;
        }
    }
    for a in &mut s.operations {
        a.member_id = None;
        for m in &mut a.messages {
            m.connection_id = None;
        }
    }
    for a in &mut s.exposures {
        a.connection_id = None;
    }
    for a in &mut s.usages {
        a.connection_id = None;
    }
    assert_eq!(
        prepare_entity(Some(&original), &manifest).unwrap(),
        original
    );
}

#[test]
fn edited_idless_manifest_cannot_strip_retained_member_identity() {
    let entity = prepare_entity(
        None,
        &Entity {
            kind: Some(entity::Kind::Specification(Specification {
                id: Some(id("standalone")),
                title: "Standalone".into(),
                messages: vec![ContractMessage {
                    name: Some(name("message")),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            system: None,
        },
    )
    .unwrap();
    let mut manifest = entity.clone();
    spec_mut(&mut manifest).messages[0].member_id = None;
    assert_eq!(prepare_entity(Some(&entity), &manifest).unwrap(), entity);
    spec_mut(&mut manifest).messages[0].doc = Some("edited".into());
    assert!(prepare_entity(Some(&entity), &manifest).is_err());
    spec_mut(&mut manifest).messages[0].name = Some(name("renamed"));
    assert!(prepare_entity(Some(&entity), &manifest).is_err());
}

#[test]
fn changed_schema_definition_preserves_local_bindings_but_invalidates_foreign_pin() {
    let original = fixture();
    let pin = type_pin(&original);
    let mut changed = original.clone();
    let s = spec_mut(&mut changed);
    s.schemas[0].definition.as_mut().unwrap().selector = None;
    s.schemas[0].definition.as_mut().unwrap().connection_id = None;
    let changed = prepare_entity(Some(&original), &changed).unwrap();
    validate_specification(spec(&changed)).unwrap();
    assert_eq!(
        spec(&changed).messages[0].schemas,
        spec(&original).messages[0].schemas
    );
    assert!(resolve_type(&changed, &pin).is_err());
}

#[test]
fn limits_and_transport_declarations_are_admitted_without_semantic_document_parsing() {
    let entity = fixture();
    for mutation in 0..8 {
        let mut changed = entity.clone();
        let s = spec_mut(&mut changed);
        match mutation {
            0 => s.artifacts[0].path.as_mut().unwrap().value = "../orders".into(),
            1 => s.artifacts[0].media_type.as_mut().unwrap().value = "non-ascii-λ".into(),
            2 => s.artifacts[0].content = "x".repeat(256 * 1024 + 1),
            3 => {
                s.artifacts[0].format = Some(ContractFormat {
                    kind: Some(contract_format::Kind::Known(999)),
                });
            }
            4 => s.messages[0].schemas[0].role = 999,
            5 => s.operations[0].messages[0].role = 0,
            6 => {
                s.operations[0].binding = Some(OperationBinding {
                    kind: Some(operation_binding::Kind::Http(HttpOperationBinding {
                        method: "get".into(),
                        path: "/orders".into(),
                        response_status_codes: vec![200],
                    })),
                });
            }
            _ => {
                s.operations[0].binding = Some(OperationBinding {
                    kind: Some(operation_binding::Kind::Http(HttpOperationBinding {
                        method: "GET".into(),
                        path: "/orders".into(),
                        response_status_codes: vec![200, 200],
                    })),
                });
            }
        }
        assert!(validate_specification(s).is_err(), "mutation {mutation}");
    }
    let mut opaque = entity;
    let s = spec_mut(&mut opaque);
    s.artifacts[0].format = Some(ContractFormat {
        kind: Some(contract_format::Kind::Other("custom format".into())),
    });
    validate_specification(s).unwrap();
}

#[test]
fn missing_persisted_member_or_association_ids_never_bypass_noop_validation() {
    let original = fixture();
    let mut broken = original.clone();
    spec_mut(&mut broken).messages[0].member_id = None;
    assert!(prepare_entity(Some(&broken), &broken).is_err());
    let mut broken = original;
    spec_mut(&mut broken).usages[0].connection_id = None;
    assert!(prepare_entity(Some(&broken), &broken).is_err());
}

#[test]
fn strict_member_uuid_validation_and_new_owner_assignment() {
    for value in [
        "",
        "00000000-0000-0000-0000-000000000000",
        "019382AB-4455-7666-8888-123456789012",
        "019382ab445576668888123456789012",
        "arbitrary",
    ] {
        assert!(value.parse::<MemberId>().is_err());
    }
    let accepted = MemberId::new();
    assert_eq!(accepted.to_string().parse::<MemberId>().unwrap(), accepted);
    let input = Entity {
        kind: Some(entity::Kind::Specification(Specification {
            id: Some(id("fresh")),
            title: "Fresh".into(),
            messages: vec![ContractMessage {
                name: Some(name("unmapped")),
                ..Default::default()
            }],
            ..Default::default()
        })),
        system: None,
    };
    let created = prepare_entity(None, &input).unwrap();
    assert!(spec(&created).messages[0].member_id.is_some());
    validate_specification(spec(&created)).unwrap();
    let repeated = prepare_entity(None, &input).unwrap();
    assert_ne!(
        spec(&created).messages[0].member_id,
        spec(&repeated).messages[0].member_id
    );
}

#[test]
fn resolver_rejects_corrupt_stored_member_identity_sets() {
    let original = fixture();
    let pin = type_pin(&original);
    for mutation in 0..3 {
        let mut corrupt = original.clone();
        let s = spec_mut(&mut corrupt);
        match mutation {
            0 => s.schemas.push(s.schemas[0].clone()),
            1 => s.messages[0].member_id = None,
            _ => s.messages[0].member_id = s.schemas[0].member_id.clone(),
        }
        assert!(
            resolve_member(&corrupt, pin.schema.as_ref().unwrap()).is_err(),
            "mutation {mutation}"
        );
        assert!(resolve_type(&corrupt, &pin).is_err(), "mutation {mutation}");
    }
}

#[test]
fn same_owner_type_pins_require_local_schema_selection() {
    let mut original = fixture();
    let pin = type_pin(&original);
    spec_mut(&mut original).messages[0]
        .schemas
        .push(SchemaBinding {
            role: schema_binding::Role::Payload as i32,
            r#type: Some(schema_binding::Type::ContractType(pin)),
            ..Default::default()
        });
    let prepared = prepare_entity(None, &original).unwrap();
    assert!(validate_specification(spec(&prepared)).is_err());
}
