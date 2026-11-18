#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fmt::Write as _, future::Future, sync::Arc};

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use sha2::{Digest, Sha256};
use tonic::{transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{BearerAuth, RegistryFile, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "lifecycle-admin-test-token-aaaaaaaa";
const SCOPED_WRITER: &str = "lifecycle-scoped-test-token-bbbbbbbb";

fn request<T>(token: &str, branch: Option<&str>, value: T) -> Request<T> {
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

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn owner(entity: &pb::Entity) -> pb::EntityRef {
    pb::EntityRef {
        kind: trogon_atlas_core::refs::entity_kind(entity).unwrap() as i32,
        id: trogon_atlas_core::refs::entity_id(entity).cloned(),
    }
}

fn read_model(namespace: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id(namespace, slug)),
            title: title.into(),
            doc: "Complete owner documentation".into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn whole_owner() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::WholeOwner(
            pb::SourceWholeOwnerOwnership {},
        )),
    }
}

fn no_ownership() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::None(
            pb::SourceNoOwnership {},
        )),
    }
}

fn title_ownership() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: vec![pb::SourceOwnedField::Title as i32],
            },
        )),
    }
}

struct Fixture {
    client: EventModelServiceClient<Channel>,
    task: tokio::task::JoinHandle<()>,
    namespace: String,
    registration: pb::SourceRegistration,
    branch: Option<String>,
    token: &'static str,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    fn new(deletion_policy: pb::SourceDeletionPolicy) -> impl Future<Output = Self> {
        Box::pin(async move {
            let store = trogon_atlas_testsupport::shared().await.store().await;
            let implementation =
                EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default())
                    .unwrap()
                    .with_snapshot_cap(0);
            let registry: RegistryFile = toml::from_str(&format!(
                r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.scoped]
role = "writer"
parent = "visible-owner"
tokens = ["{SCOPED_WRITER}"]
"#
            ))
            .unwrap();
            let service = trogon_atlas_server::mcp::data_plane(
                EventModelServiceServer::new(implementation),
                BearerAuth::new(Arc::new(
                    TokenRegistry::from_file_contents(registry).unwrap(),
                )),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(service)
                    .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                    .await
                    .unwrap();
            });
            let mut client = EventModelServiceClient::connect(format!("http://{address}"))
                .await
                .unwrap();
            let namespace = client
                .register_namespace(request(
                    ADMIN,
                    None,
                    pb::RegisterNamespaceRequest {
                        name: "source-lifecycle".into(),
                        parent: "visible-owner".into(),
                    },
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id;
            let registration = client
                .create_source_registration(request(
                    ADMIN,
                    None,
                    pb::CreateSourceRegistrationRequest {
                        registration: Some(pb::SourceRegistration {
                            id: "lifecycle-source".into(),
                            administration_namespace: namespace.clone(),
                            destination_namespaces: vec![namespace.clone()],
                            profile_revision: 1,
                            ownership_policy_revision: "policy-v1".into(),
                            deletion_policy: deletion_policy as i32,
                            ..Default::default()
                        }),
                    },
                ))
                .await
                .unwrap()
                .into_inner()
                .registration
                .unwrap();
            Self {
                client,
                task,
                namespace,
                registration,
                branch: None,
                token: ADMIN,
            }
        })
    }

    fn req<T>(&self, value: T) -> Request<T> {
        request(self.token, self.branch.as_deref(), value)
    }

    fn source(&self, key: &str) -> pb::SourceResourceKey {
        pb::SourceResourceKey {
            registration_id: self.registration.id.clone(),
            category: "model".into(),
            key: key.into(),
        }
    }

    fn payload(&self, key: &str, entity: pb::Entity) -> pb::SourceNativePayload {
        pb::SourceNativePayload {
            source: Some(self.source(key)),
            entity: Some(entity),
        }
    }

    async fn snapshot(
        &mut self,
        version: &str,
        coverage: &str,
        keys: &[&str],
    ) -> pb::SourceSnapshot {
        let inventory_bytes = serde_json::to_vec(&serde_json::json!({
            "profile": "supplied-native-v1", "profile_revision": 1,
            "registration": self.registration.id, "source_version": version,
            "partition": {"categories": ["model"], "logical_roots": ["native"], "selected_keys": []},
            "coverage": coverage,
            "resources": keys.iter().map(|key| serde_json::json!({
                "key": {"category": "model", "key": key}, "file": "native/owners.json",
                "pointer": {"kind": "native", "raw": format!("#/{key}")}
            })).collect::<Vec<_>>(), "relations": []
        })).unwrap();
        let req = self.req(pb::AcceptSourceSnapshotRequest {
            registration_id: self.registration.id.clone(),
            registration_revision: self.registration.revision.clone(),
            inventory_bytes,
            files: vec![pb::SourceFile {
                logical_name: "native/owners.json".into(),
                media_type: "application/json".into(),
                content: b"{\"fixture\":\"retained supplied-native bytes\"}\n".to_vec(),
                ..Default::default()
            }],
        });
        self.client
            .accept_source_snapshot(req)
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap()
    }

    async fn put(&mut self, entity: pb::Entity) -> pb::PutEntityResponse {
        let req = self.req(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        });
        self.client.put_entity(req).await.unwrap().into_inner()
    }

    async fn get(
        &mut self,
        selection: &pb::EntityRef,
    ) -> Result<pb::GetEntityResponse, tonic::Status> {
        let req = self.req(pb::GetEntityRequest {
            kind: selection.kind,
            id: selection.id.clone(),
        });
        self.client
            .get_entity(req)
            .await
            .map(tonic::Response::into_inner)
    }

    fn proposed_mapping(
        &self,
        key: &str,
        selection: pb::source_selection::Selection,
        ownership: pb::SourceFieldOwnership,
        adoption: Option<pb::SourceAdoption>,
    ) -> pb::SourceMapping {
        pb::SourceMapping {
            id: format!("mapping-{key}"),
            source: Some(self.source(key)),
            selection: Some(pb::SourceSelection {
                selection: Some(selection),
            }),
            ownership: Some(ownership),
            adoption,
            branch: self.branch.clone(),
            ..Default::default()
        }
    }

    fn create_mapping(
        &mut self,
        mapping: pb::SourceMapping,
    ) -> impl Future<Output = Result<pb::SourceMapping, tonic::Status>> + '_ {
        Box::pin(async move {
            let req = self.req(pb::CreateSourceMappingRequest {
                mapping: Some(mapping),
                registration_revision: self.registration.revision.clone(),
            });
            self.client
                .create_source_mapping(req)
                .await
                .map(|r| r.into_inner().mapping.unwrap())
        })
    }

    async fn mapping(&mut self, key: &str) -> pb::SourceMapping {
        let req = self.req(pb::GetSourceMappingRequest {
            registration_id: self.registration.id.clone(),
            mapping_id: format!("mapping-{key}"),
        });
        self.client
            .get_source_mapping(req)
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap()
    }

    fn map_absent<'a>(
        &'a mut self,
        key: &'a str,
        entity: &'a pb::Entity,
    ) -> impl Future<Output = pb::SourceMapping> + 'a {
        Box::pin(async move {
            let mapping = self.proposed_mapping(
                key,
                pb::source_selection::Selection::Entity(owner(entity)),
                whole_owner(),
                None,
            );
            self.create_mapping(mapping).await.unwrap()
        })
    }

    fn adopt<'a>(
        &'a mut self,
        key: &'a str,
        native: &'a pb::PutEntityResponse,
        ownership: pb::SourceFieldOwnership,
    ) -> impl Future<Output = pb::SourceMapping> + 'a {
        Box::pin(async move {
            let entity = native.entity.as_ref().unwrap();
            let mapping = self.proposed_mapping(
                key,
                pb::source_selection::Selection::Entity(owner(entity)),
                ownership,
                Some(adoption(native)),
            );
            self.create_mapping(mapping).await.unwrap()
        })
    }

    fn preview_request(
        &self,
        snapshot: &pb::SourceSnapshot,
        run_id: &str,
        payloads: Vec<pb::SourceNativePayload>,
        base: Option<&pb::SourceSnapshot>,
    ) -> pb::PreviewSourceRunRequest {
        pb::PreviewSourceRunRequest {
            registration_id: self.registration.id.clone(),
            registration_revision: self.registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: run_id.into(),
            native_payloads: payloads,
            completeness: base.map(|base| pb::SourceCompletenessDecision {
                verified_complete: true,
                inventory_digest: snapshot.inventory_digest.clone(),
                base_snapshot_id: base.id.clone(),
            }),
            ..Default::default()
        }
    }

    async fn preview(
        &mut self,
        snapshot: &pb::SourceSnapshot,
        run_id: &str,
        payloads: Vec<pb::SourceNativePayload>,
        base: Option<&pb::SourceSnapshot>,
    ) -> Result<pb::ReconciliationRun, tonic::Status> {
        let req = self.req(self.preview_request(snapshot, run_id, payloads, base));
        self.client
            .preview_source_run(req)
            .await
            .map(|r| r.into_inner().run.unwrap())
    }

    async fn apply(
        &mut self,
        run: &pb::ReconciliationRun,
    ) -> Result<pb::ReconciliationRun, tonic::Status> {
        let req = self.req(pb::ApplySourceRunRequest {
            registration_id: self.registration.id.clone(),
            run_id: run.id.clone(),
            if_match: run.revision.clone(),
            preview_digest: run.preview_digest.clone(),
        });
        self.client
            .apply_source_run(req)
            .await
            .map(|r| r.into_inner().run.unwrap())
    }

    async fn results(&mut self, run: &pb::ReconciliationRun) -> Vec<pb::SourceRunResult> {
        let mut results = Vec::new();
        let mut page_token = String::new();
        for _ in 0..100 {
            let req = self.req(pb::ListSourceRunResultsRequest {
                registration_id: self.registration.id.clone(),
                run_id: run.id.clone(),
                page_size: 1,
                page_token,
            });
            let page = self
                .client
                .list_source_run_results(req)
                .await
                .unwrap()
                .into_inner();
            assert_eq!(page.run_revision, run.revision);
            assert_eq!(page.preview_digest, run.preview_digest);
            results.extend(page.results);
            if page.next_page_token.is_empty() {
                return results;
            }
            page_token = page.next_page_token;
        }
        panic!("bounded result pagination did not finish")
    }

    async fn operations(
        &mut self,
        run: &pb::ReconciliationRun,
    ) -> Vec<pb::SourceRunOperationEntry> {
        let mut entries = Vec::new();
        let mut page_token = String::new();
        for _ in 0..100 {
            let req = self.req(pb::ListSourceRunOperationsRequest {
                registration_id: self.registration.id.clone(),
                run_id: run.id.clone(),
                page_size: 1,
                page_token,
            });
            let page = self
                .client
                .list_source_run_operations(req)
                .await
                .unwrap()
                .into_inner();
            assert_eq!(page.run_revision, run.revision);
            assert_eq!(page.preview_digest, run.preview_digest);
            entries.extend(page.entries);
            if page.next_page_token.is_empty() {
                return entries;
            }
            page_token = page.next_page_token;
        }
        panic!("bounded operation pagination did not finish")
    }

    fn accepted_owner(&mut self) -> impl Future<Output = (pb::SourceSnapshot, pb::EntityRef)> + '_ {
        Box::pin(async move {
            let incoming = read_model(&self.namespace, "receipt", "Source receipt");
            let selection = owner(&incoming);
            self.map_absent("receipt", &incoming).await;
            let snapshot = self.snapshot("revision-A", "COMPLETE", &["receipt"]).await;
            let payload = self.payload("receipt", incoming);
            let run = self
                .preview(&snapshot, "initial-create", vec![payload], None)
                .await
                .unwrap();
            assert_eq!(
                self.apply(&run).await.unwrap().status,
                pb::SourceRunStatus::Applied as i32
            );
            (snapshot, selection)
        })
    }
}

fn adoption(native: &pb::PutEntityResponse) -> pb::SourceAdoption {
    let entity = native.entity.as_ref().unwrap();
    pb::SourceAdoption {
        observed: Some(pb::SourceObservedOwner {
            owner: Some(owner(entity)),
            uid: entity.system.as_ref().unwrap().uid.clone(),
            etag: native.etag.clone(),
        }),
        authorized_adoption: true,
    }
}

fn has_disposition(
    results: &[pb::SourceRunResult],
    disposition: pb::SourceResultDisposition,
) -> bool {
    results.iter().any(|r| r.disposition == disposition as i32)
}

fn assert_admission_status(status: &tonic::Status) {
    assert!(
        matches!(
            status.code(),
            Code::InvalidArgument
                | Code::FailedPrecondition
                | Code::Aborted
                | Code::PermissionDenied
                | Code::NotFound
        ),
        "an operational failure is not proof of a semantic refusal: {status:?}"
    );
}

fn refused<T: std::fmt::Debug>(result: Result<T, tonic::Status>) {
    assert_admission_status(&result.expect_err("invalid or stale request must be refused"));
}

#[tokio::test]
async fn absent_typed_owner_creates_once_and_accepts_returned_identity_as_base() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let incoming = read_model(&f.namespace, "receipt", "Source receipt");
    let selection = owner(&incoming);
    assert_eq!(f.get(&selection).await.unwrap_err().code(), Code::NotFound);
    let mapping = f.map_absent("receipt", &incoming).await;
    assert!(mapping.base.is_none());
    assert_eq!(mapping.observed.as_ref().unwrap().uid, "");
    let snapshot = f.snapshot("00.RC-A", "PARTIAL", &["receipt"]).await;
    let payload = f.payload("receipt", incoming.clone());
    let run = f
        .preview(&snapshot, "create", vec![payload], None)
        .await
        .unwrap();
    assert!(has_disposition(
        &f.results(&run).await,
        pb::SourceResultDisposition::Create
    ));
    let entries = f.operations(&run).await;
    let puts: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            let Some(pb::source_run_operation_entry::Entry::Operation(operation)) = &entry.entry
            else {
                return None;
            };
            let Some(pb::source_native_operation::Operation::Put(put)) = &operation.operation
            else {
                return None;
            };
            Some(put)
        })
        .collect();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].entity.as_ref(), Some(&incoming));
    assert_eq!(
        puts[0].guard.as_ref().unwrap().owner.as_ref(),
        Some(&selection)
    );
    assert_eq!(
        puts[0].guard.as_ref().unwrap().condition,
        Some(pb::source_native_guard::Condition::Absent(true))
    );
    assert_eq!(f.get(&selection).await.unwrap_err().code(), Code::NotFound);
    assert_eq!(
        f.mapping("receipt").await,
        mapping,
        "preview cannot advance the base"
    );
    let applied = f.apply(&run).await.unwrap();
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let native = f.get(&selection).await.unwrap();
    let uid = &native.entity.as_ref().unwrap().system.as_ref().unwrap().uid;
    assert_ne!(uid, "");
    assert!(trogon_atlas_core::semantically_equal(
        native.entity.as_ref().unwrap(),
        &incoming
    ));
    let accepted = f.mapping("receipt").await;
    assert_eq!(accepted.last_accepted_run_id, applied.id);
    let base = accepted.base.as_ref().unwrap();
    assert_eq!(base.snapshot_id, snapshot.id);
    assert_eq!(base.source_version, "00.RC-A");
    assert_eq!(base.observed.as_ref().unwrap().uid, *uid);
    assert_eq!(base.observed.as_ref().unwrap().etag, native.etag);
    assert_eq!(base.native_value, native.entity);
    assert_eq!(base.source_value.as_ref(), Some(&incoming));
    let replay = f.apply(&run).await.unwrap();
    assert_eq!(replay.status, pb::SourceRunStatus::Applied as i32);
    assert_eq!(f.get(&selection).await.unwrap(), native);
    assert_eq!(f.mapping("receipt").await, accepted);
}

#[tokio::test]
async fn existing_owner_requires_explicit_current_uid_and_etag_adoption() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let native = f
        .put(read_model(&f.namespace, "receipt", "Local receipt"))
        .await;
    let other = f
        .put(read_model(&f.namespace, "other", "Other receipt"))
        .await;
    let entity = native.entity.as_ref().unwrap();
    let valid = adoption(&native);
    for (label, decision) in [
        ("missing", None),
        (
            "unauthorized",
            Some(pb::SourceAdoption {
                authorized_adoption: false,
                ..valid.clone()
            }),
        ),
        (
            "wrong-uid",
            Some(pb::SourceAdoption {
                observed: Some(pb::SourceObservedOwner {
                    uid: other
                        .entity
                        .as_ref()
                        .unwrap()
                        .system
                        .as_ref()
                        .unwrap()
                        .uid
                        .clone(),
                    ..valid.observed.clone().unwrap()
                }),
                ..valid.clone()
            }),
        ),
        (
            "wrong-etag",
            Some(pb::SourceAdoption {
                observed: Some(pb::SourceObservedOwner {
                    etag: "stale-etag".into(),
                    ..valid.observed.clone().unwrap()
                }),
                ..valid.clone()
            }),
        ),
    ] {
        let proposed = f.proposed_mapping(
            "receipt",
            pb::source_selection::Selection::Entity(owner(entity)),
            title_ownership(),
            decision,
        );
        let status = f.create_mapping(proposed).await.expect_err(label);
        assert_admission_status(&status);
        assert_eq!(f.get(&owner(entity)).await.unwrap().etag, native.etag);
    }
    let adopted = f.adopt("receipt", &native, title_ownership()).await;
    assert_eq!(adopted.observed, valid.observed);
    assert_eq!(adopted.base.unwrap().native_value, native.entity);
}

#[tokio::test]
async fn changed_registration_invalidates_preview_without_advancing_native_or_base() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let native = f
        .put(read_model(&f.namespace, "receipt", "Local receipt"))
        .await;
    let mapping = f.adopt("receipt", &native, title_ownership()).await;
    let snapshot = f.snapshot("revision-A", "PARTIAL", &["receipt"]).await;
    let payload = f.payload(
        "receipt",
        read_model(&f.namespace, "receipt", "Source edit"),
    );
    let run = f
        .preview(&snapshot, "config-guard", vec![payload], None)
        .await
        .unwrap();
    let old_revision = f.registration.revision.clone();
    let mut changed = f.registration.clone();
    changed.retention.as_mut().unwrap().preview_seconds -= 1;
    let req = f.req(pb::UpdateSourceRegistrationRequest {
        registration: Some(changed.clone()),
        if_match: old_revision.clone(),
    });
    f.registration = f
        .client
        .update_source_registration(req)
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let req = f.req(pb::UpdateSourceRegistrationRequest {
        registration: Some(changed),
        if_match: old_revision,
    });
    refused(f.client.update_source_registration(req).await);
    refused(f.apply(&run).await);
    assert_eq!(
        f.get(&owner(native.entity.as_ref().unwrap()))
            .await
            .unwrap()
            .etag,
        native.etag
    );
    assert_eq!(f.mapping("receipt").await, mapping);
}

#[tokio::test]
async fn changed_mapping_revision_invalidates_preview_and_rejects_stale_mapping_edit() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let native = f
        .put(read_model(&f.namespace, "receipt", "Local receipt"))
        .await;
    let mapping = f.adopt("receipt", &native, title_ownership()).await;
    let snapshot = f.snapshot("revision-A", "PARTIAL", &["receipt"]).await;
    let payload = f.payload(
        "receipt",
        read_model(&f.namespace, "receipt", "Source edit"),
    );
    let run = f
        .preview(&snapshot, "mapping-guard", vec![payload], None)
        .await
        .unwrap();
    let update = pb::UpdateSourceMappingRequest {
        mapping: Some(mapping.clone()),
        if_match: mapping.revision.clone(),
        registration_revision: f.registration.revision.clone(),
    };
    let req = f.req(update.clone());
    let current = f
        .client
        .update_source_mapping(req)
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert_ne!(current.revision, mapping.revision);
    let req = f.req(update);
    refused(f.client.update_source_mapping(req).await);
    refused(f.apply(&run).await);
    assert_eq!(
        f.get(&owner(native.entity.as_ref().unwrap()))
            .await
            .unwrap()
            .etag,
        native.etag
    );
    assert_eq!(f.mapping("receipt").await, current);
}

#[tokio::test]
async fn partial_omission_retains_native_and_accepted_mapping() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let (_, selection) = f.accepted_owner().await;
    let native = f.get(&selection).await.unwrap();
    let mapping = f.mapping("receipt").await;
    let partial = f.snapshot("partial-omission", "PARTIAL", &[]).await;
    let run = f
        .preview(&partial, "partial-omission", vec![], None)
        .await
        .unwrap();
    let results = f.results(&run).await;
    assert!(!has_disposition(
        &results,
        pb::SourceResultDisposition::Delete
    ));
    assert!(!has_disposition(
        &results,
        pb::SourceResultDisposition::Detach
    ));
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    assert_eq!(f.get(&selection).await.unwrap(), native);
    assert_eq!(f.mapping("receipt").await, mapping);
}

#[tokio::test]
async fn complete_omission_with_retain_policy_retires_mapping_and_keeps_native() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let (base, selection) = f.accepted_owner().await;
    let native = f.get(&selection).await.unwrap();
    let complete = f.snapshot("complete-omission", "COMPLETE", &[]).await;
    let run = f
        .preview(&complete, "retire", vec![], Some(&base))
        .await
        .unwrap();
    assert!(has_disposition(
        &f.results(&run).await,
        pb::SourceResultDisposition::Detach
    ));
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    assert!(f.mapping("receipt").await.detached);
    assert_eq!(f.get(&selection).await.unwrap(), native);
}

#[tokio::test]
async fn safe_delete_policy_cannot_delete_an_owner_with_only_title_ownership() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let native = f
        .put(read_model(&f.namespace, "receipt", "Local receipt"))
        .await;
    f.adopt("receipt", &native, title_ownership()).await;
    let base = f.snapshot("title-base", "COMPLETE", &["receipt"]).await;
    let payload = f.payload(
        "receipt",
        read_model(&f.namespace, "receipt", "Source receipt"),
    );
    let run = f
        .preview(&base, "title-update", vec![payload], None)
        .await
        .unwrap();
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    let selection = owner(native.entity.as_ref().unwrap());
    let accepted = f.get(&selection).await.unwrap();
    let complete = f.snapshot("title-omitted", "COMPLETE", &[]).await;
    let run = f
        .preview(&complete, "title-retire", vec![], Some(&base))
        .await
        .unwrap();
    let results = f.results(&run).await;
    assert!(has_disposition(
        &results,
        pb::SourceResultDisposition::Detach
    ));
    assert!(!has_disposition(
        &results,
        pb::SourceResultDisposition::Delete
    ));
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    assert!(f.mapping("receipt").await.detached);
    assert_eq!(f.get(&selection).await.unwrap(), accepted);
}

#[tokio::test]
async fn partial_snapshot_cannot_be_promoted_into_complete_deletion_authority() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let (base, selection) = f.accepted_owner().await;
    let native = f.get(&selection).await.unwrap();
    let mapping = f.mapping("receipt").await;
    let partial = f.snapshot("partial-claim", "PARTIAL", &[]).await;
    refused(
        f.preview(&partial, "false-completeness", vec![], Some(&base))
            .await,
    );
    assert_eq!(f.get(&selection).await.unwrap(), native);
    assert_eq!(f.mapping("receipt").await, mapping);
}

#[tokio::test]
async fn explicit_mapping_detach_keeps_native_and_historical_base() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let (_, selection) = f.accepted_owner().await;
    let native = f.get(&selection).await.unwrap();
    let mapping = f.mapping("receipt").await;
    let req = f.req(pb::DeleteSourceMappingRequest {
        registration_id: f.registration.id.clone(),
        mapping_id: mapping.id.clone(),
        if_match: mapping.revision.clone(),
        registration_revision: f.registration.revision.clone(),
    });
    f.client.delete_source_mapping(req).await.unwrap();
    let detached = f.mapping("receipt").await;
    assert!(detached.detached);
    assert_eq!(detached.base, mapping.base);
    assert_eq!(f.get(&selection).await.unwrap(), native);
}

#[tokio::test]
async fn exclusive_complete_omission_safely_deletes_unreferenced_whole_owner() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let (base, selection) = f.accepted_owner().await;
    let complete = f.snapshot("deleted", "COMPLETE", &[]).await;
    let run = f
        .preview(&complete, "delete", vec![], Some(&base))
        .await
        .unwrap();
    assert!(has_disposition(
        &f.results(&run).await,
        pb::SourceResultDisposition::Delete
    ));
    assert!(f.get(&selection).await.is_ok(), "preview cannot delete");
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    assert_eq!(f.get(&selection).await.unwrap_err().code(), Code::NotFound);
    let mapping = f.mapping("receipt").await;
    assert!(mapping.detached);
    assert_eq!(mapping.last_accepted_run_id, run.id);
}

fn linked_document(namespace: &str, slug: &str, target: pb::EntityRef) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Document(pb::Document {
            id: Some(id(namespace, slug)),
            title: Some(pb::CatalogTitle {
                value: "Reference note".into(),
            }),
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.invalid/inert".into(),
            })),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Entity(target)),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[tokio::test]
async fn scoped_writer_cannot_claim_complete_inventory_for_source_deletion() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let (base, selection) = f.accepted_owner().await;
    let native = f.get(&selection).await.unwrap();
    let mapping = f.mapping("receipt").await;
    let hidden = f
        .client
        .register_namespace(request(
            ADMIN,
            None,
            pb::RegisterNamespaceRequest {
                name: "hidden-reference".into(),
                parent: "hidden-owner".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let referrer = f
        .put(linked_document(&hidden, "private-note", selection.clone()))
        .await;
    f.token = SCOPED_WRITER;
    assert_eq!(
        f.get(&owner(referrer.entity.as_ref().unwrap()))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let complete = f.snapshot("hidden-delete", "COMPLETE", &[]).await;
    let refusal = f
        .preview(&complete, "hidden-delete", vec![], Some(&base))
        .await
        .expect_err("COMPLETE inventory verification requires an administrator");
    assert_eq!(refusal.code(), Code::PermissionDenied);
    let detail = format!("{refusal:?}");
    for secret in [&hidden, "private-note", "Reference note"] {
        assert!(
            !detail.contains(secret),
            "refusal must not disclose hidden referrer: {detail}"
        );
    }
    assert_eq!(f.get(&selection).await.unwrap(), native);
    assert_eq!(f.mapping("receipt").await, mapping);
    f.token = ADMIN;
    assert_eq!(
        f.get(&owner(referrer.entity.as_ref().unwrap()))
            .await
            .unwrap()
            .entity,
        referrer.entity
    );
}

#[tokio::test]
async fn surviving_referrer_blocks_administrator_source_safe_delete() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let (base, selection) = f.accepted_owner().await;
    let native = f.get(&selection).await.unwrap();
    let mapping = f.mapping("receipt").await;
    let foreign = f
        .client
        .register_namespace(request(
            ADMIN,
            None,
            pb::RegisterNamespaceRequest {
                name: "surviving-reference".into(),
                parent: "other-owner".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let referrer = f
        .put(linked_document(
            &foreign,
            "surviving-note",
            selection.clone(),
        ))
        .await;
    let complete = f.snapshot("blocked-delete", "COMPLETE", &[]).await;
    match f
        .preview(&complete, "blocked-delete", vec![], Some(&base))
        .await
    {
        Err(status) => assert_dependency_refusal(&status),
        Ok(run) => match f.apply(&run).await {
            Err(status) => assert_dependency_refusal(&status),
            Ok(settled) => assert_eq!(
                settled.status,
                pb::SourceRunStatus::Rejected as i32,
                "surviving referrer must prevent native publication"
            ),
        },
    }
    assert_eq!(f.get(&selection).await.unwrap(), native);
    assert_eq!(f.mapping("receipt").await, mapping);
    assert_eq!(
        f.get(&owner(referrer.entity.as_ref().unwrap()))
            .await
            .unwrap()
            .entity,
        referrer.entity
    );
    let referrer_owner = owner(referrer.entity.as_ref().unwrap());
    let req = f.req(pb::DeleteEntityRequest {
        kind: referrer_owner.kind,
        id: referrer_owner.id,
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        if_match: referrer.etag,
        ..Default::default()
    });
    f.client.delete_entity(req).await.unwrap();
    let run = f
        .preview(&complete, "after-referrer-removal", vec![], Some(&base))
        .await
        .unwrap();
    assert!(has_disposition(
        &f.results(&run).await,
        pb::SourceResultDisposition::Delete
    ));
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    assert_eq!(f.get(&selection).await.unwrap_err().code(), Code::NotFound);
    assert!(f.mapping("receipt").await.detached);
}

fn assert_dependency_refusal(status: &tonic::Status) {
    assert!(
        matches!(status.code(), Code::FailedPrecondition | Code::Aborted),
        "authorized COMPLETE deletion must reach dependency admission: {status:?}"
    );
}

#[tokio::test]
async fn final_overlay_can_remove_source_owned_referrer_and_target_together() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::ExclusiveSafeDelete).await;
    let (_, target) = f.accepted_owner().await;
    let document = linked_document(&f.namespace, "note", target.clone());
    let referrer = owner(&document);
    f.map_absent("note", &document).await;
    let target_value = f.get(&target).await.unwrap().entity.unwrap();
    let base = f
        .snapshot("with-referrer", "COMPLETE", &["receipt", "note"])
        .await;
    let payloads = vec![
        f.payload("receipt", target_value),
        f.payload("note", document),
    ];
    let run = f
        .preview(&base, "add-referrer", payloads, None)
        .await
        .unwrap();
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    assert!(f.get(&referrer).await.is_ok());
    let complete = f.snapshot("remove-both", "COMPLETE", &[]).await;
    let run = f
        .preview(&complete, "remove-both", vec![], Some(&base))
        .await
        .unwrap();
    let results = f.results(&run).await;
    for key in ["receipt", "note"] {
        assert!(results.iter().any(|r| r.source.as_ref().unwrap().key == key
            && r.disposition == pb::SourceResultDisposition::Delete as i32));
    }
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    assert_eq!(f.get(&target).await.unwrap_err().code(), Code::NotFound);
    assert_eq!(f.get(&referrer).await.unwrap_err().code(), Code::NotFound);
    assert!(f.mapping("receipt").await.detached);
    assert!(f.mapping("note").await.detached);
}

#[tokio::test]
async fn branch_apply_changes_only_branch_native_and_branch_mapping() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let (_, selection) = f.accepted_owner().await;
    let baseline = f.get(&selection).await.unwrap();
    let baseline_mapping = f.mapping("receipt").await;
    f.client
        .create_branch(request(
            ADMIN,
            None,
            pb::CreateBranchRequest {
                name: "source-review".into(),
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    f.branch = Some("source-review".into());
    let branch_native = f.get(&selection).await.unwrap();
    let adoption_response = pb::PutEntityResponse {
        entity: branch_native.entity.clone(),
        etag: branch_native.etag.clone(),
        ..Default::default()
    };
    let mut branch_mapping = f.proposed_mapping(
        "receipt",
        pb::source_selection::Selection::Entity(selection.clone()),
        whole_owner(),
        Some(adoption(&adoption_response)),
    );
    branch_mapping.id = "branch-receipt".into();
    f.create_mapping(branch_mapping).await.unwrap();
    let snapshot = f.snapshot("branch-revision", "PARTIAL", &["receipt"]).await;
    let payload = f.payload(
        "receipt",
        read_model(&f.namespace, "receipt", "Branch receipt"),
    );
    let run = f
        .preview(&snapshot, "branch-update", vec![payload], None)
        .await
        .unwrap();
    assert_eq!(run.branch.as_deref(), Some("source-review"));
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    let changed = f.get(&selection).await.unwrap();
    let Some(pb::entity::Kind::ReadModel(model)) = &changed.entity.as_ref().unwrap().kind else {
        panic!("read model")
    };
    assert_eq!(model.title, "Branch receipt");
    let req = f.req(pb::GetSourceMappingRequest {
        registration_id: f.registration.id.clone(),
        mapping_id: "branch-receipt".into(),
    });
    let mapping = f
        .client
        .get_source_mapping(req)
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert_eq!(
        mapping.base.unwrap().branch.as_deref(),
        Some("source-review")
    );
    f.branch = None;
    assert_eq!(f.get(&selection).await.unwrap(), baseline);
    assert_eq!(f.mapping("receipt").await, baseline_mapping);
}

#[tokio::test]
async fn cancellation_before_apply_prevents_native_write_and_base_advance() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let (_, selection) = f.accepted_owner().await;
    let native = f.get(&selection).await.unwrap();
    let mapping = f.mapping("receipt").await;
    let snapshot = f
        .snapshot("cancelled-revision", "PARTIAL", &["receipt"])
        .await;
    let payload = f.payload(
        "receipt",
        read_model(&f.namespace, "receipt", "Never published"),
    );
    let run = f
        .preview(&snapshot, "cancel", vec![payload], None)
        .await
        .unwrap();
    let req = f.req(pb::CancelSourceRunRequest {
        registration_id: f.registration.id.clone(),
        run_id: run.id.clone(),
        if_match: run.revision.clone(),
    });
    let cancelled = f
        .client
        .cancel_source_run(req)
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(cancelled.status, pb::SourceRunStatus::Cancelled as i32);
    assert_eq!(
        cancelled.cancellation,
        pb::SourceCancellationDisposition::PreventedPublication as i32
    );
    assert!(!cancelled.recovery_required);
    let replay = f.apply(&cancelled).await.unwrap();
    assert_eq!(replay.status, pb::SourceRunStatus::Cancelled as i32);
    assert_eq!(
        replay.cancellation,
        pb::SourceCancellationDisposition::PreventedPublication as i32
    );
    assert_eq!(f.get(&selection).await.unwrap(), native);
    assert_eq!(f.mapping("receipt").await, mapping);
    let req = f.req(pb::GetSourceSnapshotRequest {
        registration_id: f.registration.id.clone(),
        snapshot_id: snapshot.id,
        include_bytes: false,
    });
    assert!(f
        .client
        .get_source_snapshot(req)
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap()
        .accepted_at_unix_ms
        .is_none());
}

#[tokio::test]
async fn report_continuations_reject_a_changed_run_revision() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let native = f
        .put(read_model(&f.namespace, "receipt", "Native receipt"))
        .await;
    f.adopt("receipt", &native, title_ownership()).await;
    let snapshot = f.snapshot("report-revision", "PARTIAL", &["receipt"]).await;
    let payload = f.payload(
        "receipt",
        read_model(&f.namespace, "receipt", "Source receipt"),
    );
    let run = f
        .preview(&snapshot, "reports", vec![payload], None)
        .await
        .unwrap();
    let req = f.req(pb::ListSourceRunResultsRequest {
        registration_id: f.registration.id.clone(),
        run_id: run.id.clone(),
        page_size: 1,
        ..Default::default()
    });
    let results = f
        .client
        .list_source_run_results(req)
        .await
        .unwrap()
        .into_inner();
    let req = f.req(pb::ListSourceRunOperationsRequest {
        registration_id: f.registration.id.clone(),
        run_id: run.id.clone(),
        page_size: 1,
        ..Default::default()
    });
    let operations = f
        .client
        .list_source_run_operations(req)
        .await
        .unwrap()
        .into_inner();
    assert_ne!(results.next_page_token, "");
    assert_ne!(operations.next_page_token, "");
    let req = f.req(pb::CancelSourceRunRequest {
        registration_id: f.registration.id.clone(),
        run_id: run.id.clone(),
        if_match: run.revision.clone(),
    });
    let cancelled = f
        .client
        .cancel_source_run(req)
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_ne!(cancelled.revision, run.revision);
    let req = f.req(pb::ListSourceRunResultsRequest {
        registration_id: f.registration.id.clone(),
        run_id: run.id.clone(),
        page_size: 1,
        page_token: results.next_page_token,
    });
    refused(f.client.list_source_run_results(req).await);
    let req = f.req(pb::ListSourceRunOperationsRequest {
        registration_id: f.registration.id.clone(),
        run_id: run.id.clone(),
        page_size: 1,
        page_token: operations.next_page_token,
    });
    refused(f.client.list_source_run_operations(req).await);
    assert_ne!(
        f.results(&cancelled).await,
        Vec::<pb::SourceRunResult>::new(),
        "fresh continuation starts from the new revision"
    );
}

#[tokio::test]
async fn exact_repeated_connection_selection_is_descriptive_and_rejects_wrong_uid_or_kind() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    f.put(read_model(&f.namespace, "receipt", "Receipt")).await;
    let processor = pb::Entity {
        kind: Some(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id(&f.namespace, "observe")),
            title: "Notifier".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let processor = f.put(processor).await;
    let native = f
        .put(pb::Entity {
            kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(id(&f.namespace, "observe")),
                title: "Observe receipt".into(),
                source_read_models: (0..2)
                    .map(|_| pb::ReadModelEdge {
                        read_model: Some(pb::ReadModelRef {
                            id: Some(id(&f.namespace, "receipt")),
                        }),
                        ..Default::default()
                    })
                    .collect(),
                processor: Some(pb::ProcessorEdge {
                    processor: Some(pb::ProcessorRef {
                        id: Some(id(&f.namespace, "observe")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        })
        .await;
    let entity = native.entity.as_ref().unwrap();
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &entity.kind else {
        panic!("slice")
    };
    assert_ne!(
        slice.source_read_models[0].connection_id,
        slice.source_read_models[1].connection_id
    );
    let connection = pb::ConnectionRef {
        owner: Some(owner(entity)),
        owner_uid: entity.system.as_ref().unwrap().uid.clone(),
        id: slice.source_read_models[1].connection_id.clone(),
    };
    let mut wrong_kind = connection.clone();
    wrong_kind.owner.as_mut().unwrap().kind = pb::EntityKind::Processor as i32;
    for (key, selected) in [
        (
            "wrong-uid",
            pb::ConnectionRef {
                owner_uid: processor
                    .entity
                    .as_ref()
                    .unwrap()
                    .system
                    .as_ref()
                    .unwrap()
                    .uid
                    .clone(),
                ..connection.clone()
            },
        ),
        ("wrong-kind", wrong_kind),
    ] {
        let mapping = f.proposed_mapping(
            key,
            pb::source_selection::Selection::Connection(selected),
            no_ownership(),
            Some(adoption(&native)),
        );
        let status = f.create_mapping(mapping).await.expect_err(key);
        assert_admission_status(&status);
    }
    let writable = f.proposed_mapping(
        "writable",
        pb::source_selection::Selection::Connection(connection.clone()),
        whole_owner(),
        Some(adoption(&native)),
    );
    refused(f.create_mapping(writable).await);
    let proposed = f.proposed_mapping(
        "occurrence",
        pb::source_selection::Selection::Connection(connection.clone()),
        no_ownership(),
        Some(adoption(&native)),
    );
    let mapping = f.create_mapping(proposed).await.unwrap();
    assert_eq!(
        mapping.selection.as_ref().unwrap().selection,
        Some(pb::source_selection::Selection::Connection(connection))
    );
    assert_descriptive_selection(&mut f, "occurrence", &native, &mapping).await;
}

#[tokio::test]
async fn exact_document_anchor_selection_is_descriptive_and_rejects_wrong_uid_or_anchor() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let subject = f.put(read_model(&f.namespace, "receipt", "Receipt")).await;
    let content = "# Receipt\n\n## Delivery\n";
    let digest = content_digest(content);
    let native = f
        .put(pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(id(&f.namespace, "note")),
                title: Some(pb::CatalogTitle {
                    value: "Receipt note".into(),
                }),
                subjects: vec![pb::SubjectAssociation {
                    subject: Some(pb::CatalogSubject {
                        selection: Some(pb::catalog_subject::Selection::Entity(owner(
                            subject.entity.as_ref().unwrap(),
                        ))),
                    }),
                    ..Default::default()
                }],
                body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                    media_type: Some(pb::MediaType {
                        value: "text/markdown".into(),
                    }),
                    content: content.into(),
                    digest: Some(pb::ContentDigest { sha256: digest }),
                })),
                anchors: vec![pb::DocumentAnchor {
                    fragment: Some(pb::AnchorFragment {
                        value: "delivery".into(),
                    }),
                    label: Some(pb::CatalogTitle {
                        value: "Delivery".into(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        })
        .await;
    let entity = native.entity.as_ref().unwrap();
    let Some(pb::entity::Kind::Document(document)) = &entity.kind else {
        panic!("document")
    };
    let Some(pb::document::Body::Retained(body)) = &document.body else {
        panic!("retained")
    };
    let mut foreign_document = document.clone();
    foreign_document.id = Some(id(&f.namespace, "other-note"));
    for anchor in &mut foreign_document.anchors {
        anchor.anchor_id = None;
    }
    for subject in &mut foreign_document.subjects {
        subject.connection_id = None;
    }
    let foreign = f
        .put(pb::Entity {
            kind: Some(pb::entity::Kind::Document(foreign_document)),
            ..Default::default()
        })
        .await;
    let foreign_entity = foreign.entity.as_ref().unwrap();
    let Some(pb::entity::Kind::Document(foreign_document)) = &foreign_entity.kind else {
        panic!("document")
    };
    let anchor = pb::DocumentAnchorRef {
        document: Some(pb::DocumentRef {
            id: document.id.clone(),
        }),
        owner_uid: entity.system.as_ref().unwrap().uid.clone(),
        anchor_id: document.anchors[0].anchor_id.clone(),
        expected_digest: body.digest.clone(),
    };
    for (key, selected) in [
        (
            "wrong-uid",
            pb::DocumentAnchorRef {
                owner_uid: foreign_entity.system.as_ref().unwrap().uid.clone(),
                ..anchor.clone()
            },
        ),
        (
            "wrong-anchor",
            pb::DocumentAnchorRef {
                anchor_id: foreign_document.anchors[0].anchor_id.clone(),
                ..anchor.clone()
            },
        ),
        (
            "wrong-digest",
            pb::DocumentAnchorRef {
                expected_digest: Some(pb::ContentDigest {
                    sha256: "0".repeat(64),
                }),
                ..anchor.clone()
            },
        ),
    ] {
        let mapping = f.proposed_mapping(
            key,
            anchor_selection(selected),
            no_ownership(),
            Some(adoption(&native)),
        );
        let status = f.create_mapping(mapping).await.expect_err(key);
        assert_admission_status(&status);
    }
    let proposed = f.proposed_mapping(
        "anchor",
        anchor_selection(anchor.clone()),
        no_ownership(),
        Some(adoption(&native)),
    );
    let mapping = f.create_mapping(proposed).await.unwrap();
    assert_eq!(
        mapping.selection.as_ref().unwrap().selection,
        Some(anchor_selection(anchor))
    );
    assert_descriptive_selection(&mut f, "anchor", &native, &mapping).await;
}

fn content_digest(content: &str) -> String {
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(content.as_bytes()) {
        write!(digest, "{byte:02x}").unwrap();
    }
    digest
}

async fn specification_owner(f: &mut Fixture, slug: &str) -> pb::PutEntityResponse {
    let content = "type Order { id: ID! }\n";
    let first = f
        .put(pb::Entity {
            kind: Some(pb::entity::Kind::Specification(pb::Specification {
                id: Some(id(&f.namespace, slug)),
                title: "Order contract".into(),
                artifacts: vec![pb::ContractArtifact {
                    name: Some(pb::MemberName {
                        value: "Order source".into(),
                    }),
                    path: Some(pb::ArtifactPath {
                        value: "order.graphql".into(),
                    }),
                    format: Some(pb::ContractFormat {
                        kind: Some(pb::contract_format::Kind::Known(
                            pb::contract_format::Known::Graphql as i32,
                        )),
                    }),
                    media_type: Some(pb::MediaType {
                        value: "application/graphql".into(),
                    }),
                    content: content.into(),
                    digest: Some(pb::ContentDigest {
                        sha256: content_digest(content),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        })
        .await;
    let mut entity = first.entity.unwrap();
    let Some(pb::entity::Kind::Specification(spec)) = &mut entity.kind else {
        panic!("Specification")
    };
    spec.schemas.push(pb::ContractSchema {
        name: Some(pb::MemberName {
            value: "Order shape".into(),
        }),
        definition: Some(pb::SchemaDefinition {
            target: Some(pb::schema_definition::Target::Artifact(
                pb::LocalArtifactSelection {
                    artifact_id: spec.artifacts[0].member_id.clone(),
                    expected_digest: spec.artifacts[0].digest.clone(),
                },
            )),
            selector: Some(pb::OpaqueTypeSelector {
                syntax: "graphql-type".into(),
                value: "Order".into(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    });
    let req = f.req(pb::PutEntityRequest {
        entity: Some(entity),
        if_match: first.etag,
        ..Default::default()
    });
    f.client.put_entity(req).await.unwrap().into_inner()
}

fn specification_value(native: &pb::PutEntityResponse) -> &pb::Specification {
    let Some(pb::entity::Kind::Specification(spec)) = &native.entity.as_ref().unwrap().kind else {
        panic!("Specification")
    };
    spec
}

fn assert_source_selection<'a>(
    f: &'a mut Fixture,
    native: &'a pb::PutEntityResponse,
    selection: pb::source_selection::Selection,
) -> impl Future<Output = ()> + 'a {
    Box::pin(async move {
        let proposed = f.proposed_mapping(
            "selection",
            selection.clone(),
            no_ownership(),
            Some(adoption(native)),
        );
        let mapping = f.create_mapping(proposed).await.unwrap();
        assert_eq!(
            f.mapping("selection").await.selection,
            Some(pb::SourceSelection {
                selection: Some(selection)
            })
        );
        assert_descriptive_selection(f, "selection", native, &mapping).await;
    })
}

#[tokio::test]
async fn source_contract_member_mapping_preserves_exact_artifact_and_refuses_stale_pins() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let native = specification_owner(&mut f, "contract").await;
    let foreign = specification_owner(&mut f, "other-contract").await;
    let spec = specification_value(&native);
    let selected = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: spec.id.clone(),
        }),
        owner_uid: native
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
            .clone(),
        kind: pb::ContractMemberKind::Artifact as i32,
        member_id: spec.artifacts[0].member_id.clone(),
        expected_digest: spec.artifacts[0].digest.clone(),
    };
    for (key, invalid) in [
        (
            "uid",
            pb::ContractMemberRef {
                owner_uid: foreign
                    .entity
                    .as_ref()
                    .unwrap()
                    .system
                    .as_ref()
                    .unwrap()
                    .uid
                    .clone(),
                ..selected.clone()
            },
        ),
        (
            "kind",
            pb::ContractMemberRef {
                kind: pb::ContractMemberKind::Schema as i32,
                expected_digest: None,
                ..selected.clone()
            },
        ),
        (
            "member",
            pb::ContractMemberRef {
                member_id: specification_value(&foreign).artifacts[0].member_id.clone(),
                ..selected.clone()
            },
        ),
        (
            "digest",
            pb::ContractMemberRef {
                expected_digest: Some(pb::ContentDigest {
                    sha256: "0".repeat(64),
                }),
                ..selected.clone()
            },
        ),
    ] {
        let mapping = f.proposed_mapping(
            key,
            pb::source_selection::Selection::ContractMember(invalid),
            no_ownership(),
            Some(adoption(&native)),
        );
        refused(f.create_mapping(mapping).await);
    }
    assert_source_selection(
        &mut f,
        &native,
        pb::source_selection::Selection::ContractMember(selected),
    )
    .await;
}

#[tokio::test]
async fn source_contract_type_mapping_preserves_artifact_schema_and_selector_presence() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let native = specification_owner(&mut f, "contract").await;
    let foreign = specification_owner(&mut f, "other-contract").await;
    let spec = specification_value(&native);
    let schema = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: spec.id.clone(),
        }),
        owner_uid: native
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
            .clone(),
        kind: pb::ContractMemberKind::Schema as i32,
        member_id: spec.schemas[0].member_id.clone(),
        expected_digest: None,
    };
    let selected = pb::ContractTypeRef {
        schema: Some(schema.clone()),
        expected_artifact_id: spec.artifacts[0].member_id.clone(),
        expected_digest: spec.artifacts[0].digest.clone(),
        expected_selector: spec.schemas[0]
            .definition
            .as_ref()
            .unwrap()
            .selector
            .clone(),
    };
    for (key, invalid) in [
        (
            "uid",
            pb::ContractTypeRef {
                schema: Some(pb::ContractMemberRef {
                    owner_uid: foreign
                        .entity
                        .as_ref()
                        .unwrap()
                        .system
                        .as_ref()
                        .unwrap()
                        .uid
                        .clone(),
                    ..schema.clone()
                }),
                ..selected.clone()
            },
        ),
        (
            "kind",
            pb::ContractTypeRef {
                schema: Some(pb::ContractMemberRef {
                    kind: pb::ContractMemberKind::Artifact as i32,
                    member_id: spec.artifacts[0].member_id.clone(),
                    ..schema
                }),
                ..selected.clone()
            },
        ),
        (
            "artifact",
            pb::ContractTypeRef {
                expected_artifact_id: specification_value(&foreign).artifacts[0].member_id.clone(),
                ..selected.clone()
            },
        ),
        (
            "digest",
            pb::ContractTypeRef {
                expected_digest: Some(pb::ContentDigest {
                    sha256: "0".repeat(64),
                }),
                ..selected.clone()
            },
        ),
        (
            "selector-absence",
            pb::ContractTypeRef {
                expected_selector: None,
                ..selected.clone()
            },
        ),
    ] {
        let mapping = f.proposed_mapping(
            key,
            pb::source_selection::Selection::ContractType(invalid),
            no_ownership(),
            Some(adoption(&native)),
        );
        refused(f.create_mapping(mapping).await);
    }
    assert_source_selection(
        &mut f,
        &native,
        pb::source_selection::Selection::ContractType(selected),
    )
    .await;
}

async fn product_owner(
    f: &mut Fixture,
    slug: &str,
    read_model: pb::ReadModelRef,
) -> pb::PutEntityResponse {
    f.put(pb::Entity {
        kind: Some(pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(id(&f.namespace, slug)),
            title: "Receipt data".into(),
            inputs: vec![pb::DataInputPort {
                name: Some(pb::PortName {
                    value: "Input".into(),
                }),
                data: Some(pb::InputDataSelection {
                    target: Some(pb::input_data_selection::Target::ReadModel(
                        read_model.clone(),
                    )),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Output".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(read_model),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    })
    .await
}

fn product_value(native: &pb::PutEntityResponse) -> &pb::DataProduct {
    let Some(pb::entity::Kind::DataProduct(product)) = &native.entity.as_ref().unwrap().kind else {
        panic!("DataProduct")
    };
    product
}

#[tokio::test]
async fn source_data_port_mapping_preserves_output_direction_and_refuses_foreign_identity() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    let read_model = f.put(read_model(&f.namespace, "receipt", "Receipt")).await;
    let reference = pb::ReadModelRef {
        id: owner(read_model.entity.as_ref().unwrap()).id,
    };
    let native = product_owner(&mut f, "product", reference.clone()).await;
    let foreign = product_owner(&mut f, "other-product", reference).await;
    let product = product_value(&native);
    let selected = pb::DataPortRef {
        product: Some(pb::DataProductRef {
            id: product.id.clone(),
        }),
        owner_uid: native
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
            .clone(),
        direction: pb::DataPortDirection::Output as i32,
        port_id: product.outputs[0].port_id.clone(),
    };
    for (key, invalid) in [
        (
            "uid",
            pb::DataPortRef {
                owner_uid: foreign
                    .entity
                    .as_ref()
                    .unwrap()
                    .system
                    .as_ref()
                    .unwrap()
                    .uid
                    .clone(),
                ..selected.clone()
            },
        ),
        (
            "direction",
            pb::DataPortRef {
                direction: pb::DataPortDirection::Input as i32,
                ..selected.clone()
            },
        ),
        (
            "input-id",
            pb::DataPortRef {
                port_id: product.inputs[0].port_id.clone(),
                ..selected.clone()
            },
        ),
        (
            "foreign-id",
            pb::DataPortRef {
                port_id: product_value(&foreign).outputs[0].port_id.clone(),
                ..selected.clone()
            },
        ),
    ] {
        let mapping = f.proposed_mapping(
            key,
            pb::source_selection::Selection::DataPort(invalid),
            no_ownership(),
            Some(adoption(&native)),
        );
        refused(f.create_mapping(mapping).await);
    }
    assert_source_selection(
        &mut f,
        &native,
        pb::source_selection::Selection::DataPort(selected),
    )
    .await;
}

fn anchor_selection(anchor: pb::DocumentAnchorRef) -> pb::source_selection::Selection {
    pb::source_selection::Selection::Document(pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Anchor(anchor)),
    })
}

async fn assert_descriptive_selection(
    f: &mut Fixture,
    key: &str,
    native: &pb::PutEntityResponse,
    mapping: &pb::SourceMapping,
) {
    let snapshot = f.snapshot("descriptive", "PARTIAL", &[key]).await;
    let req = f.req(pb::GetSourceOverlayRequest {
        registration_id: f.registration.id.clone(),
        snapshot_id: snapshot.id.clone(),
        page_size: 10,
        ..Default::default()
    });
    let overlay = f
        .client
        .get_source_overlay(req)
        .await
        .unwrap()
        .into_inner()
        .overlay
        .unwrap();
    assert_eq!(overlay.entries.len(), 1);
    assert_eq!(
        overlay.entries[0].status,
        pb::SourceOverlayStatus::Resolved as i32
    );
    assert_eq!(
        overlay.entries[0].authorized_candidates,
        vec![mapping.selection.clone().unwrap()]
    );
    let payload = f.payload(key, native.entity.clone().unwrap());
    let run = f
        .preview(&snapshot, "descriptive", vec![payload], None)
        .await
        .unwrap();
    let results = f.results(&run).await;
    assert!(has_disposition(
        &results,
        pb::SourceResultDisposition::Deferred
    ));
    assert!(!has_disposition(
        &results,
        pb::SourceResultDisposition::Update
    ));
    assert_eq!(run.operation_count, 0);
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    let current = f
        .get(&owner(native.entity.as_ref().unwrap()))
        .await
        .unwrap();
    assert_eq!(current.entity, native.entity);
    assert_eq!(current.etag, native.etag);
    assert_eq!(f.mapping(key).await.base, mapping.base);
}
