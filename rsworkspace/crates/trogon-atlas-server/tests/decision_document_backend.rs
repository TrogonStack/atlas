#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "catalog-admin";
const READER: &str = "catalog-reader";
const LIMITED: &str = "catalog-limited";
type Client = EventModelServiceClient<Channel>;
fn request<T>(body: T, token: &str, branch: Option<&str>) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(branch) = branch {
        req.metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    req
}
async fn client() -> Client {
    client_with_cap(0).await
}
async fn client_with_cap(cap: usize) -> Client {
    client_with_store(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        cap,
    )
    .await
}
async fn client_with_store(store: Arc<dyn trogon_atlas_store::Store>, cap: usize) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.reader]
role = "reader"
tokens = ["{READER}"]
[principals.limited]
role = "writer"
parent = "visible"
tokens = ["{LIMITED}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(cap);
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
    Client::connect(format!("http://{addr}")).await.unwrap()
}

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "catalog".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}
fn subject(reference: pb::EntityRef) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Entity(reference)),
    }
}
fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}
fn text(value: &str) -> pb::CatalogText {
    pb::CatalogText {
        value: value.into(),
    }
}
fn read_model(slug: &str) -> pb::Entity {
    entity(pb::entity::Kind::ReadModel(pb::ReadModel {
        id: Some(id(slug)),
        title: slug.into(),
        ..Default::default()
    }))
}
fn document(slug: &str, selected: pb::CatalogSubject) -> pb::Entity {
    let content = "# Delivery\nRetain <script>alert(1)</script> literally.\n";
    entity(pb::entity::Kind::Document(pb::Document {
        id: Some(id(slug)),
        title: Some(title(slug)),
        subjects: vec![pb::SubjectAssociation {
            subject: Some(selected),
            ..Default::default()
        }],
        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            media_type: Some(pb::MediaType {
                value: "text/markdown".into(),
            }),
            content: content.into(),
            digest: Some(trogon_atlas_core::specification::content_digest(content)),
        })),
        anchors: vec![pb::DocumentAnchor {
            fragment: Some(pb::AnchorFragment {
                value: "delivery".into(),
            }),
            label: Some(title("Delivery")),
            ..Default::default()
        }],
        ..Default::default()
    }))
}
fn decision(slug: &str, selected: pb::CatalogSubject, adopted: bool) -> pb::Entity {
    let option_id = pb::LocalValueId {
        value: uuid::Uuid::now_v7().to_string(),
    };
    entity(pb::entity::Kind::Decision(pb::Decision {
        id: Some(id(slug)),
        title: Some(title(slug)),
        subjects: vec![pb::SubjectAssociation {
            subject: Some(selected),
            ..Default::default()
        }],
        status: if adopted {
            pb::DecisionStatus::Accepted
        } else {
            pb::DecisionStatus::Proposed
        } as i32,
        context: Some(text("Choose delivery")),
        rationale: Some(text("Keep delivery explicit")),
        options: vec![pb::DecisionOption {
            option_id: Some(option_id.clone()),
            title: Some(title("Native delivery")),
            description: Some(text("Retain the selected choice")),
        }],
        selected_option: adopted.then_some(option_id),
        consequences: if adopted {
            vec![pb::DecisionConsequence {
                kind: pb::decision_consequence::Kind::Risk as i32,
                text: Some(text("Delivery may need repair")),
            }]
        } else {
            Vec::new()
        },
        ..Default::default()
    }))
}
async fn put(
    client: &mut Client,
    value: pb::Entity,
    branch: Option<&str>,
) -> pb::PutEntityResponse {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(value),
                ..Default::default()
            },
            ADMIN,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
async fn detail(
    client: &mut Client,
    value: &pb::Entity,
    token: &str,
) -> pb::GetDecisionDetailResponse {
    client
        .get_decision_detail(request(
            pb::GetDecisionDetailRequest {
                decision: Some(pb::DecisionRef {
                    id: reference(value).id,
                }),
                follow_replacement: false,
            },
            token,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
}
async fn delete(
    client: &mut Client,
    value: &pb::Entity,
    force: bool,
    branch: Option<&str>,
) -> Result<tonic::Response<pb::DeleteEntityResponse>, tonic::Status> {
    let selected = reference(value);
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: selected.kind,
                id: selected.id,
                mode: if force {
                    pb::delete_entity_request::Mode::Force
                } else {
                    pb::delete_entity_request::Mode::FailIfReferenced
                } as i32,
                ..Default::default()
            },
            ADMIN,
            branch,
        ))
        .await
}
fn anchor_selection(value: &pb::Entity, pin: bool) -> pb::DocumentSelection {
    let Some(pb::entity::Kind::Document(document)) = &value.kind else {
        panic!("document");
    };
    let Some(pb::document::Body::Retained(body)) = &document.body else {
        panic!("retained");
    };
    pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Anchor(
            pb::DocumentAnchorRef {
                document: Some(pb::DocumentRef {
                    id: document.id.clone(),
                }),
                owner_uid: value.system.as_ref().unwrap().uid.clone(),
                anchor_id: document.anchors[0].anchor_id.clone(),
                expected_digest: pin.then(|| body.digest.clone()).flatten(),
            },
        )),
    }
}
fn whole_selection(value: &pb::Entity) -> pb::DocumentSelection {
    pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: reference(value).id,
            },
        )),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn incoming_documents_preserve_unpaged_omission_and_explicit_page_continuation() {
    let mut client = client().await;
    let subject_owner = put(&mut client, read_model("incoming-subject"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&subject_owner));
    let mut target = document("incoming-target", selected.clone());
    let Some(pb::entity::Kind::Document(body)) = &mut target.kind else {
        panic!("document");
    };
    body.purpose = Some(pb::DocumentPurpose::Example as i32);
    let target = put(&mut client, target, None).await.entity.unwrap();
    let selection = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: reference(&target).id,
            },
        )),
    };
    let unrelated = put(
        &mut client,
        document("unrelated-incoming-owner", selected.clone()),
        None,
    )
    .await
    .entity
    .unwrap();
    let mut expected = std::collections::BTreeSet::new();
    for index in 0..101 {
        let mut owner = document(&format!("incoming-owner-{index:03}"), selected.clone());
        let Some(pb::entity::Kind::Document(body)) = &mut owner.kind else {
            panic!("document");
        };
        body.related_content.push(pb::RelatedContent {
            document: Some(selection.clone()),
            role: pb::RelatedContentRole::Example as i32,
            ..Default::default()
        });
        let saved = put(&mut client, owner, None).await.entity.unwrap();
        expected.insert(reference(&saved).id.unwrap().slug);
    }
    let legacy = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(selection.clone()),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(legacy.declarations.len(), 101);
    assert!(legacy.next_page_token.is_empty());
    assert!(!legacy.incomplete);
    let scope = legacy.scope.unwrap();
    assert!(scope.page_receipt.is_none());
    assert!(scope
        .visited
        .iter()
        .any(|owner| owner.owner.as_ref() == Some(&reference(&unrelated))));
    let actual: std::collections::BTreeSet<_> = legacy
        .declarations
        .iter()
        .map(|declaration| {
            declaration
                .owner
                .as_ref()
                .unwrap()
                .owner
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .slug
                .clone()
        })
        .collect();
    assert_eq!(actual, expected);

    let types = trogon_atlas_core::transcode::TranscodePool::builtin().unwrap();
    let explicit: pb::ListIncomingDocumentDeclarationsRequest = types
        .message_from_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.ListIncomingDocumentDeclarationsRequest",
            &serde_json::json!({"document":{"document":{"id":{
            "namespace":"catalog","slug":"incoming-target","version":"1"
        }}},"pageSize":0}),
        )
        .unwrap();
    let first = client
        .list_incoming_document_declarations(request(explicit, READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.declarations.len(), 100);
    assert!(!first.next_page_token.is_empty());
    let first_scope = first.scope.unwrap();
    let receipt = first_scope.page_receipt.unwrap();
    assert_eq!(
        (
            receipt.total_declarations,
            receipt.start_offset,
            receipt.end_offset
        ),
        (101, 0, 100)
    );
    assert!(!first_scope
        .visited
        .iter()
        .any(|owner| owner.owner.as_ref() == Some(&reference(&unrelated))));
    let second = client
        .list_incoming_document_declarations(request(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(selection),
                page_token: first.next_page_token,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.declarations.len(), 1);
    assert!(second.next_page_token.is_empty());
    assert!(!second.incomplete);
    let receipt = second.scope.unwrap().page_receipt.unwrap();
    assert_eq!(
        (
            receipt.total_declarations,
            receipt.start_offset,
            receipt.end_offset
        ),
        (101, 100, 101)
    );
    let mut pages = first.declarations;
    pages.extend(second.declarations);
    assert_eq!(pages, legacy.declarations);
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_public_exact_queries_preserve_bytes_pins_and_provenance_outside_display_cap() {
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    let note = put(
        &mut client,
        document("delivery-note", selected.clone()),
        None,
    )
    .await
    .entity
    .unwrap();
    let pinned = anchor_selection(&note, true);
    let whole = whole_selection(&note);
    let mut choice = decision("delivery-choice", selected.clone(), true);
    if let Some(pb::entity::Kind::Decision(value)) = &mut choice.kind {
        value.documents = [pinned.clone(), whole.clone()]
            .into_iter()
            .map(|document| pb::SupportingDocument {
                document: Some(document),
                ..Default::default()
            })
            .collect();
    }
    let saved = put(&mut client, choice, None).await;
    let choice = saved.entity.unwrap();
    let response = detail(&mut client, &choice, READER).await;
    assert_eq!(response.entity.as_ref(), Some(&choice));
    assert_eq!(response.etag, saved.etag);
    assert!(!response.incomplete);
    assert_eq!(response.declarations.len(), 3);
    assert!(response
        .declarations
        .iter()
        .all(|declaration| declaration.state == pb::CatalogResolutionState::Resolved as i32));
    assert!(response
        .scope
        .unwrap()
        .visited
        .iter()
        .any(|owner| owner.owner.as_ref() == Some(&reference(&note))
            && owner.uid == note.system.as_ref().unwrap().uid
            && !owner.etag.is_empty()));
    let reverse = client
        .list_subject_decisions(request(
            pb::ListSubjectDecisionsRequest {
                subject: Some(selected.clone()),
                status: Some(pb::DecisionStatus::Accepted as i32),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reverse.declarations.len(), 1);
    let Some(pb::catalog_declaration::Declaration::Subject(association)) =
        &reverse.declarations[0].declaration
    else {
        panic!("subject provenance");
    };
    assert_eq!(association.subject.as_ref(), Some(&selected));
    assert!(association.connection_id.is_some());
    for selection in [pinned.clone(), whole] {
        let incoming = client
            .list_incoming_document_declarations(request(
                pb::ListIncomingDocumentDeclarationsRequest {
                    document: Some(selection.clone()),
                    ..Default::default()
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(incoming.declarations.len(), 1);
        let Some(pb::catalog_declaration::Declaration::Supporting(association)) =
            &incoming.declarations[0].declaration
        else {
            panic!("supporting provenance");
        };
        assert_eq!(association.document.as_ref(), Some(&selection));
    }
    let note_detail = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(pinned.clone()),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(note_detail.entity.as_ref(), Some(&note));
    assert_eq!(
        note_detail.anchor.unwrap().fragment.unwrap().value,
        "delivery"
    );
    for _ in 0..2 {
        let incoming = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: pb::EntityKind::Document as i32,
                    id: reference(&note).id,
                    ..Default::default()
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(incoming.references.len(), 2);
        assert!(incoming
            .references
            .iter()
            .all(|reference| reference.connection_id.is_some()
                && reference.from_uid == choice.system.as_ref().unwrap().uid));
        assert!(incoming
            .references
            .iter()
            .any(|reference| reference.document_selection.as_ref() == Some(&pinned)));
    }
    assert_eq!(
        delete(&mut client, &note, false, None)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    delete(&mut client, &note, true, None).await.unwrap();
    let unresolved = detail(&mut client, &choice, READER).await;
    assert_eq!(
        unresolved
            .declarations
            .iter()
            .filter(
                |declaration| declaration.state == pb::CatalogResolutionState::Unavailable as i32
            )
            .count(),
        2
    );
    put(&mut client, document("unrelated", selected), None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_replacement_uses_final_overlay_and_does_not_revalidate_unchanged_incoming_owners()
{
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    let replacing = put(
        &mut client,
        decision("new-choice", selected.clone(), true),
        None,
    )
    .await
    .entity
    .unwrap();
    let mut old = decision("old-choice", selected.clone(), true);
    if let Some(pb::entity::Kind::Decision(value)) = &mut old.kind {
        value.status = pb::DecisionStatus::Superseded as i32;
        value.replaced_by = Some(pb::DecisionReplacement {
            decision: Some(pb::DecisionRef {
                id: reference(&replacing).id,
            }),
            ..Default::default()
        });
    }
    let old = put(&mut client, old, None).await.entity.unwrap();
    let mut changed = replacing.clone();
    if let Some(pb::entity::Kind::Decision(value)) = &mut changed.kind {
        value.status = pb::DecisionStatus::Withdrawn as i32;
        value.selected_option = None;
        value.consequences.clear();
    }
    put(&mut client, changed, None).await;
    let chain = client
        .get_decision_detail(request(
            pb::GetDecisionDetailRequest {
                decision: Some(pb::DecisionRef {
                    id: reference(&old).id,
                }),
                follow_replacement: true,
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(chain.incomplete);
    assert_eq!(chain.failures[0].code, Code::FailedPrecondition as i32);
    put(&mut client, document("unrelated", selected.clone()), None).await;
    let invalid_edit = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(old.clone()),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await;
    assert_eq!(invalid_edit.unwrap_err().code(), Code::InvalidArgument);
    let a = decision("cycle-a", selected.clone(), true);
    let b = decision("cycle-b", selected, true);
    let mut values = [a, b];
    let ids = [reference(&values[0]).id, reference(&values[1]).id];
    for (index, value) in values.iter_mut().enumerate() {
        if let Some(pb::entity::Kind::Decision(value)) = &mut value.kind {
            value.status = pb::DecisionStatus::Superseded as i32;
            value.replaced_by = Some(pb::DecisionReplacement {
                decision: Some(pb::DecisionRef {
                    id: ids[1 - index].clone(),
                }),
                ..Default::default()
            });
        }
    }
    let result = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: values
                    .into_iter()
                    .map(|value| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(value),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await;
    assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_admission_rejects_invalid_body_state_and_closed_subject_inventory() {
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    let note = document("note", selected.clone());
    let mut invalid = Vec::new();
    let mut bad = note.clone();
    if let Some(pb::entity::Kind::Document(value)) = &mut bad.kind {
        if let Some(pb::document::Body::Retained(body)) = &mut value.body {
            body.digest.as_mut().unwrap().sha256 = "0".repeat(64);
        }
    }
    invalid.push(bad);
    let mut bad = note.clone();
    if let Some(pb::entity::Kind::Document(value)) = &mut bad.kind {
        value.body = Some(pb::document::Body::Link(pb::ExternalDocumentLink {
            value: "https://example.test/guide".into(),
        }));
    }
    invalid.push(bad);
    let mut bad = note.clone();
    if let Some(pb::entity::Kind::Document(value)) = &mut bad.kind {
        value.subjects.clear();
    }
    invalid.push(bad);
    let mut bad = decision("bad", selected.clone(), true);
    if let Some(pb::entity::Kind::Decision(value)) = &mut bad.kind {
        value.status = pb::DecisionStatus::Proposed as i32;
    }
    invalid.push(bad);
    for value in invalid {
        assert_eq!(
            client
                .put_entity(request(
                    pb::PutEntityRequest {
                        entity: Some(value),
                        ..Default::default()
                    },
                    ADMIN,
                    None
                ))
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
    }
    let note = put(&mut client, note, None).await.entity.unwrap();
    let bad_subject = subject(reference(&note));
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(decision("recursive", bad_subject, false)),
                    ..Default::default()
                },
                ADMIN,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(decision("denied", selected, false)),
                    ..Default::default()
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_generic_retarget_refuses_anchor_owner_substitution() {
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    let original = put(&mut client, document("note", selected.clone()), None)
        .await
        .entity
        .unwrap();
    let mut next = document("note", selected.clone());
    if let Some(pb::entity::Kind::Document(value)) = &mut next.kind {
        value.id.as_mut().unwrap().version = 2;
    }
    let next = put(&mut client, next, None).await.entity.unwrap();
    let mut choice = decision("choice", selected, false);
    if let Some(pb::entity::Kind::Decision(value)) = &mut choice.kind {
        value.documents = vec![pb::SupportingDocument {
            document: Some(anchor_selection(&original, true)),
            ..Default::default()
        }];
    }
    let choice = put(&mut client, choice, None).await.entity.unwrap();
    for dry_run in [true, false] {
        let result = client
            .retarget_references(request(
                pb::RetargetReferencesRequest {
                    from: Some(reference(&original)),
                    to: Some(reference(&next)),
                    dry_run,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await;
        assert_eq!(result.unwrap_err().code(), Code::FailedPrecondition);
        assert_eq!(
            detail(&mut client, &choice, READER).await.entity.as_ref(),
            Some(&choice)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_hidden_reads_hydration_and_referrers_preserve_authorization_boundaries() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("catalog-visible", "visible"),
        ("catalog-private", "hidden"),
    ] {
        namespaces.push(
            client
                .register_namespace(request(
                    pb::RegisterNamespaceRequest {
                        name: name.into(),
                        parent: parent.into(),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    let mut target = read_model("receipt");
    if let Some(pb::entity::Kind::ReadModel(value)) = &mut target.kind {
        value
            .id
            .as_mut()
            .unwrap()
            .namespace
            .clone_from(&namespaces[0]);
    }
    let target = put(&mut client, target, None).await.entity.unwrap();
    let selected = subject(reference(&target));
    let mut hidden = document("hidden-note", selected.clone());
    if let Some(pb::entity::Kind::Document(value)) = &mut hidden.kind {
        value
            .id
            .as_mut()
            .unwrap()
            .namespace
            .clone_from(&namespaces[1]);
        value.title = Some(title("Private retained title"));
    }
    let hidden = put(&mut client, hidden, None).await.entity.unwrap();
    let hidden_selection = anchor_selection(&hidden, true);
    let mut choice = decision("visible-choice", selected.clone(), false);
    if let Some(pb::entity::Kind::Decision(value)) = &mut choice.kind {
        value
            .id
            .as_mut()
            .unwrap()
            .namespace
            .clone_from(&namespaces[0]);
        value.documents = vec![pb::SupportingDocument {
            document: Some(hidden_selection.clone()),
            ..Default::default()
        }];
    }
    let choice = put(&mut client, choice, None).await.entity.unwrap();
    let direct = client
        .get_document_detail(request(
            pb::GetDocumentDetailRequest {
                document: Some(hidden_selection.clone()),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(direct.code(), Code::NotFound);
    assert!(!direct.message().contains("Private retained title"));
    let visible = detail(&mut client, &choice, LIMITED).await;
    let support = visible
        .declarations
        .iter()
        .find(|declaration| declaration.field.starts_with("documents["))
        .unwrap();
    assert_eq!(
        support.state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    let Some(pb::catalog_declaration::Declaration::Supporting(support)) = &support.declaration
    else {
        panic!("supporting");
    };
    assert_eq!(support.document.as_ref(), Some(&hidden_selection));
    assert!(visible.scope.unwrap().visited.iter().all(|owner| owner
        .owner
        .as_ref()
        .and_then(|owner| owner.id.as_ref())
        .is_none_or(|id| id.namespace != namespaces[1])));
    let reverse = client
        .list_subject_documents(request(
            pb::ListSubjectDocumentsRequest {
                subject: Some(selected),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reverse.declarations, Vec::<pb::CatalogDeclaration>::new());
    let protected = client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: reference(&target).id,
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(protected.code(), Code::FailedPrecondition);
    assert!(!protected.message().contains("hidden-note"));
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_safe_batch_uses_final_graph_and_branch_force_delete_leaves_exact_selectors() {
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    let note = put(&mut client, document("note", selected.clone()), None)
        .await
        .entity
        .unwrap();
    let mut choice = decision("choice", selected.clone(), false);
    if let Some(pb::entity::Kind::Decision(value)) = &mut choice.kind {
        value.documents = vec![pb::SupportingDocument {
            document: Some(anchor_selection(&note, true)),
            ..Default::default()
        }];
    }
    let choice = put(&mut client, choice, None).await.entity.unwrap();
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "catalog-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    delete(&mut client, &note, true, Some("catalog-branch"))
        .await
        .unwrap();
    let branch = client
        .get_decision_detail(request(
            pb::GetDecisionDetailRequest {
                decision: Some(pb::DecisionRef {
                    id: reference(&choice).id,
                }),
                follow_replacement: false,
            },
            READER,
            Some("catalog-branch"),
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(branch.entity.as_ref(), Some(&choice));
    assert!(branch
        .declarations
        .iter()
        .any(|declaration| declaration.state == pb::CatalogResolutionState::Unavailable as i32));
    assert!(detail(&mut client, &choice, READER)
        .await
        .declarations
        .iter()
        .all(|declaration| declaration.state == pb::CatalogResolutionState::Resolved as i32));
    put(
        &mut client,
        decision("unrelated", selected, false),
        Some("catalog-branch"),
    )
    .await;
    let merged = client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: "catalog-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(merged.conflicts, Vec::<pb::BranchDiffEntry>::new());
    assert_eq!(
        detail(&mut client, &choice, READER).await.entity.as_ref(),
        Some(&choice)
    );
    let targets = [&target, &choice];
    let ops = targets
        .into_iter()
        .map(|value| {
            let reference = reference(value);
            pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    kind: reference.kind,
                    id: reference.id,
                    ..Default::default()
                })),
            }
        })
        .collect();
    let denied = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        denied.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    let unrelated = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Decision as i32,
                id: Some(id("unrelated")),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let ops = [&target, &choice, &unrelated]
        .into_iter()
        .map(|value| {
            let reference = reference(value);
            pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    kind: reference.kind,
                    id: reference.id,
                    ..Default::default()
                })),
            }
        })
        .collect();
    let applied = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        applied.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_repeated_native_connections_keep_occurrences_and_responsibility_kinds() {
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let processor = put(
        &mut client,
        entity(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("notify")),
            title: "Notify".into(),
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let automation = put(
        &mut client,
        entity(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observe")),
            title: "Observe".into(),
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: reference(&processor).id,
                }),
                ..Default::default()
            }),
            source_read_models: (0..2)
                .map(|_| pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: reference(&target).id,
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(value)) = &automation.kind else {
        panic!("automation");
    };
    let selectors: Vec<_> = value
        .source_read_models
        .iter()
        .map(|edge| pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Connection(
                pb::ConnectionRef {
                    owner: Some(reference(&automation)),
                    owner_uid: automation.system.as_ref().unwrap().uid.clone(),
                    id: edge.connection_id.clone(),
                },
            )),
        })
        .collect();
    assert_ne!(selectors[0], selectors[1]);
    let mut choice = decision("choice", selectors[0].clone(), false);
    if let Some(pb::entity::Kind::Decision(value)) = &mut choice.kind {
        value.subjects.push(pb::SubjectAssociation {
            subject: Some(selectors[1].clone()),
            ..Default::default()
        });
    }
    let choice = put(&mut client, choice, None).await.entity.unwrap();
    let note = put(
        &mut client,
        document("note", subject(reference(&target))),
        None,
    )
    .await
    .entity
    .unwrap();
    for selected in selectors {
        let found = client
            .list_subject_decisions(request(
                pb::ListSubjectDecisionsRequest {
                    subject: Some(selected.clone()),
                    status: None,
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(found.declarations.len(), 1);
        assert_eq!(
            found.subject_state,
            pb::CatalogResolutionState::Resolved as i32
        );
        let Some(pb::catalog_declaration::Declaration::Subject(value)) =
            &found.declarations[0].declaration
        else {
            panic!("subject");
        };
        assert_eq!(value.subject.as_ref(), Some(&selected));
    }
    for (index, target) in [&choice, &note].into_iter().enumerate() {
        let selected = pb::ResponsibilitySubject {
            selection: Some(pb::responsibility_subject::Selection::Entity(reference(
                target,
            ))),
        };
        let person = entity(pb::entity::Kind::Person(pb::Person {
            id: Some(id(&format!("maintainer-{index}"))),
            title: Some(pb::DirectoryDisplayName {
                value: "Maintainer".into(),
            }),
            responsibilities: vec![pb::Responsibility {
                kind: pb::ResponsibilityKind::Steward as i32,
                subject: Some(selected.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }));
        put(&mut client, person, None).await;
        let assignments = client
            .list_subject_assignments(request(
                pb::ListSubjectAssignmentsRequest {
                    subject: Some(selected),
                    kind: None,
                },
                READER,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(assignments.assignments.len(), 1);
        assert_eq!(
            assignments.subject_state,
            pb::ResponsibilityResolutionState::Resolved as i32
        );
        assert_eq!(
            delete(&mut client, target, false, None)
                .await
                .unwrap_err()
                .code(),
            Code::FailedPrecondition
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_backend_failure_is_operational_and_cannot_authorize_delete() {
    let nats = trogon_atlas_testsupport::TestNats::start().await;
    let mut client = client_with_store(Arc::new(nats.store().await), 0).await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let choice = put(
        &mut client,
        decision("choice", subject(reference(&target)), false),
        None,
    )
    .await
    .entity
    .unwrap();
    nats.stop().await.unwrap();
    let failure = client
        .get_decision_detail(request(
            pb::GetDecisionDetailRequest {
                decision: Some(pb::DecisionRef {
                    id: reference(&choice).id,
                }),
                follow_replacement: false,
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(
            failure.code(),
            Code::Unavailable | Code::Internal | Code::DeadlineExceeded
        ),
        "{failure}"
    );
    let failure = delete(&mut client, &target, false, None).await.unwrap_err();
    assert!(
        matches!(
            failure.code(),
            Code::Unavailable | Code::Internal | Code::DeadlineExceeded | Code::FailedPrecondition
        ),
        "{failure}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_discovery_work_budget_accepts_boundary_and_shares_directory_rows() {
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    for batch in 0..10 {
        let ops = (0..100)
            .map(|index| {
                let value = entity(pb::entity::Kind::Person(pb::Person {
                    id: Some(id(&format!("member-{}", batch * 100 + index))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Member".into(),
                    }),
                    ..Default::default()
                }));
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(value),
                        ..Default::default()
                    })),
                }
            })
            .collect();
        let result = client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    ops,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            result.status,
            pb::batch_mutate_response::Status::Applied as i32
        );
    }
    for index in 0..10 {
        let count = if index == 9 { 999 } else { 1_000 };
        put(
            &mut client,
            entity(pb::entity::Kind::Team(pb::Team {
                id: Some(id(&format!("team-{index}"))),
                title: Some(pb::DirectoryDisplayName {
                    value: "Team".into(),
                }),
                members: (0..count)
                    .map(|member| pb::TeamMember {
                        person: Some(pb::PersonRef {
                            id: Some(id(&format!("member-{member}"))),
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            None,
        )
        .await;
    }
    let selected = subject(reference(&target));
    let mut note = document("boundary", selected.clone());
    if let Some(pb::entity::Kind::Document(value)) = &mut note.kind {
        value.anchors.clear();
    }
    put(&mut client, note, None).await;
    let request_body = pb::ListSubjectDocumentsRequest {
        subject: Some(selected.clone()),
    };
    let boundary = client
        .list_subject_documents(request(request_body.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(boundary.declarations.len(), 1);
    assert!(!boundary.incomplete);
    let mut extra = document("beyond", selected);
    if let Some(pb::entity::Kind::Document(value)) = &mut extra.kind {
        value.anchors.clear();
    }
    put(&mut client, extra, None).await;
    let failure = client
        .list_subject_documents(request(request_body, READER, None))
        .await
        .unwrap_err();
    assert_eq!(failure.code(), Code::ResourceExhausted);
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_discovery_output_and_shared_row_boundaries_are_exact() {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 10_000).await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let destination = put(&mut client, read_model("retarget-destination"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    for start in (0..2_047).step_by(128) {
        let ops: Vec<_> = (start..(start + 128).min(2_047))
            .map(|index| {
                let mut value = document(&format!("note-{index}"), selected.clone());
                if let Some(pb::entity::Kind::Document(value)) = &mut value.kind {
                    value.anchors.clear();
                }
                MutationOp::Create {
                    kind: pb::EntityKind::Document,
                    entity: value,
                }
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    let request_body = pb::ListSubjectDocumentsRequest {
        subject: Some(selected.clone()),
    };
    let boundary = client
        .list_subject_documents(request(request_body.clone(), READER, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(boundary.declarations.len(), 2_047);
    let checked = boundary.scope.as_ref().unwrap();
    assert_eq!(
        boundary.declarations.len()
            + checked.visited.len()
            + checked.visited_subjects.len()
            + checked.visited_documents.len(),
        4_096
    );
    assert!(!boundary.incomplete);
    for start in (2_047..4_096).step_by(128) {
        let ops: Vec<_> = (start..(start + 128).min(4_096))
            .map(|index| {
                let mut value = document(&format!("note-{index}"), selected.clone());
                if let Some(pb::entity::Kind::Document(value)) = &mut value.kind {
                    value.anchors.clear();
                }
                MutationOp::Create {
                    kind: pb::EntityKind::Document,
                    entity: value,
                }
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    assert_eq!(
        client
            .list_subject_documents(request(request_body.clone(), READER, None))
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
    let failure = client
        .list_decision_amendments(request(
            pb::ListDecisionAmendmentsRequest {
                decision: Some(pb::DecisionRef {
                    id: Some(id("undeclared-decision")),
                }),
                direction: pb::DecisionAmendmentDirection::Incoming as i32,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(failure.code(), Code::ResourceExhausted);
    assert!(failure.message().contains("output"));
    let boundary_retarget = client
        .retarget_references(request(
            pb::RetargetReferencesRequest {
                from: Some(reference(&target)),
                to: Some(reference(&destination)),
                dry_run: true,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(boundary_retarget.results.len(), 4_096);
    assert_eq!(boundary_retarget.rewritten, 4_096);
    assert_eq!(boundary_retarget.failed, 0);
    let original_owner = client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Document as i32,
                id: Some(id("note-0")),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let report_request = pb::DeleteEntityRequest {
        kind: pb::EntityKind::ReadModel as i32,
        id: reference(&target).id,
        mode: pb::delete_entity_request::Mode::DryRun as i32,
        ..Default::default()
    };
    let report = client
        .delete_entity(request(report_request.clone(), ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(report.referrers.len(), 4_096);
    assert!(!report.referrers_incomplete);
    let mut extra = document("extra", selected.clone());
    if let Some(pb::entity::Kind::Document(value)) = &mut extra.kind {
        value.anchors.clear();
    }
    store
        .create(pb::EntityKind::Document, &extra, WriteContext::baseline())
        .await
        .unwrap();
    let beyond = client
        .list_subject_documents(request(request_body, READER, None))
        .await
        .unwrap_err();
    assert_eq!(beyond.code(), Code::ResourceExhausted);
    assert!(beyond.message().contains("output"));
    let mut unexpected = Vec::new();
    for dry_run in [true, false] {
        match client
            .retarget_references(request(
                pb::RetargetReferencesRequest {
                    from: Some(reference(&target)),
                    to: Some(reference(&destination)),
                    dry_run,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
        {
            Ok(response) => {
                let response = response.into_inner();
                unexpected.push(format!(
                    "dry_run={dry_run} returned {} owners and {} rewrites",
                    response.results.len(),
                    response.rewritten
                ));
            }
            Err(error) => assert_eq!(error.code(), Code::ResourceExhausted),
        }
    }
    assert!(unexpected.is_empty(), "{}", unexpected.join("; "));
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Document as i32,
                    id: Some(id("note-0"))
                },
                READER,
                None
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .as_ref(),
        Some(&original_owner)
    );
    for mode in [
        pb::delete_entity_request::Mode::DryRun,
        pb::delete_entity_request::Mode::Force,
    ] {
        let operation_id = if mode == pb::delete_entity_request::Mode::Force {
            uuid::Uuid::now_v7().to_string()
        } else {
            String::new()
        };
        let attempt = pb::DeleteEntityRequest {
            mode: mode as i32,
            operation_id: operation_id.clone(),
            ..report_request.clone()
        };
        for _ in 0..2 {
            let failure = match client
                .delete_entity(request(attempt.clone(), ADMIN, None))
                .await
            {
                Err(failure) => failure,
                Ok(report) => panic!(
                    "Expected output-budget refusal, received {} referrers",
                    report.into_inner().referrers.len()
                ),
            };
            assert_eq!(failure.code(), Code::ResourceExhausted);
        }
        if !operation_id.is_empty() {
            let outcome = client
                .get_operation(request(
                    pb::GetOperationRequest { operation_id },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(outcome.status, pb::OperationStatus::Rejected as i32);
            assert_eq!(outcome.changeset_id, "");
        }
        assert_eq!(
            client
                .get_entity(request(
                    pb::GetEntityRequest {
                        kind: pb::EntityKind::ReadModel as i32,
                        id: reference(&target).id
                    },
                    READER,
                    None
                ))
                .await
                .unwrap()
                .into_inner()
                .entity
                .as_ref(),
            Some(&target)
        );
    }
    for start in (0..903).step_by(128) {
        let ops: Vec<_> = (start..(start + 128).min(903))
            .map(|index| MutationOp::Create {
                kind: pb::EntityKind::Person,
                entity: entity(pb::entity::Kind::Person(pb::Person {
                    id: Some(id(&format!("profile-{index}"))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Profile".into(),
                    }),
                    ..Default::default()
                })),
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    let query = pb::ListSubjectDecisionsRequest {
        subject: Some(selected),
        status: None,
    };
    let boundary = client
        .list_subject_decisions(request(query.clone(), READER, None))
        .await
        .unwrap_err();
    assert_eq!(boundary.code(), Code::ResourceExhausted);
    assert!(boundary.message().contains("output"));
    store
        .create(
            pb::EntityKind::Person,
            &entity(pb::entity::Kind::Person(pb::Person {
                id: Some(id("beyond-profile")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Profile".into(),
                }),
                ..Default::default()
            })),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let beyond = client
        .list_subject_decisions(request(query, READER, None))
        .await
        .unwrap_err();
    assert_eq!(beyond.code(), Code::ResourceExhausted);
    assert!(beyond.message().contains("entity budget"));
    let report = client
        .delete_entity(request(report_request, ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(report.referrers.len(), 4_096);
    assert!(report.referrers_incomplete);
}

#[tokio::test(flavor = "multi_thread")]
async fn referenced_delete_refuses_as_referenced_beyond_the_report_budget() {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 10_000).await;
    let target = put(&mut client, read_model("hub"), None)
        .await
        .entity
        .unwrap();
    for start in (0..4_097).step_by(128) {
        let ops: Vec<_> = (start..(start + 128).min(4_097))
            .map(|index| {
                let mut value = document(&format!("note-{index}"), subject(reference(&target)));
                if let Some(pb::entity::Kind::Document(value)) = &mut value.kind {
                    value.anchors.clear();
                }
                MutationOp::Create {
                    kind: pb::EntityKind::Document,
                    entity: value,
                }
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    let failure = delete(&mut client, &target, false, None).await.unwrap_err();
    assert_eq!(failure.code(), Code::FailedPrecondition, "{failure:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_batch_delete_reporting_shares_output_budget_before_mutation() {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 10_000).await;
    let mut targets = Vec::new();
    for slug in ["first", "second"] {
        targets.push(
            put(&mut client, read_model(slug), None)
                .await
                .entity
                .unwrap(),
        );
    }
    for start in (0..2_049).step_by(128) {
        let ops: Vec<_> = (start..(start + 128).min(2_049))
            .map(|index| {
                let mut value = document(&format!("note-{index}"), subject(reference(&targets[0])));
                if let Some(pb::entity::Kind::Document(value)) = &mut value.kind {
                    value.anchors.clear();
                    value.subjects.push(pb::SubjectAssociation {
                        subject: Some(subject(reference(&targets[1]))),
                        ..Default::default()
                    });
                }
                MutationOp::Create {
                    kind: pb::EntityKind::Document,
                    entity: value,
                }
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    for validate_only in [true, false] {
        let operation_id = if validate_only {
            String::new()
        } else {
            uuid::Uuid::now_v7().to_string()
        };
        let batch = pb::BatchMutateRequest {
            ops: targets
                .iter()
                .map(|value| pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                        kind: pb::EntityKind::ReadModel as i32,
                        id: reference(value).id,
                        mode: pb::delete_entity_request::Mode::Force as i32,
                        ..Default::default()
                    })),
                })
                .collect(),
            validate_only,
            operation_id: operation_id.clone(),
        };
        let failure = match client.batch_mutate(request(batch, ADMIN, None)).await {
            Err(failure) => failure,
            Ok(response) => panic!(
                "Expected shared report-budget refusal, received batch status {}",
                response.into_inner().status
            ),
        };
        assert_eq!(failure.code(), Code::ResourceExhausted);
        if !operation_id.is_empty() {
            let outcome = client
                .get_operation(request(
                    pb::GetOperationRequest { operation_id },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner();
            assert_ne!(outcome.status, pb::OperationStatus::Applied as i32);
            assert_eq!(outcome.changeset_id, "");
        }
        for target in &targets {
            assert_eq!(
                client
                    .get_entity(request(
                        pb::GetEntityRequest {
                            kind: pb::EntityKind::ReadModel as i32,
                            id: reference(target).id
                        },
                        READER,
                        None
                    ))
                    .await
                    .unwrap()
                    .into_inner()
                    .entity
                    .as_ref(),
                Some(target)
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_delete_report_incompleteness_survives_force_batch_preview_and_replay() {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let mut client = client_with_store(store.clone(), 10_000).await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    let note = put(&mut client, document("note", selected), None)
        .await
        .entity
        .unwrap();
    let query = pb::DeleteEntityRequest {
        kind: pb::EntityKind::ReadModel as i32,
        id: reference(&target).id,
        mode: pb::delete_entity_request::Mode::DryRun as i32,
        ..Default::default()
    };
    let complete = client
        .delete_entity(request(query.clone(), ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(complete.referrers.len(), 1);
    assert!(!complete.referrers_incomplete);
    for start in (0..5_000).step_by(128) {
        let ops: Vec<_> = (start..(start + 128).min(5_000))
            .map(|index| MutationOp::Create {
                kind: pb::EntityKind::Person,
                entity: entity(pb::entity::Kind::Person(pb::Person {
                    id: Some(id(&format!("profile-{index}"))),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Profile".into(),
                    }),
                    ..Default::default()
                })),
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    let incomplete = client
        .delete_entity(request(query.clone(), ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert!(
        incomplete.referrers_incomplete,
        "visible-row fallback must be explicit"
    );
    assert_eq!(
        delete(&mut client, &target, false, None)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let batch = pb::BatchMutateRequest {
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                kind: pb::EntityKind::Person as i32,
                id: Some(id("profile-0")),
                ..Default::default()
            })),
        }],
        validate_only: true,
        ..Default::default()
    };
    let preview = client
        .batch_mutate(request(batch.clone(), ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    let Some(pb::batch_mutate_op_result::Result::Delete(deleted)) = &preview.results[0].result
    else {
        panic!("delete preview");
    };
    assert!(deleted.referrers_incomplete);
    let applied = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                validate_only: false,
                ..batch
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let Some(pb::batch_mutate_op_result::Result::Delete(deleted)) = &applied.results[0].result
    else {
        panic!("delete result");
    };
    assert!(deleted.referrers_incomplete);
    store
        .create(
            pb::EntityKind::Person,
            &entity(pb::entity::Kind::Person(pb::Person {
                id: Some(id("replacement-profile")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Profile".into(),
                }),
                ..Default::default()
            })),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let forced = pb::DeleteEntityRequest {
        kind: pb::EntityKind::Document as i32,
        id: reference(&note).id,
        mode: pb::delete_entity_request::Mode::Force as i32,
        operation_id: uuid::Uuid::now_v7().to_string(),
        ..Default::default()
    };
    let fresh = client
        .delete_entity(request(forced.clone(), ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert!(fresh.referrers_incomplete);
    assert!(!fresh.operation_receipt.unwrap().replayed);
    let replay = client
        .delete_entity(request(forced, ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert!(replay.referrers_incomplete);
    assert!(replay.operation_receipt.unwrap().replayed);
    let complete = client
        .delete_entity(request(query, ADMIN, None))
        .await
        .unwrap()
        .into_inner();
    assert!(!complete.referrers_incomplete);
    assert_eq!(complete.referrers, Vec::<pb::Reference>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_validation_rules_describe_and_enrich_retained_dangling_subjects() {
    let mut client = client_with_cap(1_000).await;
    let target = put(&mut client, read_model("receipt"), None)
        .await
        .entity
        .unwrap();
    let choice = put(
        &mut client,
        decision("choice", subject(reference(&target)), false),
        None,
    )
    .await
    .entity
    .unwrap();
    delete(&mut client, &target, true, None).await.unwrap();
    let catalog = client
        .list_validation_rules(request(pb::ListValidationRulesRequest {}, READER, None))
        .await
        .unwrap()
        .into_inner();
    let rule = catalog
        .rules
        .iter()
        .find(|rule| rule.code == "CATALOG_REFERENCE_UNRESOLVED")
        .expect("emitted Catalog code must have a discoverable rule");
    assert_ne!(rule.title, "");
    assert_ne!(rule.doc, "");
    assert_eq!(rule.category, "architecture");
    assert_eq!(
        rule.default_severity,
        pb::validation_issue::Severity::Error as i32
    );
    let report = client
        .validate_project(request(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    "catalog".into(),
                )),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let findings: Vec<_> = report
        .reports
        .iter()
        .flat_map(|report| &report.issues)
        .filter(|issue| {
            issue.code == rule.code && issue.subject.as_ref() == Some(&reference(&choice))
        })
        .collect();
    assert_ne!(findings, Vec::<&pb::ValidationIssue>::new());
    for finding in findings {
        assert_eq!(finding.rule_title, rule.title);
        assert_eq!(finding.rule_category, rule.category);
        assert_eq!(finding.rule_default_severity, rule.default_severity);
    }
}
