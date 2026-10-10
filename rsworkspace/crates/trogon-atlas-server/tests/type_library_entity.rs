#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::{metadata::MetadataValue, Code, Request, Status};
use tonic_types::StatusExt;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::service::EventModelServiceImpl;

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

async fn svc() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    EventModelServiceImpl::try_new(Arc::new(store)).unwrap()
}

fn library(ns: &str, slug: &str) -> pb::TypeLibrary {
    pb::TypeLibrary {
        id: Some(id(ns, slug, 1)),
        title: "Orders".into(),
        doc: "Order payloads".into(),
        files: vec![pb::ProtoSourceFile {
            path: format!("{}/v1/orders.proto", slug.replace('.', "/")),
            content: format!(
                "syntax = \"proto3\";\npackage {slug}.v1;\nmessage OrderPlaced {{ string order_id = 1; }}\n"
            ),
        }],
        provenance: Some(pb::TypeLibraryProvenance {
            source: Some(pb::type_library_provenance::Source::Git(
                pb::type_library_provenance::GitSnapshot {
                    repository_url: "https://example.com/acme/orders.git".into(),
                    r#ref: "main".into(),
                    commit: "0123456789abcdef".into(),
                    subdirectory: "proto".into(),
                },
            )),
        }),
        ..Default::default()
    }
}

fn with_source(mut lib: pb::TypeLibrary, path: &str, content: &str) -> pb::TypeLibrary {
    lib.files = vec![pb::ProtoSourceFile {
        path: path.into(),
        content: content.into(),
    }];
    lib
}

fn depending_on(mut lib: pb::TypeLibrary, dep: pb::Id) -> pb::TypeLibrary {
    lib.dependencies.push(pb::TypeLibraryRef { id: Some(dep) });
    lib
}

fn put_req(lib: pb::TypeLibrary) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity(lib)),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }
}

fn entity(lib: pb::TypeLibrary) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::TypeLibrary(lib)),
    }
}

fn on_branch<T>(body: T, branch: &str) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).unwrap(),
    );
    req
}

async fn put(svc: &EventModelServiceImpl, lib: pb::TypeLibrary) -> pb::PutEntityResponse {
    svc.put_entity(Request::new(put_req(lib)))
        .await
        .unwrap()
        .into_inner()
}

async fn put_err(svc: &EventModelServiceImpl, lib: pb::TypeLibrary) -> Status {
    svc.put_entity(Request::new(put_req(lib)))
        .await
        .expect_err("type library write must be rejected")
}

async fn history_len(svc: &EventModelServiceImpl, ns: &str, slug: &str) -> usize {
    svc.get_entity_history(Request::new(pb::GetEntityHistoryRequest {
        r#ref: Some(pb::EntityRef {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(id(ns, slug, 1)),
        }),
        page_token: String::new(),
        page_size: 100,
    }))
    .await
    .unwrap()
    .into_inner()
    .revisions
    .len()
}

const MONEY: &str = "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; string currency = 2; }\n";

fn money(ns: &str) -> pb::TypeLibrary {
    with_source(
        library(ns, "acme.money"),
        "acme/money/v1/money.proto",
        MONEY,
    )
}

fn billing(ns: &str) -> pb::TypeLibrary {
    depending_on(
        with_source(
            library(ns, "acme.billing"),
            "acme/billing/v1/invoice.proto",
            "syntax = \"proto3\";\npackage acme.billing.v1;\nimport \"acme/money/v1/money.proto\";\nimport \"google/protobuf/timestamp.proto\";\nmessage Invoice { acme.money.v1.Money total = 1; google.protobuf.Timestamp issued_at = 2; }\n",
        ),
        id(ns, "acme.money", 1),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn type_library_put_get_roundtrip() {
    let svc = svc().await;
    let lib = library("typelib-roundtrip", "acme.orders");
    let put = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            create_only: true,
            ..put_req(lib.clone())
        }))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(put.etag, "");

    let got = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(id("typelib-roundtrip", "acme.orders", 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.etag, put.etag);
    let Some(pb::entity::Kind::TypeLibrary(back)) = got.entity.and_then(|e| e.kind) else {
        panic!("expected a TypeLibrary back");
    };
    assert_eq!(back, lib);
}

#[tokio::test(flavor = "multi_thread")]
async fn re_putting_the_same_source_is_a_no_op() {
    let svc = svc().await;
    let ns = "typelib-idempotent";
    let first = put(&svc, library(ns, "acme.orders")).await;
    assert!(!first.no_op);
    let again = put(&svc, library(ns, "acme.orders")).await;
    assert!(again.no_op);
    assert_eq!(again.etag, first.etag);
    assert_eq!(history_len(&svc, ns, "acme.orders").await, 1);

    let mut retitled = library(ns, "acme.orders");
    retitled.title = "Order events".into();
    let retitled = put(&svc, retitled).await;
    assert!(!retitled.no_op);
    assert_eq!(history_len(&svc, ns, "acme.orders").await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn source_that_does_not_compile_is_rejected_with_diagnostics() {
    let svc = svc().await;
    let ns = "typelib-diagnostics";
    let broken = with_source(
        library(ns, "acme.orders"),
        "acme/orders/v1/orders.proto",
        "syntax = \"proto3\";\npackage acme.orders.v1;\nmessage OrderPlaced {\n  strin order_id = 1;\n}\n",
    );
    let status = put_err(&svc, broken.clone()).await;
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    let bad = status
        .get_details_bad_request()
        .expect("compile errors carry field violations");
    assert_eq!(
        bad.field_violations[0].field,
        "acme/orders/v1/orders.proto:4:3"
    );

    let dry_run = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            validate_only: true,
            ..put_req(broken)
        }))
        .await
        .expect_err("validate_only must compile too");
    assert_eq!(dry_run.code(), Code::InvalidArgument);

    let foreign = with_source(
        library(ns, "acme.orders"),
        "acme/orders/v1/orders.proto",
        "syntax = \"proto3\";\npackage acme.billing.v1;\nmessage Invoice {}\n",
    );
    assert_eq!(put_err(&svc, foreign).await.code(), Code::InvalidArgument);

    let mut kebab = library(ns, "order-events");
    kebab.files[0].content = "syntax = \"proto3\";\npackage order.events;\nmessage X {}\n".into();
    assert_eq!(put_err(&svc, kebab).await.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn overlapping_package_prefixes_are_rejected() {
    let svc = svc().await;
    let ns = "typelib-overlap";
    put(&svc, library(ns, "acme.orders")).await;
    let nested = with_source(
        library(ns, "acme.orders.returns"),
        "acme/orders/returns/v1/returns.proto",
        "syntax = \"proto3\";\npackage acme.orders.returns.v1;\nmessage Returned {}\n",
    );
    let status = put_err(&svc, nested).await;
    assert_eq!(status.code(), Code::AlreadyExists, "{status:?}");

    let sibling = with_source(
        library(ns, "acme.ordersx"),
        "acme/ordersx/v1/x.proto",
        "syntax = \"proto3\";\npackage acme.ordersx.v1;\nmessage X {}\n",
    );
    put(&svc, sibling).await;
    put(
        &svc,
        library("typelib-overlap-other", "acme.orders.returns"),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn breaking_changes_are_rejected_and_compatible_ones_land() {
    let svc = svc().await;
    let ns = "typelib-compat";
    put(&svc, money(ns)).await;

    let removed = with_source(
        library(ns, "acme.money"),
        "acme/money/v1/money.proto",
        "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; }\n",
    );
    let status = put_err(&svc, removed).await;
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(status.message().contains("currency"), "{status:?}");
    let failure = status
        .get_details_precondition_failure()
        .expect("breaking changes carry precondition violations");
    assert_eq!(failure.violations.len(), 1);

    let mut next_version = with_source(
        library(ns, "acme.money"),
        "acme/money/v1/money.proto",
        "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; }\n",
    );
    next_version.id = Some(id(ns, "acme.money", 2));
    assert_eq!(
        put_err(&svc, next_version).await.code(),
        Code::FailedPrecondition
    );

    let reserved = with_source(
        library(ns, "acme.money"),
        "acme/money/v1/money.proto",
        "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { reserved 2; reserved \"currency\"; int64 cents = 1; string memo = 3; }\n",
    );
    assert!(!put(&svc, reserved).await.no_op);
    assert_eq!(history_len(&svc, ns, "acme.money").await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn dependencies_must_exist_in_the_same_namespace() {
    let svc = svc().await;
    let ns = "typelib-deps";
    let status = put_err(&svc, billing(ns)).await;
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");

    put(&svc, money(ns)).await;
    put(&svc, billing(ns)).await;

    put(&svc, money("typelib-deps-elsewhere")).await;
    let mut cross = billing("typelib-deps-cross");
    cross.dependencies = vec![pb::TypeLibraryRef {
        id: Some(id("typelib-deps-elsewhere", "acme.money", 1)),
    }];
    assert_eq!(put_err(&svc, cross).await.code(), Code::InvalidArgument);

    let without_dep = {
        let mut lib = billing("typelib-deps-undeclared");
        lib.dependencies.clear();
        lib
    };
    put(&svc, money("typelib-deps-undeclared")).await;
    assert_eq!(
        put_err(&svc, without_dep).await.code(),
        Code::InvalidArgument
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branches_check_against_their_own_libraries() {
    let svc = svc().await;
    let ns = "typelib-branches";
    let branch = "typelib-branch-isolation";
    svc.create_branch(Request::new(pb::CreateBranchRequest {
        name: branch.into(),
        doc: String::new(),
    }))
    .await
    .unwrap();

    svc.put_entity(on_branch(put_req(library(ns, "acme.orders")), branch))
        .await
        .unwrap();

    let overlapping = with_source(
        library(ns, "acme"),
        "acme/root.proto",
        "syntax = \"proto3\";\npackage acme;\nmessage Root {}\n",
    );
    let on_branch_status = svc
        .put_entity(on_branch(put_req(overlapping.clone()), branch))
        .await
        .expect_err("the branch already owns acme.orders");
    assert_eq!(on_branch_status.code(), Code::AlreadyExists);

    put(&svc, overlapping).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn batches_check_type_libraries_too() {
    let svc = svc().await;
    let ns = "typelib-batch";
    let op = |lib: pb::TypeLibrary| pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_req(lib))),
    };
    let status = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![op(money(ns)), op(billing(ns))],
            validate_only: false,
            operation_id: String::new(),
        }))
        .await
        .expect_err("one type library per namespace per batch");
    assert_eq!(status.code(), Code::InvalidArgument);

    let status = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![op(billing(ns))],
            validate_only: false,
            operation_id: String::new(),
        }))
        .await
        .expect_err("billing's dependency is missing");
    assert_eq!(status.code(), Code::FailedPrecondition);

    svc.batch_mutate(Request::new(pb::BatchMutateRequest {
        ops: vec![op(money(ns))],
        validate_only: false,
        operation_id: String::new(),
    }))
    .await
    .unwrap();
}

fn delete_req(target: pb::Id, mode: pb::delete_entity_request::Mode) -> pb::DeleteEntityRequest {
    pb::DeleteEntityRequest {
        kind: pb::EntityKind::TypeLibrary as i32,
        id: Some(target),
        mode: mode as i32,
        if_match: String::new(),
        operation_id: String::new(),
    }
}

fn merge_req(branch: &str, dry_run: bool) -> pb::MergeBranchRequest {
    pb::MergeBranchRequest {
        name: branch.into(),
        dry_run,
        auto_merge: false,
        keep_branch: false,
        operation_id: String::new(),
    }
}

async fn create_branch(svc: &EventModelServiceImpl, branch: &str) {
    svc.create_branch(Request::new(pb::CreateBranchRequest {
        name: branch.into(),
        doc: String::new(),
    }))
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn libraries_that_others_depend_on_cannot_be_deleted_even_by_force() {
    let svc = svc().await;
    let ns = "typelib-delete-dependency";
    put(&svc, money(ns)).await;
    put(&svc, billing(ns)).await;

    for mode in [
        pb::delete_entity_request::Mode::FailIfReferenced,
        pb::delete_entity_request::Mode::Force,
    ] {
        let status = svc
            .delete_entity(Request::new(delete_req(id(ns, "acme.money", 1), mode)))
            .await
            .expect_err("acme.billing still depends on acme.money");
        assert_eq!(
            status.code(),
            Code::FailedPrecondition,
            "{mode:?}: {status:?}"
        );
    }

    let status = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(delete_req(
                    id(ns, "acme.money", 1),
                    pb::delete_entity_request::Mode::Force,
                ))),
            }],
            validate_only: false,
            operation_id: String::new(),
        }))
        .await
        .expect_err("a batch must not orphan acme.billing either");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let violations = status
        .get_details_precondition_failure()
        .expect("the dependents are named")
        .violations;
    assert_eq!(violations[0].subject, "acme.billing@v1");

    svc.batch_mutate(Request::new(pb::BatchMutateRequest {
        ops: [id(ns, "acme.billing", 1), id(ns, "acme.money", 1)]
            .into_iter()
            .map(|target| pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(delete_req(
                    target,
                    pb::delete_entity_request::Mode::Force,
                ))),
            })
            .collect(),
        validate_only: false,
        operation_id: String::new(),
    }))
    .await
    .expect("removing the dependent alongside its dependency leaves nothing dangling");
}

#[tokio::test(flavor = "multi_thread")]
async fn merges_refuse_to_land_overlapping_libraries() {
    let svc = svc().await;
    let ns = "typelib-merge-overlap";
    let branch = "typelib-merge-overlap";
    create_branch(&svc, branch).await;
    svc.put_entity(on_branch(put_req(library(ns, "acme.orders")), branch))
        .await
        .unwrap();
    put(
        &svc,
        with_source(
            library(ns, "acme"),
            "acme/root.proto",
            "syntax = \"proto3\";\npackage acme;\nmessage Root {}\n",
        ),
    )
    .await;

    for dry_run in [true, false] {
        let status = svc
            .merge_branch(Request::new(merge_req(branch, dry_run)))
            .await
            .expect_err("baseline acme now owns acme.orders");
        assert_eq!(
            status.code(),
            Code::AlreadyExists,
            "dry_run={dry_run}: {status:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn merges_refuse_to_land_libraries_whose_dependencies_are_gone() {
    let svc = svc().await;
    let ns = "typelib-merge-dependency";
    let branch = "typelib-merge-dependency";
    put(&svc, money(ns)).await;
    create_branch(&svc, branch).await;
    svc.put_entity(on_branch(put_req(billing(ns)), branch))
        .await
        .unwrap();
    svc.delete_entity(Request::new(delete_req(
        id(ns, "acme.money", 1),
        pb::delete_entity_request::Mode::FailIfReferenced,
    )))
    .await
    .unwrap();

    let status = svc
        .merge_branch(Request::new(merge_req(branch, false)))
        .await
        .expect_err("acme.money is gone from baseline");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");

    put(&svc, money(ns)).await;
    let merged = svc
        .merge_branch(Request::new(merge_req(branch, false)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32
    );
}

fn compile_req(lib: pb::TypeLibrary) -> pb::CompileTypeLibraryRequest {
    pb::CompileTypeLibraryRequest { library: Some(lib) }
}

fn resolve_req(ns: &str, type_url: &str) -> pb::ResolveTypeRequest {
    pb::ResolveTypeRequest {
        namespace: ns.into(),
        type_url: type_url.into(),
    }
}

fn file_names(encoded: &[u8]) -> Vec<String> {
    use prost::Message as _;
    prost_types::FileDescriptorSet::decode(encoded)
        .unwrap()
        .file
        .into_iter()
        .map(|f| f.name.unwrap_or_default())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn compiling_a_library_reports_problems_without_writing_it() {
    let svc = svc().await;
    let ns = "typelib-dry-run";

    let clean = svc
        .compile_type_library(Request::new(compile_req(money(ns))))
        .await
        .unwrap()
        .into_inner();
    assert!(clean.diagnostics.is_empty(), "{clean:?}");
    assert!(clean.compatibility_violations.is_empty(), "{clean:?}");
    let missing = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(id(ns, "acme.money", 1)),
        }))
        .await
        .expect_err("a dry run writes nothing");
    assert_eq!(missing.code(), Code::NotFound);

    let broken = with_source(
        library(ns, "acme.money"),
        "acme/money/v1/money.proto",
        "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money {\n  in64 cents = 1;\n}\n",
    );
    let report = svc
        .compile_type_library(Request::new(compile_req(broken)))
        .await
        .unwrap()
        .into_inner();
    let diagnostic = report.diagnostics.first().expect("a diagnostic");
    assert_eq!(diagnostic.path, "acme/money/v1/money.proto");
    assert_eq!(diagnostic.line, 4, "{diagnostic:?}");

    put(&svc, money(ns)).await;
    let renumbered = with_source(
        library(ns, "acme.money"),
        "acme/money/v1/money.proto",
        "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 3; string currency = 2; }\n",
    );
    let report = svc
        .compile_type_library(Request::new(compile_req(renumbered)))
        .await
        .unwrap()
        .into_inner();
    assert!(report.diagnostics.is_empty(), "{report:?}");
    assert!(
        !report.compatibility_violations.is_empty(),
        "removing field 1 breaks stored payloads"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolving_a_type_returns_its_library_and_imports() {
    let svc = svc().await;
    let ns = "typelib-resolve";
    put(&svc, money(ns)).await;
    put(&svc, billing(ns)).await;

    let resolved = svc
        .resolve_type(Request::new(resolve_req(
            ns,
            "type.googleapis.com/acme.billing.v1.Invoice",
        )))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resolved.library, Some(id(ns, "acme.billing", 1)));
    let files = file_names(&resolved.file_descriptor_set);
    assert_eq!(
        files.last().map(String::as_str),
        Some("acme/billing/v1/invoice.proto")
    );
    assert!(files.contains(&"acme/money/v1/money.proto".to_owned()));
    assert!(files.contains(&"google/protobuf/timestamp.proto".to_owned()));

    let builtin = svc
        .resolve_type(Request::new(resolve_req(
            ns,
            "type.googleapis.com/google.protobuf.Timestamp",
        )))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(builtin.library, None);

    let unknown = svc
        .resolve_type(Request::new(resolve_req(
            ns,
            "type.googleapis.com/acme.billing.v1.Refund",
        )))
        .await
        .expect_err("nothing declares Refund");
    assert_eq!(unknown.code(), Code::NotFound);

    let malformed = svc
        .resolve_type(Request::new(resolve_req(ns, "Invoice")))
        .await
        .expect_err("not a type url");
    assert_eq!(malformed.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_descriptor_set_lists_every_live_tenant_message() {
    let svc = svc().await;
    let ns = "typelib-descriptor-set";
    put(&svc, money(ns)).await;
    put(&svc, billing(ns)).await;

    let set = svc
        .get_type_library_descriptor_set(Request::new(pb::GetTypeLibraryDescriptorSetRequest {
            namespace: ns.into(),
        }))
        .await
        .unwrap()
        .into_inner();
    let mut messages: Vec<(String, Option<pb::Id>)> = set
        .messages
        .into_iter()
        .map(|m| (m.full_name, m.library))
        .collect();
    messages.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        messages,
        vec![
            (
                "acme.billing.v1.Invoice".to_owned(),
                Some(id(ns, "acme.billing", 1))
            ),
            (
                "acme.money.v1.Money".to_owned(),
                Some(id(ns, "acme.money", 1))
            ),
        ]
    );
    let files = file_names(&set.file_descriptor_set);
    let money_at = files
        .iter()
        .position(|f| f == "acme/money/v1/money.proto")
        .unwrap();
    let invoice_at = files
        .iter()
        .position(|f| f == "acme/billing/v1/invoice.proto")
        .unwrap();
    assert!(money_at < invoice_at, "dependencies come first: {files:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn type_lookups_follow_the_branch_header() {
    let svc = svc().await;
    let ns = "typelib-resolve-branch";
    let branch = "typelib-resolve-branch";
    create_branch(&svc, branch).await;
    svc.put_entity(on_branch(put_req(money(ns)), branch))
        .await
        .unwrap();

    let url = "type.googleapis.com/acme.money.v1.Money";
    let on_branch_resolved = svc
        .resolve_type(on_branch(resolve_req(ns, url), branch))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(on_branch_resolved.library, Some(id(ns, "acme.money", 1)));
    let baseline = svc
        .resolve_type(Request::new(resolve_req(ns, url)))
        .await
        .expect_err("baseline has no acme.money yet");
    assert_eq!(baseline.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_advertises_type_library_lookups() {
    let svc = svc().await;
    let info = svc
        .get_server_info(Request::new(pb::GetServerInfoRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert!(info.features.unwrap().type_libraries);
}

fn event_of(ns: &str, slug: &str, type_url: &str, value: Vec<u8>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, 1)),
            title: slug.into(),
            schema: Some(prost_types::Any {
                type_url: type_url.into(),
                value,
            }),
            ..Default::default()
        })),
    }
}

fn put_entity_req(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        ..put_req(library("unused", "unused"))
    }
}

const MONEY_URL: &str = "type.googleapis.com/acme.money.v1.Money";

fn batch_put(entity: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_entity_req(entity))),
    }
}

fn batch_delete(kind: pb::EntityKind, target: pb::Id) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            kind: kind as i32,
            ..delete_req(target, pb::delete_entity_request::Mode::Force)
        })),
    }
}

async fn batch(
    svc: &EventModelServiceImpl,
    ops: Vec<pb::BatchMutateOp>,
) -> Result<pb::BatchMutateResponse, Status> {
    svc.batch_mutate(Request::new(pb::BatchMutateRequest {
        ops,
        ..Default::default()
    }))
    .await
    .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn events_must_name_a_type_their_namespace_declares() {
    let svc = svc().await;
    let ns = "typelib-schema-unknown";
    put(&svc, money(ns)).await;

    for type_url in [
        "type.googleapis.com/acme.money.v1.Missing",
        "type.googleapis.com/acme.ledger.v1.Money",
        "not a type url",
    ] {
        let status = svc
            .put_entity(Request::new(put_entity_req(event_of(
                ns,
                "payment-received",
                type_url,
                Vec::new(),
            ))))
            .await
            .expect_err("an undeclared schema type must be refused");
        assert_eq!(
            status.code(),
            Code::InvalidArgument,
            "{type_url}: {status:?}"
        );
    }

    svc.put_entity(Request::new(put_entity_req(event_of(
        ns,
        "payment-received",
        MONEY_URL,
        Vec::new(),
    ))))
    .await
    .unwrap();

    let elsewhere = svc
        .put_entity(Request::new(put_entity_req(event_of(
            "typelib-schema-unknown-other",
            "payment-received",
            MONEY_URL,
            Vec::new(),
        ))))
        .await
        .expect_err("another namespace's types are not in scope");
    assert_eq!(elsewhere.code(), Code::InvalidArgument, "{elsewhere:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn event_payloads_must_decode_as_their_type() {
    let svc = svc().await;
    let ns = "typelib-schema-bytes";
    put(&svc, money(ns)).await;

    let truncated = vec![0x12, 0x05, b'U'];
    let undeclared_field = vec![0x08, 0x05, 0x48, 0x01];
    for value in [truncated, undeclared_field] {
        let status = svc
            .put_entity(Request::new(put_entity_req(event_of(
                ns,
                "payment-received",
                MONEY_URL,
                value.clone(),
            ))))
            .await
            .expect_err("bytes that are not the named type must be refused");
        assert_eq!(
            status.code(),
            Code::InvalidArgument,
            "{value:?}: {status:?}"
        );
    }

    let valid = vec![0x08, 0x05, 0x12, 0x03, b'U', b'S', b'D'];
    svc.put_entity(Request::new(put_entity_req(event_of(
        ns,
        "payment-received",
        MONEY_URL,
        valid,
    ))))
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_built_in_schema_type_needs_no_type_library() {
    use prost::Message as _;

    let svc = svc().await;
    let ns = "typelib-schema-builtin";
    let schema = pb::Schema {
        fields: vec![pb::FieldSpec {
            name: "order_id".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    svc.put_entity(Request::new(put_entity_req(event_of(
        ns,
        "order-placed",
        "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema",
        schema.encode_to_vec(),
    ))))
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn libraries_in_use_by_schemas_show_their_users_and_resist_deletion() {
    let svc = svc().await;
    let ns = "typelib-schema-impact";
    put(&svc, money(ns)).await;
    let event = id(ns, "payment-received", 1);
    svc.put_entity(Request::new(put_entity_req(event_of(
        ns,
        "payment-received",
        MONEY_URL,
        Vec::new(),
    ))))
    .await
    .unwrap();
    let library_ref = pb::EntityRef {
        kind: pb::EntityKind::TypeLibrary as i32,
        id: Some(id(ns, "acme.money", 1)),
    };

    let incoming = svc
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(id(ns, "acme.money", 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert!(
        incoming.iter().any(|r| r.from_field == "schema"
            && r.from.as_ref().and_then(|f| f.id.as_ref()) == Some(&event)),
        "{incoming:?}"
    );

    let impact = svc
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(library_ref),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(
        impact
            .nodes
            .iter()
            .any(|n| n.entity.as_ref().and_then(|e| e.id.as_ref()) == Some(&event)),
        "{impact:?}"
    );

    let status = svc
        .delete_entity(Request::new(delete_req(
            id(ns, "acme.money", 1),
            pb::delete_entity_request::Mode::Force,
        )))
        .await
        .expect_err("a library an event schema uses must not be deleted");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");

    let status = batch(
        &svc,
        vec![batch_delete(
            pb::EntityKind::TypeLibrary,
            id(ns, "acme.money", 1),
        )],
    )
    .await
    .expect_err("a batch must not strand an event schema either");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");

    batch(
        &svc,
        vec![
            batch_delete(pb::EntityKind::Event, event),
            batch_delete(pb::EntityKind::TypeLibrary, id(ns, "acme.money", 1)),
        ],
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_may_write_a_library_together_with_the_events_using_it() {
    let svc = svc().await;
    let ns = "typelib-schema-batch";
    batch(
        &svc,
        vec![
            batch_put(entity(money(ns))),
            batch_put(event_of(ns, "payment-received", MONEY_URL, Vec::new())),
        ],
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_schema_check_goes_stale_when_the_libraries_it_saw_move() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store: Arc<dyn trogon_atlas_store::Store> = Arc::new(nats.store().await);
    let svc = EventModelServiceImpl::try_new(Arc::clone(&store)).unwrap();
    let gate = trogon_atlas_server::type_libraries::TypeLibraryGate::default();
    let ns = "typelib-schema-race";
    put(&svc, money(ns)).await;
    let admission = gate
        .check_schema(
            &*store,
            &event_of(ns, "payment-received", MONEY_URL, Vec::new()),
            &[],
            None,
        )
        .await
        .unwrap()
        .expect("a tenant-typed schema yields an admission");
    gate.confirm(&*store, &admission, None).await.unwrap();

    let mut next = with_source(
        library(ns, "acme.money"),
        "acme/money/v1/money.proto",
        "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; string currency = 2; string note = 3; }\n",
    );
    next.id = Some(id(ns, "acme.money", 2));
    put(&svc, next).await;

    let err = gate.confirm(&*store, &admission, None).await.unwrap_err();
    assert_eq!(err.code(), Code::Aborted);
}
