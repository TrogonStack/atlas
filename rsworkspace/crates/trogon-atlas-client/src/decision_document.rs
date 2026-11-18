use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;
pub use crate::responsibility::parse_request;

pub async fn list_subject_decisions(
    client: &mut Client,
    request: pb::ListSubjectDecisionsRequest,
) -> Result<Value> {
    let response = client
        .list_subject_decisions(request)
        .await
        .context("ListSubjectDecisions failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSubjectDecisionsResponse",
            &response,
        )
        .context("encode ListSubjectDecisions response")
}

pub async fn list_subject_documents(
    client: &mut Client,
    request: pb::ListSubjectDocumentsRequest,
) -> Result<Value> {
    let response = client
        .list_subject_documents(request)
        .await
        .context("ListSubjectDocuments failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSubjectDocumentsResponse",
            &response,
        )
        .context("encode ListSubjectDocuments response")
}

pub async fn get_decision_detail(
    client: &mut Client,
    request: pb::GetDecisionDetailRequest,
) -> Result<Value> {
    let response = client
        .get_decision_detail(request)
        .await
        .context("GetDecisionDetail failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetDecisionDetailResponse",
            &response,
        )
        .context("encode GetDecisionDetail response")
}

pub async fn get_document_detail(
    client: &mut Client,
    request: pb::GetDocumentDetailRequest,
) -> Result<Value> {
    let response = client
        .get_document_detail(request)
        .await
        .context("GetDocumentDetail failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetDocumentDetailResponse",
            &response,
        )
        .context("encode GetDocumentDetail response")
}

pub async fn list_incoming_decision_declarations(
    client: &mut Client,
    request: pb::ListIncomingDecisionDeclarationsRequest,
) -> Result<Value> {
    let response = client
        .list_incoming_decision_declarations(request)
        .await
        .context("ListIncomingDecisionDeclarations failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListIncomingDecisionDeclarationsResponse",
            &response,
        )
        .context("encode ListIncomingDecisionDeclarations response")
}

pub async fn list_incoming_document_declarations(
    client: &mut Client,
    request: pb::ListIncomingDocumentDeclarationsRequest,
) -> Result<Value> {
    let response = client
        .list_incoming_document_declarations(request)
        .await
        .context("ListIncomingDocumentDeclarations failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListIncomingDocumentDeclarationsResponse",
            &response,
        )
        .context("encode ListIncomingDocumentDeclarations response")
}

pub async fn list_decision_amendments(
    client: &mut Client,
    request: pb::ListDecisionAmendmentsRequest,
) -> Result<Value> {
    let response = client
        .list_decision_amendments(request)
        .await
        .context("ListDecisionAmendments failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListDecisionAmendmentsResponse",
            &response,
        )
        .context("encode ListDecisionAmendments response")
}
