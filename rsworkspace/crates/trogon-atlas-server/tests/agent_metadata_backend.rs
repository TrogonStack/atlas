use std::sync::Arc;

use prost::Message;
use tonic::Request;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn assert_metadata_rejected(entity: pb::Entity) -> Result<(), Box<dyn std::error::Error>> {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )?;
    let kind = trogon_atlas_core::entity_kind(&entity).ok_or("missing fixture kind")?;
    let id = trogon_atlas_core::entity_id(&entity)
        .ok_or("missing fixture ID")?
        .clone();
    let rejected = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            create_only: true,
            ..Default::default()
        }))
        .await
        .err()
        .ok_or("invalid metadata was accepted")?;
    assert_eq!(rejected.code(), tonic::Code::InvalidArgument);
    let missing = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id),
        }))
        .await
        .err()
        .ok_or("rejected metadata published an entity")?;
    assert_eq!(missing.code(), tonic::Code::NotFound);
    Ok(())
}

#[tokio::test]
async fn agent_annotation_rejects_a_behavioral_owner_without_publication(
) -> Result<(), Box<dyn std::error::Error>> {
    assert_metadata_rejected(pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "metadata-boundaries".into(),
                slug: "agent-on-event".into(),
                version: 1,
            }),
            title: "Agent on event".into(),
            metadata: vec![prost_types::Any {
                type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation"
                    .into(),
                value: Vec::new(),
            }],
            ..Default::default()
        })),
        ..Default::default()
    })
    .await
}

#[tokio::test]
async fn malformed_lifecycle_annotation_rejects_without_publication(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut entity = boundary("malformed-lifecycle", false, None);
    if let Some(pb::entity::Kind::Component(value)) = entity.kind.as_mut() {
        value.metadata.push(prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation"
                .into(),
            value: vec![0xff],
        });
    }
    assert_metadata_rejected(entity).await
}

#[tokio::test]
async fn duplicate_lifecycle_annotations_reject_without_publication(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut entity = boundary("duplicate-lifecycle", false, Some("draft"));
    if let Some(pb::entity::Kind::Component(value)) = entity.kind.as_mut() {
        value.metadata.push(lifecycle("draft"));
    }
    assert_metadata_rejected(entity).await
}

fn lifecycle(status: &str) -> prost_types::Any {
    prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation".into(),
        value: pb::LifecycleAnnotation {
            status: status.into(),
            ..Default::default()
        }
        .encode_to_vec(),
    }
}

fn boundary(slug: &str, external: bool, status: Option<&str>) -> pb::Entity {
    let id = Some(pb::Id {
        namespace: "metadata-boundaries".into(),
        slug: slug.into(),
        version: 1,
    });
    let metadata = status.map(lifecycle).into_iter().collect();
    pb::Entity {
        kind: Some(if external {
            pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                id,
                title: slug.into(),
                metadata,
                ..Default::default()
            })
        } else {
            pb::entity::Kind::Component(pb::Component {
                id,
                title: slug.into(),
                metadata,
                ..Default::default()
            })
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn lifecycle_filters_include_implementation_and_external_boundaries(
) -> Result<(), Box<dyn std::error::Error>> {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )?;
    for external in [false, true] {
        let kind = if external {
            pb::EntityKind::ExternalSystem
        } else {
            pb::EntityKind::Component
        };
        for (suffix, status) in [
            ("draft", Some(" DrAfT ")),
            ("accepted", Some("accepted")),
            ("undeclared", None),
        ] {
            service
                .put_entity(Request::new(pb::PutEntityRequest {
                    entity: Some(boundary(
                        &format!(
                            "{}-{suffix}",
                            if external { "external" } else { "component" }
                        ),
                        external,
                        status,
                    )),
                    create_only: true,
                    ..Default::default()
                }))
                .await?;
        }
        for (include, exclude, expected_suffix) in [
            (vec!["draft".into()], vec![], "-draft"),
            (
                vec!["draft".into(), "accepted".into()],
                vec!["draft".into()],
                "-accepted",
            ),
            (vec![String::new()], vec![], "-undeclared"),
        ] {
            let result = service
                .list_entities(Request::new(pb::ListEntitiesRequest {
                    namespaces: vec!["metadata-boundaries".into()],
                    kinds: vec![kind as i32],
                    lifecycle_status_in: include,
                    lifecycle_status_not_in: exclude,
                    ..Default::default()
                }))
                .await?
                .into_inner();
            assert_eq!(result.entities.len(), 1, "lifecycle filter for {kind:?}");
            let selected = trogon_atlas_core::entity_id(&result.entities[0]);
            assert!(selected.is_some_and(|id| id.slug.ends_with(expected_suffix)));
            assert_eq!(result.next_page_token, "");
        }
    }
    Ok(())
}
