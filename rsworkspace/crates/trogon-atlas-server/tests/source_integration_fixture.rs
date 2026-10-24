#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use serde_json::json;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn source_key(registration: &str) -> pb::SourceResourceKey {
    pb::SourceResourceKey {
        registration_id: registration.into(),
        category: "model".into(),
        key: "receipt".into(),
    }
}

fn bundle(registration: &str, version: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "profile": "supplied-native-v1", "profile_revision": 1,
        "registration": registration, "source_version": version,
        "partition": {"categories": ["model"], "logical_roots": ["native"], "selected_keys": []},
        "coverage": "PARTIAL",
        "resources": [{"key": {"category": "model", "key": "receipt"}, "file": "native/model.yaml", "pointer": {"kind": "native", "raw": "#/receipt"}}],
        "relations": [
            {"owner": {"category": "model", "key": "receipt"}, "occurrence": "first", "role": "observes", "position": 7, "target": {"kind": "opaque", "raw": "unresolved"}, "file": "native/model.yaml"},
            {"owner": {"category": "model", "key": "receipt"}, "occurrence": "second", "role": "observes", "position": 3, "target": {"kind": "opaque", "raw": "unresolved"}, "file": "native/model.yaml"}
        ]
    })).unwrap()
}

async fn get(service: &EventModelServiceImpl, namespace: &str) -> pb::GetEntityResponse {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(namespace, "receipt")),
        }))
        .await
        .unwrap()
        .into_inner()
}

fn read_model(entity: &pb::Entity) -> &pb::ReadModel {
    let Some(pb::entity::Kind::ReadModel(value)) = &entity.kind else {
        panic!("read model")
    };
    value
}

fn changed_title(entity: &pb::Entity, title: &str) -> pb::Entity {
    let mut entity = entity.clone();
    let Some(pb::entity::Kind::ReadModel(value)) = &mut entity.kind else {
        panic!("read model")
    };
    value.title = title.into();
    value.doc = "Unowned source documentation".into();
    entity
}

#[tokio::test]
async fn source_reconciliation_preserves_native_anchors_local_fields_and_original_bytes() {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0);
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "source-fixture".into(),
            parent: "source-fixture-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let put = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                    id: Some(id(&namespace, "receipt")),
                    title: "Native receipt".into(),
                    doc: "Locally maintained documentation".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let original = put.entity.unwrap();
    let original_uid = original.system.as_ref().unwrap().uid.clone();
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Processor(pb::Processor {
                    id: Some(id(&namespace, "notifier")),
                    title: "Notifier".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    let slice = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                    id: Some(id(&namespace, "observe")),
                    title: "Observe receipt".into(),
                    source_read_models: (0..2)
                        .map(|_| pb::ReadModelEdge {
                            read_model: Some(pb::ReadModelRef {
                                id: Some(id(&namespace, "receipt")),
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    processor: Some(pb::ProcessorEdge {
                        processor: Some(pb::ProcessorRef {
                            id: Some(id(&namespace, "notifier")),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(observation)) = &slice.kind else {
        panic!("slice")
    };
    assert_ne!(
        observation.source_read_models[0].connection_id,
        observation.source_read_models[1].connection_id
    );
    let registration = service
        .create_source_registration(Request::new(pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: "receipt-source".into(),
                administration_namespace: namespace.clone(),
                destination_namespaces: vec![namespace.clone()],
                profile_revision: 1,
                ownership_policy_revision: "policy-a".into(),
                deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let inventory = bundle(&registration.id, "00.RC-A");
    let file_bytes = b"# retained original bytes\nreceipt: opaque\n";
    let snapshot_request = pb::AcceptSourceSnapshotRequest {
        registration_id: registration.id.clone(),
        registration_revision: registration.revision.clone(),
        inventory_bytes: inventory.clone(),
        files: vec![pb::SourceFile {
            logical_name: "native/model.yaml".into(),
            media_type: "application/yaml".into(),
            content: file_bytes.to_vec(),
            ..Default::default()
        }],
    };
    let snapshot = service
        .accept_source_snapshot(Request::new(snapshot_request.clone()))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    let same = service
        .accept_source_snapshot(Request::new(snapshot_request.clone()))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert_eq!(snapshot.id, same.id);
    assert_eq!(snapshot.bundle_digest, same.bundle_digest);
    let mut collision = snapshot_request;
    collision.inventory_bytes.push(b' ');
    assert_eq!(
        service
            .accept_source_snapshot(Request::new(collision))
            .await
            .unwrap_err()
            .code(),
        Code::AlreadyExists
    );
    let observed = pb::SourceObservedOwner {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(&namespace, "receipt")),
        }),
        uid: original_uid.clone(),
        etag: put.etag.clone(),
    };
    let mapping = service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "receipt-title".into(),
                source: Some(source_key(&registration.id)),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(
                        observed.owner.clone().unwrap(),
                    )),
                }),
                ownership: Some(pb::SourceFieldOwnership {
                    policy: Some(pb::source_field_ownership::Policy::Fields(
                        pb::SourceOwnedFields {
                            fields: vec![pb::SourceOwnedField::Title as i32],
                        },
                    )),
                }),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(observed),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    let overlay = service
        .get_source_overlay(Request::new(pb::GetSourceOverlayRequest {
            registration_id: registration.id.clone(),
            snapshot_id: snapshot.id.clone(),
            page_size: 500,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .overlay
        .unwrap();
    let occurrences: Vec<_> = overlay
        .entries
        .iter()
        .filter_map(|entry| entry.occurrence_key.as_deref())
        .collect();
    assert_eq!(occurrences, ["first", "second"]);
    let preview = service
        .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: "receipt-update".into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source_key(&registration.id)),
                entity: Some(changed_title(&original, "Source receipt")),
            }],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(get(&service, &namespace).await.etag, put.etag);
    assert_eq!(preview.status, pb::SourceRunStatus::Previewed as i32);
    let operations = service
        .list_source_run_operations(Request::new(pb::ListSourceRunOperationsRequest {
            registration_id: registration.id.clone(),
            run_id: preview.id.clone(),
            page_size: 500,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(operations.preview_digest, preview.preview_digest);
    assert_ne!(
        operations.entries,
        Vec::<pb::SourceRunOperationEntry>::new()
    );
    let apply_request = pb::ApplySourceRunRequest {
        registration_id: registration.id.clone(),
        run_id: preview.id.clone(),
        if_match: preview.revision.clone(),
        preview_digest: preview.preview_digest.clone(),
    };
    let applied = service
        .apply_source_run(Request::new(apply_request.clone()))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let result_page = service
        .list_source_run_results(Request::new(pb::ListSourceRunResultsRequest {
            registration_id: registration.id.clone(),
            run_id: applied.id.clone(),
            page_size: 500,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let fidelity = result_page.fidelity.unwrap();
    assert_eq!(
        fidelity.checked_coverage,
        pb::SourceCoverage::Partial as i32
    );
    assert!(fidelity.original_bytes_recoverable);
    let retained_occurrences: Vec<_> = fidelity
        .records
        .iter()
        .filter_map(|record| {
            record.occurrence_key.as_ref().map(|key| {
                assert!(matches!(
                    pb::SourceFidelityDisposition::try_from(record.disposition).unwrap(),
                    pb::SourceFidelityDisposition::RetainedOverlayOnly
                        | pb::SourceFidelityDisposition::Deferred
                ));
                key.as_str()
            })
        })
        .collect();
    assert_eq!(retained_occurrences, ["first", "second"]);
    let current = get(&service, &namespace).await;
    let current_entity = current.entity.as_ref().unwrap();
    assert_eq!(read_model(current_entity).title, "Source receipt");
    assert_eq!(
        read_model(current_entity).doc,
        "Locally maintained documentation"
    );
    assert_eq!(current_entity.system.as_ref().unwrap().uid, original_uid);
    let replay = service
        .apply_source_run(Request::new(apply_request))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(replay.status, pb::SourceRunStatus::Applied as i32);
    assert_eq!(get(&service, &namespace).await.etag, current.etag);
    let preserved_slice = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(id(&namespace, "observe")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(preserved_slice, slice);
    let retained = service
        .get_source_snapshot(Request::new(pb::GetSourceSnapshotRequest {
            registration_id: registration.id.clone(),
            snapshot_id: snapshot.id.clone(),
            include_bytes: true,
        }))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert_eq!(retained.inventory_bytes, inventory);
    assert_eq!(retained.files[0].content, file_bytes);
    let accepted_mapping = service
        .get_source_mapping(Request::new(pb::GetSourceMappingRequest {
            registration_id: registration.id.clone(),
            mapping_id: mapping.id.clone(),
        }))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert_eq!(accepted_mapping.last_accepted_run_id, applied.id);
    let base = accepted_mapping.base.unwrap();
    assert_eq!(
        read_model(base.native_value.as_ref().unwrap()).doc,
        "Locally maintained documentation"
    );
    assert_eq!(
        read_model(base.source_value.as_ref().unwrap()).doc,
        "Unowned source documentation"
    );

    let mut local = current_entity.clone();
    let Some(pb::entity::Kind::ReadModel(local_model)) = &mut local.kind else {
        panic!("read model")
    };
    local_model.title = "Local receipt edit".into();
    let local = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(local),
            if_match: current.etag,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    for (version, run_id, incoming_title, expected) in [
        (
            "00.RC-B",
            "receipt-drift",
            "Source receipt",
            pb::SourceResultDisposition::LocalDrift,
        ),
        (
            "00.RC-C",
            "receipt-conflict",
            "Source receipt revision",
            pb::SourceResultDisposition::Conflict,
        ),
    ] {
        let snapshot = service
            .accept_source_snapshot(Request::new(pb::AcceptSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                inventory_bytes: bundle(&registration.id, version),
                files: vec![pb::SourceFile {
                    logical_name: "native/model.yaml".into(),
                    media_type: "application/yaml".into(),
                    content: file_bytes.to_vec(),
                    ..Default::default()
                }],
            }))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap();
        assert_eq!(snapshot.accepted_at_unix_ms, None);
        let preview = service
            .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                snapshot_id: snapshot.id,
                run_id: run_id.into(),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(source_key(&registration.id)),
                    entity: Some(changed_title(&original, incoming_title)),
                }],
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        let results = service
            .list_source_run_results(Request::new(pb::ListSourceRunResultsRequest {
                registration_id: registration.id.clone(),
                run_id: preview.id.clone(),
                page_size: 500,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(results
            .results
            .iter()
            .any(|result| result.disposition == expected as i32));
        if expected == pb::SourceResultDisposition::Conflict {
            assert_eq!(
                service
                    .apply_source_run(Request::new(pb::ApplySourceRunRequest {
                        registration_id: registration.id.clone(),
                        run_id: preview.id,
                        if_match: preview.revision,
                        preview_digest: preview.preview_digest,
                    }))
                    .await
                    .unwrap_err()
                    .code(),
                Code::FailedPrecondition
            );
        }
        assert_eq!(get(&service, &namespace).await.etag, local.etag);
        let mapping = service
            .get_source_mapping(Request::new(pb::GetSourceMappingRequest {
                registration_id: registration.id.clone(),
                mapping_id: mapping.id.clone(),
            }))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert_eq!(mapping.last_accepted_run_id, applied.id);
        assert_eq!(mapping.base.unwrap(), base);
    }
}
