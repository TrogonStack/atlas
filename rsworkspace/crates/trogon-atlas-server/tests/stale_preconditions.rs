//! A caller plans a write from a read and enforces that read at apply time.
//! These tests pin the wire contract that makes that possible: lookups report
//! the etag each planned write must match, and a refused precondition comes
//! back as structured data naming the entity and both revisions.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{metadata::MetadataValue, transport::Channel, Code};
use tonic_types::StatusExt;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{conv::reason, metrics::ServerMetrics, service::EventModelServiceImpl};

async fn client() -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn event_ref(ns: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, slug)),
    }
}

fn put(entity: pb::Entity, create_only: bool, if_match: &str) -> pb::PutEntityRequest {
    put_with_validate_only(entity, create_only, if_match, false)
}

fn put_with_validate_only(
    entity: pb::Entity,
    create_only: bool,
    if_match: &str,
    validate_only: bool,
) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only,
        if_match: if_match.into(),
        validate_only,
        force: false,
    }
}

fn with_branch<T>(body: T, branch: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).expect("valid header value"),
    );
    req
}

async fn expect_put_status(
    c: &mut EventModelServiceClient<Channel>,
    req: tonic::Request<pb::PutEntityRequest>,
    code: Code,
    reason_code: &str,
    expected_etag: &str,
    actual_etag: &str,
) {
    let err = c
        .put_entity(req)
        .await
        .expect_err("put must refuse with a typed status");
    assert_eq!(err.code(), code, "{err}");
    let info = err
        .get_details_error_info()
        .expect("put refusal must carry ErrorInfo");
    assert_eq!(info.reason, reason_code, "{err}");
    assert_eq!(
        info.metadata.get("expected_etag").map(String::as_str),
        Some(expected_etag),
        "{err}"
    );
    assert_eq!(
        info.metadata.get("actual_etag").map(String::as_str),
        Some(actual_etag),
        "{err}"
    );
}

async fn get_event(
    c: &mut EventModelServiceClient<Channel>,
    ns: &str,
    slug: &str,
    branch: Option<&str>,
) -> Result<(String, String), tonic::Status> {
    let req = pb::GetEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, slug)),
    };
    let resp = match branch {
        Some(name) => c.get_entity(with_branch(req, name)).await?.into_inner(),
        None => c.get_entity(req).await?.into_inner(),
    };
    let title = match resp.entity.as_ref().unwrap().kind.as_ref().unwrap() {
        pb::entity::Kind::Event(e) => e.title.clone(),
        other => panic!("unexpected kind {other:?}"),
    };
    Ok((resp.etag, title))
}

async fn changesets_touching_ns(
    c: &mut EventModelServiceClient<Channel>,
    namespace: &str,
    branch: &str,
) -> usize {
    let mut count = 0;
    let mut page_token = String::new();
    loop {
        let resp = c
            .list_changesets(pb::ListChangesetsRequest {
                page_token: page_token.clone(),
                page_size: 100,
                branch: branch.into(),
                namespace: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        count += resp
            .changesets
            .iter()
            .filter(|cs| {
                cs.ops.iter().any(|op| {
                    op.entity
                        .as_ref()
                        .and_then(|e| e.id.as_ref())
                        .is_some_and(|id| id.namespace == namespace)
                })
            })
            .count();
        if resp.next_page_token.is_empty() {
            return count;
        }
        page_token = resp.next_page_token;
    }
}

async fn put_etag(c: &mut EventModelServiceClient<Channel>, entity: pb::Entity) -> String {
    c.put_entity(put(entity, false, ""))
        .await
        .unwrap()
        .into_inner()
        .etag
}

async fn batch_put(
    c: &mut EventModelServiceClient<Channel>,
    op: pb::PutEntityRequest,
    validate_only: bool,
) -> pb::BatchMutateResponse {
    c.batch_mutate(pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(op)),
        }],
        validate_only,
    })
    .await
    .unwrap()
    .into_inner()
}

fn expect_precondition_failure(resp: &pb::BatchMutateResponse) -> &pb::PreconditionFailure {
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32,
        "{resp:?}"
    );
    resp.precondition_failure
        .as_ref()
        .expect("a refused precondition must be reported as structured data")
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_reports_the_etag_each_planned_write_must_match() {
    let mut c = client().await;
    let etag = put_etag(&mut c, event("stale-etags", "present", "Present")).await;

    for dense in [true, false] {
        let resp = c
            .batch_get_entities(pb::BatchGetEntitiesRequest {
                keys: vec![
                    event_ref("stale-etags", "present"),
                    event_ref("stale-etags", "absent"),
                ],
                dense,
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(resp.etags.len(), resp.entities.len(), "dense={dense}");
        assert_eq!(resp.etags[0], etag, "dense={dense}");
        if dense {
            assert_eq!(resp.etags[1], "", "a placeholder carries no etag");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_only_against_a_concurrent_create_reports_both_revisions() {
    let mut c = client().await;
    let actual = put_etag(&mut c, event("stale-create", "e", "Other agent")).await;

    for validate_only in [true, false] {
        let resp = batch_put(
            &mut c,
            put(event("stale-create", "e", "Mine"), true, ""),
            validate_only,
        )
        .await;
        let failure = expect_precondition_failure(&resp);
        assert_eq!(failure.subject, Some(event_ref("stale-create", "e")));
        assert_eq!(failure.expected_etag, "", "the plan expected no entity");
        assert_eq!(failure.actual_etag, actual, "validate_only={validate_only}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn if_match_against_a_newer_revision_reports_both_revisions() {
    let mut c = client().await;
    let planned = put_etag(&mut c, event("stale-update", "e", "Base")).await;
    let actual = put_etag(&mut c, event("stale-update", "e", "Other agent")).await;

    for validate_only in [true, false] {
        let resp = batch_put(
            &mut c,
            put(event("stale-update", "e", "Mine"), false, &planned),
            validate_only,
        )
        .await;
        let failure = expect_precondition_failure(&resp);
        assert_eq!(failure.subject, Some(event_ref("stale-update", "e")));
        assert_eq!(failure.expected_etag, planned);
        assert_eq!(failure.actual_etag, actual, "validate_only={validate_only}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn if_match_against_a_deleted_entity_reports_it_absent() {
    let mut c = client().await;
    let planned = put_etag(&mut c, event("stale-delete", "e", "Base")).await;
    c.delete_entity(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id("stale-delete", "e")),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        if_match: String::new(),
    })
    .await
    .unwrap();

    for validate_only in [true, false] {
        let resp = batch_put(
            &mut c,
            put(event("stale-delete", "e", "Mine"), false, &planned),
            validate_only,
        )
        .await;
        let failure = expect_precondition_failure(&resp);
        assert_eq!(failure.expected_etag, planned);
        assert_eq!(failure.actual_etag, "", "validate_only={validate_only}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unrelated_batch_failure_is_not_reported_as_stale() {
    let mut c = client().await;
    let resp = c
        .batch_mutate(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    operation_id: String::new(),
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("stale-unrelated", "missing")),
                    mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                    if_match: String::new(),
                })),
            }],
            validate_only: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(resp.precondition_failure, None);
}

/// Single-Put dry runs must enforce the same preconditions as apply.
/// BatchMutate already does; PutEntity with `validate_only` must not report
/// success against a stale etag (see docs/explanation/stale-plans.md).

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_refuses_a_stale_if_match_like_apply() {
    let mut c = client().await;
    let ns = "stale-put-dry";
    let planned = put_etag(&mut c, event(ns, "e", "Base")).await;
    let actual = put_etag(&mut c, event(ns, "e", "Other agent")).await;
    let before_changesets = changesets_touching_ns(&mut c, ns, "").await;

    for validate_only in [true, false] {
        if validate_only {
            expect_put_status(
                &mut c,
                tonic::Request::new(put_with_validate_only(
                    event(ns, "e", "Mine"),
                    false,
                    &planned,
                    true,
                )),
                Code::Aborted,
                reason::STALE_PRECONDITION,
                &planned,
                &actual,
            )
            .await;
            let (etag, title) = get_event(&mut c, ns, "e", None).await.unwrap();
            assert_eq!(etag, actual, "dry-run refusal must not write");
            assert_eq!(title, "Other agent");
            assert_eq!(
                changesets_touching_ns(&mut c, ns, "").await,
                before_changesets,
                "dry-run refusal must not mint a changeset"
            );
        } else {
            expect_put_status(
                &mut c,
                tonic::Request::new(put_with_validate_only(
                    event(ns, "e", "Mine"),
                    false,
                    &planned,
                    false,
                )),
                Code::Aborted,
                reason::STALE_PRECONDITION,
                &planned,
                &actual,
            )
            .await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_and_apply_agree_on_malformed_if_match_actual_etag() {
    let mut c = client().await;
    let ns = "stale-put-malformed";
    let actual = put_etag(&mut c, event(ns, "e", "Base")).await;

    for validate_only in [true, false] {
        expect_put_status(
            &mut c,
            tonic::Request::new(put_with_validate_only(
                event(ns, "e", "Mine"),
                false,
                "not-a-revision",
                validate_only,
            )),
            Code::Aborted,
            reason::STALE_PRECONDITION,
            "not-a-revision",
            &actual,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_and_apply_agree_when_malformed_if_match_target_is_absent() {
    // Baseline `update` rejects a non-numeric `if_match` before the existence
    // check. Apply must still report the same NOT_FOUND wire shape dry-run
    // uses when the key is gone — not STALE_PRECONDITION with empty actual.
    let mut c = client().await;
    let ns = "stale-put-malformed-absent";

    for validate_only in [true, false] {
        expect_put_status(
            &mut c,
            tonic::Request::new(put_with_validate_only(
                event(ns, "e", "Mine"),
                false,
                "not-a-revision",
                validate_only,
            )),
            Code::NotFound,
            reason::NOT_FOUND,
            "not-a-revision",
            "",
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_refuses_create_only_against_an_existing_entity() {
    let mut c = client().await;
    let ns = "stale-put-create";
    let actual = put_etag(&mut c, event(ns, "e", "Other agent")).await;
    let before_changesets = changesets_touching_ns(&mut c, ns, "").await;

    for validate_only in [true, false] {
        expect_put_status(
            &mut c,
            tonic::Request::new(put_with_validate_only(
                event(ns, "e", "Mine"),
                true,
                "",
                validate_only,
            )),
            Code::AlreadyExists,
            reason::ALREADY_EXISTS,
            "",
            &actual,
        )
        .await;
        if validate_only {
            let (etag, title) = get_event(&mut c, ns, "e", None).await.unwrap();
            assert_eq!(etag, actual);
            assert_eq!(title, "Other agent");
            assert_eq!(
                changesets_touching_ns(&mut c, ns, "").await,
                before_changesets
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_refuses_if_match_when_the_entity_is_absent() {
    let mut c = client().await;
    let ns = "stale-put-absent";
    let planned = put_etag(&mut c, event(ns, "e", "Base")).await;
    c.delete_entity(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, "e")),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        if_match: String::new(),
    })
    .await
    .unwrap();
    let before_changesets = changesets_touching_ns(&mut c, ns, "").await;

    for validate_only in [true, false] {
        expect_put_status(
            &mut c,
            tonic::Request::new(put_with_validate_only(
                event(ns, "e", "Mine"),
                false,
                &planned,
                validate_only,
            )),
            Code::NotFound,
            reason::NOT_FOUND,
            &planned,
            "",
        )
        .await;
        if validate_only {
            let err = get_event(&mut c, ns, "e", None).await.unwrap_err();
            assert_eq!(err.code(), Code::NotFound);
            assert_eq!(
                changesets_touching_ns(&mut c, ns, "").await,
                before_changesets,
                "dry-run refusal must not mint a changeset"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_with_a_current_etag_succeeds_without_writing() {
    let mut c = client().await;
    let ns = "stale-put-ok";
    let current = put_etag(&mut c, event(ns, "e", "Base")).await;
    let before_changesets = changesets_touching_ns(&mut c, ns, "").await;

    let preview = c
        .put_entity(put_with_validate_only(
            event(ns, "e", "Preview title"),
            false,
            &current,
            true,
        ))
        .await
        .expect("current etag dry run must succeed")
        .into_inner();
    assert_eq!(preview.etag, "", "dry run must not mint an etag");
    assert!(
        preview.operation_receipt.is_none(),
        "dry run must not mint an operation receipt"
    );
    assert!(
        !preview.no_op,
        "validate_only is a preview, not a no-op write receipt"
    );

    let (etag, title) = get_event(&mut c, ns, "e", None).await.unwrap();
    assert_eq!(etag, current, "dry run must not change the store");
    assert_eq!(title, "Base", "dry run must not persist preview content");
    assert_eq!(
        changesets_touching_ns(&mut c, ns, "").await,
        before_changesets,
        "dry run must not mint a changeset"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_create_only_on_absent_target_succeeds_without_writing() {
    let mut c = client().await;
    let ns = "stale-put-create-ok";
    let before_changesets = changesets_touching_ns(&mut c, ns, "").await;

    let preview = c
        .put_entity(put_with_validate_only(
            event(ns, "e", "Would create"),
            true,
            "",
            true,
        ))
        .await
        .expect("create_only dry run on an absent target must succeed")
        .into_inner();
    assert_eq!(preview.etag, "");
    assert!(preview.operation_receipt.is_none());

    let err = get_event(&mut c, ns, "e", None).await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound, "dry run must not create");
    assert_eq!(
        changesets_touching_ns(&mut c, ns, "").await,
        before_changesets
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn single_put_dry_run_on_a_branch_refuses_stale_and_preserves_branch_state() {
    let mut c = client().await;
    let ns = "stale-put-branch";
    let branch = "stale-put-branch-b";
    c.create_branch(pb::CreateBranchRequest {
        name: branch.into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let planned = c
        .put_entity(with_branch(
            put_with_validate_only(event(ns, "e", "Base"), false, "", false),
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .etag;
    let actual = c
        .put_entity(with_branch(
            put_with_validate_only(event(ns, "e", "Other agent"), false, "", false),
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .etag;
    let before_changesets = changesets_touching_ns(&mut c, ns, branch).await;

    expect_put_status(
        &mut c,
        with_branch(
            put_with_validate_only(event(ns, "e", "Mine"), false, &planned, true),
            branch,
        ),
        Code::Aborted,
        reason::STALE_PRECONDITION,
        &planned,
        &actual,
    )
    .await;

    let (etag, title) = get_event(&mut c, ns, "e", Some(branch)).await.unwrap();
    assert_eq!(etag, actual);
    assert_eq!(title, "Other agent");
    assert_eq!(
        changesets_touching_ns(&mut c, ns, branch).await,
        before_changesets
    );

    let preview = c
        .put_entity(with_branch(
            put_with_validate_only(event(ns, "e", "Preview"), false, &actual, true),
            branch,
        ))
        .await
        .expect("current branch etag dry run must succeed")
        .into_inner();
    assert_eq!(preview.etag, "");
    assert!(preview.operation_receipt.is_none());
    let (etag, title) = get_event(&mut c, ns, "e", Some(branch)).await.unwrap();
    assert_eq!(etag, actual);
    assert_eq!(title, "Other agent");
    assert_eq!(
        changesets_touching_ns(&mut c, ns, branch).await,
        before_changesets
    );

    let baseline = get_event(&mut c, ns, "e", None).await.unwrap_err();
    assert_eq!(
        baseline.code(),
        Code::NotFound,
        "branch-only entity must stay invisible on baseline"
    );
}
