#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use support::two_tenants::{authed, make_registry, serve_with_registry, ACME_TOKEN, ROOT_TOKEN};
use tonic::{transport::Channel, Code, Status};
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type Client = EventModelServiceClient<Channel>;
struct Fixture {
    client: Client,
    namespace: String,
    registration: pb::SourceRegistration,
}
fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}
fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}
fn whole() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::WholeOwner(
            pb::SourceWholeOwnerOwnership {},
        )),
    }
}
fn fields(values: Vec<pb::SourceOwnedField>) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: values.into_iter().map(|v| v as i32).collect(),
            },
        )),
    }
}
impl Fixture {
    async fn new() -> Self {
        let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
        let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
        let mut client = serve_with_registry(service, make_registry()).await;
        let namespace = client
            .register_namespace(authed(
                pb::RegisterNamespaceRequest {
                    name: "hosting-source".into(),
                    ..Default::default()
                },
                ACME_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .namespace
            .unwrap()
            .id;
        client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(pb::Entity {
                        kind: Some(pb::entity::Kind::Ui(pb::Ui {
                            id: Some(id(&namespace, "review")),
                            title: "Review".into(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap();
        let registration = client
            .create_source_registration(authed(
                pb::CreateSourceRegistrationRequest {
                    registration: Some(pb::SourceRegistration {
                        id: "hosting-source".into(),
                        administration_namespace: namespace.clone(),
                        destination_namespaces: vec![namespace.clone()],
                        profile_revision: 1,
                        ownership_policy_revision: "native-only".into(),
                        deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                        ..Default::default()
                    }),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .registration
            .unwrap();
        Self {
            client,
            namespace,
            registration,
        }
    }
    fn owner(&self, key: &str, hosted: bool) -> pb::Entity {
        pb::Entity {
            kind: Some(pb::entity::Kind::Component(pb::Component {
                id: Some(id(&self.namespace, key)),
                title: "Native portal".into(),
                ui_hostings: if hosted {
                    vec![pb::UiHosting {
                        ui: Some(pb::UiRef {
                            id: Some(id(&self.namespace, "review")),
                        }),
                        label: Some(pb::CatalogTitle {
                            value: "Review portal".into(),
                        }),
                        doc: Some(pb::CatalogText {
                            value: "Native hosting".into(),
                        }),
                        ..Default::default()
                    }]
                } else {
                    Vec::new()
                },
                ..Default::default()
            })),
            ..Default::default()
        }
    }
    fn source(&self, key: &str) -> pb::SourceResourceKey {
        pb::SourceResourceKey {
            registration_id: self.registration.id.clone(),
            category: "model".into(),
            key: key.into(),
        }
    }
    async fn put(&mut self, value: pb::Entity) -> pb::PutEntityResponse {
        self.client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(value),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
    }
    async fn get(&mut self, key: &str) -> Result<pb::GetEntityResponse, Status> {
        self.client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id(&self.namespace, key)),
                },
                ROOT_TOKEN,
            ))
            .await
            .map(tonic::Response::into_inner)
    }
    async fn mapping(
        &mut self,
        key: &str,
        policy: pb::SourceFieldOwnership,
        native: Option<&pb::PutEntityResponse>,
    ) -> Result<pb::SourceMapping, Status> {
        let incoming = self.owner(key, false);
        self.client
            .create_source_mapping(authed(
                pb::CreateSourceMappingRequest {
                    registration_revision: self.registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: format!("map-{key}"),
                        source: Some(self.source(key)),
                        selection: Some(pb::SourceSelection {
                            selection: Some(pb::source_selection::Selection::Entity(reference(
                                &incoming,
                            ))),
                        }),
                        ownership: Some(policy),
                        adoption: native.map(|saved| pb::SourceAdoption {
                            observed: Some(pb::SourceObservedOwner {
                                owner: Some(reference(saved.entity.as_ref().unwrap())),
                                uid: saved
                                    .entity
                                    .as_ref()
                                    .unwrap()
                                    .system
                                    .as_ref()
                                    .unwrap()
                                    .uid
                                    .clone(),
                                etag: saved.etag.clone(),
                            }),
                            authorized_adoption: true,
                        }),
                        ..Default::default()
                    }),
                },
                ROOT_TOKEN,
            ))
            .await
            .map(|response| response.into_inner().mapping.unwrap())
    }
    async fn mapping_after(&mut self, mapping: &pb::SourceMapping) -> pb::SourceMapping {
        self.client
            .get_source_mapping(authed(
                pb::GetSourceMappingRequest {
                    registration_id: self.registration.id.clone(),
                    mapping_id: mapping.id.clone(),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap()
    }
    async fn snapshot(&mut self, key: &str) -> pb::SourceSnapshot {
        let version = format!("v-{key}");
        let inventory = serde_json::to_vec(&serde_json::json!({
            "profile": "supplied-native-v1", "profile_revision": 1, "registration": self.registration.id,
            "source_version": version,
            "partition": { "categories": ["model"], "logical_roots": ["native"], "selected_keys": [] },
            "coverage": "PARTIAL", "resources": [{ "key": { "category": "model", "key": key }, "file": "native/portal.json", "pointer": { "kind": "native", "raw": "#/portal" } }], "relations": []
        })).unwrap();
        self.client
            .accept_source_snapshot(authed(
                pb::AcceptSourceSnapshotRequest {
                    registration_id: self.registration.id.clone(),
                    registration_revision: self.registration.revision.clone(),
                    inventory_bytes: inventory,
                    files: vec![pb::SourceFile {
                        logical_name: "native/portal.json".into(),
                        media_type: "application/json".into(),
                        content: b"{\"portal\":\"supplied bytes\"}".to_vec(),
                        ..Default::default()
                    }],
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap()
    }
    async fn preview(
        &mut self,
        key: &str,
        incoming: pb::Entity,
    ) -> Result<pb::ReconciliationRun, Status> {
        let snapshot = self.snapshot(key).await;
        self.client
            .preview_source_run(authed(
                pb::PreviewSourceRunRequest {
                    registration_id: self.registration.id.clone(),
                    registration_revision: self.registration.revision.clone(),
                    snapshot_id: snapshot.id,
                    run_id: format!("run-{key}"),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(self.source(key)),
                        entity: Some(incoming),
                    }],
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .map(|response| response.into_inner().run.unwrap())
    }
}

#[tokio::test]
async fn every_source_create_policy_refuses_hosting_without_owner_or_mapping_base_publication() {
    let mut f = Fixture::new().await;
    let owned = [
        pb::SourceOwnedField::Title,
        pb::SourceOwnedField::Doc,
        pb::SourceOwnedField::Tech,
        pb::SourceOwnedField::Members,
        pb::SourceOwnedField::Metadata,
        pb::SourceOwnedField::Supersedes,
    ];
    let mut policies = vec![
        pb::SourceFieldOwnership {
            policy: Some(pb::source_field_ownership::Policy::None(
                pb::SourceNoOwnership {},
            )),
        },
        whole(),
    ];
    policies.extend(owned.map(|field| fields(vec![field])));
    policies.push(fields(owned.to_vec()));
    for (index, policy) in policies.into_iter().enumerate() {
        let key = format!("create-{index}");
        let mapping = Box::pin(f.mapping(&key, policy, None)).await.unwrap();
        assert!(mapping.base.is_none());
        let incoming = f.owner(&key, true);
        let error = f.preview(&key, incoming).await.unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{key}: {error}");
        assert!(
            error.message().contains("native authoring only"),
            "{key}: {error}"
        );
        assert_eq!(f.get(&key).await.unwrap_err().code(), Code::NotFound);
        assert_eq!(f.mapping_after(&mapping).await, mapping);
    }
}

#[tokio::test]
async fn whole_owner_rejects_current_incoming_and_clear_but_title_apply_preserves_hosting() {
    let mut f = Fixture::new().await;
    let current = f.owner("current", true);
    let current = f.put(current).await;
    let before = f.get("current").await.unwrap();
    let error = Box::pin(f.mapping("current", whole(), Some(&current)))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(f.get("current").await.unwrap(), before);
    for (key, add_native) in [("incoming", false), ("clear", true)] {
        let plain = f.owner(key, false);
        let plain = f.put(plain).await;
        let mapping = Box::pin(f.mapping(key, whole(), Some(&plain)))
            .await
            .unwrap();
        if add_native {
            let mut native = plain.entity.clone().unwrap();
            let Some(pb::entity::Kind::Component(value)) = &mut native.kind else {
                panic!()
            };
            let hosted = f.owner(key, true);
            let Some(pb::entity::Kind::Component(hosted)) = hosted.kind else {
                panic!()
            };
            value.ui_hostings = hosted.ui_hostings;
            f.put(native).await;
        }
        let before = f.get(key).await.unwrap();
        let incoming = f.owner(key, !add_native);
        let error = f.preview(key, incoming).await.unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert_eq!(f.get(key).await.unwrap(), before);
        assert_eq!(f.mapping_after(&mapping).await, mapping);
    }
    let mapping = Box::pin(f.mapping(
        "current",
        fields(vec![pb::SourceOwnedField::Title]),
        Some(&current),
    ))
    .await
    .unwrap();
    let mut incoming = f.owner("current", false);
    let Some(pb::entity::Kind::Component(value)) = &mut incoming.kind else {
        panic!()
    };
    value.title = "Source title".into();
    let run = f.preview("current", incoming).await.unwrap();
    f.client
        .apply_source_run(authed(
            pb::ApplySourceRunRequest {
                registration_id: f.registration.id.clone(),
                run_id: run.id,
                if_match: run.revision,
                preview_digest: run.preview_digest,
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let after = f.get("current").await.unwrap();
    let mut expected = current.entity.unwrap();
    let Some(pb::entity::Kind::Component(value)) = &mut expected.kind else {
        panic!()
    };
    value.title = "Source title".into();
    assert_eq!(after.entity, Some(expected));
    let accepted = f.mapping_after(&mapping).await;
    assert_ne!(accepted.revision, mapping.revision);
    assert_eq!(accepted.base.as_ref().unwrap().native_value, after.entity);
}
