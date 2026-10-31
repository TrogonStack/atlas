#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use support::two_tenants::{authed, make_registry, serve_with_registry, ROOT_TOKEN};
use tonic::transport::Channel;
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 9_007_199_254_740_993,
    }
}

async fn put(
    client: &mut EventModelServiceClient<Channel>,
    kind: pb::entity::Kind,
) -> pb::PutEntityResponse {
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(kind),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn connection(entity: &pb::Entity, path: &str) -> pb::ConnectionRef {
    let occurrence = trogon_atlas_core::connections::connection_occurrences(entity)
        .unwrap()
        .into_iter()
        .find(|item| item.path == path)
        .unwrap();
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(entity).unwrap()),
        owner_uid: entity.system.as_ref().unwrap().uid.clone(),
        id: Some(pb::ConnectionId {
            value: occurrence.id.unwrap().to_string(),
        }),
    }
}

async fn fixture() -> (
    EventModelServiceClient<Channel>,
    pb::InspectHumanInteractionRequest,
    pb::InspectHumanInteractionRequest,
) {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "human-inspection".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let ui = id(&namespace, "review");
    let persona = id(&namespace, "reviewer");
    let command = id(&namespace, "approve");
    let read_model = id(&namespace, "summary");
    put(
        &mut client,
        pb::entity::Kind::Ui(pb::Ui {
            id: Some(ui.clone()),
            title: "Review".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Persona(pb::Persona {
            id: Some(persona.clone()),
            title: "Reviewer".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Command(pb::Command {
            id: Some(command.clone()),
            title: "Approve".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(read_model.clone()),
            title: "Summary".into(),
            ..Default::default()
        }),
    )
    .await;
    let hosting = pb::UiHosting {
        ui: Some(pb::UiRef {
            id: Some(ui.clone()),
        }),
        label: Some(pb::CatalogTitle {
            value: "Review portal 🦜".into(),
        }),
        ..Default::default()
    };
    let component = put(
        &mut client,
        pb::entity::Kind::Component(pb::Component {
            id: Some(id(&namespace, "portal")),
            title: "Portal".into(),
            ui_hostings: vec![hosting.clone(), hosting],
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let slice = put(
        &mut client,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(&namespace, "issue")),
            title: "Approve from review".into(),
            persona: Some(pb::PersonaEdge {
                persona: Some(pb::PersonaRef {
                    id: Some(persona.clone()),
                }),
                ..Default::default()
            }),
            ui: Some(pb::UiEdge {
                ui: Some(pb::UiRef {
                    id: Some(ui.clone()),
                }),
                ..Default::default()
            }),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef { id: Some(command) }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let read_edge = pb::ReadModelEdge {
        read_model: Some(pb::ReadModelRef {
            id: Some(read_model),
        }),
        ..Default::default()
    };
    let display = put(
        &mut client,
        pb::entity::Kind::UiSlice(pb::UiSlice {
            id: Some(id(&namespace, "display")),
            title: "Review summary".into(),
            persona: Some(pb::PersonaEdge {
                persona: Some(pb::PersonaRef { id: Some(persona) }),
                ..Default::default()
            }),
            ui: Some(pb::UiEdge {
                ui: Some(pb::UiRef { id: Some(ui) }),
                ..Default::default()
            }),
            source_read_models: vec![read_edge.clone(), read_edge],
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let issuance = pb::InspectHumanInteractionRequest {
        hosting: Some(connection(&component, "component.ui_hostings[1]")),
        path: Some(pb::inspect_human_interaction_request::Path::Issuance(
            pb::HumanIssuanceSelection {
                persona: Some(connection(&slice, "command_slice.persona")),
                ui: Some(connection(&slice, "command_slice.ui")),
                command: Some(connection(&slice, "command_slice.command")),
            },
        )),
    };
    let display = pb::InspectHumanInteractionRequest {
        hosting: Some(connection(&component, "component.ui_hostings[0]")),
        path: Some(pb::inspect_human_interaction_request::Path::Display(
            pb::HumanDisplaySelection {
                persona: Some(connection(&display, "ui_slice.persona")),
                ui: Some(connection(&display, "ui_slice.ui")),
                source_read_model: Some(connection(&display, "ui_slice.source_read_models[1]")),
            },
        )),
    };
    (client, issuance, display)
}

#[tokio::test(flavor = "multi_thread")]
async fn inspect_exact_human_paths_returns_coherent_checked_receipts() {
    let (mut client, issuance, display) = fixture().await;
    for request in [issuance, display] {
        let response = client
            .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.request, Some(request));
        assert_eq!(response.state, pb::CatalogResolutionState::Resolved as i32);
        assert_eq!(
            response.consistency,
            pb::HumanInteractionConsistency::PerOwnerCheckedReadSet as i32
        );
        assert_eq!(response.edges.len(), 4);
        assert!(response.edges.iter().all(|edge| edge
            .checked_target
            .as_ref()
            .is_some_and(|row| !row.uid.is_empty() && !row.etag.is_empty())));
        let scope = response.scope.unwrap();
        assert_eq!(scope.visited.len(), 5);
        assert!(!scope.revision.is_empty());
        assert_eq!(
            scope
                .visited
                .iter()
                .map(|row| row.owner.clone())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            5
        );
    }
}
