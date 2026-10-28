#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{decision_document, export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

fn subjects() -> serde_json::Value {
    json!([
        {"connectionId":{"value":"association-a"},"subject":{"entity":{"kind":"read_model","id":"receipt@2"}}},
        {"connectionId":{"value":"association-b"},"subject":{"connection":{"owner":{"kind":"automation_slice","id":"observe@3"},"ownerUid":"owner-uid","id":{"value":"occurrence-b"}}}},
        {"connectionId":{"value":"association-c"},"subject":{"member":{"specification":"contract@4","ownerUid":"spec-uid","kind":"CONTRACT_MEMBER_KIND_ARTIFACT","memberId":{"value":"artifact-id"},"expectedDigest":{"sha256":"a".repeat(64)}}}},
        {"connectionId":{"value":"association-d"},"subject":{"port":{"product":"product@5","ownerUid":"product-uid","direction":"DATA_PORT_DIRECTION_OUTPUT","portId":{"value":"port-id"}}}}
    ])
}
fn manifest(kind: &str, spec: &serde_json::Value) -> serde_json::Value {
    json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"catalog","name":"note","version":2},"spec":spec})
}
#[test]
fn native_catalog_manifests_preserve_bytes_pins_local_ids_and_authored_order() {
    let anchor = json!({"anchor":{"document":"notes/delivery@7","ownerUid":"document-uid","anchorId":{"value":"anchor-id"},"expectedDigest":{"sha256":"b".repeat(64)}}});
    let decision = json!({"title":{"value":"Choice"},"supersedes":"note@1","subjects":subjects(),"status":"DECISION_STATUS_ACCEPTED","decisionMakers":[{"person":"maker@2"},{"team":"makers@3"}],"amends":[{"decision":"original@4","doc":{"value":"Clarifies original"}}],"authoredOn":{"value":"2024-02-29"},"decidedOn":{"value":"2024-03-01"},"context":{"value":"Question"},"rationale":{"value":"Rationale"},"options":[{"optionId":{"value":"option-b"},"title":{"value":"B"},"description":{"value":"Alternative"}},{"optionId":{"value":"option-a"},"title":{"value":"A"},"description":{"value":"Alternative A"}}],"selectedOption":{"value":"option-b"},"consequences":[{"kind":"KIND_RISK","text":{"value":"Delivery risk"}}],"documents":[{"connectionId":{"value":"support-id"},"document":anchor}]});
    let text = "# Delivery\r\n\nCafe\u{301} <script>blocked()</script>\n";
    let document = json!({"title":{"value":"Notes"},"subjects":subjects(),"retained":{"mediaType":{"value":"text/markdown"},"content":text,"digest":{"sha256":"c".repeat(64)}},"anchors":[{"anchorId":{"value":"anchor-b"},"fragment":{"value":"Delivery"},"label":{"value":"Delivery"}}],"navigation":[{"connectionId":{"value":"navigation-id"},"document":anchor}]});
    let schema = manifest_schema::generate().unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for raw in [
        manifest("Decision", &decision),
        manifest("Document", &document),
        manifest(
            "Document",
            &json!({"title":{"value":"External"},"subjects":subjects(),"link":{"value":"https://example.test/notes"}}),
        ),
    ] {
        assert!(validator.is_valid(&raw));
        let loaded = manifest::parse_str(&raw.to_string(), "catalog.yaml").unwrap();
        let output = export::render_board(
            std::slice::from_ref(&loaded[0].entity),
            &[],
            &TranscodePool::builtin().unwrap(),
        )
        .unwrap();
        assert_eq!(output.skipped, []);
        let reparsed = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
        assert_eq!(reparsed[0].entity, loaded[0].entity);
        if let Some(pb::entity::Kind::Document(doc)) = &loaded[0].entity.kind {
            if let Some(pb::document::Body::Retained(body)) = &doc.body {
                assert_eq!(body.content.as_bytes(), text.as_bytes());
            }
        }
    }
}
#[test]
fn catalog_selectors_require_explicit_alternatives_in_manifest_and_schema() {
    for spec in [
        json!({"subjects":[{"subject":"receipt@2"}]}),
        json!({"navigation":[{"document":"note@2"}]}),
    ] {
        let raw = manifest("Document", &spec);
        assert!(manifest::parse_str(&raw.to_string(), "invalid.yaml").is_err());
        assert!(
            !jsonschema::validator_for(&manifest_schema::generate().unwrap())
                .unwrap()
                .is_valid(&raw)
        );
    }
}
#[test]
fn native_query_json_retains_full_selection_and_optional_filter_presence() {
    let raw = json!({"document":{"anchor":{"document":{"id":{"namespace":"notes","slug":"delivery","version":"9007199254740993"}},"ownerUid":"incarnation","anchorId":{"value":"anchor-b"},"expectedDigest":{"sha256":"d".repeat(64)}}}});
    let request: pb::GetDocumentDetailRequest =
        decision_document::parse_request("GetDocumentDetailRequest", &raw).unwrap();
    let value = TranscodePool::builtin()
        .unwrap()
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetDocumentDetailRequest",
            &request,
        )
        .unwrap();
    assert_eq!(value, raw);
    for invalid in [
        json!({"unknown":true}),
        json!({"document":{"document":{},"anchor":{}}}),
        json!({"document":{"anchor":{"ownerUid":false}}}),
    ] {
        assert!(
            decision_document::parse_request::<pb::GetDocumentDetailRequest>(
                "GetDocumentDetailRequest",
                &invalid
            )
            .is_err()
        );
    }
    let absent: pb::ListSubjectDecisionsRequest =
        decision_document::parse_request("ListSubjectDecisionsRequest", &json!({})).unwrap();
    let present: pb::ListSubjectDecisionsRequest = decision_document::parse_request(
        "ListSubjectDecisionsRequest",
        &json!({"status":"DECISION_STATUS_PROPOSED"}),
    )
    .unwrap();
    assert_eq!(absent.status, None);
    assert_eq!(present.status, Some(pb::DecisionStatus::Proposed as i32));
}

#[test]
fn generic_description_preserves_catalog_association_provenance_and_nested_pins() {
    let types = TranscodePool::builtin().unwrap();
    let reference_json = json!({"from":{"kind":"ENTITY_KIND_DECISION","id":{"namespace":"catalog","slug":"choice","version":"1"}},"fromUid":"owner-incarnation","fromField":"documents","to":{"kind":"ENTITY_KIND_DOCUMENT","id":{"namespace":"catalog","slug":"note","version":"2"}},"connectionId":{"value":"support-occurrence"},"documentSelection":{"anchor":{"document":{"id":{"namespace":"catalog","slug":"note","version":"2"}},"ownerUid":"document-incarnation","anchorId":{"value":"anchor-b"},"expectedDigest":{"sha256":"e".repeat(64)}}}});
    let reference: pb::Reference = types
        .message_from_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.Reference",
            &reference_json,
        )
        .unwrap();
    let decision: pb::Entity = types.message_from_json_value("trogonatlas.eventmodel.v1alpha1.Entity",&json!({"decision":{"id":{"namespace":"catalog","slug":"choice","version":"1"},"title":{"value":"Choice"}}})).unwrap();
    let description =
        trogon_atlas_client::view::describe(&decision, &types, &[reference], &[]).unwrap();
    let rendered = serde_json::to_value(description).unwrap();
    assert_eq!(rendered["outgoing"][0]["provenance"], reference_json);
    assert_eq!(rendered["title"], "Choice");
}

#[test]
fn unchanged_native_catalog_reapply_classifies_as_unchanged_without_stripping_edits() {
    let subject = json!({"subject":{"entity":{"kind":"ENTITY_KIND_EVENT","id":"receipt"}}});
    for (kind, spec) in [
        (
            "Decision",
            json!({"title":{"value":"Choice"},"subjects":[subject],"status":"DECISION_STATUS_PROPOSED","context":{"value":"Question"},"rationale":{"value":"Reason"},"options":[{"title":{"value":"Retain"},"description":{"value":"Retain notes"}}]}),
        ),
        (
            "Document",
            json!({"title":{"value":"Note"},"subjects":[subject],"retained":{"mediaType":{"value":"text/plain"},"content":"note","digest":{"sha256":"edb465624291e4053c6c5ea4b7eb320dec773e10a57d26b95dcf0564f8e310f8"}},"anchors":[{"fragment":{"value":"Delivery"},"label":{"value":"Delivery"}}]}),
        ),
    ] {
        let desired = manifest::parse_str(&manifest(kind, &spec).to_string(), "catalog.yaml")
            .unwrap()
            .remove(0)
            .entity;
        let stored =
            trogon_atlas_core::connection_persistence::prepare_entity(None, &desired).unwrap();
        assert_eq!(
            trogon_atlas_client::apply::classify(&desired, Some(&stored)),
            trogon_atlas_client::apply::Action::Unchanged,
            "{kind}"
        );
        let mut changed_spec = spec;
        changed_spec["title"] = json!({"value":"Changed wording"});
        let changed =
            manifest::parse_str(&manifest(kind, &changed_spec).to_string(), "changed.yaml")
                .unwrap()
                .remove(0)
                .entity;
        assert_eq!(
            trogon_atlas_client::apply::classify(&changed, Some(&stored)),
            trogon_atlas_client::apply::Action::Configure,
            "changed {kind}"
        );
    }
}

#[test]
fn typed_delete_report_distinguishes_incomplete_empty_reporting_from_complete_absence() {
    use prost::Message as _;
    let response = pb::DeleteEntityResponse {
        operation_receipt: None,
        referrers: vec![],
        referrers_incomplete: true,
        validation: vec![],
    };
    let decoded = pb::DeleteEntityResponse::decode(response.encode_to_vec().as_slice()).unwrap();
    assert!(decoded.referrers_incomplete);
    let json = TranscodePool::builtin()
        .unwrap()
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.DeleteEntityResponse",
            &decoded,
        )
        .unwrap();
    assert_eq!(json["referrersIncomplete"], true);
}
