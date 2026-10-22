#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto::{
    self as pb, event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type Client = EventModelServiceClient<tonic::transport::Channel>;
async fn client() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
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
    let content = "# Delivery\nDelivery guidance.\n";
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
fn decision(slug: &str, selected: pb::CatalogSubject) -> pb::Entity {
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
        status: pb::DecisionStatus::Proposed as i32,
        context: Some(text("Choose delivery")),
        rationale: Some(text("Keep delivery explicit")),
        options: vec![pb::DecisionOption {
            option_id: Some(option_id.clone()),
            title: Some(title("Native delivery")),
            description: Some(text("Retain the selected choice")),
        }],
        ..Default::default()
    }))
}
async fn put(client: &mut Client, value: pb::Entity) -> pb::PutEntityResponse {
    client
        .put_entity(pb::PutEntityRequest {
            entity: Some(value),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_document_diagnostics_identify_each_supporting_and_navigation_occurrence() {
    let mut client = client().await;
    let target = put(&mut client, read_model("receipt"))
        .await
        .entity
        .unwrap();
    let selected = subject(reference(&target));
    let mut note = document("note", selected.clone());
    if let Some(pb::entity::Kind::Document(value)) = &mut note.kind {
        value.anchors.push(pb::DocumentAnchor {
            fragment: Some(pb::AnchorFragment {
                value: "operations".into(),
            }),
            label: Some(title("Operations")),
            ..Default::default()
        });
    }
    let note = put(&mut client, note).await.entity.unwrap();
    let Some(pb::entity::Kind::Document(note_value)) = &note.kind else {
        unreachable!()
    };
    let whole = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: reference(&note).id,
            },
        )),
    };
    let pinned = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Anchor(
            pb::DocumentAnchorRef {
                document: Some(pb::DocumentRef {
                    id: reference(&note).id,
                }),
                owner_uid: note.system.as_ref().unwrap().uid.clone(),
                anchor_id: note_value.anchors[0].anchor_id.clone(),
                expected_digest: match &note_value.body {
                    Some(pb::document::Body::Retained(body)) => body.digest.clone(),
                    _ => unreachable!(),
                },
            },
        )),
    };
    let mut second_pin = pinned.clone();
    if let Some(pb::document_selection::Selection::Anchor(anchor)) = &mut second_pin.selection {
        anchor.anchor_id = note_value.anchors[1].anchor_id.clone();
    }
    let mut choice = decision("choice", selected.clone());
    if let Some(pb::entity::Kind::Decision(value)) = &mut choice.kind {
        value.documents = [whole.clone(), pinned.clone(), second_pin.clone()]
            .into_iter()
            .map(|document| pb::SupportingDocument {
                document: Some(document),
                ..Default::default()
            })
            .collect();
    }
    let choice = put(&mut client, choice).await.entity.unwrap();
    let mut navigation = document("navigation", selected);
    if let Some(pb::entity::Kind::Document(value)) = &mut navigation.kind {
        value.navigation = [whole, pinned.clone(), second_pin.clone()]
            .into_iter()
            .map(|document| pb::DocumentNavigation {
                document: Some(document),
                ..Default::default()
            })
            .collect();
    }
    let navigation = put(&mut client, navigation).await.entity.unwrap();
    let mut changed = note;
    if let Some(pb::entity::Kind::Document(value)) = &mut changed.kind {
        let Some(pb::document::Body::Retained(body)) = &mut value.body else {
            unreachable!()
        };
        body.content.push_str("Changed bytes\n");
        body.digest = Some(trogon_atlas_core::specification::content_digest(
            &body.content,
        ));
    }
    put(&mut client, changed).await;
    let report = client
        .validate_project(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                "catalog".into(),
            )),
        })
        .await
        .unwrap()
        .into_inner();
    let mut actual_fields = Vec::new();
    for (owner, prefix) in [(&choice, "documents"), (&navigation, "navigation")] {
        let mut fields: Vec<_> = report
            .reports
            .iter()
            .flat_map(|report| &report.issues)
            .filter(|issue| {
                issue.code == "CATALOG_REFERENCE_UNRESOLVED"
                    && issue.subject.as_ref() == Some(&reference(owner))
            })
            .map(|issue| issue.subject_field.as_str())
            .collect();
        fields.sort_unstable();
        actual_fields.push((prefix, fields));
    }
    assert_eq!(
        actual_fields,
        vec![
            (
                "documents",
                vec!["documents[1].document", "documents[2].document"]
            ),
            (
                "navigation",
                vec!["navigation[1].document", "navigation[2].document"]
            ),
        ]
    );
    for owner in [&choice, &navigation] {
        let selections: Vec<_> = match &owner.kind {
            Some(pb::entity::Kind::Decision(value)) => value
                .documents
                .iter()
                .map(|association| (&association.connection_id, &association.document))
                .collect(),
            Some(pb::entity::Kind::Document(value)) => value
                .navigation
                .iter()
                .map(|association| (&association.connection_id, &association.document))
                .collect(),
            _ => unreachable!(),
        };
        assert_ne!(selections[1].0, selections[2].0);
        assert!(selections[1].0.is_some());
        assert_eq!(selections[1].1.as_ref(), Some(&pinned));
        assert_eq!(selections[2].1.as_ref(), Some(&second_pin));
    }
}
