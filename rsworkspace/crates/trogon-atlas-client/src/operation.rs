// Durable idempotency-key lookup (`GetOperation`). Complements the inline
// `OperationReceipt` a mutating RPC returns on the response that claimed the
// id: this is the surface a caller reaches for after it lost that response
// (a crashed client, a dropped connection) and only has the id left.

use anyhow::{Context as _, Result};
use trogon_atlas_proto as pb;

use crate::client::Client;

/// Look up the current status of a previously used `operation_id`, scoped to
/// the caller that claimed it. Reports `OPERATION_STATUS_UNKNOWN` both for an
/// id nothing ever claimed and for one that guarded a branch-scoped write
/// (those keep no durable receipt).
pub async fn get_operation(
    client: &mut Client,
    operation_id: &str,
) -> Result<pb::GetOperationResponse> {
    let resp = client
        .get_operation(pb::GetOperationRequest {
            operation_id: operation_id.to_string(),
        })
        .await
        .context("GetOperation failed")?
        .into_inner();
    Ok(resp)
}

/// Render `OperationStatus` the way every structured-output surface (CLI
/// JSON, MCP tool result) should label it: lowercase, without the
/// `OPERATION_STATUS_` prefix.
#[must_use]
pub fn status_label(status: i32) -> &'static str {
    match pb::OperationStatus::try_from(status) {
        Ok(pb::OperationStatus::Pending) => "pending",
        Ok(pb::OperationStatus::Applied) => "applied",
        Ok(pb::OperationStatus::Rejected) => "rejected",
        Ok(pb::OperationStatus::NotApplied) => "not_applied",
        Ok(pb::OperationStatus::Unknown | pb::OperationStatus::Unspecified) | Err(_) => "unknown",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn status_label_covers_every_documented_status() {
        assert_eq!(
            status_label(pb::OperationStatus::Unspecified as i32),
            "unknown"
        );
        assert_eq!(status_label(pb::OperationStatus::Pending as i32), "pending");
        assert_eq!(status_label(pb::OperationStatus::Applied as i32), "applied");
        assert_eq!(
            status_label(pb::OperationStatus::Rejected as i32),
            "rejected"
        );
        assert_eq!(
            status_label(pb::OperationStatus::NotApplied as i32),
            "not_applied"
        );
        assert_eq!(status_label(pb::OperationStatus::Unknown as i32), "unknown");
        assert_eq!(status_label(999), "unknown");
    }
}
