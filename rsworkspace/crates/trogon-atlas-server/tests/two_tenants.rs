//! Cross-tenant isolation with two tenants who each register a namespace
//! under the *same* bare name and use the *same* slugs, which is the gap
//! `tests/cross_tenant_reads.rs` leaves open (there, beta owns nothing under
//! the name acme used, so a `NotFound` proves little about a name beta also
//! holds). `RegisterNamespace` only requires a name be unused within the
//! caller's own parent, so acme and beta each mint a distinct id under the
//! identical display name `tt-shared`.
//!
//! Three things live here:
//!
//! - A coverage guard: every RPC the proto defines must fall into either the
//!   read matrix or the write matrix `required_role` produces, so a new RPC
//!   that forgets to classify itself fails this test instead of silently
//!   defaulting to `PermissionDenied` for everyone or, worse, to no check at
//!   all.
//! - A canary sweep of the read matrix: beta must never see acme's canary,
//!   under the token-file authorizer and, since the harness exists, under a
//!   real SpiceDB too.
//! - Regression tests for the admin+parent refusal and the branch-existence
//!   check this same change introduced, so the tests that were red while
//!   writing the fix stay in the tree as the proof the fix is still in force.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{collections::BTreeSet, sync::Arc};

use support::two_tenants::{
    assert_no_leak, authed, authed_branch, canary, client, create_branch, eref, event_entity,
    event_entity_titled, id, put_req, spicedb_world, world, ACME_TOKEN, BETA_TOKEN, EVENT_SLUG,
    GAMMA_TOKEN, MODEL_SLUG, NS, ROOT_TOKEN, SLICE_SLUG, STORYBOARD_SLUG,
};
use tonic::{Code, Request};
use trogon_atlas_core::OwnerId;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{
    auth::{Principal, PrincipalKind, Role, TokenRegistry},
    scope::NamespaceScope,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::Store;

// ---------------------------------------------------------------------------
// Coverage guard
// ---------------------------------------------------------------------------

/// Every RPC the proto declares must be classified as a read or a write, so
/// the sweep below is a sweep of the whole surface rather than of whichever
/// RPCs someone remembered to add a test for.
#[test]
fn every_rpc_is_in_the_read_matrix_or_the_write_matrix() {
    use prost_reflect::DescriptorPool;

    let pool = DescriptorPool::decode(trogon_atlas_proto::FILE_DESCRIPTOR_SET)
        .expect("FILE_DESCRIPTOR_SET must be a valid FileDescriptorSet");
    let svc = pool
        .get_service_by_name("trogonatlas.api.eventmodel.v1alpha1.EventModelService")
        .expect("EventModelService must exist in the descriptor pool");

    let mut read_matrix: BTreeSet<String> = BTreeSet::new();
    let mut write_matrix: BTreeSet<String> = BTreeSet::new();
    let mut unclassified: Vec<String> = Vec::new();

    for method in svc.methods() {
        let path = format!(
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/{}",
            method.name()
        );
        match trogon_atlas_server::auth::required_role(&path) {
            Some(Role::Reader) => {
                read_matrix.insert(path);
            }
            Some(Role::Writer | Role::Admin) => {
                write_matrix.insert(path);
            }
            None => unclassified.push(path),
        }
    }

    assert!(
        unclassified.is_empty(),
        "RPCs with no required_role entry, so neither matrix covers them: {unclassified:?}",
    );
    assert!(!read_matrix.is_empty(), "the read matrix must not be empty");
    assert!(
        !write_matrix.is_empty(),
        "the write matrix must not be empty"
    );
}

// ---------------------------------------------------------------------------
// C2: admin+parent is refused, at load time and at the handler
// ---------------------------------------------------------------------------

#[test]
fn registry_refuses_an_admin_principal_bound_to_a_parent() {
    let toml = r#"
[principals.dangerous]
role = "admin"
tokens = ["whatever"]
parent = "acme"
"#;
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(toml).expect("valid registry TOML");
    let err = TokenRegistry::from_file_contents(file)
        .expect_err("admin bound to a parent must be refused at load time");
    assert!(
        err.contains("admin") && err.contains("parent"),
        "error should explain the refusal: {err}",
    );
}

/// Bypasses the token registry entirely and injects the `Principal` the way
/// `BearerAuth` would, so this proves the handler itself refuses the
/// combination rather than relying on the registry having been the only way
/// to produce one.
#[tokio::test(flavor = "multi_thread")]
async fn move_namespace_refuses_a_bound_admin_even_injected_directly() {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let svc = EventModelServiceImpl::try_new(store).unwrap();

    let mut req = Request::new(pb::MoveNamespaceRequest {
        id: "whatever".into(),
        parent: "beta".into(),
    });
    req.extensions_mut().insert(Principal {
        name: Arc::from("cannot-happen-via-the-registry-but-might-another-way"),
        role: Role::Admin,
        kind: PrincipalKind::User,
        is_anonymous: false,
        namespaces: NamespaceScope::unrestricted(),
        parent: Some(OwnerId::parse("acme").unwrap()),
    });

    let err = svc
        .move_namespace(req)
        .await
        .expect_err("a bound admin must never be able to move a namespace");
    assert_eq!(err.code(), Code::PermissionDenied);
}

// ---------------------------------------------------------------------------
// C3: writes require the branch to exist
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_refuses_a_nonexistent_branch() {
    let mut c = client().await;
    let err = c
        .put_entity(authed_branch(
            put_req(event_entity(NS, EVENT_SLUG, &canary("acme"))),
            ACME_TOKEN,
            "no-such-branch",
        ))
        .await
        .expect_err("writing under a branch that was never created must be refused");
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_refuses_a_nonexistent_branch() {
    let mut c = client().await;
    c.put_entity(authed(
        put_req(event_entity(NS, EVENT_SLUG, &canary("acme"))),
        ACME_TOKEN,
    ))
    .await
    .expect("acme writes the entity on baseline first");

    let err = c
        .delete_entity(authed_branch(
            pb::DeleteEntityRequest {
                operation_id: String::new(),
                kind: pb::EntityKind::Event as i32,
                id: Some(id(NS, EVENT_SLUG)),
                mode: pb::delete_entity_request::Mode::Force as i32,
                if_match: String::new(),
            },
            ACME_TOKEN,
            "no-such-branch",
        ))
        .await
        .expect_err("deleting under a branch that was never created must be refused");
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_refuses_a_nonexistent_branch() {
    let mut c = client().await;
    let err = c
        .batch_mutate(authed_branch(
            pb::BatchMutateRequest {
                operation_id: String::new(),
                ops: vec![pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(put_req(event_entity(
                        NS,
                        EVENT_SLUG,
                        &canary("acme"),
                    )))),
                }],
                validate_only: false,
            },
            ACME_TOKEN,
            "no-such-branch",
        ))
        .await
        .expect_err("batch-writing under a branch that was never created must be refused");
    assert_eq!(err.code(), Code::NotFound);
}

/// Claim-on-first-write means the namespace below belongs to whoever writes
/// it first; this test is only useful because neither tenant has touched it
/// via `world()`, unlike the shared-name namespace the canary sweep uses.
fn acme_only_entity(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "tt-acme-only".into(),
                slug: slug.into(),
                version: 1,
            }),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

/// The branch check is additive: a write into a branch that exists but into
/// a namespace the caller does not own must still be `PermissionDenied`, not
/// quietly allowed because the branch happened to check out.
#[tokio::test(flavor = "multi_thread")]
async fn writing_into_a_real_branch_still_respects_namespace_ownership() {
    let mut c = client().await;
    c.put_entity(authed(put_req(acme_only_entity("seed")), ACME_TOKEN))
        .await
        .expect("acme claims its namespace");
    create_branch(&mut c, "acme-work").await;

    let err = c
        .put_entity(authed_branch(
            put_req(acme_only_entity("intruder")),
            BETA_TOKEN,
            "acme-work",
        ))
        .await
        .expect_err(
            "beta must not be able to write acme's namespace merely because the branch exists",
        );
    assert_eq!(err.code(), Code::PermissionDenied);
}

// ---------------------------------------------------------------------------
// C4: GetOperation is scoped by claim key, not the raw operation_id string
// ---------------------------------------------------------------------------

/// A claim key is `sha256(principal)[:16].operation_id`, so two tenants
/// reusing the identical literal `operation_id` never collide and, just as
/// importantly, neither can read the other's receipt back: beta asking
/// `GetOperation` for the exact id acme used must see no claim at all, never
/// acme's changeset.
#[tokio::test(flavor = "multi_thread")]
async fn get_operation_does_not_leak_a_receipt_across_tenants() {
    let mut c = client().await;
    let shared_operation_id = "op-shared-across-tenants";

    let put = c
        .put_entity(authed(
            pb::PutEntityRequest {
                operation_id: shared_operation_id.into(),
                ..put_req(acme_only_entity("receipt-owner"))
            },
            ACME_TOKEN,
        ))
        .await
        .expect("acme claims and applies the operation")
        .into_inner();
    let acme_changeset_id = put
        .operation_receipt
        .expect("a claimed put carries a receipt")
        .changeset_id;
    assert_ne!(acme_changeset_id, "");

    let beta_view = c
        .get_operation(authed(
            pb::GetOperationRequest {
                operation_id: shared_operation_id.into(),
            },
            BETA_TOKEN,
        ))
        .await
        .expect("GetOperation itself must succeed for beta")
        .into_inner();
    assert_eq!(
        beta_view.status,
        pb::OperationStatus::Unspecified as i32,
        "beta must see no claim under acme's operation_id, not acme's status"
    );
    assert_eq!(beta_view.changeset_id, "");
    assert_ne!(beta_view.changeset_id, acme_changeset_id);

    let acme_view = c
        .get_operation(authed(
            pb::GetOperationRequest {
                operation_id: shared_operation_id.into(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect("acme can read its own receipt back")
        .into_inner();
    assert_eq!(acme_view.status, pb::OperationStatus::Applied as i32);
    assert_eq!(acme_view.changeset_id, acme_changeset_id);
}

// ---------------------------------------------------------------------------
// Read-matrix canary sweep: identical namespace name, identical slugs
// ---------------------------------------------------------------------------

/// Every "cheap" read RPC (no pagination sweep, no streaming), called as
/// beta against a world where acme holds a namespace registered under the
/// identical display name and slugs beta's own namespace (`beta_ns`, a
/// distinct minted id) does. The only acceptable outcomes are beta's own
/// data, an empty/NotFound answer, or anything else that does not contain
/// acme's canary; see [`assert_no_leak`] for why `PermissionDenied` does not
/// count as safe here.
async fn sweep_read_matrix(
    mut c: trogon_atlas_proto::event_model_service_client::EventModelServiceClient<
        tonic::transport::Channel,
    >,
    beta_token: &str,
    beta_ns: &str,
    acme_canary: &str,
) {
    assert_no_leak(
        "get_entity",
        c.get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(beta_ns, EVENT_SLUG)),
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "batch_get_entities",
        c.batch_get_entities(authed(
            pb::BatchGetEntitiesRequest {
                keys: vec![
                    eref(beta_ns, pb::EntityKind::Event, EVENT_SLUG),
                    eref(beta_ns, pb::EntityKind::CommandSlice, SLICE_SLUG),
                ],
                dense: false,
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "list_entities",
        c.list_entities(authed(
            pb::ListEntitiesRequest {
                kinds: vec![pb::EntityKind::Event as i32],
                namespaces: vec![beta_ns.into()],
                latest_versions_only: false,
                page_size: 0,
                page_token: String::new(),
                lifecycle_status_in: Vec::new(),
                lifecycle_status_not_in: Vec::new(),
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "search_entities",
        c.search_entities(authed(
            pb::SearchEntitiesRequest {
                query: "canary".into(),
                kinds: Vec::new(),
                namespaces: Vec::new(),
                limit: 0,
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_impact",
        c.get_impact(authed(
            pb::GetImpactRequest {
                root: Some(eref(beta_ns, pb::EntityKind::Event, EVENT_SLUG)),
                max_depth: 5,
                ..Default::default()
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_outgoing_references",
        c.get_outgoing_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id(beta_ns, SLICE_SLUG)),
                ..Default::default()
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_incoming_references",
        c.get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(beta_ns, EVENT_SLUG)),
                ..Default::default()
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_supersession_chain",
        c.get_supersession_chain(authed(
            pb::GetSupersessionChainRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(beta_ns, EVENT_SLUG)),
                direction: pb::get_supersession_chain_request::Direction::Both as i32,
                ..Default::default()
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "list_versions",
        c.list_versions(authed(
            pb::ListVersionsRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: beta_ns.into(),
                slug: EVENT_SLUG.into(),
                ..Default::default()
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_latest_version",
        c.get_latest_version(authed(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: beta_ns.into(),
                slug: EVENT_SLUG.into(),
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_entity_history",
        c.get_entity_history(authed(
            pb::GetEntityHistoryRequest {
                r#ref: Some(eref(beta_ns, pb::EntityKind::Event, EVENT_SLUG)),
                ..Default::default()
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_slice_projection",
        c.get_slice_projection(authed(
            pb::GetSliceProjectionRequest {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id(beta_ns, SLICE_SLUG)),
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_storyboard_projection",
        c.get_storyboard_projection(authed(
            pb::GetStoryboardProjectionRequest {
                id: Some(id(beta_ns, STORYBOARD_SLUG)),
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "get_event_model_projection",
        c.get_event_model_projection(authed(
            pb::GetEventModelProjectionRequest {
                id: Some(id(beta_ns, MODEL_SLUG)),
                include_cross_model: true,
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "validate_event_model",
        c.validate_event_model(authed(
            pb::ValidateEventModelRequest {
                event_model: None,
                event_model_id: Some(id(beta_ns, MODEL_SLUG)),
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    assert_no_leak(
        "list_entities_by_domain",
        c.list_entities_by_domain(authed(
            pb::ListEntitiesByDomainRequest {
                domain_id: Some(id(beta_ns, MODEL_SLUG)),
                ..Default::default()
            },
            beta_token,
        ))
        .await,
        acme_canary,
    );

    // Not an `assert_no_leak` candidate: a baseline changeset carries no
    // owner at all (`changeset_visible` in `src/service.rs`), so it stays
    // invisible to a bound caller regardless of which branch or namespace is
    // asked about -- an empty page here carries no information about
    // whether acme's name exists, since beta would get the same empty page
    // asking about anything. Asserting it stays that way (and does not
    // quietly start returning changesets) is still worth doing.
    let changesets = c
        .list_changesets(authed(
            pb::ListChangesetsRequest {
                page_token: String::new(),
                page_size: 0,
                branch: String::new(),
            },
            beta_token,
        ))
        .await
        .expect("ListChangesets now filters instead of refusing a bound caller")
        .into_inner();
    assert!(
        changesets.changesets.is_empty(),
        "an unattributed changeset must stay invisible to a bound caller, got {changesets:?}",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_matrix_leaks_no_acme_canary_to_beta_under_the_token_file_authorizer() {
    let w = world().await;
    sweep_read_matrix(w.client, BETA_TOKEN, &w.beta_ns, &canary("acme")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn read_matrix_leaks_no_acme_canary_to_beta_under_spicedb() {
    let w = spicedb_world("rm1").await;
    sweep_read_matrix(w.client, &w.beta_token, &w.beta_ns, &canary("acme")).await;
}

/// `TokenRegistry` is used by both authorizer backends, so the registry
/// level of C2 is backend-agnostic. This still exercises it once more
/// end-to-end with SpiceDB wired in, since that is the deployment shape
/// GA actually runs.
#[tokio::test(flavor = "multi_thread")]
async fn put_entity_refuses_a_nonexistent_branch_under_spicedb() {
    let mut w = spicedb_world("branch1").await;
    let err = w
        .client
        .put_entity(authed_branch(
            put_req(event_entity(
                "tt-branch1-never-seen",
                "never-seen",
                &canary("acme"),
            )),
            &w.acme_token,
            "no-such-branch",
        ))
        .await
        .expect_err("writing under a branch that was never created must be refused");
    assert_eq!(err.code(), Code::NotFound);
}

// ---------------------------------------------------------------------------
// C5: `ListChanges` cursor must not leak records it filtered out
// ---------------------------------------------------------------------------

/// Both tenants' worlds land in one global change log: acme's four writes
/// land at seq 1-4, then beta's four at seq 5-8. A caller bound to acme must
/// not learn that eight records exist system-wide -- only that four do, and
/// that there is nothing more for it to see past them.
#[tokio::test(flavor = "multi_thread")]
async fn list_changes_next_token_stops_at_the_callers_last_visible_record() {
    let mut w = world().await;
    let resp = w
        .client
        .list_changes(authed(
            pb::ListChangesRequest {
                since_token: "0".into(),
                page_size: 50,
                scopes: Vec::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect("list_changes")
        .into_inner();
    assert_eq!(resp.events.len(), 4, "acme must see only its own writes");
    assert_eq!(
        resp.next_token, "4",
        "the cursor must stop at acme's own last visible record, not beta's",
    );
}

/// A tenant that owns no namespace at all admits nothing in the global
/// change log, but it must not be left polling the same spot forever just
/// because nothing in reach was visible: the cursor still advances to the
/// end of what exists. That advanced cursor must not be a plain sequence
/// number either -- that would tell an otherwise empty-handed caller
/// exactly how many records other tenants have written.
#[tokio::test(flavor = "multi_thread")]
async fn list_changes_does_not_hand_an_unprivileged_caller_the_global_tip() {
    let mut w = world().await;
    let resp = w
        .client
        .list_changes(authed(
            pb::ListChangesRequest {
                since_token: "0".into(),
                page_size: 50,
                scopes: Vec::new(),
            },
            GAMMA_TOKEN,
        ))
        .await
        .expect("list_changes")
        .into_inner();
    assert!(resp.events.is_empty(), "gamma owns nothing visible");
    assert_ne!(
        resp.next_token, "0",
        "the cursor must still advance past a page that is entirely invisible",
    );
    assert!(
        resp.next_token.parse::<u64>().is_err(),
        "a cursor advanced past invisible-only records must not be a plain \
         sequence number, or it would disclose how far other tenants have \
         written",
    );
}

/// An empty `since_token` resolves to the store's current tip before any
/// visibility filtering runs. When a bound caller polls at that exact
/// position and nothing has been written since, the lookahead loop never
/// reads a single record, so `next` never moves off its starting value --
/// the live tip itself. That value must still not be handed back as a plain
/// sequence number: acme does not know beta wrote all the way to seq 8, and
/// echoing "8" in plain text would tell it so on the very first poll.
#[tokio::test(flavor = "multi_thread")]
async fn list_changes_does_not_disclose_the_live_tip_through_an_empty_since_token() {
    let mut w = world().await;
    let resp = w
        .client
        .list_changes(authed(
            pb::ListChangesRequest {
                since_token: String::new(),
                page_size: 50,
                scopes: Vec::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect("list_changes")
        .into_inner();
    assert!(
        resp.events.is_empty(),
        "nothing has been written since the live tip",
    );
    assert!(
        resp.next_token.parse::<u64>().is_err(),
        "an empty since_token must never resolve to a plain, disclosed tip",
    );
}

/// A caller resuming right where its own visible writes end, with only
/// another tenant's writes ahead of it, must not stall on a page that is
/// entirely that other tenant's records: the cursor keeps scanning until it
/// reaches the true end of the log, and a second poll from the resulting
/// (sealed) cursor confirms there is nothing further to find.
#[tokio::test(flavor = "multi_thread")]
async fn list_changes_reaches_the_end_past_a_page_of_only_another_tenants_records() {
    let mut w = world().await;
    let first = w
        .client
        .list_changes(authed(
            pb::ListChangesRequest {
                since_token: "4".into(),
                page_size: 50,
                scopes: Vec::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect("list_changes")
        .into_inner();
    assert!(
        first.events.is_empty(),
        "every record after acme's own seq 4 belongs to beta",
    );
    assert_ne!(
        first.next_token, "4",
        "the cursor must advance past beta's page, not stall at acme's own tip",
    );

    let second = w
        .client
        .list_changes(authed(
            pb::ListChangesRequest {
                since_token: first.next_token,
                page_size: 50,
                scopes: Vec::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect("a sealed cursor must resolve for the principal it was minted for")
        .into_inner();
    assert!(
        second.events.is_empty(),
        "there is nothing beyond beta's writes for acme to see",
    );
}

// ---------------------------------------------------------------------------
// C6: search must not starve on a filter, and a tenant's own score must not
// move when another tenant writes more matching content
// ---------------------------------------------------------------------------

/// A caller-scoped namespace filter narrows which documents the ranking
/// considers, not which of an unfiltered top-N survive afterward: a real
/// match outside the top-N by raw score must still be found.
#[tokio::test(flavor = "multi_thread")]
async fn search_does_not_starve_behind_a_namespace_filter() {
    let mut w = world().await;

    w.client
        .put_entity(authed(
            put_req(event_entity_titled(
                &w.acme_ns,
                "needle",
                "needle zzzstarve",
            )),
            ACME_TOKEN,
        ))
        .await
        .expect("acme writes the one real match");

    // Six decoys, each repeating the term five times so every one of them
    // outscores the single-occurrence needle above. A prefetch-then-filter
    // search that caps itself at a handful of top-ranked documents before
    // ever looking at the namespace filter never reaches the needle.
    for i in 0..6 {
        w.client
            .put_entity(authed(
                put_req(event_entity_titled(
                    &w.beta_ns,
                    &format!("decoy-{i}"),
                    "zzzstarve zzzstarve zzzstarve zzzstarve zzzstarve",
                )),
                BETA_TOKEN,
            ))
            .await
            .expect("beta writes a decoy");
    }

    let resp = w
        .client
        .search_entities(authed(
            pb::SearchEntitiesRequest {
                query: "zzzstarve".into(),
                kinds: Vec::new(),
                namespaces: vec![w.acme_ns.clone()],
                limit: 1,
            },
            ROOT_TOKEN,
        ))
        .await
        .expect("search_entities")
        .into_inner();

    assert_eq!(
        resp.results.len(),
        1,
        "the namespace filter must still find acme's match even though six \
         higher-scoring decoys outrank it globally",
    );
}

/// A bound caller's own BM25 score must depend only on what it can see.
/// Scoring off a shared index whose term statistics span every tenant makes
/// an owner's result ranking shift with another tenant's writes -- itself a
/// leak of that tenant's volume even though no content crosses the boundary.
#[tokio::test(flavor = "multi_thread")]
async fn search_score_is_unaffected_by_another_tenants_writes() {
    let mut w = world().await;

    w.client
        .put_entity(authed(
            put_req(event_entity_titled(&w.acme_ns, "rareterm", "zzzinvariant")),
            ACME_TOKEN,
        ))
        .await
        .expect("acme writes its own match");

    let before = w
        .client
        .search_entities(authed(
            pb::SearchEntitiesRequest {
                query: "zzzinvariant".into(),
                kinds: Vec::new(),
                namespaces: Vec::new(),
                limit: 10,
            },
            ACME_TOKEN,
        ))
        .await
        .expect("search_entities before")
        .into_inner();
    assert_eq!(before.results.len(), 1, "only acme's own match is visible");
    let score_before = before.results[0].score;

    for i in 0..20 {
        w.client
            .put_entity(authed(
                put_req(event_entity_titled(
                    &w.beta_ns,
                    &format!("noise-{i}"),
                    "zzzinvariant zzzinvariant zzzinvariant",
                )),
                BETA_TOKEN,
            ))
            .await
            .expect("beta writes noise sharing acme's term");
    }

    let after = w
        .client
        .search_entities(authed(
            pb::SearchEntitiesRequest {
                query: "zzzinvariant".into(),
                kinds: Vec::new(),
                namespaces: Vec::new(),
                limit: 10,
            },
            ACME_TOKEN,
        ))
        .await
        .expect("search_entities after")
        .into_inner();
    assert_eq!(
        after.results.len(),
        1,
        "beta's noise must still never appear in acme's results",
    );
    assert_eq!(
        after.results[0].score, score_before,
        "acme's own score must be unaffected by another tenant's writes, \
         not merely acme's own result set",
    );
}

// ---------------------------------------------------------------------------
// C8: deleting a referenced entity must see referrers in every tenant
// ---------------------------------------------------------------------------

/// A caller's own referrer scan is narrowed to what it can see, but the
/// `FailIfReferenced` gate must not be: a referrer in a tenant the deleting
/// caller cannot see is still a referrer, and letting the delete through
/// silently breaks that other tenant's reference. The response must not say
/// so, though -- the delete simply stays blocked, the same generic error a
/// same-tenant referrer would produce.
#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_is_blocked_by_a_referrer_in_a_tenant_it_cannot_see() {
    let mut w = world().await;

    w.client
        .put_entity(authed(
            put_req(event_entity(&w.acme_ns, "lonely", "mark")),
            ACME_TOKEN,
        ))
        .await
        .expect("acme writes a standalone event nothing of its own references");

    w.client
        .put_entity(authed(
            put_req(pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                    id: Some(id(&w.beta_ns, "spy-slice")),
                    title: "spy-slice".into(),
                    emitted_events: vec![pb::EventEdge {
                        event: Some(pb::EventRef {
                            id: Some(id(&w.acme_ns, "lonely")),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
            }),
            BETA_TOKEN,
        ))
        .await
        .expect("beta writes a slice referencing acme's event across the tenant boundary");

    let status = w
        .client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                operation_id: String::new(),
                kind: pb::EntityKind::Event as i32,
                id: Some(id(&w.acme_ns, "lonely")),
                if_match: String::new(),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            },
            ACME_TOKEN,
        ))
        .await
        .expect_err("beta's reference must block the delete even though acme cannot see it");

    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        !status.message().contains(&w.beta_ns),
        "the rejection must not name the tenant holding the reference",
    );
}
