#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{future::Future, sync::Arc};

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tonic::{transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{BearerAuth, RegistryFile, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::NatsStore;

const ADMIN: &str = "source-admin-token-aaaaaaaa";
const READER: &str = "source-reader-token-bbbbbbbb";
const WRITER: &str = "source-writer-token-cccccccc";
const SCOPED: &str = "source-scoped-token-dddddddd";
struct Server {
    client: EventModelServiceClient<Channel>,
    task: tokio::task::JoinHandle<()>,
    service: Arc<EventModelServiceImpl>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn server(store: impl trogon_atlas_store::Store + 'static) -> impl Future<Output = Server> {
    server_with_mirror(store, None)
}
fn server_with_mirror(
    store: impl trogon_atlas_store::Store + 'static,
    mirror: Option<Arc<trogon_atlas_server::git_mirror::GitMirror>>,
) -> impl Future<Output = Server> {
    Box::pin(async move {
        let registry: RegistryFile = toml::from_str(&format!(
            r#"
[principals.admin]
role="admin"
tokens=["{ADMIN}"]
[principals.reader]
role="reader"
tokens=["{READER}"]
[principals.writer]
role="writer"
tokens=["{WRITER}"]
[principals.scoped]
role="reader"
parent="visible-owner"
tokens=["{SCOPED}"]
"#
        ))
        .unwrap();
        let mut implementation =
            EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default())
                .unwrap()
                .with_snapshot_cap(0);
        if let Some(mirror) = mirror {
            implementation = implementation.with_git_mirror(mirror);
        }
        let implementation = Arc::new(implementation);
        let grpc = EventModelServiceServer::from_arc(implementation.clone())
            .max_decoding_message_size(36 * 1024 * 1024)
            .max_encoding_message_size(36 * 1024 * 1024);
        let service = trogon_atlas_server::mcp::data_plane(
            grpc,
            BearerAuth::new(Arc::new(
                TokenRegistry::from_file_contents(registry).unwrap(),
            )),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let client = EventModelServiceClient::connect(format!("http://{addr}"))
            .await
            .unwrap()
            .max_decoding_message_size(36 * 1024 * 1024)
            .max_encoding_message_size(36 * 1024 * 1024);
        Server {
            client,
            task,
            service: implementation,
        }
    })
}

fn req<T>(token: &str, value: T, branch: Option<&str>) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    request
}
async fn namespace(
    client: &mut EventModelServiceClient<Channel>,
    name: &str,
    parent: &str,
) -> String {
    client
        .register_namespace(req(
            ADMIN,
            pb::RegisterNamespaceRequest {
                name: name.into(),
                parent: parent.into(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id
}
async fn registration(
    client: &mut EventModelServiceClient<Channel>,
    id: &str,
    namespace: &str,
    budgets: Option<pb::SourceRunBudgets>,
    retention: Option<pb::SourceRetentionPolicy>,
) -> pb::SourceRegistration {
    client
        .create_source_registration(req(
            ADMIN,
            pb::CreateSourceRegistrationRequest {
                registration: Some(pb::SourceRegistration {
                    id: id.into(),
                    administration_namespace: namespace.into(),
                    destination_namespaces: vec![namespace.into()],
                    profile_revision: 1,
                    ownership_policy_revision: "policy-v1".into(),
                    deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                    budgets,
                    retention,
                    ..Default::default()
                }),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap()
}
fn inventory(registration: &str, version: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"profile":"supplied-native-v1","profile_revision":1,"registration":registration,"source_version":version,"partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL","resources":[{"key":{"category":"model","key":"read-model"},"file":"native/model","pointer":{"kind":"native","raw":"opaque"}}],"relations":[]})).unwrap()
}
fn source(registration: &str) -> pb::SourceResourceKey {
    pb::SourceResourceKey {
        registration_id: registration.into(),
        category: "model".into(),
        key: "read-model".into(),
    }
}
fn entity(namespace: &str, title: &str, doc: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(pb::Id {
                namespace: namespace.into(),
                slug: "read-model".into(),
                version: 1,
            }),
            title: title.into(),
            doc: doc.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}
struct Seed {
    registration: pb::SourceRegistration,
    snapshot: pb::SourceSnapshot,
    mapping: pb::SourceMapping,
    entity: pb::Entity,
    etag: String,
    namespace: String,
}
fn seed<'a>(
    client: &'a mut EventModelServiceClient<Channel>,
    branch: Option<&'a str>,
    fields: Vec<pb::SourceOwnedField>,
) -> impl Future<Output = Seed> + 'a {
    Box::pin(async move {
        let namespace = namespace(client, "source-native", "visible-owner").await;
        if let Some(branch) = branch {
            client
                .create_branch(req(
                    ADMIN,
                    pb::CreateBranchRequest {
                        name: branch.into(),
                        ..Default::default()
                    },
                    None,
                ))
                .await
                .unwrap();
        }
        let put = client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(entity(&namespace, "before", "local doc")),
                    ..Default::default()
                },
                branch,
            ))
            .await
            .unwrap()
            .into_inner();
        let entity = put.entity.unwrap();
        let registration = registration(client, "native-source", &namespace, None, None).await;
        let snapshot = client
            .accept_source_snapshot(req(
                ADMIN,
                pb::AcceptSourceSnapshotRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    inventory_bytes: inventory(&registration.id, "opaque-A"),
                    files: vec![pb::SourceFile {
                        logical_name: "native/model".into(),
                        media_type: "application/octet-stream".into(),
                        content: b"opaque supplied bytes".to_vec(),
                        content_digest: String::new(),
                    }],
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap();
        let observed = pb::SourceObservedOwner {
            owner: Some(pb::EntityRef {
                kind: pb::EntityKind::ReadModel as i32,
                id: trogon_atlas_core::refs::entity_id(&entity).cloned(),
            }),
            uid: entity.system.as_ref().unwrap().uid.clone(),
            etag: put.etag.clone(),
        };
        let mapping = client
            .create_source_mapping(req(
                ADMIN,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: "native-mapping".into(),
                        source: Some(source(&registration.id)),
                        selection: Some(pb::SourceSelection {
                            selection: Some(pb::source_selection::Selection::Entity(
                                observed.owner.clone().unwrap(),
                            )),
                        }),
                        ownership: Some(pb::SourceFieldOwnership {
                            policy: Some(pb::source_field_ownership::Policy::Fields(
                                pb::SourceOwnedFields {
                                    fields: fields.iter().map(|f| *f as i32).collect(),
                                },
                            )),
                        }),
                        adoption: Some(pb::SourceAdoption {
                            observed: Some(observed),
                            authorized_adoption: true,
                        }),
                        branch: branch.map(str::to_owned),
                        ..Default::default()
                    }),
                },
                branch,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        Seed {
            registration,
            snapshot,
            mapping,
            entity,
            etag: put.etag,
            namespace,
        }
    })
}

async fn preview(
    client: &mut EventModelServiceClient<Channel>,
    seed: &Seed,
    id: &str,
    value: pb::Entity,
    branch: Option<&str>,
) -> pb::ReconciliationRun {
    client
        .preview_source_run(req(
            ADMIN,
            pb::PreviewSourceRunRequest {
                registration_id: seed.registration.id.clone(),
                registration_revision: seed.registration.revision.clone(),
                snapshot_id: seed.snapshot.id.clone(),
                run_id: id.into(),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(source(&seed.registration.id)),
                    entity: Some(value),
                }],
                ..Default::default()
            },
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap()
}
async fn get_native(
    client: &mut EventModelServiceClient<Channel>,
    seed: &Seed,
    branch: Option<&str>,
) -> pb::GetEntityResponse {
    client
        .get_entity(req(
            ADMIN,
            pb::GetEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: trogon_atlas_core::refs::entity_id(&seed.entity).cloned(),
            },
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
fn title(value: &pb::Entity) -> &str {
    let Some(pb::entity::Kind::ReadModel(value)) = &value.kind else {
        panic!("read model")
    };
    &value.title
}
fn doc(value: &pb::Entity) -> &str {
    let Some(pb::entity::Kind::ReadModel(value)) = &value.kind else {
        panic!("read model")
    };
    &value.doc
}

#[tokio::test]
async fn hidden_only_registration_inventory_has_no_observable_pagination_continuation() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let query = pb::ListSourceRegistrationsRequest {
        page_size: 1,
        ..Default::default()
    };
    let empty = server
        .client
        .list_source_registrations(req(SCOPED, query.clone(), None))
        .await
        .unwrap()
        .into_inner();
    let hidden = namespace(&mut server.client, "hidden", "hidden-owner").await;
    registration(&mut server.client, "hidden-one", &hidden, None, None).await;
    registration(&mut server.client, "hidden-two", &hidden, None, None).await;
    let filtered = server
        .client
        .list_source_registrations(req(SCOPED, query, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        filtered, empty,
        "hidden registration existence must not affect a reader's empty page or continuation"
    );
}

#[tokio::test]
async fn stale_preview_refusal_has_a_definitive_recoverable_noncommit_outcome() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let run = preview(
        &mut server.client,
        &seed,
        "stale",
        entity(&seed.namespace, "source change", "ignored"),
        None,
    )
    .await;
    let edited = server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(entity(&seed.namespace, "later native", "local doc")),
                if_match: seed.etag.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let refused = server
        .client
        .apply_source_run(req(
            ADMIN,
            pb::ApplySourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision,
                preview_digest: run.preview_digest,
            },
            None,
        ))
        .await
        .expect_err("stale native guard must refuse publication");
    let retained = server
        .client
        .get_source_run(req(
            ADMIN,
            pb::GetSourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    let recovered = server
        .client
        .recover_source_run(req(
            ADMIN,
            pb::RecoverSourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id,
                if_match: retained.revision,
            },
            None,
        ))
        .await
        .expect("a refusal before native publication must settle definitively")
        .into_inner()
        .run
        .unwrap();
    assert_eq!(recovered.status, pb::SourceRunStatus::Rejected as i32);
    assert!(!recovered.recovery_required);
    assert!(
        matches!(
            refused.code(),
            Code::FailedPrecondition | Code::InvalidArgument
        ),
        "{refused:?}"
    );
    assert_eq!(
        get_native(&mut server.client, &seed, None).await.etag,
        edited.etag
    );
}

async fn accept(
    client: &mut EventModelServiceClient<Channel>,
    registration: &pb::SourceRegistration,
    version: &str,
    extra: bool,
) -> pb::SourceSnapshot {
    let mut bytes: serde_json::Value =
        serde_json::from_slice(&inventory(&registration.id, version)).unwrap();
    if extra {
        bytes["resources"].as_array_mut().unwrap().push(serde_json::json!({"key":{"category":"model","key":"uncaptured"},"file":"native/model","pointer":{"kind":"opaque","raw":"inert uncaptured declaration"}}));
    }
    client
        .accept_source_snapshot(req(
            WRITER,
            pb::AcceptSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                inventory_bytes: serde_json::to_vec(&bytes).unwrap(),
                files: vec![pb::SourceFile {
                    logical_name: "native/model".into(),
                    media_type: "application/octet-stream".into(),
                    content: b"opaque supplied bytes".to_vec(),
                    content_digest: String::new(),
                }],
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap()
}
async fn apply(
    client: &mut EventModelServiceClient<Channel>,
    run: &pb::ReconciliationRun,
) -> pb::ReconciliationRun {
    client
        .apply_source_run(req(
            if run.principal == "admin" {
                ADMIN
            } else {
                WRITER
            },
            pb::ApplySourceRunRequest {
                registration_id: run.registration_id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision.clone(),
                preview_digest: run.preview_digest.clone(),
            },
            run.branch.as_deref(),
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap()
}
async fn reports(
    client: &mut EventModelServiceClient<Channel>,
    run: &pb::ReconciliationRun,
    page_size: u32,
) -> (Vec<pb::SourceRunResult>, pb::SourceFidelityReport) {
    let mut cursor = String::new();
    let mut results = Vec::new();
    let mut report = None;
    for _ in 0..100 {
        let page = client
            .list_source_run_results(req(
                READER,
                pb::ListSourceRunResultsRequest {
                    registration_id: run.registration_id.clone(),
                    run_id: run.id.clone(),
                    page_size,
                    page_token: cursor,
                },
                run.branch.as_deref(),
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(page.run_revision, run.revision);
        assert_eq!(page.preview_digest, run.preview_digest);
        results.extend(page.results);
        let fidelity = page.fidelity.unwrap();
        if let Some(report) = report.as_mut() {
            let report: &mut pb::SourceFidelityReport = report;
            report.records.extend(fidelity.records);
            report.checked_owners.extend(fidelity.checked_owners);
        } else {
            report = Some(fidelity);
        }
        cursor = page.next_page_token;
        if cursor.is_empty() {
            return (results, report.unwrap());
        }
    }
    panic!("report pagination did not terminate")
}

fn uncaptured_resource_case(subset: bool) -> impl Future<Output = ()> {
    Box::pin(async move {
        let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
        let mut seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
        seed.snapshot = accept(
            &mut server.client,
            &seed.registration,
            "with-uncaptured",
            subset,
        )
        .await;
        let run = server
            .client
            .preview_source_run(req(
                WRITER,
                pb::PreviewSourceRunRequest {
                    registration_id: seed.registration.id.clone(),
                    registration_revision: seed.registration.revision.clone(),
                    snapshot_id: seed.snapshot.id.clone(),
                    run_id: "uncaptured".into(),
                    native_payloads: if subset {
                        vec![pb::SourceNativePayload {
                            source: Some(source(&seed.registration.id)),
                            entity: Some(entity(&seed.namespace, "source update", "ignored")),
                        }]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        let (results, fidelity) = reports(&mut server.client, &run, 1).await;
        let key = if subset { "uncaptured" } else { "read-model" };
        assert!(
            results
                .iter()
                .any(|r| r.source.as_ref().is_some_and(|s| s.key == key)
                    && r.disposition == pb::SourceResultDisposition::Deferred as i32),
            "uncaptured resource must remain visible in the public report: {results:?}"
        );
        assert!(fidelity
            .records
            .iter()
            .any(|r| r.source.as_ref().is_some_and(|s| s.key == key)
                && r.disposition == pb::SourceFidelityDisposition::RetainedOverlayOnly as i32
                && !r.native_reapply_supported));
        assert!(fidelity.incomplete);
        assert!(!fidelity.native_reapply_supported);
        assert_eq!(
            title(
                get_native(&mut server.client, &seed, None)
                    .await
                    .entity
                    .as_ref()
                    .unwrap()
            ),
            "before"
        );
        assert_eq!(run.operation_count, u32::from(subset));
    })
}

#[tokio::test]
async fn empty_native_payloads_report_every_uncaptured_inventory_resource() {
    uncaptured_resource_case(false).await;
}

#[tokio::test]
async fn subset_native_payloads_report_every_uncaptured_inventory_resource() {
    uncaptured_resource_case(true).await;
}

#[tokio::test]
async fn mixed_local_title_drift_and_source_doc_update_preserves_unit_bases_and_expiry() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let seed = seed(
        &mut server.client,
        None,
        vec![pb::SourceOwnedField::Title, pb::SourceOwnedField::Doc],
    )
    .await;
    let original = seed.mapping.base.as_ref().unwrap();
    let native = server
        .client
        .put_entity(req(
            WRITER,
            pb::PutEntityRequest {
                entity: Some(entity(&seed.namespace, "locally edited title", "local doc")),
                if_match: seed.etag.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let run = preview(
        &mut server.client,
        &seed,
        "mixed-drift",
        entity(&seed.namespace, "before", "source doc update"),
        None,
    )
    .await;
    let (results, _) = reports(&mut server.client, &run, 1).await;
    assert!(results
        .iter()
        .any(|r| r.field == Some(pb::SourceOwnedField::Title as i32)
            && r.disposition == pb::SourceResultDisposition::LocalDrift as i32));
    assert!(results
        .iter()
        .any(|r| r.field == Some(pb::SourceOwnedField::Doc as i32)
            && r.disposition == pb::SourceResultDisposition::Update as i32));
    let settled = apply(&mut server.client, &run).await;
    assert_eq!(settled.status, pb::SourceRunStatus::Applied as i32);
    let persisted = get_native(&mut server.client, &seed, None).await;
    assert_ne!(persisted.etag, native.etag);
    assert_eq!(
        title(persisted.entity.as_ref().unwrap()),
        "locally edited title"
    );
    assert_eq!(doc(persisted.entity.as_ref().unwrap()), "source doc update");
    let mapping = server
        .client
        .get_source_mapping(req(
            READER,
            pb::GetSourceMappingRequest {
                registration_id: seed.registration.id.clone(),
                mapping_id: seed.mapping.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    let base = mapping.base.unwrap();
    assert_eq!(title(base.source_value.as_ref().unwrap()), "before");
    assert_eq!(title(base.native_value.as_ref().unwrap()), "before");
    assert_eq!(
        doc(base.source_value.as_ref().unwrap()),
        "source doc update"
    );
    assert_eq!(
        doc(base.native_value.as_ref().unwrap()),
        "source doc update"
    );
    assert_eq!(base.accepted_at_unix_ms, original.accepted_at_unix_ms);
    assert_eq!(base.expires_at_unix_ms, original.expires_at_unix_ms);
    let retry = preview(
        &mut server.client,
        &seed,
        "mixed-drift-recheck",
        entity(&seed.namespace, "before", "source doc update"),
        None,
    )
    .await;
    let (results, _) = reports(&mut server.client, &retry, 1).await;
    assert!(results
        .iter()
        .any(|r| r.field == Some(pb::SourceOwnedField::Title as i32)
            && r.disposition == pb::SourceResultDisposition::LocalDrift as i32));
    assert_eq!(retry.operation_count, 0);
}

#[tokio::test]
async fn raw_expiry_keeps_logical_overlay_without_claiming_original_bytes() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let namespace = namespace(&mut server.client, "raw-retention", "visible-owner").await;
    let retention = pb::SourceRetentionPolicy {
        raw_bytes_seconds: 1,
        overlay_seconds: 30,
        ..trogon_atlas_core::source_integration::profile_retention()
    };
    let registration = registration(
        &mut server.client,
        "raw-expiry",
        &namespace,
        None,
        Some(retention),
    )
    .await;
    let snapshot = accept(&mut server.client, &registration, "opaque-A", false).await;
    let run = server
        .client
        .preview_source_run(req(
            WRITER,
            pb::PreviewSourceRunRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                snapshot_id: snapshot.id.clone(),
                run_id: "raw-report".into(),
                overlay_only: true,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let expired = server
        .client
        .get_source_snapshot(req(
            READER,
            pb::GetSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                snapshot_id: snapshot.id.clone(),
                include_bytes: true,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert!(
        !expired.bytes_available,
        "raw byte clock must be enforced independently of snapshot lifetime"
    );
    assert_eq!(expired.files, []);
    assert_eq!(expired.inventory_bytes, Vec::<u8>::new());
    let overlay = server
        .client
        .get_source_overlay(req(
            READER,
            pb::GetSourceOverlayRequest {
                registration_id: registration.id.clone(),
                snapshot_id: snapshot.id.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .overlay
        .unwrap();
    assert_eq!(overlay.entries.len(), 1);
    assert_eq!(overlay.entries[0].pointer.as_ref().unwrap().raw, "opaque");
    let (_, fidelity) = reports(&mut server.client, &run, 1).await;
    assert!(!fidelity.original_bytes_recoverable);
    assert!(fidelity
        .records
        .iter()
        .all(|r| !r.original_bytes_recoverable));
    let replay = accept(&mut server.client, &registration, "opaque-A", false).await;
    assert_eq!(replay.id, snapshot.id);
    assert!(!replay.bytes_available);
    let mut changed = inventory(&registration.id, "opaque-A");
    changed.push(b' ');
    let conflict = server
        .client
        .accept_source_snapshot(req(
            WRITER,
            pb::AcceptSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                inventory_bytes: changed,
                files: vec![pb::SourceFile {
                    logical_name: "native/model".into(),
                    media_type: "application/octet-stream".into(),
                    content: b"opaque supplied bytes".to_vec(),
                    content_digest: String::new(),
                }],
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(conflict.code(), Code::AlreadyExists);
}

#[tokio::test]
async fn raw_expiry_preserves_native_reapply_fidelity() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let run = preview(
        &mut server.client,
        &seed,
        "raw-native-fidelity",
        entity(&seed.namespace, "source title", "local doc"),
        None,
    )
    .await;
    let (_, before) = reports(&mut server.client, &run, 1).await;
    assert!(before.native_reapply_supported);
    assert!(before.original_bytes_recoverable);
    assert_ne!(before.records, []);
    assert!(before
        .records
        .iter()
        .all(|record| { record.native_reapply_supported && record.original_bytes_recoverable }));
    let native_flags: Vec<_> = before
        .records
        .iter()
        .map(|record| record.native_reapply_supported)
        .collect();
    let mut registration = seed.registration.clone();
    registration.retention.as_mut().unwrap().raw_bytes_seconds = 1;
    server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(registration),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap();
    wait_backend_expiry(
        seed.snapshot
            .accepted_at_unix_ms
            .unwrap_or(seed.snapshot.created_at_unix_ms)
            + 1000,
    )
    .await;
    let expired = server
        .client
        .get_source_snapshot(req(
            READER,
            pb::GetSourceSnapshotRequest {
                registration_id: seed.registration.id.clone(),
                snapshot_id: seed.snapshot.id.clone(),
                include_bytes: true,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert!(!expired.bytes_available);
    assert_eq!(expired.inventory_bytes, Vec::<u8>::new());
    assert_eq!(expired.files, []);
    let (_, after) = reports(&mut server.client, &run, 1).await;
    assert!(!after.original_bytes_recoverable);
    assert!(after
        .records
        .iter()
        .all(|record| !record.original_bytes_recoverable));
    assert_eq!(
        after.native_reapply_supported, before.native_reapply_supported,
        "raw byte expiry must preserve same-store native reapply fidelity"
    );
    assert_eq!(
        after
            .records
            .iter()
            .map(|record| record.native_reapply_supported)
            .collect::<Vec<_>>(),
        native_flags,
        "raw byte expiry must preserve per-record native reapply fidelity"
    );
}

#[derive(Clone, PartialEq, prost::Message)]
struct RetainedRunAudit {
    #[prost(message, optional, tag = "1")]
    run: Option<pb::ReconciliationRun>,
    #[prost(message, repeated, tag = "2")]
    payloads: Vec<pb::SourceNativePayload>,
    #[prost(message, repeated, tag = "3")]
    values: Vec<pb::SourceCommittedValue>,
}

// The service omits retained audit payloads, so inspect their persisted wire fields.
async fn retained_run_audit(store: &NatsStore, run: &pb::ReconciliationRun) -> RetainedRunAudit {
    use prost::Message;
    use trogon_atlas_store::{source::SourceDocumentKey, Store};

    #[derive(Clone, PartialEq, Message)]
    struct RetainedSourceRuns {
        #[prost(message, repeated, tag = "4")]
        runs: Vec<RetainedRunAudit>,
    }

    let document = store
        .source_get(&SourceDocumentKey::new(run.registration_id.clone()).unwrap())
        .await
        .unwrap()
        .unwrap();
    RetainedSourceRuns::decode(document.bytes.as_slice())
        .unwrap()
        .runs
        .into_iter()
        .find(|stored| stored.run.as_ref().is_some_and(|value| value.id == run.id))
        .unwrap()
}

#[tokio::test]
async fn outcome_expiry_preserves_independent_marker_and_audit_retention() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut server = server(store.clone()).await;
    let mut seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let mut configuration = seed.registration.clone();
    let retention = configuration.retention.as_mut().unwrap();
    retention.settled_outcome_seconds = 1;
    retention.marker_seconds = 5;
    retention.audit_seconds = 10;
    seed.registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(configuration),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let previewed = preview(
        &mut server.client,
        &seed,
        "independent-outcome-retention",
        entity(&seed.namespace, "source title", "local doc"),
        None,
    )
    .await;
    let applied = apply(&mut server.client, &previewed).await;
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let before = retained_run_audit(&store, &applied).await;
    assert!(before.run.as_ref().unwrap().marker.is_some());
    assert_ne!(before.payloads, []);
    assert_ne!(before.values, []);
    wait_backend_expiry(applied.expires_at_unix_ms).await;
    seed.registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(seed.registration.clone()),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let after = retained_run_audit(&store, &applied).await;
    let expired = after.run.as_ref().unwrap();
    assert!(expired.details_omitted);
    assert_eq!(expired.results, []);
    assert!(expired.fidelity.is_none());
    assert_eq!(
        (expired.marker.as_ref(), &after.payloads, &after.values),
        (
            before.run.as_ref().unwrap().marker.as_ref(),
            &before.payloads,
            &before.values
        ),
        "outcome expiry must preserve marker and audit until their independent deadlines"
    );
    let settled = applied.settled_at_unix_ms.unwrap();
    wait_backend_expiry(settled + 5000).await;
    seed.registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(seed.registration.clone()),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let marker_expired = retained_run_audit(&store, &applied).await;
    assert!(marker_expired.run.as_ref().unwrap().marker.is_none());
    assert_eq!(marker_expired.payloads, before.payloads);
    assert_eq!(marker_expired.values, before.values);
    wait_backend_expiry(settled + 10_000).await;
    server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(seed.registration.clone()),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap();
    let audit_expired = retained_run_audit(&store, &applied).await;
    assert_eq!(audit_expired.payloads, []);
    assert_eq!(audit_expired.values, []);
    assert_eq!(
        title(
            get_native(&mut server.client, &seed, None)
                .await
                .entity
                .as_ref()
                .unwrap()
        ),
        "source title"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automatic_sweep_discards_expired_unpublished_native_payloads() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut server = server(store.clone()).await;
    let mut seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let mut configuration = seed.registration.clone();
    let retention = configuration.retention.as_mut().unwrap();
    retention.preview_seconds = 1;
    retention.audit_seconds = 300;
    seed.registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(configuration),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let run = preview(
        &mut server.client,
        &seed,
        "abandoned-native-preview",
        entity(&seed.namespace, "source title", "local doc"),
        None,
    )
    .await;
    let before = retained_run_audit(&store, &run).await;
    assert_ne!(before.payloads, []);
    assert_ne!(before.values, []);
    let after = tokio::time::timeout(std::time::Duration::from_secs(90), async {
        loop {
            let stored = retained_run_audit(&store, &run).await;
            if stored.run.as_ref().unwrap().settled_at_unix_ms.is_some() {
                break stored;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("the real retention sweep must settle an expired unpublished preview");
    let expired = after.run.unwrap();
    assert_eq!(expired.status, pb::SourceRunStatus::Rejected as i32);
    assert_eq!(expired.settled_at_unix_ms, Some(run.expires_at_unix_ms));
    assert!(expired.details_omitted);
    assert!(expired.marker.is_none());
    assert_eq!(after.payloads, []);
    assert_eq!(after.values, []);
    assert_eq!(
        get_native(&mut server.client, &seed, None).await.etag,
        seed.etag
    );
}

#[tokio::test]
async fn overlay_expiry_keeps_raw_bytes_without_claiming_a_retained_overlay() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let namespace = namespace(&mut server.client, "overlay-retention", "visible-owner").await;
    let retention = pb::SourceRetentionPolicy {
        raw_bytes_seconds: 30,
        overlay_seconds: 1,
        ..trogon_atlas_core::source_integration::profile_retention()
    };
    let registration = registration(
        &mut server.client,
        "overlay-expiry",
        &namespace,
        None,
        Some(retention),
    )
    .await;
    let snapshot = accept(&mut server.client, &registration, "opaque-A", false).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let expired = server
        .client
        .get_source_overlay(req(
            READER,
            pb::GetSourceOverlayRequest {
                registration_id: registration.id.clone(),
                snapshot_id: snapshot.id.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .expect_err("overlay clock must expire independently of retained raw bytes");
    assert_eq!(expired.code(), Code::FailedPrecondition);
    let retained = server
        .client
        .get_source_snapshot(req(
            READER,
            pb::GetSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                snapshot_id: snapshot.id.clone(),
                include_bytes: true,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert!(retained.bytes_available);
    assert_eq!(retained.files[0].content, b"opaque supplied bytes");
    assert_eq!(
        retained.inventory_bytes,
        inventory(&registration.id, "opaque-A")
    );
}

#[tokio::test]
async fn authenticated_roles_replay_and_hidden_history_recheck_current_scope() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let denied = server
        .client
        .update_source_registration(req(
            WRITER,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(seed.registration.clone()),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);
    let request = pb::PreviewSourceRunRequest {
        registration_id: seed.registration.id.clone(),
        registration_revision: seed.registration.revision.clone(),
        snapshot_id: seed.snapshot.id.clone(),
        run_id: "auth-replay".into(),
        native_payloads: vec![pb::SourceNativePayload {
            source: Some(source(&seed.registration.id)),
            entity: Some(entity(&seed.namespace, "source title", "ignored")),
        }],
        ..Default::default()
    };
    assert_eq!(
        server
            .client
            .preview_source_run(req(READER, request.clone(), None))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    let run = server
        .client
        .preview_source_run(req(WRITER, request.clone(), None))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    let first = server
        .client
        .list_source_run_results(req(
            READER,
            pb::ListSourceRunResultsRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
                page_size: 1,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(first.next_page_token, "");
    let settled = apply(&mut server.client, &run).await;
    let stale = server
        .client
        .list_source_run_results(req(
            READER,
            pb::ListSourceRunResultsRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
                page_size: 1,
                page_token: first.next_page_token,
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(stale.code(), Code::FailedPrecondition);
    let replay = server
        .client
        .preview_source_run(req(WRITER, request.clone(), None))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(replay, settled);
    let mut changed = request;
    changed.native_payloads[0].entity = Some(entity(&seed.namespace, "different title", "ignored"));
    assert_eq!(
        server
            .client
            .preview_source_run(req(WRITER, changed, None))
            .await
            .unwrap_err()
            .code(),
        Code::AlreadyExists
    );
    assert_eq!(
        server
            .client
            .apply_source_run(req(
                READER,
                pb::ApplySourceRunRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id.clone(),
                    if_match: run.revision.clone(),
                    preview_digest: run.preview_digest.clone()
                },
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        server
            .client
            .get_source_run(req(
                SCOPED,
                pb::GetSourceRunRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id.clone()
                },
                None
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap(),
        settled
    );
    let hidden = namespace(&mut server.client, "hidden-history", "hidden-owner").await;
    let registration = registration(
        &mut server.client,
        "hidden-history-source",
        &hidden,
        None,
        None,
    )
    .await;
    let snapshot = accept(&mut server.client, &registration, "hidden-version", false).await;
    assert_eq!(
        server
            .client
            .get_source_snapshot(req(
                SCOPED,
                pb::GetSourceSnapshotRequest {
                    registration_id: registration.id.clone(),
                    snapshot_id: snapshot.id,
                    include_bytes: true
                },
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(
        server
            .client
            .get_source_run(req(
                READER,
                pb::GetSourceRunRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id
                },
                Some("unrelated-branch")
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test]
async fn visible_registration_pagination_is_independent_of_hidden_roots() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let visible = namespace(&mut server.client, "visible-paging", "visible-owner").await;
    let hidden = namespace(&mut server.client, "hidden-paging", "hidden-owner").await;
    for id in ["b-visible", "d-visible"] {
        registration(&mut server.client, id, &visible, None, None).await;
    }
    let query = pb::ListSourceRegistrationsRequest {
        page_size: 1,
        ..Default::default()
    };
    let first_before = server
        .client
        .list_source_registrations(req(SCOPED, query.clone(), None))
        .await
        .unwrap()
        .into_inner();
    let last_query = pb::ListSourceRegistrationsRequest {
        page_size: 1,
        page_token: first_before.next_page_token.clone(),
        ..Default::default()
    };
    let last_before = server
        .client
        .list_source_registrations(req(SCOPED, last_query.clone(), None))
        .await
        .unwrap()
        .into_inner();
    for id in ["a-hidden", "c-hidden", "e-hidden"] {
        registration(&mut server.client, id, &hidden, None, None).await;
    }
    let first = server
        .client
        .list_source_registrations(req(SCOPED, query, None))
        .await
        .unwrap()
        .into_inner();
    let last = server
        .client
        .list_source_registrations(req(SCOPED, last_query, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first, first_before);
    assert_eq!(last, last_before);
    assert_eq!(first.registrations.len(), 1);
    assert_eq!(first.next_page_token, "1");
    assert_eq!(last.registrations.len(), 1);
    assert_eq!(last.next_page_token, "");
    assert_ne!(first.registrations[0].id, last.registrations[0].id);
}

#[tokio::test]
async fn lowered_work_budgets_refuse_before_native_writes_and_page_caps_hold() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let seed = seed(
        &mut server.client,
        None,
        vec![pb::SourceOwnedField::Title, pb::SourceOwnedField::Doc],
    )
    .await;
    let mut registration = seed.registration.clone();
    registration.budgets.as_mut().unwrap().result_records = 1;
    registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(registration),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let refused = server
        .client
        .preview_source_run(req(
            WRITER,
            pb::PreviewSourceRunRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                snapshot_id: seed.snapshot.id.clone(),
                run_id: "over-budget".into(),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(source(&registration.id)),
                    entity: Some(entity(&seed.namespace, "source title", "source doc")),
                }],
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::ResourceExhausted);
    assert_eq!(
        get_native(&mut server.client, &seed, None).await.etag,
        seed.etag
    );
    let old_revision = registration.revision.clone();
    registration.budgets = Some(pb::SourceRunBudgets {
        page_records: 1,
        ..trogon_atlas_core::source_integration::profile_budgets()
    });
    registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(registration),
                if_match: old_revision,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let mut seed = seed;
    seed.registration = registration;
    let run = preview(
        &mut server.client,
        &seed,
        "bounded-pages",
        entity(&seed.namespace, "source title", "source doc"),
        None,
    )
    .await;
    let (results, fidelity) = reports(&mut server.client, &run, 1).await;
    assert_eq!(results.len(), 2);
    assert_eq!(fidelity.records.len(), 2);
    assert_eq!(
        server
            .client
            .list_source_run_results(req(
                READER,
                pb::ListSourceRunResultsRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id.clone(),
                    page_size: 2,
                    ..Default::default()
                },
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let mut cursor = String::new();
    let mut operations = 0;
    let mut entries = 0;
    for _ in 0..20 {
        let page = server
            .client
            .list_source_run_operations(req(
                READER,
                pb::ListSourceRunOperationsRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id.clone(),
                    page_size: 1,
                    page_token: cursor,
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(page.entries.len(), 1);
        entries += page.entries.len();
        operations += page
            .entries
            .iter()
            .filter(|e| {
                matches!(
                    e.entry,
                    Some(pb::source_run_operation_entry::Entry::Operation(_))
                )
            })
            .count();
        cursor = page.next_page_token;
        if cursor.is_empty() {
            break;
        }
    }
    assert_eq!(cursor, "");
    assert_eq!(operations, 1);
    assert!(entries > operations);
}

#[tokio::test]
async fn historical_run_scope_and_identity_survive_preview_expiry() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let mut seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let hidden = namespace(
        &mut server.client,
        "historical-hidden-owner",
        "hidden-owner",
    )
    .await;
    server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(entity(&hidden, "hidden owner", "private body")),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap();
    let mut registration = seed.registration.clone();
    registration.retention.as_mut().unwrap().preview_seconds = 1;
    seed.registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(registration),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let request = pb::PreviewSourceRunRequest {
        registration_id: seed.registration.id.clone(),
        registration_revision: seed.registration.revision.clone(),
        snapshot_id: seed.snapshot.id.clone(),
        run_id: "expired-identity".into(),
        native_payloads: vec![pb::SourceNativePayload {
            source: Some(source(&seed.registration.id)),
            entity: Some(entity(&seed.namespace, "source title", "ignored")),
        }],
        ..Default::default()
    };
    let run = server
        .client
        .preview_source_run(req(ADMIN, request.clone(), None))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let current = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(current.status, pb::SourceRunStatus::Rejected as i32);
    assert_eq!(
        server
            .client
            .get_source_run(req(
                SCOPED,
                pb::GetSourceRunRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id.clone()
                },
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(
        server
            .client
            .preview_source_run(req(SCOPED, request.clone(), None))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    let replay = server
        .client
        .preview_source_run(req(WRITER, request.clone(), None))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(replay.id, run.id);
    assert_eq!(replay.status, pb::SourceRunStatus::Rejected as i32);
    let mut changed = request;
    changed.native_payloads[0].entity = Some(entity(&seed.namespace, "another title", "ignored"));
    assert_eq!(
        server
            .client
            .preview_source_run(req(WRITER, changed, None))
            .await
            .unwrap_err()
            .code(),
        Code::AlreadyExists
    );
    assert_eq!(
        server
            .client
            .list_source_run_operations(req(
                READER,
                pb::ListSourceRunOperationsRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id,
                    ..Default::default()
                },
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let unauthenticated = server
        .client
        .get_source_registration(Request::new(pb::GetSourceRegistrationRequest {
            registration_id: seed.registration.id.clone(),
        }))
        .await
        .unwrap_err();
    assert_eq!(unauthenticated.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn physical_registration_quota_refuses_snapshot_without_partial_identity() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let namespace = namespace(&mut server.client, "physical-quota", "visible-owner").await;
    let registration = registration(
        &mut server.client,
        "physical-quota-source",
        &namespace,
        None,
        Some(pb::SourceRetentionPolicy {
            registration_bytes: 8192,
            ..trogon_atlas_core::source_integration::profile_retention()
        }),
    )
    .await;
    let oversized = server
        .client
        .accept_source_snapshot(req(
            WRITER,
            pb::AcceptSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                inventory_bytes: inventory(&registration.id, "quota-version"),
                files: vec![pb::SourceFile {
                    logical_name: "native/model".into(),
                    media_type: "application/octet-stream".into(),
                    content: vec![b'x'; 16384],
                    content_digest: String::new(),
                }],
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(oversized.code(), Code::ResourceExhausted);
    let snapshots = server
        .client
        .list_source_snapshots(req(
            READER,
            pb::ListSourceSnapshotsRequest {
                registration_id: registration.id.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(snapshots.snapshots, []);
    assert_eq!(snapshots.next_page_token, "");
    let accepted = accept(&mut server.client, &registration, "quota-version", false).await;
    assert!(accepted.bytes_available);
}

struct InterruptPreparedIntent;
impl trogon_atlas_store::source::SourcePublicationObserver for InterruptPreparedIntent {
    fn reached(
        &self,
        point: trogon_atlas_store::source::SourcePublicationPoint,
    ) -> trogon_atlas_store::StoreResult<()> {
        if point == trogon_atlas_store::source::SourcePublicationPoint::IntentDurable {
            return Err(trogon_atlas_store::StoreError::Unavailable(
                "interrupted after durable source preparation".into(),
            ));
        }
        Ok(())
    }
}

#[tokio::test]
async fn interrupted_prepare_can_cancel_and_expire_without_stranding_phase_zero() {
    use trogon_atlas_store::{source::SourceDocumentKey, Store};
    let store = trogon_atlas_testsupport::shared()
        .await
        .store()
        .await
        .with_source_publication_observer(Arc::new(InterruptPreparedIntent));
    let mut server = server(store.clone()).await;
    let seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let run = preview(
        &mut server.client,
        &seed,
        "prepare-interrupted",
        entity(&seed.namespace, "source title", "ignored"),
        None,
    )
    .await;
    assert_eq!(
        server
            .client
            .apply_source_run(req(
                ADMIN,
                pb::ApplySourceRunRequest {
                    registration_id: seed.registration.id.clone(),
                    run_id: run.id.clone(),
                    if_match: run.revision.clone(),
                    preview_digest: run.preview_digest.clone()
                },
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    let retained = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(retained.status, pb::SourceRunStatus::Previewed as i32);
    assert!(!retained.recovery_required);
    let cancelled = server
        .client
        .cancel_source_run(req(
            ADMIN,
            pb::CancelSourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
                if_match: retained.revision,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(cancelled.status, pb::SourceRunStatus::Cancelled as i32);
    assert!(!cancelled.recovery_required);
    let key = SourceDocumentKey::new(seed.registration.id.clone()).unwrap();
    store
        .source_expire_transaction(&key, &run.id, &run.preview_digest)
        .await
        .expect("a cancelled prepared intent must be durably settled before retention expires it");
    assert!(
        store
            .source_recover(&key, &run.id, &run.preview_digest)
            .await
            .is_err(),
        "missing transaction evidence must remain unknown after expiry"
    );
    assert_eq!(
        get_native(&mut server.client, &seed, None).await.etag,
        seed.etag
    );
    assert!(store
        .source_recover(&key, "missing-intent", &run.preview_digest)
        .await
        .is_err());
}

#[tokio::test]
async fn tenant_any_native_payload_previews_applies_and_replays_with_map_equivalence() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let namespace = namespace(&mut server.client, "tenant-source", "visible-owner").await;
    let library=pb::Entity{kind:Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary{
        id:Some(pb::Id{namespace:namespace.clone(),slug:"source.money".into(),version:1}),
        title:"Money".into(),files:vec![pb::ProtoSourceFile{path:"source/money/v1/money.proto".into(),content:"syntax = \"proto3\"; package source.money.v1; message Money { int64 cents = 1; string currency = 2; map<string,string> labels = 3; }".into()}],..Default::default()
    })),..Default::default()};
    server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(library),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap();
    let registration =
        registration(&mut server.client, "tenant-source", &namespace, None, None).await;
    let snapshot = accept(&mut server.client, &registration, "money-A", false).await;
    let owner = pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: namespace.clone(),
            slug: "paid".into(),
            version: 1,
        }),
    };
    server
        .client
        .create_source_mapping(req(
            WRITER,
            pb::CreateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: "tenant-mapping".into(),
                    source: Some(source(&registration.id)),
                    selection: Some(pb::SourceSelection {
                        selection: Some(pb::source_selection::Selection::Entity(owner.clone())),
                    }),
                    ownership: Some(pb::SourceFieldOwnership {
                        policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                            pb::SourceWholeOwnerOwnership {},
                        )),
                    }),
                    ..Default::default()
                }),
            },
            None,
        ))
        .await
        .unwrap();
    let payload = |reverse: bool| {
        let mut bytes = vec![0x08, 0x96, 0x01, 0x12, 0x03, b'U', b'S', b'D'];
        let first = [0x1a, 0x06, 0x0a, 0x01, b'a', 0x12, 0x01, b'1'];
        let second = [0x1a, 0x06, 0x0a, 0x01, b'b', 0x12, 0x01, b'2'];
        if reverse {
            bytes.extend(second);
            bytes.extend(first);
        } else {
            bytes.extend(first);
            bytes.extend(second);
        }
        pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: owner.id.clone(),
                title: "Paid".into(),
                schema: Some(prost_types::Any {
                    type_url: "type.googleapis.com/source.money.v1.Money".into(),
                    value: bytes,
                }),
                ..Default::default()
            })),
            ..Default::default()
        }
    };
    let request = pb::PreviewSourceRunRequest {
        registration_id: registration.id.clone(),
        registration_revision: registration.revision.clone(),
        snapshot_id: snapshot.id.clone(),
        run_id: "tenant-any".into(),
        native_payloads: vec![pb::SourceNativePayload {
            source: Some(source(&registration.id)),
            entity: Some(payload(false)),
        }],
        ..Default::default()
    };
    let run=server.client.preview_source_run(req(WRITER,request.clone(),None)).await.expect("connected tenant Any must use its persisted TypeLibrary during canonical preview hashing").into_inner().run.unwrap();
    let mut reordered = request;
    reordered.native_payloads[0].entity = Some(payload(true));
    let replay = server
        .client
        .preview_source_run(req(WRITER, reordered, None))
        .await
        .expect("map wire order must not change canonical request identity")
        .into_inner()
        .run
        .unwrap();
    assert_eq!(replay, run);
    let (_, fidelity) = reports(&mut server.client, &run, 1).await;
    assert!(fidelity.native_reapply_supported);
    assert!(!fidelity.incomplete);
    let settled = apply(&mut server.client, &run).await;
    assert_eq!(settled.status, pb::SourceRunStatus::Applied as i32);
    let native = server
        .client
        .get_entity(req(
            READER,
            pb::GetEntityRequest {
                kind: owner.kind,
                id: owner.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert!(trogon_atlas_core::semantically_equal(
        &native,
        &payload(false)
    ));
    let mapping = server
        .client
        .get_source_mapping(req(
            READER,
            pb::GetSourceMappingRequest {
                registration_id: registration.id.clone(),
                mapping_id: "tenant-mapping".into(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert!(trogon_atlas_core::semantically_equal(
        mapping
            .base
            .as_ref()
            .unwrap()
            .source_value
            .as_ref()
            .unwrap(),
        &payload(false)
    ));
}

struct PausePreparedIntent {
    entered: tokio::sync::Notify,
    release: std::sync::Barrier,
}
impl trogon_atlas_store::source::SourcePublicationObserver for PausePreparedIntent {
    fn reached(
        &self,
        point: trogon_atlas_store::source::SourcePublicationPoint,
    ) -> trogon_atlas_store::StoreResult<()> {
        if point == trogon_atlas_store::source::SourcePublicationPoint::IntentDurable {
            self.entered.notify_one();
            self.release.wait();
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apply_phase_budget_includes_waiting_for_durable_preparation() {
    use trogon_atlas_store::{
        source::{SourceDocumentKey, SourcePublication},
        Store,
    };
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut server = server(store.clone()).await;
    let mut seed = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let mut configuration = seed.registration.clone();
    configuration.budgets.as_mut().unwrap().phase_timeout_ms = 2000;
    seed.registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(configuration),
                if_match: seed.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let run = preview(
        &mut server.client,
        &seed,
        "bounded-attempt",
        entity(&seed.namespace, "source title", "ignored"),
        None,
    )
    .await;
    let blocker = registration(
        &mut server.client,
        "prepare-blocker",
        &seed.namespace,
        None,
        None,
    )
    .await;
    let key = SourceDocumentKey::new(blocker.id).unwrap();
    let document = store.source_get(&key).await.unwrap().unwrap();
    let observer = Arc::new(PausePreparedIntent {
        entered: tokio::sync::Notify::new(),
        release: std::sync::Barrier::new(2),
    });
    let blocking_store = store
        .clone()
        .with_source_publication_observer(observer.clone());
    let publication = SourcePublication {
        registration: key,
        control_revision: document.revision,
        run_id: "independent-prepare".into(),
        plan_digest: "independent-plan".into(),
        plan: pb::SourceRunPlan::default(),
        values: Vec::new(),
        storage_limit: blocker.retention.unwrap().registration_bytes,
        finalization_reserve_bytes: 0,
    };
    let prepared = tokio::spawn(async move { blocking_store.source_prepare(&publication).await });
    observer.entered.notified().await;
    let attempted = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        server.client.apply_source_run(req(
            ADMIN,
            pb::ApplySourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision.clone(),
                preview_digest: run.preview_digest.clone(),
            },
            None,
        )),
    )
    .await;
    observer.release.wait();
    prepared.await.unwrap().unwrap();
    let refused = attempted
        .expect("the whole advertised attempt must time out while preparation is blocked")
        .expect_err("preparation cannot complete before the attempt deadline");
    assert_eq!(refused.code(), Code::DeadlineExceeded);
    let retained = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: seed.registration.id.clone(),
                run_id: run.id,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(retained.status, pb::SourceRunStatus::Previewed as i32);
    assert!(!retained.recovery_required);
    assert_eq!(
        get_native(&mut server.client, &seed, None).await.etag,
        seed.etag
    );
}

mod delayed_preview_store {
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use tokio::sync::broadcast;
    use trogon_atlas_store::{
        store::{
            BranchInfo, ChangeRecord, ChangeStreamStats, ListFilter, MutationOp, MutationOutcome,
            Store, Written,
        },
        BatchRecoveryReport, ChangeKind, ChangesetPage, ChangesetRecord, ChangesetRef,
        RecoveryPolicy, StoredEntity, WriteContext,
    };

    use super::*;
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum PreviewGatePoint {
        NativeRead,
        FinalSave,
        Recovered,
        Published,
        OwnerUnavailable,
        RecoveryUnavailable,
    }
    pub struct PreviewGate {
        pub point: PreviewGatePoint,
        pub armed: AtomicBool,
        pub entered: tokio::sync::Notify,
        pub release: tokio::sync::Notify,
    }
    impl PreviewGate {
        pub fn new(point: PreviewGatePoint) -> Self {
            Self {
                point,
                armed: AtomicBool::new(false),
                entered: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            }
        }
        async fn pause(&self) {
            if self.armed.swap(false, Ordering::AcqRel) {
                self.entered.notify_one();
                self.release.notified().await;
            }
        }
    }
    pub struct DelayedPreviewStore {
        pub inner: NatsStore,
        pub gate: Arc<PreviewGate>,
    }
    #[async_trait]
    impl Store for DelayedPreviewStore {
        fn is_writer(&self) -> bool {
            self.inner.is_writer()
        }

        async fn recover_batches(
            &self,
            policy: RecoveryPolicy,
        ) -> trogon_atlas_store::StoreResult<BatchRecoveryReport> {
            self.inner.recover_batches(policy).await
        }

        async fn register_namespace(
            &self,
            name: &trogon_atlas_core::NamespaceName,
            parent: &trogon_atlas_core::OwnerId,
            created_by: &str,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
            self.inner
                .register_namespace(name, parent, created_by)
                .await
        }

        async fn adopt_namespace(
            &self,
            name: &trogon_atlas_core::NamespaceName,
            parent: &trogon_atlas_core::OwnerId,
            tenure: &trogon_atlas_store::NamespaceTenure,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
            self.inner.adopt_namespace(name, parent, tenure).await
        }

        async fn restore_namespace(
            &self,
            record: &trogon_atlas_store::NamespaceRecord,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
            self.inner.restore_namespace(record).await
        }

        async fn release_namespace(
            &self,
            id: &trogon_atlas_core::NamespaceId,
        ) -> trogon_atlas_store::StoreResult<()> {
            self.inner.release_namespace(id).await
        }

        async fn promote_namespace(
            &self,
            id: &trogon_atlas_core::NamespaceId,
        ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
            self.inner.promote_namespace(id).await
        }

        async fn get_namespace(
            &self,
            id: &trogon_atlas_core::NamespaceId,
        ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
            self.inner.get_namespace(id).await
        }

        async fn resolve_namespace(
            &self,
            parent: &trogon_atlas_core::OwnerId,
            name: &trogon_atlas_core::NamespaceName,
        ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_core::NamespaceId>> {
            self.inner.resolve_namespace(parent, name).await
        }

        async fn list_namespaces(
            &self,
        ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::NamespaceRecord>> {
            self.inner.list_namespaces().await
        }

        async fn move_namespace(
            &self,
            id: &trogon_atlas_core::NamespaceId,
            new_parent: &trogon_atlas_core::OwnerId,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceRecord> {
            self.inner.move_namespace(id, new_parent).await
        }

        async fn get(
            &self,
            kind: pb::EntityKind,
            id: &pb::Id,
            branch: Option<&str>,
        ) -> trogon_atlas_store::StoreResult<StoredEntity> {
            if self.gate.point == PreviewGatePoint::OwnerUnavailable
                && self.gate.armed.swap(false, Ordering::AcqRel)
            {
                return Err(trogon_atlas_store::StoreError::Unavailable(
                    "overlay backend read unavailable".into(),
                ));
            }
            self.inner.get(kind, id, branch).await
        }

        async fn batch_get(
            &self,
            keys: &[(pb::EntityKind, pb::Id)],
            branch: Option<&str>,
        ) -> trogon_atlas_store::StoreResult<Vec<Option<StoredEntity>>> {
            self.inner.batch_get(keys, branch).await
        }

        async fn create(
            &self,
            kind: pb::EntityKind,
            entity: &pb::Entity,
            ctx: WriteContext<'_>,
        ) -> trogon_atlas_store::StoreResult<Written> {
            self.inner.create(kind, entity, ctx).await
        }

        async fn put(
            &self,
            kind: pb::EntityKind,
            entity: &pb::Entity,
            force: bool,
            ctx: WriteContext<'_>,
        ) -> trogon_atlas_store::StoreResult<Written> {
            self.inner.put(kind, entity, force, ctx).await
        }

        async fn update(
            &self,
            kind: pb::EntityKind,
            entity: &pb::Entity,
            expected_etag: &str,
            force: bool,
            ctx: WriteContext<'_>,
        ) -> trogon_atlas_store::StoreResult<Written> {
            self.inner
                .update(kind, entity, expected_etag, force, ctx)
                .await
        }

        async fn delete(
            &self,
            kind: pb::EntityKind,
            id: &pb::Id,
            expected_etag: Option<&str>,
            ctx: WriteContext<'_>,
        ) -> trogon_atlas_store::StoreResult<()> {
            self.inner.delete(kind, id, expected_etag, ctx).await
        }

        async fn referenced_targets(
            &self,
            targets: &[pb::EntityKey],
            branch: Option<&str>,
        ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
            self.inner.referenced_targets(targets, branch).await
        }

        async fn referenced_targets_with_overlay(
            &self,
            targets: &[pb::EntityKey],
            branch: Option<&str>,
            overlay: &[(pb::EntityKey, Option<pb::Entity>)],
        ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
            self.inner
                .referenced_targets_with_overlay(targets, branch, overlay)
                .await
        }

        async fn list(
            &self,
            filter: ListFilter<'_>,
            limit: Option<usize>,
            branch: Option<&str>,
        ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
            if self.gate.point == PreviewGatePoint::NativeRead {
                self.gate.pause().await;
            }
            self.inner.list(filter, limit, branch).await
        }

        async fn record_change(
            &self,
            kind: ChangeKind,
            entity_ref: &pb::EntityRef,
            changeset: Option<ChangesetRef<'_>>,
        ) -> trogon_atlas_store::StoreResult<u64> {
            self.inner.record_change(kind, entity_ref, changeset).await
        }

        async fn read_changes(
            &self,
            since: u64,
            limit: usize,
        ) -> trogon_atlas_store::StoreResult<Vec<ChangeRecord>> {
            self.inner.read_changes(since, limit).await
        }

        async fn current_change_seq(&self) -> trogon_atlas_store::StoreResult<u64> {
            self.inner.current_change_seq().await
        }

        async fn batch_apply(
            &self,
            ops: &[MutationOp],
            ctx: WriteContext<'_>,
        ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
            self.inner.batch_apply(ops, ctx).await
        }

        fn subscribe(&self) -> broadcast::Receiver<ChangeRecord> {
            self.inner.subscribe()
        }

        async fn prune_changes(&self, before_seq: u64) -> trogon_atlas_store::StoreResult<()> {
            self.inner.prune_changes(before_seq).await
        }

        async fn change_stream_stats(
            &self,
        ) -> trogon_atlas_store::StoreResult<Option<ChangeStreamStats>> {
            self.inner.change_stream_stats().await
        }

        async fn create_branch(
            &self,
            name: &str,
            doc: &str,
        ) -> trogon_atlas_store::StoreResult<BranchInfo> {
            self.inner.create_branch(name, doc).await
        }

        async fn set_branch_fork_point(
            &self,
            name: &str,
            fork_changeset_id: &str,
        ) -> trogon_atlas_store::StoreResult<()> {
            self.inner
                .set_branch_fork_point(name, fork_changeset_id)
                .await
        }

        async fn list_branches(&self) -> trogon_atlas_store::StoreResult<Vec<BranchInfo>> {
            self.inner.list_branches().await
        }

        async fn delete_branch(&self, name: &str) -> trogon_atlas_store::StoreResult<()> {
            self.inner.delete_branch(name).await
        }

        async fn list_branch_deltas(
            &self,
            branch: &str,
        ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::store::BranchDeltaEntry>>
        {
            self.inner.list_branch_deltas(branch).await
        }

        async fn land_branch_merge(
            &self,
            branch: &str,
            ops: &[trogon_atlas_store::store::BranchLandOp],
            ctx: WriteContext<'_>,
        ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
            self.inner.land_branch_merge(branch, ops, ctx).await
        }

        async fn rebase_branch_deltas(
            &self,
            branch: &str,
            rebases: &[(pb::EntityKind, pb::Id, Option<(pb::Entity, String)>)],
        ) -> trogon_atlas_store::StoreResult<u32> {
            self.inner.rebase_branch_deltas(branch, rebases).await
        }

        async fn resolve_branch_entry(
            &self,
            branch: &str,
            kind: pb::EntityKind,
            id: &pb::Id,
            take_theirs: bool,
        ) -> trogon_atlas_store::StoreResult<()> {
            self.inner
                .resolve_branch_entry(branch, kind, id, take_theirs)
                .await
        }

        async fn append_changeset(
            &self,
            record: &ChangesetRecord,
        ) -> trogon_atlas_store::StoreResult<()> {
            self.inner.append_changeset(record).await
        }

        async fn get_changeset(
            &self,
            id: &str,
        ) -> trogon_atlas_store::StoreResult<ChangesetRecord> {
            self.inner.get_changeset(id).await
        }

        async fn list_changesets(
            &self,
            page: ChangesetPage<'_>,
        ) -> trogon_atlas_store::StoreResult<Vec<ChangesetRecord>> {
            self.inner.list_changesets(page).await
        }

        async fn list_entity_revisions(
            &self,
            kind: pb::EntityKind,
            id: &pb::Id,
            page: trogon_atlas_store::RevisionPage<'_>,
        ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::EntityRevisionRecord>>
        {
            self.inner.list_entity_revisions(kind, id, page).await
        }

        async fn get_entity_revision(
            &self,
            kind: pb::EntityKind,
            id: &pb::Id,
            changeset_id: &str,
            branch: Option<&str>,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::EntityRevisionRecord> {
            self.inner
                .get_entity_revision(kind, id, changeset_id, branch)
                .await
        }

        async fn source_expire_transaction(
            &self,
            key: &trogon_atlas_store::source::SourceDocumentKey,
            run: &str,
            digest: &str,
        ) -> trogon_atlas_store::StoreResult<()> {
            self.inner.source_expire_transaction(key, run, digest).await
        }
        async fn source_prepare(
            &self,
            publication: &trogon_atlas_store::source::SourcePublication,
        ) -> trogon_atlas_store::StoreResult<()> {
            self.inner.source_prepare(publication).await
        }
        async fn source_publish(
            &self,
            publication: &trogon_atlas_store::source::SourcePublication,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::source::SourcePublicationOutcome>
        {
            let outcome = self.inner.source_publish(publication).await?;
            if self.gate.point == PreviewGatePoint::Published {
                self.gate.pause().await;
            }
            Ok(outcome)
        }
        async fn source_recover(
            &self,
            key: &trogon_atlas_store::source::SourceDocumentKey,
            run: &str,
            digest: &str,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::source::SourcePublicationOutcome>
        {
            if self.gate.point == PreviewGatePoint::RecoveryUnavailable
                && self.gate.armed.swap(false, Ordering::AcqRel)
            {
                return Err(trogon_atlas_store::StoreError::Unavailable(
                    "source recovery temporarily unavailable".into(),
                ));
            }
            let outcome = self.inner.source_recover(key, run, digest).await?;
            if self.gate.point == PreviewGatePoint::Recovered {
                self.gate.pause().await;
            }
            Ok(outcome)
        }
        async fn source_get(
            &self,
            key: &trogon_atlas_store::source::SourceDocumentKey,
        ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::source::SourceDocument>>
        {
            self.inner.source_get(key).await
        }
        async fn source_compare_and_swap(
            &self,
            key: &trogon_atlas_store::source::SourceDocumentKey,
            revision: Option<u64>,
            bytes: &[u8],
            limit: u64,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::source::SourceDocument> {
            if self.gate.point == PreviewGatePoint::FinalSave {
                self.gate.pause().await;
            }
            self.inner
                .source_compare_and_swap(key, revision, bytes, limit)
                .await
        }
        async fn source_list(
            &self,
            after: Option<&str>,
            limit: usize,
        ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::source::SourceDocumentPage>
        {
            self.inner.source_list(after, limit).await
        }
    }
}

fn expiring_seed(
    client: &mut EventModelServiceClient<Channel>,
    whole_owner: bool,
) -> impl Future<Output = Seed> + '_ {
    Box::pin(async move {
        let mut value = seed(client, None, vec![pb::SourceOwnedField::Title]).await;
        let mut configuration = value.registration.clone();
        configuration.retention.as_mut().unwrap().base_seconds = 1;
        if whole_owner {
            configuration.deletion_policy = pb::SourceDeletionPolicy::ExclusiveSafeDelete as i32;
        }
        value.registration = client
            .update_source_registration(req(
                ADMIN,
                pb::UpdateSourceRegistrationRequest {
                    registration: Some(configuration),
                    if_match: value.registration.revision.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .registration
            .unwrap();
        let mut mapping = value.mapping.clone();
        if whole_owner {
            mapping.ownership = Some(pb::SourceFieldOwnership {
                policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                    pb::SourceWholeOwnerOwnership {},
                )),
            });
        }
        value.mapping = client
            .update_source_mapping(req(
                ADMIN,
                pb::UpdateSourceMappingRequest {
                    registration_revision: value.registration.revision.clone(),
                    if_match: value.mapping.revision.clone(),
                    mapping: Some(mapping),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        value
    })
}
async fn wait_backend_expiry(deadline: i64) {
    let delay = deadline
        .saturating_sub(chrono::Utc::now().timestamp_millis())
        .saturating_add(50);
    tokio::time::sleep(std::time::Duration::from_millis(
        u64::try_from(delay).unwrap_or(0),
    ))
    .await;
}

#[tokio::test]
async fn apply_refuses_a_base_that_expired_after_preview_without_native_or_base_advance() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let value = expiring_seed(&mut server.client, false).await;
    let original = value.mapping.base.as_ref().unwrap().clone();
    let run = preview(
        &mut server.client,
        &value,
        "expired-base-update",
        entity(&value.namespace, "source title", "ignored"),
        None,
    )
    .await;
    wait_backend_expiry(original.expires_at_unix_ms).await;
    let outcome = server
        .client
        .apply_source_run(req(
            ADMIN,
            pb::ApplySourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision.clone(),
                preview_digest: run.preview_digest.clone(),
            },
            None,
        ))
        .await;
    let native = get_native(&mut server.client, &value, None).await;
    assert_eq!(
        native.etag, value.etag,
        "expired accepted evidence cannot authorize a native publication: {outcome:?}"
    );
    assert_eq!(outcome.unwrap_err().code(), Code::FailedPrecondition);
    let retained = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(retained.status, pb::SourceRunStatus::Previewed as i32);
    assert!(!retained.recovery_required);
    let mapping = server
        .client
        .get_source_mapping(req(
            READER,
            pb::GetSourceMappingRequest {
                registration_id: value.registration.id.clone(),
                mapping_id: value.mapping.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    let base = mapping.base.unwrap();
    assert_eq!(base.revision, original.revision);
    assert_eq!(base.accepted_at_unix_ms, original.accepted_at_unix_ms);
    assert_eq!(base.expires_at_unix_ms, original.expires_at_unix_ms);
    assert_eq!(
        mapping.last_accepted_run_id,
        value.mapping.last_accepted_run_id
    );
}

#[tokio::test]
async fn safe_delete_refuses_a_base_that_expired_after_complete_omission_preview() {
    let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
    let mut value = expiring_seed(&mut server.client, true).await;
    let accepted = preview(
        &mut server.client,
        &value,
        "accepted-whole-owner",
        entity(&value.namespace, "before", "local doc"),
        None,
    )
    .await;
    apply(&mut server.client, &accepted).await;
    value.mapping = server
        .client
        .get_source_mapping(req(
            READER,
            pb::GetSourceMappingRequest {
                registration_id: value.registration.id.clone(),
                mapping_id: value.mapping.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    let original = value.mapping.base.as_ref().unwrap().clone();
    let mut omitted: serde_json::Value =
        serde_json::from_slice(&inventory(&value.registration.id, "omitted")).unwrap();
    omitted["coverage"] = serde_json::json!("COMPLETE");
    omitted["resources"] = serde_json::json!([]);
    let snapshot = server
        .client
        .accept_source_snapshot(req(
            ADMIN,
            pb::AcceptSourceSnapshotRequest {
                registration_id: value.registration.id.clone(),
                registration_revision: value.registration.revision.clone(),
                inventory_bytes: serde_json::to_vec(&omitted).unwrap(),
                files: Vec::new(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    let run = server
        .client
        .preview_source_run(req(
            ADMIN,
            pb::PreviewSourceRunRequest {
                registration_id: value.registration.id.clone(),
                registration_revision: value.registration.revision.clone(),
                snapshot_id: snapshot.id.clone(),
                run_id: "expired-base-delete".into(),
                completeness: Some(pb::SourceCompletenessDecision {
                    verified_complete: true,
                    inventory_digest: snapshot.inventory_digest.clone(),
                    base_snapshot_id: value.snapshot.id.clone(),
                }),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(run.operation_count, 1);
    wait_backend_expiry(original.expires_at_unix_ms).await;
    let outcome = server
        .client
        .apply_source_run(req(
            ADMIN,
            pb::ApplySourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision.clone(),
                preview_digest: run.preview_digest.clone(),
            },
            None,
        ))
        .await;
    let native = get_native(&mut server.client, &value, None).await;
    assert_eq!(
        native.etag, value.etag,
        "expired deletion evidence cannot authorize native deletion: {outcome:?}"
    );
    assert_eq!(outcome.unwrap_err().code(), Code::FailedPrecondition);
    let retained = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert!(!retained.recovery_required);
    assert_eq!(retained.status, pb::SourceRunStatus::Previewed as i32);
}

struct CommitPastBaseExpiry(std::sync::atomic::AtomicI64);
impl trogon_atlas_store::source::SourcePublicationObserver for CommitPastBaseExpiry {
    fn reached(
        &self,
        point: trogon_atlas_store::source::SourcePublicationPoint,
    ) -> trogon_atlas_store::StoreResult<()> {
        if point == trogon_atlas_store::source::SourcePublicationPoint::CommitDecisionDurable {
            let delay = self
                .0
                .load(std::sync::atomic::Ordering::Acquire)
                .saturating_sub(chrono::Utc::now().timestamp_millis())
                .saturating_add(50);
            std::thread::sleep(std::time::Duration::from_millis(
                u64::try_from(delay).unwrap_or(0),
            ));
            return Err(trogon_atlas_store::StoreError::Unavailable(
                "interrupted after committed publication past base and raw expiry".into(),
            ));
        }
        Ok(())
    }
}
struct ReleaseRetainedPublication(Arc<delayed_preview_store::PreviewGate>);
impl Drop for ReleaseRetainedPublication {
    fn drop(&mut self) {
        self.0.release.notify_one();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn valid_started_publication_keeps_immutable_base_evidence_for_late_committed_recovery() {
    use delayed_preview_store::{DelayedPreviewStore, PreviewGate, PreviewGatePoint};
    use trogon_atlas_store::Store;
    let observer = Arc::new(CommitPastBaseExpiry(std::sync::atomic::AtomicI64::new(0)));
    let recovery = Arc::new(PreviewGate::new(PreviewGatePoint::Recovered));
    let _release = ReleaseRetainedPublication(recovery.clone());
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut server = server(DelayedPreviewStore {
        inner: store
            .clone()
            .with_source_publication_observer(observer.clone()),
        gate: recovery.clone(),
    })
    .await;
    let mut value = expiring_seed(&mut server.client, false).await;
    let mut configuration = value.registration.clone();
    configuration.retention.as_mut().unwrap().raw_bytes_seconds = 1;
    value.registration = server
        .client
        .update_source_registration(req(
            ADMIN,
            pb::UpdateSourceRegistrationRequest {
                registration: Some(configuration),
                if_match: value.registration.revision.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    value.snapshot = accept(
        &mut server.client,
        &value.registration,
        "protected-raw",
        false,
    )
    .await;
    let original = server
        .client
        .get_source_snapshot(req(
            READER,
            pb::GetSourceSnapshotRequest {
                registration_id: value.registration.id.clone(),
                snapshot_id: value.snapshot.id.clone(),
                include_bytes: true,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert!(original.bytes_available);
    assert_ne!(original.files[0].content, Vec::<u8>::new());
    observer.0.store(
        value
            .mapping
            .base
            .as_ref()
            .unwrap()
            .expires_at_unix_ms
            .max(original.created_at_unix_ms + 1000),
        std::sync::atomic::Ordering::Release,
    );
    let run = preview(
        &mut server.client,
        &value,
        "protected-base-recovery",
        entity(&value.namespace, "source title", "ignored"),
        None,
    )
    .await;
    recovery
        .armed
        .store(true, std::sync::atomic::Ordering::Release);
    let mut client = server.client.clone();
    let sent = run.clone();
    let attempt = tokio::spawn(async move { apply(&mut client, &sent).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        recovery.entered.notified(),
    )
    .await
    .unwrap();
    wait_backend_expiry(
        value
            .mapping
            .base
            .as_ref()
            .unwrap()
            .expires_at_unix_ms
            .max(original.created_at_unix_ms + 1000),
    )
    .await;
    let applying = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(applying.status, pb::SourceRunStatus::Applying as i32);
    assert!(applying.recovery_required);
    let retained = server
        .client
        .get_source_snapshot(req(
            READER,
            pb::GetSourceSnapshotRequest {
                registration_id: value.registration.id.clone(),
                snapshot_id: value.snapshot.id.clone(),
                include_bytes: true,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert!(retained.bytes_available);
    assert_eq!(retained.inventory_bytes, original.inventory_bytes);
    assert_eq!(retained.files, original.files);
    assert_eq!(retained.bundle_digest, original.bundle_digest);
    assert_eq!(retained.inventory_digest, original.inventory_digest);
    let (_, fidelity) = reports(&mut server.client, &applying, 1).await;
    assert!(fidelity.original_bytes_recoverable);
    assert!(fidelity
        .records
        .iter()
        .all(|record| record.original_bytes_recoverable));
    let committed = get_native(&mut server.client, &value, None).await;
    let mut later = committed.entity.unwrap();
    let Some(pb::entity::Kind::ReadModel(model)) = later.kind.as_mut() else {
        panic!("ReadModel")
    };
    model.title = "later native edit".into();
    store
        .put(
            pb::EntityKind::ReadModel,
            &later,
            false,
            trogon_atlas_store::WriteContext::scoped(None),
        )
        .await
        .unwrap();
    recovery.release.notify_one();
    let settled = tokio::time::timeout(std::time::Duration::from_secs(10), attempt)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.status, pb::SourceRunStatus::Applied as i32);
    assert!(!settled.recovery_required);
    let native = get_native(&mut server.client, &value, None).await;
    assert_eq!(title(native.entity.as_ref().unwrap()), "later native edit");
    let mapping = server
        .client
        .get_source_mapping(req(
            READER,
            pb::GetSourceMappingRequest {
                registration_id: value.registration.id.clone(),
                mapping_id: value.mapping.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert_eq!(mapping.last_accepted_run_id, run.id);
    let base = mapping.base.as_ref().unwrap();
    assert_eq!(title(base.source_value.as_ref().unwrap()), "source title");
    assert_eq!(title(base.native_value.as_ref().unwrap()), "source title");
    assert_eq!(base.observed.as_ref().unwrap().etag, committed.etag);
    assert_ne!(base.observed.as_ref().unwrap().etag, native.etag);
    let (_, fidelity) = reports(&mut server.client, &settled, 1).await;
    assert!(fidelity.original_bytes_recoverable);
}

fn preview_deadline_case(
    point: delayed_preview_store::PreviewGatePoint,
) -> impl Future<Output = ()> {
    Box::pin(async move {
        use delayed_preview_store::{DelayedPreviewStore, PreviewGate};
        let gate = Arc::new(PreviewGate::new(point));
        let mut server = server(DelayedPreviewStore {
            inner: trogon_atlas_testsupport::shared().await.store().await,
            gate: gate.clone(),
        })
        .await;
        let mut value = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
        let mut configuration = value.registration.clone();
        configuration.budgets.as_mut().unwrap().phase_timeout_ms = 1000;
        value.registration = server
            .client
            .update_source_registration(req(
                ADMIN,
                pb::UpdateSourceRegistrationRequest {
                    registration: Some(configuration),
                    if_match: value.registration.revision.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .registration
            .unwrap();
        gate.armed.store(true, std::sync::atomic::Ordering::Release);
        let request = pb::PreviewSourceRunRequest {
            registration_id: value.registration.id.clone(),
            registration_revision: value.registration.revision.clone(),
            snapshot_id: value.snapshot.id.clone(),
            run_id: "bounded-preview".into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source(&value.registration.id)),
                entity: Some(entity(&value.namespace, "source title", "ignored")),
            }],
            ..Default::default()
        };
        let mut client = server.client.clone();
        let sent = request.clone();
        let mut attempt =
            tokio::spawn(async move { client.preview_source_run(req(ADMIN, sent, None)).await });
        tokio::time::timeout(std::time::Duration::from_secs(3), gate.entered.notified())
            .await
            .expect("public preview reached the selected dependency boundary");
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), &mut attempt).await;
        gate.release.notify_one();
        let outcome = if let Ok(joined) = result {
            joined.unwrap()
        } else {
            let late = attempt.await.unwrap();
            panic!("preview exceeded advertised deadline while dependency remained blocked; late outcome: {late:?}")
        };
        assert_eq!(outcome.unwrap_err().code(), Code::DeadlineExceeded);
        assert_eq!(
            server
                .client
                .get_source_run(req(
                    READER,
                    pb::GetSourceRunRequest {
                        registration_id: value.registration.id.clone(),
                        run_id: "bounded-preview".into()
                    },
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
        assert_eq!(
            get_native(&mut server.client, &value, None).await.etag,
            value.etag
        );
        let retry = server
            .client
            .preview_source_run(req(ADMIN, request, None))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        assert_eq!(retry.status, pb::SourceRunStatus::Previewed as i32);
    })
}
#[tokio::test]
async fn preview_deadline_includes_awaited_native_reads() {
    preview_deadline_case(delayed_preview_store::PreviewGatePoint::NativeRead).await;
}
#[tokio::test]
async fn preview_deadline_includes_final_plan_persistence() {
    preview_deadline_case(delayed_preview_store::PreviewGatePoint::FinalSave).await;
}

fn result_boundary_case(limit: u32) -> impl Future<Output = ()> {
    Box::pin(async move {
        let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
        let mut value = seed(
            &mut server.client,
            None,
            vec![pb::SourceOwnedField::Title, pb::SourceOwnedField::Doc],
        )
        .await;
        let mut configuration = value.registration.clone();
        configuration.budgets.as_mut().unwrap().result_records = limit;
        value.registration = server
            .client
            .update_source_registration(req(
                ADMIN,
                pb::UpdateSourceRegistrationRequest {
                    registration: Some(configuration),
                    if_match: value.registration.revision.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .registration
            .unwrap();
        let outcome = server
            .client
            .preview_source_run(req(
                ADMIN,
                pb::PreviewSourceRunRequest {
                    registration_id: value.registration.id.clone(),
                    registration_revision: value.registration.revision.clone(),
                    snapshot_id: value.snapshot.id.clone(),
                    run_id: "exact-results-boundary".into(),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(source(&value.registration.id)),
                        entity: Some(entity(&value.namespace, "source title", "source doc")),
                    }],
                    ..Default::default()
                },
                None,
            ))
            .await;
        if limit < 5 {
            assert_eq!(outcome.unwrap_err().code(), Code::ResourceExhausted);
            assert_eq!(
                get_native(&mut server.client, &value, None).await.etag,
                value.etag
            );
            assert_eq!(
                server
                    .client
                    .get_source_run(req(
                        READER,
                        pb::GetSourceRunRequest {
                            registration_id: value.registration.id.clone(),
                            run_id: "exact-results-boundary".into()
                        },
                        None
                    ))
                    .await
                    .unwrap_err()
                    .code(),
                Code::NotFound
            );
        } else {
            let run = outcome
                .expect("five retained records fit exactly, without duplicate core/service charges")
                .into_inner()
                .run
                .unwrap();
            let (results, fidelity) = reports(&mut server.client, &run, 1).await;
            assert_eq!(results.len(), 2);
            assert_eq!(fidelity.records.len(), 2);
            assert_eq!(fidelity.checked_owners.len(), 1);
            let settled = apply(&mut server.client, &run).await;
            assert_eq!(settled.status, pb::SourceRunStatus::Applied as i32);
            let native = get_native(&mut server.client, &value, None).await;
            assert_eq!(title(native.entity.as_ref().unwrap()), "source title");
            assert_eq!(doc(native.entity.as_ref().unwrap()), "source doc");
        }
    })
}
#[tokio::test]
async fn exact_five_record_merge_report_succeeds() {
    result_boundary_case(5).await;
}
#[tokio::test]
async fn four_record_limit_refuses_five_record_merge_before_writes() {
    result_boundary_case(4).await;
}

struct PreparePastBaseExpiry(std::sync::atomic::AtomicI64);
impl trogon_atlas_store::source::SourcePublicationObserver for PreparePastBaseExpiry {
    fn reached(
        &self,
        point: trogon_atlas_store::source::SourcePublicationPoint,
    ) -> trogon_atlas_store::StoreResult<()> {
        if point == trogon_atlas_store::source::SourcePublicationPoint::IntentDurable {
            let delay = self
                .0
                .load(std::sync::atomic::Ordering::Acquire)
                .saturating_sub(chrono::Utc::now().timestamp_millis())
                .saturating_add(50);
            std::thread::sleep(std::time::Duration::from_millis(
                u64::try_from(delay).unwrap_or(0),
            ));
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apply_refuses_base_expiry_during_prepare_before_applying_cas() {
    let observer = Arc::new(PreparePastBaseExpiry(std::sync::atomic::AtomicI64::new(0)));
    let store = trogon_atlas_testsupport::shared()
        .await
        .store()
        .await
        .with_source_publication_observer(observer.clone());
    let mut server = server(store).await;
    let value = expiring_seed(&mut server.client, false).await;
    let base = value.mapping.base.as_ref().unwrap();
    observer.0.store(
        base.expires_at_unix_ms,
        std::sync::atomic::Ordering::Release,
    );
    let run = preview(
        &mut server.client,
        &value,
        "expiry-during-prepare",
        entity(&value.namespace, "source title", "ignored"),
        None,
    )
    .await;
    let outcome = server
        .client
        .apply_source_run(req(
            ADMIN,
            pb::ApplySourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision.clone(),
                preview_digest: run.preview_digest.clone(),
            },
            None,
        ))
        .await;
    assert_eq!(
        get_native(&mut server.client, &value, None).await.etag,
        value.etag,
        "base must still be live when prepare finishes: {outcome:?}"
    );
    assert_eq!(outcome.unwrap_err().code(), Code::FailedPrecondition);
    let retained = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(retained.status, pb::SourceRunStatus::Previewed as i32);
    assert!(!retained.recovery_required);
    let cancelled = server
        .client
        .cancel_source_run(req(
            ADMIN,
            pb::CancelSourceRunRequest {
                registration_id: value.registration.id.clone(),
                run_id: run.id,
                if_match: retained.revision,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(cancelled.status, pb::SourceRunStatus::Cancelled as i32);
    assert!(!cancelled.recovery_required);
}

fn tenant_merge_case(historical: bool) -> impl Future<Output = ()> {
    Box::pin(async move {
        let mut server = server(trogon_atlas_testsupport::shared().await.store().await).await;
        let namespace = namespace(&mut server.client, "tenant-source", "visible-owner").await;
        let library=pb::Entity{kind:Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary{
        id:Some(pb::Id{namespace:namespace.clone(),slug:"source.money".into(),version:1}),
        title:"Money".into(),files:vec![pb::ProtoSourceFile{path:"source/money/v1/money.proto".into(),content:"syntax = \"proto3\"; package source.money.v1; message Money { int64 cents = 1; string currency = 2; map<string,string> labels = 3; }".into()}],..Default::default()
    })),..Default::default()};
        server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(library.clone()),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap();
        let registration =
            registration(&mut server.client, "tenant-source", &namespace, None, None).await;
        let snapshot = accept(&mut server.client, &registration, "money-A", false).await;
        let owner = pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: namespace.clone(),
                slug: "paid".into(),
                version: 1,
            }),
        };
        server
            .client
            .create_source_mapping(req(
                WRITER,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: "tenant-mapping".into(),
                        source: Some(source(&registration.id)),
                        selection: Some(pb::SourceSelection {
                            selection: Some(pb::source_selection::Selection::Entity(owner.clone())),
                        }),
                        ownership: Some(pb::SourceFieldOwnership {
                            policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                                pb::SourceWholeOwnerOwnership {},
                            )),
                        }),
                        ..Default::default()
                    }),
                },
                None,
            ))
            .await
            .unwrap();
        let payload = |reverse: bool| {
            let mut bytes = vec![0x08, 0x96, 0x01, 0x12, 0x03, b'U', b'S', b'D'];
            let first = [0x1a, 0x06, 0x0a, 0x01, b'a', 0x12, 0x01, b'1'];
            let second = [0x1a, 0x06, 0x0a, 0x01, b'b', 0x12, 0x01, b'2'];
            if reverse {
                bytes.extend(second);
                bytes.extend(first);
            } else {
                bytes.extend(first);
                bytes.extend(second);
            }
            pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: owner.id.clone(),
                    title: "Paid".into(),
                    schema: Some(prost_types::Any {
                        type_url: "type.googleapis.com/source.money.v1.Money".into(),
                        value: bytes,
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            }
        };
        let request = pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: "tenant-any".into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source(&registration.id)),
                entity: Some(payload(false)),
            }],
            ..Default::default()
        };
        let run=server.client.preview_source_run(req(WRITER,request.clone(),None)).await.expect("connected tenant Any must use its persisted TypeLibrary during canonical preview hashing").into_inner().run.unwrap();
        let mut reordered = request;
        reordered.native_payloads[0].entity = Some(payload(true));
        let replay = server
            .client
            .preview_source_run(req(WRITER, reordered, None))
            .await
            .expect("map wire order must not change canonical request identity")
            .into_inner()
            .run
            .unwrap();
        assert_eq!(replay, run);
        let (_, fidelity) = reports(&mut server.client, &run, 1).await;
        assert!(fidelity.native_reapply_supported);
        assert!(!fidelity.incomplete);
        let settled = apply(&mut server.client, &run).await;
        assert_eq!(settled.status, pb::SourceRunStatus::Applied as i32);
        let native = server
            .client
            .get_entity(req(
                READER,
                pb::GetEntityRequest {
                    kind: owner.kind,
                    id: owner.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(trogon_atlas_core::semantically_equal(
            native.entity.as_ref().unwrap(),
            &payload(false)
        ));
        let mapping = server
            .client
            .get_source_mapping(req(
                READER,
                pb::GetSourceMappingRequest {
                    registration_id: registration.id.clone(),
                    mapping_id: "tenant-mapping".into(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert!(trogon_atlas_core::semantically_equal(
            mapping
                .base
                .as_ref()
                .unwrap()
                .source_value
                .as_ref()
                .unwrap(),
            &payload(false)
        ));
        let baseline_etag = server
            .client
            .get_entity(req(
                READER,
                pb::GetEntityRequest {
                    kind: owner.kind,
                    id: owner.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .etag;
        let fresh_request = |id: &str, entity: pb::Entity| pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: id.into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source(&registration.id)),
                entity: Some(entity),
            }],
            ..Default::default()
        };
        if historical {
            let mut without_schema = payload(false);
            let Some(pb::entity::Kind::Event(event)) = without_schema.kind.as_mut() else {
                panic!("Event")
            };
            event.schema = None;
            let changed = server
                .client
                .put_entity(req(
                    ADMIN,
                    pb::PutEntityRequest {
                        entity: Some(without_schema.clone()),
                        if_match: baseline_etag,
                        ..Default::default()
                    },
                    None,
                ))
                .await
                .expect("public native migration must release the tenant type reference")
                .into_inner();
            let Some(pb::entity::Kind::TypeLibrary(library)) = library.kind else {
                panic!("TypeLibrary")
            };
            server
                .client
                .delete_entity(req(
                    ADMIN,
                    pb::DeleteEntityRequest {
                        kind: pb::EntityKind::TypeLibrary as i32,
                        id: library.id,
                        mode: pb::delete_entity_request::Mode::Force as i32,
                        ..Default::default()
                    },
                    None,
                ))
                .await
                .expect("unused library must be removable through public guarded native admission");
            let outcome = server
                .client
                .preview_source_run(req(
                    WRITER,
                    fresh_request("historical-unavailable", without_schema),
                    None,
                ))
                .await;
            let current = server
                .client
                .get_entity(req(
                    READER,
                    pb::GetEntityRequest {
                        kind: owner.kind,
                        id: owner.id.clone(),
                    },
                    None,
                ))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(current.etag, changed.etag);
            let status =
                outcome.expect_err("historical tenant type must resolve explicitly during merge");
            assert_eq!(status.code(), Code::InvalidArgument);
            assert!(status.message().contains("owned source type"), "{status}");
            return;
        }
        let Some(pb::entity::Kind::Event(first)) = payload(false).kind else {
            panic!("Event")
        };
        let Some(pb::entity::Kind::Event(second)) = payload(true).kind else {
            panic!("Event")
        };
        assert_ne!(first.schema.unwrap().value, second.schema.unwrap().value);
        let same = server
            .client
            .preview_source_run(req(
                WRITER,
                fresh_request("fresh-map-order", payload(true)),
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        let (results, _) = reports(&mut server.client, &same, 1).await;
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].disposition,
            pb::SourceResultDisposition::Unchanged as i32,
            "same typed map must remain unchanged in a NEW run"
        );
        assert_eq!(same.operation_count, 0);
        let operations = server
            .client
            .list_source_run_operations(req(
                READER,
                pb::ListSourceRunOperationsRequest {
                    registration_id: registration.id.clone(),
                    run_id: same.id.clone(),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(operations.entries.iter().all(|entry| matches!(
            entry.entry,
            Some(
                pb::source_run_operation_entry::Entry::NativeGuard(_)
                    | pb::source_run_operation_entry::Entry::MappingGuard(_)
            )
        )));
        assert!(operations.entries.iter().any(|entry| matches!(
            entry.entry,
            Some(pb::source_run_operation_entry::Entry::NativeGuard(_))
        )));
        assert_eq!(
            apply(&mut server.client, &same).await.status,
            pb::SourceRunStatus::Applied as i32
        );
        let unchanged = server
            .client
            .get_entity(req(
                READER,
                pb::GetEntityRequest {
                    kind: owner.kind,
                    id: owner.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(unchanged.etag, baseline_etag);
        let mut changed = payload(true);
        let Some(pb::entity::Kind::Event(event)) = changed.kind.as_mut() else {
            panic!("Event")
        };
        event.schema.as_mut().unwrap().value[1] = 0x97;
        let update = server
            .client
            .preview_source_run(req(
                WRITER,
                fresh_request("real-typed-change", changed),
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        let (results, _) = reports(&mut server.client, &update, 1).await;
        assert_eq!(
            results[0].disposition,
            pb::SourceResultDisposition::Update as i32
        );
        assert_eq!(
            apply(&mut server.client, &update).await.status,
            pb::SourceRunStatus::Applied as i32
        );
        let updated = server
            .client
            .get_entity(req(
                READER,
                pb::GetEntityRequest {
                    kind: owner.kind,
                    id: owner.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_ne!(updated.etag, baseline_etag);
    })
}
#[tokio::test]
async fn fresh_tenant_map_wire_order_is_unchanged_but_real_value_updates() {
    tenant_merge_case(false).await;
}
#[tokio::test]
async fn historical_tenant_type_must_resolve_before_merge() {
    tenant_merge_case(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automatic_retention_sweep_reclaims_persisted_source_bytes_without_source_writes() {
    use trogon_atlas_store::{source::SourceDocumentKey, Store};
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let started = tokio::time::Instant::now();
    let mut server = server(store.clone()).await;
    let namespace = namespace(&mut server.client, "automatic-retention", "visible-owner").await;
    let registration = registration(
        &mut server.client,
        "automatic-retention",
        &namespace,
        None,
        Some(pb::SourceRetentionPolicy {
            raw_bytes_seconds: 1,
            overlay_seconds: 300,
            ..trogon_atlas_core::source_integration::profile_retention()
        }),
    )
    .await;
    let bytes = inventory(&registration.id, "unaccepted-sweep");
    let files = vec![pb::SourceFile {
        logical_name: "native/model".into(),
        media_type: "application/octet-stream".into(),
        content: vec![b'z'; 512 * 1024],
        content_digest: String::new(),
    }];
    let snapshot = server
        .client
        .accept_source_snapshot(req(
            WRITER,
            pb::AcceptSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                inventory_bytes: bytes.clone(),
                files: files.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert!(snapshot.accepted_at_unix_ms.is_none());
    let key = SourceDocumentKey::new(registration.id.clone()).unwrap();
    let before = store.source_get(&key).await.unwrap().unwrap();
    assert!(before.bytes.len() > files[0].content.len());
    wait_backend_expiry(snapshot.created_at_unix_ms + 1000).await;
    let unchanged = store.source_get(&key).await.unwrap().unwrap();
    assert_eq!(unchanged.revision, before.revision);
    assert_eq!(unchanged.bytes.len(), before.bytes.len());
    let deadline = started + std::time::Duration::from_secs(90);
    let after = tokio::time::timeout_at(deadline, async {
        loop {
            let current = store.source_get(&key).await.unwrap().unwrap();
            if current.revision != before.revision
                && before.bytes.len().saturating_sub(current.bytes.len()) >= files[0].content.len()
            {
                break current;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("constructor's real 60-second sweep must reclaim persisted bytes within 90 seconds");
    assert!(started.elapsed() >= std::time::Duration::from_secs(55));
    assert!(after.revision > before.revision);
    assert!(before.bytes.len() - after.bytes.len() >= files[0].content.len());
    let expired = server
        .client
        .get_source_snapshot(req(
            READER,
            pb::GetSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                snapshot_id: snapshot.id.clone(),
                include_bytes: true,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert!(!expired.bytes_available);
    assert_eq!(expired.files, Vec::<pb::SourceFile>::new());
    assert_eq!(expired.inventory_bytes, Vec::<u8>::new());
    assert_eq!(expired.bundle_digest, snapshot.bundle_digest);
    let overlay = server
        .client
        .get_source_overlay(req(
            READER,
            pb::GetSourceOverlayRequest {
                registration_id: registration.id.clone(),
                snapshot_id: snapshot.id.clone(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .overlay
        .unwrap();
    assert_eq!(overlay.entries.len(), 1);
    assert_eq!(
        store.source_get(&key).await.unwrap().unwrap().revision,
        after.revision
    );
    let mut changed = bytes;
    changed.push(b' ');
    let collision = server
        .client
        .accept_source_snapshot(req(
            WRITER,
            pb::AcceptSourceSnapshotRequest {
                registration_id: registration.id,
                registration_revision: registration.revision,
                inventory_bytes: changed,
                files,
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(collision.code(), Code::AlreadyExists);
    assert_eq!(
        store.source_get(&key).await.unwrap().unwrap().revision,
        after.revision
    );
}

async fn overlay_test_store() -> NatsStore {
    let Ok(url) = std::env::var("ATLAS_SOURCE_OVERLAY_TEST_NATS_URL") else {
        return trogon_atlas_testsupport::shared().await.store().await;
    };
    let scope = format!("source-overlay-{}", uuid::Uuid::now_v7().simple());
    let mut configuration =
        trogon_atlas_store::NatsStoreConfig::new(url, trogon_atlas_store::StoreMetrics::default());
    configuration.bucket = format!("{scope}-entities");
    configuration.branches_bucket = format!("{scope}-branches");
    configuration.changesets_bucket = format!("{scope}-changesets");
    configuration.revisions_bucket = format!("{scope}-revisions");
    configuration.batches_bucket = format!("{scope}-batches");
    configuration.namespaces_bucket = format!("{scope}-namespaces");
    configuration.operations_bucket = format!("{scope}-operations");
    configuration.lease_bucket = format!("{scope}-lease");
    configuration.stream = scope.replace('-', "_").to_uppercase();
    configuration.subject_root = scope.clone();
    configuration.changes_max_msgs = Some(10_000);
    configuration.changes_max_age = None;
    eprintln!("SOURCE_OVERLAY_TEST_SCOPE={scope}");
    NatsStore::connect_with(configuration)
        .await
        .expect("isolated overlay test backend must connect")
}
fn checked_overlay_mapping<'a>(
    client: &'a mut EventModelServiceClient<Channel>,
    registration: &'a pb::SourceRegistration,
    selection: pb::SourceSelection,
    native: &'a pb::PutEntityResponse,
    mapping_id: &'a str,
) -> impl Future<Output = pb::SourceMapping> + 'a {
    Box::pin(async move {
        let entity = native.entity.as_ref().unwrap();
        let owner = pb::EntityRef {
            kind: trogon_atlas_core::refs::entity_kind(entity).unwrap() as i32,
            id: trogon_atlas_core::refs::entity_id(entity).cloned(),
        };
        client
            .create_source_mapping(req(
                ADMIN,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: mapping_id.into(),
                        source: Some(source(&registration.id)),
                        selection: Some(selection),
                        ownership: Some(pb::SourceFieldOwnership {
                            policy: Some(pb::source_field_ownership::Policy::None(
                                pb::SourceNoOwnership {},
                            )),
                        }),
                        adoption: Some(pb::SourceAdoption {
                            observed: Some(pb::SourceObservedOwner {
                                owner: Some(owner),
                                uid: entity.system.as_ref().unwrap().uid.clone(),
                                etag: native.etag.clone(),
                            }),
                            authorized_adoption: true,
                        }),
                        ..Default::default()
                    }),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap()
    })
}
async fn checked_overlay_entry(
    client: &mut EventModelServiceClient<Channel>,
    token: &str,
    registration: &pb::SourceRegistration,
    snapshot: &pb::SourceSnapshot,
) -> Result<pb::SourceOverlayEntry, tonic::Status> {
    let overlay = client
        .get_source_overlay(req(
            token,
            pb::GetSourceOverlayRequest {
                registration_id: registration.id.clone(),
                snapshot_id: snapshot.id.clone(),
                ..Default::default()
            },
            None,
        ))
        .await?
        .into_inner()
        .overlay
        .unwrap();
    assert_eq!(overlay.entries.len(), 1);
    Ok(overlay.entries.into_iter().next().unwrap())
}
#[tokio::test]
async fn source_overlay_explicit_mapping_reports_authorized_missing_and_recreated_conflict() {
    let mut server = server(overlay_test_store().await).await;
    let namespace = namespace(&mut server.client, "overlay-status", "visible-owner").await;
    let registration =
        registration(&mut server.client, "overlay-status", &namespace, None, None).await;
    let snapshot = accept(&mut server.client, &registration, "status-A", false).await;
    let deferred = checked_overlay_entry(&mut server.client, READER, &registration, &snapshot)
        .await
        .unwrap();
    assert_eq!(deferred.status, pb::SourceOverlayStatus::Deferred as i32);
    assert_eq!(
        deferred.authorized_candidates,
        Vec::<pb::SourceSelection>::new()
    );
    let native = server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(entity(&namespace, "before", "doc")),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let owner = pb::EntityRef {
        kind: pb::EntityKind::ReadModel as i32,
        id: trogon_atlas_core::refs::entity_id(native.entity.as_ref().unwrap()).cloned(),
    };
    let selection = pb::SourceSelection {
        selection: Some(pb::source_selection::Selection::Entity(owner.clone())),
    };
    let mapping = checked_overlay_mapping(
        &mut server.client,
        &registration,
        selection.clone(),
        &native,
        "status-map",
    )
    .await;
    let resolved = checked_overlay_entry(&mut server.client, READER, &registration, &snapshot)
        .await
        .unwrap();
    assert_eq!(resolved.status, pb::SourceOverlayStatus::Resolved as i32);
    assert_eq!(resolved.authorized_candidates, vec![selection]);
    let mut duplicate = mapping.clone();
    duplicate.id = "duplicate-map".into();
    duplicate.base = None;
    duplicate.observed = None;
    duplicate.revision.clear();
    let duplicate = server
        .client
        .create_source_mapping(req(
            ADMIN,
            pb::CreateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                mapping: Some(duplicate),
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(duplicate.code(), Code::AlreadyExists);
    server
        .client
        .delete_entity(req(
            ADMIN,
            pb::DeleteEntityRequest {
                kind: owner.kind,
                id: owner.id.clone(),
                if_match: native.etag.clone(),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap();
    let missing = checked_overlay_entry(&mut server.client, READER, &registration, &snapshot)
        .await
        .unwrap();
    assert_eq!(
        missing.status,
        pb::SourceOverlayStatus::Missing as i32,
        "an authorized exact Store NotFound must establish Missing"
    );
    assert_eq!(missing.reason_code, "exact_owner_missing");
    assert_eq!(
        missing.authorized_candidates,
        Vec::<pb::SourceSelection>::new()
    );
    let recreated = server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(entity(&namespace, "recreated", "doc")),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(
        recreated
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        native.entity.as_ref().unwrap().system.as_ref().unwrap().uid
    );
    let conflict = checked_overlay_entry(&mut server.client, READER, &registration, &snapshot)
        .await
        .unwrap();
    assert_eq!(conflict.status, pb::SourceOverlayStatus::Conflict as i32);
    assert_eq!(conflict.reason_code, "native_incarnation_changed");
    assert_eq!(
        conflict.authorized_candidates,
        Vec::<pb::SourceSelection>::new()
    );
}
#[tokio::test]
async fn source_overlay_hidden_owner_remains_unavailable_even_after_deletion() {
    let mut server = server(overlay_test_store().await).await;
    let visible = namespace(&mut server.client, "overlay-visible", "visible-owner").await;
    let hidden = namespace(&mut server.client, "overlay-hidden", "hidden-owner").await;
    let registration =
        registration(&mut server.client, "hidden-overlay", &visible, None, None).await;
    let snapshot = accept(&mut server.client, &registration, "hidden-A", false).await;
    let native = server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(entity(&hidden, "hidden", "doc")),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let owner = pb::EntityRef {
        kind: pb::EntityKind::ReadModel as i32,
        id: trogon_atlas_core::refs::entity_id(native.entity.as_ref().unwrap()).cloned(),
    };
    checked_overlay_mapping(
        &mut server.client,
        &registration,
        pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Entity(owner.clone())),
        },
        &native,
        "hidden-map",
    )
    .await;
    for deleted in [false, true] {
        if deleted {
            server
                .client
                .delete_entity(req(
                    ADMIN,
                    pb::DeleteEntityRequest {
                        kind: owner.kind,
                        id: owner.id.clone(),
                        if_match: native.etag.clone(),
                        mode: pb::delete_entity_request::Mode::Force as i32,
                        ..Default::default()
                    },
                    None,
                ))
                .await
                .unwrap();
        }
        let unavailable =
            checked_overlay_entry(&mut server.client, SCOPED, &registration, &snapshot)
                .await
                .unwrap();
        assert_eq!(
            unavailable.status,
            pb::SourceOverlayStatus::Unavailable as i32
        );
        assert_eq!(
            unavailable.authorized_candidates,
            Vec::<pb::SourceSelection>::new()
        );
    }
}
#[tokio::test]
async fn source_overlay_stale_local_pin_is_unavailable_and_operational_failure_is_preserved() {
    use delayed_preview_store::{DelayedPreviewStore, PreviewGate, PreviewGatePoint};
    let fault = Arc::new(PreviewGate::new(PreviewGatePoint::OwnerUnavailable));
    let mut server = server(DelayedPreviewStore {
        inner: overlay_test_store().await,
        gate: fault.clone(),
    })
    .await;
    let namespace = namespace(&mut server.client, "overlay-local", "visible-owner").await;
    let registration =
        registration(&mut server.client, "local-overlay", &namespace, None, None).await;
    let snapshot = accept(&mut server.client, &registration, "local-A", false).await;
    let subject = server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(entity(&namespace, "Receipt", "doc")),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let subject = pb::EntityRef {
        kind: pb::EntityKind::ReadModel as i32,
        id: trogon_atlas_core::refs::entity_id(subject.entity.as_ref().unwrap()).cloned(),
    };
    let native = server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(pb::Id {
                            namespace: namespace.clone(),
                            slug: "note".into(),
                            version: 1,
                        }),
                        title: Some(pb::CatalogTitle {
                            value: "Note".into(),
                        }),
                        subjects: vec![pb::SubjectAssociation {subject:Some(pb::CatalogSubject {selection:Some(pb::catalog_subject::Selection::Entity(subject))}),..Default::default()}],
                        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                            media_type: Some(pb::MediaType {
                                value: "text/markdown".into(),
                            }),
                            content: "# Note\n".into(),
                            digest: Some(pb::ContentDigest { sha256: "c2f92031c1bdc84166a86e6003926514861b838b5cda62775be8cc6fd066caac".into() }),
                        })),
                        anchors: vec![pb::DocumentAnchor {
                            fragment: Some(pb::AnchorFragment {
                                value: "note".into(),
                            }),
                            label: Some(pb::CatalogTitle {
                                value: "Note".into(),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let current = native.entity.as_ref().unwrap();
    let Some(pb::entity::Kind::Document(document)) = &current.kind else {
        panic!("Document")
    };
    let Some(pb::document::Body::Retained(body)) = &document.body else {
        panic!("retained document")
    };
    checked_overlay_mapping(
        &mut server.client,
        &registration,
        pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Document(
                pb::DocumentSelection {
                    selection: Some(pb::document_selection::Selection::Anchor(
                        pb::DocumentAnchorRef {
                            document: Some(pb::DocumentRef {
                                id: document.id.clone(),
                            }),
                            owner_uid: current.system.as_ref().unwrap().uid.clone(),
                            anchor_id: document.anchors[0].anchor_id.clone(),
                            expected_digest: body.digest.clone(),
                        },
                    )),
                },
            )),
        },
        &native,
        "local-map",
    )
    .await;
    assert_eq!(
        checked_overlay_entry(&mut server.client, READER, &registration, &snapshot)
            .await
            .unwrap()
            .status,
        pb::SourceOverlayStatus::Resolved as i32
    );
    let mut removed = current.clone();
    let Some(pb::entity::Kind::Document(document)) = removed.kind.as_mut() else {
        panic!("Document")
    };
    document.anchors.clear();
    server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(removed),
                if_match: native.etag,
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap();
    let unavailable = checked_overlay_entry(&mut server.client, READER, &registration, &snapshot)
        .await
        .unwrap();
    assert_eq!(
        unavailable.status,
        pb::SourceOverlayStatus::Unavailable as i32
    );
    assert_eq!(
        unavailable.authorized_candidates,
        Vec::<pb::SourceSelection>::new()
    );
    fault
        .armed
        .store(true, std::sync::atomic::Ordering::Release);
    let operational = checked_overlay_entry(&mut server.client, READER, &registration, &snapshot)
        .await
        .unwrap_err();
    assert_eq!(operational.code(), Code::Unavailable);
    assert!(operational
        .message()
        .contains("overlay backend read unavailable"));
}

fn source_test_mirror() -> (
    tempfile::TempDir,
    Arc<trogon_atlas_server::git_mirror::GitMirror>,
) {
    let directory = tempfile::tempdir().unwrap();
    let mirror = trogon_atlas_server::git_mirror::GitMirror::open(
        directory.path().to_path_buf(),
        trogon_atlas_server::git_mirror::MirrorAuthor {
            name: "source mirror test".into(),
            email: "source@test.invalid".into(),
        },
    )
    .unwrap();
    (directory, mirror)
}
fn mirrored_native(directory: &std::path::Path, namespace: &str) -> Option<pb::Entity> {
    use prost::Message;
    let path = directory
        .join("read_models")
        .join(namespace)
        .join("read-model/1.binpb");
    match std::fs::read(path) {
        Ok(bytes) => Some(pb::Entity::decode(bytes.as_slice()).unwrap()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("cannot read source mirror: {error}"),
    }
}
#[derive(Clone, Copy)]
enum SourceMirrorCase {
    Create,
    Update,
    Delete,
    Branch,
}
fn source_mirror_case(case: SourceMirrorCase) -> impl Future<Output = ()> {
    Box::pin(async move {
        let (directory, mirror) = source_test_mirror();
        let mut server = server_with_mirror(overlay_test_store().await, Some(mirror)).await;
        let branch = matches!(case, SourceMirrorCase::Branch).then_some("source-mirror-branch");
        let mut value = if matches!(case, SourceMirrorCase::Create) {
            let namespace = namespace(&mut server.client, "mirror-create", "visible-owner").await;
            let registration =
                registration(&mut server.client, "native-source", &namespace, None, None).await;
            let snapshot = accept(&mut server.client, &registration, "create-A", false).await;
            let entity = entity(&namespace, "source title", "doc");
            let mapping = server
                .client
                .create_source_mapping(req(
                    ADMIN,
                    pb::CreateSourceMappingRequest {
                        registration_revision: registration.revision.clone(),
                        mapping: Some(pb::SourceMapping {
                            id: "native-mapping".into(),
                            source: Some(source(&registration.id)),
                            selection: Some(pb::SourceSelection {
                                selection: Some(pb::source_selection::Selection::Entity(
                                    pb::EntityRef {
                                        kind: pb::EntityKind::ReadModel as i32,
                                        id: trogon_atlas_core::refs::entity_id(&entity).cloned(),
                                    },
                                )),
                            }),
                            ownership: Some(pb::SourceFieldOwnership {
                                policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                                    pb::SourceWholeOwnerOwnership {},
                                )),
                            }),
                            ..Default::default()
                        }),
                    },
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .mapping
                .unwrap();
            Seed {
                namespace,
                registration,
                snapshot,
                mapping,
                entity,
                etag: String::new(),
            }
        } else {
            seed(
                &mut server.client,
                branch,
                vec![pb::SourceOwnedField::Title],
            )
            .await
        };
        if !matches!(case, SourceMirrorCase::Create | SourceMirrorCase::Branch) {
            assert_eq!(
                title(
                    &mirrored_native(directory.path(), &value.namespace)
                        .expect("ordinary baseline PutEntity establishes mirror control")
                ),
                "before"
            );
        }
        if matches!(case, SourceMirrorCase::Delete) {
            let mut configuration = value.registration.clone();
            configuration.deletion_policy = pb::SourceDeletionPolicy::ExclusiveSafeDelete as i32;
            value.registration = server
                .client
                .update_source_registration(req(
                    ADMIN,
                    pb::UpdateSourceRegistrationRequest {
                        registration: Some(configuration),
                        if_match: value.registration.revision.clone(),
                    },
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .registration
                .unwrap();
            let mut mapping = value.mapping.clone();
            mapping.ownership = Some(pb::SourceFieldOwnership {
                policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                    pb::SourceWholeOwnerOwnership {},
                )),
            });
            value.mapping = server
                .client
                .update_source_mapping(req(
                    ADMIN,
                    pb::UpdateSourceMappingRequest {
                        registration_revision: value.registration.revision.clone(),
                        if_match: value.mapping.revision.clone(),
                        mapping: Some(mapping),
                    },
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .mapping
                .unwrap();
            let run = preview(
                &mut server.client,
                &value,
                "accept-delete-base",
                value.entity.clone(),
                None,
            )
            .await;
            assert_eq!(
                apply(&mut server.client, &run).await.status,
                pb::SourceRunStatus::Applied as i32
            );
            let mut omitted: serde_json::Value =
                serde_json::from_slice(&inventory(&value.registration.id, "delete-B")).unwrap();
            omitted["coverage"] = serde_json::json!("COMPLETE");
            omitted["resources"] = serde_json::json!([]);
            let snapshot = server
                .client
                .accept_source_snapshot(req(
                    ADMIN,
                    pb::AcceptSourceSnapshotRequest {
                        registration_id: value.registration.id.clone(),
                        registration_revision: value.registration.revision.clone(),
                        inventory_bytes: serde_json::to_vec(&omitted).unwrap(),
                        files: Vec::new(),
                    },
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .snapshot
                .unwrap();
            let run = server
                .client
                .preview_source_run(req(
                    ADMIN,
                    pb::PreviewSourceRunRequest {
                        registration_id: value.registration.id.clone(),
                        registration_revision: value.registration.revision.clone(),
                        snapshot_id: snapshot.id.clone(),
                        run_id: "mirror-delete".into(),
                        completeness: Some(pb::SourceCompletenessDecision {
                            verified_complete: true,
                            inventory_digest: snapshot.inventory_digest,
                            base_snapshot_id: value.snapshot.id.clone(),
                        }),
                        ..Default::default()
                    },
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .run
                .unwrap();
            assert_eq!(run.operation_count, 1);
            assert_eq!(
                apply(&mut server.client, &run).await.status,
                pb::SourceRunStatus::Applied as i32
            );
            assert_eq!(
                server
                    .client
                    .get_entity(req(
                        READER,
                        pb::GetEntityRequest {
                            kind: pb::EntityKind::ReadModel as i32,
                            id: trogon_atlas_core::refs::entity_id(&value.entity).cloned()
                        },
                        None
                    ))
                    .await
                    .unwrap_err()
                    .code(),
                Code::NotFound
            );
            assert_eq!(
                mirrored_native(directory.path(), &value.namespace),
                None,
                "source baseline deletion must remove the mirror file"
            );
        } else {
            let run = preview(
                &mut server.client,
                &value,
                "source-mirror",
                entity(&value.namespace, "source title", "doc"),
                branch,
            )
            .await;
            assert_eq!(
                apply(&mut server.client, &run).await.status,
                pb::SourceRunStatus::Applied as i32
            );
            let native = get_native(&mut server.client, &value, branch)
                .await
                .entity
                .unwrap();
            assert_eq!(title(&native), "source title");
            if branch.is_some() {
                assert_eq!(
                    mirrored_native(directory.path(), &value.namespace),
                    None,
                    "branch source runs must not write the shared mirror"
                );
            } else {
                assert_eq!(
                    mirrored_native(directory.path(), &value.namespace),
                    Some(native),
                    "source baseline publication must mirror exact current stored native bytes"
                );
            }
        }
    })
}
#[tokio::test]
async fn source_mirror_baseline_create() {
    source_mirror_case(SourceMirrorCase::Create).await;
}
#[tokio::test]
async fn source_mirror_baseline_update() {
    source_mirror_case(SourceMirrorCase::Update).await;
}
#[tokio::test]
async fn source_mirror_baseline_delete() {
    source_mirror_case(SourceMirrorCase::Delete).await;
}
#[tokio::test]
async fn source_mirror_branch_remains_unmirrored() {
    source_mirror_case(SourceMirrorCase::Branch).await;
}
fn source_mirror_recovery_case(later_edit: bool) -> impl Future<Output = ()> {
    Box::pin(async move {
        use delayed_preview_store::{DelayedPreviewStore, PreviewGate, PreviewGatePoint};
        use trogon_atlas_store::{
            source::{SourceDocumentKey, SourcePublicationOutcome},
            Store,
        };
        let (directory, mirror) = source_test_mirror();
        let recovery = Arc::new(PreviewGate::new(PreviewGatePoint::RecoveryUnavailable));
        let store = overlay_test_store().await;
        let mut server = server_with_mirror(
            DelayedPreviewStore {
                inner: store.clone().with_source_publication_observer(Arc::new(
                    CommitPastBaseExpiry(std::sync::atomic::AtomicI64::new(0)),
                )),
                gate: recovery.clone(),
            },
            Some(mirror),
        )
        .await;
        let value = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
        assert_eq!(
            title(&mirrored_native(directory.path(), &value.namespace).unwrap()),
            "before"
        );
        let run = preview(
            &mut server.client,
            &value,
            "mirror-recovery",
            entity(&value.namespace, "source title", "doc"),
            None,
        )
        .await;
        let warm = server
            .client
            .search_entities(req(
                READER,
                pb::SearchEntitiesRequest {
                    query: "before".into(),
                    namespaces: vec![value.namespace.clone()],
                    limit: 10,
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(warm.results.len(), 1);
        recovery
            .armed
            .store(true, std::sync::atomic::Ordering::Release);
        let interrupted = server
            .client
            .apply_source_run(req(
                ADMIN,
                pb::ApplySourceRunRequest {
                    registration_id: run.registration_id.clone(),
                    run_id: run.id.clone(),
                    if_match: run.revision.clone(),
                    preview_digest: run.preview_digest.clone(),
                },
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(interrupted.code(), Code::Unavailable);
        let unknown = server
            .client
            .get_source_run(req(
                READER,
                pb::GetSourceRunRequest {
                    registration_id: run.registration_id.clone(),
                    run_id: run.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        assert!(unknown.recovery_required);
        assert_eq!(unknown.status, pb::SourceRunStatus::OutcomeUnknown as i32);
        assert!(matches!(
            store
                .source_recover(
                    &SourceDocumentKey::new(run.registration_id.clone()).unwrap(),
                    &run.id,
                    &run.preview_digest
                )
                .await
                .unwrap(),
            SourcePublicationOutcome::Committed(_)
        ));
        if later_edit {
            let current = get_native(&mut server.client, &value, None).await;
            let mut entity = current.entity.unwrap();
            let Some(pb::entity::Kind::ReadModel(model)) = entity.kind.as_mut() else {
                panic!("ReadModel")
            };
            model.title = "later native edit".into();
            server
                .client
                .put_entity(req(
                    ADMIN,
                    pb::PutEntityRequest {
                        entity: Some(entity),
                        if_match: current.etag,
                        ..Default::default()
                    },
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(
                title(&mirrored_native(directory.path(), &value.namespace).unwrap()),
                "later native edit",
                "ordinary PutEntity must already mirror the later write"
            );
        }
        let recovered = server
            .client
            .recover_source_run(req(
                ADMIN,
                pb::RecoverSourceRunRequest {
                    registration_id: run.registration_id.clone(),
                    run_id: run.id.clone(),
                    if_match: unknown.revision,
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        assert_eq!(recovered.status, pb::SourceRunStatus::Applied as i32);
        assert!(!recovered.recovery_required);
        let search = server
            .client
            .search_entities(req(
                READER,
                pb::SearchEntitiesRequest {
                    query: if later_edit { "later" } else { "source" }.into(),
                    namespaces: vec![value.namespace.clone()],
                    limit: 10,
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            search.results.len(),
            1,
            "recovered commits must refresh search"
        );
        let native = get_native(&mut server.client, &value, None)
            .await
            .entity
            .unwrap();
        assert_eq!(
            mirrored_native(directory.path(), &value.namespace),
            Some(native),
            "recovery mirror catchup must use current native, not stale marker image"
        );
        let mapping = server
            .client
            .get_source_mapping(req(
                READER,
                pb::GetSourceMappingRequest {
                    registration_id: value.registration.id,
                    mapping_id: value.mapping.id,
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert_eq!(
            title(
                mapping
                    .base
                    .as_ref()
                    .unwrap()
                    .native_value
                    .as_ref()
                    .unwrap()
            ),
            "source title",
            "accepted native base must remain the exact marker image"
        );
    })
}
#[tokio::test]
async fn source_mirror_recovered_commit_catches_up() {
    source_mirror_recovery_case(false).await;
}
#[tokio::test]
async fn source_mirror_recovery_preserves_later_ordinary_mirrored_write() {
    source_mirror_recovery_case(true).await;
}

#[tokio::test]
async fn source_mirror_failure_does_not_rollback_committed_native_or_base() {
    let (directory, mirror) = source_test_mirror();
    let mut server = server_with_mirror(overlay_test_store().await, Some(mirror)).await;
    let value = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    assert_eq!(
        title(&mirrored_native(directory.path(), &value.namespace).unwrap()),
        "before"
    );
    let blocked = directory.path().join("read_models").join(&value.namespace);
    std::fs::remove_dir_all(&blocked).unwrap();
    std::fs::write(&blocked, b"test blocks the mirror directory").unwrap();
    let run = preview(
        &mut server.client,
        &value,
        "mirror-failure",
        entity(&value.namespace, "source title", "doc"),
        None,
    )
    .await;
    let settled = apply(&mut server.client, &run).await;
    assert_eq!(settled.status, pb::SourceRunStatus::Applied as i32);
    assert!(!settled.recovery_required);
    assert_eq!(
        title(
            get_native(&mut server.client, &value, None)
                .await
                .entity
                .as_ref()
                .unwrap()
        ),
        "source title"
    );
    let mapping = server
        .client
        .get_source_mapping(req(
            READER,
            pb::GetSourceMappingRequest {
                registration_id: value.registration.id,
                mapping_id: value.mapping.id,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert_eq!(mapping.last_accepted_run_id, run.id);
    assert_eq!(
        title(
            mapping
                .base
                .as_ref()
                .unwrap()
                .native_value
                .as_ref()
                .unwrap()
        ),
        "source title"
    );
    assert!(blocked.is_file());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn source_apply_preserves_concurrent_recovery_settlement() {
    use delayed_preview_store::{DelayedPreviewStore, PreviewGate, PreviewGatePoint};
    let gate = Arc::new(PreviewGate::new(PreviewGatePoint::Published));
    let _release = ReleaseRetainedPublication(gate.clone());
    let mut server = server(DelayedPreviewStore {
        inner: overlay_test_store().await,
        gate: gate.clone(),
    })
    .await;
    let value = seed(&mut server.client, None, vec![pb::SourceOwnedField::Title]).await;
    let run = preview(
        &mut server.client,
        &value,
        "concurrent-recovery",
        entity(&value.namespace, "source title", "doc"),
        None,
    )
    .await;
    gate.armed.store(true, std::sync::atomic::Ordering::Release);
    let mut apply_client = server.client.clone();
    let apply_request = pb::ApplySourceRunRequest {
        registration_id: run.registration_id.clone(),
        run_id: run.id.clone(),
        if_match: run.revision.clone(),
        preview_digest: run.preview_digest.clone(),
    };
    let applying = tokio::spawn(async move {
        apply_client
            .apply_source_run(req(ADMIN, apply_request, None))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), gate.entered.notified())
        .await
        .unwrap();
    let current = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: run.registration_id.clone(),
                run_id: run.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(current.status, pb::SourceRunStatus::Applying as i32);
    let recover_request = pb::RecoverSourceRunRequest {
        registration_id: run.registration_id.clone(),
        run_id: run.id.clone(),
        if_match: current.revision,
    };
    use pb::event_model_service_server::EventModelService;
    let mut recover_request = req(ADMIN, recover_request, None);
    recover_request
        .extensions_mut()
        .insert(trogon_atlas_server::auth::Principal {
            name: Arc::from("admin"),
            role: trogon_atlas_server::auth::Role::Admin,
            kind: trogon_atlas_server::auth::PrincipalKind::User,
            is_anonymous: false,
            namespaces: Default::default(),
            parent: None,
        });
    let recovering = server.service.recover_source_run(recover_request);
    tokio::pin!(recovering);
    assert!(
        std::future::poll_fn(|cx| { std::task::Poll::Ready(recovering.as_mut().poll(cx)) })
            .await
            .is_pending()
    );
    gate.release.notify_one();
    let recovered = recovering.await.unwrap().into_inner().run.unwrap();
    assert_eq!(recovered.status, pb::SourceRunStatus::Applied as i32);
    let applied = applying.await.unwrap().unwrap().into_inner().run.unwrap();
    assert_eq!(
        applied, recovered,
        "apply must preserve the existing settlement"
    );
    let retained = server
        .client
        .get_source_run(req(
            READER,
            pb::GetSourceRunRequest {
                registration_id: run.registration_id,
                run_id: run.id,
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(retained, recovered);
}

#[tokio::test]
async fn architectural_source_title_only_create_rejects_without_native_or_mapping_base_publication()
{
    for external in [false, true] {
        let nats = trogon_atlas_testsupport::shared().await;
        let mut server = server(nats.store().await).await;
        let namespace = namespace(&mut server.client, "source-architecture", "visible-owner").await;
        let registration = registration(
            &mut server.client,
            "architecture-source",
            &namespace,
            None,
            None,
        )
        .await;
        let target_id = pb::Id {
            namespace: namespace.clone(),
            slug: "target".into(),
            version: 1,
        };
        server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(pb::Entity {
                        kind: Some(pb::entity::Kind::Component(pb::Component {
                            id: Some(target_id.clone()),
                            title: "Target".into(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap();
        let owner_id = pb::Id {
            namespace: namespace.clone(),
            slug: "source-owner".into(),
            version: 1,
        };
        let kind = if external {
            pb::EntityKind::ExternalSystem
        } else {
            pb::EntityKind::Component
        };
        let selection = pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Entity(pb::EntityRef {
                kind: kind as i32,
                id: Some(owner_id.clone()),
            })),
        };
        let mapping = server
            .client
            .create_source_mapping(req(
                ADMIN,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: "architecture-mapping".into(),
                        source: Some(source(&registration.id)),
                        selection: Some(selection),
                        ownership: Some(pb::SourceFieldOwnership {
                            policy: Some(pb::source_field_ownership::Policy::Fields(
                                pb::SourceOwnedFields {
                                    fields: vec![pb::SourceOwnedField::Title as i32],
                                },
                            )),
                        }),
                        ..Default::default()
                    }),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert!(mapping.base.is_none());
        let snapshot = server
            .client
            .accept_source_snapshot(req(
                ADMIN,
                pb::AcceptSourceSnapshotRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    inventory_bytes: inventory(&registration.id, "v1"),
                    files: vec![pb::SourceFile {
                        logical_name: "native/model".into(),
                        media_type: "application/octet-stream".into(),
                        content: b"native supplied content".to_vec(),
                        ..Default::default()
                    }],
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap();
        let relationships = vec![pb::ArchitecturalRelationship {
            target: Some(pb::ArchitecturalEndpoint {
                target: Some(pb::architectural_endpoint::Target::Component(
                    pb::ComponentRef {
                        id: Some(target_id),
                    },
                )),
            }),
            label: Some(pb::CatalogTitle {
                value: "calls".into(),
            }),
            ..Default::default()
        }];
        let incoming = pb::Entity {
            kind: Some(if external {
                pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                    id: Some(owner_id.clone()),
                    title: "Imported".into(),
                    relationships,
                    ..Default::default()
                })
            } else {
                pb::entity::Kind::Component(pb::Component {
                    id: Some(owner_id.clone()),
                    title: "Imported".into(),
                    relationships,
                    ..Default::default()
                })
            }),
            ..Default::default()
        };
        let error = server
            .client
            .preview_source_run(req(
                ADMIN,
                pb::PreviewSourceRunRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    snapshot_id: snapshot.id.clone(),
                    run_id: "architecture-create".into(),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(source(&registration.id)),
                        entity: Some(incoming.clone()),
                    }],
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(
            server
                .client
                .get_entity(req(
                    ADMIN,
                    pb::GetEntityRequest {
                        kind: kind as i32,
                        id: Some(owner_id.clone())
                    },
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
        let after = server
            .client
            .get_source_mapping(req(
                ADMIN,
                pb::GetSourceMappingRequest {
                    registration_id: registration.id.clone(),
                    mapping_id: mapping.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert_eq!(after, mapping);
        let native = server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(incoming),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        let native_entity = native.entity.unwrap();
        let mut adoption = mapping.clone();
        adoption.adoption = Some(pb::SourceAdoption {
            authorized_adoption: true,
            observed: Some(pb::SourceObservedOwner {
                owner: Some(pb::EntityRef {
                    kind: kind as i32,
                    id: Some(owner_id.clone()),
                }),
                uid: native_entity.system.as_ref().unwrap().uid.clone(),
                etag: native.etag.clone(),
            }),
        });
        let adopted = server
            .client
            .update_source_mapping(req(
                ADMIN,
                pb::UpdateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    if_match: mapping.revision,
                    mapping: Some(adoption),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        let mut unrelated = native_entity.clone();
        match unrelated.kind.as_mut().unwrap() {
            pb::entity::Kind::Component(value) => {
                value.title = "Source title".into();
                value.relationships.clear();
            }
            pb::entity::Kind::ExternalSystem(value) => {
                value.title = "Source title".into();
                value.relationships.clear();
            }
            _ => panic!("architectural owner"),
        }
        let run = server
            .client
            .preview_source_run(req(
                ADMIN,
                pb::PreviewSourceRunRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    snapshot_id: snapshot.id,
                    run_id: "architecture-title-update".into(),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(source(&registration.id)),
                        entity: Some(unrelated),
                    }],
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        let applied = apply(&mut server.client, &run).await;
        assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
        let persisted = server
            .client
            .get_entity(req(
                ADMIN,
                pb::GetEntityRequest {
                    kind: kind as i32,
                    id: Some(owner_id),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        assert_eq!(
            trogon_atlas_core::architectural_relationships::relationships(&persisted),
            trogon_atlas_core::architectural_relationships::relationships(&native_entity)
        );
        let current_mapping = server
            .client
            .get_source_mapping(req(
                ADMIN,
                pb::GetSourceMappingRequest {
                    registration_id: registration.id.clone(),
                    mapping_id: adopted.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        let mut whole_owner = current_mapping.clone();
        whole_owner.ownership = Some(pb::SourceFieldOwnership {
            policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                pb::SourceWholeOwnerOwnership {},
            )),
        });
        let error = server
            .client
            .update_source_mapping(req(
                ADMIN,
                pb::UpdateSourceMappingRequest {
                    registration_revision: registration.revision,
                    if_match: current_mapping.revision,
                    mapping: Some(whole_owner),
                },
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
    }
}

#[tokio::test]
async fn communication_source_title_only_create_rejects_without_native_or_mapping_base_publication()
{
    for external in [false, true] {
        let nats = trogon_atlas_testsupport::shared().await;
        let mut server = server(nats.store().await).await;
        let namespace =
            namespace(&mut server.client, "source-communication", "visible-owner").await;
        let registration = registration(
            &mut server.client,
            "communication-source",
            &namespace,
            None,
            None,
        )
        .await;
        let target_id = pb::Id {
            namespace: namespace.clone(),
            slug: "target".into(),
            version: 1,
        };
        server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(pb::Entity {
                        kind: Some(pb::entity::Kind::Component(pb::Component {
                            id: Some(target_id.clone()),
                            title: "Target".into(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap();
        let owner_id = pb::Id {
            namespace: namespace.clone(),
            slug: "source-owner".into(),
            version: 1,
        };
        let kind = if external {
            pb::EntityKind::ExternalSystem
        } else {
            pb::EntityKind::Component
        };
        let selection = pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Entity(pb::EntityRef {
                kind: kind as i32,
                id: Some(owner_id.clone()),
            })),
        };
        let mapping = server
            .client
            .create_source_mapping(req(
                ADMIN,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: "communication-mapping".into(),
                        source: Some(source(&registration.id)),
                        selection: Some(selection),
                        ownership: Some(pb::SourceFieldOwnership {
                            policy: Some(pb::source_field_ownership::Policy::Fields(
                                pb::SourceOwnedFields {
                                    fields: vec![pb::SourceOwnedField::Title as i32],
                                },
                            )),
                        }),
                        ..Default::default()
                    }),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert!(mapping.base.is_none());
        let snapshot = server
            .client
            .accept_source_snapshot(req(
                ADMIN,
                pb::AcceptSourceSnapshotRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    inventory_bytes: inventory(&registration.id, "v1"),
                    files: vec![pb::SourceFile {
                        logical_name: "native/model".into(),
                        media_type: "application/octet-stream".into(),
                        content: b"native supplied content".to_vec(),
                        ..Default::default()
                    }],
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap();
        let specification = server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(pb::Entity {
                        kind: Some(pb::entity::Kind::Specification(pb::Specification {
                            id: Some(pb::Id {
                                namespace: namespace.clone(),
                                slug: "notification-contract".into(),
                                version: 1,
                            }),
                            title: "Notification contract".into(),
                            messages: vec![pb::ContractMessage {
                                name: Some(pb::MemberName {
                                    value: "Notification".into(),
                                }),
                                ..Default::default()
                            }],
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        let Some(pb::entity::Kind::Specification(spec)) = &specification.kind else {
            panic!("specification")
        };
        let communications = vec![pb::CommunicationDeclaration {
            direction: pb::CommunicationDirection::Send as i32,
            peer: Some(pb::ArchitecturalEndpoint {
                target: Some(pb::architectural_endpoint::Target::Component(
                    pb::ComponentRef {
                        id: Some(target_id),
                    },
                )),
            }),
            message: Some(pb::ContractMemberRef {
                specification: Some(pb::SpecificationRef {
                    id: spec.id.clone(),
                }),
                owner_uid: specification.system.as_ref().unwrap().uid.clone(),
                kind: pb::ContractMemberKind::Message as i32,
                member_id: spec.messages[0].member_id.clone(),
                ..Default::default()
            }),
            label: Some(pb::CatalogTitle {
                value: "notifies".into(),
            }),
            ..Default::default()
        }];
        let incoming = pb::Entity {
            kind: Some(if external {
                pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                    id: Some(owner_id.clone()),
                    title: "Imported".into(),
                    communications,
                    ..Default::default()
                })
            } else {
                pb::entity::Kind::Component(pb::Component {
                    id: Some(owner_id.clone()),
                    title: "Imported".into(),
                    communications,
                    ..Default::default()
                })
            }),
            ..Default::default()
        };
        let error = server
            .client
            .preview_source_run(req(
                ADMIN,
                pb::PreviewSourceRunRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    snapshot_id: snapshot.id.clone(),
                    run_id: "communication-create".into(),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(source(&registration.id)),
                        entity: Some(incoming.clone()),
                    }],
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(
            server
                .client
                .get_entity(req(
                    ADMIN,
                    pb::GetEntityRequest {
                        kind: kind as i32,
                        id: Some(owner_id.clone())
                    },
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
        let after = server
            .client
            .get_source_mapping(req(
                ADMIN,
                pb::GetSourceMappingRequest {
                    registration_id: registration.id.clone(),
                    mapping_id: mapping.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert_eq!(after, mapping);
        let native = server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(incoming),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        let native_entity = native.entity.unwrap();
        let mut adoption = mapping.clone();
        adoption.adoption = Some(pb::SourceAdoption {
            authorized_adoption: true,
            observed: Some(pb::SourceObservedOwner {
                owner: Some(pb::EntityRef {
                    kind: kind as i32,
                    id: Some(owner_id.clone()),
                }),
                uid: native_entity.system.as_ref().unwrap().uid.clone(),
                etag: native.etag.clone(),
            }),
        });
        let adopted = server
            .client
            .update_source_mapping(req(
                ADMIN,
                pb::UpdateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    if_match: mapping.revision,
                    mapping: Some(adoption),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        let mut unrelated = native_entity.clone();
        match unrelated.kind.as_mut().unwrap() {
            pb::entity::Kind::Component(value) => {
                value.title = "Source title".into();
                value.communications.clear();
            }
            pb::entity::Kind::ExternalSystem(value) => {
                value.title = "Source title".into();
                value.communications.clear();
            }
            _ => panic!("architectural owner"),
        }
        let run = server
            .client
            .preview_source_run(req(
                ADMIN,
                pb::PreviewSourceRunRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    snapshot_id: snapshot.id,
                    run_id: "communication-title-update".into(),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(source(&registration.id)),
                        entity: Some(unrelated),
                    }],
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        let applied = apply(&mut server.client, &run).await;
        assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
        let persisted = server
            .client
            .get_entity(req(
                ADMIN,
                pb::GetEntityRequest {
                    kind: kind as i32,
                    id: Some(owner_id),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        assert_eq!(
            trogon_atlas_core::communication_intent::declarations(&persisted),
            trogon_atlas_core::communication_intent::declarations(&native_entity)
        );
        let current_mapping = server
            .client
            .get_source_mapping(req(
                ADMIN,
                pb::GetSourceMappingRequest {
                    registration_id: registration.id.clone(),
                    mapping_id: adopted.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        let mut whole_owner = current_mapping.clone();
        whole_owner.ownership = Some(pb::SourceFieldOwnership {
            policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                pb::SourceWholeOwnerOwnership {},
            )),
        });
        let error = server
            .client
            .update_source_mapping(req(
                ADMIN,
                pb::UpdateSourceMappingRequest {
                    registration_revision: registration.revision,
                    if_match: current_mapping.revision,
                    mapping: Some(whole_owner),
                },
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
    }
}

fn native_any(url: &str, value: Vec<u8>) -> prost_types::Any {
    prost_types::Any {
        type_url: format!("type.googleapis.com/{url}"),
        value,
    }
}

fn native_additions(label: &str, target: &pb::Id) -> (bool, prost_types::Any) {
    use prost::Message as _;
    match label {
        "agent" => (
            false,
            native_any(
                "trogonatlas.eventmodel.v1alpha1.AgentAnnotation",
                Vec::new(),
            ),
        ),
        "badges" => (
            true,
            native_any(
                "trogonatlas.annotation.v1alpha1.BadgesAnnotation",
                pb::BadgesAnnotation {
                    badges: vec![pb::Badge {
                        content: Some(pb::BadgeText {
                            value: "beta".into(),
                        }),
                        background_color: Some(pb::HexColor {
                            value: "#aAbBcC".into(),
                        }),
                        text_color: Some(pb::HexColor {
                            value: "#000000".into(),
                        }),
                        ..Default::default()
                    }],
                }
                .encode_to_vec(),
            ),
        ),
        "groups" => (
            true,
            native_any(
                "trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation",
                pb::ResourceGroupsAnnotation {
                    groups: vec![pb::ResourceGroup {
                        items: vec![pb::ResourceGroupItem {
                            target: Some(pb::EntityRef {
                                kind: pb::EntityKind::Component as i32,
                                id: Some(target.clone()),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                }
                .encode_to_vec(),
            ),
        ),
        _ => (
            true,
            native_any(
                "trogonatlas.annotation.v1alpha1.LifecycleAnnotation",
                pb::LifecycleAnnotation {
                    status: "accepted".into(),
                    ..Default::default()
                }
                .encode_to_vec(),
            ),
        ),
    }
}

fn native_owner(
    external: bool,
    id: &pb::Id,
    title: &str,
    metadata: prost_types::Any,
) -> pb::Entity {
    pb::Entity {
        kind: Some(if external {
            pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                id: Some(id.clone()),
                title: title.into(),
                metadata: vec![metadata],
                ..Default::default()
            })
        } else {
            pb::entity::Kind::Component(pb::Component {
                id: Some(id.clone()),
                title: title.into(),
                metadata: vec![metadata],
                ..Default::default()
            })
        }),
        ..Default::default()
    }
}

fn native_ownership(whole: bool) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(if whole {
            pb::source_field_ownership::Policy::WholeOwner(pb::SourceWholeOwnerOwnership {})
        } else {
            pb::source_field_ownership::Policy::Fields(pb::SourceOwnedFields {
                fields: vec![pb::SourceOwnedField::Title as i32],
            })
        }),
    }
}

#[tokio::test]
async fn native_metadata_additions_stay_native_only_across_source_ownership() {
    for (label, external, whole) in [
        ("agent", false, false),
        ("badges", true, false),
        ("groups", true, false),
        ("agent", false, true),
        ("badges", true, true),
        ("groups", true, true),
    ] {
        let nats = trogon_atlas_testsupport::shared().await;
        let mut server = server(nats.store().await).await;
        let namespace = namespace(
            &mut server.client,
            "source-native-metadata",
            "visible-owner",
        )
        .await;
        let registration = registration(
            &mut server.client,
            "metadata-source",
            &namespace,
            None,
            None,
        )
        .await;
        let target_id = pb::Id {
            namespace: namespace.clone(),
            slug: "target".into(),
            version: 1,
        };
        server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(pb::Entity {
                        kind: Some(pb::entity::Kind::Component(pb::Component {
                            id: Some(target_id.clone()),
                            title: "Target".into(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap();
        let owner_id = pb::Id {
            namespace: namespace.clone(),
            slug: "source-owner".into(),
            version: 1,
        };
        let kind = if external {
            pb::EntityKind::ExternalSystem
        } else {
            pb::EntityKind::Component
        };
        let selection = pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Entity(pb::EntityRef {
                kind: kind as i32,
                id: Some(owner_id.clone()),
            })),
        };
        let mapping = server
            .client
            .create_source_mapping(req(
                ADMIN,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: "metadata-mapping".into(),
                        source: Some(source(&registration.id)),
                        selection: Some(selection),
                        ownership: Some(native_ownership(whole)),
                        ..Default::default()
                    }),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        let snapshot = server
            .client
            .accept_source_snapshot(req(
                ADMIN,
                pb::AcceptSourceSnapshotRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    inventory_bytes: inventory(&registration.id, "v1"),
                    files: vec![pb::SourceFile {
                        logical_name: "native/model".into(),
                        media_type: "application/octet-stream".into(),
                        content: b"native supplied content".to_vec(),
                        ..Default::default()
                    }],
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap();
        let preview_client = server.client.clone();
        let preview = |run: &str, entity: pb::Entity| {
            let request = pb::PreviewSourceRunRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                snapshot_id: snapshot.id.clone(),
                run_id: run.into(),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(source(&registration.id)),
                    entity: Some(entity),
                }],
                ..Default::default()
            };
            let mut client = preview_client.clone();
            async move { client.preview_source_run(req(ADMIN, request, None)).await }
        };
        let (_, additions) = native_additions(label, &target_id);
        let rejected = preview(
            "native-create",
            native_owner(external, &owner_id, "Imported", additions.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(
            rejected.code(),
            Code::InvalidArgument,
            "{label} {external} {whole}: {rejected}"
        );
        assert_eq!(
            server
                .client
                .get_entity(req(
                    ADMIN,
                    pb::GetEntityRequest {
                        kind: kind as i32,
                        id: Some(owner_id.clone()),
                    },
                    None,
                ))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
        let (_, lifecycle) = native_additions("lifecycle", &target_id);
        if !whole {
            preview(
                "lifecycle-create",
                native_owner(external, &owner_id, "Imported", lifecycle),
            )
            .await
            .unwrap();
        }
        let _ = mapping;
    }
}

fn metadata_of(entity: &pb::Entity) -> Vec<prost_types::Any> {
    match entity.kind.as_ref().unwrap() {
        pb::entity::Kind::Component(value) => value.metadata.clone(),
        pb::entity::Kind::ExternalSystem(value) => value.metadata.clone(),
        _ => panic!("native owner"),
    }
}

fn retitle(entity: &mut pb::Entity, title: &str, clear_metadata: bool) {
    let (slot, metadata) = match entity.kind.as_mut().unwrap() {
        pb::entity::Kind::Component(value) => (&mut value.title, &mut value.metadata),
        pb::entity::Kind::ExternalSystem(value) => (&mut value.title, &mut value.metadata),
        _ => panic!("native owner"),
    };
    *slot = title.into();
    if clear_metadata {
        metadata.clear();
    }
}

#[tokio::test]
async fn adopted_native_metadata_is_preserved_by_field_specific_sources_and_vetoes_whole_owner() {
    for (label, external, clear) in [
        ("agent", false, false),
        ("agent", false, true),
        ("badges", true, false),
        ("groups", true, true),
    ] {
        let nats = trogon_atlas_testsupport::shared().await;
        let mut server = server(nats.store().await).await;
        let namespace = namespace(
            &mut server.client,
            "source-adopted-metadata",
            "visible-owner",
        )
        .await;
        let registration =
            registration(&mut server.client, "adopted-source", &namespace, None, None).await;
        let target_id = pb::Id {
            namespace: namespace.clone(),
            slug: "target".into(),
            version: 1,
        };
        server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(pb::Entity {
                        kind: Some(pb::entity::Kind::Component(pb::Component {
                            id: Some(target_id.clone()),
                            title: "Target".into(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap();
        let owner_id = pb::Id {
            namespace: namespace.clone(),
            slug: "adopted-owner".into(),
            version: 1,
        };
        let kind = if external {
            pb::EntityKind::ExternalSystem
        } else {
            pb::EntityKind::Component
        };
        let (_, additions) = native_additions(label, &target_id);
        let native = server
            .client
            .put_entity(req(
                ADMIN,
                pb::PutEntityRequest {
                    entity: Some(native_owner(external, &owner_id, "Native", additions)),
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        let native_entity = native.entity.clone().unwrap();
        let owner = pb::EntityRef {
            kind: kind as i32,
            id: Some(owner_id.clone()),
        };
        let selection = pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Entity(owner.clone())),
        };
        let adoption = pb::SourceAdoption {
            authorized_adoption: true,
            observed: Some(pb::SourceObservedOwner {
                owner: Some(owner.clone()),
                uid: native_entity.system.as_ref().unwrap().uid.clone(),
                etag: native.etag.clone(),
            }),
        };
        let whole = server
            .client
            .create_source_mapping(req(
                ADMIN,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: "whole-mapping".into(),
                        source: Some(source(&registration.id)),
                        selection: Some(selection.clone()),
                        ownership: Some(native_ownership(true)),
                        adoption: Some(adoption.clone()),
                        ..Default::default()
                    }),
                },
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(whole.code(), Code::InvalidArgument, "{label}: {whole}");

        let adopted = server
            .client
            .create_source_mapping(req(
                ADMIN,
                pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: "title-mapping".into(),
                        source: Some(source(&registration.id)),
                        selection: Some(selection),
                        ownership: Some(native_ownership(false)),
                        adoption: Some(adoption),
                        ..Default::default()
                    }),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        let snapshot = server
            .client
            .accept_source_snapshot(req(
                ADMIN,
                pb::AcceptSourceSnapshotRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    inventory_bytes: inventory(&registration.id, "v1"),
                    files: vec![pb::SourceFile {
                        logical_name: "native/model".into(),
                        media_type: "application/octet-stream".into(),
                        content: b"native supplied content".to_vec(),
                        ..Default::default()
                    }],
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap();
        let mut incoming = native_entity.clone();
        retitle(&mut incoming, "Source title", clear);
        let run = server
            .client
            .preview_source_run(req(
                ADMIN,
                pb::PreviewSourceRunRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    snapshot_id: snapshot.id,
                    run_id: "adopted-title-update".into(),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(source(&registration.id)),
                        entity: Some(incoming),
                    }],
                    ..Default::default()
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap();
        let applied = apply(&mut server.client, &run).await;
        assert_eq!(
            applied.status,
            pb::SourceRunStatus::Applied as i32,
            "{label}"
        );
        let persisted = server
            .client
            .get_entity(req(
                ADMIN,
                pb::GetEntityRequest {
                    kind: kind as i32,
                    id: Some(owner_id),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        assert_eq!(
            metadata_of(&persisted),
            metadata_of(&native_entity),
            "{label} clear={clear}"
        );

        let current_mapping = server
            .client
            .get_source_mapping(req(
                ADMIN,
                pb::GetSourceMappingRequest {
                    registration_id: registration.id.clone(),
                    mapping_id: adopted.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        let mut whole_owner = current_mapping.clone();
        whole_owner.ownership = Some(native_ownership(true));
        let error = server
            .client
            .update_source_mapping(req(
                ADMIN,
                pb::UpdateSourceMappingRequest {
                    registration_revision: registration.revision,
                    if_match: current_mapping.revision,
                    mapping: Some(whole_owner),
                },
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{label}: {error}");
    }
}
