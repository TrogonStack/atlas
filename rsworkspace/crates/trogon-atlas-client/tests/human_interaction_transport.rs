#![allow(clippy::unwrap_used)]

use std::convert::Infallible;

use serde_json::json;
use tonic::{body::Body, Code, Status};
use trogon_atlas_client::{
    client::{client_for_transport, ConnectOptions, Transport},
    failure::{Failure, FailureCategory},
    human_interaction,
};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

#[test]
fn interaction_request_codec_keeps_full_selectors_and_large_versions() {
    let connection = |kind, slug, uid, connection| {
        json!({
            "owner":{"kind":kind,"id":{"namespace":"portal","slug":slug,"version":u64::MAX.to_string()}},
            "ownerUid":uid,"id":{"value":connection}
        })
    };
    let hosting = connection(
        "ENTITY_KIND_COMPONENT",
        "portal",
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
    );
    for path in ["issuance", "display"] {
        let kind = if path == "issuance" {
            "ENTITY_KIND_COMMAND_SLICE"
        } else {
            "ENTITY_KIND_UI_SLICE"
        };
        let persona = connection(
            kind,
            "review",
            "33333333-3333-4333-8333-333333333333",
            "44444444-4444-4444-8444-444444444444",
        );
        let ui = connection(
            kind,
            "review",
            "33333333-3333-4333-8333-333333333333",
            "55555555-5555-4555-8555-555555555555",
        );
        let terminal = connection(
            kind,
            "review",
            "33333333-3333-4333-8333-333333333333",
            "66666666-6666-4666-8666-666666666666",
        );
        let mut request = json!({"hosting":hosting});
        request[path] = json!({"persona":persona,"ui":ui});
        let field = if path == "issuance" {
            "command"
        } else {
            "sourceReadModel"
        };
        request[path][field] = terminal;
        let parsed: pb::InspectHumanInteractionRequest =
            human_interaction::parse_request("InspectHumanInteractionRequest", &request).unwrap();
        assert_eq!(
            parsed
                .hosting
                .as_ref()
                .unwrap()
                .owner
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .version,
            u64::MAX
        );
        let encoded = TranscodePool::builtin()
            .unwrap()
            .message_json_value(
                "trogonatlas.api.eventmodel.v1alpha1.InspectHumanInteractionRequest",
                &parsed,
            )
            .unwrap();
        assert_eq!(encoded, request);
    }
}

#[tokio::test]
async fn inspection_forwards_branch_auth_and_preserves_operational_refusals() {
    for code in [
        Code::Unavailable,
        Code::DeadlineExceeded,
        Code::ResourceExhausted,
        Code::NotFound,
        Code::PermissionDenied,
    ] {
        let transport = Transport::new(tower::service_fn(
            move |request: http::Request<Body>| async move {
                assert!(request.uri().path().ends_with("/InspectHumanInteraction"));
                assert_eq!(request.headers()["x-trogon-atlas-branch"], "hosting-review");
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer inspection-test-token"
                );
                assert!(request.headers().contains_key("x-request-id"));
                Ok::<_, Infallible>(Status::new(code, "inspection refused").into_http::<Body>())
            },
        ));
        let mut client = client_for_transport(
            transport,
            Some("inspection-test-token"),
            &ConnectOptions {
                branch: Some("hosting-review".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let error = human_interaction::inspect_human_interaction(
            &mut client,
            pb::InspectHumanInteractionRequest::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Status>().unwrap().code(), code);
        let category = match code {
            Code::NotFound => FailureCategory::NotFound,
            Code::PermissionDenied => FailureCategory::Unauthorized,
            Code::Unavailable | Code::DeadlineExceeded => FailureCategory::Unavailable,
            _ => FailureCategory::Validation,
        };
        assert_eq!(Failure::classify(&error).category, category);
    }
}
