#![allow(clippy::unwrap_used)]

use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use http_body::Body as _;
use prost::Message;
use tonic::{body::Body, Status};
use trogon_atlas_client::{
    client::{client_for_transport, Client, ConnectOptions, Transport},
    failure::{Failure, FailureCategory},
    glossary,
    precondition::StalePlan,
};
use trogon_atlas_proto as pb;

#[derive(Clone, Copy)]
enum FailurePoint {
    Discovery,
    Descriptors,
    AfterReview,
    TypeAppearsBeforeReview,
    FieldAppearsBeforeReview,
    StaleCandidate,
}

#[derive(Default)]
struct Observed {
    reviews: AtomicUsize,
    calls: Mutex<Vec<String>>,
}

#[derive(Clone, PartialEq, Message)]
struct Note {
    #[prost(string, tag = "1")]
    text: String,
}

fn term_id() -> pb::Id {
    pb::Id {
        namespace: "review".into(),
        slug: "parcel".into(),
        version: u64::MAX,
    }
}

fn descriptors() -> pb::GetTypeLibraryDescriptorSetResponse {
    use prost_types::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    };

    pb::GetTypeLibraryDescriptorSetResponse {
        file_descriptor_set: FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("review/notes/v1/note.proto".into()),
                package: Some("review.notes.v1".into()),
                syntax: Some("proto3".into()),
                message_type: vec![DescriptorProto {
                    name: Some("Note".into()),
                    field: vec![FieldDescriptorProto {
                        name: Some("text".into()),
                        number: Some(1),
                        label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
                        r#type: Some(prost_types::field_descriptor_proto::Type::String as i32),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec(),
        ..Default::default()
    }
}

fn receipt() -> pb::GetGlossaryTermResponse {
    pb::GetGlossaryTermResponse {
        entity: Some(pb::Entity {
            kind: Some(pb::entity::Kind::Term(pb::Term {
                id: Some(term_id()),
                title: "Parcel".into(),
                doc: "A parcel is a shipment.".into(),
                glossary: Some(pb::GlossaryEntry {
                    generation_revision: "controlled-transport-v1".into(),
                    audience: "internal".into(),
                    evidence: vec![pb::GlossaryEvidence {
                        document: Some(pb::DocumentRef {
                            id: Some(pb::Id {
                                slug: "source".into(),
                                ..term_id()
                            }),
                        }),
                        owner_uid: "retained-source-owner".into(),
                        content_digest: Some(pb::ContentDigest {
                            sha256: "0".repeat(64),
                        }),
                        locator: "paragraph:1".into(),
                        quote: "A parcel".into(),
                    }],
                    ..Default::default()
                }),
                metadata: vec![prost_types::Any {
                    type_url: "type.googleapis.com/review.notes.v1.Note".into(),
                    value: Note {
                        text: "retained receipt".into(),
                    }
                    .encode_to_vec(),
                }],
                ..Default::default()
            })),
            ..Default::default()
        }),
        etag: "reviewed-etag".into(),
        ..Default::default()
    }
}

fn receipt_after_change(failure: FailurePoint) -> pb::GetGlossaryTermResponse {
    let mut response = receipt();
    if matches!(failure, FailurePoint::FieldAppearsBeforeReview) {
        let Some(pb::entity::Kind::Term(term)) = response.entity.as_mut().unwrap().kind.as_mut()
        else {
            panic!("Term required")
        };
        prost::encoding::string::encode(
            2,
            &"Declared before server review".to_owned(),
            &mut term.metadata[0].value,
        );
    }
    response
}

fn reply(value: &impl Message) -> http::Response<Body> {
    let message = value.encode_to_vec();
    let mut wire = vec![0];
    wire.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
    wire.extend_from_slice(&message);
    http::Response::builder()
        .header("content-type", "application/grpc")
        .header("grpc-status", "0")
        .body(Body::new(ReplyBody(Some(wire.into()))))
        .unwrap()
}

fn connect(failure: FailurePoint, branch: Option<&str>) -> (Client, Arc<Observed>) {
    let observed = Arc::new(Observed::default());
    let state = observed.clone();
    let expected_branch = branch.map(str::to_owned);
    let transport = Transport::new(tower::service_fn(move |request: http::Request<Body>| {
        let state = state.clone();
        let branch = expected_branch.clone();
        async move {
            assert_eq!(
                request
                    .headers()
                    .get("x-trogon-atlas-branch")
                    .map(|value| value.to_str().unwrap()),
                branch.as_deref()
            );
            let method = request.uri().path().rsplit('/').next().unwrap().to_owned();
            state.calls.lock().unwrap().push(method.clone());
            let mut body = request.into_body();
            let mut wire = Vec::new();
            while let Some(frame) =
                std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
            {
                if let Ok(bytes) = frame.unwrap().into_data() {
                    wire.extend_from_slice(&bytes);
                }
            }
            let prepared_after_review = matches!(failure, FailurePoint::AfterReview)
                && state.reviews.load(Ordering::SeqCst) != 0;
            let response = match method.as_str() {
                "GetServerInfo"
                    if matches!(failure, FailurePoint::Discovery) || prepared_after_review =>
                {
                    Status::unavailable("discovery unavailable").into_http::<Body>()
                }
                "GetServerInfo" => reply(&pb::GetServerInfoResponse {
                    features: Some(pb::get_server_info_response::Features {
                        type_libraries: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                "GetTypeLibraryDescriptorSet" => {
                    assert_eq!(
                        pb::GetTypeLibraryDescriptorSetRequest::decode(&wire[5..])
                            .unwrap()
                            .namespace,
                        "review"
                    );
                    if matches!(failure, FailurePoint::Descriptors) || prepared_after_review {
                        Status::unavailable("descriptors unavailable").into_http::<Body>()
                    } else if matches!(failure, FailurePoint::TypeAppearsBeforeReview) {
                        reply(&pb::GetTypeLibraryDescriptorSetResponse::default())
                    } else {
                        reply(&descriptors())
                    }
                }
                "GetEntity" => {
                    let request = pb::GetEntityRequest::decode(&wire[5..]).unwrap();
                    assert_eq!(request.id, Some(term_id()));
                    assert_eq!(request.kind, pb::EntityKind::Term as i32);
                    reply(&pb::GetEntityResponse {
                        entity: receipt_after_change(failure).entity,
                        etag: if matches!(failure, FailurePoint::StaleCandidate) {
                            "changed-etag".into()
                        } else {
                            "draft-etag".into()
                        },
                    })
                }
                "ReviewGlossaryTerm" => {
                    let request = pb::ReviewGlossaryTermRequest::decode(&wire[5..]).unwrap();
                    assert_eq!(request.term.unwrap().id, Some(term_id()));
                    assert_eq!(request.if_match, "draft-etag");
                    state.reviews.fetch_add(1, Ordering::SeqCst);
                    reply(&receipt_after_change(failure))
                }
                other => panic!("unexpected RPC {other}"),
            };
            Ok::<_, Infallible>(response)
        }
    }));
    let client = client_for_transport(
        transport,
        None,
        &ConnectOptions {
            branch: branch.map(str::to_owned),
            ..Default::default()
        },
    )
    .unwrap();
    (client, observed)
}

fn request(preview: bool) -> pb::ReviewGlossaryTermRequest {
    pb::ReviewGlossaryTermRequest {
        term: Some(pb::TermRef {
            id: Some(term_id()),
        }),
        if_match: "draft-etag".into(),
        disposition: pb::GlossaryReviewDisposition::Accepted as i32,
        reason: "Controlled transport review".into(),
        validate_only: preview,
    }
}

#[tokio::test]
async fn preparation_failure_never_submits_a_review() {
    for point in [FailurePoint::Discovery, FailurePoint::Descriptors] {
        for branch in [None, Some("reviewer/tenant-receipt")] {
            for preview in [true, false] {
                let (mut client, observed) = connect(point, branch);
                let error = glossary::review_glossary_term(&mut client, request(preview))
                    .await
                    .unwrap_err();
                assert_eq!(observed.reviews.load(Ordering::SeqCst), 0);
                assert!(format!("{error:#}").contains("unavailable"));
            }
        }
    }
}

#[tokio::test]
async fn prepared_receipt_needs_no_discovery_after_review_succeeds() {
    for branch in [None, Some("reviewer/tenant-receipt")] {
        for preview in [true, false] {
            let (mut client, observed) = connect(FailurePoint::AfterReview, branch);
            let receipt = glossary::review_glossary_term(&mut client, request(preview))
                .await
                .unwrap();
            assert_eq!(receipt["etag"], "reviewed-etag");
            assert_eq!(
                receipt["entity"]["term"]["id"]["version"],
                u64::MAX.to_string()
            );
            assert_eq!(
                receipt["entity"]["term"]["metadata"][0]["text"],
                "retained receipt"
            );
            assert_eq!(observed.reviews.load(Ordering::SeqCst), 1);
            assert_eq!(
                *observed.calls.lock().unwrap(),
                [
                    "GetServerInfo",
                    "GetTypeLibraryDescriptorSet",
                    "GetEntity",
                    "ReviewGlossaryTerm"
                ]
            );
        }
    }
}

#[tokio::test]
async fn a_prepared_pool_must_represent_the_exact_candidate_before_review() {
    for point in [
        FailurePoint::TypeAppearsBeforeReview,
        FailurePoint::FieldAppearsBeforeReview,
    ] {
        for branch in [None, Some("reviewer/tenant-receipt")] {
            for preview in [true, false] {
                let (mut client, observed) = connect(point, branch);
                let error = glossary::review_glossary_term(&mut client, request(preview))
                    .await
                    .unwrap_err();
                assert_eq!(
                    Failure::classify(&error).category,
                    FailureCategory::Validation
                );
                assert_eq!(observed.reviews.load(Ordering::SeqCst), 0);
            }
        }
    }
}

#[tokio::test]
async fn changed_candidate_preserves_typed_stale_state_and_empty_etag_is_validation() {
    for branch in [None, Some("reviewer/tenant-receipt")] {
        for preview in [true, false] {
            let (mut client, observed) = connect(FailurePoint::StaleCandidate, branch);
            let error = glossary::review_glossary_term(&mut client, request(preview))
                .await
                .unwrap_err();
            assert_eq!(
                Failure::classify(&error).category,
                FailureCategory::StaleState
            );
            let stale = error.downcast_ref::<StalePlan>().unwrap();
            assert_eq!(
                stale.entity,
                pb::EntityKey::new(pb::EntityKind::Term, &term_id())
            );
            assert_eq!(stale.expected.to_wire(), "draft-etag");
            assert_eq!(stale.actual.to_wire(), "changed-etag");
            assert_eq!(observed.reviews.load(Ordering::SeqCst), 0);

            let (mut client, observed) = connect(FailurePoint::AfterReview, branch);
            let mut empty = request(preview);
            empty.if_match.clear();
            let error = glossary::review_glossary_term(&mut client, empty)
                .await
                .unwrap_err();
            assert_eq!(
                Failure::classify(&error).category,
                FailureCategory::Validation
            );
            assert_eq!(observed.reviews.load(Ordering::SeqCst), 0);
        }
    }
}

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
