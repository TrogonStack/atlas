#![allow(clippy::unwrap_used)]

use trogon_atlas_client::view;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

fn team() -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Team(pb::Team {
            id: Some(pb::Id {
                namespace: "catalog".into(),
                slug: "team".into(),
                version: 1,
            }),
            title: Some(pb::DirectoryDisplayName {
                value: "Catalog team".into(),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn directory_description_preserves_wrapped_display_name() {
    let description =
        view::describe(&team(), &TranscodePool::builtin().unwrap(), &[], &[]).unwrap();
    assert_eq!(description.title, "Catalog team");
}

#[test]
fn description_preserves_backend_generic_reference_provenance() {
    let types = TranscodePool::builtin().unwrap();
    let reference: pb::Reference = types.message_from_json_value("trogonatlas.api.eventmodel.v1alpha1.Reference", &serde_json::json!({"from":{"kind":"ENTITY_KIND_TEAM","id":{"namespace":"catalog","slug":"team","version":"1"}},"fromUid":"team-uid","fromField":"responsibilities[0]","to":{"kind":"ENTITY_KIND_DATA_PRODUCT","id":{"namespace":"catalog","slug":"product","version":"2"}},"responsibility":{"connectionId":{"value":"assignment-id"},"kind":"RESPONSIBILITY_KIND_STEWARD","subject":{"port":{"product":{"id":{"namespace":"catalog","slug":"product","version":"2"}},"ownerUid":"product-uid","direction":"DATA_PORT_DIRECTION_OUTPUT","portId":{"value":"port-id"}}}}})).unwrap();
    let expected = types
        .message_json_value("trogonatlas.api.eventmodel.v1alpha1.Reference", &reference)
        .unwrap();
    let description = view::describe(&team(), &types, &[reference], &[]).unwrap();
    let json = serde_json::to_value(&description).unwrap();
    assert_eq!(json["outgoing"][0]["provenance"], expected);
    let text = view::render_description(&description);
    assert!(text.contains("assignment-id"));
    assert!(text.contains("product-uid"));
}
