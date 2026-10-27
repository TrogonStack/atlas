#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{apply, export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

fn raw(kind: &str, relationships: &serde_json::Value) -> serde_json::Value {
    json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"architecture","name":"checkout","version":"18446744073709551615"},"spec":{"title":"Checkout","relationships":relationships}})
}
fn declarations() -> serde_json::Value {
    json!([
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000001"},"target":{"externalSystem":"provider@9007199254740993"},"label":{"value":"submits orders"},"doc":{"value":"Cafe\u{301}\r\n<script>inert()</script>"}},
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000002"},"target":{"externalSystem":"provider@9007199254740993"},"label":{"value":"submits orders"},"doc":{"value":"Cafe\u{301}\r\n<script>inert()</script>"}},
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000003"},"target":{"component":"worker@2"},"label":{"value":"contains"}}
    ])
}
#[test]
fn native_relationships_roundtrip_parallel_ids_exact_versions_wording_and_optional_doc() {
    let types = TranscodePool::builtin().unwrap();
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    for kind in ["Component", "ExternalSystem"] {
        let input = raw(kind, &declarations());
        assert!(validator.is_valid(&input));
        let loaded = manifest::parse_str(&input.to_string(), "architecture.yaml").unwrap();
        let entity = &loaded[0].entity;
        let relationships = trogon_atlas_core::architectural_relationships::relationships(entity);
        assert_eq!(relationships.len(), 3);
        assert_ne!(
            relationships[0].connection_id,
            relationships[1].connection_id
        );
        assert_eq!(relationships[2].doc, None);
        let endpoint = trogon_atlas_core::architectural_relationships::endpoint_ref(
            relationships[0].target.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(endpoint.id.unwrap().version, 9_007_199_254_740_993);
        assert_eq!(loaded[0].id.version, u64::MAX);
        let output = export::render_board(std::slice::from_ref(entity), &[], &types).unwrap();
        assert_eq!(output.skipped, []);
        let reparsed = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
        assert_eq!(reparsed[0].entity, *entity);
        assert_eq!(
            apply::classify(&reparsed[0].entity, Some(entity)),
            apply::Action::Unchanged
        );
        let mut reordered = declarations();
        reordered.as_array_mut().unwrap().reverse();
        let changed =
            manifest::parse_str(&raw(kind, &reordered).to_string(), "reordered.yaml").unwrap();
        assert_eq!(
            apply::classify(&changed[0].entity, Some(entity)),
            apply::Action::Configure
        );
    }
}
#[test]
fn native_relationship_structure_rejects_ambiguous_targets_and_unknown_fields() {
    for target in [
        json!("provider@1"),
        json!({"component":"worker","externalSystem":"provider"}),
        json!({"entity":{"kind":"component","id":"worker"}}),
    ] {
        let input = raw(
            "Component",
            &json!([{"target":target,"label":{"value":"uses"}}]),
        );
        assert!(manifest::parse_str(&input.to_string(), "invalid.yaml").is_err());
        assert!(
            !jsonschema::validator_for(&manifest_schema::generate().unwrap())
                .unwrap()
                .is_valid(&input)
        );
    }
}
#[test]
fn architectural_subject_requests_preserve_full_connection_ref_and_reject_coercion() {
    let types = TranscodePool::builtin().unwrap();
    for kind in ["ENTITY_KIND_COMPONENT", "ENTITY_KIND_EXTERNAL_SYSTEM"] {
        let input = json!({"subject":{"connection":{"owner":{"kind":kind,"id":{"namespace":"architecture","slug":"checkout","version":"18446744073709551615"}},"ownerUid":"opaque-owner-UID","id":{"value":"01930000-0000-7000-8000-000000000001"}}}});
        let request: pb::ListSubjectAssignmentsRequest =
            trogon_atlas_client::responsibility::parse_request(
                "ListSubjectAssignmentsRequest",
                &input,
            )
            .unwrap();
        assert_eq!(
            types
                .message_json_value(
                    "trogonatlas.api.eventmodel.v1alpha1.ListSubjectAssignmentsRequest",
                    &request
                )
                .unwrap(),
            input
        );
        for invalid in [
            json!({"subject":{"connection":{"ownerUid":false}}}),
            json!({"subject":{"connection":{},"entity":{}}}),
            json!({"unknown":true}),
        ] {
            assert!(trogon_atlas_client::responsibility::parse_request::<
                pb::ListSubjectAssignmentsRequest,
            >("ListSubjectAssignmentsRequest", &invalid)
            .is_err());
        }
    }
}

#[test]
fn generic_description_retains_incoming_parallel_declaration_provenance() {
    let types = TranscodePool::builtin().unwrap();
    let entity: pb::Entity = types.message_from_json_value("trogonatlas.eventmodel.v1alpha1.Entity", &json!({"externalSystem":{"id":{"namespace":"architecture","slug":"provider","version":"9007199254740993"},"title":"Provider"}})).unwrap();
    let references: Vec<_> = ["01930000-0000-7000-8000-000000000001", "01930000-0000-7000-8000-000000000002"].iter().enumerate().map(|(index, value)| json!({"from":{"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":"architecture","slug":"checkout","version":"18446744073709551615"}},"fromUid":"opaque-owner-UID","fromField":format!("relationships[{index}].target.external_system"),"to":{"kind":"ENTITY_KIND_EXTERNAL_SYSTEM","id":{"namespace":"architecture","slug":"provider","version":"9007199254740993"}},"connectionId":{"value":value}})).collect();
    let typed = references
        .iter()
        .map(|value| {
            types
                .message_from_json_value("trogonatlas.api.eventmodel.v1alpha1.Reference", value)
                .unwrap()
        })
        .collect::<Vec<pb::Reference>>();
    let description = trogon_atlas_client::view::describe(&entity, &types, &[], &typed).unwrap();
    let rendered = serde_json::to_value(description).unwrap();
    assert_eq!(rendered["incoming"].as_array().unwrap().len(), 2);
    for (index, reference) in references.iter().enumerate() {
        assert_eq!(rendered["incoming"][index]["provenance"], *reference);
    }
}
