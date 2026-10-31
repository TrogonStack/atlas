use std::collections::BTreeSet;

use tonic::Status;

use super::{pb, snapshot_digest, EventModelServiceImpl, Lens, Visibility};
use crate::backing::{self, SubjectScope, ViewLimits, VisibleNamespaces};

impl EventModelServiceImpl {
    pub(super) async fn backing_report(
        &self,
        req: pb::GetBackingReportRequest,
        branch: Option<String>,
        vis: &Visibility,
    ) -> Result<pb::GetBackingReportResponse, Status> {
        for namespace in &req.namespaces {
            self.require_visible(vis, namespace).await?;
        }
        let (rows, truncated) = self
            .snapshot_with_truncation(branch.as_deref(), vis)
            .await?;
        let digest = snapshot_digest(&rows, &BTreeSet::new(), false)?;
        let visibility = match self.lens(vis).await? {
            Lens::Everything => VisibleNamespaces::All,
            Lens::Only(ids) => {
                VisibleNamespaces::Only(ids.iter().map(|id| id.as_str().to_owned()).collect())
            }
        };
        let authorized_namespaces = match &visibility {
            VisibleNamespaces::All => Vec::new(),
            VisibleNamespaces::Only(namespaces) => namespaces.iter().cloned().collect(),
        };
        let scope = SubjectScope::new(req.namespaces);
        let report = backing::report(
            &rows,
            &scope,
            &ViewLimits {
                snapshot_truncated: truncated,
                visibility,
            },
        );
        let mut findings: Vec<pb::ValidationIssue> = if req.include_unbacked_findings {
            report
                .unbacked
                .iter()
                .map(backing::unbacked_finding)
                .collect()
        } else {
            Vec::new()
        };
        crate::validation::enrich_issues_with_catalog(&mut findings);
        Ok(pb::GetBackingReportResponse {
            snapshot_id: digest.snapshot_id,
            snapshot_truncated: truncated,
            branch: branch.unwrap_or_default(),
            visibility: Some(pb::BackingVisibility {
                restricted: !vis.is_unrestricted(),
                authorized_namespaces,
            }),
            namespaces: scope.namespaces().map(str::to_owned).collect(),
            totals: Some(report.totals),
            kinds: report.kinds,
            unresolved_declarations: report.unresolved_declarations,
            incomplete_reasons: report
                .incomplete_reasons
                .into_iter()
                .map(|reason| reason as i32)
                .collect(),
            findings,
        })
    }
}
