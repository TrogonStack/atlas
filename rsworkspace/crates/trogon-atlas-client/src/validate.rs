//! Model-level validation (`ValidateEventModel`) of a stored EventModel.
//! Per-entity checks run on every write; the rules that need the whole
//! model in view (storyboard entries, projection order, field provenance)
//! run only here.

use anyhow::{Context as _, Result};
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    view::{self, EntityAddress},
};

/// Parse `<namespace>/<model>[@version]`; `version: None` means latest.
pub fn parse_model_address(s: &str) -> Result<EntityAddress> {
    if s.matches('/').count() != 1 {
        crate::invalid_input!("expected <namespace>/<model>[@version], got {s:?}");
    }
    view::parse_address(&format!("event_model/{s}"))
}

/// Validate the addressed EventModel and return the stored id it resolved
/// to along with the server's response. Both reads carry the client's branch
/// header, so a branch-scoped client validates the branch's view of the model.
pub async fn validate_event_model(
    client: &mut Client,
    address: &EntityAddress,
) -> Result<(pb::Id, pb::ValidateEventModelResponse)> {
    // The RPC needs a concrete version, so resolve "latest" first.
    let id = if let Some(version) = address.version {
        pb::Id {
            namespace: address.namespace.clone(),
            slug: address.slug.clone(),
            version,
        }
    } else {
        let entity = view::fetch_entity(client, address).await?;
        trogon_atlas_core::entity_id(&entity)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("stored event model has no id"))?
    };
    let resp = client
        .validate_event_model(pb::ValidateEventModelRequest {
            event_model: None,
            event_model_id: Some(id.clone()),
        })
        .await
        .context("ValidateEventModel failed")?
        .into_inner();
    Ok((id, resp))
}

/// What `ValidateProject` covers: every model in a namespace, or those a
/// Domain or Project claims. Strategic findings (BoundedContext, Domain,
/// Subdomain, Project rules) come back once on the report with no
/// `event_model_id`, so a scope with no EventModel still reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectScope {
    Namespace(String),
    Domain(EntityAddress),
    Project(EntityAddress),
}

impl ProjectScope {
    /// Scope for a Domain, parsed from `<namespace>/<slug>[@version]`.
    pub fn domain(s: &str) -> Result<Self> {
        parse_scope_address(pb::EntityKind::Domain, s).map(Self::Domain)
    }

    /// Scope for a Project, parsed from `<namespace>/<slug>[@version]`.
    pub fn project(s: &str) -> Result<Self> {
        parse_scope_address(pb::EntityKind::Project, s).map(Self::Project)
    }

    /// Scope for a namespace; the name must be a valid id component.
    pub fn namespace(s: &str) -> Result<Self> {
        trogon_atlas_core::validate_id_component(s)
            .map_err(|e| crate::failure::invalid_input(format!("namespace {e}")))?;
        Ok(Self::Namespace(s.to_string()))
    }
}

fn parse_scope_address(kind: pb::EntityKind, s: &str) -> Result<EntityAddress> {
    if s.matches('/').count() != 1 {
        crate::invalid_input!("expected <namespace>/<slug>[@version], got {s:?}");
    }
    let address = view::parse_address(&format!("{}/{s}", pb::canonical::kind_short(kind)))?;
    Ok(address)
}

async fn resolve_id(client: &mut Client, address: &EntityAddress) -> Result<pb::Id> {
    if let Some(version) = address.version {
        return Ok(pb::Id {
            namespace: address.namespace.clone(),
            slug: address.slug.clone(),
            version,
        });
    }
    let entity = view::fetch_entity(client, address).await?;
    trogon_atlas_core::entity_id(&entity)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("stored entity has no id"))
}

/// Validate every model in `scope`. Both the latest-version lookup and the
/// RPC carry the client's branch header, so a branch-scoped client validates
/// the branch's view.
pub async fn validate_project(
    client: &mut Client,
    scope: &ProjectScope,
) -> Result<pb::ValidateProjectResponse> {
    let scope = match scope {
        ProjectScope::Namespace(namespace) => {
            pb::validate_project_request::Scope::Namespace(namespace.clone())
        }
        ProjectScope::Domain(address) => {
            pb::validate_project_request::Scope::DomainId(resolve_id(client, address).await?)
        }
        ProjectScope::Project(address) => {
            pb::validate_project_request::Scope::ProjectId(resolve_id(client, address).await?)
        }
    };
    Ok(client
        .validate_project(pb::ValidateProjectRequest { scope: Some(scope) })
        .await
        .context("ValidateProject failed")?
        .into_inner())
}

/// `SKIPPED` reasons the server reports for a suite that is not part of the
/// entry point by design, so skipping it leaves nothing unexamined. Any other
/// skip reason (a suite that applied but was blocked, or a reason this client
/// does not know) counts as a gap, so a new server reason can never read as
/// a pass.
const BY_DESIGN_SKIP_REASONS: &[&str] = &[
    "write_path",
    "merge_path",
    "kind_not_applicable",
    "event_model_id_absent",
];

/// Why a validation run did not examine everything it was asked to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageGap {
    /// The server sent no coverage, so nothing proves the run was complete.
    CoverageAbsent,
    /// The server sent coverage that names no checks, so nothing it reports
    /// proves any suite ran.
    NoChecks,
    /// The snapshot the suites ran against was truncated.
    SnapshotTruncated,
    /// A suite finished `PARTIAL`, `ERROR`, an unspecified state, or was
    /// skipped for a reason that is not by design.
    Check {
        check_id: String,
        state: &'static str,
        reason: String,
    },
}

impl CoverageGap {
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::CoverageAbsent => serde_json::json!({"gap": "coverage_absent"}),
            Self::NoChecks => serde_json::json!({"gap": "no_checks"}),
            Self::SnapshotTruncated => serde_json::json!({"gap": "snapshot_truncated"}),
            Self::Check {
                check_id,
                state,
                reason,
            } => serde_json::json!({
                "gap": "check",
                "check_id": check_id,
                "state": state,
                "reason": reason,
            }),
        }
    }
}

/// The gaps in a validation run's coverage; empty means complete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completeness {
    pub gaps: Vec<CoverageGap>,
}

impl Completeness {
    #[must_use]
    pub fn of(coverage: Option<&pb::ValidationCoverage>) -> Self {
        let Some(coverage) = coverage else {
            return Self {
                gaps: vec![CoverageGap::CoverageAbsent],
            };
        };
        let mut gaps = Vec::new();
        if coverage.checks.is_empty() {
            gaps.push(CoverageGap::NoChecks);
        }
        if coverage.snapshot_truncated {
            gaps.push(CoverageGap::SnapshotTruncated);
        }
        gaps.extend(coverage.checks.iter().filter_map(check_gap));
        Self { gaps }
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
}

fn check_gap(check: &pb::ValidationCheckResult) -> Option<CoverageGap> {
    use pb::validation_check_result::State;
    let state = match State::try_from(check.state) {
        Ok(State::Completed) => return None,
        Ok(State::Skipped) if BY_DESIGN_SKIP_REASONS.contains(&check.reason.as_str()) => {
            return None
        }
        Ok(State::Skipped) => "SKIPPED",
        Ok(State::Partial) => "PARTIAL",
        Ok(State::Error) => "ERROR",
        _ => "UNSPECIFIED",
    };
    Some(CoverageGap::Check {
        check_id: check.check_id.clone(),
        state,
        reason: check.reason.clone(),
    })
}

/// What a `validate` / `validate-project` run concluded. An Error finding is
/// definitive whatever the coverage, so it outranks an incomplete run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationVerdict {
    Clean,
    Findings,
    Incomplete,
}

impl ValidationVerdict {
    #[must_use]
    pub fn of(errors: usize, completeness: &Completeness) -> Self {
        if errors > 0 {
            Self::Findings
        } else if completeness.is_complete() {
            Self::Clean
        } else {
            Self::Incomplete
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Findings => "findings",
            Self::Incomplete => "incomplete",
        }
    }
}

/// Add `coverage` (as the server sent it), `verdict` and `coverage_gaps` to
/// a validate command's JSON body, so a script reads the conclusion without
/// mapping exit codes itself.
pub fn insert_verdict(
    body: &mut serde_json::Value,
    coverage: Option<&pb::ValidationCoverage>,
    verdict: ValidationVerdict,
    completeness: &Completeness,
) {
    crate::apply::insert_coverage(body, coverage);
    if let Some(fields) = body.as_object_mut() {
        fields.insert("verdict".into(), verdict.as_str().into());
        fields.insert(
            "coverage_gaps".into(),
            completeness.gaps.iter().map(CoverageGap::to_json).collect(),
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespace_model_and_optional_version() {
        let latest = parse_model_address("shop/checkout").unwrap();
        assert_eq!(latest.kind, pb::EntityKind::EventModel);
        assert_eq!(
            (
                latest.namespace.as_str(),
                latest.slug.as_str(),
                latest.version
            ),
            ("shop", "checkout", None)
        );
        let pinned = parse_model_address("shop/checkout@3").unwrap();
        assert_eq!(pinned.version, Some(3));
    }

    #[test]
    fn rejects_addresses_that_are_not_namespace_slash_model() {
        for bad in [
            "checkout",
            "event_model/shop/checkout",
            "shop/",
            "shop/checkout@0",
        ] {
            assert!(parse_model_address(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn project_scopes_parse_namespace_slug_and_optional_version() {
        let ProjectScope::Domain(domain) = ProjectScope::domain("shop/shop").unwrap() else {
            panic!("expected a domain scope");
        };
        assert_eq!(domain.kind, pb::EntityKind::Domain);
        assert_eq!(domain.version, None);
        let ProjectScope::Project(project) = ProjectScope::project("shop/main@2").unwrap() else {
            panic!("expected a project scope");
        };
        assert_eq!(project.kind, pb::EntityKind::Project);
        assert_eq!(
            (
                project.namespace.as_str(),
                project.slug.as_str(),
                project.version
            ),
            ("shop", "main", Some(2))
        );
        assert_eq!(
            ProjectScope::namespace("shop").unwrap(),
            ProjectScope::Namespace("shop".into())
        );
    }

    #[test]
    fn project_scopes_reject_malformed_input() {
        for bad in ["shop", "domain/shop/shop", "shop/", "shop/shop@0"] {
            assert!(ProjectScope::domain(bad).is_err(), "{bad} should not parse");
            assert!(
                ProjectScope::project(bad).is_err(),
                "{bad} should not parse"
            );
        }
        assert!(ProjectScope::namespace("").is_err());
        assert!(ProjectScope::namespace("a/b").is_err());
    }

    fn check(
        check_id: &str,
        state: pb::validation_check_result::State,
        reason: &str,
    ) -> pb::ValidationCheckResult {
        pb::ValidationCheckResult {
            check_id: check_id.into(),
            state: state as i32,
            reason: reason.into(),
            subjects_examined: 1,
        }
    }

    fn coverage(checks: Vec<pb::ValidationCheckResult>, truncated: bool) -> pb::ValidationCoverage {
        pb::ValidationCoverage {
            checks,
            snapshot_truncated: truncated,
            ..Default::default()
        }
    }

    fn verdict(errors: usize, coverage: Option<&pb::ValidationCoverage>) -> ValidationVerdict {
        ValidationVerdict::of(errors, &Completeness::of(coverage))
    }

    #[test]
    fn verdict_follows_errors_first_then_coverage() {
        use pb::validation_check_result::State;
        let complete = coverage(vec![check("model", State::Completed, "")], false);
        let partial = coverage(
            vec![check("model", State::Partial, "snapshot_truncated")],
            false,
        );
        let errored = coverage(vec![check("model", State::Error, "boom")], false);
        let truncated = coverage(vec![check("model", State::Completed, "")], true);
        let by_design = coverage(
            vec![
                check("model", State::Completed, ""),
                check("strategic", State::Skipped, "event_model_id_absent"),
                check("store_hygiene", State::Skipped, "write_path"),
                check("request_shape", State::Skipped, "merge_path"),
                check("scenarios", State::Skipped, "kind_not_applicable"),
            ],
            false,
        );
        let cases = [
            (
                "clean and complete",
                0,
                Some(&complete),
                ValidationVerdict::Clean,
            ),
            ("errors", 1, Some(&complete), ValidationVerdict::Findings),
            (
                "errors and incomplete",
                2,
                Some(&partial),
                ValidationVerdict::Findings,
            ),
            ("errors and absent", 1, None, ValidationVerdict::Findings),
            (
                "partial check",
                0,
                Some(&partial),
                ValidationVerdict::Incomplete,
            ),
            (
                "errored check",
                0,
                Some(&errored),
                ValidationVerdict::Incomplete,
            ),
            (
                "truncated snapshot",
                0,
                Some(&truncated),
                ValidationVerdict::Incomplete,
            ),
            (
                "by-design skips",
                0,
                Some(&by_design),
                ValidationVerdict::Clean,
            ),
            ("absent coverage", 0, None, ValidationVerdict::Incomplete),
        ];
        for (name, errors, coverage, expected) in cases {
            assert_eq!(verdict(errors, coverage), expected, "{name}");
        }
    }

    #[test]
    fn blocked_unknown_and_unspecified_checks_are_gaps() {
        use pb::validation_check_result::State;
        for (state, reason) in [
            (State::Skipped, "blocked_by_hygiene"),
            (State::Skipped, "blocked_by_scenarios"),
            (State::Skipped, ""),
            (State::Skipped, "a_reason_this_client_does_not_know"),
            (State::Unspecified, ""),
        ] {
            let coverage = coverage(vec![check("strategic", state, reason)], false);
            assert_eq!(
                verdict(0, Some(&coverage)),
                ValidationVerdict::Incomplete,
                "{state:?} {reason}"
            );
        }
    }

    #[test]
    fn gaps_name_each_incomplete_part_in_json() {
        use pb::validation_check_result::State;
        let coverage = coverage(
            vec![
                check("model", State::Partial, "snapshot_truncated"),
                check("store_hygiene", State::Skipped, "write_path"),
            ],
            true,
        );
        let completeness = Completeness::of(Some(&coverage));
        let mut body = serde_json::json!({});
        insert_verdict(
            &mut body,
            Some(&coverage),
            ValidationVerdict::of(0, &completeness),
            &completeness,
        );
        assert_eq!(body["verdict"], "incomplete");
        assert_eq!(
            body["coverage_gaps"],
            serde_json::json!([
                {"gap": "snapshot_truncated"},
                {"gap": "check", "check_id": "model", "state": "PARTIAL", "reason": "snapshot_truncated"},
            ])
        );
        assert_eq!(body["coverage"]["snapshot_truncated"], true);

        let mut absent = serde_json::json!({});
        let completeness = Completeness::of(None);
        insert_verdict(
            &mut absent,
            None,
            ValidationVerdict::of(0, &completeness),
            &completeness,
        );
        assert_eq!(absent["verdict"], "incomplete");
        assert_eq!(
            absent["coverage_gaps"],
            serde_json::json!([{"gap": "coverage_absent"}])
        );
        assert!(absent.get("coverage").is_none());
    }

    #[test]
    fn coverage_without_checks_is_incomplete() {
        let completeness = Completeness::of(Some(&pb::ValidationCoverage::default()));
        assert_eq!(completeness.gaps, vec![CoverageGap::NoChecks]);
        assert_eq!(
            ValidationVerdict::of(0, &completeness),
            ValidationVerdict::Incomplete
        );
        assert_eq!(
            CoverageGap::NoChecks.to_json(),
            serde_json::json!({"gap": "no_checks"})
        );
    }
}
