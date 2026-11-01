use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;
pub use crate::responsibility::parse_request;

pub async fn get_design_graph_detail(
    client: &mut Client,
    request: pb::GetDesignGraphDetailRequest,
) -> Result<Value> {
    let response = client
        .get_design_graph_detail(request)
        .await
        .context("GetDesignGraphDetail failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetDesignGraphDetailResponse",
            &response,
        )
        .context("encode GetDesignGraphDetail response")
}

pub async fn list_subject_design_graphs(
    client: &mut Client,
    request: pb::ListSubjectDesignGraphsRequest,
) -> Result<Value> {
    let response = client
        .list_subject_design_graphs(request)
        .await
        .context("ListSubjectDesignGraphs failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSubjectDesignGraphsResponse",
            &response,
        )
        .context("encode ListSubjectDesignGraphs response")
}

pub async fn traverse_design_graph_topology(
    client: &mut Client,
    request: pb::TraverseDesignGraphTopologyRequest,
) -> Result<Value> {
    let response = client
        .traverse_design_graph_topology(request)
        .await
        .context("TraverseDesignGraphTopology failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.TraverseDesignGraphTopologyResponse",
            &response,
        )
        .context("encode TraverseDesignGraphTopology response")
}

pub async fn list_incoming_design_graph_subflows(
    client: &mut Client,
    request: pb::ListIncomingDesignGraphSubflowsRequest,
) -> Result<Value> {
    let response = client
        .list_incoming_design_graph_subflows(request)
        .await
        .context("ListIncomingDesignGraphSubflows failed")?
        .into_inner();
    TranscodePool::builtin()?
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListIncomingDesignGraphSubflowsResponse",
            &response,
        )
        .context("encode ListIncomingDesignGraphSubflows response")
}
