#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{apply, export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;

fn declaration() -> serde_json::Value {
    json!({"connectionId":{"value":"01930000-0000-7000-8000-000000000001"},"direction":"COMMUNICATION_DIRECTION_SEND","peer":{"externalSystem":"provider@9007199254740993"},"message":{"specification":"messages@18446744073709551615","ownerUid":"checked-specification-uid","kind":"CONTRACT_MEMBER_KIND_MESSAGE","memberId":{"value":"checked-message-id"}},"label":{"value":"Order hint"},"condition":{"value":"Cafe\u{301}\r\n<script>inert()</script>"}})
}
fn manifest_with(kind: &str, communications: &serde_json::Value) -> serde_json::Value {
    json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"communication","name":"checkout","version":"18446744073709551615"},"spec":{"title":"Checkout","communications":communications}})
}
#[test]
fn communication_manifest_preserves_direction_member_identity_precision_presence_and_reapply() {
    let types = TranscodePool::builtin().unwrap();
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    for kind in ["Component", "ExternalSystem"] {
        let mut receive = declaration();
        receive["connectionId"]["value"] = json!("01930000-0000-7000-8000-000000000002");
        receive["direction"] = json!("COMMUNICATION_DIRECTION_RECEIVE");
        receive["message"]["memberId"]["value"] = json!("another-message-id");
        receive.as_object_mut().unwrap().remove("condition");
        let input = manifest_with(kind, &json!([declaration(), receive]));
        assert!(validator.is_valid(&input));
        let parsed = manifest::parse_str(&input.to_string(), "communication.yaml").unwrap();
        let entity = &parsed[0].entity;
        let values = trogon_atlas_core::communication_intent::declarations(entity);
        assert_eq!(values.len(), 2);
        assert_ne!(values[0].connection_id, values[1].connection_id);
        assert!(values[0].condition.is_some());
        assert_eq!(values[1].condition, None);
        assert_eq!(
            values[0].message.as_ref().unwrap().owner_uid,
            "checked-specification-uid"
        );
        assert_eq!(
            values[0]
                .message
                .as_ref()
                .unwrap()
                .specification
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .version,
            u64::MAX
        );
        assert_eq!(parsed[0].id.version, u64::MAX);
        let output = export::render_board(std::slice::from_ref(entity), &[], &types).unwrap();
        assert_eq!(output.skipped, []);
        let reparsed = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
        assert_eq!(reparsed[0].entity, *entity);
        assert_eq!(
            apply::classify(&reparsed[0].entity, Some(entity)),
            apply::Action::Unchanged
        );
    }
}
#[test]
fn communication_manifest_rejects_ambiguous_peers_enum_coercion_and_wrong_field_types() {
    let mut invalid = Vec::new();
    for peer in [
        json!("provider@1"),
        json!({"component":"worker","externalSystem":"provider"}),
        json!({"entity":"provider"}),
    ] {
        let mut value = declaration();
        value["peer"] = peer;
        invalid.push(value);
    }
    for direction in [
        json!(true),
        json!("SEND"),
        json!("COMMUNICATION_DIRECTION_UNKNOWN"),
        json!([]),
    ] {
        let mut value = declaration();
        value["direction"] = direction;
        invalid.push(value);
    }
    for (field, value) in [
        ("condition", json!(false)),
        ("message", json!("message-a")),
        ("label", json!([])),
        ("unknown", json!(true)),
    ] {
        let mut declaration = declaration();
        declaration[field] = value;
        invalid.push(declaration);
    }
    for value in invalid {
        let input = manifest_with("Component", &json!([value]));
        assert!(
            manifest::parse_str(&input.to_string(), "invalid.yaml").is_err(),
            "{input}"
        );
        assert!(
            !jsonschema::validator_for(&manifest_schema::generate().unwrap())
                .unwrap()
                .is_valid(&input),
            "{input}"
        );
    }
}

#[test]
fn member_selectors_do_not_infer_incomplete_identity_from_bare_specification_strings() {
    let input = json!({"apiVersion":manifest::API_VERSION,"kind":"Document","metadata":{"namespace":"communication","name":"rationale"},"spec":{"title":{"value":"Rationale"},"subjects":[{"subject":{"member":"messages@1"}}]}});
    assert!(manifest::parse_str(&input.to_string(), "member.yaml").is_err());
    assert!(
        !jsonschema::validator_for(&manifest_schema::generate().unwrap())
            .unwrap()
            .is_valid(&input)
    );
}
