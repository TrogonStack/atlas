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
