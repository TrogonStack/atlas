#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{
    export,
    failure::{Failure, FailureCategory},
    manifest, manifest_schema, responsibility,
};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

#[test]
fn responsibility_native_manifest_preserves_profile_presence_and_all_exact_selectors() {
    let id = json!({"namespace":"catalog", "slug":"selected", "version":"2"});
    let owner = json!({"kind":"ENTITY_KIND_SPECIFICATION", "id":id});
    let selectors = [
        json!({"entity":owner}),
        json!({"connection":{"owner":owner,"ownerUid":"spec-uid","id":{"value":"connection-id"}}}),
        json!({"member":{"specification":{"id":id},"ownerUid":"spec-uid","kind":"CONTRACT_MEMBER_KIND_ARTIFACT","memberId":{"value":"member-id"},"expectedDigest":{"sha256":"a".repeat(64)}}}),
        json!({"port":{"product":{"id":id},"ownerUid":"product-uid","direction":"DATA_PORT_DIRECTION_OUTPUT","portId":{"value":"port-id"}}}),
    ];
    let assignments: Vec<_> = selectors.iter().enumerate().map(|(i,subject)| json!({"connectionId":{"value":format!("assignment-{i}")},"kind":"RESPONSIBILITY_KIND_OWNER","subject":subject})).collect();
    for (kind, extra) in [
        (
            "Team",
            json!({"purpose":"Catalog stewardship", "members":[{"connectionId":{"value":"membership-id"},"person":"person@3"}]}),
        ),
        (
            "Person",
            json!({"role":{"value":"Directory role"},"directoryPointer":{"provider":{"value":"catalog"},"key":{"value":"opaque-key"}}}),
        ),
    ] {
        let mut spec = json!({"title":{"value":"Retained profile"},"contacts":[{"email":{"value":"catalog@example.test"},"label":"Work"},{"web":{"value":"https://example.test/contact"}},{"chat":{"service":{"value":"chat"},"handle":{"value":"opaque-handle"}}}],"responsibilities":assignments});
        spec.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let raw = json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"catalog","name":"profile","version":1},"spec":spec});
        assert!(
            jsonschema::validator_for(&manifest_schema::generate().unwrap())
                .unwrap()
                .is_valid(&raw)
        );
        let loaded = manifest::parse_str(&raw.to_string(), "profile.yaml").unwrap();
        let types = TranscodePool::builtin().unwrap();
        let output =
            export::render_board(std::slice::from_ref(&loaded[0].entity), &[], &types).unwrap();
        assert_eq!(output.skipped, []);
        assert_eq!(
            manifest::parse_str(&output.manifests, "export.yaml").unwrap()[0].entity,
            loaded[0].entity
        );
    }
}

#[test]
fn responsibility_selector_requires_explicit_typed_alternative_in_parser_and_schema() {
    let raw = json!({"apiVersion":manifest::API_VERSION,"kind":"Team","metadata":{"namespace":"catalog","name":"profile"},"spec":{"title":{"value":"Team"},"responsibilities":[{"kind":"RESPONSIBILITY_KIND_OWNER","subject":"selected@2"}]}});
    let error = manifest::parse_str(&raw.to_string(), "profile.yaml").unwrap_err();
    assert_eq!(
        Failure::classify(&error).category,
        FailureCategory::Validation
    );
    assert!(
        !jsonschema::validator_for(&manifest_schema::generate().unwrap())
            .unwrap()
            .is_valid(&raw)
    );
}

#[test]
fn typed_query_json_preserves_selector_and_rejects_unknown_or_malformed_fields() {
    let request: pb::ListSubjectAssignmentsRequest = responsibility::parse_request("ListSubjectAssignmentsRequest", &json!({"subject":{"port":{"product":{"id":{"namespace":"catalog","slug":"product","version":"2"}},"ownerUid":"uid","direction":"DATA_PORT_DIRECTION_INPUT","portId":{"value":"port-id"}}},"kind":"RESPONSIBILITY_KIND_CONTACT"})).unwrap();
    assert_eq!(request.kind, Some(pb::ResponsibilityKind::Contact as i32));
    assert!(matches!(
        request.subject.unwrap().selection,
        Some(pb::responsibility_subject::Selection::Port(_))
    ));
    for raw in [
        json!({"unknown":true}),
        json!({"subject":{"member":{"kind":"invented"}}}),
        json!({"subject":{"entity":{},"port":{}}}),
    ] {
        let error = responsibility::parse_request::<pb::ListSubjectAssignmentsRequest>(
            "ListSubjectAssignmentsRequest",
            &raw,
        )
        .unwrap_err();
        assert_eq!(
            Failure::classify(&error).category,
            FailureCategory::Validation
        );
    }
}
