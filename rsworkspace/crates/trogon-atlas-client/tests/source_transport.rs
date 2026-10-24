#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::convert::Infallible;

use serde_json::json;
use tonic::{body::Body, Code, Status};
use trogon_atlas_client::{
    client::{client_for_transport, ConnectOptions, Transport},
    source_integration,
};
use trogon_atlas_proto as pb;

#[test]
fn source_json_preserves_exact_selectors_guards_presence_and_ownership() {
    let json = json!({"mapping": {
        "id":"m", "source":{"registrationId":"r","category":"model","key":"opaque"},
        "selection":{"document":{"anchor":{"document":{"id":{"namespace":"n","slug":"doc","version":"9007199254740993"}},"ownerUid":"incarnation","anchorId":{"value":"anchor"},"expectedDigest":{"sha256":"a".repeat(64)}}}},
        "ownership":{"none":{}},"branch":"review","adoption":{"authorizedAdoption":false}
    }, "registrationRevision":"rev"});
    let request: pb::CreateSourceMappingRequest =
        source_integration::parse_request("CreateSourceMappingRequest", &json).unwrap();
    let mapping = request.mapping.unwrap();
    assert_eq!(mapping.branch.as_deref(), Some("review"));
    assert!(!mapping.adoption.unwrap().authorized_adoption);
    assert!(matches!(
        mapping.ownership.unwrap().policy,
        Some(pb::source_field_ownership::Policy::None(_))
    ));
    let Some(pb::source_selection::Selection::Document(document)) =
        mapping.selection.unwrap().selection
    else {
        panic!("document selection");
    };
    let Some(pb::document_selection::Selection::Anchor(anchor)) = document.selection else {
        panic!("anchor selection");
    };
    assert_eq!(anchor.owner_uid, "incarnation");
    assert_eq!(
        anchor.document.unwrap().id.unwrap().version,
        9_007_199_254_740_993
    );
    assert_eq!(anchor.expected_digest.unwrap().sha256, "a".repeat(64));
    for invalid in [
        json!({"registrationId":"r","includeBytes":"false"}),
        json!({"registration_id":"r","unknown":"private-host-path"}),
    ] {
        let error = source_integration::parse_request::<pb::GetSourceSnapshotRequest>(
            "GetSourceSnapshotRequest",
            &invalid,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "invalid source request");
    }
}

#[tokio::test]
async fn source_calls_preserve_branch_credentials_and_operational_status() {
    for code in [
        Code::NotFound,
        Code::PermissionDenied,
        Code::Unavailable,
        Code::DeadlineExceeded,
        Code::ResourceExhausted,
    ] {
        let transport = Transport::new(tower::service_fn(
            move |request: http::Request<Body>| async move {
                assert!(request.uri().path().ends_with("/RecoverSourceRun"));
                assert_eq!(request.headers()["x-trogon-atlas-branch"], "source-review");
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer surface-test-token"
                );
                assert!(request.headers().contains_key("x-request-id"));
                Ok::<_, Infallible>(Status::new(code, "recovery unavailable").into_http::<Body>())
            },
        ));
        let mut client = client_for_transport(
            transport,
            Some("surface-test-token"),
            &ConnectOptions {
                branch: Some("source-review".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let error = source_integration::recover_source_run(
            &mut client,
            pb::RecoverSourceRunRequest {
                registration_id: "r".into(),
                run_id: "same-run".into(),
                if_match: "revision".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Status>().unwrap().code(), code);
    }
}

#[test]
fn source_parser_rejects_quoted_32bit_numbers_before_serialization() {
    assert!(
        source_integration::parse_request::<pb::ListSourceRunsRequest>(
            "ListSourceRunsRequest",
            &json!({"registrationId":"r","pageSize":"100"})
        )
        .is_err()
    );
}

#[test]
fn source_parser_rejects_unknown_numeric_enums_before_serialization() {
    assert!(
        source_integration::parse_request::<pb::CreateSourceRegistrationRequest>(
            "CreateSourceRegistrationRequest",
            &json!({"registration":{"deletionPolicy":991}})
        )
        .is_err()
    );
}

#[test]
fn source_parser_rejects_null_presence() {
    assert!(
        source_integration::parse_request::<pb::GetSourceSnapshotRequest>(
            "GetSourceSnapshotRequest",
            &json!({"registrationId":"r","includeBytes":null})
        )
        .is_err()
    );
}

#[test]
fn source_parser_requires_exactly_one_selector_alternative() {
    for selection in [json!({}), json!({"entity":{},"document":{}})] {
        assert!(
            source_integration::parse_request::<pb::CreateSourceMappingRequest>(
                "CreateSourceMappingRequest",
                &json!({"mapping":{"selection":selection}})
            )
            .is_err()
        );
    }
}

#[test]
fn source_parser_preserves_64bit_strings_known_enum_numbers_and_optional_absence() {
    let request: pb::CreateSourceRegistrationRequest = source_integration::parse_request("CreateSourceRegistrationRequest", &json!({"registration":{"deletionPolicy":1,"retention":{"registrationBytes":"9007199254740993"}}})).unwrap();
    assert_eq!(
        request
            .registration
            .unwrap()
            .retention
            .unwrap()
            .registration_bytes,
        9_007_199_254_740_993
    );
    let request: pb::CreateSourceMappingRequest = source_integration::parse_request("CreateSourceMappingRequest", &json!({"mapping":{"ownership":{"none":{}},"selection":{"entity":{"kind":3,"id":{"namespace":"n","slug":"s","version":"1"}}}}})).unwrap();
    assert_eq!(request.mapping.unwrap().branch, None);
}

#[tokio::test]
async fn source_native_payload_keeps_connected_type_lookup_failure_semantics() {
    for code in [
        Code::PermissionDenied,
        Code::Unavailable,
        Code::DeadlineExceeded,
    ] {
        let transport = Transport::new(tower::service_fn(
            move |request: http::Request<Body>| async move {
                if request.uri().path().ends_with("/GetServerInfo") {
                    use prost::Message;
                    let message = pb::GetServerInfoResponse {
                        features: Some(pb::get_server_info_response::Features {
                            type_libraries: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                    .encode_to_vec();
                    let mut frame = vec![0];
                    frame.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
                    frame.extend_from_slice(&message);
                    return Ok::<_, Infallible>(
                        http::Response::builder()
                            .header("content-type", "application/grpc")
                            .header("grpc-status", "0")
                            .body(Body::new(SourceReplyBody(Some(frame.into()))))
                            .unwrap(),
                    );
                }
                assert!(request
                    .uri()
                    .path()
                    .ends_with("/GetTypeLibraryDescriptorSet"));
                Ok::<_, Infallible>(
                    Status::new(code, "type lookup unavailable").into_http::<Body>(),
                )
            },
        ));
        let mut client = client_for_transport(transport, None, &ConnectOptions::default()).unwrap();
        let error = source_integration::native_payload(&mut client, pb::SourceResourceKey { registration_id:"r".into(), category:"model".into(), key:"x".into() }, b"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: ReadModel\nmetadata: {namespace: models, name: receipt}\nspec: {title: Receipt}\n").await.unwrap_err();
        assert_eq!(error.downcast_ref::<Status>().unwrap().code(), code);
        let document = source_integration::prepare_preview_request(&json!({"nativePayloads":[{"source":{"registrationId":"r","category":"event","key":"x"},"entity":{"event":{"id":{"namespace":"models","slug":"receipt","version":"1"},"title":"Receipt"}}}]})).unwrap();
        let error = document.load(&mut client).await.unwrap_err();
        assert_eq!(error.downcast_ref::<Status>().unwrap().code(), code);
    }
}

struct SourceReplyBody(Option<bytes::Bytes>);
impl http_body::Body for SourceReplyBody {
    type Data = bytes::Bytes;
    type Error = Infallible;
    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(self.0.take().map(|bytes| Ok(http_body::Frame::data(bytes))))
    }
}

#[tokio::test]
async fn source_native_payload_never_echoes_invalid_supplied_values_or_field_names() {
    use prost::Message;
    for spec in [
        "{key: PRIVATE_SECRET_SENTINEL}",
        "{'/private/PRIVATE_SOURCE_PATH': hidden}",
    ] {
        let transport = Transport::new(tower::service_fn(
            |request: http::Request<Body>| async move {
                assert!(request.uri().path().ends_with("/GetServerInfo"));
                let message = pb::GetServerInfoResponse::default().encode_to_vec();
                let mut frame = vec![0];
                frame.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
                frame.extend_from_slice(&message);
                Ok::<_, Infallible>(
                    http::Response::builder()
                        .header("content-type", "application/grpc")
                        .header("grpc-status", "0")
                        .body(SourceReplyBody(Some(frame.into())))
                        .unwrap(),
                )
            },
        ));
        let mut client = client_for_transport(transport, None, &ConnectOptions::default()).unwrap();
        let yaml = format!(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: ReadModel\nmetadata: {{namespace: models, name: receipt}}\nspec: {spec}\n"
        );
        let error = source_integration::native_payload(
            &mut client,
            pb::SourceResourceKey {
                registration_id: "r".into(),
                category: "model".into(),
                key: "receipt".into(),
            },
            yaml.as_bytes(),
        )
        .await
        .unwrap_err();
        assert!(error
            .downcast_ref::<trogon_atlas_client::failure::InvalidInput>()
            .is_some());
        let rendered = format!("{error:#}");
        assert!(
            !rendered.contains("PRIVATE_SECRET_SENTINEL"),
            "supplied content leaked: {rendered}"
        );
        assert!(
            !rendered.contains("PRIVATE_SOURCE_PATH"),
            "supplied field leaked: {rendered}"
        );
    }
}

#[test]
fn source_parser_preserves_native_any_payload_and_explicit_json_null_values() {
    let request: pb::PreviewSourceRunRequest = source_integration::parse_request("PreviewSourceRunRequest", &json!({"nativePayloads":[{"source":{"registrationId":"r","category":"model","key":"receipt"},"entity":{"readModel":{"id":{"namespace":"models","slug":"receipt","version":"1"},"title":"Receipt","metadata":[{"@type":"type.googleapis.com/google.protobuf.Struct","value":{"empty":null,"flag":true,"nested":[1,"text"]}}]}}}]})).unwrap();
    let Some(pb::entity::Kind::ReadModel(owner)) = request.native_payloads[0]
        .entity
        .as_ref()
        .unwrap()
        .kind
        .as_ref()
    else {
        panic!("read model");
    };
    assert_eq!(
        owner.metadata[0].type_url,
        "type.googleapis.com/google.protobuf.Struct"
    );
    assert_ne!(owner.metadata[0].value, [] as [u8; 0]);
}

#[test]
fn source_connected_preview_preflight_retains_strict_envelope_and_owner_shape() {
    for request in [
        json!({"registrationId":true}),
        json!({"unknown":"PRIVATE_SENTINEL"}),
        json!({"budget":{"maxResults":"1"}}),
        json!({"nativePayloads":[{"entity":{"event":{"id":{"namespace":"n"},"unknown":"PRIVATE_SENTINEL"}}}]}),
        json!({"nativePayloads":[{"entity":{"event":{},"readModel":{}}}]}),
        json!({"nativePayloads":[{"entity":{"event":{"schema":{"@type":false}}}}]}),
    ] {
        assert!(source_integration::prepare_preview_request(&request).is_err());
    }
    assert!(source_integration::prepare_preview_request(&json!({"nativePayloads":[{"entity":{"event":{"id":{"namespace":"n","slug":"e","version":"9007199254740993"},"schema":{"@type":"type.googleapis.com/acme.money.v1.Money","cents":"9007199254740994"}}}}]})).is_ok());
}
