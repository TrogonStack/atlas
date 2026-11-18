#![allow(clippy::unwrap_used)]
use prost::Message;
use trogon_atlas_core::agent_metadata::validate;
use trogon_atlas_proto::{entity, Component, Entity, Event};

fn annotation(name: &str, value: Vec<u8>) -> prost_types::Any {
    prost_types::Any {
        type_url: format!("type.googleapis.com/{name}"),
        value,
    }
}
#[test]
fn agent_presence_is_component_only_and_singleton() {
    let any = annotation("trogonatlas.eventmodel.v1alpha1.AgentAnnotation", vec![]);
    let component = |metadata| Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata,
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(validate(&component(vec![any.clone()])).is_ok());
    assert!(validate(&component(vec![any.clone(), any.clone()])).is_err());
    let event = Entity {
        kind: Some(entity::Kind::Event(Event {
            metadata: vec![any],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(validate(&event).is_err());
}
#[test]
fn lifecycle_calendar_date_and_known_wire_are_validated() {
    use trogon_atlas_proto::{CalendarDate, LifecycleAnnotation};
    let owner = |value| Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(
                "trogonatlas.annotation.v1alpha1.LifecycleAnnotation",
                value,
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    for (date, valid) in [
        ("2024-02-29", true),
        ("2023-02-29", false),
        ("0000-01-01", false),
    ] {
        let lifecycle = LifecycleAnnotation {
            status: "deprecated".into(),
            deprecated_on: Some(CalendarDate { value: date.into() }),
            ..Default::default()
        };
        assert_eq!(validate(&owner(lifecycle.encode_to_vec())).is_ok(), valid);
    }
    assert!(validate(&owner(vec![255])).is_err());
}

#[test]
fn values_and_resource_occurrences_are_native() {
    use trogon_atlas_core::connections::{assign_connection_ids, connection_occurrences};
    use trogon_atlas_proto::{
        AgentAnnotation, AgentModel, ResourceGroup, ResourceGroupItem, ResourceGroupsAnnotation,
    };
    let owner = |name, bytes| Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(name, bytes)],
            ..Default::default()
        })),
        ..Default::default()
    };
    let empty_model = AgentAnnotation {
        model: Some(AgentModel::default()),
        ..Default::default()
    };
    assert!(validate(&owner(
        "trogonatlas.eventmodel.v1alpha1.AgentAnnotation",
        empty_model.encode_to_vec()
    ))
    .is_err());
    let groups = ResourceGroupsAnnotation {
        groups: vec![ResourceGroup {
            items: vec![ResourceGroupItem::default()],
            ..Default::default()
        }],
    };
    let mut entity = owner(
        "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
        groups.encode_to_vec(),
    );
    assert!(validate(&entity).is_err());
    assert_eq!(assign_connection_ids(&mut entity).unwrap(), 2);
    assert_eq!(connection_occurrences(&entity).unwrap().len(), 2);
}

#[test]
fn group_refs_retarget_and_malformed_payloads_are_accounted_for() {
    use trogon_atlas_core::{outbound_refs, retarget_refs};
    use trogon_atlas_proto::*;
    let target = Id {
        namespace: "example".into(),
        slug: "target".into(),
        version: 1,
    };
    let groups = ResourceGroupsAnnotation {
        groups: vec![ResourceGroup {
            items: vec![
                ResourceGroupItem {
                    target: Some(EntityRef {
                        kind: EntityKind::Component as i32,
                        id: Some(target.clone())
                    }),
                    ..Default::default()
                };
                2
            ],
            ..Default::default()
        }],
    };
    let mut owner = Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(
                "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
                groups.encode_to_vec(),
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    trogon_atlas_core::connections::assign_connection_ids(&mut owner).unwrap();
    assert_eq!(outbound_refs(&owner).len(), 2);
    let new = Id {
        slug: "new".into(),
        ..target.clone()
    };
    assert_eq!(
        retarget_refs(&mut owner, EntityKind::Component, &target, &new).len(),
        2
    );
    assert!(outbound_refs(&owner).iter().all(|r| r.to_id == new));
}

#[test]
fn known_annotations_cannot_hide_behind_unknown_wire_fields() {
    let owner = Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(
                "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
                vec![0xA0, 0x06, 0x01],
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(validate(&owner).is_err());
    assert!(trogon_atlas_core::refs::try_outbound_refs(&owner).is_err());
}

#[test]
fn groups_share_identity_domain_with_native_members() {
    use trogon_atlas_proto::{
        ContractArtifact, ContractMemberId, ResourceGroup, ResourceGroupsAnnotation, Specification,
    };
    let id = trogon_atlas_core::connections::ConnectionId::new().to_string();
    let groups = ResourceGroupsAnnotation {
        groups: vec![ResourceGroup {
            group_id: Some(trogon_atlas_proto::ConnectionId { value: id.clone() }),
            ..Default::default()
        }],
    };
    let owner = Entity {
        kind: Some(entity::Kind::Specification(Specification {
            artifacts: vec![ContractArtifact {
                member_id: Some(ContractMemberId { value: id }),
                ..Default::default()
            }],
            metadata: vec![annotation(
                "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
                groups.encode_to_vec(),
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(validate(&owner).is_err());
}

#[test]
fn source_metadata_ownership_cannot_claim_native_additions() {
    use trogon_atlas_proto::{
        source_field_ownership::Policy, SourceFieldOwnership, SourceOwnedField, SourceOwnedFields,
    };
    let owner = Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(
                "trogonatlas.eventmodel.v1alpha1.AgentAnnotation",
                vec![],
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    let policy = SourceFieldOwnership {
        policy: Some(Policy::Fields(SourceOwnedFields {
            fields: vec![SourceOwnedField::Metadata as i32],
        })),
    };
    assert!(trogon_atlas_core::source_integration::validate_ownership(&owner, &policy).is_err());
}

#[test]
fn public_admission_enforces_tool_and_badge_values_and_budgets() {
    use trogon_atlas_proto::{
        AgentAnnotation, AgentDescription, AgentIcon, AgentTool, AgentUrl, Badge, BadgeText,
        BadgesAnnotation, HexColor, ToolKindText, ToolName,
    };
    let tool = AgentTool {
        name: Some(ToolName {
            value: "lookup".into(),
        }),
        r#type: Some(ToolKindText {
            value: "descriptive".into(),
        }),
        description: Some(AgentDescription {
            value: "line\n\ttext".into(),
        }),
        icon: Some(AgentIcon {
            value: "unknown:token".into(),
        }),
        url: Some(AgentUrl {
            value: "https://example.com/docs".into(),
        }),
    };
    let owner = |name, bytes| Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(name, bytes)],
            ..Default::default()
        })),
        ..Default::default()
    };
    let agent = |tools| {
        owner(
            "trogonatlas.eventmodel.v1alpha1.AgentAnnotation",
            AgentAnnotation {
                tools,
                ..Default::default()
            }
            .encode_to_vec(),
        )
    };
    assert!(validate(&agent(vec![tool.clone()])).is_ok());
    assert!(validate(&agent(vec![tool.clone(); 2])).is_err());
    let mut tools = Vec::new();
    for i in 0..64 {
        let mut t = tool.clone();
        t.name = Some(ToolName {
            value: format!("tool{i}"),
        });
        tools.push(t);
    }
    assert!(validate(&agent(tools.clone())).is_ok());
    let mut extra = tool.clone();
    extra.name = Some(ToolName {
        value: "extra".into(),
    });
    tools.push(extra);
    assert!(validate(&agent(tools)).is_err());
    for value in [
        "http://example.com",
        "https://user@example.com",
        "https://@example.com",
        "relative",
    ] {
        let mut t = tool.clone();
        t.url = Some(AgentUrl {
            value: value.into(),
        });
        assert!(validate(&agent(vec![t])).is_err());
    }
    for value in ["", " ", "bad\u{0007}"] {
        let mut t = tool.clone();
        t.name = Some(ToolName {
            value: value.into(),
        });
        assert!(validate(&agent(vec![t])).is_err());
    }
    let mut t = tool.clone();
    t.name = Some(ToolName {
        value: "é".repeat(128),
    });
    assert!(validate(&agent(vec![t.clone()])).is_ok());
    t.name.as_mut().unwrap().value.push('x');
    assert!(validate(&agent(vec![t])).is_err());
    let badge = Badge {
        content: Some(BadgeText {
            value: "label".into(),
        }),
        background_color: Some(HexColor {
            value: "#aAbBcC".into(),
        }),
        text_color: Some(HexColor {
            value: "#000000".into(),
        }),
        ..Default::default()
    };
    let badges = |badges| {
        owner(
            "trogonatlas.annotation.v1alpha1.BadgesAnnotation",
            BadgesAnnotation { badges }.encode_to_vec(),
        )
    };
    assert!(validate(&badges(vec![badge.clone()])).is_ok());
    assert!(validate(&badges(vec![badge.clone(); 2])).is_err());
    let mut changed = badge.clone();
    changed.background_color = Some(HexColor {
        value: "red".into(),
    });
    assert!(validate(&badges(vec![changed])).is_err());
    let mut changed = badge.clone();
    changed.background_color = Some(HexColor {
        value: "#FFFFFF".into(),
    });
    assert!(validate(&badges(vec![badge, changed])).is_ok());
}

#[test]
fn resource_group_identity_survives_presentation_edits_and_json_roundtrip() {
    use trogon_atlas_core::connections::{assign_connection_ids, reconcile_connection_ids};
    use trogon_atlas_proto::{
        EntityKind, EntityRef, Id, MetadataTitle, ResourceGroup, ResourceGroupItem,
        ResourceGroupsAnnotation,
    };
    let groups = ResourceGroupsAnnotation {
        groups: vec![ResourceGroup {
            items: vec![
                ResourceGroupItem {
                    target: Some(EntityRef {
                        kind: EntityKind::Component as i32,
                        id: Some(Id {
                            namespace: "example".into(),
                            slug: "b".into(),
                            version: 1
                        })
                    }),
                    ..Default::default()
                };
                2
            ],
            ..Default::default()
        }],
    };
    let mut previous = Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata: vec![annotation(
                "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
                groups.encode_to_vec(),
            )],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(assign_connection_ids(&mut previous).unwrap(), 3);
    let json = trogon_atlas_core::transcode::entity_to_json(&previous).unwrap();
    let roundtrip: Entity = trogon_atlas_core::transcode::message_from_json(
        "trogonatlas.eventmodel.v1alpha1.Entity",
        &json,
    )
    .unwrap();
    assert!(trogon_atlas_core::semantically_equal(&previous, &roundtrip));
    let mut proposed = previous.clone();
    let Some(entity::Kind::Component(owner)) = &mut proposed.kind else {
        unreachable!()
    };
    let mut groups = ResourceGroupsAnnotation::decode(owner.metadata[0].value.as_slice()).unwrap();
    groups.groups[0].title = Some(MetadataTitle {
        value: "title".into(),
    });
    groups.groups[0].items.reverse();
    owner.metadata[0].value = groups.encode_to_vec();
    assert_eq!(
        reconcile_connection_ids(&previous, &mut proposed).unwrap(),
        0
    );
    assert!(!trogon_atlas_core::semantically_equal(&previous, &proposed));
    let Some(entity::Kind::Component(owner)) = &mut proposed.kind else {
        unreachable!()
    };
    groups.groups[0].items[0]
        .target
        .as_mut()
        .unwrap()
        .id
        .as_mut()
        .unwrap()
        .slug = "changed".into();
    owner.metadata[0].value = groups.encode_to_vec();
    assert!(reconcile_connection_ids(&previous, &mut proposed).is_err());
}
