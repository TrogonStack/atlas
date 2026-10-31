#![allow(clippy::unwrap_used)]

use std::convert::Infallible;

use tonic::{body::Body, Code, Status};
use trogon_atlas_client::{
    client::{client_for_transport, ConnectOptions, Transport},
    failure::{Failure, FailureCategory},
};
use trogon_atlas_proto as pb;

#[test]
fn term_domain_parser_preserves_exact_identity_and_paging() {
    let request: pb::ListTermDomainDeclarationsRequest =
        trogon_atlas_client::term_domains::parse_request(
            "ListTermDomainDeclarationsRequest",
            &serde_json::json!({
                "domain": {"id": {
                    "namespace": "catalog", "slug": "orders",
                    "version": "18446744073709551615"
                }},
                "pageSize": 1,
                "pageToken": "opaque-cursor"
            }),
        )
        .unwrap();
    assert_eq!(request.domain.unwrap().id.unwrap().version, u64::MAX);
    assert_eq!(request.page_size, 1);
    assert_eq!(request.page_token, "opaque-cursor");

    for invalid in [
        serde_json::json!({"pageSize": -1}),
        serde_json::json!({"unknown": true}),
    ] {
        let error = trogon_atlas_client::term_domains::parse_request::<
            pb::ListTermDomainDeclarationsRequest,
        >("ListTermDomainDeclarationsRequest", &invalid)
        .unwrap_err();
        assert_eq!(
            Failure::classify(&error).category,
            FailureCategory::Validation
        );
    }
}

#[tokio::test]
async fn native_catalog_queries_preserve_branch_auth_and_operational_failure_codes() {
    for (rpc, code) in [
        ("ListTermDomainDeclarations", Code::Unavailable),
        ("ListTermDomainDeclarations", Code::DeadlineExceeded),
        ("ListTermDomainDeclarations", Code::ResourceExhausted),
    ] {
        let transport = Transport::new(tower::service_fn(
            move |request: http::Request<Body>| async move {
                assert!(request.uri().path().ends_with(rpc));
                assert_eq!(request.headers()["x-trogon-atlas-branch"], "catalog-review");
                assert_eq!(request.headers()["authorization"], "Bearer test-token");
                assert!(request.headers().contains_key("x-request-id"));
                Ok::<_, Infallible>(Status::new(code, "query refused").into_http::<Body>())
            },
        ));
        let mut client = client_for_transport(
            transport,
            Some("test-token"),
            &ConnectOptions {
                branch: Some("catalog-review".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let result = trogon_atlas_client::term_domains::list_term_domain_declarations(
            &mut client,
            pb::ListTermDomainDeclarationsRequest::default(),
        )
        .await;

        let error = result.unwrap_err();
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
