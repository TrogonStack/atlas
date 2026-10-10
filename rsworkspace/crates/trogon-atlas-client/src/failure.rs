//! What a caller does next when a write fails: the one value object every
//! surface (trogon-atlas's exit code, the MCP adapter's `data`) switches on,
//! instead of each one re-deriving a verdict from a gRPC code, an
//! `ErrorInfo` reason, or (worse) `message()` text on its own.
//!
//! `Failure::classify` is the single place that walks an error chain: typed
//! errors first (`StalePlan`, `StaleResolution`, `ApplyRejected`,
//! `CompatibilityError`, `InvalidInput`), then the `tonic::Status` at its
//! root, by gRPC code and, when the server attached one, its
//! `ErrorInfo.reason` (`docs/reference/failure-reasons.md` is the registry
//! both sides read). No branch in this module matches on error text.

use std::fmt;

use serde::Serialize;
use serde_json::{json, Value};
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{
    apply::ApplyRejected,
    compat::CompatibilityError,
    precondition::{StalePlan, StaleResolution},
};

/// Domain of the `ErrorInfo` this client recognizes. Must match the
/// server's `trogon-atlas-server::conv::ERROR_DOMAIN`; duplicated here rather
/// than adding a server dependency to the client for one constant.
const SERVER_ERROR_DOMAIN: &str = "trogonatlas.api.eventmodel.v1alpha1";

/// A caller-supplied value failed a check this client makes locally,
/// before ever asking the server (a malformed manifest, an empty
/// namespace, a reserved branch name, a missing file). `Failure::classify`
/// recognizes this type ahead of its generic `Internal` fallback, so the
/// message a caller needs reaches them as `FailureCategory::Validation`
/// instead of a scrubbed "internal error" -- the caller authored the
/// problem and the message says how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidInput(String);

impl InvalidInput {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for InvalidInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidInput {}

/// Build an `anyhow::Error` around `InvalidInput`. Called by the
/// `invalid_input!` macro, which is to this what `anyhow::bail!` is to a
/// generic `anyhow::anyhow!`: use it at a `bail!` site that is really a
/// caller-input problem rather than an unexpected failure.
#[must_use]
pub fn invalid_input(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(InvalidInput::new(message))
}

/// Like `anyhow::bail!`, but for a site where the request itself is wrong
/// rather than something unexpected happening: the resulting error
/// classifies as `FailureCategory::Validation` (CLI exit `5`, MCP
/// `next_action: "fix_input"`) and keeps its message verbatim, instead of
/// collapsing to `Internal`'s generic text.
#[macro_export]
macro_rules! invalid_input {
    ($($arg:tt)*) => {
        return Err($crate::failure::invalid_input(format!($($arg)*)))
    };
}

/// What kind of retry, if any, resolves this failure. A CLI exit code and
/// an MCP `next_action` are both a direct function of this, never of
/// `code` or `message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    /// A plan (apply, conflict resolution) was made against a revision the
    /// server no longer holds. Read fresh state and plan again.
    StaleState,
    /// The request's precondition (an etag, a referrer check) was refused
    /// by current state, but no plan was staled: retrying with corrected
    /// input (or `mode=FORCE`) is what moves it forward.
    Precondition,
    /// The request itself does not satisfy a rule the server enforces.
    /// Fixing the request is the only way forward.
    Validation,
    /// The caller is not allowed to do this.
    Unauthorized,
    /// This client and server do not speak compatible contract revisions.
    Incompatible,
    /// The condition is expected to clear; retry the same request.
    Unavailable,
    /// A batch left entities in an intermediate state; a recovery pass
    /// (see `details`'s `journal`, when present) resolves it.
    PartialApply,
    NotFound,
    /// `operation_id` was reused for a request with a different digest: a
    /// concurrent-write conflict scoped to one idempotency key, distinct
    /// from a stale precondition against the entity's own state.
    OperationConflict,
    /// Nothing the caller can do; the server logged root-cause detail.
    Internal,
}

impl FailureCategory {
    #[must_use]
    pub fn next_action(self) -> NextAction {
        match self {
            FailureCategory::StaleState
            | FailureCategory::Precondition
            | FailureCategory::OperationConflict => NextAction::Replan,
            FailureCategory::Unavailable | FailureCategory::PartialApply => NextAction::Retry,
            FailureCategory::Validation => NextAction::FixInput,
            FailureCategory::Unauthorized
            | FailureCategory::Incompatible
            | FailureCategory::NotFound
            | FailureCategory::Internal => NextAction::None,
        }
    }
}

impl fmt::Display for FailureCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FailureCategory::StaleState => "stale_state",
            FailureCategory::Precondition => "precondition",
            FailureCategory::Validation => "validation",
            FailureCategory::Unauthorized => "unauthorized",
            FailureCategory::Incompatible => "incompatible",
            FailureCategory::Unavailable => "unavailable",
            FailureCategory::PartialApply => "partial_apply",
            FailureCategory::NotFound => "not_found",
            FailureCategory::OperationConflict => "operation_conflict",
            FailureCategory::Internal => "internal",
        })
    }
}

/// What the caller should do about a `Failure`. A plain string on the wire
/// (MCP `data.next_action`, the CLI's `next_action` field) so an agent can
/// switch on it without a schema for every category's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextAction {
    /// Read fresh state and plan again.
    Replan,
    /// Retry the same request unchanged.
    Retry,
    /// Fix the request; retrying it unchanged will not help.
    FixInput,
    /// Another call is still applying this `operation_id`; poll
    /// `GetOperation` instead of retrying the mutation itself.
    CheckOperation,
    /// This `operation_id` is already bound to a different request; retry
    /// with a new one instead of reusing it.
    UseNewOperationId,
    /// A `ListChanges` cursor failed to open; retry the poll with an empty
    /// `since_token` instead of reusing it.
    RestartListing,
    /// No retry helps.
    None,
}

impl fmt::Display for NextAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NextAction::Replan => "replan",
            NextAction::Retry => "retry",
            NextAction::FixInput => "fix_input",
            NextAction::CheckOperation => "check_operation",
            NextAction::UseNewOperationId => "use_new_operation_id",
            NextAction::RestartListing => "restart_listing",
            NextAction::None => "none",
        })
    }
}

/// A stable, machine-readable failure code (an `ErrorInfo.reason` or a
/// `BatchMutateResponse.failure[].code`; see
/// `docs/reference/failure-reasons.md`). Validated so a caller can trust its
/// shape without re-checking it: non-empty, `[A-Z0-9_]+`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct FailureCode(String);

impl FailureCode {
    pub fn new(value: impl Into<String>) -> anyhow::Result<Self> {
        let value = value.into();
        if value.is_empty() {
            anyhow::bail!("failure code must not be empty");
        }
        if !value
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            anyhow::bail!("failure code {value:?} must match [A-Z0-9_]+");
        }
        Ok(Self(value))
    }

    /// Build from one of this module's own literal match arms, which are
    /// valid by construction. A debug-only assertion catches a typo in a
    /// constant without paying a validation cost (or an `unwrap`) on every
    /// classified failure.
    fn known(code: &'static str) -> Self {
        debug_assert!(
            Self::new(code).is_ok(),
            "invalid failure code constant: {code}"
        );
        Self(code.to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FailureCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Structured, failure-specific detail (a stale plan's two revisions, a
/// partial apply's journal id, a rejected batch's validation issues, ...).
/// Shape varies by `code`; a caller keys off `category`/`code`, not this
/// map's keys, since they differ per failure.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FailureDetails(serde_json::Map<String, Value>);

impl FailureDetails {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_json(value: Value) -> Self {
        match value {
            Value::Object(map) => Self(map),
            _ => Self::default(),
        }
    }

    #[must_use]
    pub fn into_json(self) -> Value {
        Value::Object(self.0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A classified failure: what kind it is, what to do next, the stable code
/// that named it (when one was available), the human-readable message, and
/// any failure-specific detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub category: FailureCategory,
    pub next_action: NextAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<FailureCode>,
    pub message: String,
    #[serde(skip_serializing_if = "FailureDetails::is_empty")]
    pub details: FailureDetails,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Failure {
    #[must_use]
    pub fn new(category: FailureCategory, message: impl Into<String>) -> Self {
        Self {
            next_action: category.next_action(),
            category,
            code: None,
            message: message.into(),
            details: FailureDetails::empty(),
        }
    }

    #[must_use]
    pub fn with_code(mut self, code: FailureCode) -> Self {
        self.code = Some(code);
        self
    }

    #[must_use]
    pub fn with_details(mut self, details: FailureDetails) -> Self {
        self.details = details;
        self
    }

    #[must_use]
    pub fn with_next_action(mut self, next_action: NextAction) -> Self {
        self.next_action = next_action;
        self
    }

    /// Classify an error from anywhere in a surface's call chain (apply,
    /// diff, branch merge, a raw RPC). Walks typed errors first, then the
    /// `tonic::Status` at the chain's root. Never matches on error text.
    #[must_use]
    pub fn classify(err: &anyhow::Error) -> Self {
        if let Some(stale) = err.chain().find_map(|c| c.downcast_ref::<StalePlan>()) {
            return Self::new(FailureCategory::StaleState, stale.to_string())
                .with_details(FailureDetails::from_json(stale.to_json()));
        }
        if let Some(stale) = err
            .chain()
            .find_map(|c| c.downcast_ref::<StaleResolution>())
        {
            return Self::new(FailureCategory::StaleState, stale.to_string())
                .with_details(FailureDetails::from_json(stale.to_json()));
        }
        if let Some(rejected) = err.chain().find_map(|c| c.downcast_ref::<ApplyRejected>()) {
            return Self::from_apply_rejected(rejected);
        }
        if let Some(incompatible) = err
            .chain()
            .find_map(|c| c.downcast_ref::<CompatibilityError>())
        {
            return Self::from_compatibility_error(incompatible);
        }
        if let Some(invalid) = err.chain().find_map(|c| c.downcast_ref::<InvalidInput>()) {
            return Self::new(FailureCategory::Validation, invalid.to_string());
        }
        if let Some(status) = err.chain().find_map(|c| c.downcast_ref::<Status>()) {
            return Self::from_status(status);
        }
        Self::new(FailureCategory::Internal, format!("{err:#}"))
    }

    #[must_use]
    pub fn from_compatibility_error(err: &CompatibilityError) -> Self {
        if let CompatibilityError::Discovery(status) = err {
            return Self::from_status(status);
        }
        Self::new(FailureCategory::Incompatible, err.to_string())
    }

    /// Classify a `tonic::Status` returned directly by an RPC: its
    /// `ErrorInfo.reason` when the server attached one in `SERVER_ERROR_DOMAIN`
    /// and this client recognizes it, otherwise its gRPC code alone.
    #[must_use]
    pub fn from_status(status: &Status) -> Self {
        let reason = status
            .get_details_error_info()
            .filter(|info| info.domain == SERVER_ERROR_DOMAIN)
            .and_then(|info| Self::from_reason(&info.reason, status, &info.metadata));
        reason.unwrap_or_else(|| Self::from_code(status))
    }

    fn from_reason(
        reason: &str,
        status: &Status,
        metadata: &std::collections::HashMap<String, String>,
    ) -> Option<Self> {
        let (category, code, next_action): (FailureCategory, &'static str, Option<NextAction>) =
            match reason {
                "NOT_FOUND" => (FailureCategory::NotFound, "NOT_FOUND", None),
                "ALREADY_EXISTS" => (FailureCategory::Precondition, "ALREADY_EXISTS", None),
                "STALE_PRECONDITION" => (FailureCategory::StaleState, "STALE_PRECONDITION", None),
                "VALIDATION_FAILED" => (FailureCategory::Validation, "VALIDATION_FAILED", None),
                "UNAVAILABLE" => (FailureCategory::Unavailable, "UNAVAILABLE", None),
                "PARTIAL_APPLY" => (FailureCategory::PartialApply, "PARTIAL_APPLY", None),
                "INTERNAL" => (FailureCategory::Internal, "INTERNAL", None),
                "OPERATION_IN_PROGRESS" => (
                    FailureCategory::Unavailable,
                    "OPERATION_IN_PROGRESS",
                    Some(NextAction::CheckOperation),
                ),
                "OPERATION_ID_REUSED" => (
                    FailureCategory::OperationConflict,
                    "OPERATION_ID_REUSED",
                    Some(NextAction::UseNewOperationId),
                ),
                "NOT_WRITER" => (FailureCategory::Unavailable, "NOT_WRITER", None),
                "CURSOR_REJECTED" => (
                    FailureCategory::Validation,
                    "CURSOR_REJECTED",
                    Some(NextAction::RestartListing),
                ),
                _ => return None,
            };
        let details = match metadata.get("journal") {
            Some(journal) => FailureDetails::from_json(json!({ "journal": journal })),
            None => FailureDetails::empty(),
        };
        let failure = Self::new(category, status.message().to_string())
            .with_code(FailureCode::known(code))
            .with_details(details);
        Some(match next_action {
            Some(na) => failure.with_next_action(na),
            None => failure,
        })
    }

    /// Fallback classification from the gRPC code alone, for a status that
    /// carries no `ErrorInfo` this client recognizes (most `InvalidArgument`
    /// statuses raised directly by request validation, for instance).
    fn from_code(status: &Status) -> Self {
        let category = match status.code() {
            Code::Aborted => FailureCategory::StaleState,
            Code::FailedPrecondition | Code::AlreadyExists => FailureCategory::Precondition,
            Code::InvalidArgument | Code::OutOfRange => FailureCategory::Validation,
            Code::NotFound => FailureCategory::NotFound,
            Code::Unauthenticated | Code::PermissionDenied => FailureCategory::Unauthorized,
            Code::Cancelled | Code::DeadlineExceeded | Code::Unavailable => {
                FailureCategory::Unavailable
            }
            _ => FailureCategory::Internal,
        };
        Self::new(category, status.message().to_string())
    }

    /// A rejected batch (`BatchMutateResponse.status == FAILED`, with no
    /// `precondition_failure`): one op's store-failure code when exactly
    /// one issue was reported and it names one from the registry, otherwise
    /// `validation` (the common case: a schema/scenario validation error,
    /// which can report several issues from one op and uses its own code
    /// namespace, e.g. `SCENARIO_MISSING_REF`).
    fn from_apply_rejected(rejected: &ApplyRejected) -> Self {
        let classified = match rejected.issues.as_slice() {
            [issue] => batch_issue_category(&issue.code),
            _ => None,
        };
        let failure = Self::new(
            classified.map_or(FailureCategory::Validation, |(category, _)| category),
            rejected.to_string(),
        )
        .with_details(FailureDetails::from_json(rejected.to_json()));
        match classified {
            Some((_, code)) => failure.with_code(FailureCode::known(code)),
            None => failure,
        }
    }
}

/// `BatchMutateResponse.failure[].code` -> `FailureCategory`, for the codes
/// `docs/reference/failure-reasons.md` registers. Any other code (a schema
/// or scenario validation rule, e.g. `SCENARIO_MISSING_REF`) is not in this
/// table on purpose: those are per-rule codes in their own namespace, not
/// store-failure codes, and always classify as `validation`.
fn batch_issue_category(code: &str) -> Option<(FailureCategory, &'static str)> {
    Some(match code {
        "STALE_PRECONDITION" => (FailureCategory::StaleState, "STALE_PRECONDITION"),
        "ENTITY_NOT_FOUND" => (FailureCategory::NotFound, "ENTITY_NOT_FOUND"),
        "ENTITY_REFERENCED" => (FailureCategory::Precondition, "ENTITY_REFERENCED"),
        "INVALID_OP" => (FailureCategory::Validation, "INVALID_OP"),
        "BATCH_OP_FAILED" => (FailureCategory::Internal, "BATCH_OP_FAILED"),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use trogon_atlas_proto as pb;

    use super::*;

    #[test]
    fn category_display_matches_its_wire_serialization() {
        for category in [
            FailureCategory::StaleState,
            FailureCategory::Precondition,
            FailureCategory::Validation,
            FailureCategory::Unauthorized,
            FailureCategory::Incompatible,
            FailureCategory::Unavailable,
            FailureCategory::PartialApply,
            FailureCategory::NotFound,
            FailureCategory::OperationConflict,
            FailureCategory::Internal,
        ] {
            let serialized: String = serde_json::from_value(json!(category)).unwrap();
            assert_eq!(category.to_string(), serialized);
        }
    }

    #[test]
    fn next_action_display_matches_its_wire_serialization() {
        for action in [
            NextAction::Replan,
            NextAction::Retry,
            NextAction::FixInput,
            NextAction::None,
        ] {
            let serialized: String = serde_json::from_value(json!(action)).unwrap();
            assert_eq!(action.to_string(), serialized);
        }
    }

    #[test]
    fn failure_code_rejects_lowercase_and_empty() {
        assert!(FailureCode::new("").is_err());
        assert!(FailureCode::new("Not_Found").is_err());
        assert!(FailureCode::new("not-found").is_err());
        assert!(FailureCode::new("NOT_FOUND").is_ok());
        assert!(FailureCode::new("ENTITY_NOT_FOUND_2").is_ok());
    }

    #[test]
    fn stale_state_and_precondition_categories_resolve_to_replan() {
        assert_eq!(
            FailureCategory::StaleState.next_action(),
            NextAction::Replan
        );
        assert_eq!(
            FailureCategory::Precondition.next_action(),
            NextAction::Replan
        );
    }

    #[test]
    fn unavailable_and_partial_apply_resolve_to_retry() {
        assert_eq!(
            FailureCategory::Unavailable.next_action(),
            NextAction::Retry
        );
        assert_eq!(
            FailureCategory::PartialApply.next_action(),
            NextAction::Retry
        );
    }

    #[test]
    fn classify_a_stale_plan_carries_its_revisions() {
        let stale = StalePlan {
            entity: pb::EntityKey {
                kind: pb::EntityKind::Event,
                namespace: "shop".into(),
                slug: "order.placed".into(),
                version: 1,
            },
            expected: crate::precondition::Revision::Absent,
            actual: "7".parse().unwrap(),
        };
        let err = anyhow::Error::new(stale).context("apply_manifests failed");
        let failure = Failure::classify(&err);
        assert_eq!(failure.category, FailureCategory::StaleState);
        assert_eq!(failure.next_action, NextAction::Replan);
        assert_eq!(failure.details.0["actual_etag"], json!("7"));
    }

    #[test]
    fn classify_an_apply_rejected_with_a_single_known_code_uses_it() {
        let rejected = ApplyRejected {
            failed: "event:shop/order.placed@1".into(),
            issues: vec![pb::ValidationIssue {
                code: "ENTITY_REFERENCED".into(),
                message: "entity is referenced".into(),
                ..Default::default()
            }],
        };
        let err = anyhow::Error::new(rejected).context("BatchMutate failed");
        let failure = Failure::classify(&err);
        assert_eq!(failure.category, FailureCategory::Precondition);
        assert_eq!(failure.code.as_ref().unwrap().as_str(), "ENTITY_REFERENCED");
    }

    #[test]
    fn classify_an_apply_rejected_with_scenario_validation_issues_is_validation() {
        let rejected = ApplyRejected {
            failed: "event:shop/order.placed@1".into(),
            issues: vec![
                pb::ValidationIssue {
                    code: "SCENARIO_MISSING_REF".into(),
                    message: "missing ref".into(),
                    ..Default::default()
                },
                pb::ValidationIssue {
                    code: "SCENARIO_MISSING_REF".into(),
                    message: "another missing ref".into(),
                    ..Default::default()
                },
            ],
        };
        let err = anyhow::Error::new(rejected);
        let failure = Failure::classify(&err);
        assert_eq!(failure.category, FailureCategory::Validation);
        assert_eq!(failure.code, None);
        assert_eq!(failure.next_action, NextAction::FixInput);
    }

    #[test]
    fn classify_an_invalid_input_keeps_its_message_as_validation() {
        let err = invalid_input("namespace \"shop\" has no entities");
        let failure = Failure::classify(&err);
        assert_eq!(failure.category, FailureCategory::Validation);
        assert_eq!(failure.next_action, NextAction::FixInput);
        assert_eq!(failure.code, None);
        assert!(failure.message.contains("no entities"), "{failure:?}");
    }

    #[test]
    fn classify_prefers_typed_errors_over_a_wrapped_status() {
        // A plain tonic::Status with an unrelated domain's ErrorInfo must
        // not be mistaken for this server's registry.
        let status = Status::aborted("etag mismatch");
        let err = anyhow::Error::new(status).context("apply_manifests failed");
        assert_eq!(
            Failure::classify(&err).category,
            FailureCategory::StaleState
        );
    }

    #[test]
    fn status_with_server_error_info_classifies_by_reason_not_just_code() {
        use std::collections::HashMap;

        use tonic_types::{ErrorDetails, StatusExt};

        let details = ErrorDetails::with_error_info(
            "PARTIAL_APPLY",
            SERVER_ERROR_DOMAIN,
            HashMap::from([("journal".to_string(), "journal-123".to_string())]),
        );
        let status =
            Status::with_error_details(Code::Unavailable, "batch partially applied", details);
        let failure = Failure::from_status(&status);
        assert_eq!(failure.category, FailureCategory::PartialApply);
        assert_eq!(failure.next_action, NextAction::Retry);
        assert_eq!(failure.code.unwrap().as_str(), "PARTIAL_APPLY");
        assert_eq!(failure.details.0["journal"], json!("journal-123"));
    }

    #[test]
    fn status_with_not_writer_reason_classifies_as_unavailable_with_its_metadata() {
        use std::collections::HashMap;

        use tonic_types::{ErrorDetails, StatusExt};

        let details = ErrorDetails::with_error_info(
            "NOT_WRITER",
            SERVER_ERROR_DOMAIN,
            HashMap::from([
                ("role".to_string(), "standby".to_string()),
                ("epoch".to_string(), "3".to_string()),
            ]),
        );
        let status = Status::with_error_details(
            Code::Unavailable,
            "this process is not the current writer",
            details,
        );
        let failure = Failure::from_status(&status);
        assert_eq!(failure.category, FailureCategory::Unavailable);
        assert_eq!(failure.next_action, NextAction::Retry);
        assert_eq!(failure.code.unwrap().as_str(), "NOT_WRITER");
    }

    #[test]
    fn status_without_error_info_falls_back_to_its_code() {
        let status = Status::not_found("entity not found");
        let failure = Failure::from_status(&status);
        assert_eq!(failure.category, FailureCategory::NotFound);
        assert_eq!(failure.code, None);
    }

    #[test]
    fn status_from_an_unrelated_domain_falls_back_to_its_code() {
        use std::collections::HashMap;

        use tonic_types::{ErrorDetails, StatusExt};

        let details =
            ErrorDetails::with_error_info("SOME_REASON", "unrelated.service.v1", HashMap::new());
        let status = Status::with_error_details(Code::NotFound, "not found", details);
        let failure = Failure::from_status(&status);
        assert_eq!(failure.category, FailureCategory::NotFound);
        assert_eq!(failure.code, None);
    }

    #[test]
    fn incompatible_discovery_failure_classifies_by_its_underlying_status() {
        let err = CompatibilityError::Discovery(Box::new(Status::unavailable("no route")));
        let failure = Failure::from_compatibility_error(&err);
        assert_eq!(failure.category, FailureCategory::Unavailable);
    }

    #[test]
    fn incompatible_schema_mismatch_classifies_as_incompatible() {
        let err = CompatibilityError::SchemaMismatch {
            server: "eventmodel.v2".into(),
            client: "trogonatlas.eventmodel.v1alpha1",
        };
        let failure = Failure::from_compatibility_error(&err);
        assert_eq!(failure.category, FailureCategory::Incompatible);
        assert_eq!(failure.next_action, NextAction::None);
    }

    #[test]
    fn an_untyped_error_classifies_as_internal() {
        let err = anyhow::anyhow!("something unexpected happened");
        let failure = Failure::classify(&err);
        assert_eq!(failure.category, FailureCategory::Internal);
    }
}
