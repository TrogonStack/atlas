#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{apply, export, manifest, manifest_schema};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

fn product_manifest() -> serde_json::Value {
    let id = json!({"namespace": "contracts", "slug": "api", "version": "18446744073709551615"});
    json!({
        "apiVersion": manifest::API_VERSION,
        "kind": "DataProduct",
        "metadata": {"namespace": "contracts", "name": "offering", "version": "18446744073709551615"},
        "spec": {
            "title": "Offering",
            "outputs": [{"name": {"value": "Receipt"}, "data": {"readModel": {"id": {"namespace": "contracts", "slug": "receipt", "version": "2"}}}}],
            "resourceAssociations": [
                {
                    "connectionId": {"value": "01930000-0000-7000-8000-000000000001"},
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "event": {"id": {"namespace": "contracts", "slug": "placed", "version": "9007199254740993"}},
                    "contract": {"schema": {"id": {"namespace": "contracts", "slug": "shape", "version": "4"}}},
                    "doc": "authored"
                },
                {
                    "connectionId": {"value": "01930000-0000-7000-8000-000000000002"},
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "event": {"id": {"namespace": "contracts", "slug": "placed", "version": "9007199254740993"}}
                },
                {
                    "connectionId": {"value": "01930000-0000-7000-8000-000000000003"},
                    "direction": "DATA_PORT_DIRECTION_INPUT",
                    "command": {"id": {"namespace": "contracts", "slug": "place", "version": "1"}}
                },
                {
                    "connectionId": {"value": "01930000-0000-7000-8000-000000000004"},
                    "direction": "DATA_PORT_DIRECTION_OUTPUT",
                    "message": {
                        "specification": {"id": id},
                        "ownerUid": "spec-uid",
                        "kind": "CONTRACT_MEMBER_KIND_MESSAGE",
                        "memberId": {"value": "22222222-2222-4222-8222-222222222222"}
                    }
                }
            ]
        }
    })
}

fn specification_manifest() -> serde_json::Value {
    json!({
        "apiVersion": manifest::API_VERSION,
        "kind": "Specification",
        "metadata": {"namespace": "contracts", "name": "contract", "version": "18446744073709551615"},
        "spec": {
            "title": "Contract",
            "messages": [{
                "memberId": {"value": "11111111-1111-4111-8111-111111111111"},
                "name": {"value": "Order"},
                "schemas": [
                    {
                        "connectionId": {"value": "01930000-0000-7000-8000-000000000011"},
                        "role": "ROLE_PAYLOAD",
                        "localSchema": {"value": "33333333-3333-4333-8333-333333333333"},
                        "environments": [{"value": "a".repeat(64)}],
                        "isDefault": false
                    },
                    {
                        "connectionId": {"value": "01930000-0000-7000-8000-000000000012"},
                        "role": "ROLE_HEADERS",
                        "localSchema": {"value": "33333333-3333-4333-8333-333333333333"},
                        "environments": [{"value": "a".repeat(64)}]
                    }
                ]
            }],
            "messageChannels": [
                {
                    "connectionId": {"value": "01930000-0000-7000-8000-000000000021"},
                    "memberId": {"value": "11111111-1111-4111-8111-111111111111"},
                    "channel": {"id": {"namespace": "contracts", "slug": "orders", "version": "1"}},
                    "parameterValues": {"topic": "orders", "note": ""},
                    "doc": "carried"
                },
                {
                    "connectionId": {"value": "01930000-0000-7000-8000-000000000022"},
                    "memberId": {"value": "11111111-1111-4111-8111-111111111111"},
                    "channel": {"id": {"namespace": "contracts", "slug": "orders", "version": "1"}},
                    "parameterValues": {"topic": "orders", "note": ""},
                    "deliveryMode": "CLAIM_DELIVERY_MODE_PUSH"
                }
            ]
        }
    })
}

#[test]
fn manifest_roundtrip_keeps_presence_equal_siblings_and_uint64_versions() {
    let types = TranscodePool::builtin().unwrap();
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    for input in [product_manifest(), specification_manifest()] {
        assert!(validator.is_valid(&input), "{input}");
        let loaded = manifest::parse_str(&input.to_string(), "native.yaml").unwrap();
        assert_eq!(loaded[0].id.version, u64::MAX);
        let entity = &loaded[0].entity;
        let output = export::render_board(std::slice::from_ref(entity), &[], &types).unwrap();
        assert_eq!(output.skipped, []);
        let reparsed = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
        assert_eq!(reparsed[0].entity, *entity);
        assert_eq!(
            apply::classify(&reparsed[0].entity, Some(entity)),
            apply::Action::Unchanged
        );
    }

    let product = manifest::parse_str(&product_manifest().to_string(), "product.yaml").unwrap();
    let Some(pb::entity::Kind::DataProduct(product)) = &product[0].entity.kind else {
        panic!("product");
    };
    assert_eq!(product.resource_associations.len(), 4);
    assert_ne!(
        product.resource_associations[0].connection_id,
        product.resource_associations[1].connection_id
    );
    assert!(product.resource_associations[0].contract.is_some());
    assert!(product.resource_associations[1].contract.is_none());
    let Some(pb::product_resource_association::Target::Message(message)) =
        &product.resource_associations[3].target
    else {
        panic!("message target");
    };
    assert_eq!(
        message
            .specification
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        u64::MAX
    );
    assert_eq!(message.kind, pb::ContractMemberKind::Message as i32);

    let specification =
        manifest::parse_str(&specification_manifest().to_string(), "spec.yaml").unwrap();
    let Some(pb::entity::Kind::Specification(specification)) = &specification[0].entity.kind else {
        panic!("specification");
    };
    assert_eq!(specification.messages[0].schemas[0].is_default, Some(false));
    assert!(specification.messages[0].schemas[1].is_default.is_none());
    assert_eq!(
        specification.messages[0].schemas[0].environments[0]
            .value
            .len(),
        64
    );
    assert!(specification.message_channels[0].delivery_mode.is_none());
    assert_eq!(
        specification.message_channels[1].delivery_mode,
        Some(pb::ClaimDeliveryMode::Push as i32)
    );
    assert_eq!(
        specification.message_channels[0]
            .parameter_values
            .get("note")
            .map(String::as_str),
        Some("")
    );
    assert_ne!(
        specification.message_channels[0].connection_id,
        specification.message_channels[1].connection_id
    );
}
