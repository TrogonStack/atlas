#![allow(clippy::unwrap_used)]

use serde_json::{json, Value};
use trogon_atlas_client::{export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

fn component(hosting: Value) -> Value {
    json!({
        "apiVersion": manifest::API_VERSION,
        "kind": "Component",
        "metadata": {"namespace": "shop", "name": "frontend"},
        "spec": {"title": "Frontend", "uiHostings": [hosting]}
    })
}

#[test]
fn schema_rejects_bare_hosting_that_omits_authored_occurrence_fields() {
    let schema = manifest_schema::generate().unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(!validator.is_valid(&component(json!("shop/review@1"))));
}

#[test]
fn parser_rejects_bare_hosting_that_omits_authored_occurrence_fields() {
    let parsed = manifest::parse_str(
        &component(json!("shop/review@1")).to_string(),
        "hosting.yaml",
    );
    assert!(parsed.is_err(), "bare hosting was accepted: {parsed:?}");
}

#[test]
fn object_hosting_preserves_ui_pin_label_and_occurrence_identity() {
    let document = component(json!({
        "ui": "shop/review@2",
        "label": {"value": "Customer review"},
        "connectionId": {"value": "review-hosting"}
    }));
    let schema = manifest_schema::generate().unwrap();
    assert!(jsonschema::validator_for(&schema)
        .unwrap()
        .is_valid(&document));
    let resources = manifest::parse_str(&document.to_string(), "hosting.yaml").unwrap();
    let entity = &resources[0].entity;
    let Some(pb::entity::Kind::Component(body)) = &entity.kind else {
        panic!("expected Component");
    };
    let hosting = &body.ui_hostings[0];
    assert_eq!(
        hosting.ui.as_ref().unwrap().id,
        Some(pb::Id {
            namespace: "shop".into(),
            slug: "review".into(),
            version: 2,
        })
    );
    assert_eq!(hosting.label.as_ref().unwrap().value, "Customer review");
    assert_eq!(
        hosting.connection_id.as_ref().unwrap().value,
        "review-hosting"
    );
    let types = TranscodePool::builtin().unwrap();
    let rendered = export::render_board(std::slice::from_ref(entity), &[], &types).unwrap();
    assert_eq!(
        manifest::parse_str(&rendered.manifests, "export.yaml").unwrap()[0].entity,
        *entity
    );
    let ui_ref = &schema["$defs"]["trogonatlas.eventmodel.v1alpha1.UIRef"]["oneOf"];
    assert!(ui_ref
        .as_array()
        .unwrap()
        .iter()
        .any(|alternative| { alternative["$ref"] == "#/$defs/_ShorthandRef" }));
}
