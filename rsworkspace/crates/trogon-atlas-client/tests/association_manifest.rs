#![allow(clippy::unwrap_used)]

use serde_json::{json, Value};
use trogon_atlas_client::{apply, export, manifest, manifest_schema};
use trogon_atlas_core::{channel_links, storage_claims, transcode::TranscodePool};

const MAX: &str = "18446744073709551615";

fn envelope(kind: &str, spec: Value) -> Value {
    json!({"apiVersion":manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"association","name":"subject","version":MAX},"spec":spec})
}

fn links() -> Value {
    json!([
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000001"},"kind":"CHANNEL_LINK_KIND_COMPOSES","target":"association/inner@9007199254740993","parameterValues":{"tenant":"acme","region":"eu"},"doc":{"value":"Cafe\u{301}\r\n<script>inert()</script>"}},
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000002"},"kind":"CHANNEL_LINK_KIND_COMPOSES","target":"association/inner@9007199254740993"},
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000003"},"kind":"CHANNEL_LINK_KIND_ROUTES_TO","target":"association/outer@18446744073709551615"}
    ])
}

fn claims() -> Value {
    let attachment = json!({"owner":{"kind":"ENTITY_KIND_DATA_STORE","id":{"namespace":"association","slug":"warehouse","version":"18446744073709551615"}},"ownerUid":"store-uid","id":{"value":"attachment-id"}});
    json!([
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000011"},"store":"association/warehouse@18446744073709551615","direction":"STORAGE_DIRECTION_READ","attachment":attachment,"portId":{"value":"output-port"},"doc":{"value":"Exact\r\nread"}},
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000012"},"store":"association/warehouse@18446744073709551615","direction":"STORAGE_DIRECTION_WRITE"},
        {"connectionId":{"value":"01930000-0000-7000-8000-000000000013"},"store":"association/warehouse@18446744073709551615","direction":"STORAGE_DIRECTION_WRITE"}
    ])
}

fn round_trip(input: &Value) -> pb_entity::Entity {
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    assert!(validator.is_valid(input), "{input}");
    let parsed = manifest::parse_str(&input.to_string(), "association.yaml").unwrap();
    assert_eq!(parsed[0].id.version, u64::MAX);
    let entity = parsed[0].entity.clone();
    let types = TranscodePool::builtin().unwrap();
    let output = export::render_board(std::slice::from_ref(&entity), &[], &types).unwrap();
    assert_eq!(output.skipped, []);
    let reparsed = manifest::parse_str(&output.manifests, "export.yaml").unwrap();
    assert_eq!(reparsed[0].entity, entity);
    assert_eq!(
        apply::classify(&reparsed[0].entity, Some(&entity)),
        apply::Action::Unchanged
    );
    entity
}

mod pb_entity {
    pub use trogon_atlas_proto::Entity;
}

#[test]
fn channel_links_keep_kind_exact_target_values_ids_and_repeats_through_export_and_reapply() {
    let entity = round_trip(&envelope(
        "Channel",
        json!({"title":"Subject","links":links()}),
    ));
    let values = channel_links::links(&entity);
    assert_eq!(values.len(), 3);
    assert_eq!(values[0].parameter_values.len(), 2);
    assert!(values[1].parameter_values.is_empty());
    assert_eq!(values[0].target, values[1].target);
    assert_ne!(values[0].connection_id, values[1].connection_id);
    assert_eq!(
        values[0]
            .target
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        9_007_199_254_740_993
    );
    assert_eq!(
        values[2]
            .target
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        u64::MAX
    );
    assert!(values[0].doc.as_ref().unwrap().value.contains("\r\n"));
}

#[test]
fn storage_claims_keep_store_direction_attachment_port_ids_and_repeats_for_every_owner_kind() {
    for kind in ["Component", "ExternalSystem", "DataProduct"] {
        let entity = round_trip(&envelope(
            kind,
            json!({"title":"Subject","storageClaims":claims()}),
        ));
        let values = storage_claims::claims(&entity);
        assert_eq!(values.len(), 3, "{kind}");
        assert_ne!(values[1].connection_id, values[2].connection_id, "{kind}");
        assert_eq!(values[1].store, values[2].store, "{kind}");
        assert!(values[0].attachment.is_some(), "{kind}");
        assert!(values[0].port_id.is_some(), "{kind}");
        assert_eq!(values[1].attachment, None, "{kind}");
        assert_eq!(values[1].port_id, None, "{kind}");
        assert_eq!(
            values[0]
                .store
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .version,
            u64::MAX,
            "{kind}"
        );
    }
}

#[test]
fn association_manifests_reject_coercions_and_unknown_fields() {
    let mut invalid = Vec::new();
    for (kind, field, bad) in [
        (
            "Channel",
            "links",
            json!([{"kind":"COMPOSES","target":"association/inner"}]),
        ),
        (
            "Channel",
            "links",
            json!([{"kind":"CHANNEL_LINK_KIND_COMPOSES","target":{"component":"inner"}}]),
        ),
        (
            "Channel",
            "links",
            json!([{"kind":"CHANNEL_LINK_KIND_COMPOSES","target":"association/inner","parameterValues":{"tenant":1}}]),
        ),
        (
            "Channel",
            "links",
            json!([{"kind":"CHANNEL_LINK_KIND_COMPOSES","target":"association/inner","unknown":true}]),
        ),
        (
            "Component",
            "storageClaims",
            json!([{"store":"association/warehouse","direction":"READ"}]),
        ),
        (
            "Component",
            "storageClaims",
            json!([{"store":"association/warehouse","direction":true}]),
        ),
        (
            "DataProduct",
            "storageClaims",
            json!([{"store":"association/warehouse","direction":"STORAGE_DIRECTION_READ","portId":"output-port"}]),
        ),
        (
            "ExternalSystem",
            "storageClaims",
            json!([{"store":"association/warehouse","direction":"STORAGE_DIRECTION_READ","unknown":true}]),
        ),
    ] {
        invalid.push(envelope(kind, json!({"title":"Subject", field: bad})));
    }
    let validator = jsonschema::validator_for(&manifest_schema::generate().unwrap()).unwrap();
    for input in invalid {
        assert!(
            manifest::parse_str(&input.to_string(), "invalid.yaml").is_err(),
            "{input}"
        );
        assert!(!validator.is_valid(&input), "{input}");
    }
}
