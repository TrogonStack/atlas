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
            Some(first) => merge_page(first, page, &mut pages),
        }
        if page_token.is_empty() {
            break;
        }
    }
    let response = aggregated.unwrap_or_default();
    Ok(ImpactClassification { response, pages })
}

fn merge_page(
    first: &mut pb::ClassifyImpactResponse,
    page: pb::ClassifyImpactResponse,
    pages: &mut PageSnapshots,
) {
    if first.snapshot_id != page.snapshot_id || first.verdict != page.verdict {
        *pages = PageSnapshots::Mixed;
    }
    for reason in page.incompleteness {
        if !first.incompleteness.contains(&reason) {
            first.incompleteness.push(reason);
        }
    }
    first.dependents.extend(page.dependents);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::{
        convert::Infallible,
        pin::Pin,
        sync::{Arc, Mutex},
    };

    use http_body::Body as _;
    use prost::Message;
    use tonic::body::Body;

    use super::*;
    use crate::client::{client_for_transport, ConnectOptions, Transport};

    struct ReplyBody(Option<bytes::Bytes>);

    impl http_body::Body for ReplyBody {
        type Data = bytes::Bytes;
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            std::task::Poll::Ready(self.0.take().map(|bytes| Ok(http_body::Frame::data(bytes))))
        }
    }

    fn grpc_response(message: impl Message) -> http::Response<ReplyBody> {
        let message = message.encode_to_vec();
        let mut frame = vec![0];
        frame.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
        frame.extend_from_slice(&message);
        http::Response::builder()
            .header("content-type", "application/grpc")
            .header("grpc-status", "0")
            .body(ReplyBody(Some(frame.into())))
            .unwrap()
    }

    fn entity_ref(slug: &str) -> pb::EntityRef {
        pb::EntityRef {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(pb::Id {
                namespace: "orders".into(),
                slug: slug.into(),
                version: 1,
            }),
        }
    }

    fn dependent(slug: &str) -> pb::DependentImpact {
        pb::DependentImpact {
            dependent: Some(entity_ref(slug)),
            classification: pb::DependentClassification::BumpOnly as i32,
            ..Default::default()
        }
    }

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
    fn later_pages_merge_incompleteness_and_conflicting_verdicts() {
        let mut first = pb::ClassifyImpactResponse {
            snapshot_id: "same".into(),
            verdict: pb::DependentClassification::BumpOnly as i32,
            incompleteness: vec![pb::ImpactIncompleteness::DepthLimited as i32],
            ..Default::default()
        };
        let page = pb::ClassifyImpactResponse {
            snapshot_id: "same".into(),
            verdict: pb::DependentClassification::Unknown as i32,
            incompleteness: vec![
                pb::ImpactIncompleteness::DepthLimited as i32,
                pb::ImpactIncompleteness::SnapshotTruncated as i32,
            ],
            ..Default::default()
        };
        let mut pages = PageSnapshots::Consistent;

        merge_page(&mut first, page, &mut pages);

        assert_eq!(pages, PageSnapshots::Mixed);
        assert_eq!(
            first.incompleteness,
            [
                pb::ImpactIncompleteness::DepthLimited as i32,
                pb::ImpactIncompleteness::SnapshotTruncated as i32,
            ]
        );
    }

    #[tokio::test]
    async fn classify_reads_every_page_and_reports_mixed_snapshots() {
        let tokens = Arc::new(Mutex::new(Vec::new()));
        let service_tokens = Arc::clone(&tokens);
        let transport = Transport::new(tower::service_fn(move |request: http::Request<Body>| {
            let tokens = Arc::clone(&service_tokens);
            async move {
                if request.uri().path().ends_with("/GetServerInfo") {
                    return Ok::<_, Infallible>(grpc_response(pb::GetServerInfoResponse {
                        features: Some(pb::get_server_info_response::Features {
                            impact_classification: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    }));
                }

                assert!(request.uri().path().ends_with("/ClassifyImpact"));
                let mut body = request.into_body();
                let mut wire = Vec::new();
                while let Some(frame) =
                    std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
                {
                    if let Ok(data) = frame.unwrap().into_data() {
                        wire.extend_from_slice(&data);
                    }
                }
                let request = pb::ClassifyImpactRequest::decode(&wire[5..]).unwrap();
                tokens.lock().unwrap().push(request.page_token.clone());

                let response = if request.page_token.is_empty() {
                    pb::ClassifyImpactResponse {
                        verdict: pb::DependentClassification::BumpOnly as i32,
                        dependents: vec![dependent("projection")],
                        next_page_token: "second-page".into(),
                        snapshot_id: "snapshot-a".into(),
                        ..Default::default()
                    }
                } else {
                    assert_eq!(request.page_token, "second-page");
                    pb::ClassifyImpactResponse {
                        verdict: pb::DependentClassification::BumpOnly as i32,
                        dependents: vec![dependent("dashboard")],
                        snapshot_id: "snapshot-b".into(),
                        ..Default::default()
                    }
                };
                Ok(grpc_response(response))
            }
        }));
        let mut client = client_for_transport(transport, None, &ConnectOptions::default()).unwrap();

        let result = classify(
            &mut client,
            entity_ref("current"),
            entity_ref("candidate"),
            0,
        )
        .await
        .unwrap();

        assert_eq!(tokens.lock().unwrap().as_slice(), ["", "second-page"]);
        assert_eq!(
            result
                .response()
                .dependents
                .iter()
                .map(|dependent| dependent
                    .dependent
                    .as_ref()
                    .unwrap()
                    .id
                    .as_ref()
                    .unwrap()
                    .slug
                    .as_str())
                .collect::<Vec<_>>(),
            ["projection", "dashboard"]
        );
        assert_eq!(result.pages(), PageSnapshots::Mixed);
        assert_eq!(result.verdict(), ImpactVerdict::Unknown);
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
