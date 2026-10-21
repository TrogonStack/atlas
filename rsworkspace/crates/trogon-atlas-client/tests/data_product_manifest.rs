#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{export, manifest};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

#[test]
fn data_product_manifest_roundtrip_preserves_exact_pins_presence_and_repeated_occurrences() {
    let id = json!({"namespace": "contracts", "slug": "api", "version": "3"});
    let operation = json!({"specification": {"id": id}, "ownerUid": "spec-uid", "kind": "CONTRACT_MEMBER_KIND_OPERATION", "memberId": {"value": "operation-id"}});
    let connection = json!({"owner": {"kind": "ENTITY_KIND_SPECIFICATION", "id": id}, "ownerUid": "spec-uid", "id": {"value": "response-id"}});
    let manifest = json!({
        "apiVersion": manifest::API_VERSION, "kind": "DataProduct", "metadata": {"namespace": "contracts", "name": "offering", "version": 2},
        "spec": {"title": "Offering", "purpose": "Receipt access", "inputs": [{"portId": {"value": "input-port"}, "name": {"value": "Input"}, "data": {"connectionId": {"value": "input-selection"}, "readModel": "contracts/input@4"}}],
        "outputs": [
            {"portId": {"value": "output-port"}, "name": {"value": "Receipt"}, "data": {"connectionId": {"value": "output-selection"}, "readModel": "contracts/receipt@5"},
                "contract": {"connectionId": {"value": "contract-selection"}, "contractType": {"schema": {"specification": {"id": id}, "ownerUid": "spec-uid", "kind": "CONTRACT_MEMBER_KIND_SCHEMA", "memberId": {"value": "schema-id"}}, "expectedArtifactId": {"value": "artifact-id"}, "expectedDigest": {"sha256": "a".repeat(64)}, "expectedSelector": {"syntax": "graphql-type", "value": "Order"}}},
                "access": [
                    {"connectionId": {"value": "access-a"}, "operation": {"operation": operation, "response": connection}},
                    {"connectionId": {"value": "access-b"}, "operation": {"operation": operation, "response": connection, "providerExposure": connection}},
                    {"connectionId": {"value": "access-c"}, "operation": {"operation": operation, "response": connection}}
                ], "derivedFrom": {"inputs": [{"connectionId": {"value": "lineage-selection"}, "inputPortId": {"value": "input-port"}}]}},
            {"name": {"value": "Explicit empty"}, "derivedFrom": {}},
            {"name": {"value": "Unknown lineage"}},
            {"name": {"value": "Native shape"}, "contract": {"schema": "contracts/shape@6"}},
            {"name": {"value": "Protobuf shape"}, "contract": {"libraryType": {"library": "contracts/types@7", "message": {"value": "fixture.Order"}}}}
        ]}
    });
    let resources = manifest::parse_str(&manifest.to_string(), "offering.yaml").unwrap();
    assert_eq!(resources[0].kind, pb::EntityKind::DataProduct);
    let types = TranscodePool::builtin().unwrap();
    let original = &resources[0].entity;
    let exported = export::render_board(std::slice::from_ref(original), &[], &types).unwrap();
    assert_eq!(exported.skipped, [] as [pb::SkippedEntity; 0]);
    assert_eq!(
        manifest::parse_str(&exported.manifests, "export.yaml").unwrap()[0].entity,
        *original
    );
    let Some(pb::entity::Kind::DataProduct(product)) = &original.kind else {
        panic!("expected DataProduct")
    };
    assert!(product.outputs[1].derived_from.is_some());
    assert!(product.outputs[2].derived_from.is_none());
    assert_eq!(product.outputs[0].access.len(), 3);
    let Some(pb::output_access::Selection::Operation(access)) =
        &product.outputs[0].access[0].selection
    else {
        panic!("expected operation")
    };
    assert!(access.provider_exposure.is_none());
}
