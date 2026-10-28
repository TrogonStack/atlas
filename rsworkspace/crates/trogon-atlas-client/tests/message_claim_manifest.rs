#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_client::{apply, export, manifest};
use trogon_atlas_core::transcode::TranscodePool;

#[test]
fn independent_message_claims_round_trip_on_every_native_owner() {
    let types = TranscodePool::builtin().unwrap();
    for kind in ["Component", "ExternalSystem", "Domain"] {
        let input = json!({
            "apiVersion": manifest::API_VERSION,
            "kind": kind,
            "metadata": {"namespace":"claims", "name":"owner", "version":"18446744073709551615"},
            "spec": {
                "title":"Independent owner",
                "messageClaims": {
                    "claims": [{
                        "connectionId":{"value":"01930000-0000-7000-8000-000000000001"},
                        "direction":"COMMUNICATION_DIRECTION_SEND",
                        "message": {
                            "specification":"messages@9007199254740993",
                            "ownerUid":"returned-specification-uid",
                            "kind":"CONTRACT_MEMBER_KIND_MESSAGE",
                            "memberId":{"value":"returned-message-id"}
                        }
                    }]
                }
            }
        });
        let parsed = manifest::parse_str(&input.to_string(), "claims.yaml");
        assert!(
            parsed.is_ok(),
            "{kind} must accept independent native claims: {parsed:?}"
        );
        let parsed = parsed.unwrap();
        assert_eq!(parsed[0].id.version, u64::MAX);
        let output = export::render_board(&[parsed[0].entity.clone()], &[], &types).unwrap();
        assert!(output.skipped.is_empty());
        let restored = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
        assert_eq!(restored[0].entity, parsed[0].entity);
        assert_eq!(
            apply::classify(&restored[0].entity, Some(&parsed[0].entity)),
            apply::Action::Unchanged
        );
    }
}

#[test]
fn flat_claim_lists_preserve_exact_selectors_and_optional_delivery() {
    let types = TranscodePool::builtin().unwrap();
    for kind in ["Component", "ExternalSystem", "Domain"] {
        let reference = json!({"specification":"messages@18446744073709551615","ownerUid":"returned-uid","kind":"CONTRACT_MEMBER_KIND_MESSAGE","memberId":{"value":"returned-member-id"}});
        let input = json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"claims","name":"owner"},"spec":{"title":"Owner","messageClaims":{
          "claims":[{"connectionId":{"value":"claim-a"},"direction":"COMMUNICATION_DIRECTION_RECEIVE","message":reference,"group":{"value":"Incoming"}},{"connectionId":{"value":"claim-b"},"direction":"COMMUNICATION_DIRECTION_RECEIVE","message":reference}],
          "routes":[{"connectionId":{"value":"route-a"},"claimId":{"value":"claim-a"},"channel":"channel@9007199254740993","parameterValues":{"tenant":""}},{"connectionId":{"value":"route-b"},"claimId":{"value":"claim-b"},"channel":"channel@9007199254740993","deliveryMode":"CLAIM_DELIVERY_MODE_PUSH_PULL"}],
          "triggers":[{"connectionId":{"value":"trigger-a"},"claimId":{"value":"claim-a"},"message":reference,"condition":{"value":"when documented"}}],
          "fieldSelections":[{"connectionId":{"value":"selection-a"},"claimId":{"value":"claim-a"},"binding":{"owner":{"kind":"ENTITY_KIND_SPECIFICATION","id":"messages@18446744073709551615"},"ownerUid":"returned-uid","id":{"value":"binding-a"}},"selectors":[{"path":{"segments":["customer","name"]}},{"opaque":{"syntax":"json-pointer","value":"/opaque"}}]}]
        }}});
        let parsed = manifest::parse_str(&input.to_string(), "claims.yaml").unwrap();
        let v = trogon_atlas_core::message_claims::declarations(&parsed[0].entity).unwrap();
        assert_eq!(v.claims.len(), 2);
        assert_ne!(v.claims[0].connection_id, v.claims[1].connection_id);
        assert_eq!(v.routes[0].delivery_mode, None);
        assert_eq!(
            v.routes[1].delivery_mode,
            Some(trogon_atlas_proto::ClaimDeliveryMode::PushPull as i32)
        );
        assert_eq!(
            v.routes[0]
                .channel
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .version,
            9_007_199_254_740_993
        );
        let rendered = export::render_board(&[parsed[0].entity.clone()], &[], &types).unwrap();
        let reparsed = manifest::parse_str(&rendered.manifests, "export.yaml").unwrap();
        assert_eq!(reparsed[0].entity, parsed[0].entity);
        assert_eq!(
            apply::classify(&reparsed[0].entity, Some(&parsed[0].entity)),
            apply::Action::Unchanged
        );
        let schema = trogon_atlas_client::manifest_schema::generate().unwrap();
        assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&input));
    }
}
