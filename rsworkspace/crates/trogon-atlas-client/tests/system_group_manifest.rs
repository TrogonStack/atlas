#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{apply, export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;

const MEMBER_A: &str = "01930000-0000-7000-8000-000000000001";
const MEMBER_B: &str = "01930000-0000-7000-8000-000000000002";

fn group(members: &serde_json::Value) -> serde_json::Value {
    json!({"apiVersion":manifest::API_VERSION,"kind":"SystemGroup","metadata":{"namespace":"architecture","name":"platform","version":"18446744073709551615"},"spec":{
        "title":"Platform",
        "scope":"SYSTEM_GROUP_SCOPE_EXTERNAL",
        "members":members,
        "relationships":[{"connectionId":{"value":"01930000-0000-7000-8000-000000000003"},"target":"billing@9007199254740993"}],
        "actors":[{"connectionId":{"value":"01930000-0000-7000-8000-000000000004"},"target":{"persona":"operator"},"direction":"SYSTEM_ACTOR_DIRECTION_INBOUND"}]
    }})
}
fn members() -> serde_json::Value {
    json!([
        {"connectionId":{"value":MEMBER_A},"member":{"component":"worker@9007199254740993"},"doc":{"value":"Cafe\u{301}\r\n<script>inert()</script>"}},
        {"connectionId":{"value":MEMBER_B},"member":{"component":"worker@9007199254740993"},"doc":{"value":"Cafe\u{301}\r\n<script>inert()</script>"}},
        {"member":{"systemGroup":"billing"}}
    ])
}

#[test]
fn native_groups_roundtrip_parallel_ids_exact_versions_and_optional_labels() {
    let types = TranscodePool::builtin().unwrap();
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    let input = group(&members());
    assert!(validator.is_valid(&input), "{input}");
    let loaded = manifest::parse_str(&input.to_string(), "groups.yaml").unwrap();
    let entity = &loaded[0].entity;
    assert_eq!(loaded[0].id.version, u64::MAX);
    let Some(trogon_atlas_proto::entity::Kind::SystemGroup(parsed)) = &entity.kind else {
        panic!("expected system group");
    };
    assert_eq!(parsed.members.len(), 3);
    assert_ne!(
        parsed.members[0].connection_id,
        parsed.members[1].connection_id
    );
    assert_eq!(parsed.members[2].doc, None);
    assert_eq!(parsed.relationships[0].label, None);
    assert_eq!(parsed.actors[0].label, None);
    let output = export::render_board(std::slice::from_ref(entity), &[], &types).unwrap();
    assert_eq!(output.skipped, []);
    let reparsed = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
    assert_eq!(reparsed[0].entity, *entity);
    assert_eq!(
        apply::classify(&reparsed[0].entity, Some(entity)),
        apply::Action::Unchanged
    );
    let mut reordered = members();
    reordered.as_array_mut().unwrap().reverse();
    let changed = manifest::parse_str(&group(&reordered).to_string(), "reordered.yaml").unwrap();
    assert_eq!(
        apply::classify(&changed[0].entity, Some(entity)),
        apply::Action::Configure
    );
}

#[test]
fn domains_author_members_and_stay_unchanged_without_them() {
    let types = TranscodePool::builtin().unwrap();
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    let domain = |spec: serde_json::Value| json!({"apiVersion":manifest::API_VERSION,"kind":"Domain","metadata":{"namespace":"architecture","name":"commerce"},"spec":spec});
    let plain = domain(json!({"title":"Commerce","project":"architecture"}));
    let authored = domain(
        json!({"title":"Commerce","project":"architecture","members":[
            {"connectionId":{"value":MEMBER_A},"member":{"domain":"payments@2"}},
            {"connectionId":{"value":MEMBER_B},"member":{"systemGroup":"platform"}}
        ]}),
    );
    for input in [&plain, &authored] {
        assert!(validator.is_valid(input), "{input}");
    }
    let loaded = manifest::parse_str(&authored.to_string(), "domain.yaml").unwrap();
    let entity = &loaded[0].entity;
    let output = export::render_board(std::slice::from_ref(entity), &[], &types).unwrap();
    let reparsed = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
    assert_eq!(reparsed[0].entity, *entity);
    let bare = manifest::parse_str(&plain.to_string(), "plain.yaml").unwrap();
    let bare_output =
        export::render_board(std::slice::from_ref(&bare[0].entity), &[], &types).unwrap();
    assert!(!bare_output.manifests.contains("members"));
}

#[test]
fn group_structure_rejects_ambiguous_targets_and_unknown_fields() {
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    for member in [
        json!({"member":{"component":"worker","systemGroup":"billing"}}),
        json!({"member":"worker"}),
        json!({"member":{"unknown":"worker"}}),
        json!({"member":{"component":"worker"},"unknown":true}),
    ] {
        let input = group(&json!([member]));
        assert!(manifest::parse_str(&input.to_string(), "invalid.yaml").is_err());
        assert!(!validator.is_valid(&input));
    }
}

fn parsed(value: &serde_json::Value) -> trogon_atlas_proto::Entity {
    manifest::parse_str(&value.to_string(), "diff.yaml").unwrap()[0]
        .entity
        .clone()
}

fn group_mut(entity: &mut trogon_atlas_proto::Entity) -> &mut trogon_atlas_proto::SystemGroup {
    let Some(trogon_atlas_proto::entity::Kind::SystemGroup(value)) = entity.kind.as_mut() else {
        panic!("expected system group");
    };
    value
}

fn id_of(value: &str) -> Option<trogon_atlas_proto::ConnectionId> {
    Some(trogon_atlas_proto::ConnectionId {
        value: value.into(),
    })
}

#[test]
fn group_diff_is_exact_for_every_persisted_member_relationship_and_actor_field() {
    use trogon_atlas_proto as pb;
    let types = TranscodePool::builtin().unwrap();
    let current = parsed(&group(&members()));
    let output = export::render_board(std::slice::from_ref(&current), &[], &types).unwrap();
    let exported = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
    assert_eq!(exported[0].entity, current);
    assert_eq!(
        apply::classify(&exported[0].entity, Some(&current)),
        apply::Action::Unchanged
    );

    let component = |slug: &str, version: u64| {
        pb::system_group_member::Target::Component(pb::ComponentRef {
            id: Some(pb::Id {
                namespace: "architecture".into(),
                slug: slug.into(),
                version,
            }),
        })
    };
    let mutations: Vec<(&str, Box<dyn Fn(&mut pb::SystemGroup)>)> = vec![
        ("title", Box::new(|g| g.title.push('!'))),
        (
            "scope",
            Box::new(|g| g.scope = pb::SystemGroupScope::Internal as i32),
        ),
        (
            "member target slug",
            Box::new(move |g| {
                g.members[0].member.as_mut().unwrap().target = Some(component("other", u64::MAX));
            }),
        ),
        (
            "member target version",
            Box::new(move |g| {
                g.members[0].member.as_mut().unwrap().target =
                    Some(component("worker", 9_007_199_254_740_992));
            }),
        ),
        (
            "member target kind",
            Box::new(|g| {
                g.members[0].member.as_mut().unwrap().target = Some(
                    pb::system_group_member::Target::SystemGroup(pb::SystemGroupRef {
                        id: Some(pb::Id {
                            namespace: "architecture".into(),
                            slug: "worker".into(),
                            version: 9_007_199_254_740_993,
                        }),
                    }),
                );
            }),
        ),
        (
            "member doc text",
            Box::new(|g| g.members[0].doc.as_mut().unwrap().value.push('x')),
        ),
        ("member doc cleared", Box::new(|g| g.members[0].doc = None)),
        (
            "member doc added",
            Box::new(|g| {
                g.members[2].doc = Some(pb::CatalogText {
                    value: String::new(),
                });
            }),
        ),
        (
            "member connection id",
            Box::new(|g| {
                g.members[0].connection_id = id_of("01930000-0000-7000-8000-0000000000ff");
            }),
        ),
        ("member removed", Box::new(|g| drop(g.members.pop()))),
        ("member order", Box::new(|g| g.members.swap(0, 1))),
        (
            "member appended",
            Box::new(|g| g.members.push(g.members[2].clone())),
        ),
        (
            "relationship target version",
            Box::new(|g| {
                g.relationships[0]
                    .target
                    .as_mut()
                    .unwrap()
                    .id
                    .as_mut()
                    .unwrap()
                    .version = 1;
            }),
        ),
        (
            "relationship label added",
            Box::new(|g| {
                g.relationships[0].label = Some(pb::CatalogTitle {
                    value: String::new(),
                });
            }),
        ),
        (
            "relationship connection id",
            Box::new(|g| {
                g.relationships[0].connection_id = id_of("01930000-0000-7000-8000-0000000000fe");
            }),
        ),
        (
            "relationship removed",
            Box::new(|g| g.relationships.clear()),
        ),
        (
            "actor direction",
            Box::new(|g| g.actors[0].direction = pb::SystemActorDirection::Outbound as i32),
        ),
        (
            "actor target",
            Box::new(|g| {
                g.actors[0].target = Some(pb::SystemActorTarget {
                    target: Some(pb::system_actor_target::Target::Persona(pb::PersonaRef {
                        id: Some(pb::Id {
                            namespace: "architecture".into(),
                            slug: "customer".into(),
                            version: 1,
                        }),
                    })),
                });
            }),
        ),
        (
            "actor label added",
            Box::new(|g| {
                g.actors[0].label = Some(pb::CatalogTitle {
                    value: "uses".into(),
                });
            }),
        ),
        (
            "actor connection id",
            Box::new(|g| {
                g.actors[0].connection_id = id_of("01930000-0000-7000-8000-0000000000fd");
            }),
        ),
        ("actor removed", Box::new(|g| g.actors.clear())),
    ];
    for (name, mutate) in mutations {
        let mut desired = current.clone();
        mutate(group_mut(&mut desired));
        assert_ne!(desired, current, "{name}");
        assert_eq!(
            apply::classify(&desired, Some(&current)),
            apply::Action::Configure,
            "{name}"
        );
        let output = export::render_board(std::slice::from_ref(&desired), &[], &types).unwrap();
        assert_eq!(output.skipped, [], "{name}");
        let reapplied = manifest::parse_str(&output.manifests, "mutated.yaml").unwrap();
        assert_eq!(reapplied[0].entity, desired, "{name}");
        assert_eq!(
            apply::classify(&reapplied[0].entity, Some(&current)),
            apply::Action::Configure,
            "{name}"
        );
        assert_eq!(
            apply::classify(&reapplied[0].entity, Some(&desired)),
            apply::Action::Unchanged,
            "{name}"
        );
    }
}

#[test]
fn group_diff_treats_omitted_ids_as_unassigned_only_for_the_same_ordered_endpoints() {
    let mut stored = members();
    stored[2]["connectionId"] = json!({"value":"01930000-0000-7000-8000-000000000005"});
    let current = parsed(&group(&stored));
    let strip = |mut value: serde_json::Value| {
        for collection in ["members", "relationships", "actors"] {
            for link in value["spec"][collection].as_array_mut().unwrap() {
                link.as_object_mut().unwrap().remove("connectionId");
            }
        }
        value
    };
    let unassigned = parsed(&strip(group(&members())));
    assert_eq!(
        apply::classify(&unassigned, Some(&current)),
        apply::Action::Unchanged
    );
    let mut reordered = members();
    reordered.as_array_mut().unwrap().reverse();
    let unassigned = parsed(&strip(group(&reordered)));
    assert_eq!(
        apply::classify(&unassigned, Some(&current)),
        apply::Action::Configure
    );
    let mut shorter = members();
    shorter.as_array_mut().unwrap().pop();
    let unassigned = parsed(&strip(group(&shorter)));
    assert_eq!(
        apply::classify(&unassigned, Some(&current)),
        apply::Action::Configure
    );
}
