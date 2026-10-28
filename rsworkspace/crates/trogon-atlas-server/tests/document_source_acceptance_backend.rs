#![allow(clippy::unwrap_used, clippy::expect_used, clippy::large_futures)]

//! Public Source transport acceptance for native Document catalog-context fields.

use std::{future::Future, sync::Arc};

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use prost::Message;
use tonic::{Code, Request, transport::Channel};
use trogon_atlas_core::document_catalog_context::{COLLECTION, bytes_content_digest};
use trogon_atlas_proto as pb;

const ADMIN: &str = "document-source-admin-token-aaaa";

#[derive(Clone, Copy, Debug)]
enum NativeExtension {
    BinaryOnly,
    CollectionOnly,
    Purpose,
    Filename,
    RelatedContent,
}

struct Fixture {
    client: EventModelServiceClient<Channel>,
    task: tokio::task::JoinHandle<()>,
    namespace: String,
    registration: pb::SourceRegistration,
    subject: pb::CatalogSubject,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn req<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {ADMIN}").parse().unwrap());
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

fn title_only() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: vec![pb::SourceOwnedField::Title as i32],
            },
        )),
    }
}

fn whole_owner() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::WholeOwner(
            pb::SourceWholeOwnerOwnership {},
        )),
    }
}

fn field_owner(field: pb::SourceOwnedField) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: vec![field as i32],
            },
        )),
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

fn fixture() -> impl Future<Output = Fixture> {
    Box::pin(async move {
        use trogon_atlas_server::{
            auth::{BearerAuth, RegistryFile, TokenRegistry},
            metrics::ServerMetrics,
            service::EventModelServiceImpl,
        };
        let store = trogon_atlas_testsupport::shared().await.store().await;
        let implementation =
            EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
        let registry: RegistryFile = toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
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
            .register_namespace(req(pb::RegisterNamespaceRequest {
                name: "document-source".into(),
                parent: "visible-owner".into(),
            }))
            .await
            .unwrap()
            .into_inner()
            .namespace
            .unwrap()
            .id;
        let registration = client
            .create_source_registration(req(pb::CreateSourceRegistrationRequest {
                registration: Some(pb::SourceRegistration {
                    id: "doc-native-source".into(),
                    administration_namespace: namespace.clone(),
                    destination_namespaces: vec![namespace.clone()],
                    profile_revision: 1,
                    ownership_policy_revision: "policy-v1".into(),
                    deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                    ..Default::default()
                }),
            }))
            .await
            .unwrap()
            .into_inner()
            .registration
            .unwrap();
        let receipt = client
            .put_entity(req(pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                        id: Some(id(&namespace, "receipt")),
                        title: "Receipt".into(),
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
        let subject = pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Entity(owner(&receipt))),
        };
        Fixture {
            client,
            task,
            namespace,
            registration,
            subject,
        }
    })
}

impl Fixture {
    fn source(&self, key: &str) -> pb::SourceResourceKey {
        pb::SourceResourceKey {
            registration_id: self.registration.id.clone(),
            category: "model".into(),
            key: key.into(),
        }
    }

    async fn snapshot(&mut self, key: &str) -> pb::SourceSnapshot {
        let version = format!("v-{key}");
        let inventory = serde_json::to_vec(&serde_json::json!({
            "profile": "supplied-native-v1", "profile_revision": 1,
            "registration": self.registration.id, "source_version": version,
            "partition": {"categories": ["model"], "logical_roots": ["native"], "selected_keys": []},
            "coverage": "PARTIAL",
            "resources": [{"key": {"category": "model", "key": key}, "file": "native/doc.json",
                "pointer": {"kind": "native", "raw": "#/doc"}}],
            "relations": []
        }))
        .unwrap();
        self.client
            .accept_source_snapshot(req(pb::AcceptSourceSnapshotRequest {
                registration_id: self.registration.id.clone(),
                registration_revision: self.registration.revision.clone(),
                inventory_bytes: inventory,
                files: vec![pb::SourceFile {
                    logical_name: "native/doc.json".into(),
                    media_type: "application/json".into(),
                    content: b"{\"doc\":\"supplied bytes\"}\n".to_vec(),
                    ..Default::default()
                }],
            }))
            .await
            .unwrap()
            .into_inner()
            .snapshot
            .unwrap()
    }

    fn base_document(&self, slug: &str) -> pb::Entity {
        pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(id(&self.namespace, slug)),
                title: Some(pb::CatalogTitle {
                    value: format!("Native {slug}"),
                }),
                subjects: vec![pb::SubjectAssociation {
                    subject: Some(self.subject.clone()),
                    ..Default::default()
                }],
                body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                    value: "https://example.com/doc".into(),
                })),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn apply_extension(&self, entity: &mut pb::Entity, extension: NativeExtension) {
        let Some(pb::entity::Kind::Document(document)) = &mut entity.kind else {
            panic!("document");
        };
        match extension {
            NativeExtension::BinaryOnly => {
                document.body = Some(pb::document::Body::RetainedContent(pb::RetainedContent {
                    media_type: Some(pb::MediaType {
                        value: "application/json".into(),
                    }),
                    content: vec![0xff, 0x00],
                    digest: Some(bytes_content_digest(&[0xff, 0x00])),
                }));
                document.anchors.clear();
            }
            NativeExtension::CollectionOnly => {
                document.metadata = vec![prost_types::Any {
                    type_url: format!("type.googleapis.com/{COLLECTION}"),
                    value: pb::DocumentCollectionAnnotation {
                        category: Some(pb::CatalogTitle {
                            value: "Guides".into(),
                        }),
                        ..Default::default()
                    }
                    .encode_to_vec(),
                }];
            }
            NativeExtension::Purpose => {
                document.purpose = Some(pb::DocumentPurpose::Diagram as i32);
            }
            NativeExtension::Filename => {
                document.filename = Some(pb::LogicalFilename {
                    value: "diagram.svg".into(),
                });
            }
            NativeExtension::RelatedContent => {
                document.related_content = vec![pb::RelatedContent {
                    role: pb::RelatedContentRole::Diagram as i32,
                    document: Some(pb::DocumentSelection {
                        selection: Some(pb::document_selection::Selection::Document(
                            pb::DocumentRef {
                                id: Some(id(&self.namespace, "related-target")),
                            },
                        )),
                    }),
                    ..Default::default()
                }];
            }
        }
    }

    async fn put(&mut self, entity: pb::Entity) -> pb::PutEntityResponse {
        self.put_if_match(entity, String::new()).await
    }

    async fn put_if_match(
        &mut self,
        entity: pb::Entity,
        if_match: String,
    ) -> pb::PutEntityResponse {
        self.client
            .put_entity(req(pb::PutEntityRequest {
                entity: Some(entity),
                if_match,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
    }

    async fn get_document(&mut self, slug: &str) -> pb::Entity {
        self.client
            .get_entity(req(pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&self.namespace, slug)),
            }))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap()
    }

    async fn document_exists(&mut self, slug: &str) -> bool {
        match self
            .client
            .get_entity(req(pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&self.namespace, slug)),
            }))
            .await
        {
            Ok(response) => {
                assert!(response.into_inner().entity.is_some());
                true
            }
            Err(status) => {
                assert_eq!(status.code(), Code::NotFound, "{status:?}");
                false
            }
        }
    }

    async fn create_mapping(
        &mut self,
        key: &str,
        entity: &pb::Entity,
        ownership: pb::SourceFieldOwnership,
        adopt: Option<pb::SourceAdoption>,
    ) -> Result<pb::SourceMapping, tonic::Status> {
        self.client
            .create_source_mapping(req(pb::CreateSourceMappingRequest {
                registration_revision: self.registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: format!("map-{key}"),
                    source: Some(self.source(key)),
                    selection: Some(pb::SourceSelection {
                        selection: Some(pb::source_selection::Selection::Entity(owner(entity))),
                    }),
                    ownership: Some(ownership),
                    adoption: adopt,
                    ..Default::default()
                }),
            }))
            .await
            .map(|r| r.into_inner().mapping.unwrap())
    }

    async fn map_absent(
        &mut self,
        key: &str,
        ownership: pb::SourceFieldOwnership,
    ) -> pb::SourceMapping {
        self.client
            .create_source_mapping(req(pb::CreateSourceMappingRequest {
                registration_revision: self.registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: format!("map-{key}"),
                    source: Some(self.source(key)),
                    selection: Some(pb::SourceSelection {
                        selection: Some(pb::source_selection::Selection::Entity(pb::EntityRef {
                            kind: pb::EntityKind::Document as i32,
                            id: Some(id(&self.namespace, key)),
                        })),
                    }),
                    ownership: Some(ownership),
                    ..Default::default()
                }),
            }))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap()
    }

    async fn preview(
        &mut self,
        snapshot: &pb::SourceSnapshot,
        key: &str,
        entity: pb::Entity,
    ) -> Result<pb::ReconciliationRun, tonic::Status> {
        self.client
            .preview_source_run(req(pb::PreviewSourceRunRequest {
                registration_id: self.registration.id.clone(),
                registration_revision: self.registration.revision.clone(),
                snapshot_id: snapshot.id.clone(),
                run_id: format!("run-{key}"),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(self.source(key)),
                    entity: Some(entity),
                }],
                ..Default::default()
            }))
            .await
            .map(|r| r.into_inner().run.unwrap())
    }

    async fn apply(&mut self, run: &pb::ReconciliationRun) -> pb::ReconciliationRun {
        self.client
            .apply_source_run(req(pb::ApplySourceRunRequest {
                registration_id: self.registration.id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision.clone(),
                preview_digest: run.preview_digest.clone(),
            }))
            .await
            .unwrap()
            .into_inner()
            .run
            .unwrap()
    }
}

fn document_catalog_guard(status: &tonic::Status) -> bool {
    status.code() == Code::InvalidArgument
        && status
            .message()
            .contains("document catalog context supports native authoring only")
}

async fn assert_create_rejects(
    fixture: &mut Fixture,
    extension: NativeExtension,
    policy: pb::SourceFieldOwnership,
    key: &str,
) {
    let mut incoming = fixture.base_document(key);
    fixture.apply_extension(&mut incoming, extension);
    let snapshot = fixture.snapshot(key).await;
    let mapping = fixture.map_absent(key, policy).await;
    let existed_before = fixture.document_exists(key).await;
    assert!(!existed_before);
    let err = fixture.preview(&snapshot, key, incoming).await.unwrap_err();
    assert!(
        document_catalog_guard(&err),
        "expected native catalog guard, got {err:?} for {extension:?}"
    );
    let mapping_after = fixture
        .client
        .get_source_mapping(req(pb::GetSourceMappingRequest {
            registration_id: fixture.registration.id.clone(),
            mapping_id: mapping.id.clone(),
        }))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert_eq!(mapping_after, mapping);
    assert_eq!(fixture.document_exists(key).await, existed_before);
}

#[tokio::test]
async fn source_create_with_each_native_extension_rejects_under_other_supported_policies() {
    let mut fixture = fixture().await;
    fixture.put(fixture.base_document("related-target")).await;
    let fields = [
        pb::SourceOwnedField::Doc,
        pb::SourceOwnedField::Metadata,
        pb::SourceOwnedField::Body,
        pb::SourceOwnedField::Subjects,
        pb::SourceOwnedField::Navigation,
        pb::SourceOwnedField::Anchors,
        pb::SourceOwnedField::Supersedes,
    ];
    let mut policies = vec![pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::None(
            pb::SourceNoOwnership {},
        )),
    }];
    policies.extend(fields.map(field_owner));
    policies.push(pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: fields.into_iter().map(|field| field as i32).collect(),
            },
        )),
    });
    for (policy_index, policy) in policies.into_iter().enumerate() {
        for (extension_index, extension) in [
            NativeExtension::BinaryOnly,
            NativeExtension::CollectionOnly,
            NativeExtension::Purpose,
            NativeExtension::Filename,
            NativeExtension::RelatedContent,
        ]
        .into_iter()
        .enumerate()
        {
            let key = format!("crt-other-{policy_index}-{extension_index}");
            assert_create_rejects(&mut fixture, extension, policy.clone(), &key).await;
        }
    }
}

#[tokio::test]
async fn source_create_with_each_native_extension_rejects_under_title_only_policy() {
    let mut fixture = fixture().await;
    fixture.put(fixture.base_document("related-target")).await;
    for extension in [
        NativeExtension::BinaryOnly,
        NativeExtension::CollectionOnly,
        NativeExtension::Purpose,
        NativeExtension::Filename,
        NativeExtension::RelatedContent,
    ] {
        let key = match extension {
            NativeExtension::BinaryOnly => "crt-title-bin",
            NativeExtension::CollectionOnly => "crt-title-col",
            NativeExtension::Purpose => "crt-title-pur",
            NativeExtension::Filename => "crt-title-nam",
            NativeExtension::RelatedContent => "crt-title-rel",
        };
        assert_create_rejects(&mut fixture, extension, title_only(), key).await;
    }
}

#[tokio::test]
async fn source_create_with_each_native_extension_rejects_under_whole_owner_policy() {
    let mut fixture = fixture().await;
    fixture.put(fixture.base_document("related-target")).await;
    for extension in [
        NativeExtension::BinaryOnly,
        NativeExtension::CollectionOnly,
        NativeExtension::Purpose,
        NativeExtension::Filename,
        NativeExtension::RelatedContent,
    ] {
        let key = match extension {
            NativeExtension::BinaryOnly => "crt-whole-bin",
            NativeExtension::CollectionOnly => "crt-whole-col",
            NativeExtension::Purpose => "crt-whole-pur",
            NativeExtension::Filename => "crt-whole-nam",
            NativeExtension::RelatedContent => "crt-whole-rel",
        };
        assert_create_rejects(&mut fixture, extension, whole_owner(), key).await;
    }
}

#[tokio::test]
async fn source_whole_owner_mapping_and_preview_reject_native_extensions() {
    let mut fixture = fixture().await;
    let mut extended = fixture.base_document("extended");
    fixture.apply_extension(&mut extended, NativeExtension::BinaryOnly);
    fixture.apply_extension(&mut extended, NativeExtension::Purpose);
    let native = fixture.put(extended.clone()).await;
    let whole_err = fixture
        .create_mapping(
            "extended",
            native.entity.as_ref().unwrap(),
            whole_owner(),
            Some(adoption(&native)),
        )
        .await
        .unwrap_err();
    assert!(document_catalog_guard(&whole_err), "{whole_err:?}");
    let mapping = fixture
        .create_mapping(
            "extended",
            native.entity.as_ref().unwrap(),
            whole_owner(),
            Some(adoption(&native)),
        )
        .await;
    assert!(mapping.is_err());
}

#[tokio::test]
async fn source_whole_owner_preview_rejects_when_native_gained_extensions() {
    let mut fixture = fixture().await;
    let plain = fixture.put(fixture.base_document("plain")).await;
    fixture
        .create_mapping(
            "plain",
            plain.entity.as_ref().unwrap(),
            whole_owner(),
            Some(adoption(&plain)),
        )
        .await
        .unwrap();
    let stored = fixture.get_document("plain").await;
    let etag = fixture
        .client
        .get_entity(req(pb::GetEntityRequest {
            kind: pb::EntityKind::Document as i32,
            id: Some(id(&fixture.namespace, "plain")),
        }))
        .await
        .unwrap()
        .into_inner()
        .etag;
    let mut native = stored;
    fixture.apply_extension(&mut native, NativeExtension::BinaryOnly);
    fixture.put_if_match(native, etag).await;
    let snapshot = fixture.snapshot("plain").await;
    let mut incoming = fixture.base_document("plain");
    incoming.system = plain.entity.as_ref().unwrap().system.clone();
    let err = fixture
        .preview(&snapshot, "plain", incoming)
        .await
        .unwrap_err();
    assert!(document_catalog_guard(&err), "{err:?}");
}

#[tokio::test]
async fn source_field_ownership_claims_on_native_extensions_reject_via_reflection() {
    let mut fixture = fixture().await;
    let cases = [
        (NativeExtension::Purpose, pb::SourceOwnedField::Purpose),
        (NativeExtension::BinaryOnly, pb::SourceOwnedField::Body),
        (
            NativeExtension::CollectionOnly,
            pb::SourceOwnedField::Metadata,
        ),
    ];
    for (extension, field) in cases {
        let slug = match field {
            pb::SourceOwnedField::Purpose => "fld-purpose",
            pb::SourceOwnedField::Body => "fld-body",
            pb::SourceOwnedField::Metadata => "fld-metadata",
            _ => unreachable!(),
        };
        let mut extended = fixture.base_document(slug);
        fixture.apply_extension(&mut extended, extension);
        let native = fixture.put(extended).await;
        let err = fixture
            .create_mapping(
                slug,
                native.entity.as_ref().unwrap(),
                field_owner(field),
                Some(adoption(&native)),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument, "{err:?}");
    }
}

#[tokio::test]
async fn source_title_only_apply_preserves_native_bytes_ids_and_annotations() {
    let mut fixture = fixture().await;
    let mut extended = fixture.base_document("preserve");
    fixture.apply_extension(&mut extended, NativeExtension::BinaryOnly);
    fixture.apply_extension(&mut extended, NativeExtension::CollectionOnly);
    fixture.apply_extension(&mut extended, NativeExtension::Purpose);
    let native = fixture.put(extended).await;
    let before = native.entity.clone().unwrap();
    let uid = before.system.as_ref().unwrap().uid.clone();
    fixture
        .create_mapping("preserve", &before, title_only(), Some(adoption(&native)))
        .await
        .unwrap();
    let snapshot = fixture.snapshot("preserve").await;
    let mut incoming = before.clone();
    if let Some(pb::entity::Kind::Document(document)) = &mut incoming.kind {
        document.title = Some(pb::CatalogTitle {
            value: "Source-owned title".into(),
        });
    }
    let run = fixture
        .preview(&snapshot, "preserve", incoming)
        .await
        .unwrap();
    fixture.apply(&run).await;
    let after = fixture.get_document("preserve").await;
    assert_eq!(
        after.system.as_ref().unwrap().uid,
        uid,
        "native uid must survive title-only source merge"
    );
    assert_eq!(after.encode_to_vec(), {
        let mut expected = before;
        if let Some(pb::entity::Kind::Document(document)) = &mut expected.kind {
            document.title = Some(pb::CatalogTitle {
                value: "Source-owned title".into(),
            });
        }
        expected.encode_to_vec()
    });
}
