//! Typed error boundary between the MCP adapter's internals and the
//! `rmcp` protocol surface.
//!
//! Internal helpers (`descriptor_for`, `message_from_json`,
//! `message_to_json`, ...) keep returning `anyhow::Result` for
//! ergonomic context-attachment. But the public dispatch surface --
//! everywhere a tool handler returns `Result<_, rmcp::ErrorData>` --
//! routes through `McpError` so the conversion to rmcp's error codes
//! is structural, not "everything maps to `internal_error`".
//!
//! Add a variant here when a tool dispatch path has a failure mode the
//! caller might want to act on (typically a bad-input case that
//! should surface as `invalid_params`, not as `internal_error`).

use thiserror::Error;
use trogon_atlas_client::{
    compat::CompatibilityError,
    failure::{Failure, FailureCategory},
};
use trogon_atlas_proto as pb;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("unknown entity kind: {0}")]
    UnknownKind(String),

    #[error("kind {kind:?} cannot be used as an analysis scope")]
    InvalidScopeKind { kind: pb::EntityKind },

    #[error("invalid base64: {0}")]
    InvalidBase64(String),

    #[error("invalid binpb for {message}: {source}")]
    InvalidBinpb {
        message: &'static str,
        #[source]
        source: prost::DecodeError,
    },

    #[error("invalid JSON for {message}: {detail}")]
    InvalidJson {
        message: &'static str,
        detail: String,
    },

    #[error("descriptor for {0} not found")]
    DescriptorNotFound(String),

    #[error("missing required parameter: {0}")]
    MissingParam(&'static str),

    #[error("{field} {detail}")]
    InvalidIdComponent { field: &'static str, detail: String },

    #[error(transparent)]
    Incompatible(#[from] CompatibilityError),

    #[error("rpc transport: {0}")]
    Transport(#[source] Box<tonic::Status>),

    #[error("{0}")]
    InvalidParams(String),

    /// A write failed for a reason `trogon_atlas_client::failure::Failure`
    /// already classified (a stale plan, a rejected batch op, a
    /// `tonic::Status` with or without an `ErrorInfo`, ...). Carries the
    /// classification through to `rmcp::ErrorData` untouched, so an agent
    /// reads the same `category`/`next_action`/`code` an trogon-atlas user would
    /// see in `CommandOutcome`.
    #[error("{0}")]
    Failed(Failure),

    #[error("{0}")]
    Internal(String),
}

impl McpError {
    fn is_client_error(&self) -> bool {
        matches!(
            self,
            McpError::UnknownKind(_)
                | McpError::InvalidScopeKind { .. }
                | McpError::InvalidBase64(_)
                | McpError::InvalidBinpb { .. }
                | McpError::InvalidJson { .. }
                | McpError::MissingParam(_)
                | McpError::InvalidIdComponent { .. }
                | McpError::InvalidParams(_)
        )
    }
}

/// `{category, next_action}` plus whatever richer detail a classified
/// failure carries, as the `data` of an `rmcp::ErrorData`. `category` and
/// `next_action` stay flat top-level strings (an existing contract: an
/// agent switches on `data.category == "stale_state"` without a schema per
/// category); everything else -- a stale plan's revisions, a partial
/// apply's journal id, a rejected batch op's code -- nests under
/// `next_action_detail` instead of flattening into `data`, so a new detail
/// field can never collide with `category`/`next_action` on the wire.
fn mcp_data(failure: &Failure) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("category".into(), failure.category.to_string().into());
    map.insert("next_action".into(), failure.next_action.to_string().into());
    if let Some(code) = &failure.code {
        map.insert("code".into(), code.as_str().into());
    }
    if !failure.details.is_empty() {
        map.insert(
            "next_action_detail".into(),
            failure.details.clone().into_json(),
        );
    }
    serde_json::Value::Object(map)
}

/// The same two buckets `rmcp` offers a tool caller: `INVALID_PARAMS` for
/// "change your request and retry" (including a replan or a plain retry --
/// the MCP protocol has no error code for those, so `data.next_action`
/// carries that distinction instead), `INTERNAL_ERROR` for "nothing the
/// caller can do about this specific call".
fn rmcp_code_for(category: FailureCategory) -> rmcp::model::ErrorCode {
    match category {
        FailureCategory::Validation
        | FailureCategory::NotFound
        | FailureCategory::Precondition
        | FailureCategory::StaleState
        | FailureCategory::OperationConflict => rmcp::model::ErrorCode::INVALID_PARAMS,
        FailureCategory::Unauthorized
        | FailureCategory::Internal
        | FailureCategory::Unavailable
        | FailureCategory::PartialApply
        | FailureCategory::Incompatible => rmcp::model::ErrorCode::INTERNAL_ERROR,
    }
}

/// Build the `rmcp::ErrorData` for a classified `Failure`.
///
/// A `Failure` with a `code` (an `ErrorInfo` reason this client recognizes,
/// or a batch op's own failure code) always carries a message the server
/// authored for a caller to read, so it passes through unchanged. Without
/// one -- a bare gRPC code with no further detail, or no RPC involved at
/// all -- `Unauthorized`/`Internal` fall back to a generic message rather
/// than risk forwarding backend detail that was never meant to leave the
/// server; every other category's message is already caller-safe text by
/// construction (a validation rule, a stale-plan summary, a compatibility
/// mismatch), so it passes through either way.
fn error_data_for_failure(failure: &Failure) -> rmcp::ErrorData {
    let message = if failure.code.is_some() {
        failure.message.clone()
    } else {
        match failure.category {
            FailureCategory::Unauthorized => "authentication or authorization error".to_string(),
            FailureCategory::Unavailable | FailureCategory::Internal => {
                "internal error".to_string()
            }
            _ => failure.message.clone(),
        }
    };
    rmcp::ErrorData::new(
        rmcp_code_for(failure.category),
        message,
        Some(mcp_data(failure)),
    )
}

/// Classify a `tonic::Status` into an `rmcp::ErrorData`, via the same
/// `Failure` classification every other surface (trogon-atlas, the MCP adapter's
/// own anyhow bridge) uses.
pub fn classify_status(s: &tonic::Status) -> rmcp::ErrorData {
    error_data_for_failure(&Failure::from_status(s))
}

impl From<McpError> for rmcp::ErrorData {
    fn from(e: McpError) -> Self {
        match e {
            McpError::Incompatible(err) => {
                error_data_for_failure(&Failure::from_compatibility_error(&err))
            }
            McpError::Transport(status) => classify_status(&status),
            McpError::Failed(failure) => error_data_for_failure(&failure),
            other => {
                let msg = other.to_string();
                if other.is_client_error() {
                    rmcp::ErrorData::invalid_params(msg, None)
                } else {
                    rmcp::ErrorData::internal_error(msg, None)
                }
            }
        }
    }
}

impl From<tonic::Status> for McpError {
    fn from(e: tonic::Status) -> Self {
        McpError::Transport(Box::new(e))
    }
}

/// Bridge to keep existing call sites that still propagate
/// `anyhow::Error` working until they migrate to typed errors.
/// `Failure::classify` already walks the chain for every typed error this
/// crate cares about (and the `tonic::Status` at its root, with or without
/// an `ErrorInfo`), so this bridge has nothing left to special-case.
impl From<anyhow::Error> for McpError {
    fn from(e: anyhow::Error) -> Self {
        McpError::Failed(Failure::classify(&e))
    }
}

/// Converts a `tonic::Status` to `rmcp::ErrorData`, logging the full
/// status at WARN level (so operators see root-cause detail) and
/// returning only the classified, scrubbed error to the MCP client.
///
/// Fold `operation_id` into an already-classified error's `data`, so a
/// caller whose mutating call failed can still look the attempt up with
/// `get_operation` -- the id is the one thing a bare `ErrorData` would
/// otherwise lose. A no-op for an empty id (nothing was ever claimed with
/// it, e.g. a dry run, or the call never carried one).
///
/// Usage: `.map_err(tool_err("tool_name"))?` for the RPC, then
/// `.map_err(|e| attach_operation_id(e, &operation_id))?` to add the id,
/// or chain both in one `.map_err` closure.
pub fn attach_operation_id(mut err: rmcp::ErrorData, operation_id: &str) -> rmcp::ErrorData {
    if operation_id.is_empty() {
        return err;
    }
    let mut map = match err.data.take() {
        Some(serde_json::Value::Object(map)) => map,
        Some(other) => {
            let mut map = serde_json::Map::new();
            map.insert("value".into(), other);
            map
        }
        None => serde_json::Map::new(),
    };
    map.insert("operation_id".into(), operation_id.into());
    err.data = Some(serde_json::Value::Object(map));
    err
}

/// Usage: `.map_err(tool_err("tool_name"))`
pub fn tool_err(tool: &'static str) -> impl Fn(tonic::Status) -> rmcp::ErrorData {
    move |s| {
        use tonic::Code;
        match s.code() {
            Code::Unauthenticated | Code::PermissionDenied => {
                tracing::warn!(tool = tool, grpc_code = ?s.code(), "MCP tool RPC auth error");
            }
            _ => {
                tracing::warn!(tool = tool, grpc_code = ?s.code(), error = %s, "MCP tool RPC error");
            }
        }
        classify_status(&s)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use rmcp::model::ErrorCode;

    use super::*;

    #[test]
    fn stale_plan_reaches_the_agent_as_replan_with_nested_revisions() {
        let stale = trogon_atlas_client::precondition::StalePlan {
            entity: pb::EntityKey {
                kind: pb::EntityKind::Event,
                namespace: "shop".into(),
                slug: "order.placed".into(),
                version: 1,
            },
            expected: trogon_atlas_client::precondition::Revision::Absent,
            actual: "7".parse().unwrap(),
        };
        let chained = anyhow::Error::new(stale).context("apply_manifests failed");
        let err: rmcp::ErrorData = McpError::from(chained).into();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("stale plan"), "{}", err.message);
        let data = err
            .data
            .expect("a stale refusal must carry structured data");
        assert_eq!(data["category"], "stale_state");
        assert_eq!(data["next_action"], "replan");
        assert_eq!(
            data["next_action_detail"]["expected_etag"],
            serde_json::Value::Null
        );
        assert_eq!(data["next_action_detail"]["actual_etag"], "7");
    }

    // apply_manifests / create_branch route tonic errors through anyhow →
    // McpError. classify_status maps ABORTED (etag mismatch) to
    // invalid_params; the anyhow bridge must do the same or agents see a scrubbed
    // internal_error and lose the actionable etag message.
    #[test]
    fn anyhow_aborted_status_classifies_as_invalid_params() {
        let status = tonic::Status::aborted("etag mismatch on conditional write");
        let chained = anyhow::Error::new(status).context("apply_manifests failed");
        let mcp: McpError = chained.into();
        let err: rmcp::ErrorData = mcp.into();
        assert_eq!(
            err.code,
            ErrorCode::INVALID_PARAMS,
            "ABORTED via anyhow must stay invalid_params (same as classify_status), got {err:?}"
        );
        assert!(
            err.message.contains("etag mismatch"),
            "etag detail must survive the anyhow bridge: {}",
            err.message
        );
    }

    // The anyhow bridge already maps OutOfRange → InvalidParams. classify_status
    // (used by tool_err for most RPC tools) must agree, or the same gRPC code
    // becomes INVALID_PARAMS via apply_manifests and INTERNAL_ERROR via get_impact.
    #[test]
    fn classify_status_out_of_range_is_invalid_params() {
        let s = tonic::Status::out_of_range("page_token out of range");
        let err = classify_status(&s);
        assert_eq!(
            err.code,
            ErrorCode::INVALID_PARAMS,
            "OutOfRange must match the anyhow bridge (invalid_params), got {err:?}"
        );
        assert!(
            err.message.contains("page_token"),
            "client-safe OutOfRange detail must survive: {}",
            err.message
        );
    }

    // apply_manifests / diff_manifests surface authoring failures (duplicate
    // manifests, BatchMutate status=Failed validation) via anyhow. A bare
    // bail! collapses to INTERNAL_ERROR; those failures must carry a tonic
    // Status so agents get INVALID_PARAMS with the validation detail.
    #[test]
    fn apply_authoring_failure_status_classifies_as_invalid_params() {
        let status = tonic::Status::invalid_argument(
            "apply failed at event:shop/x@1:\n  error SCENARIO_MISSING_REF: missing ref",
        );
        let chained = anyhow::Error::new(status).context("BatchMutate failed");
        let mcp: McpError = chained.into();
        let err: rmcp::ErrorData = mcp.into();
        assert_eq!(
            err.code,
            ErrorCode::INVALID_PARAMS,
            "apply authoring/validation failures must stay invalid_params, got {err:?}"
        );
        assert!(
            err.message.contains("apply failed") || err.message.contains("SCENARIO_MISSING_REF"),
            "validation detail must survive the anyhow bridge: {}",
            err.message
        );
    }

    // A bare bail! (no typed error, no Status anywhere in the chain) carries
    // no structural information Failure::classify can act on, so it must
    // classify as Internal and its text must not leak into the message --
    // proves the old `msg.contains("apply failed")` heuristic is gone.
    #[test]
    fn bare_untyped_bail_classifies_as_internal_and_does_not_leak_its_text() {
        let bare = anyhow::anyhow!(
            "apply failed at event:shop/x@1:\n  error SCENARIO_MISSING_REF: missing ref"
        );
        let mcp: McpError = bare.into();
        let err: rmcp::ErrorData = mcp.into();
        assert_eq!(
            err.code,
            ErrorCode::INTERNAL_ERROR,
            "an untyped bail carries no structural information and must not be guessed at via text, got {err:?}"
        );
        assert_eq!(err.message, "internal error");
    }

    // ApplyRejected (apply.rs's typed replacement for the old
    // Status::invalid_argument(text) wrapping) must still reach the agent as
    // invalid_params with the batch op's own code in `data`, now that the
    // heuristic it used to rely on is gone.
    #[test]
    fn apply_rejected_with_a_known_batch_code_carries_it_in_data() {
        let rejected = trogon_atlas_client::apply::ApplyRejected {
            failed: "event:shop/order.placed@1".into(),
            issues: vec![pb::ValidationIssue {
                code: "ENTITY_REFERENCED".into(),
                message: "entity is referenced".into(),
                ..Default::default()
            }],
        };
        let chained = anyhow::Error::new(rejected).context("BatchMutate failed");
        let mcp: McpError = chained.into();
        let err: rmcp::ErrorData = mcp.into();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        let data = err
            .data
            .expect("a rejected apply must carry structured data");
        assert_eq!(data["category"], "precondition");
        assert_eq!(data["code"], "ENTITY_REFERENCED");
    }
}
