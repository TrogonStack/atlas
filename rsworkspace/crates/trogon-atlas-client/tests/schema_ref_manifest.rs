#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{manifest, manifest_schema};
use trogon_atlas_core::refs::outbound_refs;
use trogon_atlas_proto as pb;

#[test]
fn schema_type_ref_manifest_preserves_pin_and_validates_against_committed_schema() {
    let raw = json!({
        "apiVersion": manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": "catalog", "name": "order"},
        "spec": {"fields": [{
            "name": "source",
            "type": {"ref": {"ref": {"schema": "catalog/source@3"}}}
        }]}
    });
    let loaded = manifest::parse_str(&raw.to_string(), "schema-ref.yaml").unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].kind, pb::EntityKind::Schema);
    let refs = outbound_refs(&loaded[0].entity);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].to_kind, Some(pb::EntityKind::Schema));
    assert_eq!(
        refs[0].to_id,
        pb::Id {
            namespace: "catalog".into(),
            slug: "source".into(),
            version: 3,
        }
    );
    assert_eq!(refs[0].from_field, "fields[0].ref");

    let generated = manifest_schema::generate().unwrap();
    jsonschema::validator_for(&generated)
        .unwrap()
        .validate(&raw)
        .unwrap();
    let committed = serde_json::from_str(include_str!("../manifest.schema.json")).unwrap();
    jsonschema::validator_for(&committed)
        .unwrap()
        .validate(&raw)
        .unwrap();
}
