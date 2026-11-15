use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::client::Client;

/// A backing report as the server returned it.
#[derive(Debug, Clone)]
pub struct BackingReport(pb::GetBackingReportResponse);

impl BackingReport {
    #[must_use]
    pub fn response(&self) -> &pb::GetBackingReportResponse {
        &self.0
    }

    /// Whether every counted subject was settled as backed or unbacked over
    /// an untruncated snapshot. Anything else leaves the rate unestablished.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.0.snapshot_truncated
            && self.0.incomplete_reasons.is_empty()
            && self
                .0
                .totals
                .as_ref()
                .is_some_and(|totals| totals.unknown == 0)
    }

    pub fn to_json(&self) -> Result<Value> {
        TranscodePool::builtin()?
            .message_json_value(
                "trogonatlas.api.eventmodel.v1alpha1.GetBackingReportResponse",
                &self.0,
            )
            .context("encode GetBackingReport response")
    }
}

pub async fn get_backing_report(
    client: &mut Client,
    request: pb::GetBackingReportRequest,
) -> Result<BackingReport> {
    let response = client
        .get_backing_report(request)
        .await
        .context("GetBackingReport failed")?
        .into_inner();
    Ok(BackingReport(response))
}
