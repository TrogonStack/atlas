#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use serde_json::json;
use tokio::net::TcpListener;
use trogon_atlas_client::{client, source_integration as source};
use trogon_atlas_core::source_bundle::{validate_bundle, SuppliedFile};
use trogon_atlas_proto::{
    self as pb,
    event_model_service_server::{EventModelService, EventModelServiceServer},
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

async fn server() -> String {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let namespace = service
        .register_namespace(tonic::Request::new(pb::RegisterNamespaceRequest {
            name: "source-surfaces".into(),
            parent: "source-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let registry = TokenRegistry::from_file_contents(serde_json::from_value(json!({"principals": {
        "admin": {"role":"admin", "tokens":["source-admin"]},
        "writer": {"role":"writer", "tokens":["source-writer"], "parent":"source-owner", "namespaces":[namespace]},
        "reader": {"role":"reader", "tokens":["source-reader"], "parent":"source-owner"},
        "hidden": {"role":"reader", "tokens":["source-hidden"], "parent":"other-owner"}
    }})).unwrap()).unwrap();
    let grpc = EventModelServiceServer::new(service)
        .max_decoding_message_size(source::MAX_SOURCE_MESSAGE_BYTES)
        .max_encoding_message_size(source::MAX_SOURCE_MESSAGE_BYTES);
    let secured = tonic::service::interceptor::InterceptedService::new(
        tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
        BearerAuth::new(registry),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(secured)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{address}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_source_client_roundtrips_native_commit_fidelity_and_bytes() {
    let endpoint = server().await;
    let mut admin = client::connect(&endpoint, Some("source-admin"), 30)
        .await
        .unwrap();
    let namespace = admin
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "source-surfaces".into(),
            parent: "source-owner".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let original = admin
        .put_entity(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                    id: Some(pb::Id {
                        namespace: namespace.clone(),
                        slug: "receipt".into(),
                        version: 1,
                    }),
                    title: "Original".into(),
                    doc: "Local rationale retained".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let native = original.entity.unwrap();
    let registration = source::create_source_registration(
        &mut admin,
        pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: "client-source".into(),
                administration_namespace: namespace.clone(),
                destination_namespaces: vec![namespace.clone()],
                profile_revision: 1,
                ownership_policy_revision: "policy".into(),
                deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                ..Default::default()
            }),
        },
    )
    .await
    .unwrap();
    let revision = registration["registration"]["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut writer = client::connect(&endpoint, Some("source-writer"), 30)
        .await
        .unwrap();
    let mut reader = client::connect(&endpoint, Some("source-reader"), 30)
        .await
        .unwrap();
    let inventory = serde_json::to_vec(&json!({"profile":"supplied-native-v1", "profile_revision":1, "registration":"client-source", "source_version":"opaque.RC", "partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]}, "coverage":"PARTIAL", "resources":[{"key":{"category":"model","key":"receipt"},"file":"native/model.yaml","pointer":{"kind":"native","raw":"#/receipt"}}], "relations":[{"owner":{"category":"model","key":"receipt"},"occurrence":"b","role":"observes","position":9,"target":{"kind":"opaque","raw":"same"},"file":"native/model.yaml"},{"owner":{"category":"model","key":"receipt"},"occurrence":"a","role":"observes","position":2,"target":{"kind":"opaque","raw":"same"},"file":"native/model.yaml"}]})).unwrap();
    let bytes = b"# exact original bytes\nreceipt: supplied\n";
    let bundle = validate_bundle(
        &inventory,
        &[SuppliedFile {
            logical_name: "native/model.yaml",
            media_type: "application/yaml",
            bytes,
        }],
    )
    .unwrap();
    let snapshot = source::accept_source_snapshot(
        &mut writer,
        source::snapshot_request("client-source".into(), revision.clone(), &bundle),
    )
    .await
    .unwrap();
    let snapshot_id = snapshot["snapshot"]["id"].as_str().unwrap().to_owned();
    assert!(snapshot["snapshot"].get("acceptedAtUnixMs").is_none());
    let recovered = source::get_source_snapshot(
        &mut reader,
        pb::GetSourceSnapshotRequest {
            registration_id: "client-source".into(),
            snapshot_id: snapshot_id.clone(),
            include_bytes: true,
        },
    )
    .await
    .unwrap();
    let typed: pb::GetSourceSnapshotResponse =
        source::parse_request("GetSourceSnapshotResponse", &recovered).unwrap();
    assert_eq!(typed.snapshot.unwrap().files[0].content, bytes);
    let key = pb::SourceResourceKey {
        registration_id: "client-source".into(),
        category: "model".into(),
        key: "receipt".into(),
    };
    let owner = pb::EntityRef {
        kind: pb::EntityKind::ReadModel as i32,
        id: Some(pb::Id {
            namespace: namespace.clone(),
            slug: "receipt".into(),
            version: 1,
        }),
    };
    let mapping = Box::pin(source::create_source_mapping(
        &mut writer,
        pb::CreateSourceMappingRequest {
            registration_revision: revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "receipt-title".into(),
                source: Some(key.clone()),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(owner.clone())),
                }),
                ownership: Some(pb::SourceFieldOwnership {
                    policy: Some(pb::source_field_ownership::Policy::Fields(
                        pb::SourceOwnedFields {
                            fields: vec![pb::SourceOwnedField::Title as i32],
                        },
                    )),
                }),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(pb::SourceObservedOwner {
                        owner: Some(owner.clone()),
                        uid: native.system.as_ref().unwrap().uid.clone(),
                        etag: original.etag,
                    }),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        },
    ))
    .await
    .unwrap();
    assert_eq!(mapping["mapping"]["source"]["key"], "receipt");
    let payload = source::native_payload(&mut writer, key, format!("apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: ReadModel\nmetadata: {{namespace: {namespace}, name: receipt}}\nspec: {{title: Updated, doc: Source document must not take local ownership}}\n").as_bytes()).await.unwrap();
    let preview = source::preview_source_run(
        &mut writer,
        pb::PreviewSourceRunRequest {
            registration_id: "client-source".into(),
            registration_revision: revision,
            snapshot_id: snapshot_id.clone(),
            run_id: "client-run".into(),
            native_payloads: vec![payload],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(preview["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    let overlay = source::get_source_overlay(
        &mut reader,
        pb::GetSourceOverlayRequest {
            registration_id: "client-source".into(),
            snapshot_id,
            page_size: 100,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let occurrences = overlay["overlay"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["occurrenceKey"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(occurrences, vec!["b", "a"]);
    let apply = pb::ApplySourceRunRequest {
        registration_id: "client-source".into(),
        run_id: "client-run".into(),
        if_match: preview["run"]["revision"].as_str().unwrap().into(),
        preview_digest: preview["run"]["previewDigest"].as_str().unwrap().into(),
    };
    assert_eq!(
        source::apply_source_run(&mut reader, apply.clone())
            .await
            .unwrap_err()
            .downcast_ref::<tonic::Status>()
            .unwrap()
            .code(),
        tonic::Code::PermissionDenied
    );
    let applied = source::apply_source_run(&mut writer, apply.clone())
        .await
        .unwrap();
    assert_eq!(applied["run"]["status"], "SOURCE_RUN_STATUS_APPLIED");
    let replay = source::apply_source_run(&mut writer, apply).await.unwrap();
    assert_eq!(replay["run"]["status"], "SOURCE_RUN_STATUS_APPLIED");
    let current = reader
        .get_entity(pb::GetEntityRequest {
            kind: owner.kind,
            id: owner.id,
        })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(
        current.system.as_ref().unwrap().uid,
        native.system.as_ref().unwrap().uid
    );
    let Some(pb::entity::Kind::ReadModel(current)) = current.kind else {
        panic!("read model");
    };
    assert_eq!(current.title, "Updated");
    assert_eq!(current.doc, "Local rationale retained");
    let results = source::list_source_run_results(
        &mut reader,
        pb::ListSourceRunResultsRequest {
            registration_id: "client-source".into(),
            run_id: "client-run".into(),
            page_size: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(results["runRevision"].as_str().is_some());
    assert!(results["previewDigest"].as_str().is_some());
    let mut hidden = client::connect(&endpoint, Some("source-hidden"), 30)
        .await
        .unwrap();
    let error = source::get_source_registration(
        &mut hidden,
        pb::GetSourceRegistrationRequest {
            registration_id: "client-source".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<tonic::Status>().unwrap().code(),
        tonic::Code::NotFound | tonic::Code::PermissionDenied
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_native_payload_sanitizes_only_local_manifest_conversion_failures() {
    let endpoint = server().await;
    let mut admin = client::connect(&endpoint, Some("source-admin"), 30)
        .await
        .unwrap();
    let namespace = admin
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "source-surfaces".into(),
            parent: "source-owner".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let mut writer = client::connect(&endpoint, Some("source-writer"), 30)
        .await
        .unwrap();
    let key = pb::SourceResourceKey {
        registration_id: "r".into(),
        category: "model".into(),
        key: "receipt".into(),
    };
    for spec in [
        "{key: PRIVATE_SECRET_SENTINEL}",
        "{'/private/PRIVATE_SOURCE_PATH': hidden}",
    ] {
        let yaml = format!(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: ReadModel\nmetadata: {{namespace: {namespace}, name: receipt}}\nspec: {spec}\n"
        );
        let error = source::native_payload(&mut writer, key.clone(), yaml.as_bytes())
            .await
            .unwrap_err();
        assert!(error
            .downcast_ref::<trogon_atlas_client::failure::InvalidInput>()
            .is_some());
        let rendered = format!("{error:#}");
        assert!(
            !rendered.contains("PRIVATE_SECRET_SENTINEL"),
            "supplied content leaked: {rendered}"
        );
        assert!(
            !rendered.contains("PRIVATE_SOURCE_PATH"),
            "supplied field leaked: {rendered}"
        );
    }
}

fn money_value(
    types: &trogon_atlas_core::transcode::TranscodePool,
    entity: &pb::Entity,
) -> serde_json::Value {
    let Some(pb::entity::Kind::Event(event)) = &entity.kind else {
        panic!("tenant payload must remain an Event");
    };
    let schema = event.schema.as_ref().unwrap();
    assert_eq!(schema.type_url, "type.googleapis.com/acme.money.v1.Money");
    let value = prost_reflect::DynamicMessage::decode(
        types.message_descriptor("acme.money.v1.Money").unwrap(),
        schema.value.as_slice(),
    )
    .unwrap();
    types
        .message_json_value("acme.money.v1.Money", &value)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_source_tenant_any_survives_mapping_and_operation_json_roundtrips() {
    let endpoint = server().await;
    let mut admin = client::connect(&endpoint, Some("source-admin"), 30)
        .await
        .unwrap();
    let namespace = admin
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "source-tenant-types".into(),
            parent: "source-owner".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let id = |slug: &str| pb::Id {
        namespace: namespace.clone(),
        slug: slug.into(),
        version: 1,
    };
    admin.put_entity(pb::PutEntityRequest {
        entity: Some(pb::Entity { kind: Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
            id: Some(id("acme.money")), title: "Money".into(),
            files: vec![pb::ProtoSourceFile {
                path: "acme/money/v1/money.proto".into(),
                content: "syntax = \"proto3\";\npackage acme.money.v1;\nmessage Money { int64 cents = 1; string currency = 2; map<string, string> labels = 3; }\n".into(),
            }], ..Default::default()
        })), ..Default::default() }), create_only: true, ..Default::default()
    }).await.unwrap();
    let key = pb::SourceResourceKey {
        registration_id: "tenant-source".into(),
        category: "event".into(),
        key: "priced".into(),
    };
    let manifest = |cents: &str, labels: &str| {
        format!(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {{namespace: {namespace}, name: priced}}\nspec:\n  title: Priced\n  doc: Locally retained rationale\n  schema:\n    '@type': type.googleapis.com/acme.money.v1.Money\n    cents: '{cents}'\n    currency: USD\n    labels: {labels}\n"
        )
    };
    let initial = source::native_payload(
        &mut admin,
        key.clone(),
        manifest("9007199254740993", "{z: last, a: first}").as_bytes(),
    )
    .await
    .unwrap();
    let original = admin
        .put_entity(pb::PutEntityRequest {
            entity: initial.entity,
            create_only: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let observed = pb::SourceObservedOwner {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("priced")),
        }),
        uid: original
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
            .clone(),
        etag: original.etag,
    };
    let registration = source::create_source_registration(
        &mut admin,
        pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: "tenant-source".into(),
                administration_namespace: namespace.clone(),
                destination_namespaces: vec![namespace.clone()],
                profile_revision: 1,
                ownership_policy_revision: "money-policy".into(),
                deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                ..Default::default()
            }),
        },
    )
    .await
    .unwrap();
    let registration_revision = registration["registration"]["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    Box::pin(source::create_source_mapping(
        &mut admin,
        pb::CreateSourceMappingRequest {
            registration_revision: registration_revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "priced-schema".into(),
                source: Some(key.clone()),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(
                        observed.owner.clone().unwrap(),
                    )),
                }),
                ownership: Some(pb::SourceFieldOwnership {
                    policy: Some(pb::source_field_ownership::Policy::Fields(
                        pb::SourceOwnedFields {
                            fields: vec![pb::SourceOwnedField::Schema as i32],
                        },
                    )),
                }),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(observed.clone()),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        },
    ))
    .await
    .unwrap();
    let supplied = manifest("9007199254740994", "{z: last, a: first}");
    let incoming = source::native_payload(&mut admin, key.clone(), supplied.as_bytes())
        .await
        .unwrap();
    let mut scope = trogon_atlas_client::tenant_types::TypeScope::default();
    scope.add_namespace(namespace.clone());
    let types = trogon_atlas_client::tenant_types::transcode_pool(&mut admin, &scope)
        .await
        .unwrap();
    let expected =
        json!({"cents":"9007199254740994","currency":"USD","labels":{"a":"first","z":"last"}});
    assert_eq!(
        money_value(&types, incoming.entity.as_ref().unwrap()),
        expected
    );
    let inventory = serde_json::to_vec(&json!({
        "profile":"supplied-native-v1","profile_revision":1,"registration":"tenant-source","source_version":"money-revision",
        "partition":{"categories":["event"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL",
        "resources":[{"key":{"category":"event","key":"priced"},"file":"native/event.yaml","pointer":{"kind":"native","raw":"#/priced"}}],"relations":[]
    })).unwrap();
    let bundle = validate_bundle(
        &inventory,
        &[SuppliedFile {
            logical_name: "native/event.yaml",
            media_type: "application/yaml",
            bytes: supplied.as_bytes(),
        }],
    )
    .unwrap();
    let snapshot = source::accept_source_snapshot(
        &mut admin,
        source::snapshot_request(
            "tenant-source".into(),
            registration_revision.clone(),
            &bundle,
        ),
    )
    .await
    .unwrap();
    let snapshot_id = snapshot["snapshot"]["id"].as_str().unwrap().to_owned();
    let preview = source::preview_source_run(
        &mut admin,
        pb::PreviewSourceRunRequest {
            registration_id: "tenant-source".into(),
            registration_revision: registration_revision.clone(),
            snapshot_id: snapshot_id.clone(),
            run_id: "tenant-money-run".into(),
            native_payloads: vec![incoming],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(preview["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    let operation_request = pb::ListSourceRunOperationsRequest {
        registration_id: "tenant-source".into(),
        run_id: "tenant-money-run".into(),
        page_size: 100,
        ..Default::default()
    };
    let operations = source::list_source_run_operations(&mut admin, operation_request.clone())
        .await
        .unwrap();
    let typed_operations: pb::ListSourceRunOperationsResponse = types
        .message_from_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListSourceRunOperationsResponse",
            &operations,
        )
        .unwrap();
    let planned = typed_operations
        .entries
        .iter()
        .find_map(|entry| match &entry.entry {
            Some(pb::source_run_operation_entry::Entry::Operation(operation)) => {
                match &operation.operation {
                    Some(pb::source_native_operation::Operation::Put(put)) => put.entity.as_ref(),
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("a typed schema update must remain in the operation page");
    assert_eq!(money_value(&types, planned), expected);
    assert_eq!(
        typed_operations.preview_digest,
        preview["run"]["previewDigest"].as_str().unwrap()
    );
    let applied = source::apply_source_run(
        &mut admin,
        pb::ApplySourceRunRequest {
            registration_id: "tenant-source".into(),
            run_id: "tenant-money-run".into(),
            if_match: preview["run"]["revision"].as_str().unwrap().into(),
            preview_digest: preview["run"]["previewDigest"].as_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(applied["run"]["status"], "SOURCE_RUN_STATUS_APPLIED");
    let mapping_request = pb::GetSourceMappingRequest {
        registration_id: "tenant-source".into(),
        mapping_id: "priced-schema".into(),
    };
    let mapping = source::get_source_mapping(&mut admin, mapping_request.clone())
        .await
        .unwrap();
    let typed_mapping: pb::GetSourceMappingResponse = types
        .message_from_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.GetSourceMappingResponse",
            &mapping,
        )
        .unwrap();
    let base = typed_mapping.mapping.unwrap().base.unwrap();
    assert_eq!(
        money_value(&types, base.source_value.as_ref().unwrap()),
        expected
    );
    assert_eq!(
        money_value(&types, base.native_value.as_ref().unwrap()),
        expected
    );
    assert_eq!(base.observed.unwrap().uid, observed.uid);
    let equivalent = source::native_payload(
        &mut admin,
        key,
        manifest("9007199254740994", "{a: first, z: last}").as_bytes(),
    )
    .await
    .unwrap();
    let same = source::preview_source_run(
        &mut admin,
        pb::PreviewSourceRunRequest {
            registration_id: "tenant-source".into(),
            registration_revision,
            snapshot_id,
            run_id: "tenant-money-map-order".into(),
            native_payloads: vec![equivalent],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(same["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    let same_results = source::list_source_run_results(
        &mut admin,
        pb::ListSourceRunResultsRequest {
            registration_id: "tenant-source".into(),
            run_id: "tenant-money-map-order".into(),
            page_size: 100,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(same_results["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["disposition"] == "SOURCE_RESULT_DISPOSITION_UNCHANGED"));
    let mut current = admin
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("priced")),
        })
        .await
        .unwrap()
        .into_inner();
    let Some(pb::entity::Kind::Event(event)) = &mut current.entity.as_mut().unwrap().kind else {
        panic!("Event");
    };
    event.schema = None;
    admin
        .put_entity(pb::PutEntityRequest {
            entity: current.entity,
            if_match: current.etag,
            ..Default::default()
        })
        .await
        .unwrap();
    admin
        .delete_entity(pb::DeleteEntityRequest {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(id("acme.money")),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        })
        .await
        .unwrap();
    let without_historical_type =
        trogon_atlas_client::tenant_types::transcode_pool(&mut admin, &scope)
            .await
            .unwrap();
    assert!(without_historical_type
        .message_descriptor("acme.money.v1.Money")
        .is_err());
    let historical = admin
        .get_source_mapping(mapping_request.clone())
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap()
        .base
        .unwrap();
    let Some(pb::entity::Kind::Event(event)) = historical.native_value.unwrap().kind else {
        panic!("historical Event");
    };
    assert_eq!(
        event.schema.unwrap().type_url,
        "type.googleapis.com/acme.money.v1.Money"
    );
    assert!(
        source::get_source_mapping(&mut admin, mapping_request)
            .await
            .is_err(),
        "unavailable historical descriptors must not yield guessed or opaque successful JSON"
    );
    assert!(
        source::list_source_run_operations(&mut admin, operation_request)
            .await
            .is_err(),
        "operation rendering must surface unavailable historical types"
    );
}
