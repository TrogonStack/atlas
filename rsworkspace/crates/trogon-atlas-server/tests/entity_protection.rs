//! Enforced entity protection, through the public RPCs over a real auth
//! stack: who may protect and lift, that every write route refuses a
//! protected identity the same way in dry run and apply, that the known
//! bypasses (stripping the annotation, merge, revert, imported annotations)
//! do not work, and that a protect racing a write never lets the write land
//! after the protection.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use prost::Message;
use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
};
use tonic_types::StatusExt;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    conv::reason,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const READER: &str = "ep-reader-token";
const WRITER: &str = "ep-writer-token";
const ADMIN: &str = "ep-admin-token";
const ADMIN_NAME: &str = "ep-admin";

type Client = EventModelServiceClient<Channel>;

fn registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.ep-reader]
role = "reader"
tokens = ["{READER}"]

[principals.ep-writer]
role = "writer"
tokens = ["{WRITER}"]

[principals.{ADMIN_NAME}]
role = "admin"
tokens = ["{ADMIN}"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile = toml::from_str(&toml).unwrap();
    TokenRegistry::from_file_contents(file).unwrap()
}

async fn serve(svc: EventModelServiceImpl, auth: Option<TokenRegistry>) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let grpc = EventModelServiceServer::new(svc);
    match auth {
        Some(registry) => {
            let plane = InterceptedService::new(
                tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
                BearerAuth::new(Arc::new(registry)),
            );
            tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(plane)
                    .serve_with_incoming(incoming)
                    .await
                    .unwrap();
            });
        }
        None => {
            tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(grpc)
                    .serve_with_incoming(incoming)
                    .await
                    .unwrap();
            });
        }
    }
    EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

async fn service() -> EventModelServiceImpl {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap()
}

async fn start() -> Client {
    serve(service().await, Some(registry())).await
}

fn as_token<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    req
}

fn on_branch<T>(body: T, token: &str, branch: &str) -> tonic::Request<T> {
    let mut req = as_token(body, token);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).unwrap(),
    );
    req
}

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn event(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn annotated(mut entity: pb::Entity, reason: &str) -> pb::Entity {
    if let Some(pb::entity::Kind::Event(e)) = entity.kind.as_mut() {
        e.metadata.push(prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.ProtectionAnnotation"
                .into(),
            value: pb::ProtectionAnnotation {
                reason: reason.into(),
            }
            .encode_to_vec(),
        });
    }
    entity
}

fn slice_emitting(ns: &str, slug: &str, emitted: pb::Id) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(ns, slug, 1)),
            title: slug.into(),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef { id: Some(emitted) }),
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}

fn put(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }
}

fn put_op(entity: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put(entity))),
    }
}

fn delete(ns: &str, slug: &str, mode: pb::delete_entity_request::Mode) -> pb::DeleteEntityRequest {
    pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, slug, 1)),
        if_match: String::new(),
        mode: mode as i32,
    }
}

fn delete_by_query(
    ns: &str,
    slug: &str,
    mode: pb::delete_entity_request::Mode,
) -> pb::DeleteByQueryRequest {
    pb::DeleteByQueryRequest {
        project: String::new(),
        namespace: ns.into(),
        kind: pb::EntityKind::Event as i32,
        slug: slug.into(),
        mode: mode as i32,
        max_deletes: 10,
    }
}

fn protect_req(kind: pb::EntityKind, ns: &str, slug: &str) -> pb::ProtectEntityRequest {
    pb::ProtectEntityRequest {
        kind: kind as i32,
        namespace: ns.into(),
        slug: slug.into(),
        reason: "published contract".into(),
        validate_only: false,
    }
}

async fn protect(
    c: &mut Client,
    kind: pb::EntityKind,
    ns: &str,
    slug: &str,
) -> pb::EntityProtection {
    c.protect_entity(as_token(protect_req(kind, ns, slug), ADMIN))
        .await
        .unwrap()
        .into_inner()
        .protection
        .unwrap()
}

async fn seed(c: &mut Client, entity: pb::Entity) {
    c.put_entity(as_token(put(entity), WRITER)).await.unwrap();
}

async fn protection_of(c: &mut Client, ns: &str, slug: &str) -> Option<pb::EntityProtection> {
    c.get_entity_protection(as_token(
        pb::GetEntityProtectionRequest {
            kind: pb::EntityKind::Event as i32,
            namespace: ns.into(),
            slug: slug.into(),
        },
        READER,
    ))
    .await
    .unwrap()
    .into_inner()
    .protection
}

async fn changesets(c: &mut Client, ns: &str) -> Vec<pb::Changeset> {
    c.list_changesets(as_token(
        pb::ListChangesetsRequest {
            page_token: String::new(),
            page_size: 100,
            branch: String::new(),
            namespace: ns.into(),
        },
        READER,
    ))
    .await
    .unwrap()
    .into_inner()
    .changesets
}

async fn latest_title(c: &mut Client, ns: &str, slug: &str) -> Option<String> {
    let entity = c
        .get_latest_version(as_token(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: ns.into(),
                slug: slug.into(),
            },
            READER,
        ))
        .await
        .ok()?
        .into_inner()
        .entity?;
    match entity.kind {
        Some(pb::entity::Kind::Event(e)) => Some(e.title),
        _ => None,
    }
}

#[track_caller]
fn assert_protected(result: Result<impl std::fmt::Debug, tonic::Status>, identity: &str) {
    let err = result.expect_err("a write to a protected identity must be refused");
    assert_eq!(err.code(), Code::FailedPrecondition, "{err:?}");
    let info = err
        .get_details_error_info()
        .expect("the refusal must carry ErrorInfo");
    assert_eq!(info.reason, reason::ENTITY_PROTECTED, "{err:?}");
    assert_eq!(
        info.metadata.get("identity").map(String::as_str),
        Some(identity),
        "{err:?}"
    );
    assert!(err.message().contains("UnprotectEntity"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_authenticated_admin_may_protect_or_lift() {
    let mut c = start().await;
    seed(&mut c, event("ep-authz", "placed", 1, "Placed")).await;
    let req = || protect_req(pb::EntityKind::Event, "ep-authz", "placed");

    for token in [READER, WRITER] {
        let err = c.protect_entity(as_token(req(), token)).await.unwrap_err();
        assert_eq!(err.code(), Code::PermissionDenied, "{token}: {err:?}");
    }
    let err = c
        .protect_entity(on_branch(req(), ADMIN, "anything"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition, "{err:?}");
    assert!(protection_of(&mut c, "ep-authz", "placed").await.is_none());

    protect(&mut c, pb::EntityKind::Event, "ep-authz", "placed").await;
    let lift = || pb::UnprotectEntityRequest {
        kind: pb::EntityKind::Event as i32,
        namespace: "ep-authz".into(),
        slug: "placed".into(),
        validate_only: false,
        if_match: 0,
    };
    let err = c
        .unprotect_entity(as_token(lift(), WRITER))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied, "{err:?}");
    assert!(protection_of(&mut c, "ep-authz", "placed").await.is_some());

    let lifted = c
        .unprotect_entity(as_token(lift(), ADMIN))
        .await
        .unwrap()
        .into_inner();
    assert!(lifted.changed);
    assert!(protection_of(&mut c, "ep-authz", "placed").await.is_none());
    seed(&mut c, event("ep-authz", "placed", 1, "Edited after lift")).await;
    assert_eq!(
        latest_title(&mut c, "ep-authz", "placed").await.as_deref(),
        Some("Edited after lift")
    );

    let audit = changesets(&mut c, "ep-authz").await;
    let by_rpc = |rpc: &str| audit.iter().find(|cs| cs.rpc == rpc).cloned().unwrap();
    for (rpc, op) in [
        ("ProtectEntity", pb::change_event::Kind::Protected),
        ("UnprotectEntity", pb::change_event::Kind::Unprotected),
    ] {
        let cs = by_rpc(rpc);
        assert_eq!(cs.author, ADMIN_NAME, "{rpc}");
        assert_eq!(cs.ops.len(), 1, "{rpc}");
        assert_eq!(cs.ops[0].kind, op as i32, "{rpc}");
    }
    assert_eq!(by_rpc("UnprotectEntity").id, lifted.changeset_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_without_an_auth_stack_refuses_to_protect() {
    let mut c = serve(service().await, None).await;
    c.put_entity(put(event("ep-noauth", "placed", 1, "Placed")))
        .await
        .unwrap();
    let err = c
        .protect_entity(protect_req(pb::EntityKind::Event, "ep-noauth", "placed"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied, "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn protect_is_server_attributed_idempotent_and_previewable() {
    let mut c = start().await;
    seed(&mut c, event("ep-attr", "placed", 1, "Placed")).await;
    seed(&mut c, event("ep-attr", "shipped", 1, "Shipped")).await;

    let preview = c
        .protect_entity(as_token(
            pb::ProtectEntityRequest {
                validate_only: true,
                ..protect_req(pb::EntityKind::Event, "ep-attr", "shipped")
            },
            ADMIN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(preview.changed);
    assert!(protection_of(&mut c, "ep-attr", "shipped").await.is_none());

    let record = protect(&mut c, pb::EntityKind::Event, "ep-attr", "placed").await;
    assert_eq!(record.protected_by, ADMIN_NAME);
    assert_eq!(record.reason, "published contract");
    assert_ne!(record.changeset_id, "");
    assert_ne!(record.revision, 0);

    let again = c
        .protect_entity(as_token(
            protect_req(pb::EntityKind::Event, "ep-attr", "placed"),
            ADMIN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!again.changed);
    assert_eq!(again.protection.unwrap().changeset_id, record.changeset_id);

    let missing = c
        .protect_entity(as_token(
            protect_req(pb::EntityKind::Event, "ep-attr", "never-written"),
            ADMIN,
        ))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound, "{missing:?}");

    let listed = c
        .list_entity_protections(as_token(
            pb::ListEntityProtectionsRequest {
                namespace: "ep-attr".into(),
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner()
        .protections;
    assert_eq!(listed, vec![record]);
}

#[tokio::test(flavor = "multi_thread")]
async fn put_is_refused_for_everyone_in_dry_run_and_apply() {
    let mut c = start().await;
    seed(&mut c, event("ep-put", "placed", 1, "Placed")).await;
    protect(&mut c, pb::EntityKind::Event, "ep-put", "placed").await;
    let identity = "event/ep-put/placed";

    for token in [WRITER, ADMIN] {
        for validate_only in [true, false] {
            let edit = pb::PutEntityRequest {
                validate_only,
                ..put(event("ep-put", "placed", 1, "Edited"))
            };
            assert_protected(c.put_entity(as_token(edit, token)).await, identity);
            let next = pb::PutEntityRequest {
                validate_only,
                ..put(event("ep-put", "placed", 2, "Next version"))
            };
            assert_protected(c.put_entity(as_token(next, token)).await, identity);
        }
    }
    assert_eq!(
        latest_title(&mut c, "ep-put", "placed").await.as_deref(),
        Some("Placed")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_are_refused_in_dry_run_and_apply() {
    use pb::delete_entity_request::Mode;
    let mut c = start().await;
    seed(&mut c, event("ep-del", "placed", 1, "Placed")).await;
    protect(&mut c, pb::EntityKind::Event, "ep-del", "placed").await;
    let identity = "event/ep-del/placed";

    for mode in [Mode::DryRun, Mode::Force] {
        assert_protected(
            c.delete_entity(as_token(delete("ep-del", "placed", mode), ADMIN))
                .await,
            identity,
        );
        assert_protected(
            c.delete_by_query(as_token(delete_by_query("ep-del", "placed", mode), ADMIN))
                .await,
            identity,
        );
    }
    assert_eq!(
        latest_title(&mut c, "ep-del", "placed").await.as_deref(),
        Some("Placed")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_refuses_the_whole_batch() {
    let mut c = start().await;
    seed(&mut c, event("ep-batch", "placed", 1, "Placed")).await;
    protect(&mut c, pb::EntityKind::Event, "ep-batch", "placed").await;

    for validate_only in [true, false] {
        let req = pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![
                put_op(event("ep-batch", "unrelated", 1, "Unrelated")),
                put_op(event("ep-batch", "placed", 2, "Next version")),
            ],
            validate_only,
        };
        assert_protected(
            c.batch_mutate(as_token(req, WRITER)).await,
            "event/ep-batch/placed",
        );
    }
    assert!(latest_title(&mut c, "ep-batch", "unrelated")
        .await
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn stripping_the_annotation_or_importing_one_does_not_change_protection() {
    let mut c = start().await;

    seed(
        &mut c,
        annotated(event("ep-ann", "by-writer", 1, "W"), "self-granted"),
    )
    .await;
    assert!(protection_of(&mut c, "ep-ann", "by-writer").await.is_none());
    let batch = pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![put_op(annotated(
            event("ep-ann", "batch-writer", 1, "BW"),
            "self-granted",
        ))],
        validate_only: false,
    };
    c.batch_mutate(as_token(batch, WRITER)).await.unwrap();
    assert!(protection_of(&mut c, "ep-ann", "batch-writer")
        .await
        .is_none());

    let stored = c
        .put_entity(as_token(
            put(annotated(
                event("ep-ann", "by-admin", 1, "A"),
                "imported contract",
            )),
            ADMIN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    match stored.kind {
        Some(pb::entity::Kind::Event(e)) => assert!(
            e.metadata.is_empty(),
            "the annotation is a request, never stored content"
        ),
        other => panic!("unexpected {other:?}"),
    }
    let record = protection_of(&mut c, "ep-ann", "by-admin").await.unwrap();
    assert_eq!(record.reason, "imported contract");
    assert_eq!(record.protected_by, ADMIN_NAME);
    let audit = changesets(&mut c, "ep-ann").await;
    let cs = audit
        .iter()
        .find(|cs| cs.id == record.changeset_id)
        .expect("the write and the protection share one changeset");
    let kinds: Vec<i32> = cs.ops.iter().map(|op| op.kind).collect();
    assert_eq!(
        kinds,
        vec![
            pb::change_event::Kind::Put as i32,
            pb::change_event::Kind::Protected as i32
        ]
    );

    assert_protected(
        c.put_entity(as_token(
            put(event("ep-ann", "by-admin", 1, "Stripped")),
            ADMIN,
        ))
        .await,
        "event/ep-ann/by-admin",
    );

    let blank = c
        .put_entity(as_token(
            put(annotated(event("ep-ann", "blank", 1, "B"), "  ")),
            ADMIN,
        ))
        .await
        .unwrap_err();
    assert_eq!(blank.code(), Code::InvalidArgument, "{blank:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_refuses_to_rewrite_a_protected_referrer() {
    let mut c = start().await;
    seed(&mut c, event("ep-rt", "thing", 1, "V1")).await;
    seed(&mut c, event("ep-rt", "thing", 2, "V2")).await;
    seed(
        &mut c,
        slice_emitting("ep-rt", "slice", id("ep-rt", "thing", 1)),
    )
    .await;
    protect(&mut c, pb::EntityKind::CommandSlice, "ep-rt", "slice").await;

    for dry_run in [true, false] {
        let req = pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("ep-rt", "thing", 1)),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("ep-rt", "thing", 2)),
            }),
            dry_run,
            filter_kinds: vec![],
        };
        assert_protected(
            c.retarget_references(as_token(req, ADMIN)).await,
            "command_slice/ep-rt/slice",
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_cannot_overwrite_or_strip_protection_on_merge() {
    let mut c = start().await;
    seed(&mut c, event("ep-merge", "placed", 1, "Placed")).await;
    seed(&mut c, event("ep-merge", "shipped", 1, "Shipped")).await;
    c.create_branch(as_token(
        pb::CreateBranchRequest {
            name: "ep-merge-branch".into(),
            doc: String::new(),
        },
        WRITER,
    ))
    .await
    .unwrap();
    c.put_entity(on_branch(
        put(event("ep-merge", "placed", 1, "Overwritten on branch")),
        WRITER,
        "ep-merge-branch",
    ))
    .await
    .unwrap();
    c.delete_entity(on_branch(
        delete(
            "ep-merge",
            "shipped",
            pb::delete_entity_request::Mode::Force,
        ),
        WRITER,
        "ep-merge-branch",
    ))
    .await
    .unwrap();

    protect(&mut c, pb::EntityKind::Event, "ep-merge", "placed").await;
    protect(&mut c, pb::EntityKind::Event, "ep-merge", "shipped").await;

    assert_protected(
        c.put_entity(on_branch(
            put(event("ep-merge", "placed", 2, "Next on branch")),
            WRITER,
            "ep-merge-branch",
        ))
        .await,
        "event/ep-merge/placed",
    );
    for dry_run in [true, false] {
        let req = pb::MergeBranchRequest {
            name: "ep-merge-branch".into(),
            dry_run,
            keep_branch: true,
            auto_merge: false,
            operation_id: String::new(),
        };
        assert_protected(
            c.merge_branch(as_token(req, ADMIN)).await,
            "event/ep-merge/placed",
        );
    }
    assert_eq!(
        latest_title(&mut c, "ep-merge", "placed").await.as_deref(),
        Some("Placed")
    );
    assert_eq!(
        latest_title(&mut c, "ep-merge", "shipped").await.as_deref(),
        Some("Shipped")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revert_cannot_undo_a_protected_write_or_the_protection_itself() {
    let mut c = start().await;
    seed(&mut c, event("ep-revert", "placed", 1, "Placed")).await;
    seed(&mut c, event("ep-revert", "placed", 1, "Edited")).await;
    let edit = changesets(&mut c, "ep-revert")
        .await
        .into_iter()
        .filter(|cs| cs.rpc == "PutEntity")
        .max_by(|a, b| a.id.cmp(&b.id))
        .unwrap();
    let record = protect(&mut c, pb::EntityKind::Event, "ep-revert", "placed").await;

    for dry_run in [true, false] {
        let req = pb::RevertChangesetRequest {
            id: edit.id.clone(),
            dry_run,
            operation_id: String::new(),
        };
        assert_protected(
            c.revert_changeset(as_token(req, ADMIN)).await,
            "event/ep-revert/placed",
        );
        let undo_protect = pb::RevertChangesetRequest {
            id: record.changeset_id.clone(),
            dry_run,
            operation_id: String::new(),
        };
        let err = c
            .revert_changeset(as_token(undo_protect, ADMIN))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition, "{err:?}");
        assert!(err.message().contains("UnprotectEntity"), "{err:?}");
    }
    assert_eq!(
        latest_title(&mut c, "ep-revert", "placed").await.as_deref(),
        Some("Edited")
    );
    assert!(protection_of(&mut c, "ep-revert", "placed").await.is_some());
}

/// Race `write` against `ProtectEntity` on a fresh namespace, many times.
/// Whichever takes the mutation lock first wins, so the only acceptable
/// outcomes are a write that landed before the protection or a write
/// refused because of it. A write that landed after the protection would
/// make the protection's changeset older than the write's.
async fn race<F, Fut>(route: &str, kind: pb::EntityKind, slug: &str, write: F)
where
    F: Fn(Client, String) -> Fut,
    Fut: std::future::Future<Output = Result<(), tonic::Status>> + Send + 'static,
{
    let c = start().await;
    for round in 0..6 {
        let ns = format!("ep-race-{route}-{round}");
        let mut setup = c.clone();
        seed(&mut setup, event(&ns, "placed", 1, "Placed")).await;
        seed(&mut setup, event(&ns, "placed", 2, "Placed v2")).await;
        seed(
            &mut setup,
            slice_emitting(&ns, "slice", id(&ns, "placed", 1)),
        )
        .await;
        if route == "merge" {
            let branch = format!("{ns}-b");
            setup
                .create_branch(as_token(
                    pb::CreateBranchRequest {
                        name: branch.clone(),
                        doc: String::new(),
                    },
                    WRITER,
                ))
                .await
                .unwrap();
            setup
                .put_entity(on_branch(
                    put(event(&ns, "placed", 1, "Branch")),
                    WRITER,
                    &branch,
                ))
                .await
                .unwrap();
        }

        let mut protector = c.clone();
        let request = protect_req(kind, &ns, slug);
        let protecting =
            tokio::spawn(async move { protector.protect_entity(as_token(request, ADMIN)).await });
        let writing = tokio::spawn(write(c.clone(), ns.clone()));
        let protected = protecting.await.unwrap();
        let written = writing.await.unwrap();

        let protected = match protected {
            Ok(response) => response.into_inner().protection.unwrap(),
            Err(err) if route == "delete-by-query" => {
                assert_eq!(err.code(), Code::NotFound, "{route}: {err:?}");
                assert!(written.is_ok(), "{route}: both sides failed");
                continue;
            }
            Err(err) => panic!("{route}: protect failed: {err:?}"),
        };
        let audit = changesets(&mut c.clone(), &ns).await;
        let newest = audit.iter().map(|cs| cs.id.as_str()).max().unwrap();
        assert_eq!(
            newest, protected.changeset_id,
            "{route} round {round}: a write landed after the protection: {written:?}"
        );
        if let Err(err) = written {
            let short = if kind == pb::EntityKind::Event {
                "event"
            } else {
                "command_slice"
            };
            assert_protected(Err::<(), _>(err), &format!("{short}/{ns}/{slug}"));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_protect_and_put() {
    race(
        "put",
        pb::EntityKind::Event,
        "placed",
        |mut c, ns| async move {
            c.put_entity(as_token(put(event(&ns, "placed", 3, "Racing")), WRITER))
                .await
                .map(drop)
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_protect_and_batch_mutate() {
    race(
        "batch",
        pb::EntityKind::Event,
        "placed",
        |mut c, ns| async move {
            let req = pb::BatchMutateRequest {
                operation_id: String::new(),
                ops: vec![put_op(event(&ns, "placed", 3, "Racing"))],
                validate_only: false,
            };
            c.batch_mutate(as_token(req, WRITER)).await.map(drop)
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_protect_and_delete() {
    race(
        "delete",
        pb::EntityKind::Event,
        "placed",
        |mut c, ns| async move {
            c.delete_entity(as_token(
                pb::DeleteEntityRequest {
                    id: Some(id(&ns, "placed", 2)),
                    ..delete(&ns, "placed", pb::delete_entity_request::Mode::Force)
                },
                WRITER,
            ))
            .await
            .map(drop)
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_protect_and_delete_by_query() {
    race(
        "delete-by-query",
        pb::EntityKind::Event,
        "placed",
        |mut c, ns| async move {
            c.delete_by_query(as_token(
                delete_by_query(&ns, "placed", pb::delete_entity_request::Mode::Force),
                ADMIN,
            ))
            .await
            .map(drop)
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_protect_and_retarget_references() {
    race(
        "retarget",
        pb::EntityKind::CommandSlice,
        "slice",
        |mut c, ns| async move {
            let req = pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id(&ns, "placed", 1)),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id(&ns, "placed", 2)),
                }),
                dry_run: false,
                filter_kinds: vec![],
            };
            c.retarget_references(as_token(req, WRITER)).await.map(drop)
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_protect_and_revert() {
    race(
        "revert",
        pb::EntityKind::Event,
        "placed",
        |mut c, ns| async move {
            let edit = changesets(&mut c, &ns)
                .await
                .into_iter()
                .filter(|cs| {
                    cs.rpc == "PutEntity"
                        && cs.ops.iter().any(|op| {
                            op.entity
                                .as_ref()
                                .and_then(|e| e.id.as_ref())
                                .is_some_and(|id| id.slug == "placed")
                        })
                })
                .max_by(|a, b| a.id.cmp(&b.id))
                .unwrap();
            c.revert_changeset(as_token(
                pb::RevertChangesetRequest {
                    id: edit.id,
                    dry_run: false,
                    operation_id: String::new(),
                },
                WRITER,
            ))
            .await
            .map(drop)
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_protect_and_merge() {
    race(
        "merge",
        pb::EntityKind::Event,
        "placed",
        |mut c, ns| async move {
            c.merge_branch(as_token(
                pb::MergeBranchRequest {
                    name: format!("{ns}-b"),
                    dry_run: false,
                    keep_branch: true,
                    auto_merge: false,
                    operation_id: String::new(),
                },
                ADMIN,
            ))
            .await
            .map(drop)
        },
    )
    .await;
}

fn lift_req(ns: &str, slug: &str, if_match: u64) -> pb::UnprotectEntityRequest {
    pb::UnprotectEntityRequest {
        kind: pb::EntityKind::Event as i32,
        namespace: ns.into(),
        slug: slug.into(),
        validate_only: false,
        if_match,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lift_against_a_stale_revision_is_refused() {
    let mut c = start().await;
    seed(&mut c, event("ep-stale", "placed", 1, "Placed")).await;
    let record = protect(&mut c, pb::EntityKind::Event, "ep-stale", "placed").await;

    let stale = c
        .unprotect_entity(as_token(
            lift_req("ep-stale", "placed", record.revision + 1000),
            ADMIN,
        ))
        .await
        .unwrap_err();
    assert_eq!(stale.code(), Code::Aborted, "{stale:?}");
    assert_eq!(
        stale.get_details_error_info().unwrap().reason,
        reason::STALE_PRECONDITION
    );
    assert!(protection_of(&mut c, "ep-stale", "placed").await.is_some());

    let lifted = c
        .unprotect_entity(as_token(
            lift_req("ep-stale", "placed", record.revision),
            ADMIN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(lifted.changed);
    assert!(protection_of(&mut c, "ep-stale", "placed").await.is_none());
}

fn assert_unknown(result: Result<impl std::fmt::Debug, tonic::Status>, identity: &str) {
    let err = result.unwrap_err();
    assert_eq!(err.code(), Code::Unavailable, "{err:?}");
    let info = err.get_details_error_info().unwrap();
    assert_eq!(info.reason, reason::PROTECTION_STATE_UNKNOWN, "{err:?}");
    assert_eq!(info.metadata.get("identity").unwrap(), identity);
    assert!(info.metadata.contains_key("row_revision"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_protection_row_fails_closed_until_an_admin_lifts_it() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = trogon_atlas_store::NatsStoreConfig::new(
        &nats.url,
        trogon_atlas_store::StoreMetrics::default(),
    );
    config.bucket = format!("ep-unknown-{suffix}");
    config.branches_bucket = format!("ep-unknown-branches-{suffix}");
    config.changesets_bucket = format!("ep-unknown-changesets-{suffix}");
    config.revisions_bucket = format!("ep-unknown-revisions-{suffix}");
    config.batches_bucket = format!("ep-unknown-batches-{suffix}");
    config.namespaces_bucket = format!("ep-unknown-namespaces-{suffix}");
    config.lease_bucket = format!("ep-unknown-lease-{suffix}");
    config.stream = format!("EP_UNKNOWN_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("ep-unknown.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    let protections_bucket = format!("{}-protections", config.namespaces_bucket);
    let store = Box::pin(trogon_atlas_store::NatsStore::connect_with(config))
        .await
        .unwrap();
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    let mut c = serve(svc, Some(registry())).await;

    seed(&mut c, event("ep-unknown", "placed", 1, "Placed")).await;
    protect(&mut c, pb::EntityKind::Event, "ep-unknown", "placed").await;

    let client = async_nats::connect(&nats.url).await.unwrap();
    let kv = async_nats::jetstream::new(client)
        .get_key_value(&protections_bucket)
        .await
        .unwrap();
    let key = {
        use futures::StreamExt as _;
        let keys: Vec<String> = kv.keys().await.unwrap().map(Result::unwrap).collect().await;
        assert_eq!(keys.len(), 1, "{keys:?}");
        keys.into_iter().next().unwrap()
    };
    let garbage = kv
        .put(&key, vec![0xff, 0xff, 0xff, 0xff].into())
        .await
        .unwrap();

    let state = c
        .get_entity_protection(as_token(
            pb::GetEntityProtectionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: "ep-unknown".into(),
                slug: "placed".into(),
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(state.state, pb::ProtectionState::Unknown as i32);
    assert!(state.protection.is_none());
    assert_eq!(state.unknown.as_ref().unwrap().revision, garbage);

    let listed = c
        .list_entity_protections(as_token(
            pb::ListEntityProtectionsRequest {
                namespace: "ep-unknown".into(),
            },
            READER,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.protections, Vec::new());
    assert_eq!(listed.unknown.len(), 1);
    assert!(listed.complete);

    let edit = || put(event("ep-unknown", "placed", 1, "Edited"));
    assert_unknown(
        c.put_entity(as_token(
            pb::PutEntityRequest {
                validate_only: true,
                ..edit()
            },
            ADMIN,
        ))
        .await,
        "event/ep-unknown/placed",
    );
    assert_unknown(
        c.put_entity(as_token(edit(), ADMIN)).await,
        "event/ep-unknown/placed",
    );
    assert_unknown(
        c.protect_entity(as_token(
            protect_req(pb::EntityKind::Event, "ep-unknown", "placed"),
            ADMIN,
        ))
        .await,
        "event/ep-unknown/placed",
    );

    let lifted = c
        .unprotect_entity(as_token(lift_req("ep-unknown", "placed", garbage), ADMIN))
        .await
        .unwrap()
        .into_inner();
    assert!(lifted.changed);
    seed(&mut c, event("ep-unknown", "placed", 1, "Edited")).await;
    assert_eq!(
        latest_title(&mut c, "ep-unknown", "placed")
            .await
            .as_deref(),
        Some("Edited")
    );
}
