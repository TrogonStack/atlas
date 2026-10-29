#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{future::Future, sync::Arc};

use support::two_tenants::{authed, make_registry, serve_with_registry, ROOT_TOKEN};
use tonic::{transport::Channel, Code, Status};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{NatsStore, Store};

type Client = pb::event_model_service_client::EventModelServiceClient<Channel>;

#[derive(Clone, Debug, PartialEq)]
struct Variant {
    environments: Vec<pb::EnvironmentName>,
    is_default: Option<bool>,
}

fn variant(environments: &[&str], is_default: Option<bool>) -> Variant {
    Variant {
        environments: environments
            .iter()
            .map(|value| pb::EnvironmentName {
                value: (*value).into(),
            })
            .collect(),
        is_default,
    }
}
fn specification(entity: &pb::Entity) -> &pb::Specification {
    let Some(pb::entity::Kind::Specification(value)) = &entity.kind else {
        panic!("Specification")
    };
    value
}
fn specification_mut(entity: &mut pb::Entity) -> &mut pb::Specification {
    let Some(pb::entity::Kind::Specification(value)) = &mut entity.kind else {
        panic!("Specification")
    };
    value
}
fn fields(fields: &[pb::SourceOwnedField]) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: fields.iter().map(|field| *field as i32).collect(),
            },
        )),
    }
}
fn whole() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::WholeOwner(
            pb::SourceWholeOwnerOwnership {},
        )),
    }
}
fn none() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::None(
            pb::SourceNoOwnership {},
        )),
    }
}

struct Fixture {
    client: Client,
    store: Arc<NatsStore>,
    namespace: String,
    registration: pb::SourceRegistration,
}
impl Fixture {
    async fn new() -> Self {
        let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
        let service =
            EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
        let mut client = serve_with_registry(service, make_registry()).await;
        let namespace = client
            .register_namespace(authed(
                pb::RegisterNamespaceRequest {
                    name: "schema-variant-source".into(),
                    parent: "catalog".into(),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .namespace
            .unwrap()
            .id;
        let registration = client
            .create_source_registration(authed(
                pb::CreateSourceRegistrationRequest {
                    registration: Some(pb::SourceRegistration {
                        id: "schema-variant-source".into(),
                        administration_namespace: namespace.clone(),
                        destination_namespaces: vec![namespace.clone()],
                        profile_revision: 1,
                        ownership_policy_revision: "native-only-v1".into(),
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
        let mut fixture = Self {
            client,
            store,
            namespace,
            registration,
        };
        for slug in ["payload", "payload-other"] {
            fixture
                .put(pb::Entity {
                    kind: Some(pb::entity::Kind::Schema(pb::Schema {
                        id: Some(fixture.id(slug)),
                        title: "Payload".into(),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
                .await;
        }
        fixture
    }
    fn id(&self, slug: &str) -> pb::Id {
        pb::Id {
            namespace: self.namespace.clone(),
            slug: slug.into(),
            version: 1,
        }
    }
    fn owner(&self, key: &str) -> pb::EntityRef {
        pb::EntityRef {
            kind: pb::EntityKind::Specification as i32,
            id: Some(self.id(key)),
        }
    }
    fn source(&self, key: &str) -> pb::SourceResourceKey {
        pb::SourceResourceKey {
            registration_id: self.registration.id.clone(),
            category: "model".into(),
            key: key.into(),
        }
    }
    fn entity(&self, key: &str, value: &Variant) -> pb::Entity {
        pb::Entity {
            kind: Some(pb::entity::Kind::Specification(pb::Specification {
                id: Some(self.id(key)),
                title: key.into(),
                messages: vec![pb::ContractMessage {
                    name: Some(pb::MemberName {
                        value: "Message".into(),
                    }),
                    schemas: vec![pb::SchemaBinding {
                        role: pb::schema_binding::Role::Payload as i32,
                        r#type: Some(pb::schema_binding::Type::Schema(pb::SchemaRef {
                            id: Some(self.id("payload")),
                        })),
                        environments: value.environments.clone(),
                        is_default: value.is_default,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        }
    }
    async fn put(&mut self, entity: pb::Entity) -> pb::PutEntityResponse {
        self.client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(entity),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
    }
    async fn get(&mut self, key: &str) -> pb::GetEntityResponse {
        self.client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Specification as i32,
                    id: Some(self.id(key)),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
    }
    async fn mapping(&mut self, key: &str) -> pb::SourceMapping {
        self.client
            .get_source_mapping(authed(
                pb::GetSourceMappingRequest {
                    registration_id: self.registration.id.clone(),
                    mapping_id: key.into(),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap()
    }
    fn map<'a>(
        &'a mut self,
        key: &'a str,
        policy: pb::SourceFieldOwnership,
        native: Option<&'a pb::PutEntityResponse>,
    ) -> impl Future<Output = Result<pb::SourceMapping, Status>> + 'a {
        Box::pin(async move {
            self.client
                .create_source_mapping(authed(
                    pb::CreateSourceMappingRequest {
                        registration_revision: self.registration.revision.clone(),
                        mapping: Some(pb::SourceMapping {
                            id: key.into(),
                            source: Some(self.source(key)),
                            selection: Some(pb::SourceSelection {
                                selection: Some(pb::source_selection::Selection::Entity(
                                    self.owner(key),
                                )),
                            }),
                            ownership: Some(policy),
                            adoption: native.map(|native| pb::SourceAdoption {
                                authorized_adoption: true,
                                observed: Some(pb::SourceObservedOwner {
                                    owner: Some(self.owner(key)),
                                    uid: native
                                        .entity
                                        .as_ref()
                                        .unwrap()
                                        .system
                                        .as_ref()
                                        .unwrap()
                                        .uid
                                        .clone(),
                                    etag: native.etag.clone(),
                                }),
                            }),
                            ..Default::default()
                        }),
                    },
                    ROOT_TOKEN,
                ))
                .await
                .map(|response| response.into_inner().mapping.unwrap())
        })
    }
    async fn preview(
        &mut self,
        key: &str,
        label: &str,
        entity: pb::Entity,
    ) -> Result<pb::ReconciliationRun, Status> {
        let inventory_bytes = serde_json::to_vec(&serde_json::json!({ "profile": "supplied-native-v1", "profile_revision": 1, "registration": self.registration.id, "source_version": format!("{key}-{label}"), "partition": { "categories": ["model"], "logical_roots": ["native"], "selected_keys": [] }, "coverage": "PARTIAL", "resources": [{ "key": { "category": "model", "key": key }, "file": "native/owners.json", "pointer": { "kind": "native", "raw": format!("#/{key}") } }], "relations": [] })).unwrap();
        let snapshot = self
            .client
            .accept_source_snapshot(authed(
                pb::AcceptSourceSnapshotRequest {
                    registration_id: self.registration.id.clone(),
                    registration_revision: self.registration.revision.clone(),
                    inventory_bytes,
                    files: vec![pb::SourceFile {
                        logical_name: "native/owners.json".into(),
                        media_type: "application/json".into(),
                        content: b"{}\n".to_vec(),
                        ..Default::default()
                    }],
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap();
        self.client
            .preview_source_run(authed(
                pb::PreviewSourceRunRequest {
                    registration_id: self.registration.id.clone(),
                    registration_revision: self.registration.revision.clone(),
                    snapshot_id: snapshot.id,
                    run_id: format!("{key}-{label}"),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(self.source(key)),
                        entity: Some(entity),
                    }],
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .map(|response| response.into_inner().run.unwrap())
    }
    async fn apply(&mut self, run: pb::ReconciliationRun) -> pb::ReconciliationRun {
        self.client
            .apply_source_run(authed(
                pb::ApplySourceRunRequest {
                    registration_id: self.registration.id.clone(),
                    run_id: run.id,
                    if_match: run.revision,
                    preview_digest: run.preview_digest,
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap()
    }
}

#[tokio::test]
async fn source_messages_cannot_publish_native_variant_introduction_change_order_or_clear() {
    let mut f = Fixture::new().await;
    let mut cases = vec![
        (variant(&[], None), variant(&["Blue", "green"], None)),
        (variant(&["Blue", "green"], None), variant(&["red"], None)),
        (
            variant(&["Blue", "green"], None),
            variant(&["green", "Blue"], None),
        ),
        (variant(&["Blue", "green"], None), variant(&[], None)),
    ];
    for old in [None, Some(false), Some(true)] {
        for new in [None, Some(false), Some(true)] {
            if old != new {
                cases.push((variant(&[], old), variant(&[], new)));
            }
        }
    }
    let mut bypasses = Vec::new();
    for (index, (old, new)) in cases.into_iter().enumerate() {
        let key = format!("variant-{index}");
        let native = f.put(f.entity(&key, &old)).await;
        let mapping = f
            .map(
                &key,
                fields(&[pb::SourceOwnedField::Messages]),
                Some(&native),
            )
            .await
            .unwrap();
        assert_eq!(mapping.base.as_ref().unwrap().source_value, native.entity);
        assert_eq!(mapping.base.as_ref().unwrap().native_value, native.entity);
        let before = f.get(&key).await;
        let sequence = f.store.current_change_seq().await.unwrap();
        let mut incoming = native.entity.clone().unwrap();
        let binding = &mut specification_mut(&mut incoming).messages[0].schemas[0];
        binding.environments.clone_from(&new.environments);
        binding.is_default = new.is_default;
        binding.connection_id = None;
        match f.preview(&key, "changed", incoming).await {
            Err(status) => {
                assert_eq!(
                    status.code(),
                    Code::InvalidArgument,
                    "{old:?} -> {new:?}: {status}"
                );
                assert!(
                    status.message().contains("native authoring only"),
                    "{status}"
                );
                assert_eq!(f.get(&key).await, before);
                assert_eq!(f.mapping(&key).await, mapping);
                assert_eq!(f.store.current_change_seq().await.unwrap(), sequence);
            }
            Ok(run) => {
                let applied = f.apply(run).await;
                let after = f.get(&key).await;
                assert_eq!(
                    applied.status,
                    pb::SourceRunStatus::Applied as i32,
                    "{applied:?}"
                );
                let actual = &specification(after.entity.as_ref().unwrap()).messages[0].schemas[0];
                assert_eq!(actual.environments, new.environments);
                assert_eq!(actual.is_default, new.is_default);
                assert_ne!(after.etag, before.etag);
                assert_ne!(f.mapping(&key).await.base, mapping.base);
                bypasses.push(format!("{old:?} -> {new:?}: preview/apply published"));
            }
        }
    }
    assert!(
        bypasses.is_empty(),
        "Source Messages gained native variant authority: {bypasses:#?}"
    );
}

#[tokio::test]
async fn source_message_and_title_updates_preserve_exact_local_variants_and_plain_bindings() {
    let mut f = Fixture::new().await;
    for (key, policy) in [
        ("messages", fields(&[pb::SourceOwnedField::Messages])),
        ("title", fields(&[pb::SourceOwnedField::Title])),
    ] {
        let mut entity = f.entity(key, &variant(&["Blue", "green"], Some(false)));
        let message = &mut specification_mut(&mut entity).messages[0];
        let mut default_binding = message.schemas[0].clone();
        default_binding.environments.clear();
        default_binding.is_default = Some(true);
        message.schemas.push(default_binding);
        let mut plain = message.schemas[0].clone();
        plain.environments.clear();
        plain.is_default = None;
        message.schemas.push(plain);
        let native = f.put(entity).await;
        f.map(key, policy, Some(&native)).await.unwrap();
        let original = native.entity.as_ref().unwrap();
        let mut incoming = original.clone();
        let value = specification_mut(&mut incoming);
        if key == "messages" {
            value.messages[0].doc = Some("Source message documentation".into());
            value.messages[0].schemas[0].doc = Some("Binding documentation".into());
            value.messages[0].schemas[2].doc = Some("Ordinary plain binding".into());
            value.messages[0].schemas[2].r#type =
                Some(pb::schema_binding::Type::Schema(pb::SchemaRef {
                    id: Some(f.id("payload-other")),
                }));
            value.messages[0].schemas[2].connection_id = None;
            let mut added = value.messages[0].schemas[2].clone();
            added.connection_id = None;
            added.role = pb::schema_binding::Role::Headers as i32;
            value.messages[0].schemas.push(added);
        } else {
            value.title = "Source title".into();
            for binding in &mut value.messages[0].schemas {
                binding.environments.clear();
                binding.is_default = None;
            }
        }
        let run = f.preview(key, "supported", incoming).await.unwrap();
        let applied = f.apply(run).await;
        assert_eq!(
            applied.status,
            pb::SourceRunStatus::Applied as i32,
            "{applied:?}"
        );
        let saved = f.get(key).await.entity.unwrap();
        assert_eq!(saved.system, original.system);
        let value = specification(&saved);
        let old = specification(original);
        if key == "title" {
            assert_eq!(value.title, "Source title");
            assert_eq!(value.messages, old.messages);
        } else {
            assert_eq!(
                value.messages[0].doc.as_deref(),
                Some("Source message documentation")
            );
            assert_eq!(value.messages[0].schemas.len(), 4);
            assert_eq!(
                value.messages[0].schemas[0].doc.as_deref(),
                Some("Binding documentation")
            );
            for (binding, before) in value.messages[0]
                .schemas
                .iter()
                .take(2)
                .zip(&old.messages[0].schemas)
            {
                assert_eq!(binding.environments, before.environments);
                assert_eq!(binding.is_default, before.is_default);
                assert_eq!(binding.connection_id, before.connection_id);
            }
            assert_eq!(
                value.messages[0].schemas[2].r#type,
                Some(pb::schema_binding::Type::Schema(pb::SchemaRef {
                    id: Some(f.id("payload-other"))
                }))
            );
            assert!(value.messages[0].schemas[2].connection_id.is_some());
            assert_ne!(
                value.messages[0].schemas[2].connection_id,
                old.messages[0].schemas[2].connection_id
            );
            assert!(value.messages[0].schemas[3].connection_id.is_some());
            assert_eq!(value.messages[0].schemas[3].is_default, None);
            assert_eq!(value.messages[0].schemas[3].environments, []);
        }
    }
    let key = "local-drift";
    let plain = f.put(f.entity(key, &variant(&[], None))).await;
    f.map(key, fields(&[pb::SourceOwnedField::Messages]), Some(&plain))
        .await
        .unwrap();
    let mut local = plain.entity.clone().unwrap();
    let binding = &mut specification_mut(&mut local).messages[0].schemas[0];
    binding.environments = variant(&["Local", "second"], None).environments;
    binding.connection_id = None;
    f.put(local).await;
    let before = f.get(key).await;
    let run = f
        .preview(key, "drift", plain.entity.unwrap())
        .await
        .unwrap();
    let details = f
        .client
        .list_source_run_results(authed(
            pb::ListSourceRunResultsRequest {
                registration_id: f.registration.id.clone(),
                run_id: run.id.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        details
            .results
            .iter()
            .any(|result| result.disposition == pb::SourceResultDisposition::LocalDrift as i32),
        "{details:?}"
    );
    assert_eq!(run.operation_count, 0);
    assert_eq!(
        f.apply(run).await.status,
        pb::SourceRunStatus::Applied as i32
    );
    assert_eq!(f.get(key).await, before);
}

#[tokio::test]
async fn source_variants_keep_all_policy_create_and_whole_owner_isolation() {
    let mut f = Fixture::new().await;
    for (index, policy) in [
        none(),
        whole(),
        fields(&[pb::SourceOwnedField::Title]),
        fields(&[pb::SourceOwnedField::Messages]),
    ]
    .into_iter()
    .enumerate()
    {
        for (variant_index, value) in [
            variant(&["Blue", "green"], None),
            variant(&[], Some(false)),
            variant(&[], Some(true)),
        ]
        .into_iter()
        .enumerate()
        {
            let key = format!("create-{index}-{variant_index}");
            let mapping = f.map(&key, policy.clone(), None).await.unwrap();
            let incoming = f.entity(&key, &value);
            let error = f.preview(&key, "create", incoming).await.unwrap_err();
            assert_eq!(error.code(), Code::InvalidArgument, "{error}");
            assert!(error.message().contains("native authoring only"));
            assert_eq!(f.mapping(&key).await, mapping);
            assert_eq!(
                f.client
                    .get_entity(authed(
                        pb::GetEntityRequest {
                            kind: pb::EntityKind::Specification as i32,
                            id: Some(f.id(&key))
                        },
                        ROOT_TOKEN
                    ))
                    .await
                    .unwrap_err()
                    .code(),
                Code::NotFound
            );
        }
    }
    let key = "whole-incoming";
    let native = f.put(f.entity(key, &variant(&[], None))).await;
    let mapping = f.map(key, whole(), Some(&native)).await.unwrap();
    let before = f.get(key).await;
    let mut proposed = native.entity.clone().unwrap();
    let binding = &mut specification_mut(&mut proposed).messages[0].schemas[0];
    binding.environments = variant(&["Blue"], None).environments;
    binding.connection_id = None;
    assert_eq!(
        f.preview(key, "introduction", proposed.clone())
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(f.mapping(key).await, mapping);
    assert_eq!(f.get(key).await, before);
    f.put(proposed).await;
    let local = f.get(key).await;
    assert_eq!(
        f.preview(key, "clear", native.entity.unwrap())
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(f.mapping(key).await, mapping);
    assert_eq!(f.get(key).await, local);
    let current_key = "whole-current";
    let native = f
        .put(f.entity(current_key, &variant(&[], Some(false))))
        .await;
    assert_eq!(
        f.map(current_key, whole(), Some(&native))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(f.get(current_key).await.entity, native.entity);
}

#[tokio::test]
async fn source_messages_cannot_add_remove_or_reassign_native_variant_occurrences() {
    let mut f = Fixture::new().await;
    let mut bypasses = Vec::new();
    for mode in ["add", "remove", "move"] {
        let initial = if mode == "add" {
            variant(&[], None)
        } else {
            variant(&["Blue", "green"], Some(false))
        };
        let native = f.put(f.entity(mode, &initial)).await;
        let mapping = f
            .map(
                mode,
                fields(&[pb::SourceOwnedField::Messages]),
                Some(&native),
            )
            .await
            .unwrap();
        let before = f.get(mode).await;
        let mut incoming = native.entity.unwrap();
        let spec = specification_mut(&mut incoming);
        match mode {
            "add" => {
                let mut binding = spec.messages[0].schemas[0].clone();
                binding.connection_id = None;
                binding.environments = variant(&["Blue", "green"], None).environments;
                spec.messages[0].schemas.push(binding);
            }
            "remove" => spec.messages[0].schemas.clear(),
            "move" => {
                let mut message = spec.messages[0].clone();
                message.member_id = None;
                message.name.as_mut().unwrap().value = "Other".into();
                for binding in &mut message.schemas {
                    binding.connection_id = None;
                }
                spec.messages[0].schemas.clear();
                spec.messages.push(message);
            }
            _ => unreachable!(),
        }
        match f.preview(mode, "occurrence", incoming).await {
            Ok(run) => {
                assert_eq!(
                    f.apply(run).await.status,
                    pb::SourceRunStatus::Applied as i32
                );
                bypasses.push(mode);
            }
            Err(error) => {
                assert_eq!(error.code(), Code::InvalidArgument, "{error}");
                assert!(error.message().contains("native authoring only"));
                assert_eq!(f.get(mode).await, before);
                assert_eq!(f.mapping(mode).await, mapping);
            }
        }
    }
    assert!(
        bypasses.is_empty(),
        "Source changed native variant occurrences: {bypasses:?}"
    );
}

#[tokio::test]
async fn retained_source_preview_cannot_overwrite_a_later_local_native_variant() {
    let mut f = Fixture::new().await;
    let key = "preview-race";
    let native = f.put(f.entity(key, &variant(&[], None))).await;
    let mapping = f
        .map(
            key,
            fields(&[pb::SourceOwnedField::Messages]),
            Some(&native),
        )
        .await
        .unwrap();
    let mut incoming = native.entity.clone().unwrap();
    specification_mut(&mut incoming).messages[0].doc = Some("Pending Source change".into());
    let run = f.preview(key, "pending", incoming).await.unwrap();
    let mut local = native.entity.unwrap();
    let binding = &mut specification_mut(&mut local).messages[0].schemas[0];
    binding.environments = variant(&["Local", "second"], None).environments;
    binding.is_default = Some(false);
    binding.connection_id = None;
    f.put(local).await;
    let before = f.get(key).await;
    let sequence = f.store.current_change_seq().await.unwrap();
    let error = f
        .client
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
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error.message().contains("not active in this owner"),
        "{error}"
    );
    assert_eq!(f.get(key).await, before);
    assert_eq!(f.mapping(key).await, mapping);
    assert_eq!(f.store.current_change_seq().await.unwrap(), sequence);
}
