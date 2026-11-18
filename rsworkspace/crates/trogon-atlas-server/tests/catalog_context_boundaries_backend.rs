#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_core::document_catalog_context::{bytes_content_digest, MAX_RETAINED_BYTES};
use trogon_atlas_proto::{
    self as pb, event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NatsStore, Store};

const ADMIN: &str = "catalog-boundary-admin";

type Client = EventModelServiceClient<Channel>;

fn request<T>(body: T) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {ADMIN}").parse().unwrap());
    req
}

async fn client_with_store() -> (Client, Arc<NatsStore>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let grpc = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(service)),
        BearerAuth::new(Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (
        Client::connect(format!("http://{addr}")).await.unwrap(),
        store,
    )
}

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "catalog-boundary".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

fn read_model(slug: &str) -> pb::Entity {
    entity(pb::entity::Kind::ReadModel(pb::ReadModel {
        id: Some(id(slug)),
        title: slug.into(),
        ..Default::default()
    }))
}

fn entity_subject(slug: &str) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(slug)),
        })),
    }
}

fn associate(subject: pb::CatalogSubject) -> pb::SubjectAssociation {
    pb::SubjectAssociation {
        subject: Some(subject),
        ..Default::default()
    }
}

fn whole(slug: &str) -> pb::DocumentSelection {
    pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef { id: Some(id(slug)) },
        )),
    }
}

fn uid(value: &pb::Entity) -> String {
    value.system.as_ref().unwrap().uid.clone()
}

fn document_of(value: &pb::Entity) -> &pb::Document {
    let Some(pb::entity::Kind::Document(document)) = &value.kind else {
        panic!("document required")
    };
    document
}

async fn put(client: &mut Client, value: pb::Entity) -> pb::PutEntityResponse {
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(value),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn put_guarded(client: &mut Client, value: pb::Entity, etag: &str) -> pb::PutEntityResponse {
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(value),
            if_match: etag.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn get(client: &mut Client, kind: pb::EntityKind, slug: &str) -> pb::GetEntityResponse {
    client
        .get_entity(request(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id(slug)),
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn document_detail(
    client: &mut Client,
    selection: pb::DocumentSelection,
) -> pb::GetDocumentDetailResponse {
    client
        .get_document_detail(request(pb::GetDocumentDetailRequest {
            document: Some(selection),
        }))
        .await
        .unwrap()
        .into_inner()
}

fn link_document(slug: &str, subject: &str) -> pb::Entity {
    entity(pb::entity::Kind::Document(pb::Document {
        id: Some(id(slug)),
        title: Some(title(slug)),
        subjects: vec![associate(entity_subject(subject))],
        body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
            value: "https://example.com/boundary".into(),
        })),
        ..Default::default()
    }))
}

fn retained_content(
    slug: &str,
    subject: &str,
    media_type: &str,
    bytes: &[u8],
    purpose: pb::DocumentPurpose,
) -> pb::Entity {
    entity(pb::entity::Kind::Document(pb::Document {
        id: Some(id(slug)),
        title: Some(title(slug)),
        subjects: vec![associate(entity_subject(subject))],
        purpose: Some(purpose as i32),
        body: Some(pb::document::Body::RetainedContent(pb::RetainedContent {
            media_type: Some(pb::MediaType {
                value: media_type.into(),
            }),
            content: bytes.to_vec(),
            digest: Some(bytes_content_digest(bytes)),
        })),
        ..Default::default()
    }))
}

fn retained_text(slug: &str, subject: &str, content: &str) -> pb::Entity {
    entity(pb::entity::Kind::Document(pb::Document {
        id: Some(id(slug)),
        title: Some(title(slug)),
        subjects: vec![associate(entity_subject(subject))],
        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            media_type: Some(pb::MediaType {
                value: "text/markdown".into(),
            }),
            content: content.into(),
            digest: Some(trogon_atlas_core::specification::content_digest(content)),
        })),
        ..Default::default()
    }))
}

async fn refused(client: &mut Client, value: pb::Entity, etag: &str) -> tonic::Status {
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(value),
            if_match: etag.into(),
            ..Default::default()
        }))
        .await
        .unwrap_err()
}

#[tokio::test]
async fn authored_changelog_is_not_derived_from_resource_revisions() {
    let (mut client, store) = client_with_store().await;
    put(&mut client, read_model("changelog-resource")).await;
    let prose = "## 1.0\nAuthored release note.\n";
    let mut changelog = retained_text("authored-changelog", "changelog-resource", prose);
    let Some(pb::entity::Kind::Document(document)) = &mut changelog.kind else {
        panic!("document")
    };
    document.purpose = Some(pb::DocumentPurpose::Changelog as i32);
    let stored = put(&mut client, changelog).await;
    let stored_entity = stored.entity.unwrap();
    let before = get(&mut client, pb::EntityKind::Document, "authored-changelog").await;
    assert_eq!(before.entity.as_ref(), Some(&stored_entity));
    assert_eq!(before.etag, stored.etag);
    let detail_before = document_detail(&mut client, whole("authored-changelog")).await;
    let sequence = store.current_change_seq().await.unwrap();

    let mut etag = get(&mut client, pb::EntityKind::ReadModel, "changelog-resource")
        .await
        .etag;
    let revisions = 3;
    for revision in 0..revisions {
        let mut resource = get(&mut client, pb::EntityKind::ReadModel, "changelog-resource")
            .await
            .entity
            .unwrap();
        let Some(pb::entity::Kind::ReadModel(model)) = &mut resource.kind else {
            panic!("read model")
        };
        model.title = format!("Revised {revision}");
        let revised = put_guarded(&mut client, resource, &etag).await;
        assert_ne!(revised.etag, etag);
        etag = revised.etag;
    }

    let after = get(&mut client, pb::EntityKind::Document, "authored-changelog").await;
    assert_eq!(after.entity, before.entity);
    assert_eq!(after.etag, before.etag);
    let Some(pb::document::Body::Retained(body)) =
        &document_of(after.entity.as_ref().unwrap()).body
    else {
        panic!("authored prose")
    };
    assert_eq!(body.content, prose);
    assert_eq!(
        document_of(after.entity.as_ref().unwrap()).purpose,
        Some(pb::DocumentPurpose::Changelog as i32)
    );
    let detail_after = document_detail(&mut client, whole("authored-changelog")).await;
    assert_eq!(detail_after.entity, detail_before.entity);
    assert_eq!(detail_after.etag, detail_before.etag);
    assert_eq!(detail_after.declarations, detail_before.declarations);
    let expected_sequence = sequence + u64::try_from(revisions).unwrap();
    assert_eq!(
        store.current_change_seq().await.unwrap(),
        expected_sequence,
        "only the resource revisions advance the change sequence"
    );
}

async fn retained_content_boundaries(client: &mut Client, store: &NatsStore, label: &str) {
    let resource = format!("{label}-resource");
    put(client, read_model(&resource)).await;

    let exact_bytes = vec![0xA5_u8; MAX_RETAINED_BYTES];
    let exact_slug = format!("{label}-exact");
    let exact = put(
        client,
        retained_content(
            &exact_slug,
            &resource,
            "application/octet-stream",
            &exact_bytes,
            pb::DocumentPurpose::Attachment,
        ),
    )
    .await;
    let stored = get(client, pb::EntityKind::Document, &exact_slug).await;
    assert_eq!(stored.etag, exact.etag);
    let Some(pb::document::Body::RetainedContent(body)) =
        &document_of(stored.entity.as_ref().unwrap()).body
    else {
        panic!("retained content")
    };
    assert_eq!(body.content.len(), MAX_RETAINED_BYTES);
    assert_eq!(body.content, exact_bytes);
    assert_eq!(body.digest, Some(bytes_content_digest(&exact_bytes)));

    let sequence = store.current_change_seq().await.unwrap();
    let over_bytes = vec![0xA5_u8; MAX_RETAINED_BYTES + 1];
    let over = retained_content(
        &format!("{label}-over"),
        &resource,
        "application/octet-stream",
        &over_bytes,
        pb::DocumentPurpose::Attachment,
    );
    let Some(pb::entity::Kind::Document(over_document)) = &over.kind else {
        panic!("document")
    };
    let Some(pb::document::Body::RetainedContent(over_body)) = &over_document.body else {
        panic!("retained content")
    };
    assert_eq!(over_body.digest, Some(bytes_content_digest(&over_bytes)));
    let error = refused(client, over.clone(), "").await;
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error
            .message()
            .contains("retained binary outside byte bounds"),
        "{}",
        error.message()
    );
    assert_eq!(
        client
            .get_entity(request(pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id(&format!("{label}-over"))),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);

    let empty = retained_content(
        &format!("{label}-empty"),
        &resource,
        "application/octet-stream",
        &[],
        pb::DocumentPurpose::Attachment,
    );
    let Some(pb::entity::Kind::Document(empty_document)) = &empty.kind else {
        panic!("document")
    };
    let Some(pb::document::Body::RetainedContent(empty_body)) = &empty_document.body else {
        panic!("retained content")
    };
    assert_eq!(empty_body.digest, Some(bytes_content_digest(&[])));
    let error = refused(client, empty, "").await;
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error
            .message()
            .contains("retained binary outside byte bounds"),
        "{}",
        error.message()
    );
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);

    let mut grown = stored.entity.clone().unwrap();
    let Some(pb::entity::Kind::Document(grown_document)) = &mut grown.kind else {
        panic!("document")
    };
    grown_document.body.clone_from(&over_document.body);
    let error = refused(client, grown, &stored.etag).await;
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error
            .message()
            .contains("retained binary outside byte bounds"),
        "{}",
        error.message()
    );
    let unchanged = get(client, pb::EntityKind::Document, &exact_slug).await;
    assert_eq!(unchanged.entity, stored.entity);
    assert_eq!(unchanged.etag, stored.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
}

#[tokio::test]
async fn retained_content_bytes_admit_the_exact_ceiling_and_refuse_every_other_size_atomically() {
    let (mut client, store) = client_with_store().await;
    retained_content_boundaries(&mut client, &store, "bytes").await;
}

#[tokio::test]
async fn retained_text_boundary_is_a_separate_contract_from_retained_bytes() {
    let (mut client, store) = client_with_store().await;
    put(&mut client, read_model("text-resource")).await;
    let exact_text = "a".repeat(MAX_RETAINED_BYTES);
    let exact = put(
        &mut client,
        retained_text("text-exact", "text-resource", &exact_text),
    )
    .await;
    let stored = get(&mut client, pb::EntityKind::Document, "text-exact").await;
    assert_eq!(stored.etag, exact.etag);
    let Some(pb::document::Body::Retained(body)) =
        &document_of(stored.entity.as_ref().unwrap()).body
    else {
        panic!("retained text")
    };
    assert_eq!(body.content.len(), MAX_RETAINED_BYTES);
    let sequence = store.current_change_seq().await.unwrap();
    let over_text = "a".repeat(MAX_RETAINED_BYTES + 1);
    let error = refused(
        &mut client,
        retained_text("text-over", "text-resource", &over_text),
        "",
    )
    .await;
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error
            .message()
            .contains("retained text outside byte bounds"),
        "{}",
        error.message()
    );
    assert!(!error.message().contains("retained binary"));
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
}

async fn related_declarations(
    client: &mut Client,
    detail_of: &str,
) -> Vec<(
    pb::RelatedContentRole,
    pb::DocumentSelection,
    pb::ConnectionId,
)> {
    document_detail(client, whole(detail_of))
        .await
        .declarations
        .into_iter()
        .filter_map(|entry| match entry.declaration {
            Some(pb::catalog_declaration::Declaration::RelatedContent(value)) => Some((
                pb::RelatedContentRole::try_from(value.role).unwrap(),
                value.document.unwrap(),
                value.connection_id.unwrap(),
            )),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn example_document_member_selector_and_relation_keep_exact_identities_without_payload_inference(
) {
    let (mut client, _store) = client_with_store().await;
    put(&mut client, read_model("example-model")).await;
    let specification = put(
        &mut client,
        entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id("example-contract")),
            title: "Example contract".into(),
            messages: vec![pb::ContractMessage {
                name: Some(pb::MemberName {
                    value: "Receipt".into(),
                }),
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id("example-model")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
    )
    .await;
    let spec_entity = specification.entity.unwrap();
    let Some(pb::entity::Kind::Specification(spec)) = &spec_entity.kind else {
        panic!("specification")
    };
    let member = pb::catalog_subject::Selection::Member(pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: Some(id("example-contract")),
        }),
        owner_uid: uid(&spec_entity),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: spec.messages[0].member_id.clone(),
        expected_digest: None,
    });
    let member_subject = pb::CatalogSubject {
        selection: Some(member.clone()),
    };
    assert!(spec.messages[0].member_id.is_some());

    let raw_json = br#"{"orderId":42,"unrelated":["not","a","Receipt"]}"#;
    let mut example = retained_content(
        "example-json",
        "example-model",
        "application/json",
        raw_json,
        pb::DocumentPurpose::Example,
    );
    let Some(pb::entity::Kind::Document(example_document)) = &mut example.kind else {
        panic!("document")
    };
    example_document.subjects = vec![associate(member_subject.clone())];
    example_document.filename = Some(pb::LogicalFilename {
        value: "receipt.json".into(),
    });
    let example = put(&mut client, example).await.entity.unwrap();
    let example_subject_id = document_of(&example).subjects[0]
        .connection_id
        .clone()
        .expect("server assigns the subject occurrence identity");

    let mut hub = link_document("example-hub", "example-model");
    let Some(pb::entity::Kind::Document(hub_document)) = &mut hub.kind else {
        panic!("document")
    };
    hub_document.related_content = vec![pb::RelatedContent {
        document: Some(whole("example-json")),
        role: pb::RelatedContentRole::Example as i32,
        label: Some(title("Receipt payload")),
        ..Default::default()
    }];
    let hub = put(&mut client, hub).await.entity.unwrap();
    let related_id = document_of(&hub).related_content[0]
        .connection_id
        .clone()
        .expect("server assigns the related occurrence identity");
    assert_ne!(related_id, example_subject_id);

    let forward = related_declarations(&mut client, "example-hub").await;
    assert_eq!(
        forward,
        vec![(
            pb::RelatedContentRole::Example,
            whole("example-json"),
            related_id.clone()
        )]
    );
    let example_detail = document_detail(&mut client, whole("example-json")).await;
    let subjects: Vec<_> = example_detail
        .declarations
        .iter()
        .filter_map(|entry| match &entry.declaration {
            Some(pb::catalog_declaration::Declaration::Subject(value)) => Some(value.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(subjects.len(), 1);
    assert_eq!(
        subjects[0].connection_id.as_ref(),
        Some(&example_subject_id)
    );
    let Some(pb::catalog_subject::Selection::Member(selected)) = subjects[0]
        .subject
        .as_ref()
        .and_then(|s| s.selection.as_ref())
    else {
        panic!("member subject")
    };
    assert_eq!(selected.kind, pb::ContractMemberKind::Message as i32);
    assert_eq!(selected.owner_uid, uid(&spec_entity));
    assert_eq!(selected.member_id, spec.messages[0].member_id);
    assert_eq!(
        selected.specification.as_ref().unwrap().id,
        Some(id("example-contract"))
    );
    assert_eq!(selected.expected_digest, None);

    let by_member = client
        .list_subject_documents(request(pb::ListSubjectDocumentsRequest {
            subject: Some(member_subject.clone()),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!by_member.incomplete);
    assert_eq!(
        by_member.subject_state,
        pb::CatalogResolutionState::Resolved as i32
    );
    assert_eq!(by_member.declarations.len(), 1);
    let declaration = &by_member.declarations[0];
    assert_eq!(declaration.owner.as_ref().unwrap().uid, uid(&example));
    assert_eq!(
        declaration.owner.as_ref().unwrap().owner,
        Some(pb::EntityRef {
            kind: pb::EntityKind::Document as i32,
            id: Some(id("example-json")),
        })
    );
    let Some(pb::catalog_declaration::Declaration::Subject(association)) = &declaration.declaration
    else {
        panic!("subject association")
    };
    assert_eq!(
        association.connection_id.as_ref(),
        Some(&example_subject_id)
    );
    assert_eq!(association.subject.as_ref(), Some(&member_subject));

    let incoming = client
        .list_incoming_document_declarations(request(pb::ListIncomingDocumentDeclarationsRequest {
            document: Some(whole("example-json")),
            related_content_role: Some(pb::RelatedContentRole::Example as i32),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!incoming.incomplete);
    assert_eq!(incoming.declarations.len(), 1);
    assert_eq!(
        incoming.declarations[0].owner.as_ref().unwrap().uid,
        uid(&hub)
    );
    let Some(pb::catalog_declaration::Declaration::RelatedContent(reverse)) =
        &incoming.declarations[0].declaration
    else {
        panic!("related content")
    };
    assert_eq!(reverse.connection_id.as_ref(), Some(&related_id));
    assert_eq!(reverse.role, pb::RelatedContentRole::Example as i32);
    assert_eq!(reverse.document, Some(whole("example-json")));
    let other_role = client
        .list_incoming_document_declarations(request(pb::ListIncomingDocumentDeclarationsRequest {
            document: Some(whole("example-json")),
            related_content_role: Some(pb::RelatedContentRole::Diagram as i32),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(other_role.declarations.len(), 0);

    let by_resource = client
        .list_subject_documents(request(pb::ListSubjectDocumentsRequest {
            subject: Some(entity_subject("example-model")),
        }))
        .await
        .unwrap()
        .into_inner();
    let owners: Vec<_> = by_resource
        .declarations
        .iter()
        .map(|entry| entry.owner.as_ref().unwrap().uid.clone())
        .collect();
    assert_eq!(
        owners,
        vec![uid(&hub)],
        "mapping a message to a read model never associates the example"
    );

    let stored = get(&mut client, pb::EntityKind::Document, "example-json").await;
    assert_eq!(stored.entity.as_ref(), Some(&example));
    let Some(pb::document::Body::RetainedContent(body)) = &document_of(&example).body else {
        panic!("retained content")
    };
    assert_eq!(body.content, raw_json);
    assert_eq!(body.digest, Some(bytes_content_digest(raw_json)));
    assert_eq!(
        document_of(&example)
            .filename
            .as_ref()
            .map(|f| f.value.as_str()),
        Some("receipt.json")
    );
    let spec_after = get(
        &mut client,
        pb::EntityKind::Specification,
        "example-contract",
    )
    .await;
    assert_eq!(spec_after.entity.as_ref(), Some(&spec_entity));
    assert_eq!(spec_after.etag, specification.etag);

    let malformed = b"{ not json at all";
    let mut edited = stored.entity.clone().unwrap();
    let Some(pb::entity::Kind::Document(edited_document)) = &mut edited.kind else {
        panic!("document")
    };
    edited_document.body = Some(pb::document::Body::RetainedContent(pb::RetainedContent {
        media_type: Some(pb::MediaType {
            value: "application/json".into(),
        }),
        content: malformed.to_vec(),
        digest: Some(bytes_content_digest(malformed)),
    }));
    let edited = put_guarded(&mut client, edited, &stored.etag)
        .await
        .entity
        .unwrap();
    assert_eq!(
        document_of(&edited).subjects[0].connection_id.as_ref(),
        Some(&example_subject_id)
    );
    assert_eq!(
        related_declarations(&mut client, "example-hub").await,
        forward
    );
}

async fn put_all(client: &Client, values: Vec<pb::Entity>) {
    for chunk in values.chunks(32) {
        let mut tasks = tokio::task::JoinSet::new();
        for value in chunk {
            let mut client = client.clone();
            let value = value.clone();
            tasks.spawn(async move { put(&mut client, value).await });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    }
}

fn related(slug: &str, role: pb::RelatedContentRole) -> pb::RelatedContent {
    pb::RelatedContent {
        document: Some(whole(slug)),
        role: role as i32,
        ..Default::default()
    }
}

fn owner_document(slug: &str, subject: &str) -> pb::Entity {
    link_document(slug, subject)
}

fn related_owner(slug: &str, count: usize) -> pb::Entity {
    let mut owner = owner_document(slug, "related-resource");
    let Some(pb::entity::Kind::Document(document)) = &mut owner.kind else {
        panic!("document")
    };
    document.related_content = (0..count)
        .map(|index| {
            related(
                &format!("related-target-{index}"),
                pb::RelatedContentRole::Attachment,
            )
        })
        .collect();
    owner
}

#[tokio::test]
async fn related_content_admits_128_and_refuses_129_atomically() {
    let (mut client, store) = client_with_store().await;
    put(&mut client, read_model("related-resource")).await;
    put_all(
        &client,
        (0..129)
            .map(|index| owner_document(&format!("related-target-{index}"), "related-resource"))
            .collect(),
    )
    .await;
    let exact = put(&mut client, related_owner("related-at-limit", 128))
        .await
        .entity
        .unwrap();
    let identities: std::collections::HashSet<_> = document_of(&exact)
        .related_content
        .iter()
        .map(|item| {
            item.connection_id
                .clone()
                .expect("server-assigned identity")
        })
        .collect();
    assert_eq!(document_of(&exact).related_content.len(), 128);
    assert_eq!(identities.len(), 128);
    let stored = get(&mut client, pb::EntityKind::Document, "related-at-limit").await;
    assert_eq!(stored.entity.as_ref(), Some(&exact));

    let sequence = store.current_change_seq().await.unwrap();
    let error = refused(&mut client, related_owner("related-over-limit", 129), "").await;
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error.message().contains("related content limit exceeded"),
        "{}",
        error.message()
    );
    assert_eq!(
        client
            .get_entity(request(pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("related-over-limit")),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let mut grown = stored.entity.clone().unwrap();
    let Some(pb::entity::Kind::Document(grown_document)) = &mut grown.kind else {
        panic!("document")
    };
    grown_document.related_content.push(related(
        "related-target-128",
        pb::RelatedContentRole::Attachment,
    ));
    let error = refused(&mut client, grown, &stored.etag).await;
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error.message().contains("related content limit exceeded"),
        "{}",
        error.message()
    );
    let unchanged = get(&mut client, pb::EntityKind::Document, "related-at-limit").await;
    assert_eq!(unchanged.entity, stored.entity);
    assert_eq!(unchanged.etag, stored.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
}

#[tokio::test(flavor = "multi_thread")]
async fn document_owner_reaches_its_derived_maximum_inventory_and_refuses_one_more_subject() {
    let (mut client, store) = client_with_store().await;
    let subject_limit = trogon_atlas_core::decision_document::MAX_SUBJECTS;
    put_all(
        &client,
        (0..=subject_limit)
            .map(|index| read_model(&format!("inventory-subject-{index}")))
            .collect(),
    )
    .await;
    put_all(
        &client,
        (0..128)
            .map(|index| {
                owner_document(&format!("inventory-target-{index}"), "inventory-subject-0")
            })
            .collect(),
    )
    .await;
    let mut owner = owner_document("inventory-owner", "inventory-subject-0");
    let Some(pb::entity::Kind::Document(document)) = &mut owner.kind else {
        panic!("document")
    };
    document.subjects = (0..subject_limit)
        .map(|index| associate(entity_subject(&format!("inventory-subject-{index}"))))
        .collect();
    document.navigation = (0..128)
        .map(|index| pb::DocumentNavigation {
            document: Some(whole(&format!("inventory-target-{index}"))),
            ..Default::default()
        })
        .collect();
    document.related_content = (0..128)
        .map(|index| {
            related(
                &format!("inventory-target-{index}"),
                pb::RelatedContentRole::Attachment,
            )
        })
        .collect();
    let stored = put(&mut client, owner).await;
    let exact = stored.entity.unwrap();
    let document = document_of(&exact);
    assert_eq!(document.subjects.len(), 1_024);
    assert_eq!(document.navigation.len(), 128);
    assert_eq!(document.related_content.len(), 128);
    let identities: std::collections::HashSet<_> = document
        .subjects
        .iter()
        .map(|item| item.connection_id.clone())
        .chain(
            document
                .navigation
                .iter()
                .map(|item| item.connection_id.clone()),
        )
        .chain(
            document
                .related_content
                .iter()
                .map(|item| item.connection_id.clone()),
        )
        .collect();
    assert_eq!(identities.len(), 1_280);
    assert!(identities.iter().all(Option::is_some));
    let read = get(&mut client, pb::EntityKind::Document, "inventory-owner").await;
    assert_eq!(read.entity.as_ref(), Some(&exact));
    assert_eq!(read.etag, stored.etag);

    let sequence = store.current_change_seq().await.unwrap();
    let mut grown = exact.clone();
    let Some(pb::entity::Kind::Document(grown_document)) = &mut grown.kind else {
        panic!("document")
    };
    grown_document
        .subjects
        .push(associate(entity_subject(&format!(
            "inventory-subject-{subject_limit}"
        ))));
    let error = refused(&mut client, grown, &stored.etag).await;
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error
            .message()
            .contains("subject collection outside bounds"),
        "{}",
        error.message()
    );
    let unchanged = get(&mut client, pb::EntityKind::Document, "inventory-owner").await;
    assert_eq!(unchanged.entity, read.entity);
    assert_eq!(unchanged.etag, read.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
}

fn amending_decision(slug: &str, targets: &[String]) -> pb::Entity {
    let option_id = pb::LocalValueId {
        value: uuid::Uuid::now_v7().to_string(),
    };
    let text = |value: &str| pb::CatalogText {
        value: value.into(),
    };
    entity(pb::entity::Kind::Decision(pb::Decision {
        id: Some(id(slug)),
        title: Some(title(slug)),
        subjects: vec![associate(entity_subject("amendment-subject"))],
        status: pb::DecisionStatus::Accepted as i32,
        context: Some(text("Choose delivery")),
        rationale: Some(text("Keep delivery explicit")),
        options: vec![pb::DecisionOption {
            option_id: Some(option_id.clone()),
            title: Some(title("Native delivery")),
            description: Some(text("Retain the selected choice")),
        }],
        selected_option: Some(option_id),
        consequences: vec![pb::DecisionConsequence {
            kind: pb::decision_consequence::Kind::Risk as i32,
            text: Some(text("Delivery may need repair")),
        }],
        amends: targets
            .iter()
            .map(|slug| pb::DecisionAmendment {
                decision: Some(pb::DecisionRef { id: Some(id(slug)) }),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }))
}

async fn try_put(
    client: &mut Client,
    value: pb::Entity,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(value),
            ..Default::default()
        }))
        .await
        .map(tonic::Response::into_inner)
}

const AMENDMENT_BUDGET_ROOT_LIMIT: usize = 38;

async fn decision_state(client: &mut Client, slug: &str) -> Result<pb::GetEntityResponse, Code> {
    client
        .get_entity(request(pb::GetEntityRequest {
            kind: pb::EntityKind::Decision as i32,
            id: Some(id(slug)),
        }))
        .await
        .map(tonic::Response::into_inner)
        .map_err(|error| error.code())
}

#[tokio::test(flavor = "multi_thread")]
async fn amendment_traversal_exhausts_one_shared_write_budget_without_partial_state() {
    let (mut client, store) = client_with_store().await;
    put(&mut client, read_model("amendment-subject")).await;
    let leaves: Vec<String> = (0..128)
        .map(|index| format!("amend-leaf-{index}"))
        .collect();
    put_all(
        &client,
        leaves
            .iter()
            .map(|slug| amending_decision(slug, &[]))
            .collect(),
    )
    .await;
    let middles: Vec<String> = (0..60)
        .map(|index| format!("amend-middle-{index}"))
        .collect();
    put_all(
        &client,
        middles
            .iter()
            .map(|slug| amending_decision(slug, &leaves))
            .collect(),
    )
    .await;

    let at_limit = try_put(
        &mut client,
        amending_decision(
            "amend-root-at-limit",
            &middles[..AMENDMENT_BUDGET_ROOT_LIMIT],
        ),
    )
    .await
    .expect("traversal at the work ceiling is admitted");
    assert_eq!(
        decision_state(&mut client, "amend-root-at-limit")
            .await
            .unwrap()
            .entity,
        at_limit.entity
    );

    let sequence = store.current_change_seq().await.unwrap();
    let error = try_put(
        &mut client,
        amending_decision(
            "amend-root-over-limit",
            &middles[..=AMENDMENT_BUDGET_ROOT_LIMIT],
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert_eq!(error.message(), "catalog work budget exceeded");
    assert_eq!(
        decision_state(&mut client, "amend-root-over-limit")
            .await
            .unwrap_err(),
        Code::NotFound
    );
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);

    let stored = decision_state(&mut client, "amend-root-at-limit")
        .await
        .unwrap();
    let mut grown = stored.entity.clone().unwrap();
    let Some(pb::entity::Kind::Decision(decision)) = &mut grown.kind else {
        panic!("decision")
    };
    decision.amends.push(pb::DecisionAmendment {
        decision: Some(pb::DecisionRef {
            id: Some(id(&middles[AMENDMENT_BUDGET_ROOT_LIMIT])),
        }),
        ..Default::default()
    });
    let error = refused(&mut client, grown, &stored.etag).await;
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert_eq!(error.message(), "catalog work budget exceeded");
    let unchanged = decision_state(&mut client, "amend-root-at-limit")
        .await
        .unwrap();
    assert_eq!(unchanged.entity, stored.entity);
    assert_eq!(unchanged.etag, stored.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);

    let half = middles.len() / 3;
    let first = &middles[..half];
    let second = &middles[half..2 * half];
    for (slug, targets) in [("amend-solo-first", first), ("amend-solo-second", second)] {
        try_put(&mut client, amending_decision(slug, targets))
            .await
            .expect("each half is admitted alone");
    }
    let sequence = store.current_change_seq().await.unwrap();
    let ops = [("amend-batch-first", first), ("amend-batch-second", second)]
        .into_iter()
        .map(|(slug, targets)| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(amending_decision(slug, targets)),
                ..Default::default()
            })),
        })
        .collect();
    let error = client
        .batch_mutate(request(pb::BatchMutateRequest {
            ops,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert_eq!(error.message(), "catalog work budget exceeded");
    for slug in ["amend-batch-first", "amend-batch-second"] {
        assert_eq!(
            decision_state(&mut client, slug).await.unwrap_err(),
            Code::NotFound
        );
    }
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
}
