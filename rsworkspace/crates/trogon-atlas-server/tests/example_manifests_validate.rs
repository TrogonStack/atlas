#![allow(clippy::unwrap_used, clippy::expect_used, clippy::large_futures)]

use std::{path::PathBuf, sync::Arc};

use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const NAMESPACES: [(&str, &str); 4] = [
    ("acme-storefront", "acme-storefront-model"),
    ("acme-reviews", "acme-reviews-model"),
    (
        "acme-customer-data-deletion",
        "acme-customer-data-deletion-pipeline",
    ),
    ("acme-wishlist", "acme-wishlist-model"),
];

fn manifests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/manifests")
}

fn put(entity: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(entity),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        })),
    }
}

async fn seed_all_examples() -> EventModelServiceImpl {
    let paths: Vec<PathBuf> = NAMESPACES
        .iter()
        .map(|(ns, _)| manifests_dir().join(ns))
        .collect();
    let ops: Vec<pb::BatchMutateOp> = trogon_atlas_client::manifest::load_paths(&paths)
        .expect("example manifests should load")
        .into_iter()
        .map(|m| put(m.entity))
        .collect();
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops,
            validate_only: false,
        }))
        .await
        .expect("batch_mutate failed")
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "example seeding must succeed; failure: {:?}",
        resp.failure
    );
    svc
}

#[tokio::test(flavor = "multi_thread")]
async fn example_models_raise_no_errors_or_warnings() {
    let svc = seed_all_examples().await;
    let mut findings = Vec::new();
    for (ns, slug) in NAMESPACES {
        let issues = svc
            .validate_event_model(Request::new(pb::ValidateEventModelRequest {
                event_model: None,
                event_model_id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: slug.into(),
                    version: 1,
                }),
            }))
            .await
            .unwrap()
            .into_inner()
            .issues;
        for i in issues {
            if i.severity >= pb::validation_issue::Severity::Warning as i32 {
                findings.push(format!(
                    "{ns} {:?} {} {:?} {} :: {}",
                    pb::validation_issue::Severity::try_from(i.severity).unwrap(),
                    i.code,
                    i.subject,
                    i.subject_field,
                    i.message
                ));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "examples must validate cleanly:\n{}",
        findings.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn example_projects_raise_no_errors_or_warnings() {
    let svc = seed_all_examples().await;
    let mut findings = Vec::new();
    for (ns, _) in NAMESPACES {
        let resp = svc
            .validate_project(Request::new(pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::ProjectId(pb::Id {
                    namespace: ns.into(),
                    slug: ns.into(),
                    version: 1,
                })),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.reports.is_empty(), "{ns} project reported no models");
        for report in resp.reports {
            for i in report.issues {
                if i.severity >= pb::validation_issue::Severity::Warning as i32 {
                    findings.push(format!("{ns} {} {:?} :: {}", i.code, i.subject, i.message));
                }
            }
        }
    }
    assert!(
        findings.is_empty(),
        "example projects must validate cleanly:\n{}",
        findings.join("\n")
    );
}
