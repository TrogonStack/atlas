//! In-process gRPC integration tests for branch overlays (Phase 1:
//! Isolation, see docs/explanation/branching.md). Same pattern as
//! `noop_put.rs`: a real tonic server over an ephemeral JetStream store.
//!
//! Covers: get/list/batch_mutate merged-view behavior via the
//! `x-trogon-atlas-branch` header, change-feed suppression at the RPC layer,
//! branch lifecycle RPC round-trip, `delete_branch` making branch-only
//! entities unreachable, `delete_by_query` rejecting branch-scoped
//! requests, and RBAC enforcement (Writer role) for the three branch RPCs.
//!
//! Phase 2 (Review and merge) coverage lives at the bottom of this file:
//! `DiffBranch`/`MergeBranch`/`UpdateBranch`/`ResolveBranchEntry`, covering
//! clean merge, converged drop, each conflict class, update rebase,
//! resolve both kinds, change-feed emission on merge (not on branch
//! writes), dry_run no-persist, INVALID via dangling reference, and the
//! reader/writer RBAC split for the four new RPCs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    service::EventModelServiceImpl,
};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

async fn client(endpoint: &str) -> EventModelServiceClient<Channel> {
    EventModelServiceClient::connect(endpoint.to_string())
        .await
        .unwrap()
}

fn event(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn string_field(name: &str) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType {
            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
        }),
        repeated: false,
        optional: false,
        doc: String::new(),
        metadata: Vec::new(),
    }
}

fn event_with_fields(
    ns: &str,
    slug: &str,
    version: u64,
    title: &str,
    fields: Vec<pb::FieldSpec>,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity, force: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force,
    }
}

/// Attach the `x-trogon-atlas-branch` header to a request.
fn with_branch<T>(body: T, branch: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).expect("valid header value"),
    );
    req
}

async fn change_cursor(c: &mut EventModelServiceClient<Channel>) -> String {
    c.list_changes(pb::ListChangesRequest {
        since_token: String::new(),
        page_size: 1,
        scopes: vec![],
    })
    .await
    .unwrap()
    .into_inner()
    .next_token
}

async fn changes_in_ns(c: &mut EventModelServiceClient<Channel>, since: &str, ns: &str) -> usize {
    let mut count = 0;
    let mut token = since.to_string();
    loop {
        let page = c
            .list_changes(pb::ListChangesRequest {
                since_token: token.clone(),
                page_size: 100,
                scopes: vec![],
            })
            .await
            .unwrap()
            .into_inner();
        if page.events.is_empty() {
            return count;
        }
        count += page
            .events
            .iter()
            .filter(|e| {
                e.entity
                    .as_ref()
                    .and_then(|r| r.id.as_ref())
                    .is_some_and(|id| id.namespace == ns)
            })
            .count();
        if page.next_token == token {
            return count;
        }
        token = page.next_token;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_lifecycle_round_trip() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let created = c
        .create_branch(pb::CreateBranchRequest {
            name: "feature-x".into(),
            doc: "trying something".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .branch
        .expect("create_branch must return a branch");
    assert_eq!(created.name, "feature-x");
    assert_eq!(created.doc, "trying something");
    assert_eq!(created.delta_count, 0);

    let err = c
        .create_branch(pb::CreateBranchRequest {
            name: "feature-x".into(),
            doc: "again".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::AlreadyExists);

    let listed = c
        .list_branches(pb::ListBranchesRequest {})
        .await
        .unwrap()
        .into_inner()
        .branches;
    assert!(listed.iter().any(|b| b.name == "feature-x"));

    c.delete_branch(pb::DeleteBranchRequest {
        name: "feature-x".into(),
    })
    .await
    .unwrap();

    let listed_after = c
        .list_branches(pb::ListBranchesRequest {})
        .await
        .unwrap()
        .into_inner()
        .branches;
    assert!(!listed_after.iter().any(|b| b.name == "feature-x"));

    let err = c
        .delete_branch(pb::DeleteBranchRequest {
            name: "feature-x".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_branch_rejects_invalid_names() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let err = c
        .create_branch(pb::CreateBranchRequest {
            name: "meta".into(),
            doc: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    let err = c
        .create_branch(pb::CreateBranchRequest {
            name: "a/b/c".into(),
            doc: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    let err = c
        .create_branch(pb::CreateBranchRequest {
            name: String::new(),
            doc: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_entity_merged_view_via_branch_header() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "gv-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // Write only on the branch.
    c.put_entity(with_branch(
        put_req(event("gv-ns", "branch-only", 1, "Branch only"), false),
        "gv-branch",
    ))
    .await
    .unwrap();

    // Invisible on baseline get_entity (no branch header).
    let err = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("gv-ns", "branch-only", 1)),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);

    // Visible with the branch header.
    let resp = c
        .get_entity(with_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("gv-ns", "branch-only", 1)),
            },
            "gv-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    match resp.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Branch only"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_entities_merged_view_via_branch_header() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("lv-ns", "baseline", 1, "Baseline"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "lv-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("lv-ns", "branch-only", 1, "Branch only"), false),
        "lv-branch",
    ))
    .await
    .unwrap();

    let baseline_list = c
        .list_entities(pb::ListEntitiesRequest {
            kinds: vec![pb::EntityKind::Event as i32],
            namespaces: vec!["lv-ns".into()],
            page_size: 100,
            page_token: String::new(),
            latest_versions_only: false,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .entities;
    assert_eq!(
        baseline_list.len(),
        1,
        "baseline list must not see branch-only entity"
    );

    let branch_list = c
        .list_entities(with_branch(
            pb::ListEntitiesRequest {
                kinds: vec![pb::EntityKind::Event as i32],
                namespaces: vec!["lv-ns".into()],
                page_size: 100,
                page_token: String::new(),
                latest_versions_only: false,
                ..Default::default()
            },
            "lv-branch",
        ))
        .await
        .unwrap()
        .into_inner()
        .entities;
    assert_eq!(
        branch_list.len(),
        2,
        "merged branch list must see baseline + branch-only entity"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_merged_view_via_branch_header() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "bm-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let put_op = |e: pb::Entity| pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_req(e, false))),
    };
    let resp = c
        .batch_mutate(with_branch(
            pb::BatchMutateRequest {
                operation_id: String::new(),
                ops: vec![put_op(event("bm-ns", "a", 1, "A"))],
                validate_only: false,
            },
            "bm-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );

    // Invisible on baseline.
    let err = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("bm-ns", "a", 1)),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);

    // Visible on the branch.
    c.get_entity(with_branch(
        pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("bm-ns", "a", 1)),
        },
        "bm-branch",
    ))
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_writes_do_not_appear_in_change_feed() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "cf-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let cursor = change_cursor(&mut c).await;

    c.put_entity(with_branch(
        put_req(event("cf-ns", "silent", 1, "Silent"), false),
        "cf-branch",
    ))
    .await
    .unwrap();

    assert_eq!(
        changes_in_ns(&mut c, &cursor, "cf-ns").await,
        0,
        "branch writes must never appear in the change feed"
    );

    // A baseline write in the same namespace, for contrast, DOES appear.
    c.put_entity(put_req(event("cf-ns", "loud", 1, "Loud"), false))
        .await
        .unwrap();
    assert_eq!(
        changes_in_ns(&mut c, &cursor, "cf-ns").await,
        1,
        "baseline write must appear in the change feed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_branch_makes_branch_only_entities_unreachable() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "db-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("db-ns", "only-here", 1, "Only here"), false),
        "db-branch",
    ))
    .await
    .unwrap();

    c.delete_branch(pb::DeleteBranchRequest {
        name: "db-branch".into(),
    })
    .await
    .unwrap();

    let err = c
        .get_entity(with_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("db-ns", "only-here", 1)),
            },
            "db-branch",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

/// A bulk cleanup staged on a branch. The victim scan reads the merged
/// view, each delete lands as a tombstone delta, and baseline keeps the
/// entities until someone merges.
#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_is_branch_scoped() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("dbq-ns", "keep", 1, "Keep"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("dbq-ns", "drop", 1, "Drop"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "dbq-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let resp = c
        .delete_by_query(with_branch(
            pb::DeleteByQueryRequest {
                project: String::new(),
                namespace: "dbq-ns".into(),
                kind: 0,
                slug: "drop".into(),
                mode: pb::delete_entity_request::Mode::Force as i32,
                max_deletes: 10,
            },
            "dbq-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.deleted_count, 1);
    assert!(!resp.dry_run);

    // Gone from the branch's merged view.
    let err = c
        .get_entity(with_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("dbq-ns", "drop", 1)),
            },
            "dbq-branch",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);

    // Still on baseline: a branch delete is a tombstone, not a removal.
    c.get_entity(pb::GetEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(id("dbq-ns", "drop", 1)),
    })
    .await
    .expect("baseline must keep the entity until the branch merges");

    // The branch carries exactly the one tombstone.
    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "dbq-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        diff_status(&entries[0]),
        pb::branch_diff_entry::Status::Deleted
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_on_an_unknown_branch_is_not_found() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let err = c
        .delete_by_query(with_branch(
            pb::DeleteByQueryRequest {
                project: String::new(),
                namespace: "dbq-unknown".into(),
                kind: 0,
                slug: String::new(),
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                max_deletes: 1,
            },
            "no-such-branch",
        ))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::NotFound,
        "a typo in the branch name must not silently fall through to baseline"
    );
}

/// The motivating migration: mint `@2` on a branch, retarget every referrer
/// to it, review, merge. The sweep runs over the merged view and rewrites
/// land as deltas, so baseline is untouched until the merge.
#[tokio::test(flavor = "multi_thread")]
async fn retarget_references_is_branch_scoped() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("rt-br", "thing", 1, "V1"), false))
        .await
        .unwrap();
    c.put_entity(put_req(
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id("rt-br", "slice", 1)),
                title: "slice".into(),
                emitted_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id("rt-br", "thing", 1)),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        },
        false,
    ))
    .await
    .unwrap();

    c.create_branch(pb::CreateBranchRequest {
        name: "rt-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // `@2` exists only on the branch. Resolving `to` through baseline would
    // fail here, which is the whole point of staging a migration.
    c.put_entity(with_branch(
        put_req(event("rt-br", "thing", 2, "V2"), false),
        "rt-branch",
    ))
    .await
    .unwrap();

    let resp = c
        .retarget_references(with_branch(
            pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("rt-br", "thing", 1)),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("rt-br", "thing", 2)),
                }),
                dry_run: false,
                filter_kinds: vec![],
            },
            "rt-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.rewritten, 1, "results: {:?}", resp.results);
    assert_eq!(resp.failed, 0);

    // The branch sees the rewritten slice.
    let stored = c
        .get_entity(with_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id("rt-br", "slice", 1)),
            },
            "rt-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    let pb::entity::Kind::CommandSlice(slice) = stored.entity.unwrap().kind.unwrap() else {
        panic!("expected command slice");
    };
    assert_eq!(
        slice.emitted_events[0]
            .event
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        2
    );

    // Baseline still points at `@1`: a branch retarget writes deltas only.
    let stored = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(id("rt-br", "slice", 1)),
        })
        .await
        .unwrap()
        .into_inner();
    let pb::entity::Kind::CommandSlice(slice) = stored.entity.unwrap().kind.unwrap() else {
        panic!("expected command slice");
    };
    assert_eq!(
        slice.emitted_events[0]
            .event
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        1,
        "a branch retarget must not rewrite baseline referrers"
    );

    // The migration is one reviewable change set: the new version plus the
    // rewritten referrer, and merging it lands both.
    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "rt-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 2, "entries: {entries:?}");

    let merged = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "rt-branch".into(),
            dry_run: false,
            keep_branch: false,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32,
        "conflicts: {:?}, validation: {:?}",
        merged.conflicts,
        merged.validation
    );

    let stored = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(id("rt-br", "slice", 1)),
        })
        .await
        .unwrap()
        .into_inner();
    let pb::entity::Kind::CommandSlice(slice) = stored.entity.unwrap().kind.unwrap() else {
        panic!("expected command slice");
    };
    assert_eq!(
        slice.emitted_events[0]
            .event
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        2,
        "the merge must land the retarget on baseline"
    );
}

/// A referrer that exists only on baseline is still a referrer the branch
/// sees, so retargeting copies it into the branch rather than skipping it.
/// Skipping would leave the branch's own merged view pointing at the
/// version being migrated away from.
#[tokio::test(flavor = "multi_thread")]
async fn retarget_copies_a_baseline_only_referrer_into_the_branch() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("rt-cow", "thing", 1, "V1"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("rt-cow", "thing", 2, "V2"), false))
        .await
        .unwrap();
    c.put_entity(put_req(
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id("rt-cow", "slice", 1)),
                title: "slice".into(),
                emitted_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id("rt-cow", "thing", 1)),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        },
        false,
    ))
    .await
    .unwrap();

    c.create_branch(pb::CreateBranchRequest {
        name: "rt-cow-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    c.retarget_references(with_branch(
        pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("rt-cow", "thing", 1)),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("rt-cow", "thing", 2)),
            }),
            dry_run: false,
            filter_kinds: vec![],
        },
        "rt-cow-branch",
    ))
    .await
    .unwrap();

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "rt-cow-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        diff_status(&entries[0]),
        pb::branch_diff_entry::Status::Changed,
        "the branch had never touched this key, so the rewrite must copy it in"
    );
    assert!(
        entries[0].base.is_some(),
        "copy-on-write must capture the baseline entity, not just its etag"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_references_dry_run_on_a_branch_writes_nothing() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("rt-dry", "thing", 1, "V1"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("rt-dry", "thing", 2, "V2"), false))
        .await
        .unwrap();
    c.put_entity(put_req(
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id("rt-dry", "slice", 1)),
                title: "slice".into(),
                emitted_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id("rt-dry", "thing", 1)),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        },
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "rt-dry-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let resp = c
        .retarget_references(with_branch(
            pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("rt-dry", "thing", 1)),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("rt-dry", "thing", 2)),
                }),
                dry_run: true,
                filter_kinds: vec![],
            },
            "rt-dry-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.rewritten, 1,
        "dry run must still report what it would do"
    );

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "rt-dry-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert!(
        entries.is_empty(),
        "a dry run must leave the branch empty: {entries:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_references_on_an_unknown_branch_is_not_found() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("rt-nb", "thing", 1, "V1"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("rt-nb", "thing", 2, "V2"), false))
        .await
        .unwrap();

    let err = c
        .retarget_references(with_branch(
            pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("rt-nb", "thing", 1)),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("rt-nb", "thing", 2)),
                }),
                dry_run: true,
                filter_kinds: vec![],
            },
            "no-such-branch",
        ))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::NotFound,
        "a typo in the branch name must not silently rewrite baseline"
    );
}

// ---------------------------------------------------------------------------
// RBAC: the three branch lifecycle RPCs require Writer.
// ---------------------------------------------------------------------------

fn with_token<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("valid header value"),
    );
    req
}

const READER_TOKEN: &str = "branch-reader-token";
const WRITER_TOKEN: &str = "branch-writer-token";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.reader-svc]
role = "reader"
tokens = ["{READER_TOKEN}"]

[principals.writer-svc]
role = "writer"
tokens = ["{WRITER_TOKEN}"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

async fn start_auth_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc_impl = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc_service = EventModelServiceServer::new(svc_impl);

    let registry = Arc::new(make_registry());
    let auth = BearerAuth::new(registry.clone());

    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(grpc_service),
        auth,
    );

    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_token_denied_on_branch_mutations() {
    let addr = start_auth_server().await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    let err = c
        .create_branch(with_token(
            pb::CreateBranchRequest {
                name: "rbac-branch".into(),
                doc: String::new(),
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    c.list_branches(with_token(pb::ListBranchesRequest {}, READER_TOKEN))
        .await
        .expect("reader token must be allowed to list branches, which is observation");

    let err = c
        .delete_branch(with_token(
            pb::DeleteBranchRequest {
                name: "rbac-branch".into(),
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn writer_token_allowed_on_branch_rpcs() {
    let addr = start_auth_server().await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    c.create_branch(with_token(
        pb::CreateBranchRequest {
            name: "rbac-writer-branch".into(),
            doc: String::new(),
        },
        WRITER_TOKEN,
    ))
    .await
    .expect("writer token must be allowed to create a branch");

    c.list_branches(with_token(pb::ListBranchesRequest {}, WRITER_TOKEN))
        .await
        .expect("writer token must be allowed to list branches");

    c.delete_branch(with_token(
        pb::DeleteBranchRequest {
            name: "rbac-writer-branch".into(),
        },
        WRITER_TOKEN,
    ))
    .await
    .expect("writer token must be allowed to delete a branch");
}

// ===========================================================================
// Phase 2: Review and merge
// ===========================================================================

fn term(ns: &str, slug: &str, embodies: Vec<pb::EntityRef>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Term(pb::Term {
            id: Some(id(ns, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            embodied_by: embodies,
            metadata: vec![],
            supersedes: None,
        })),
    }
}

fn diff_status(entry: &pb::BranchDiffEntry) -> pb::branch_diff_entry::Status {
    pb::branch_diff_entry::Status::try_from(entry.status)
        .unwrap_or(pb::branch_diff_entry::Status::Unspecified)
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_branch_reports_added_changed_deleted() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("p2diff", "existing", 1, "Baseline"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("p2diff", "to-delete", 1, "Baseline"), false))
        .await
        .unwrap();

    c.create_branch(pb::CreateBranchRequest {
        name: "diff-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    c.put_entity(with_branch(
        put_req(event("p2diff", "new-on-branch", 1, "New"), false),
        "diff-branch",
    ))
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("p2diff", "existing", 1, "Changed"), false),
        "diff-branch",
    ))
    .await
    .unwrap();
    c.delete_entity(with_branch(
        pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2diff", "to-delete", 1)),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
        },
        "diff-branch",
    ))
    .await
    .unwrap();

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "diff-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 3);

    let by_slug = |slug: &str| -> &pb::BranchDiffEntry {
        entries
            .iter()
            .find(|e| e.r#ref.as_ref().unwrap().id.as_ref().unwrap().slug == slug)
            .unwrap_or_else(|| panic!("missing diff entry for {slug}"))
    };
    assert_eq!(
        diff_status(by_slug("new-on-branch")),
        pb::branch_diff_entry::Status::Added
    );
    assert_eq!(
        diff_status(by_slug("existing")),
        pb::branch_diff_entry::Status::Changed
    );
    assert_eq!(
        diff_status(by_slug("to-delete")),
        pb::branch_diff_entry::Status::Deleted
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_clean_merge_lands_and_deletes_branch() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "clean-merge".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("p2merge", "clean", 1, "Landed"), false),
        "clean-merge",
    ))
    .await
    .unwrap();

    let cursor = change_cursor(&mut c).await;

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "clean-merge".into(),
            dry_run: false,
            keep_branch: false,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(resp.applied_count, 1);

    // Landed on baseline.
    let baseline = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2merge", "clean", 1)),
        })
        .await
        .unwrap()
        .into_inner();
    match baseline.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Landed"),
        other => panic!("unexpected: {other:?}"),
    }

    // Merge landing IS an ordinary baseline write: it must appear in the
    // change feed (unlike branch writes themselves).
    assert_eq!(
        changes_in_ns(&mut c, &cursor, "p2merge").await,
        1,
        "merge landing must appear in the change feed"
    );

    // Default keep_branch=false deletes the branch.
    let listed = c
        .list_branches(pb::ListBranchesRequest {})
        .await
        .unwrap()
        .into_inner()
        .branches;
    assert!(!listed.iter().any(|b| b.name == "clean-merge"));
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_keep_branch_true_retains_empty_branch() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "keep-me".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("p2merge", "keep", 1, "Landed"), false),
        "keep-me",
    ))
    .await
    .unwrap();

    c.merge_branch(pb::MergeBranchRequest {
        operation_id: String::new(),
        name: "keep-me".into(),
        dry_run: false,
        keep_branch: true,
        auto_merge: false,
    })
    .await
    .unwrap();

    let listed = c
        .list_branches(pb::ListBranchesRequest {})
        .await
        .unwrap()
        .into_inner()
        .branches;
    let branch = listed
        .iter()
        .find(|b| b.name == "keep-me")
        .expect("branch must be kept");
    assert_eq!(branch.delta_count, 0, "landed deltas must be purged");
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_converged_entry_drops_silently() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("p2conv", "same", 1, "Same value"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "converge-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // Baseline moves to a value semantically equal to what the branch will
    // write (title equal): this is CONVERGED once diffed, not a conflict.
    // First set up the branch overlay identical to the current baseline
    // plus a second baseline write so `base` != current baseline, but the
    // branch's `ours` matches the NEW baseline exactly.
    c.put_entity(with_branch(
        put_req(event("p2conv", "same", 1, "Converged value"), false),
        "converge-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(event("p2conv", "same", 1, "Converged value"), true))
        .await
        .unwrap();

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "converge-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        diff_status(&entries[0]),
        pb::branch_diff_entry::Status::Converged
    );

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "converge-branch".into(),
            dry_run: false,
            keep_branch: false,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(resp.applied_count, 0, "converged entries land nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_conflict_edit_edit_blocks_everything() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("p2cee", "a", 1, "Base"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("p2cee", "b", 1, "Unrelated"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "cee-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // Non-conflicting change alongside the conflicting one.
    c.put_entity(with_branch(
        put_req(event("p2cee", "b", 1, "Branch edit, no conflict"), false),
        "cee-branch",
    ))
    .await
    .unwrap();
    // Conflicting edit: branch changes "a", baseline also moves "a".
    c.put_entity(with_branch(
        put_req(event("p2cee", "a", 1, "Branch edit"), false),
        "cee-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(event("p2cee", "a", 1, "Baseline moved"), true))
        .await
        .unwrap();

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "cee-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(resp.conflicts.len(), 1);
    assert_eq!(
        diff_status(&resp.conflicts[0]),
        pb::branch_diff_entry::Status::ConflictEditEdit
    );
    assert_ne!(
        resp.conflicts[0].conflict_field_paths,
        [] as [std::string::String; 0]
    );

    // All-or-nothing: the non-conflicting "b" change was NOT landed either.
    let baseline_b = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2cee", "b", 1)),
        })
        .await
        .unwrap()
        .into_inner();
    match baseline_b.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Unrelated"),
        other => panic!("unexpected: {other:?}"),
    }
}

/// `conflict_field_paths`'s JSON-diff walk must recurse into a repeated
/// field (a JSON array in the canonical transcode), not just plain object
/// leaves: build a CONFLICT_EDIT_EDIT where the two sides' `fields` lists
/// differ in length and content, so `walk_json_diff`'s array arm produces
/// per-index paths like `fields[1].name`.
#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_conflict_edit_edit_reports_array_field_paths() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(
        event_with_fields("p2ceearr", "a", 1, "Base", vec![string_field("one")]),
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "cee-arr-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    c.put_entity(with_branch(
        put_req(
            event_with_fields(
                "p2ceearr",
                "a",
                1,
                "Base",
                vec![string_field("one"), string_field("two")],
            ),
            false,
        ),
        "cee-arr-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event_with_fields(
            "p2ceearr",
            "a",
            1,
            "Base",
            vec![string_field("one"), string_field("three")],
        ),
        true,
    ))
    .await
    .unwrap();

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "cee-arr-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(resp.conflicts.len(), 1);
    assert_eq!(
        diff_status(&resp.conflicts[0]),
        pb::branch_diff_entry::Status::ConflictEditEdit
    );
    assert!(
        resp.conflicts[0]
            .conflict_field_paths
            .iter()
            .any(|p| p.contains("fields[1]")),
        "expected an array-indexed conflict path, got {:?}",
        resp.conflicts[0].conflict_field_paths
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_conflict_edit_delete_and_delete_edit() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(
        event("p2ced", "branch-edit-baseline-delete", 1, "Base"),
        false,
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event("p2ced", "branch-delete-baseline-edit", 1, "Base"),
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "ced-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // Branch edits, baseline deletes since `base` -> CONFLICT_DELETE_EDIT
    // (baseline deleted the key while the branch has a live edit).
    c.put_entity(with_branch(
        put_req(
            event("p2ced", "branch-edit-baseline-delete", 1, "Branch edit"),
            false,
        ),
        "ced-branch",
    ))
    .await
    .unwrap();
    c.delete_entity(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id("p2ced", "branch-edit-baseline-delete", 1)),
        if_match: String::new(),
        mode: pb::delete_entity_request::Mode::Force as i32,
    })
    .await
    .unwrap();

    // Branch deletes (tombstone), baseline edits since `base` ->
    // CONFLICT_EDIT_DELETE (baseline moved while the branch tombstones the
    // key).
    c.delete_entity(with_branch(
        pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2ced", "branch-delete-baseline-edit", 1)),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
        },
        "ced-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event("p2ced", "branch-delete-baseline-edit", 1, "Baseline moved"),
        true,
    ))
    .await
    .unwrap();

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "ced-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 2);
    let by_slug = |slug: &str| -> &pb::BranchDiffEntry {
        entries
            .iter()
            .find(|e| e.r#ref.as_ref().unwrap().id.as_ref().unwrap().slug == slug)
            .unwrap_or_else(|| panic!("missing diff entry for {slug}"))
    };
    assert_eq!(
        diff_status(by_slug("branch-edit-baseline-delete")),
        pb::branch_diff_entry::Status::ConflictDeleteEdit
    );
    assert_eq!(
        diff_status(by_slug("branch-delete-baseline-edit")),
        pb::branch_diff_entry::Status::ConflictEditDelete
    );

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "ced-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(resp.conflicts.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_dry_run_validates_without_persisting() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "dry-run-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("p2dry", "would-land", 1, "Would land"), false),
        "dry-run-branch",
    ))
    .await
    .unwrap();

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "dry-run-branch".into(),
            dry_run: true,
            keep_branch: false,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Validated as i32
    );
    assert_eq!(resp.applied_count, 1);

    // Nothing persisted: baseline still lacks the entity...
    let err = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2dry", "would-land", 1)),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);

    // ...and the branch itself still exists with its delta intact.
    let listed = c
        .list_branches(pb::ListBranchesRequest {})
        .await
        .unwrap()
        .into_inner()
        .branches;
    let branch = listed
        .iter()
        .find(|b| b.name == "dry-run-branch")
        .expect("dry_run must not delete the branch");
    assert_eq!(branch.delta_count, 1);
}

/// The dry-run overlay path (`req.dry_run` branch inside `merge_branch`)
/// must build a `MutationOp::Delete` for landable entries whose diff status
/// is `Deleted`, not just `MutationOp::Put` for adds/changes: validate a
/// dry-run merge that lands a branch-scoped delete of a baseline entity.
#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_dry_run_validates_a_pending_delete() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(
        event("p2dry-del", "will-be-deleted", 1, "Baseline"),
        false,
    ))
    .await
    .unwrap();

    c.create_branch(pb::CreateBranchRequest {
        name: "dry-run-delete-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.delete_entity(with_branch(
        pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2dry-del", "will-be-deleted", 1)),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
        },
        "dry-run-delete-branch",
    ))
    .await
    .unwrap();

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "dry-run-delete-branch".into(),
            dry_run: true,
            keep_branch: false,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Validated as i32
    );
    assert_eq!(resp.applied_count, 1);

    // Nothing persisted: baseline entity must still exist untouched.
    let baseline = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2dry-del", "will-be-deleted", 1)),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(baseline.entity.is_some());

    // Branch itself must still exist with its delta intact.
    let listed = c
        .list_branches(pb::ListBranchesRequest {})
        .await
        .unwrap()
        .into_inner()
        .branches;
    let branch = listed
        .iter()
        .find(|b| b.name == "dry-run-delete-branch")
        .expect("dry_run must not delete the branch");
    assert_eq!(branch.delta_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_branch_invalid_via_dangling_reference() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "invalid-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // A Term embodying an Event that does not exist anywhere (baseline or
    // branch): TERM_DANGLING_ENTITY, Error severity, in the post-merge
    // world validator.
    let dangling_term = term(
        "p2invalid",
        "ghost-term",
        vec![pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2invalid", "does-not-exist", 1)),
        }],
    );
    c.put_entity(with_branch(
        pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(dangling_term),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: true,
        },
        "invalid-branch",
    ))
    .await
    .unwrap();

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "invalid-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Invalid as i32
    );
    assert!(resp
        .validation
        .iter()
        .any(|i| i.code == "TERM_DANGLING_ENTITY"
            && i.severity == pb::validation_issue::Severity::Error as i32));

    // Nothing persisted.
    let err = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Term as i32,
            id: Some(id("p2invalid", "ghost-term", 1)),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn update_branch_rebases_non_conflicting_and_surfaces_conflicts() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("p2upd", "converges", 1, "V1"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("p2upd", "conflict", 1, "V1"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "update-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // Branch edits both keys.
    c.put_entity(with_branch(
        put_req(event("p2upd", "converges", 1, "Same in the end"), false),
        "update-branch",
    ))
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("p2upd", "conflict", 1, "Branch edit"), false),
        "update-branch",
    ))
    .await
    .unwrap();

    // Baseline moves for both keys after the branch touched them: for
    // "converges" the new baseline content is semantically identical to
    // the branch's own edit (CONVERGED, collapses on update); for
    // "conflict" the new baseline content genuinely diverges from the
    // branch's edit (CONFLICT_EDIT_EDIT, left untouched by update).
    c.put_entity(put_req(
        event("p2upd", "converges", 1, "Same in the end"),
        true,
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event("p2upd", "conflict", 1, "Baseline also moved"),
        true,
    ))
    .await
    .unwrap();

    let resp = c
        .update_branch(pb::UpdateBranchRequest {
            name: "update-branch".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.rebased_count, 1,
        "only the converged entry collapses/rebases"
    );
    assert_eq!(resp.conflicts.len(), 1);
    assert_eq!(
        diff_status(&resp.conflicts[0]),
        pb::branch_diff_entry::Status::ConflictEditEdit
    );

    // A merge attempt right after update must still see exactly one
    // conflict, and the converged entry must be gone entirely (not
    // reappear as ADDED/CHANGED/anything else) since its delta was purged.
    let merge_resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "update-branch".into(),
            dry_run: true,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merge_resp.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(merge_resp.conflicts.len(), 1);

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "update-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(
        entries.len(),
        1,
        "the converged/rebased delta must be purged from the branch"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_branch_entry_take_theirs_and_keep_ours() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("p2res", "theirs", 1, "Base"), false))
        .await
        .unwrap();
    c.put_entity(put_req(event("p2res", "ours", 1, "Base"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "resolve-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    for slug in ["theirs", "ours"] {
        c.put_entity(with_branch(
            put_req(event("p2res", slug, 1, "Branch edit"), false),
            "resolve-branch",
        ))
        .await
        .unwrap();
    }
    c.put_entity(put_req(event("p2res", "theirs", 1, "Baseline wins"), true))
        .await
        .unwrap();
    c.put_entity(put_req(event("p2res", "ours", 1, "Baseline moved"), true))
        .await
        .unwrap();

    c.resolve_branch_entry(pb::ResolveBranchEntryRequest {
        operation_id: String::new(),
        name: "resolve-branch".into(),
        r#ref: Some(pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2res", "theirs", 1)),
        }),
        resolution: pb::resolve_branch_entry_request::Resolution::TakeTheirs as i32,
        expected_state: None,
    })
    .await
    .unwrap();
    c.resolve_branch_entry(pb::ResolveBranchEntryRequest {
        operation_id: String::new(),
        name: "resolve-branch".into(),
        r#ref: Some(pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("p2res", "ours", 1)),
        }),
        resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
        expected_state: None,
    })
    .await
    .unwrap();

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "resolve-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    let by_slug = |slug: &str| -> &pb::BranchDiffEntry {
        entries
            .iter()
            .find(|e| e.r#ref.as_ref().unwrap().id.as_ref().unwrap().slug == slug)
            .unwrap_or_else(|| panic!("missing diff entry for {slug}"))
    };
    // take_theirs adopts baseline as both `base` and `ours`: no longer a
    // conflict (base/ours/theirs are all identical -- CHANGED, since the
    // freshly-rebased `base` now matches the still-unmoved baseline
    // exactly, which is a no-op landing).
    assert_eq!(
        diff_status(by_slug("theirs")),
        pb::branch_diff_entry::Status::Changed
    );
    // keep_ours rebased base forward: still a real, resolvable change, not
    // a conflict, and the branch's own edit survived.
    assert_ne!(
        diff_status(by_slug("ours")),
        pb::branch_diff_entry::Status::ConflictEditEdit
    );
    match by_slug("ours").ours.as_ref().unwrap().kind.as_ref() {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Branch edit"),
        other => panic!("unexpected: {other:?}"),
    }

    // Both must now merge cleanly.
    let merge_resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "resolve-branch".into(),
            dry_run: false,
            keep_branch: false,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merge_resp.status,
        pb::merge_branch_response::Status::Applied as i32
    );
}

// ---------------------------------------------------------------------------
// RBAC: DiffBranch is Reader; Merge/Update/ResolveBranchEntry are Writer.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn reader_token_allowed_diff_denied_mutations() {
    let addr = start_auth_server().await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    c.create_branch(with_token(
        pb::CreateBranchRequest {
            name: "p2rbac-branch".into(),
            doc: String::new(),
        },
        WRITER_TOKEN,
    ))
    .await
    .unwrap();

    // Reader IS allowed to diff.
    c.diff_branch(with_token(
        pb::DiffBranchRequest {
            name: "p2rbac-branch".into(),
        },
        READER_TOKEN,
    ))
    .await
    .expect("reader token must be allowed to diff a branch");

    // Reader is NOT allowed to merge/update/resolve.
    let err = c
        .merge_branch(with_token(
            pb::MergeBranchRequest {
                operation_id: String::new(),
                name: "p2rbac-branch".into(),
                dry_run: true,
                keep_branch: true,
                auto_merge: false,
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    let err = c
        .update_branch(with_token(
            pb::UpdateBranchRequest {
                name: "p2rbac-branch".into(),
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    let err = c
        .resolve_branch_entry(with_token(
            pb::ResolveBranchEntryRequest {
                operation_id: String::new(),
                name: "p2rbac-branch".into(),
                r#ref: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("p2rbac", "x", 1)),
                }),
                resolution: pb::resolve_branch_entry_request::Resolution::TakeTheirs as i32,
                expected_state: None,
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn writer_token_allowed_on_phase2_branch_rpcs() {
    let addr = start_auth_server().await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    c.create_branch(with_token(
        pb::CreateBranchRequest {
            name: "p2rbac-writer".into(),
            doc: String::new(),
        },
        WRITER_TOKEN,
    ))
    .await
    .unwrap();

    c.diff_branch(with_token(
        pb::DiffBranchRequest {
            name: "p2rbac-writer".into(),
        },
        WRITER_TOKEN,
    ))
    .await
    .expect("writer token must be allowed to diff");

    c.update_branch(with_token(
        pb::UpdateBranchRequest {
            name: "p2rbac-writer".into(),
        },
        WRITER_TOKEN,
    ))
    .await
    .expect("writer token must be allowed to update");

    c.merge_branch(with_token(
        pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "p2rbac-writer".into(),
            dry_run: true,
            keep_branch: true,
            auto_merge: false,
        },
        WRITER_TOKEN,
    ))
    .await
    .expect("writer token must be allowed to merge");
}

/// Docs (`docs/explanation/branching.md`) claim Phase 1 Isolation shipped
/// with "branch context on every RPC" and "reads resolve branch-first".
/// `GetOutgoingReferences` ignores `x-trogon-atlas-branch` and always loads
/// from baseline (`store.get(..., None)`), so a branch-only entity is
/// invisible even when the client sends the branch header.
#[tokio::test(flavor = "multi_thread")]
async fn get_outgoing_references_must_honor_branch_header() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "outref-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // Branch-only event with an outbound schema-less field shape is enough:
    // GetOutgoingReferences must resolve the entity under the branch header.
    c.put_entity(with_branch(
        put_req(event("outref-ns", "branch-only", 1, "Branch only"), false),
        "outref-branch",
    ))
    .await
    .unwrap();

    // Control: GetEntity with the branch header sees it.
    c.get_entity(with_branch(
        pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("outref-ns", "branch-only", 1)),
        },
        "outref-branch",
    ))
    .await
    .expect("GetEntity honors branch header");

    let result = c
        .get_outgoing_references(with_branch(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("outref-ns", "branch-only", 1)),
                filter_kinds: vec![],
                page_size: 50,
                page_token: String::new(),
            },
            "outref-branch",
        ))
        .await;

    assert!(
        result.is_ok(),
        "GetOutgoingReferences must honor x-trogon-atlas-branch and resolve \
         branch-only entities; got {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// Branch-aware SearchEntities / ExtractSubgraph / DiffEntities /
// GetSupersessionChain (post-57f0ed28 edge cases).
// ---------------------------------------------------------------------------

fn event_superseding(ns: &str, slug: &str, version: u64, title: &str, prev: u64) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: title.into(),
            supersedes: Some(id(ns, slug, prev)),
            ..Default::default()
        })),
    }
}

/// Branch writes are not indexed in the live Tantivy index. SearchEntities
/// with `x-trogon-atlas-branch` must build a one-shot index over the merged
/// snapshot; otherwise branch-only titles return a wrong-empty hit list.
#[tokio::test(flavor = "multi_thread")]
async fn search_entities_must_find_branch_only_via_oneshot_index() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "search-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let unique = "BranchOnlySearchTokenZxq9";
    c.put_entity(with_branch(
        put_req(event("search-ns", "branch-only", 1, unique), false),
        "search-branch",
    ))
    .await
    .unwrap();

    let baseline = c
        .search_entities(pb::SearchEntitiesRequest {
            query: unique.into(),
            kinds: vec![],
            namespaces: vec!["search-ns".into()],
            limit: 10,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        baseline.results.is_empty(),
        "baseline Tantivy index must not see branch-only writes"
    );

    let branched = c
        .search_entities(with_branch(
            pb::SearchEntitiesRequest {
                query: unique.into(),
                kinds: vec![],
                namespaces: vec!["search-ns".into()],
                limit: 10,
            },
            "search-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        branched.results.len(),
        1,
        "branch SearchEntities must hit the one-shot merged index; got {:?}",
        branched.results
    );
}

// ---------------------------------------------------------------------------
// Branch search index cache (G15)
// ---------------------------------------------------------------------------

async fn branch_search(
    c: &mut EventModelServiceClient<Channel>,
    branch: &str,
    query: &str,
    namespace: &str,
) -> Vec<pb::SearchResult> {
    c.search_entities(with_branch(
        pb::SearchEntitiesRequest {
            query: query.into(),
            kinds: vec![],
            namespaces: vec![namespace.into()],
            limit: 10,
        },
        branch,
    ))
    .await
    .unwrap()
    .into_inner()
    .results
}

/// The cache is keyed by a hash of the merged view, so repeating a query
/// against an unchanged branch must keep answering identically. A cache that
/// returned a different answer on the second call would be worse than no
/// cache at all.
#[tokio::test(flavor = "multi_thread")]
async fn repeating_a_branch_search_is_stable() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "cache-stable".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let unique = "CacheStableTokenQw3";
    c.put_entity(with_branch(
        put_req(event("cache-ns", "only", 1, unique), false),
        "cache-stable",
    ))
    .await
    .unwrap();

    for attempt in 0..3 {
        let results = branch_search(&mut c, "cache-stable", unique, "cache-ns").await;
        assert_eq!(results.len(), 1, "attempt {attempt} disagreed: {results:?}");
    }
}

/// The freshness case the cache has to get right: branch writes deliberately
/// bypass every baseline invalidation path, so nothing tells the cache its
/// entry is stale. Only the view hash can, and this is what proves it does.
#[tokio::test(flavor = "multi_thread")]
async fn a_branch_write_is_visible_to_the_next_branch_search() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "cache-fresh".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let first = "CacheFreshTokenOneAa1";
    let second = "CacheFreshTokenTwoBb2";
    c.put_entity(with_branch(
        put_req(event("fresh-ns", "first", 1, first), false),
        "cache-fresh",
    ))
    .await
    .unwrap();
    // Populate the cache for this view.
    assert_eq!(
        branch_search(&mut c, "cache-fresh", first, "fresh-ns")
            .await
            .len(),
        1
    );

    c.put_entity(with_branch(
        put_req(event("fresh-ns", "second", 1, second), false),
        "cache-fresh",
    ))
    .await
    .unwrap();

    assert_eq!(
        branch_search(&mut c, "cache-fresh", second, "fresh-ns")
            .await
            .len(),
        1,
        "an entity written after the index was cached must still be findable"
    );
    assert_eq!(
        branch_search(&mut c, "cache-fresh", first, "fresh-ns")
            .await
            .len(),
        1,
        "rebuilding must not drop what the previous index already held"
    );
}

/// A branch view is baseline plus deltas, so a baseline write moves it too.
/// Keying on the branch name alone would serve a stale index here.
#[tokio::test(flavor = "multi_thread")]
async fn a_baseline_write_is_visible_to_the_next_branch_search() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "cache-baseline".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let unique = "CacheBaselineTokenCc3";
    // Populate the cache while the branch view holds nothing.
    assert_eq!(
        branch_search(&mut c, "cache-baseline", unique, "base-ns").await,
        [] as [trogon_atlas_proto::SearchResult; 0]
    );

    c.put_entity(put_req(event("base-ns", "landed", 1, unique), false))
        .await
        .unwrap();

    assert_eq!(
        branch_search(&mut c, "cache-baseline", unique, "base-ns")
            .await
            .len(),
        1,
        "a branch search must see baseline writes made after its index was cached"
    );
}

/// Two branches are two views. Caching one must not answer for the other.
#[tokio::test(flavor = "multi_thread")]
async fn two_branches_do_not_share_a_cached_index() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    for name in ["cache-left", "cache-right"] {
        c.create_branch(pb::CreateBranchRequest {
            name: name.into(),
            doc: String::new(),
        })
        .await
        .unwrap();
    }

    let left = "CacheLeftTokenDd4";
    let right = "CacheRightTokenEe5";
    c.put_entity(with_branch(
        put_req(event("two-ns", "left", 1, left), false),
        "cache-left",
    ))
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("two-ns", "right", 1, right), false),
        "cache-right",
    ))
    .await
    .unwrap();

    assert_eq!(
        branch_search(&mut c, "cache-left", left, "two-ns")
            .await
            .len(),
        1
    );
    assert_eq!(
        branch_search(&mut c, "cache-right", right, "two-ns")
            .await
            .len(),
        1
    );
    assert!(
        branch_search(&mut c, "cache-left", right, "two-ns")
            .await
            .is_empty(),
        "the left branch must not see the right branch's write"
    );
    assert!(
        branch_search(&mut c, "cache-right", left, "two-ns")
            .await
            .is_empty(),
        "the right branch must not see the left branch's write"
    );
}

/// Deleting a branch and recreating the name leaves a cache entry behind
/// that describes deltas the new branch does not have.
#[tokio::test(flavor = "multi_thread")]
async fn recreating_a_branch_name_does_not_resurrect_its_deltas() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let unique = "CacheRecreatedTokenFf6";
    c.create_branch(pb::CreateBranchRequest {
        name: "cache-recycled".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("recycle-ns", "gone", 1, unique), false),
        "cache-recycled",
    ))
    .await
    .unwrap();
    assert_eq!(
        branch_search(&mut c, "cache-recycled", unique, "recycle-ns")
            .await
            .len(),
        1
    );

    c.delete_branch(pb::DeleteBranchRequest {
        name: "cache-recycled".into(),
    })
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "cache-recycled".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    assert!(
        branch_search(&mut c, "cache-recycled", unique, "recycle-ns")
            .await
            .is_empty(),
        "a recreated branch must not inherit the deleted branch's cached index"
    );
}

/// Overlaying a baseline entity's title on a branch must make the new title
/// searchable under the branch header and leave the baseline title on the
/// live index (branch writes never call search_apply_put).
#[tokio::test(flavor = "multi_thread")]
async fn search_entities_branch_overlay_title_not_baseline_index() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let base_title = "BaselineSearchTokenA1";
    let branch_title = "OverlaySearchTokenB2";
    c.put_entity(put_req(event("ov-ns", "shared", 1, base_title), false))
        .await
        .unwrap();
    // Warm the live index so the baseline title is definitely searchable.
    let warm = c
        .search_entities(pb::SearchEntitiesRequest {
            query: base_title.into(),
            kinds: vec![],
            namespaces: vec!["ov-ns".into()],
            limit: 10,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(warm.results.len(), 1, "baseline warm search");

    c.create_branch(pb::CreateBranchRequest {
        name: "overlay-search".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("ov-ns", "shared", 1, branch_title), false),
        "overlay-search",
    ))
    .await
    .unwrap();

    let baseline_still = c
        .search_entities(pb::SearchEntitiesRequest {
            query: base_title.into(),
            kinds: vec![],
            namespaces: vec!["ov-ns".into()],
            limit: 10,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        baseline_still.results.len(),
        1,
        "baseline index must keep the pre-overlay title"
    );

    let branch_hits = c
        .search_entities(with_branch(
            pb::SearchEntitiesRequest {
                query: branch_title.into(),
                kinds: vec![],
                namespaces: vec!["ov-ns".into()],
                limit: 10,
            },
            "overlay-search",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        branch_hits.results.len(),
        1,
        "branch one-shot search must see the overlay title"
    );

    let branch_old = c
        .search_entities(with_branch(
            pb::SearchEntitiesRequest {
                query: base_title.into(),
                kinds: vec![],
                namespaces: vec!["ov-ns".into()],
                limit: 10,
            },
            "overlay-search",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        branch_old.results.is_empty(),
        "merged branch view replaced the title; old title must not hit"
    );
}

/// ExtractSubgraph over a branch-only root must return that entity, not an
/// empty closure (snapshot_for, not baseline snapshot).
#[tokio::test(flavor = "multi_thread")]
async fn extract_subgraph_must_honor_branch_header() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "subgraph-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("sg-ns", "root-only", 1, "Root"), false),
        "subgraph-branch",
    ))
    .await
    .unwrap();

    let baseline = c
        .extract_subgraph(pb::ExtractSubgraphRequest {
            scope: Some(pb::AnalysisScope {
                scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("sg-ns", "root-only", 1)),
                })),
            }),
            include_cross_model: false,
            new_event_model_id: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        baseline.entities.is_empty(),
        "baseline ExtractSubgraph must not see branch-only roots"
    );

    let branched = c
        .extract_subgraph(with_branch(
            pb::ExtractSubgraphRequest {
                scope: Some(pb::AnalysisScope {
                    scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                        kind: pb::EntityKind::Event as i32,
                        id: Some(id("sg-ns", "root-only", 1)),
                    })),
                }),
                include_cross_model: false,
                new_event_model_id: None,
            },
            "subgraph-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        !branched.entities.is_empty(),
        "ExtractSubgraph must honor x-trogon-atlas-branch for branch-only roots"
    );
}

/// DiffEntities must resolve both sides through the branch merged view so an
/// overlay title change is visible when comparing two versions on the branch.
#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_must_honor_branch_overlay() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("diff-ns", "e", 1, "V1"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "diff-ent-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    // v2 only on the branch, with a distinct title.
    c.put_entity(with_branch(
        put_req(event("diff-ns", "e", 2, "BranchV2"), false),
        "diff-ent-branch",
    ))
    .await
    .unwrap();

    let err = c
        .diff_entities(pb::DiffEntitiesRequest {
            a: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("diff-ns", "e", 1)),
            }),
            b: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("diff-ns", "e", 2)),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::NotFound,
        "baseline DiffEntities must not see branch-only v2"
    );

    let ops = c
        .diff_entities(with_branch(
            pb::DiffEntitiesRequest {
                a: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("diff-ns", "e", 1)),
                }),
                b: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("diff-ns", "e", 2)),
                }),
            },
            "diff-ent-branch",
        ))
        .await
        .unwrap()
        .into_inner()
        .ops;
    assert!(
        ops.iter()
            .any(|op| op.path == "title" && op.after == "BranchV2"),
        "branch DiffEntities must compare merged views; got {ops:?}"
    );
}

/// A superseding version that exists only on the branch must appear in
/// GetSupersessionChain(descendants) under the branch header.
#[tokio::test(flavor = "multi_thread")]
async fn get_supersession_chain_must_see_branch_only_descendant() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("sup-ns", "order", 1, "V1"), false))
        .await
        .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "sup-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event_superseding("sup-ns", "order", 2, "V2", 1), false),
        "sup-branch",
    ))
    .await
    .unwrap();

    let baseline = c
        .get_supersession_chain(pb::GetSupersessionChainRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("sup-ns", "order", 1)),
            direction: pb::get_supersession_chain_request::Direction::Descendants as i32,
            page_size: 50,
            page_token: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        baseline.chain.len(),
        1,
        "baseline descendants must be just v1; got {:?}",
        baseline.chain
    );

    let branched = c
        .get_supersession_chain(with_branch(
            pb::GetSupersessionChainRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("sup-ns", "order", 1)),
                direction: pb::get_supersession_chain_request::Direction::Descendants as i32,
                page_size: 50,
                page_token: String::new(),
            },
            "sup-branch",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        branched.chain.len(),
        2,
        "branch GetSupersessionChain must include branch-only v2; got {:?}",
        branched.chain
    );
}

/// `DeleteEntity` FailIfReferenced must scan referrers in the same merged
/// view the client is editing. After `reverse_index_for` landed for reads,
/// deletes still used the baseline-only index, so a branch-only referrer
/// could not block deleting its target on that branch.
#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_on_branch_must_see_branch_referrers() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "delref-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    c.put_entity(with_branch(
        put_req(event("delref-ns", "target", 1, "Target"), false),
        "delref-branch",
    ))
    .await
    .unwrap();

    // CommandSlice on the same branch that emits the target event.
    c.put_entity(with_branch(
        put_req(
            pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                    id: Some(id("delref-ns", "referrer-slice", 1)),
                    title: "referrer-slice".into(),
                    emitted_events: vec![pb::EventEdge {
                        event: Some(pb::EventRef {
                            id: Some(id("delref-ns", "target", 1)),
                        }),
                        doc: String::new(),
                        metadata: Vec::new(),
                    }],
                    ..Default::default()
                })),
            },
            false,
        ),
        "delref-branch",
    ))
    .await
    .unwrap();

    // Control: ListIncomingReferences with the branch header sees the slice.
    let incoming = c
        .get_incoming_references(with_branch(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("delref-ns", "target", 1)),
                filter_kinds: vec![],
                page_size: 50,
                page_token: String::new(),
            },
            "delref-branch",
        ))
        .await
        .expect("GetIncomingReferences must honor branch header")
        .into_inner();
    assert!(
        !incoming.references.is_empty(),
        "branch-scoped incoming refs must include the CommandSlice"
    );

    let err = c
        .delete_entity(with_branch(
            pb::DeleteEntityRequest {
                operation_id: String::new(),
                kind: pb::EntityKind::Event as i32,
                id: Some(id("delref-ns", "target", 1)),
                if_match: String::new(),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            },
            "delref-branch",
        ))
        .await
        .expect_err("FailIfReferenced must block when a branch referrer exists");
    assert_eq!(
        err.code(),
        Code::FailedPrecondition,
        "expected FAILED_PRECONDITION from branch referrer scan; got {err}"
    );
}

// ---------------------------------------------------------------------------
// Structural three-way merge (stage 2 of the resolution ladder in
// docs/explanation/branching.md). An edit/edit conflict where the two sides
// changed different field paths has a defensible combined reading; one where
// they changed the same path does not. Everything below is about keeping
// those two cases apart, and about the combined reading staying opt-in.
// ---------------------------------------------------------------------------

fn event_with_doc(ns: &str, slug: &str, version: u64, title: &str, doc: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            doc: doc.into(),
            ..Default::default()
        })),
    }
}

fn event_parts(entity: &pb::Entity) -> (String, String) {
    match &entity.kind {
        Some(pb::entity::Kind::Event(e)) => (e.title.clone(), e.doc.clone()),
        other => panic!("expected an event, got {other:?}"),
    }
}

/// Fork `branch` off a baseline event, then have each side edit one of the
/// two scalars: the branch edits `doc`, baseline edits `title`. Disjoint
/// paths, so the entry is an edit/edit conflict with a structural merge.
async fn stage_disjoint_conflict(c: &mut EventModelServiceClient<Channel>, ns: &str, branch: &str) {
    c.put_entity(put_req(
        event_with_doc(ns, "a", 1, "Base title", "Base doc"),
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: branch.into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(
            event_with_doc(ns, "a", 1, "Base title", "Branch doc"),
            false,
        ),
        branch,
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event_with_doc(ns, "a", 1, "Baseline title", "Base doc"),
        true,
    ))
    .await
    .unwrap();
}

async fn baseline_event(c: &mut EventModelServiceClient<Channel>, ns: &str) -> pb::Entity {
    c.get_entity(pb::GetEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, "a", 1)),
    })
    .await
    .unwrap()
    .into_inner()
    .entity
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disjoint_edit_edit_conflict_carries_a_structural_merge() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;
    stage_disjoint_conflict(&mut c, "am-diff", "am-diff-branch").await;

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "am-diff-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        diff_status(&entries[0]),
        pb::branch_diff_entry::Status::ConflictEditEdit,
        "carrying a merge must not downgrade the status: it is still a conflict \
         until someone opts in"
    );
    let merged = entries[0]
        .auto_merged
        .as_ref()
        .expect("disjoint field edits must produce a structural merge");
    assert_eq!(
        event_parts(merged),
        ("Baseline title".to_string(), "Branch doc".to_string()),
        "the merge must carry each side's edit"
    );

    // Diffing is a read. Baseline keeps its own edit and nothing else.
    assert_eq!(
        event_parts(&baseline_event(&mut c, "am-diff").await),
        ("Baseline title".to_string(), "Base doc".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_same_path_collision_carries_no_structural_merge() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(
        event_with_doc("am-same", "a", 1, "Base title", "Base doc"),
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "am-same-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(
            event_with_doc("am-same", "a", 1, "Branch title", "Base doc"),
            false,
        ),
        "am-same-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event_with_doc("am-same", "a", 1, "Baseline title", "Base doc"),
        true,
    ))
    .await
    .unwrap();

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "am-same-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].auto_merged.is_none(),
        "both sides rewrote the same path; there is no combined reading to offer"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_merge_off_still_blocks_a_disjoint_conflict() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;
    stage_disjoint_conflict(&mut c, "am-off", "am-off-branch").await;

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "am-off-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Conflicts as i32,
        "combining two authors' edits is opt-in; the default must not do it"
    );
    assert_eq!(
        event_parts(&baseline_event(&mut c, "am-off").await),
        ("Baseline title".to_string(), "Base doc".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_merge_lands_the_combined_entity() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;
    stage_disjoint_conflict(&mut c, "am-on", "am-on-branch").await;

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "am-on-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: true,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Applied as i32,
        "expected the merge to apply, got conflicts: {:?}",
        resp.conflicts
    );
    assert_eq!(resp.applied_count, 1);
    assert_eq!(
        event_parts(&baseline_event(&mut c, "am-on").await),
        ("Baseline title".to_string(), "Branch doc".to_string()),
        "baseline must end up with both sides' edits, not one side's whole entity"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_merge_leaves_a_same_path_collision_blocked() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(
        event_with_doc("am-block", "a", 1, "Base title", "Base doc"),
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "am-block-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(
            event_with_doc("am-block", "a", 1, "Branch title", "Branch doc"),
            false,
        ),
        "am-block-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event_with_doc("am-block", "a", 1, "Baseline title", "Base doc"),
        true,
    ))
    .await
    .unwrap();

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "am-block-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: true,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Conflicts as i32,
        "opting in must not silently pick a winner for a contested field"
    );
    assert_eq!(
        event_parts(&baseline_event(&mut c, "am-block").await),
        ("Baseline title".to_string(), "Base doc".to_string()),
        "a blocked merge is all-or-nothing: nothing may land"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_merge_does_not_reach_an_edit_delete_conflict() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(
        event_with_doc("am-ed", "a", 1, "Base title", "Base doc"),
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "am-ed-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.delete_entity(with_branch(
        pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("am-ed", "a", 1)),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Unspecified as i32,
        },
        "am-ed-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event_with_doc("am-ed", "a", 1, "Baseline title", "Base doc"),
        true,
    ))
    .await
    .unwrap();

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "am-ed-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: true,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Conflicts as i32,
        "keeping versus dropping a key has no field-level reading to merge"
    );
    assert_eq!(
        diff_status(&resp.conflicts[0]),
        pb::branch_diff_entry::Status::ConflictEditDelete
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_the_merge_clears_the_conflict_on_the_branch() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;
    stage_disjoint_conflict(&mut c, "am-take", "am-take-branch").await;

    c.resolve_branch_entry(pb::ResolveBranchEntryRequest {
        operation_id: String::new(),
        name: "am-take-branch".into(),
        r#ref: Some(pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("am-take", "a", 1)),
        }),
        resolution: pb::resolve_branch_entry_request::Resolution::TakeMerged as i32,
        expected_state: None,
    })
    .await
    .unwrap();

    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "am-take-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        diff_status(&entries[0]),
        pb::branch_diff_entry::Status::Changed,
        "the branch now holds the combined entity over the current baseline"
    );
    assert_eq!(
        event_parts(entries[0].ours.as_ref().unwrap()),
        ("Baseline title".to_string(), "Branch doc".to_string())
    );

    // Resolution is branch-local: baseline is untouched until a merge.
    assert_eq!(
        event_parts(&baseline_event(&mut c, "am-take").await),
        ("Baseline title".to_string(), "Base doc".to_string())
    );

    let resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "am-take-branch".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::merge_branch_response::Status::Applied as i32,
        "a resolved entry must merge without needing auto_merge again: {:?}",
        resp.conflicts
    );
    assert_eq!(
        event_parts(&baseline_event(&mut c, "am-take").await),
        ("Baseline title".to_string(), "Branch doc".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_a_merge_that_does_not_exist_is_refused() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(
        event_with_doc("am-none", "a", 1, "Base title", "Base doc"),
        false,
    ))
    .await
    .unwrap();
    c.create_branch(pb::CreateBranchRequest {
        name: "am-none-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(
            event_with_doc("am-none", "a", 1, "Branch title", "Base doc"),
            false,
        ),
        "am-none-branch",
    ))
    .await
    .unwrap();
    c.put_entity(put_req(
        event_with_doc("am-none", "a", 1, "Baseline title", "Base doc"),
        true,
    ))
    .await
    .unwrap();

    let err = c
        .resolve_branch_entry(pb::ResolveBranchEntryRequest {
            operation_id: String::new(),
            name: "am-none-branch".into(),
            r#ref: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("am-none", "a", 1)),
            }),
            resolution: pb::resolve_branch_entry_request::Resolution::TakeMerged as i32,
            expected_state: None,
        })
        .await
        .expect_err("there is no merge to take for a same-path collision");
    assert_eq!(err.code(), Code::FailedPrecondition);

    // The refusal must leave the branch exactly as it was.
    let entries = c
        .diff_branch(pb::DiffBranchRequest {
            name: "am-none-branch".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .entries;
    assert_eq!(
        diff_status(&entries[0]),
        pb::branch_diff_entry::Status::ConflictEditEdit
    );
    assert_eq!(
        event_parts(entries[0].ours.as_ref().unwrap()),
        ("Branch title".to_string(), "Base doc".to_string())
    );
}

// ---------------------------------------------------------------------------
// Branch context on the two reads that used to ignore it.
//
// service.proto's "Branch context" note names the exempt RPCs explicitly:
// CreateBranch / ListBranches / DeleteBranch, SearchEntities, DeleteByQuery.
// ListNamespaces and ListEntitiesByDomain are not on that list, so answering
// from baseline while a branch header is present contradicts the contract a
// client is reading. Both read whole-store snapshots, which is exactly the
// shape a branch overlays.

#[tokio::test(flavor = "multi_thread")]
async fn list_namespaces_sees_a_namespace_that_only_exists_on_a_branch() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "ns-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    c.put_entity(with_branch(
        put_req(event("brnew-ns", "placed", 1, "Placed"), false),
        "ns-branch",
    ))
    .await
    .unwrap();

    let baseline: Vec<String> = c
        .list_namespaces(pb::ListNamespacesRequest {})
        .await
        .unwrap()
        .into_inner()
        .namespaces
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(
        !baseline.contains(&"brnew-ns".to_string()),
        "a branch write must not leak into the baseline listing: {baseline:?}"
    );

    let on_branch = c
        .list_namespaces(with_branch(pb::ListNamespacesRequest {}, "ns-branch"))
        .await
        .unwrap()
        .into_inner()
        .namespaces;
    let found = on_branch
        .iter()
        .find(|n| n.name == "brnew-ns")
        .expect("the branch's own namespace must be listed under the branch header");
    assert_eq!(found.entity_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_entities_by_domain_reads_the_branch_overlay() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "dom-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let domain = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id("brdom", "brdom", 1)),
            title: "Branch domain".into(),
            ..Default::default()
        })),
    };
    c.put_entity(with_branch(put_req(domain, true), "dom-branch"))
        .await
        .unwrap();

    let request = || pb::ListEntitiesByDomainRequest {
        domain_id: Some(id("brdom", "brdom", 1)),
        kinds: vec![],
        page_size: 50,
        page_token: String::new(),
        latest_versions_only: false,
    };

    let baseline = c
        .list_entities_by_domain(request())
        .await
        .unwrap()
        .into_inner()
        .entities;
    assert!(
        baseline.is_empty(),
        "the domain exists only on the branch: {baseline:?}"
    );

    let on_branch = c
        .list_entities_by_domain(with_branch(request(), "dom-branch"))
        .await
        .unwrap()
        .into_inner()
        .entities;
    assert_eq!(
        on_branch.len(),
        1,
        "the branch overlay must be visible to this read"
    );
}
