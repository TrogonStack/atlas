#![allow(clippy::unwrap_used)]

use std::convert::Infallible;

use tonic::{body::Body, Code, Status};
use trogon_atlas_client::{
    client::{client_for_transport, ConnectOptions, Transport},
    failure::{Failure, FailureCategory},
    responsibility,
};
use trogon_atlas_proto as pb;

#[tokio::test]
async fn responsibility_queries_preserve_branch_auth_and_operational_statuses() {
    for (rpc, code, category) in [
        (
            "ListSubjectAssignments",
            Code::NotFound,
            FailureCategory::NotFound,
        ),
        (
            "ListAssigneeResponsibilities",
            Code::PermissionDenied,
            FailureCategory::Unauthorized,
        ),
        (
            "ListTeamMembers",
            Code::DeadlineExceeded,
            FailureCategory::Unavailable,
        ),
        (
            "ListPersonTeams",
            Code::Unavailable,
            FailureCategory::Unavailable,
        ),
        (
            "GetTeamContacts",
            Code::InvalidArgument,
            FailureCategory::Validation,
        ),
        (
            "GetTeamContacts",
            Code::ResourceExhausted,
            FailureCategory::Validation,
        ),
    ] {
        let transport = Transport::new(tower::service_fn(
            move |request: http::Request<Body>| async move {
                assert!(request.uri().path().ends_with(rpc));
                assert_eq!(
                    request.headers()["x-trogon-atlas-branch"],
                    "directory-review"
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
                branch: Some("directory-review".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let error = match rpc {
            "ListSubjectAssignments" => {
                responsibility::list_subject_assignments(
                    &mut client,
                    pb::ListSubjectAssignmentsRequest::default(),
                )
                .await
            }
            "ListAssigneeResponsibilities" => {
                responsibility::list_assignee_responsibilities(
                    &mut client,
                    pb::ListAssigneeResponsibilitiesRequest::default(),
                )
                .await
            }
            "ListTeamMembers" => {
                responsibility::list_team_members(
                    &mut client,
                    pb::ListTeamMembersRequest::default(),
                )
                .await
            }
            "ListPersonTeams" => {
                responsibility::list_person_teams(
                    &mut client,
                    pb::ListPersonTeamsRequest::default(),
                )
                .await
            }
            _ => {
                responsibility::get_team_contacts(
                    &mut client,
                    pb::GetTeamContactsRequest::default(),
                )
                .await
            }
        }
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Status>().unwrap().code(), code);
        assert_eq!(Failure::classify(&error).category, category);
    }
}

#[test]
fn query_budget_exhaustion_retains_a_distinct_safe_public_failure() {
    let failure = Failure::from_status(&Status::resource_exhausted("private backend detail"));
    assert_eq!(failure.category, FailureCategory::Validation);
    assert_eq!(failure.code.unwrap().as_str(), "RESOURCE_EXHAUSTED");
    assert!(!failure.message.contains("private backend detail"));
}
