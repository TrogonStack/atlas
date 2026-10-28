#![allow(clippy::unwrap_used)]

use std::convert::Infallible;

use tonic::{body::Body, Code, Status};
use trogon_atlas_client::{
    client::{client_for_transport, ConnectOptions, Transport},
    design_topology,
    failure::{Failure, FailureCategory},
};
use trogon_atlas_proto as pb;

#[tokio::test]
async fn design_topology_queries_preserve_branch_auth_and_operational_failure_codes() {
    for (rpc, code) in [
        ("GetDesignGraphDetail", Code::Unavailable),
        ("ListSubjectDesignGraphs", Code::NotFound),
        ("TraverseDesignGraphTopology", Code::ResourceExhausted),
        ("ListIncomingDesignGraphSubflows", Code::PermissionDenied),
    ] {
        let transport = Transport::new(tower::service_fn(
            move |request: http::Request<Body>| async move {
                assert!(request.uri().path().ends_with(rpc));
                assert_eq!(
                    request.headers()["x-trogon-atlas-branch"],
                    "topology-review"
                );
                assert_eq!(request.headers()["authorization"], "Bearer test-token");
                assert!(request.headers().contains_key("x-request-id"));
                Ok::<_, Infallible>(Status::new(code, "query refused").into_http::<Body>())
            },
        ));
        let mut client = client_for_transport(
            transport,
            Some("test-token"),
            &ConnectOptions {
                branch: Some("topology-review".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let result = match rpc {
            "GetDesignGraphDetail" => {
                design_topology::get_design_graph_detail(
                    &mut client,
                    pb::GetDesignGraphDetailRequest::default(),
                )
                .await
            }
            "ListSubjectDesignGraphs" => {
                design_topology::list_subject_design_graphs(
                    &mut client,
                    pb::ListSubjectDesignGraphsRequest::default(),
                )
                .await
            }
            "TraverseDesignGraphTopology" => {
                design_topology::traverse_design_graph_topology(
                    &mut client,
                    pb::TraverseDesignGraphTopologyRequest::default(),
                )
                .await
            }
            "ListIncomingDesignGraphSubflows" => {
                design_topology::list_incoming_design_graph_subflows(
                    &mut client,
                    pb::ListIncomingDesignGraphSubflowsRequest::default(),
                )
                .await
            }
            _ => unreachable!(),
        };
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
