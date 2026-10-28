#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{future::Future, sync::Arc};

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tonic::{Code, Request, transport::Channel};
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

fn term(namespace: &str, slug: &str, domain: bool) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Term(pb::Term {
            id: Some(id(namespace, slug)),
            title: "Native vocabulary".into(),
            doc: "Native definition".into(),
            owning_domain: domain.then(|| pb::DomainRef {
                id: Some(id(namespace, "domain")),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}
async fn domain(f: &mut Fixture) {
    f.put(pb::Entity {
        kind: Some(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id(&f.namespace, "domain")),
            ..Default::default()
        })),
        ..Default::default()
    })
    .await;
}
fn fields(field: pb::SourceOwnedField) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: vec![field as i32],
            },
        )),
    }
}
#[tokio::test]
async fn source_cannot_create_domain_declaration_under_any_term_ownership_policy() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    domain(&mut f).await;
    let mut policies = vec![whole_owner(), no_ownership(), title_ownership()];
    policies.extend(
        [
            pb::SourceOwnedField::Doc,
            pb::SourceOwnedField::EmbodiedBy,
            pb::SourceOwnedField::Metadata,
            pb::SourceOwnedField::Supersedes,
        ]
        .map(fields),
    );
    policies.push(pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: vec![
                    pb::SourceOwnedField::Title as i32,
                    pb::SourceOwnedField::Doc as i32,
                    pb::SourceOwnedField::EmbodiedBy as i32,
                    pb::SourceOwnedField::Metadata as i32,
                    pb::SourceOwnedField::Supersedes as i32,
                ],
            },
        )),
    });
    for (index, ownership) in policies.into_iter().enumerate() {
        let key = format!("word-{index}");
        let value = term(&f.namespace, &key, true);
        let selection = owner(&value);
        let proposed = f.proposed_mapping(
            &key,
            pb::source_selection::Selection::Entity(selection.clone()),
            ownership,
            None,
        );
        let mapping = f.create_mapping(proposed).await.unwrap();
        let snapshot = f
            .snapshot(&format!("version-{index}"), "PARTIAL", &[&key])
            .await;
        let payload = f.payload(&key, value);
        let error = f
            .preview(&snapshot, &format!("refused-{index}"), vec![payload], None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(f.mapping(&key).await, mapping);
        assert_eq!(f.get(&selection).await.unwrap_err().code(), Code::NotFound);
    }
}
#[tokio::test]
async fn whole_owner_proposed_current_and_clear_refusals_preserve_accepted_base_and_native() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    domain(&mut f).await;
    let value = term(&f.namespace, "word", false);
    let mapping = f.proposed_mapping(
        "word",
        pb::source_selection::Selection::Entity(owner(&value)),
        whole_owner(),
        None,
    );
    f.create_mapping(mapping).await.unwrap();
    let snapshot = f.snapshot("initial", "PARTIAL", &["word"]).await;
    let payload = f.payload("word", value.clone());
    let run = f
        .preview(&snapshot, "initial", vec![payload], None)
        .await
        .unwrap();
    assert_eq!(
        f.apply(&run).await.unwrap().status,
        pb::SourceRunStatus::Applied as i32
    );
    let baseline = f.get(&owner(&value)).await.unwrap();
    let base = f.mapping("word").await;
    assert_eq!(base.base.as_ref().unwrap().native_value, baseline.entity);
    let payload = f.payload("word", term(&f.namespace, "word", true));
    assert_eq!(
        f.preview(&snapshot, "proposed", vec![payload], None)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(f.mapping("word").await, base);
    assert_eq!(f.get(&owner(&value)).await.unwrap(), baseline);
    let local = f.put(term(&f.namespace, "word", true)).await;
    assert_eq!(
        local.entity.as_ref().unwrap().system,
        baseline.entity.as_ref().unwrap().system
    );
    assert_ne!(local.etag, baseline.etag);
    for (label, proposed) in [
        ("current", term(&f.namespace, "word", true)),
        ("clear", value.clone()),
    ] {
        let payload = f.payload("word", proposed);
        assert_eq!(
            f.preview(&snapshot, label, vec![payload], None)
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
        assert_eq!(f.mapping("word").await, base);
        let actual = f.get(&owner(&value)).await.unwrap();
        assert_eq!(actual.entity, local.entity);
        assert_eq!(actual.etag, local.etag);
    }
    let adoption = f.proposed_mapping(
        "other",
        pb::source_selection::Selection::Entity(owner(local.entity.as_ref().unwrap())),
        whole_owner(),
        Some(adoption(&local)),
    );
    assert_eq!(
        f.create_mapping(adoption).await.unwrap_err().code(),
        Code::InvalidArgument
    );
    assert_eq!(f.mapping("word").await, base);
}
#[tokio::test]
async fn title_source_apply_preserves_local_domain_identity_and_advances_only_accepted_title() {
    let mut f = Fixture::new(pb::SourceDeletionPolicy::RetainNative).await;
    domain(&mut f).await;
    let local = f.put(term(&f.namespace, "word", true)).await;
    let mapping = f.adopt("word", &local, title_ownership()).await;
    let snapshot = f.snapshot("title-update", "PARTIAL", &["word"]).await;
    let mut incoming = term(&f.namespace, "word", false);
    if let Some(pb::entity::Kind::Term(word)) = &mut incoming.kind {
        word.title = "Source title".into();
        word.doc = "Ignored source definition".into();
    }
    let payload = f.payload("word", incoming.clone());
    let run = f
        .preview(&snapshot, "title", vec![payload], None)
        .await
        .unwrap();
    assert_eq!(f.mapping("word").await, mapping);
    assert_eq!(f.get(&owner(&incoming)).await.unwrap().entity, local.entity);
    let applied = f.apply(&run).await.unwrap();
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let actual = f.get(&owner(&incoming)).await.unwrap();
    let mut expected = local.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Term(word)) = &mut expected.kind {
        word.title = "Source title".into();
    }
    assert_eq!(actual.entity.as_ref(), Some(&expected));
    assert_ne!(actual.etag, local.etag);
    let accepted = f.mapping("word").await;
    assert_eq!(accepted.base.as_ref().unwrap().native_value, actual.entity);
    assert_eq!(
        accepted
            .base
            .as_ref()
            .unwrap()
            .observed
            .as_ref()
            .unwrap()
            .etag,
        actual.etag
    );
    assert_eq!(
        accepted.base.as_ref().unwrap().source_value.as_ref(),
        Some(&incoming)
    );
    f.apply(&run).await.unwrap();
    assert_eq!(f.get(&owner(&incoming)).await.unwrap(), actual);
    assert_eq!(f.mapping("word").await, accepted);
}
