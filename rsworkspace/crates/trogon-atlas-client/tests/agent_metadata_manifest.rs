#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;

fn fixture() -> serde_json::Value {
    let target = json!({"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":"catalog","slug":"target","version":"18446744073709551615"}});
    let raw = json!({"apiVersion":manifest::API_VERSION,"kind":"Component","metadata":{"namespace":"catalog","name":"agent","version":"9007199254740993"},"spec":{"title":"Agent","metadata":[
        {"@type":"type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation","model":{"version":{"value":"opaque preview"}},"tools":[{"name":{"value":"lookup"},"type":{"value":"manual"},"description":{"value":"Inert description"},"icon":{"value":"unknown:token"},"url":{"value":"https://example.test/tool"}},{"name":{"value":"lookup"},"type":{"value":"other"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation","status":"draft","draft":{"message":{"value":"Review required"}}},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.BadgesAnnotation","badges":[{"content":{"value":"Preview"},"backgroundColor":{"value":"#aAbBcC"},"textColor":{"value":"#FFFFFF"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation","groups":[{"groupId":{"value":"10000000-0000-4000-8000-000000000001"},"displayLimit":{"value":1},"sidebar":false,"items":[{"connectionId":{"value":"10000000-0000-4000-8000-000000000002"},"target":target},{"connectionId":{"value":"10000000-0000-4000-8000-000000000003"},"target":target}]}]}
    ]}});
    raw
}

#[test]
fn expanded_native_metadata_preserves_presence_order_and_exact_occurrences() {
    let raw = fixture();
    assert!(
        jsonschema::validator_for(&manifest_schema::generate().unwrap())
            .unwrap()
            .is_valid(&raw)
    );
    let loaded = manifest::parse_str(&raw.to_string(), "agent.yaml").unwrap();
    let types = TranscodePool::builtin().unwrap();
    let output =
        export::render_board(std::slice::from_ref(&loaded[0].entity), &[], &types).unwrap();
    assert_eq!(output.skipped, []);
    assert_eq!(
        manifest::parse_str(&output.manifests, "export.yaml").unwrap()[0].entity,
        loaded[0].entity
    );
    assert!(output.manifests.contains("18446744073709551615"));
    assert!(output.manifests.contains("sidebar: false"));
}

#[test]
fn inspector_retains_expanded_metadata_and_refuses_missing_type_context() {
    let raw = fixture();
    let entity = &manifest::parse_str(&raw.to_string(), "agent.yaml").unwrap()[0].entity;
    let types = TranscodePool::builtin().unwrap();
    let description = trogon_atlas_client::view::describe(entity, &types, &[], &[]).unwrap();
    assert_eq!(
        description.metadata,
        raw["spec"]["metadata"].as_array().unwrap().clone()
    );
    assert!(trogon_atlas_client::view::render_description(&description).contains("opaque preview"));
    let mut unknown = raw;
    unknown["spec"]["metadata"]
        .as_array_mut()
        .unwrap()
        .push(json!({"@type":"type.googleapis.com/catalog.notes.Note","text":"retained"}));
    assert!(manifest::parse_str(&unknown.to_string(), "unknown.yaml").is_err());
}

#[test]
fn inspector_rejects_corrupt_known_any_and_unavailable_tenant_any() {
    let original = manifest::parse_str(&fixture().to_string(), "agent.yaml")
        .unwrap()
        .remove(0)
        .entity;
    for (type_url, value) in [
        (
            "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation",
            vec![255],
        ),
        ("type.googleapis.com/catalog.notes.Note", vec![]),
    ] {
        let mut entity = original.clone();
        let trogon_atlas_proto::entity::Kind::Component(body) = entity.kind.as_mut().unwrap()
        else {
            panic!("expected component")
        };
        body.metadata = vec![prost_types::Any {
            type_url: type_url.into(),
            value,
        }];
        assert!(trogon_atlas_client::view::describe(
            &entity,
            &TranscodePool::builtin().unwrap(),
            &[],
            &[]
        )
        .is_err());
    }
}
