#![allow(clippy::unwrap_used)]

use std::{convert::Infallible, pin::Pin};

use http_body::Body as _;
use prost::Message;
use serde_json::json;
use tonic::{body::Body, Code, Status};
use trogon_atlas_client::{
    channel_messages,
    client::{client_for_transport, ConnectOptions, Transport},
    failure::{Failure, FailureCategory},
};
use trogon_atlas_proto as pb;

fn check_headers(request: &http::Request<Body>) {
    assert!(request.uri().path().ends_with("/ListChannelMessages"));
    assert_eq!(request.headers()["x-trogon-atlas-branch"], "channel-review");
    assert_eq!(request.headers()["authorization"], "Bearer test-token");
    assert!(request.headers().contains_key("x-request-id"));
}

fn connect(transport: Transport) -> trogon_atlas_client::client::Client {
    client_for_transport(
        transport,
        Some("test-token"),
        &ConnectOptions {
            branch: Some("channel-review".into()),
            ..Default::default()
        },
    )
    .unwrap()
}

#[tokio::test]
async fn channel_query_preserves_branch_auth_and_failure_codes() {
    for (code, category) in [
        (Code::NotFound, FailureCategory::NotFound),
        (Code::PermissionDenied, FailureCategory::Unauthorized),
        (Code::Unavailable, FailureCategory::Unavailable),
        (Code::DeadlineExceeded, FailureCategory::Unavailable),
        (Code::ResourceExhausted, FailureCategory::Validation),
        (Code::InvalidArgument, FailureCategory::Validation),
    ] {
        let transport = Transport::new(tower::service_fn(
            move |request: http::Request<Body>| async move {
                check_headers(&request);
                Ok::<_, Infallible>(Status::new(code, "query refused").into_http::<Body>())
            },
        ));
        let error = channel_messages::list_channel_messages(
            &mut connect(transport),
            pb::ListChannelMessagesRequest::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Status>().unwrap().code(), code);
        assert_eq!(Failure::classify(&error).category, category);
    }
}

#[tokio::test]
async fn channel_query_keeps_exact_request_and_response_presence() {
    let request: pb::ListChannelMessagesRequest = channel_messages::parse_request(
        "ListChannelMessagesRequest",
        &json!({
            "channel": {"id": {
                "namespace": "catalog", "slug": "orders",
                "version": "18446744073709551615"
            }},
            "pageSize": 1,
            "pageToken": "request-cursor"
        }),
    )
    .unwrap();
    let expected = request.clone();
    let response: pb::ListChannelMessagesResponse = channel_messages::parse_request(
        "ListChannelMessagesResponse",
        &json!({
            "records": [
                {"ownerUid": "first-owner", "connectionId": {"value": "route-a"},
                 "channel": {"id": {"namespace": "catalog", "slug": "orders",
                     "version": "18446744073709551615"}},
                 "parameterValues": {"region": "west"}, "authority": 17},
                {"ownerUid": "second-owner", "connectionId": {"value": "route-b"},
                 "deliveryMode": "CLAIM_DELIVERY_MODE_PULL", "doc": ""}
            ],
            "nextPageToken": "response-cursor", "incomplete": true,
            "complete": false, "branch": "channel-review",
            "scopeDigest": "authorized-scope", "authorizedNamespaceCount": 2
        }),
    )
    .unwrap();
    let transport = Transport::new(tower::service_fn(move |request: http::Request<Body>| {
        let expected = expected.clone();
        let response = response.clone();
        async move {
            check_headers(&request);
            let mut body = request.into_body();
            let mut wire = Vec::new();
            while let Some(frame) =
                std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
            {
                if let Ok(data) = frame.unwrap().into_data() {
                    wire.extend_from_slice(&data);
                }
            }
            assert_eq!(wire[0], 0);
            assert_eq!(
                pb::ListChannelMessagesRequest::decode(&wire[5..]).unwrap(),
                expected
            );
            let message = response.encode_to_vec();
            let mut frame = vec![0];
            frame.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
            frame.extend_from_slice(&message);
            Ok::<_, Infallible>(
                http::Response::builder()
                    .header("content-type", "application/grpc")
                    .header("grpc-status", "0")
                    .body(Body::new(ReplyBody(Some(frame.into()))))
                    .unwrap(),
            )
        }
    }));
    let result = channel_messages::list_channel_messages(&mut connect(transport), request)
        .await
        .unwrap();
    assert_eq!(result["records"][0]["ownerUid"], "first-owner");
    assert_eq!(result["records"][0]["connectionId"]["value"], "route-a");
    assert_eq!(
        result["records"][0]["channel"]["id"]["version"],
        u64::MAX.to_string()
    );
    assert_eq!(result["records"][0]["parameterValues"]["region"], "west");
    assert_eq!(result["records"][0]["authority"], 17);
    assert!(result["records"][0].get("deliveryMode").is_none());
    assert_eq!(result["records"][1]["connectionId"]["value"], "route-b");
    assert_eq!(
        result["records"][1]["deliveryMode"],
        "CLAIM_DELIVERY_MODE_PULL"
    );
    assert_eq!(result["records"][1]["doc"], "");
    assert_eq!(result["nextPageToken"], "response-cursor");
    assert_eq!(result["incomplete"], true);
    assert!(result.get("complete").is_none());
    assert_eq!(result["branch"], "channel-review");
    assert_eq!(result["scopeDigest"], "authorized-scope");
    assert_eq!(result["authorizedNamespaceCount"], 2);
}

struct ReplyBody(Option<bytes::Bytes>);

impl http_body::Body for ReplyBody {
    type Data = bytes::Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(self.0.take().map(|bytes| Ok(http_body::Frame::data(bytes))))
    }
}
