#![allow(clippy::unwrap_used)]

use serde_json::{json, Value};
use trogon_atlas_client::{export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;

fn offering(access: &Value) -> String {
    json!({"apiVersion": manifest::API_VERSION, "kind": "DataProduct",
        "metadata": {"namespace": "shop", "name": "offering"},
        "spec": {"title": "Offering", "outputs": [{"name": {"value": "Output"}, "data": {"readModel": "receipt@2"}, "access": [access]}]}
    }).to_string()
}

#[test]
fn channel_access_rejects_bare_string_that_cannot_identify_binding_and_observation() {
    let result = manifest::parse_str(&offering(&json!({"channel": "transport@3"})), "access.yaml");
    assert!(
        result.is_err(),
        "bare ChannelAccess was accepted: {result:?}"
    );
}

#[test]
fn store_access_rejects_bare_string_that_cannot_identify_attachment() {
    let result = manifest::parse_str(&offering(&json!({"store": "cache@4"})), "access.yaml");
    assert!(result.is_err(), "bare StoreAccess was accepted: {result:?}");
}

#[test]
fn manifest_schema_does_not_offer_bare_strings_for_composite_access() {
    let schema = manifest_schema::generate().unwrap();
    for name in ["ChannelAccess", "StoreAccess"] {
        let definition = &schema["$defs"][format!("trogonatlas.eventmodel.v1alpha1.{name}")];
        assert_eq!(definition["type"], "object", "{name}: {definition}");
        assert!(definition.get("oneOf").is_none(), "{name}: {definition}");
    }
}

#[test]
fn object_access_preserves_pins_and_thin_reference_shorthand() {
    let connection = |kind, owner, occurrence| json!({"owner": {"kind": kind, "id": owner}, "ownerUid": "owner-incarnation", "id": {"value": occurrence}});
    for access in [
        json!({"channel": {"channel": "transport@3", "binding": connection("channel", "transport@3", "binding-id"), "observation": connection("automation_slice", "observe@5", "observation-id")}}),
        json!({"store": {"store": "cache@4", "attachment": connection("data_store", "cache@4", "attachment-id")}}),
    ] {
        let resources = manifest::parse_str(&offering(&access), "access.yaml").unwrap();
        let types = TranscodePool::builtin().unwrap();
        let original = &resources[0].entity;
        let rendered = export::render_board(std::slice::from_ref(original), &[], &types).unwrap();
        assert_eq!(
            manifest::parse_str(&rendered.manifests, "export.yaml").unwrap()[0].entity,
            *original
        );
    }
    let schema = manifest_schema::generate().unwrap();
    for name in [
        "ChannelRef",
        "DataStoreRef",
        "ReadModelRef",
        "ReadModelEdge",
    ] {
        let alternatives = schema["$defs"][format!("trogonatlas.eventmodel.v1alpha1.{name}")]
            ["oneOf"]
            .as_array()
            .unwrap();
        assert!(alternatives
            .iter()
            .any(|value| value["$ref"] == "#/$defs/_ShorthandRef"));
    }
}
