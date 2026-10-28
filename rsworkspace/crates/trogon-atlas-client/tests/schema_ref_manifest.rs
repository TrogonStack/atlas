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

#[test]
fn schema_ref_shorthand_fills_namespace_and_version_without_inventing_a_connection() {
    let raw = json!({
        "apiVersion": manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": "catalog", "name": "order"},
        "spec": {
            "aggregateRoot": false,
            "identifiers": [{
                "property": {"segments": [
                    {"name": "customer", "repeated": false},
                    {"name": "id", "repeated": false}
                ]},
                "role": "BUSINESS_IDENTIFIER_ROLE_PRIMARY"
            }],
            "propertyRelationships": [{
                "source": {"segments": [{"name": "product_ref", "repeated": false}]},
                "targetEntity": "product",
                "label": {"value": "hasMany"}
            }]
        }
    });
    let loaded = manifest::parse_str(&raw.to_string(), "schema-business.yaml").unwrap();
    let pb::entity::Kind::Schema(schema) = loaded[0].entity.kind.as_ref().unwrap() else {
        panic!("schema");
    };
    assert_eq!(schema.aggregate_root, Some(false));
    assert!(schema.identifiers[0].connection_id.is_none());
    assert_eq!(schema.identifiers[0].property.as_ref().unwrap().segments[0].repeated, false);
    let target = schema.property_relationships[0]
        .target_entity
        .as_ref()
        .unwrap()
        .id
        .as_ref()
        .unwrap();
    assert_eq!(target.namespace, "catalog");
    assert_eq!(target.slug, "product");
    assert_eq!(target.version, 1);
    assert!(schema.property_relationships[0].connection_id.is_none());

    let pinned = json!({
        "apiVersion": manifest::API_VERSION,
        "kind": "Schema",
        "metadata": {"namespace": "catalog", "name": "order", "version": "9007199254740993"},
        "spec": {
            "propertyRelationships": [{
                "source": {"segments": [{"name": "product_ref", "repeated": true}]},
                "targetEntity": "other/product@9007199254740993"
            }]
        }
    });
    let loaded = manifest::parse_str(&pinned.to_string(), "schema-business-pin.yaml").unwrap();
    let pb::entity::Kind::Schema(schema) = loaded[0].entity.kind.as_ref().unwrap() else {
        panic!("schema");
    };
    assert_eq!(loaded[0].id.version, 9_007_199_254_740_993);
    let target = schema.property_relationships[0]
        .target_entity
        .as_ref()
        .unwrap()
        .id
        .as_ref()
        .unwrap();
    assert_eq!(target.namespace, "other");
    assert_eq!(target.slug, "product");
    assert_eq!(target.version, 9_007_199_254_740_993);
    assert!(schema.property_relationships[0].source.as_ref().unwrap().segments[0].repeated);
    assert!(schema.property_relationships[0].connection_id.is_none());
}
