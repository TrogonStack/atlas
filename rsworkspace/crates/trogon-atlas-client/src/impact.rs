//! Breaking versus bump-only classification of an exact version's dependents.

use anyhow::{Context, Result};
use serde_json::Value;
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    compat::{self, Capability, CompatibilityError},
};

/// The answer a caller acts on: whether replacing `current` with `candidate`
/// leaves every dependent with nothing to do but bump its pinned reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactVerdict {
    BumpOnly,
    Breaking,
    /// Nothing provably breaks, but something could not be proven safe.
    Unknown,
}

impl ImpactVerdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BumpOnly => "bump_only",
            Self::Breaking => "breaking",
            Self::Unknown => "unknown",
        }
    }
}

/// Whether every page of an answer was computed from the same snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSnapshots {
    Consistent,
    /// The model moved between pages, so the pages describe different states.
    Mixed,
}

/// Every page of one `ClassifyImpact` answer.
#[derive(Debug, Clone)]
pub struct ImpactClassification {
    response: pb::ClassifyImpactResponse,
    pages: PageSnapshots,
}

impl ImpactClassification {
    /// An unrecognized verdict, or a BUMP_ONLY claim beside a reason the
    /// result is incomplete or pages from different snapshots, reads as
    /// UNKNOWN.
    #[must_use]
    pub fn verdict(&self) -> ImpactVerdict {
        match pb::DependentClassification::try_from(self.response.verdict) {
            Ok(pb::DependentClassification::Breaking) => ImpactVerdict::Breaking,
            Ok(pb::DependentClassification::BumpOnly)
                if self.response.incompleteness.is_empty()
                    && self.pages == PageSnapshots::Consistent =>
            {
                ImpactVerdict::BumpOnly
            }
            _ => ImpactVerdict::Unknown,
        }
    }

    #[must_use]
    pub fn pages(&self) -> PageSnapshots {
        self.pages
    }

    #[must_use]
    pub fn response(&self) -> &pb::ClassifyImpactResponse {
        &self.response
    }

    /// Proto3 JSON of the aggregated response, with `verdict` restated in
    /// the client's reading.
    pub fn to_json(&self) -> Result<Value> {
        let mut value = trogon_atlas_core::transcode::message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ClassifyImpactResponse",
            &self.response,
        )
        .context("encode impact classification")?;
        if let Value::Object(map) = &mut value {
            map.insert("clientVerdict".into(), self.verdict().as_str().into());
            map.insert(
                "clientSnapshotConsistent".into(),
                (self.pages == PageSnapshots::Consistent).into(),
            );
        }
        Ok(value)
    }
}

/// Classify every dependent of `current` against `candidate`, following
/// every page. `max_depth` 0 asks for the server maximum.
pub async fn classify(
    client: &mut Client,
    current: pb::EntityRef,
    candidate: pb::EntityRef,
    max_depth: u32,
) -> Result<ImpactClassification> {
    if !compat::advertises(client, Capability::ImpactClassification).await? {
        return Err(CompatibilityError::UnsupportedQuery(Capability::ImpactClassification).into());
    }
    let mut aggregated: Option<pb::ClassifyImpactResponse> = None;
    let mut pages = PageSnapshots::Consistent;
    let mut page_token = String::new();
    loop {
        let mut page = client
            .classify_impact(pb::ClassifyImpactRequest {
                current: Some(current.clone()),
                candidate: Some(candidate.clone()),
                max_depth,
                page_size: 0,
                page_token,
            })
            .await
            .context("ClassifyImpact failed")?
            .into_inner();
        page_token = std::mem::take(&mut page.next_page_token);
        match aggregated.as_mut() {
            None => aggregated = Some(page),
            Some(first) => {
                if first.snapshot_id != page.snapshot_id {
                    pages = PageSnapshots::Mixed;
                }
                first.dependents.extend(page.dependents);
            }
        }
        if page_token.is_empty() {
            break;
        }
    }
    let response = aggregated.unwrap_or_default();
    Ok(ImpactClassification { response, pages })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn reading(verdict: pb::DependentClassification, incomplete: bool) -> ImpactVerdict {
        reading_pages(verdict, incomplete, PageSnapshots::Consistent)
    }

    fn reading_pages(
        verdict: pb::DependentClassification,
        incomplete: bool,
        pages: PageSnapshots,
    ) -> ImpactVerdict {
        ImpactClassification {
            response: pb::ClassifyImpactResponse {
                verdict: verdict as i32,
                incompleteness: if incomplete {
                    vec![pb::ImpactIncompleteness::HiddenDependents as i32]
                } else {
                    Vec::new()
                },
                ..Default::default()
            },
            pages,
        }
        .verdict()
    }

    #[test]
    fn only_a_complete_bump_only_answer_reads_as_bump_only() {
        use pb::DependentClassification as C;
        assert_eq!(reading(C::BumpOnly, false), ImpactVerdict::BumpOnly);
        assert_eq!(reading(C::BumpOnly, true), ImpactVerdict::Unknown);
        assert_eq!(reading(C::Breaking, true), ImpactVerdict::Breaking);
        assert_eq!(reading(C::Unknown, false), ImpactVerdict::Unknown);
        assert_eq!(reading(C::Unspecified, false), ImpactVerdict::Unknown);
    }

    #[test]
    fn pages_from_different_snapshots_never_read_as_bump_only() {
        use pb::DependentClassification as C;
        assert_eq!(
            reading_pages(C::BumpOnly, false, PageSnapshots::Mixed),
            ImpactVerdict::Unknown
        );
        assert_eq!(
            reading_pages(C::Breaking, false, PageSnapshots::Mixed),
            ImpactVerdict::Breaking
        );
    }

    #[test]
    fn json_carries_the_client_verdict() {
        let json = ImpactClassification {
            response: pb::ClassifyImpactResponse {
                verdict: pb::DependentClassification::Breaking as i32,
                ..Default::default()
            },
            pages: PageSnapshots::Mixed,
        }
        .to_json()
        .unwrap();
        assert_eq!(json["clientVerdict"], "breaking");
        assert_eq!(json["clientSnapshotConsistent"], false);
        assert_eq!(json["verdict"], "DEPENDENT_CLASSIFICATION_BREAKING");
    }
}
