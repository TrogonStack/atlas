// `tonic::Status` is ~176 bytes by design; every gRPC boundary function that
// returns `Result<_, Status>` trips `result_large_err`. We cannot reduce the
// size (it is a third-party type); allow it once for the whole file instead of
// repeating the attribute on every function.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;

use tonic::{Code, Status};
use tonic_types::{ErrorDetails, StatusExt};
use trogon_atlas_proto as pb;
use trogon_atlas_store::StoreError;

/// `ErrorInfo.domain` for every status this server attaches error details to.
/// Lets a client tell "this server's STALE_PRECONDITION" apart from a reason
/// string coined by an unrelated gRPC service behind the same proxy.
pub const ERROR_DOMAIN: &str = "trogonatlas.api.eventmodel.v1alpha1";

/// Attach an `ErrorInfo` (domain `ERROR_DOMAIN`, the given stable `reason`,
/// and `metadata`) to a status, so a client can classify the failure without
/// parsing `message()`. `message()` stays human-readable prose; `reason` is
/// the part a program is meant to switch on.
fn with_reason(code: Code, message: impl Into<String>, reason: &str) -> Status {
    with_reason_and_metadata(code, message, reason, HashMap::new())
}

fn with_reason_and_metadata(
    code: Code,
    message: impl Into<String>,
    reason: &str,
    metadata: HashMap<String, String>,
) -> Status {
    let details = ErrorDetails::with_error_info(reason, ERROR_DOMAIN, metadata);
    Status::with_error_details(code, message, details)
}

/// Hard ceiling on entity `doc` field length in bytes. Larger docs bloat the
/// Tantivy index and the in-memory snapshot proportionally; 64 KiB is well
/// above any real design document and still fits in a single gRPC message.
pub const MAX_DOC_BYTES: usize = 64 * 1024;

/// Hard ceiling on entity `title` field length in bytes.
pub const MAX_TITLE_BYTES: usize = 1024;

/// Hard ceiling on the number of fields an entity's `schema` declares. Protects the
/// Tantivy field-body index and per-entity memory from unbounded growth.
pub const MAX_FIELDS_COUNT: usize = 1000;

/// Hard ceiling on the number of `metadata` (Any) entries per entity.
pub const MAX_METADATA_COUNT: usize = 100;

/// Reject any namespace/slug that could escape the on-disk mirror layout,
/// confuse NATS subject parsing, or contain control characters. Applied at
/// the gRPC boundary so the same predicate guards every mutation path
/// (single entity, batch, dry-run) and every storage backend.
///
/// Delegates to `trogon_atlas_core::validate_id_component` so this is the
/// single source of truth for "what's a safe component"; the git mirror
/// and any future NATS subject builder reuse the same predicate.
pub fn validate_id_component(label: &str, value: &str) -> Result<(), Status> {
    trogon_atlas_core::validate_id_component(value)
        .map_err(|e| Status::invalid_argument(format!("{label} {e}")))
}

/// Validate a branch name, accepting both the classic form and an owned
/// form (`@<owner>:<name>`, see `crate::ownership::OwnedBranchName`). An
/// owned-shaped name is validated by `OwnedBranchName::try_parse` itself
/// (owner charset, then the name half through
/// [`validate_classic_branch_name`]); anything else falls through to the
/// classic grammar unchanged.
pub fn validate_branch_name(name: &str) -> Result<(), Status> {
    if let Some(result) = crate::ownership::OwnedBranchName::try_parse(name) {
        return result.map(|_| ());
    }
    validate_classic_branch_name(name)
}

/// Validate a branch name (Phase 1: Isolation): at most one `/`-separated
/// segment pair (e.g. `team/feature-x`), each segment subject to the same
/// charset rule as namespace/slug components
/// (`trogon_atlas_core::validate_id_component`), and the whole name must not
/// be the reserved value `"meta"` (case-insensitive) -- see
/// `trogon_atlas_store::key::is_reserved_branch_name` for why that name is
/// unsafe as a KV bucket key prefix.
///
/// `pub(crate)` rather than private: `OwnedBranchName::build` validates its
/// name half through this directly, never through [`validate_branch_name`],
/// so a name half that is itself owned-shaped (e.g. `@a:@b:c`) cannot
/// recurse into a second round of owner parsing.
pub(crate) fn validate_classic_branch_name(name: &str) -> Result<(), Status> {
    if name.is_empty() {
        return Err(Status::invalid_argument("branch name must not be empty"));
    }
    if trogon_atlas_store::key::is_reserved_branch_name(name) {
        return Err(Status::invalid_argument(format!(
            "branch name {name:?} is reserved"
        )));
    }
    let mut segments = name.splitn(3, '/');
    let first = segments.next().unwrap_or_default();
    let second = segments.next();
    if segments.next().is_some() {
        return Err(Status::invalid_argument(
            "branch name must contain at most one '/'",
        ));
    }
    validate_id_component("branch name segment", first)?;
    if let Some(second) = second {
        validate_id_component("branch name segment", second)?;
    }
    Ok(())
}

pub fn kind_from_i32(value: i32, field: &str) -> Result<pb::EntityKind, Status> {
    let k = pb::EntityKind::try_from(value)
        .map_err(|_| Status::invalid_argument(format!("{field}: unknown EntityKind {value}")))?;
    if matches!(k, pb::EntityKind::Unspecified) {
        return Err(Status::invalid_argument(format!(
            "{field}: must not be UNSPECIFIED"
        )));
    }
    Ok(k)
}

pub fn require_id<'a>(id: Option<&'a pb::Id>, field: &str) -> Result<&'a pb::Id, Status> {
    let id = id.ok_or_else(|| Status::invalid_argument(format!("{field}: id is required")))?;
    if id.slug.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{field}.id.slug is required"
        )));
    }
    let scope = if field.is_empty() {
        "id".to_string()
    } else {
        format!("{field}.id")
    };
    validate_id_component(&format!("{scope}.namespace"), &id.namespace)?;
    validate_id_component(&format!("{scope}.slug"), &id.slug)?;
    Ok(id)
}

/// Stable `ErrorInfo.reason` values this server attaches to RPC statuses.
/// See `docs/reference/failure-reasons.md` for what each one means to a
/// caller and which `FailureCategory` it classifies into.
pub mod reason {
    pub const NOT_FOUND: &str = "NOT_FOUND";
    pub const ALREADY_EXISTS: &str = "ALREADY_EXISTS";
    pub const STALE_PRECONDITION: &str = "STALE_PRECONDITION";
    pub const VALIDATION_FAILED: &str = "VALIDATION_FAILED";
    pub const UNAVAILABLE: &str = "UNAVAILABLE";
    pub const INTERNAL: &str = "INTERNAL";
    pub const PARTIAL_APPLY: &str = "PARTIAL_APPLY";
    pub const OPERATION_IN_PROGRESS: &str = "OPERATION_IN_PROGRESS";
    pub const OPERATION_ID_REUSED: &str = "OPERATION_ID_REUSED";
    pub const NOT_WRITER: &str = "NOT_WRITER";
    pub const CURSOR_REJECTED: &str = "CURSOR_REJECTED";
    pub const BREAKING_CHANGE: &str = "BREAKING_CHANGE";
}

/// `UNAVAILABLE`: another call already owns this `operation_id` and has not
/// settled it yet. The caller should back off and retry rather than treat
/// the mutation as failed.
pub fn operation_in_progress(message: impl Into<String>) -> Status {
    with_reason(Code::Unavailable, message, reason::OPERATION_IN_PROGRESS)
}

/// `ALREADY_EXISTS`: `operation_id` was already claimed for a request with a
/// different canonical digest. Reusing an id for a different request would
/// otherwise silently mean two different things.
pub fn operation_id_reused(message: impl Into<String>) -> Status {
    with_reason(Code::AlreadyExists, message, reason::OPERATION_ID_REUSED)
}

/// `INVALID_ARGUMENT`: a `ListChanges` `since_token` carried the sealed
/// cursor prefix but failed to open against this process's key. Distinct
/// from the plain `invalid_argument("invalid since_token")` a garbage or
/// foreign-format token gets, because its next action is narrower: restart
/// the listing with an empty `since_token` rather than guess at a fix.
pub fn cursor_rejected_err(message: impl Into<String>) -> Status {
    with_reason(Code::InvalidArgument, message, reason::CURSOR_REJECTED)
}

pub fn store_err(err: StoreError) -> Status {
    match err {
        StoreError::NotFound => with_reason(Code::NotFound, "entity not found", reason::NOT_FOUND),
        StoreError::AlreadyExists => with_reason(
            Code::AlreadyExists,
            "entity already exists",
            reason::ALREADY_EXISTS,
        ),
        StoreError::EtagMismatch { expected, found } => {
            tracing::debug!(expected = %expected, found = %found, "etag mismatch");
            with_reason(Code::Aborted, "etag mismatch", reason::STALE_PRECONDITION)
        }
        StoreError::InvalidArgument(msg) => {
            with_reason(Code::InvalidArgument, msg, reason::VALIDATION_FAILED)
        }
        StoreError::Unavailable(msg) => with_reason(Code::Unavailable, msg, reason::UNAVAILABLE),
        StoreError::Backend(msg) => {
            // Backend messages can carry NATS cluster addresses, bucket
            // names, or filesystem paths -- log full detail server-side and
            // return a generic status to the client.
            tracing::error!(error = %msg, "storage backend error");
            with_reason(Code::Internal, "storage error", reason::INTERNAL)
        }
        StoreError::PartialApply {
            keys,
            journal: Some(journal),
        } => {
            tracing::error!(
                affected_keys = ?keys,
                journal = %journal,
                "batch apply partially applied; recovery pending"
            );
            with_reason_and_metadata(
                Code::Unavailable,
                format!("batch apply partially applied; recovery pending for journal {journal}"),
                reason::PARTIAL_APPLY,
                HashMap::from([("journal".to_string(), journal.as_str().to_string())]),
            )
        }
        StoreError::PartialApply {
            keys,
            journal: None,
        } => {
            tracing::error!(
                affected_keys = ?keys,
                "unjournaled batch partially applied; affected keys need manual repair"
            );
            with_reason(
                Code::Unavailable,
                "batch apply partially applied; affected keys need manual repair",
                reason::PARTIAL_APPLY,
            )
        }
        StoreError::ChangeEventLost => {
            tracing::error!("change event lost; entity write succeeded but change feed has a gap");
            with_reason(
                Code::Internal,
                "change event publish failed; entity was written but change feed may have a gap",
                reason::INTERNAL,
            )
        }
        StoreError::CorruptEntry { key, reason: why } => {
            tracing::error!(key = %key, reason = %why, "corrupt entity entry in store");
            with_reason(Code::Internal, "storage error", reason::INTERNAL)
        }
        StoreError::BatchFailed { index, source } => {
            tracing::error!(index, error = %source, "batch op failed");
            store_err(*source)
        }
        StoreError::SchemaMismatch { expected, found } => {
            tracing::error!(expected = %expected, found = %found, "schema version mismatch in stored entry");
            with_reason(
                Code::Internal,
                "storage schema version mismatch",
                reason::INTERNAL,
            )
        }
        StoreError::LegacyFieldsUnmigrated { rows } => {
            tracing::error!(rows, "store holds rows in the retired fields layout");
            with_reason(
                Code::Internal,
                "storage holds unmigrated rows",
                reason::INTERNAL,
            )
        }
        StoreError::NotWriter { role, epoch } => with_reason_and_metadata(
            Code::Unavailable,
            format!("this process is not the current writer (role: {role}, epoch: {epoch})"),
            reason::NOT_WRITER,
            HashMap::from([
                ("role".to_string(), role.to_string()),
                ("epoch".to_string(), epoch.to_string()),
            ]),
        ),
    }
}

pub fn entity_kind(entity: &pb::Entity) -> Result<pb::EntityKind, Status> {
    use pb::entity::Kind as K;
    match entity.kind.as_ref() {
        Some(K::Event(_)) => Ok(pb::EntityKind::Event),
        Some(K::Command(_)) => Ok(pb::EntityKind::Command),
        Some(K::ReadModel(_)) => Ok(pb::EntityKind::ReadModel),
        Some(K::Processor(_)) => Ok(pb::EntityKind::Processor),
        Some(K::Ui(_)) => Ok(pb::EntityKind::Ui),
        Some(K::Persona(_)) => Ok(pb::EntityKind::Persona),
        Some(K::Swimlane(_)) => Ok(pb::EntityKind::Swimlane),
        Some(K::CommandSlice(_)) => Ok(pb::EntityKind::CommandSlice),
        Some(K::ReadModelSlice(_)) => Ok(pb::EntityKind::ReadModelSlice),
        Some(K::AutomationSlice(_)) => Ok(pb::EntityKind::AutomationSlice),
        Some(K::Storyboard(_)) => Ok(pb::EntityKind::Storyboard),
        Some(K::EventModel(_)) => Ok(pb::EntityKind::EventModel),
        Some(K::Component(_)) => Ok(pb::EntityKind::Component),
        Some(K::ExternalSystem(_)) => Ok(pb::EntityKind::ExternalSystem),
        Some(K::Tracker(_)) => Ok(pb::EntityKind::Tracker),
        Some(K::BoundedContext(_)) => Ok(pb::EntityKind::BoundedContext),
        Some(K::Domain(_)) => Ok(pb::EntityKind::Domain),
        Some(K::Subdomain(_)) => Ok(pb::EntityKind::Subdomain),
        Some(K::Schema(_)) => Ok(pb::EntityKind::Schema),
        Some(K::Project(_)) => Ok(pb::EntityKind::Project),
        Some(K::Screen(_)) => Ok(pb::EntityKind::Screen),
        Some(K::Term(_)) => Ok(pb::EntityKind::Term),
        Some(K::Ambiguity(_)) => Ok(pb::EntityKind::Ambiguity),
        Some(K::UiSlice(_)) => Ok(pb::EntityKind::UiSlice),
        Some(K::ServiceLevelIndicator(_)) => Ok(pb::EntityKind::ServiceLevelIndicator),
        Some(K::ServiceLevelObjective(_)) => Ok(pb::EntityKind::ServiceLevelObjective),
        Some(K::AlertPolicy(_)) => Ok(pb::EntityKind::AlertPolicy),
        Some(K::AlertNotificationTarget(_)) => Ok(pb::EntityKind::AlertNotificationTarget),
        Some(K::TypeLibrary(_)) => Ok(pb::EntityKind::TypeLibrary),
        None => Err(Status::invalid_argument("entity.kind oneof is required")),
    }
}

pub fn entity_id(entity: &pb::Entity) -> Result<&pb::Id, Status> {
    use pb::entity::Kind as K;
    let id = match entity.kind.as_ref() {
        Some(K::Event(x)) => x.id.as_ref(),
        Some(K::Command(x)) => x.id.as_ref(),
        Some(K::ReadModel(x)) => x.id.as_ref(),
        Some(K::Processor(x)) => x.id.as_ref(),
        Some(K::Ui(x)) => x.id.as_ref(),
        Some(K::Persona(x)) => x.id.as_ref(),
        Some(K::Swimlane(x)) => x.id.as_ref(),
        Some(K::CommandSlice(x)) => x.id.as_ref(),
        Some(K::ReadModelSlice(x)) => x.id.as_ref(),
        Some(K::AutomationSlice(x)) => x.id.as_ref(),
        Some(K::Storyboard(x)) => x.id.as_ref(),
        Some(K::EventModel(x)) => x.id.as_ref(),
        Some(K::Component(x)) => x.id.as_ref(),
        Some(K::ExternalSystem(x)) => x.id.as_ref(),
        Some(K::Tracker(x)) => x.id.as_ref(),
        Some(K::BoundedContext(x)) => x.id.as_ref(),
        Some(K::Domain(x)) => x.id.as_ref(),
        Some(K::Subdomain(x)) => x.id.as_ref(),
        Some(K::Schema(x)) => x.id.as_ref(),
        Some(K::Project(x)) => x.id.as_ref(),
        Some(K::Screen(x)) => x.id.as_ref(),
        Some(K::Term(x)) => x.id.as_ref(),
        Some(K::Ambiguity(x)) => x.id.as_ref(),
        Some(K::UiSlice(x)) => x.id.as_ref(),
        Some(K::ServiceLevelIndicator(x)) => x.id.as_ref(),
        Some(K::ServiceLevelObjective(x)) => x.id.as_ref(),
        Some(K::AlertPolicy(x)) => x.id.as_ref(),
        Some(K::AlertNotificationTarget(x)) => x.id.as_ref(),
        Some(K::TypeLibrary(x)) => x.id.as_ref(),
        None => return Err(Status::invalid_argument("entity.kind oneof is required")),
    };
    let id = id.ok_or_else(|| Status::invalid_argument("entity.id is required"))?;
    if id.slug.is_empty() {
        return Err(Status::invalid_argument("entity.id.slug is required"));
    }
    validate_id_component("entity.id.namespace", &id.namespace)?;
    validate_id_component("entity.id.slug", &id.slug)?;
    Ok(id)
}

pub fn entity_to_ref(entity: &pb::Entity) -> Result<pb::EntityRef, Status> {
    Ok(pb::EntityRef {
        kind: entity_kind(entity)? as i32,
        id: Some(entity_id(entity)?.clone()),
    })
}

/// Enforce hard size ceilings on the mutable content fields of an entity.
/// Called at every mutation boundary (`put_entity`, `batch_mutate`) before the
/// entity reaches the store or the git mirror. Keeps the Tantivy index and
/// in-memory snapshot bounded regardless of gRPC message-size limits.
pub fn validate_entity_content(entity: &pb::Entity) -> Result<(), Status> {
    use pb::entity::Kind as K;

    let (title, doc, fields_len, metadata_len) = match entity.kind.as_ref() {
        Some(K::Event(x)) => (
            x.title.as_str(),
            x.doc.as_str(),
            trogon_atlas_core::schema::schema_fields(x.schema.as_ref()).len(),
            x.metadata.len(),
        ),
        Some(K::Command(x)) => (
            x.title.as_str(),
            x.doc.as_str(),
            trogon_atlas_core::schema::schema_fields(x.schema.as_ref()).len(),
            x.metadata.len(),
        ),
        Some(K::ReadModel(x)) => (
            x.title.as_str(),
            x.doc.as_str(),
            trogon_atlas_core::schema::schema_fields(x.schema.as_ref()).len(),
            x.metadata.len(),
        ),
        Some(K::Processor(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Ui(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Persona(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Swimlane(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::CommandSlice(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::ReadModelSlice(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::AutomationSlice(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::UiSlice(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::Storyboard(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::EventModel(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Component(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::ExternalSystem(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Tracker(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::BoundedContext(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Domain(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Subdomain(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Schema(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::Project(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::Screen(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::Term(x)) => (x.title.as_str(), x.doc.as_str(), 0, 0),
        Some(K::Ambiguity(x)) => ("", x.doc.as_str(), 0, 0),
        Some(K::ServiceLevelIndicator(x)) => {
            (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len())
        }
        Some(K::ServiceLevelObjective(x)) => {
            (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len())
        }
        Some(K::AlertPolicy(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        Some(K::AlertNotificationTarget(x)) => {
            (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len())
        }
        Some(K::TypeLibrary(x)) => (x.title.as_str(), x.doc.as_str(), 0, x.metadata.len()),
        None => return Ok(()),
    };

    if title.len() > MAX_TITLE_BYTES {
        return Err(Status::invalid_argument(format!(
            "entity.title exceeds maximum of {MAX_TITLE_BYTES} bytes (got {})",
            title.len()
        )));
    }
    if doc.len() > MAX_DOC_BYTES {
        return Err(Status::invalid_argument(format!(
            "entity.doc exceeds maximum of {MAX_DOC_BYTES} bytes (got {})",
            doc.len()
        )));
    }
    if fields_len > MAX_FIELDS_COUNT {
        return Err(Status::invalid_argument(format!(
            "entity.schema.fields length {fields_len} exceeds maximum of {MAX_FIELDS_COUNT}"
        )));
    }
    if metadata_len > MAX_METADATA_COUNT {
        return Err(Status::invalid_argument(format!(
            "entity.metadata length {metadata_len} exceeds maximum of {MAX_METADATA_COUNT}"
        )));
    }
    Ok(())
}

impl From<crate::auth::Role> for pb::Role {
    fn from(role: crate::auth::Role) -> Self {
        match role {
            crate::auth::Role::Reader => Self::Reader,
            crate::auth::Role::Writer => Self::Writer,
            crate::auth::Role::Admin => Self::Admin,
        }
    }
}

impl From<crate::auth::PrincipalKind> for pb::PrincipalKind {
    fn from(kind: crate::auth::PrincipalKind) -> Self {
        match kind {
            crate::auth::PrincipalKind::User => Self::User,
            crate::auth::PrincipalKind::Agent => Self::Agent,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod validate_id_component_tests {
    use trogon_atlas_store::StoreError;

    use super::{store_err, validate_id_component};

    #[test]
    fn accepts_typical_slugs() {
        for v in [
            "orders",
            "ecommerce",
            "order.placed",
            "order-placed",
            "v2",
            "a_b",
        ] {
            assert!(validate_id_component("test", v).is_ok(), "rejected {v:?}");
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_id_component("test", "").is_err());
    }

    #[test]
    fn rejects_path_traversal() {
        for v in ["..", ".", "../etc", "./foo", "/abs", "a/b", "a\\b"] {
            assert!(
                validate_id_component("test", v).is_err(),
                "accepted hostile {v:?}"
            );
        }
    }

    #[test]
    fn rejects_leading_dot() {
        for v in [".git", ".hidden", "..rc"] {
            assert!(
                validate_id_component("test", v).is_err(),
                "accepted leading-dot {v:?}"
            );
        }
    }

    #[test]
    fn rejects_control_and_unicode() {
        for v in ["a\nb", "a\tb", "a b", "naïve", "a\u{202E}b"] {
            assert!(
                validate_id_component("test", v).is_err(),
                "accepted hostile {v:?}"
            );
        }
    }

    #[test]
    fn rejects_overlong() {
        let long = "a".repeat(201);
        assert!(validate_id_component("test", &long).is_err());
    }

    #[test]
    fn batch_failed_preserves_underlying_etag_mismatch_as_aborted() {
        let status = store_err(StoreError::BatchFailed {
            index: 2,
            source: Box::new(StoreError::EtagMismatch {
                expected: "want".into(),
                found: "have".into(),
            }),
        });
        assert_eq!(
            status.code(),
            tonic::Code::Aborted,
            "BatchFailed wrapping EtagMismatch must surface as ABORTED (got {:?}: {})",
            status.code(),
            status.message()
        );
    }

    #[test]
    fn not_found_carries_error_info_with_domain_and_reason() {
        use tonic_types::StatusExt;

        let status = store_err(StoreError::NotFound);
        let info = status
            .get_details_error_info()
            .expect("NOT_FOUND status must carry ErrorInfo");
        assert_eq!(info.domain, super::ERROR_DOMAIN);
        assert_eq!(info.reason, super::reason::NOT_FOUND);
    }

    #[test]
    fn partial_apply_with_journal_carries_journal_metadata() {
        use tonic_types::StatusExt;

        let status = store_err(StoreError::PartialApply {
            keys: vec!["Event:ns/slug".into()],
            journal: Some(trogon_atlas_store::recovery::BatchJournalId::new(
                "journal-123",
            )),
        });
        assert_eq!(status.code(), tonic::Code::Unavailable);
        let info = status
            .get_details_error_info()
            .expect("PARTIAL_APPLY status must carry ErrorInfo");
        assert_eq!(info.reason, super::reason::PARTIAL_APPLY);
        assert_eq!(
            info.metadata.get("journal").map(String::as_str),
            Some("journal-123")
        );
    }

    #[test]
    fn partial_apply_without_journal_carries_reason_but_no_journal_metadata() {
        use tonic_types::StatusExt;

        let status = store_err(StoreError::PartialApply {
            keys: vec!["Event:ns/slug".into()],
            journal: None,
        });
        let info = status
            .get_details_error_info()
            .expect("PARTIAL_APPLY status must carry ErrorInfo");
        assert_eq!(info.reason, super::reason::PARTIAL_APPLY);
        assert!(!info.metadata.contains_key("journal"));
    }

    #[test]
    fn not_writer_carries_role_and_epoch_metadata() {
        use tonic_types::StatusExt;
        use trogon_atlas_core::{Epoch, WriterRole};

        let status = store_err(StoreError::NotWriter {
            role: WriterRole::Standby,
            epoch: Epoch::new(3),
        });
        assert_eq!(status.code(), tonic::Code::Unavailable);
        let info = status
            .get_details_error_info()
            .expect("NOT_WRITER status must carry ErrorInfo");
        assert_eq!(info.reason, super::reason::NOT_WRITER);
        assert_eq!(
            info.metadata.get("role").map(String::as_str),
            Some("standby")
        );
        assert_eq!(info.metadata.get("epoch").map(String::as_str), Some("3"));
    }

    #[test]
    fn etag_mismatch_classifies_as_stale_precondition_not_generic_internal() {
        use tonic_types::StatusExt;

        let status = store_err(StoreError::EtagMismatch {
            expected: "1".into(),
            found: "2".into(),
        });
        let info = status.get_details_error_info().unwrap();
        assert_eq!(info.reason, super::reason::STALE_PRECONDITION);
    }

    #[test]
    fn backend_error_sanitizes_message_but_still_carries_internal_reason() {
        use tonic_types::StatusExt;

        let status = store_err(StoreError::Backend("nats://10.0.0.5:4222 down".into()));
        assert!(!status.message().contains("10.0.0.5"));
        let info = status.get_details_error_info().unwrap();
        assert_eq!(info.reason, super::reason::INTERNAL);
    }

    #[test]
    fn batch_failed_preserves_underlying_not_found() {
        let status = store_err(StoreError::BatchFailed {
            index: 0,
            source: Box::new(StoreError::NotFound),
        });
        assert_eq!(
            status.code(),
            tonic::Code::NotFound,
            "BatchFailed wrapping NotFound must surface as NOT_FOUND (got {:?}: {})",
            status.code(),
            status.message()
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod validate_branch_name_tests {
    use super::{validate_branch_name, validate_classic_branch_name};

    #[test]
    fn accepts_an_owned_branch() {
        assert!(validate_branch_name("@acme:feature-x").is_ok());
    }

    #[test]
    fn rejects_an_owned_branch_with_a_bad_owner() {
        assert!(validate_branch_name("@:feature-x").is_err());
    }

    #[test]
    fn rejects_an_owned_branch_with_a_bad_name() {
        assert!(validate_branch_name("@acme:").is_err());
    }

    #[test]
    fn still_accepts_a_plain_branch() {
        assert!(validate_branch_name("feature-x").is_ok());
    }

    #[test]
    fn classic_validator_rejects_the_owned_shape_directly() {
        // The name half of an owned branch is validated through this
        // classic-only path (never back through `validate_branch_name`), so
        // it must still reject `@`/`:` the same way the shared id-component
        // charset always has.
        assert!(validate_classic_branch_name("@acme:feature-x").is_err());
    }
}
