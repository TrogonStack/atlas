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

fn reference() -> serde_json::Value {
    json!({"specification":"messages@1","ownerUid":"spec-uid","kind":"CONTRACT_MEMBER_KIND_MESSAGE","memberId":{"value":"message-id"}})
}
fn claim_manifest(kind: &str, claims: serde_json::Value) -> String {
    json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"claims","name":"owner"},"spec":{"title":"Owner","messageClaims":claims}}).to_string()
}
fn selection(binding: serde_json::Value, selectors: serde_json::Value) -> serde_json::Value {
    json!({"claims":[{"connectionId":{"value":"claim-a"},"direction":"COMMUNICATION_DIRECTION_SEND","message":reference()}],
        "fieldSelections":[{"connectionId":{"value":"selection-a"},"claimId":{"value":"claim-a"},"binding":binding,"selectors":selectors}]})
}
fn exact_binding() -> serde_json::Value {
    json!({"owner":{"kind":"ENTITY_KIND_SPECIFICATION","id":"messages@1"},"ownerUid":"spec-uid","id":{"value":"binding-a"}})
}

#[test]
fn malformed_claim_manifests_are_rejected_without_guessing_targets_or_alternatives() {
    let cases: Vec<(&str, serde_json::Value)> = vec![
        (
            "both selector alternatives",
            selection(
                exact_binding(),
                json!([{"path":{"segments":["a"]},"opaque":{"syntax":"json-pointer","value":"/a"}}]),
            ),
        ),
        (
            "selector with no alternative",
            selection(exact_binding(), json!([{}])),
        ),
        (
            "unknown selector alternative",
            selection(exact_binding(), json!([{"regex":{"value":"a.*"}}])),
        ),
        (
            "bare string path alternative",
            selection(exact_binding(), json!([{"path":"a.b"}])),
        ),
        (
            "bare string binding without kind or incarnation",
            selection(json!("messages@1"), json!([])),
        ),
        (
            "binding owner without an exact kind",
            selection(
                json!({"owner":"messages@1","ownerUid":"spec-uid","id":{"value":"binding-a"}}),
                json!([]),
            ),
        ),
        (
            "bare string message selector",
            json!({"claims":[{"direction":"COMMUNICATION_DIRECTION_SEND","message":"messages@1"}]}),
        ),
        (
            "abbreviated direction",
            json!({"claims":[{"direction":"SEND","message":reference()}]}),
        ),
        (
            "numeric delivery mode",
            json!({"claims":[{"connectionId":{"value":"claim-a"},"direction":"COMMUNICATION_DIRECTION_SEND","message":reference()}],
                "routes":[{"claimId":{"value":"claim-a"},"channel":"channel@1","deliveryMode":"PUSH"}]}),
        ),
        (
            "route parameter value that is not text",
            json!({"claims":[{"connectionId":{"value":"claim-a"},"direction":"COMMUNICATION_DIRECTION_SEND","message":reference()}],
                "routes":[{"claimId":{"value":"claim-a"},"channel":"channel@1","parameterValues":{"tenant":1}}]}),
        ),
        (
            "unknown claim field",
            json!({"claims":[{"direction":"COMMUNICATION_DIRECTION_SEND","message":reference(),"peer":"provider@1"}]}),
        ),
        (
            "unknown container list",
            json!({"claims":[],"links":[]}),
        ),
        (
            "trigger with a bare string message",
            json!({"triggers":[{"claimId":{"value":"claim-a"},"message":"messages@1","condition":{"value":"c"}}]}),
        ),
    ];
    let mut accepted = Vec::new();
    for kind in ["Component", "ExternalSystem", "Domain"] {
        for (name, claims) in &cases {
            if manifest::parse_str(&claim_manifest(kind, claims.clone()), "claims.yaml").is_ok() {
                accepted.push(format!("{kind}: {name}"));
            }
        }
    }
    assert!(accepted.is_empty(), "accepted malformed manifests: {accepted:#?}");
}

#[test]
fn describe_keeps_every_claim_occurrence_and_exact_provenance_without_merging_equals() {
    use trogon_atlas_core::refs::outbound_refs;
    let types = TranscodePool::builtin().unwrap();
    let message = reference();
    let claim = |id: &str| {
        json!({"connectionId":{"value":id},"direction":"COMMUNICATION_DIRECTION_SEND","message":message})
    };
    let input = claim_manifest(
        "Domain",
        json!({
            "claims":[claim("claim-a"), claim("claim-b")],
            "routes":[
                {"connectionId":{"value":"route-a"},"claimId":{"value":"claim-a"},"channel":"orders@1"},
                {"connectionId":{"value":"route-b"},"claimId":{"value":"claim-a"},"channel":"orders@1"}
            ],
            "triggers":[{"connectionId":{"value":"trigger-a"},"claimId":{"value":"claim-b"},"message":message,"condition":{"value":"when approved"}}],
            "fieldSelections":[{"connectionId":{"value":"selection-a"},"claimId":{"value":"claim-a"},"binding":exact_binding(),"selectors":[{"path":{"segments":["a"]}}]}]
        }),
    );
    let entity = manifest::parse_str(&input, "claims.yaml")
        .unwrap()
        .remove(0)
        .entity;
    let owner = trogon_atlas_proto::EntityRef {
        kind: trogon_atlas_proto::EntityKind::Domain as i32,
        id: Some(trogon_atlas_proto::Id {
            namespace: "claims".into(),
            slug: "owner".into(),
            version: 1,
        }),
    };
    let outgoing: Vec<_> = outbound_refs(&entity)
        .into_iter()
        .map(|found| trogon_atlas_proto::Reference {
            from: Some(owner.clone()),
            from_field: found.from_field.clone(),
            to: found.to_entity_ref(),
            from_uid: "owner-uid".into(),
            connection_id: found.from_field.contains("routes[1]").then(|| {
                trogon_atlas_proto::ConnectionId {
                    value: "route-b".into(),
                }
            }),
            ..Default::default()
        })
        .collect();
    let described =
        trogon_atlas_client::view::describe(&entity, &types, &outgoing, &[]).unwrap();
    let text = trogon_atlas_client::view::render_description(&described);
    for field in [
        "message_claims.claims[0].message.specification",
        "message_claims.claims[1].message.specification",
        "message_claims.triggers[0].message.specification",
        "message_claims.routes[0].channel",
        "message_claims.routes[1].channel",
        "message_claims.field_selections[0].binding.owner",
    ] {
        let lines = text
            .lines()
            .filter(|line| line.trim_start().starts_with(field))
            .count();
        assert_eq!(lines, 1, "{field}\n{text}");
    }
    assert_eq!(text.matches("Exact reference provenance").count(), 6);
    assert_eq!(
        text.matches(r#""connectionId":{"value":"route-b"}"#).count(),
        1
    );
}
