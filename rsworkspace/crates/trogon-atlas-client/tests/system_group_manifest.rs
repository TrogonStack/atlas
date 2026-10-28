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
