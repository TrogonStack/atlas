//! The one JSON document a structured-output surface (trogon-atlas's `--format
//! json`) prints for a single command invocation, so a calling agent parses
//! one value instead of scraping free-form text. Diagnostics unrelated to
//! the result (progress, warnings) go elsewhere (stderr, for the CLI); this
//! envelope is only ever the result itself.

use serde::Serialize;
use serde_json::Value;

use crate::failure::Failure;

/// Schema tag for `CommandOutcome`, so a consumer can tell this shape apart
/// from any other JSON document it might see on the same stream, and detect
/// a breaking change to the envelope itself.
pub const SCHEMA: &str = "trogon-atlas.result.v1";

#[derive(Debug, Clone, Serialize)]
pub struct CommandOutcome {
    pub schema: &'static str,
    pub ok: bool,
    /// Command-specific payload (apply's planned changes, diff's unified
    /// diff, export's written paths, ...). Shape varies by command; a
    /// caller that only needs `ok`/`failure` can ignore it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    /// The idempotency key a mutating command minted or was given
    /// (`--operation-id`), echoed back so a caller that only sees this
    /// envelope (e.g. a crashed run's last line) can still look the
    /// operation up with `trogon-atlas operation get`. Absent for commands that
    /// carry no idempotency key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

impl CommandOutcome {
    #[must_use]
    pub fn ok(data: Value) -> Self {
        Self {
            schema: SCHEMA,
            ok: true,
            data: Some(data),
            failure: None,
            operation_id: None,
        }
    }

    #[must_use]
    pub fn ok_empty() -> Self {
        Self {
            schema: SCHEMA,
            ok: true,
            data: None,
            failure: None,
            operation_id: None,
        }
    }

    #[must_use]
    pub fn failed(failure: Failure) -> Self {
        Self {
            schema: SCHEMA,
            ok: false,
            data: None,
            failure: Some(failure),
            operation_id: None,
        }
    }

    #[must_use]
    pub fn with_operation_id(mut self, operation_id: impl Into<String>) -> Self {
        self.operation_id = Some(operation_id.into());
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::failure::FailureCategory;

    #[test]
    fn ok_outcome_serializes_without_a_failure_field() {
        let outcome = CommandOutcome::ok(json!({"lines": ["event:shop/x@1 created"]}));
        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["schema"], SCHEMA);
        assert_eq!(value["ok"], true);
        assert!(value.get("failure").is_none());
    }

    #[test]
    fn failed_outcome_carries_the_classified_failure_and_no_data() {
        let failure = Failure::new(FailureCategory::Validation, "bad input");
        let outcome = CommandOutcome::failed(failure);
        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["ok"], false);
        assert!(value.get("data").is_none());
        assert_eq!(value["failure"]["category"], "validation");
        assert_eq!(value["failure"]["next_action"], "fix_input");
    }

    #[test]
    fn operation_id_is_absent_by_default_and_present_once_attached() {
        let outcome = CommandOutcome::ok_empty();
        let value = serde_json::to_value(&outcome).unwrap();
        assert!(value.get("operation_id").is_none());

        let outcome = CommandOutcome::ok_empty().with_operation_id("op-123");
        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["operation_id"], "op-123");
    }

    #[test]
    fn failed_outcome_can_also_carry_the_operation_id() {
        let failure = Failure::new(FailureCategory::OperationConflict, "already claimed");
        let outcome = CommandOutcome::failed(failure).with_operation_id("op-456");
        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["operation_id"], "op-456");
    }
}
