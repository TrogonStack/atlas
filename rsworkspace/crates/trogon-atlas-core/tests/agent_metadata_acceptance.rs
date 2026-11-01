//! Acceptance matrix for the agent and shared metadata contract that the
//! existing core tests leave implicit: exact partial-model roundtrips, nested
//! scope refusal, singleton conflicts, draft/date rules, group invariants,
//! malformed payloads, and the maximum/maximum-plus-one budget boundaries.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_core::{agent_metadata::validate, refs::try_outbound_refs, transcode};
use trogon_atlas_proto::{
    entity, AgentAnnotation, AgentDescription, AgentModel, AgentTool, Badge, BadgeText,
    BadgesAnnotation, CalendarDate, CommandScenario, CommandSlice, Component, DraftNotice, Entity,
    EntityKind, EntityRef, Event, FieldSpec, HexColor, Id, LifecycleAnnotation, MetadataText,
    MetadataTitle, ModelName, ModelProvider, ModelVersion, PositiveDisplayLimit, ResourceGroup,
    ResourceGroupItem, ResourceGroupsAnnotation, Schema, ToolKindText, ToolName,
};

const AGENT: &str = "trogonatlas.eventmodel.v1alpha1.AgentAnnotation";
const LIFECYCLE: &str = "trogonatlas.annotation.v1alpha1.LifecycleAnnotation";
const BADGES: &str = "trogonatlas.annotation.v1alpha1.BadgesAnnotation";
const GROUPS: &str = "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation";

fn any(name: &str, value: Vec<u8>) -> prost_types::Any {
    prost_types::Any {
        type_url: format!("type.googleapis.com/{name}"),
        value,
    }
}

fn component(metadata: Vec<prost_types::Any>) -> Entity {
    Entity {
        kind: Some(entity::Kind::Component(Component {
            metadata,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn accepted(metadata: Vec<prost_types::Any>) -> bool {
    validate(&component(metadata)).is_ok()
}

fn model(provider: Option<&str>, name: Option<&str>, version: Option<&str>) -> AgentAnnotation {
    AgentAnnotation {
        model: Some(AgentModel {
            provider: provider.map(|value| ModelProvider {
                value: value.into(),
            }),
            name: name.map(|value| ModelName {
                value: value.into(),
            }),
            version: version.map(|value| ModelVersion {
                value: value.into(),
            }),
        }),
        ..Default::default()
    }
}

fn agent(annotation: &AgentAnnotation) -> prost_types::Any {
    any(AGENT, annotation.encode_to_vec())
}

fn lifecycle(value: &LifecycleAnnotation) -> prost_types::Any {
    any(LIFECYCLE, value.encode_to_vec())
}

fn target(version: u64) -> EntityRef {
    EntityRef {
        kind: EntityKind::Component as i32,
        id: Some(Id {
            namespace: "example".into(),
            slug: "target".into(),
            version,
        }),
    }
}

fn item() -> ResourceGroupItem {
    ResourceGroupItem {
        connection_id: None,
        target: Some(target(1)),
    }
}

fn groups(groups: Vec<ResourceGroup>) -> prost_types::Any {
    any(GROUPS, ResourceGroupsAnnotation { groups }.encode_to_vec())
}

fn badge(label: &str) -> Badge {
    Badge {
        content: Some(BadgeText {
            value: label.into(),
        }),
        background_color: Some(HexColor {
            value: "#000000".into(),
        }),
        text_color: Some(HexColor {
            value: "#FFFFFF".into(),
        }),
        ..Default::default()
    }
}

fn tool(name: &str) -> AgentTool {
    AgentTool {
        name: Some(ToolName { value: name.into() }),
        r#type: Some(ToolKindText {
            value: "manual".into(),
        }),
        ..Default::default()
    }
}

fn description(value: String) -> AgentDescription {
    AgentDescription { value }
}

#[test]
fn partial_models_roundtrip_exactly_through_wire_and_json() {
    for annotation in [
        model(Some("example"), None, None),
        model(None, Some("any-name"), None),
        model(None, None, Some("not-semver 1.x preview")),
    ] {
        let owner = component(vec![agent(&annotation)]);
        validate(&owner).unwrap();
        let json = transcode::entity_to_json(&owner).unwrap();
        let decoded: Entity =
            transcode::message_from_json("trogonatlas.eventmodel.v1alpha1.Entity", &json).unwrap();
        let Some(entity::Kind::Component(back)) = &decoded.kind else {
            panic!("component")
        };
        assert_eq!(
            AgentAnnotation::decode(back.metadata[0].value.as_slice()).unwrap(),
            annotation
        );
        assert!(trogon_atlas_core::semantically_equal(&owner, &decoded));
    }
}

#[test]
fn model_values_have_exact_byte_ceilings() {
    let at_limit = "é".repeat(128);
    assert!(accepted(vec![agent(&model(
        Some(&at_limit),
        Some(&at_limit),
        Some(&at_limit)
    ))]));
    let over = format!("{at_limit}x");
    for annotation in [
        model(Some(&over), None, None),
        model(None, Some(&over), None),
        model(None, None, Some(&over)),
        model(Some(" "), None, None),
        model(None, Some(""), None),
    ] {
        assert!(!accepted(vec![agent(&annotation)]), "{annotation:?}");
    }
}

#[test]
fn proposed_annotations_are_refused_below_top_level_native_metadata() {
    let proposed = [
        agent(&AgentAnnotation::default()),
        any(BADGES, BadgesAnnotation::default().encode_to_vec()),
        groups(vec![]),
        lifecycle(&LifecycleAnnotation {
            status: "draft".into(),
            ..Default::default()
        }),
    ];
    for annotation in &proposed {
        let schema = Schema {
            fields: vec![FieldSpec {
                name: "field".into(),
                metadata: vec![annotation.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let event = Entity {
            kind: Some(entity::Kind::Event(Event {
                schema: Some(any(
                    "trogonatlas.eventmodel.v1alpha1.Schema",
                    schema.encode_to_vec(),
                )),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(
            validate(&event).is_err(),
            "{} accepted on a nested FieldSpec",
            annotation.type_url
        );

        let slice = Entity {
            kind: Some(entity::Kind::CommandSlice(CommandSlice {
                scenarios: vec![CommandScenario {
                    metadata: vec![annotation.clone()],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(
            validate(&slice).is_err(),
            "{} accepted on a scenario",
            annotation.type_url
        );
    }
    let unrelated = prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.NoteAnnotation".into(),
        value: trogon_atlas_proto::NoteAnnotation {
            text: "kept".into(),
            ..Default::default()
        }
        .encode_to_vec(),
    };
    let slice = Entity {
        kind: Some(entity::Kind::CommandSlice(CommandSlice {
            scenarios: vec![CommandScenario {
                metadata: vec![unrelated],
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    validate(&slice).unwrap();
}

#[test]
fn every_known_singleton_rejects_identical_and_conflicting_duplicates() {
    let pairs = [
        (
            agent(&model(Some("a"), None, None)),
            agent(&model(Some("b"), None, None)),
        ),
        (
            lifecycle(&LifecycleAnnotation {
                status: "draft".into(),
                ..Default::default()
            }),
            lifecycle(&LifecycleAnnotation {
                status: "accepted".into(),
                ..Default::default()
            }),
        ),
        (
            any(
                BADGES,
                BadgesAnnotation {
                    badges: vec![badge("one")],
                }
                .encode_to_vec(),
            ),
            any(
                BADGES,
                BadgesAnnotation {
                    badges: vec![badge("two")],
                }
                .encode_to_vec(),
            ),
        ),
        (
            groups(vec![ResourceGroup {
                items: vec![item()],
                ..Default::default()
            }]),
            groups(vec![ResourceGroup {
                items: vec![item(), item()],
                ..Default::default()
            }]),
        ),
    ];
    for (first, second) in pairs {
        assert!(accepted(vec![first.clone()]), "{}", first.type_url);
        assert!(
            !accepted(vec![first.clone(), first.clone()]),
            "{}",
            first.type_url
        );
        assert!(
            !accepted(vec![first.clone(), second.clone()]),
            "{}",
            first.type_url
        );
        assert!(!accepted(vec![second, first]));
    }
}

#[test]
fn draft_notice_rules_and_calendar_dates() {
    let draft = |title: Option<&str>, message: Option<&str>, status: &str| {
        lifecycle(&LifecycleAnnotation {
            status: status.into(),
            draft: Some(DraftNotice {
                title: title.map(|value| MetadataTitle {
                    value: value.into(),
                }),
                message: message.map(|value| MetadataText {
                    value: value.into(),
                }),
            }),
            ..Default::default()
        })
    };
    assert!(accepted(vec![draft(
        Some("Title"),
        Some("Review"),
        " DrAfT "
    )]));
    assert!(accepted(vec![draft(None, Some("Review"), "draft")]));
    assert!(accepted(vec![lifecycle(&LifecycleAnnotation {
        status: "draft".into(),
        ..Default::default()
    })]));
    assert!(!accepted(vec![draft(None, Some("   "), "draft")]));
    assert!(!accepted(vec![draft(None, Some(""), "draft")]));
    assert!(!accepted(vec![draft(None, None, "draft")]));
    assert!(!accepted(vec![draft(Some(" "), Some("Review"), "draft")]));
    assert!(!accepted(vec![draft(None, Some("Review"), "accepted")]));

    let owner = component(vec![draft(None, Some("Review"), "draft")]);
    let Some(entity::Kind::Component(body)) = &owner.kind else {
        panic!("component")
    };
    let stored = LifecycleAnnotation::decode(body.metadata[0].value.as_slice()).unwrap();
    assert!(
        stored.draft.unwrap().title.is_none(),
        "no title is synthesized"
    );

    let dated = |status: &str, date: &str| {
        lifecycle(&LifecycleAnnotation {
            status: status.into(),
            deprecated_on: Some(CalendarDate { value: date.into() }),
            ..Default::default()
        })
    };
    for date in ["2024-02-29", "0001-01-01", "9999-12-31"] {
        assert!(accepted(vec![dated("deprecated", date)]), "{date}");
        assert!(accepted(vec![dated(" Deprecated ", date)]), "{date}");
        assert!(
            !accepted(vec![dated("accepted", date)]),
            "{date} under accepted"
        );
        assert!(!accepted(vec![dated("", date)]), "{date} without status");
    }
    for date in [
        "2023-02-29",
        "2026-04-31",
        "0000-01-01",
        "10000-01-01",
        "2024-02-29T00:00:00Z",
        "2024-02-29+01:00",
        "2024-2-9",
        "",
    ] {
        assert!(!accepted(vec![dated("deprecated", date)]), "{date:?}");
    }
}

#[test]
fn invalid_groups_are_refused_with_exact_selectors_only() {
    let one = |group: ResourceGroup| groups(vec![group]);
    let good = ResourceGroup {
        items: vec![item()],
        ..Default::default()
    };
    assert!(accepted(vec![one(good.clone())]));
    let mut zero = good.clone();
    zero.display_limit = Some(PositiveDisplayLimit { value: 0 });
    assert!(!accepted(vec![one(zero)]));
    let mut latest = good.clone();
    latest.items[0].target = Some(target(0));
    assert!(!accepted(vec![one(latest)]));
    let mut missing = good.clone();
    missing.items[0].target = None;
    assert!(!accepted(vec![one(missing)]));
    let mut unspecified = good.clone();
    unspecified.items[0].target.as_mut().unwrap().kind = EntityKind::Unspecified as i32;
    assert!(!accepted(vec![one(unspecified)]));
    let mut unknown_kind = good.clone();
    unknown_kind.items[0].target.as_mut().unwrap().kind = 9999;
    assert!(!accepted(vec![one(unknown_kind)]));
    let mut blank = good.clone();
    blank.title = Some(MetadataTitle { value: " ".into() });
    assert!(!accepted(vec![one(blank)]));

    let id = trogon_atlas_core::connections::ConnectionId::new().to_string();
    let with_id = |group: &str, items: [&str; 2]| {
        one(ResourceGroup {
            group_id: Some(trogon_atlas_proto::ConnectionId {
                value: group.into(),
            }),
            items: items
                .iter()
                .map(|value| ResourceGroupItem {
                    connection_id: Some(trogon_atlas_proto::ConnectionId {
                        value: (*value).into(),
                    }),
                    target: Some(target(1)),
                })
                .collect(),
            ..Default::default()
        })
    };
    let other = trogon_atlas_core::connections::ConnectionId::new().to_string();
    let third = trogon_atlas_core::connections::ConnectionId::new().to_string();
    assert!(accepted(vec![with_id(&id, [&other, &third])]));
    assert!(
        !accepted(vec![with_id(&id, [&other, &other])]),
        "duplicate item ID"
    );
    assert!(
        !accepted(vec![with_id(&id, [&other, &id])]),
        "item reuses group ID"
    );
    let twice = groups(vec![
        ResourceGroup {
            group_id: Some(trogon_atlas_proto::ConnectionId { value: id.clone() }),
            ..Default::default()
        },
        ResourceGroup {
            group_id: Some(trogon_atlas_proto::ConnectionId { value: id }),
            ..Default::default()
        },
    ]);
    assert!(!accepted(vec![twice]), "duplicate group ID");
}

#[test]
fn malformed_known_payloads_never_masquerade_as_empty() {
    let valid = ResourceGroupsAnnotation {
        groups: vec![ResourceGroup {
            items: vec![item()],
            ..Default::default()
        }],
    }
    .encode_to_vec();
    let truncated = valid[..valid.len() - 1].to_vec();
    let lifecycle_bytes = LifecycleAnnotation {
        status: "draft".into(),
        ..Default::default()
    }
    .encode_to_vec();
    for (name, bytes) in [
        (GROUPS, truncated),
        (GROUPS, lifecycle_bytes),
        (GROUPS, vec![0x08, 0x01]),
        (AGENT, vec![0x08, 0x01]),
        (BADGES, vec![0x0a, 0x05, 0x01]),
        (LIFECYCLE, vec![0xff]),
    ] {
        let owner = component(vec![any(name, bytes.clone())]);
        assert!(validate(&owner).is_err(), "{name} {bytes:?}");
        assert!(try_outbound_refs(&owner).is_err(), "{name} {bytes:?}");
    }
}

#[test]
fn native_looking_strings_in_other_registered_annotations_create_no_edges() {
    let looking = r#"{"groups":[{"items":[{"target":{"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":"example","slug":"target","version":"1"}}}]}]} type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation"#;
    let note = any(
        "trogonatlas.annotation.v1alpha1.NoteAnnotation",
        trogon_atlas_proto::NoteAnnotation {
            text: looking.into(),
            author: looking.into(),
            created_at: looking.into(),
        }
        .encode_to_vec(),
    );
    let owner = component(vec![note]);
    validate(&owner).unwrap();
    assert!(try_outbound_refs(&owner).unwrap().is_empty());
    let unregistered = component(vec![any("catalog.notes.Unregistered", looking.into())]);
    assert!(try_outbound_refs(&unregistered).unwrap().is_empty());
    let Some(entity::Kind::Component(_)) = &unregistered.kind else {
        panic!("component")
    };
    assert!(
        transcode::entity_to_json(&unregistered).is_err(),
        "an unavailable descriptor fails JSON conversion instead of falling back"
    );
}

fn tools_of(count: usize) -> Vec<AgentTool> {
    (0..count).map(|i| tool(&format!("tool{i}"))).collect()
}

#[test]
fn collection_value_and_display_ceilings_accept_the_bound_and_refuse_one_more() {
    let agent_with = |tools| {
        agent(&AgentAnnotation {
            tools,
            ..Default::default()
        })
    };
    assert!(accepted(vec![agent_with(tools_of(64))]));
    assert!(!accepted(vec![agent_with(tools_of(65))]));

    let badges = |count: usize| {
        any(
            BADGES,
            BadgesAnnotation {
                badges: (0..count).map(|i| badge(&format!("b{i}"))).collect(),
            }
            .encode_to_vec(),
        )
    };
    assert!(accepted(vec![badges(32)]));
    assert!(!accepted(vec![badges(33)]));

    let group_list = |count: usize, per_group: usize| {
        groups(
            (0..count)
                .map(|_| ResourceGroup {
                    items: vec![item(); per_group],
                    ..Default::default()
                })
                .collect(),
        )
    };
    assert!(accepted(vec![group_list(32, 8)]));
    assert!(!accepted(vec![group_list(33, 0)]));
    assert!(
        !accepted(vec![group_list(32, 9)]),
        "distributed group items exceed the owner limit"
    );
    assert!(accepted(vec![group_list(1, 256)]));
    assert!(!accepted(vec![group_list(1, 257)]));

    let limit = |value| {
        groups(vec![ResourceGroup {
            display_limit: Some(PositiveDisplayLimit { value }),
            ..Default::default()
        }])
    };
    assert!(accepted(vec![limit(1)]));
    assert!(accepted(vec![limit(500)]));
    assert!(!accepted(vec![limit(501)]));

    let with_description = |value: String| {
        let mut one = tool("lookup");
        one.description = Some(description(value));
        agent_with(vec![one])
    };
    assert!(accepted(vec![with_description("é".repeat(2048))]));
    assert!(!accepted(vec![with_description(format!(
        "{}x",
        "é".repeat(2048)
    ))]));
    assert!(accepted(vec![with_description("a".repeat(4096))]));
    assert!(!accepted(vec![with_description("a".repeat(4097))]));

    let with_title = |value: String| {
        let mut owner = groups(vec![ResourceGroup {
            title: Some(MetadataTitle { value }),
            ..Default::default()
        }]);
        owner.type_url = format!("type.googleapis.com/{GROUPS}");
        owner
    };
    assert!(accepted(vec![with_title("é".repeat(128))]));
    assert!(!accepted(vec![with_title(format!("{}x", "é".repeat(128)))]));

    let with_badge_label = |value: String| {
        any(
            BADGES,
            BadgesAnnotation {
                badges: vec![badge(&value)],
            }
            .encode_to_vec(),
        )
    };
    assert!(accepted(vec![with_badge_label("é".repeat(128))]));
    assert!(!accepted(vec![with_badge_label(format!(
        "{}x",
        "é".repeat(128)
    ))]));

    let with_url = |value: String| {
        let mut one = tool("lookup");
        one.url = Some(trogon_atlas_proto::AgentUrl { value });
        agent_with(vec![one])
    };
    let base = "https://example.com/";
    assert!(accepted(vec![with_url(format!(
        "{base}{}",
        "a".repeat(2048 - base.len())
    ))]));
    assert!(!accepted(vec![with_url(format!(
        "{base}{}",
        "a".repeat(2049 - base.len())
    ))]));

    let with_icon = |value: String| {
        let mut one = tool("lookup");
        one.icon = Some(trogon_atlas_proto::AgentIcon { value });
        agent_with(vec![one])
    };
    assert!(accepted(vec![with_icon("a".repeat(128))]));
    assert!(!accepted(vec![with_icon("a".repeat(129))]));
}

/// Builds Agent annotation bytes whose Any encoding, summed with `others`,
/// lands exactly on `target`, and returns None when no description length does.
fn sized_agent(others: usize, target: usize, filler: &str) -> Option<prost_types::Any> {
    let mut tools: Vec<AgentTool> = (0..15)
        .map(|i| {
            let mut one = tool(&format!("tool{i}"));
            one.description = Some(description(filler.repeat(4096 / filler.len())));
            one
        })
        .collect();
    for length in (0..=4096 / filler.len()).rev() {
        for pad in 0..=1 {
            let text = format!("{}{}", filler.repeat(length), "a".repeat(pad));
            if text.is_empty() || text.len() > 4096 {
                continue;
            }
            let mut last = tool("last");
            last.description = Some(description(text));
            tools.push(last);
            let candidate = agent(&AgentAnnotation {
                tools: tools.clone(),
                ..Default::default()
            });
            tools.pop();
            if candidate.encoded_len() + others == target {
                return Some(candidate);
            }
        }
    }
    None
}

#[test]
fn aggregate_encoded_ceiling_is_inclusive_and_counts_utf8_bytes() {
    let lifecycle_any = lifecycle(&LifecycleAnnotation {
        status: "draft".into(),
        ..Default::default()
    });
    let badges_any = any(
        BADGES,
        BadgesAnnotation {
            badges: vec![badge("label")],
        }
        .encode_to_vec(),
    );
    let others = lifecycle_any.encoded_len() + badges_any.encoded_len();
    for filler in ["a", "é"] {
        let at = sized_agent(others, 65536, filler)
            .unwrap_or_else(|| panic!("no exact 64 KiB size for {filler:?}"));
        assert!(
            accepted(vec![at.clone(), lifecycle_any.clone(), badges_any.clone()]),
            "exactly 64 KiB is accepted for {filler:?}"
        );
        let over = sized_agent(others, 65537, filler)
            .unwrap_or_else(|| panic!("no 64 KiB + 1 size for {filler:?}"));
        assert!(
            !accepted(vec![over, lifecycle_any.clone(), badges_any.clone()]),
            "64 KiB + 1 is refused for {filler:?}"
        );
    }
    let multibyte_chars = "é".repeat(2048);
    assert_eq!(multibyte_chars.chars().count(), 2048);
    assert_eq!(multibyte_chars.len(), 4096);
}
