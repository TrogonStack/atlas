#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use prost::Message;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const BUILT_IN: &str = "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.OrphanAnnotation";
const TENANT: &str = "type.googleapis.com/acme.notes.v1.ReviewNote";
const MISSPELLED: &str = "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.OrphanAnotation";

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default()).unwrap()
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn annotation(type_url: &str) -> prost_types::Any {
    prost_types::Any {
        type_url: type_url.into(),
        value: pb::OrphanAnnotation {
            doc: "kept for replaying v1 streams".into(),
        }
        .encode_to_vec(),
    }
}

fn notes_library(namespace: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
            id: Some(id(namespace, "acme.notes")),
            title: "Notes".into(),
            files: vec![pb::ProtoSourceFile {
                path: "acme/notes/v1/notes.proto".into(),
                content: "syntax = \"proto3\";\npackage acme.notes.v1;\nmessage ReviewNote { string doc = 1; }\n".into(),
            }],
            provenance: Some(pb::TypeLibraryProvenance {
                source: Some(pb::type_library_provenance::Source::Git(
                    pb::type_library_provenance::GitSnapshot {
                        repository_url: "https://example.com/acme/notes.git".into(),
                        r#ref: "main".into(),
                        commit: "0123456789abcdef".into(),
                        subdirectory: "proto".into(),
                    },
                )),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn annotated_event(namespace: &str, type_urls: &[&str]) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(namespace, "placed")),
            title: "placed".into(),
            metadata: type_urls.iter().map(|url| annotation(url)).collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn put_request(entity: pb::Entity, validate_only: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        validate_only,
        ..Default::default()
    }
}

async fn put(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
    validate_only: bool,
) -> pb::PutEntityResponse {
    service
        .put_entity(Request::new(put_request(entity, validate_only)))
        .await
        .unwrap()
        .into_inner()
}

async fn batch_put(
    service: &EventModelServiceImpl,
    entities: Vec<pb::Entity>,
    validate_only: bool,
) -> Vec<pb::PutEntityResponse> {
    service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: entities
                .into_iter()
                .map(|entity| pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(put_request(entity, false))),
                })
                .collect(),
            validate_only,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .results
        .into_iter()
        .map(|result| match result.result {
            Some(pb::batch_mutate_op_result::Result::Put(put)) => put,
            other => panic!("expected a put result, got {other:?}"),
        })
        .collect()
}

/// Each unknown-annotation warning as (severity, subject, path, message names the url).
fn unknown_annotations(issues: &[pb::ValidationIssue]) -> Vec<(i32, pb::Id, String, bool)> {
    issues
        .iter()
        .filter(|i| i.code == "UNKNOWN_ANNOTATION_TYPE")
        .map(|i| {
            (
                i.severity,
                i.subject.as_ref().unwrap().id.clone().unwrap(),
                i.subject_field.clone(),
                i.message.contains(MISSPELLED),
            )
        })
        .collect()
}

fn misspelled_at(namespace: &str, path: &str) -> Vec<(i32, pb::Id, String, bool)> {
    vec![(
        pb::validation_issue::Severity::Warning as i32,
        id(namespace, "placed"),
        path.into(),
        true,
    )]
}

#[tokio::test(flavor = "multi_thread")]
async fn put_warns_only_about_the_misspelled_annotation() {
    let service = service().await;
    for validate_only in [true, false] {
        let ns = format!("uat-put-{validate_only}");
        put(&service, notes_library(&ns), false).await;

        let response = put(
            &service,
            annotated_event(&ns, &[BUILT_IN, TENANT, MISSPELLED]),
            validate_only,
        )
        .await;

        assert_eq!(
            unknown_annotations(&response.validation),
            misspelled_at(&ns, "metadata[2]"),
            "validate_only={validate_only}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn put_warns_about_a_tenant_type_its_namespace_has_no_library_for() {
    let service = service().await;
    let ns = "uat-no-library";

    let response = put(&service, annotated_event(ns, &[BUILT_IN, TENANT]), false).await;

    let found: Vec<String> = response
        .validation
        .iter()
        .filter(|i| i.code == "UNKNOWN_ANNOTATION_TYPE")
        .map(|i| i.subject_field.clone())
        .collect();
    assert_eq!(found, vec!["metadata[1]".to_owned()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_resolves_a_library_written_in_the_same_batch() {
    let service = service().await;
    for validate_only in [true, false] {
        let ns = format!("uat-batch-{validate_only}");

        let results = batch_put(
            &service,
            vec![
                notes_library(&ns),
                annotated_event(&ns, &[BUILT_IN, TENANT, MISSPELLED]),
            ],
            validate_only,
        )
        .await;

        assert_eq!(unknown_annotations(&results[0].validation), Vec::new());
        assert_eq!(
            unknown_annotations(&results[1].validation),
            misspelled_at(&ns, "metadata[2]"),
            "validate_only={validate_only}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_warns_about_misspelled_annotations_nested_in_a_schema() {
    let service = service().await;
    let ns = "uat-nested";
    let mut event = annotated_event(ns, &[]);
    if let Some(pb::entity::Kind::Event(event)) = event.kind.as_mut() {
        event.schema = Some(prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema".into(),
            value: pb::Schema {
                fields: vec![pb::FieldSpec {
                    name: "order_id".into(),
                    r#type: Some(pb::FieldType {
                        kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
                    }),
                    metadata: vec![annotation(MISSPELLED)],
                    ..Default::default()
                }],
                ..Default::default()
            }
            .encode_to_vec(),
        });
    }

    let results = batch_put(&service, vec![event], false).await;

    assert_eq!(
        unknown_annotations(&results[0].validation),
        misspelled_at(ns, "schema.fields[0].metadata[0]")
    );
}
