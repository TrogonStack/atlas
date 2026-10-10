#![allow(clippy::unwrap_used, clippy::expect_used, clippy::large_futures)]

use std::{path::PathBuf, sync::Arc};

use tonic::Request;
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

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/manifests/acme-customer-data-deletion")
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

fn load_manifests() -> Vec<pb::Entity> {
    trogon_atlas_client::manifest::load_paths(&[fixture_dir()])
        .expect("acme-customer-data-deletion manifests should load")
        .into_iter()
        .map(|m| m.entity)
        .collect()
}

async fn seed_entities(entities: Vec<pb::Entity>) -> EventModelServiceImpl {
    let ops: Vec<pb::BatchMutateOp> = entities.into_iter().map(put).collect();
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
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
        "fixture seeding must succeed; failure: {:?}",
        resp.failure
    );
    svc
}

async fn seed_service() -> EventModelServiceImpl {
    seed_entities(load_manifests()).await
}

async fn validate(svc: &EventModelServiceImpl) -> Vec<pb::ValidationIssue> {
    svc.validate_event_model(Request::new(pb::ValidateEventModelRequest {
        event_model: None,
        event_model_id: Some(id(
            "acme-customer-data-deletion",
            "acme-customer-data-deletion-pipeline",
            1,
        )),
    }))
    .await
    .unwrap()
    .into_inner()
    .issues
}

const PERSONAL_DATA_TYPE_URL: &str =
    "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.PersonalDataAnnotation";

fn edit_fields(entity: &mut pb::Entity, edit: impl FnOnce(&mut Vec<pb::FieldSpec>)) {
    let schema = match entity.kind.as_mut().expect("entity should carry a kind") {
        pb::entity::Kind::Event(e) => &mut e.schema,
        pb::entity::Kind::Command(c) => &mut c.schema,
        pb::entity::Kind::ReadModel(r) => &mut r.schema,
        other => panic!("unexpected entity kind for a PII fixture field: {other:?}"),
    };
    let any = schema
        .as_mut()
        .expect("fixture entity should carry a schema");
    let mut decoded = <pb::Schema as prost::Message>::decode(any.value.as_slice())
        .expect("fixture schema should be a Schema");
    edit(&mut decoded.fields);
    any.value = <pb::Schema as prost::Message>::encode_to_vec(&decoded);
}

fn personal_data_any(field: &mut pb::FieldSpec) -> &mut prost_types::Any {
    field
        .metadata
        .iter_mut()
        .find(|a| {
            a.type_url
                .ends_with("trogonatlas.eventmodel.v1alpha1.PersonalDataAnnotation")
        })
        .expect("field should carry a PersonalDataAnnotation")
}

/// Finds the first entity (by kind + id) whose `schema` has one named
/// `field_name`, for fixture entities that mutate exactly one occurrence.
fn find_entity_mut<'a>(
    entities: &'a mut [pb::Entity],
    kind: pb::EntityKind,
    namespace: &str,
    slug: &str,
) -> &'a mut pb::Entity {
    entities
        .iter_mut()
        .find(|e| match e.kind.as_ref() {
            Some(pb::entity::Kind::Event(ev)) => {
                kind == pb::EntityKind::Event
                    && ev
                        .id
                        .as_ref()
                        .is_some_and(|id| id.namespace == namespace && id.slug == slug)
            }
            Some(pb::entity::Kind::ReadModel(rm)) => {
                kind == pb::EntityKind::ReadModel
                    && rm
                        .id
                        .as_ref()
                        .is_some_and(|id| id.namespace == namespace && id.slug == slug)
            }
            _ => false,
        })
        .unwrap_or_else(|| panic!("entity {namespace}/{slug} not found among loaded manifests"))
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_customer_id_annotations_raise_no_pii_issues() {
    let svc = seed_service().await;
    let issues = validate(&svc).await;
    let pii_issues: Vec<_> = issues
        .iter()
        .filter(|i| i.code.starts_with("PII_"))
        .collect();
    assert!(
        pii_issues.is_empty(),
        "customer_id's PersonalDataAnnotation should not raise any PII issue: {pii_issues:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_personal_data_annotation_with_empty_doc_raises_pii_doc_missing() {
    let mut entities = load_manifests();
    let target = find_entity_mut(
        &mut entities,
        pb::EntityKind::Event,
        "acme-customer-data-deletion",
        "customer-data-deletion.completed",
    );
    edit_fields(target, |fields| {
        let field = fields
            .iter_mut()
            .find(|f| f.name == "customer_id")
            .expect("customer-data-deletion.completed should carry a customer_id field");
        let any = personal_data_any(field);
        let mut annotation =
            <pb::PersonalDataAnnotation as prost::Message>::decode(any.value.as_slice()).unwrap();
        annotation.doc = String::new();
        any.value = <pb::PersonalDataAnnotation as prost::Message>::encode_to_vec(&annotation);
    });

    let svc = seed_entities(entities).await;
    let issues = validate(&svc).await;
    let hit = issues
        .iter()
        .find(|i| i.code == "PII_DOC_MISSING" && i.subject_field.starts_with("schema.fields["));
    assert!(
        hit.is_some(),
        "an empty PersonalDataAnnotation doc should raise PII_DOC_MISSING: {issues:?}"
    );
    assert_eq!(
        hit.unwrap().severity,
        pb::validation_issue::Severity::Error as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_downstream_field_missing_annotation_raises_pii_flow_unmarked() {
    let mut entities = load_manifests();
    let target = find_entity_mut(
        &mut entities,
        pb::EntityKind::ReadModel,
        "acme-customer-data-deletion",
        "analytics-deletion-queue",
    );
    edit_fields(target, |fields| {
        fields
            .iter_mut()
            .find(|f| f.name == "customer_id")
            .expect("analytics-deletion-queue should carry a customer_id field")
            .metadata
            .retain(|a| a.type_url != PERSONAL_DATA_TYPE_URL);
    });

    let svc = seed_entities(entities).await;
    let issues = validate(&svc).await;
    let hits: Vec<_> = issues
        .iter()
        .filter(|i| {
            i.code == "PII_FLOW_UNMARKED"
                && i.subject
                    .as_ref()
                    .and_then(|s| s.id.as_ref())
                    .is_some_and(|id| id.slug == "analytics-deletion-queue")
        })
        .collect();
    assert!(
        hits.len() == 1,
        "a downstream read model field sourced from a personal-data event field, with no annotation of its own, should raise PII_FLOW_UNMARKED: {issues:?}"
    );
    assert!(
        hits.iter()
            .all(|i| i.severity == pb::validation_issue::Severity::Warning as i32),
        "{hits:?}"
    );
    assert!(
        hits.iter().all(|i| i.message.contains("customer_id")),
        "message should name the source field: {hits:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_raises_no_error_severity_issues() {
    let svc = seed_service().await;
    let issues = validate(&svc).await;
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == pb::validation_issue::Severity::Error as i32)
        .collect();
    assert!(
        errors.is_empty(),
        "fixture should validate with zero Error-severity issues: {errors:?}"
    );
}
