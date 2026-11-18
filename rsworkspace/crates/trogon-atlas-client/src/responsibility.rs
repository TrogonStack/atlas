use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::{client::Client, failure::invalid_input};

const PACKAGE: &str = "trogonatlas.api.eventmodel.v1alpha1";

pub fn parse_request<T: prost::Message + Default>(name: &str, json: &Value) -> Result<T> {
    TranscodePool::builtin()?
        .message_from_json_value(&format!("{PACKAGE}.{name}"), json)
        .map_err(|error| invalid_input(format!("invalid {name}: {error:#}")))
}

pub async fn list_subject_assignments(
    client: &mut Client,
    request: pb::ListSubjectAssignmentsRequest,
) -> Result<Value> {
    let response = client
        .list_subject_assignments(request)
        .await
        .context("ListSubjectAssignments failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSubjectAssignmentsResponse",
            &response,
        )
        .context("encode ListSubjectAssignments response")
}

pub async fn list_assignee_responsibilities(
    client: &mut Client,
    request: pb::ListAssigneeResponsibilitiesRequest,
) -> Result<Value> {
    let response = client
        .list_assignee_responsibilities(request)
        .await
        .context("ListAssigneeResponsibilities failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListAssigneeResponsibilitiesResponse",
            &response,
        )
        .context("encode ListAssigneeResponsibilities response")
}

pub async fn list_team_members(
    client: &mut Client,
    request: pb::ListTeamMembersRequest,
) -> Result<Value> {
    let response = client
        .list_team_members(request)
        .await
        .context("ListTeamMembers failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListTeamMembersResponse",
            &response,
        )
        .context("encode ListTeamMembers response")
}

pub async fn list_person_teams(
    client: &mut Client,
    request: pb::ListPersonTeamsRequest,
) -> Result<Value> {
    let response = client
        .list_person_teams(request)
        .await
        .context("ListPersonTeams failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListPersonTeamsResponse",
            &response,
        )
        .context("encode ListPersonTeams response")
}

pub async fn get_team_contacts(
    client: &mut Client,
    request: pb::GetTeamContactsRequest,
) -> Result<Value> {
    let response = client
        .get_team_contacts(request)
        .await
        .context("GetTeamContacts failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetTeamContactsResponse",
            &response,
        )
        .context("encode GetTeamContacts response")
}
