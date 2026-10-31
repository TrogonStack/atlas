#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use support::two_tenants::{authed, serve_with_registry, ROOT_TOKEN};
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
    let registry = trogon_atlas_server::auth::TokenRegistry::from_file_contents(
        toml::from_str(
            r#"
[principals.root]
role = "admin"
tokens = ["tt-root-token"]
[principals.reader]
role = "reader"
parent = "default"
tokens = ["human-reader-token"]
[principals.hidden]
role = "reader"
parent = "hidden-owner"
tokens = ["human-hidden-token"]
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let mut client = serve_with_registry(service, registry).await;
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

async fn get(
    client: &mut EventModelServiceClient<Channel>,
    selected: &pb::ConnectionRef,
) -> pb::GetEntityResponse {
    let owner = selected.owner.as_ref().unwrap();
    client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: owner.kind,
                id: owner.id.clone(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_roles_siblings_and_owner_incarnations_never_substitute() {
    let (mut client, issuance, display) = fixture().await;
    for case in 0..6 {
        let mut request = issuance.clone();
        match case {
            0 => request.hosting.as_mut().unwrap().owner_uid = "stale-component".into(),
            1 => {
                request.hosting.as_mut().unwrap().id.as_mut().unwrap().value =
                    "123e4567-e89b-42d3-a456-426614174000".into()
            }
            2 => {
                let pb::inspect_human_interaction_request::Path::Issuance(path) =
                    request.path.as_mut().unwrap()
                else {
                    unreachable!()
                };
                path.persona = path.command.clone();
            }
            3 => {
                let pb::inspect_human_interaction_request::Path::Issuance(path) =
                    request.path.as_mut().unwrap()
                else {
                    unreachable!()
                };
                path.persona.as_mut().unwrap().owner_uid = "stale-slice".into();
            }
            4 => {
                let pb::inspect_human_interaction_request::Path::Issuance(path) =
                    request.path.as_mut().unwrap()
                else {
                    unreachable!()
                };
                let pb::inspect_human_interaction_request::Path::Display(other) =
                    display.path.as_ref().unwrap()
                else {
                    unreachable!()
                };
                path.ui = other.ui.clone();
            }
            _ => {
                let pb::inspect_human_interaction_request::Path::Issuance(path) =
                    request.path.as_ref().unwrap()
                else {
                    unreachable!()
                };
                request.hosting = path.ui.clone();
            }
        }
        let result = client
            .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(result.request, Some(request));
        assert_eq!(
            result.state,
            pb::CatalogResolutionState::Unresolved as i32,
            "case {case}"
        );
        assert!(result.edges.is_empty(), "case {case}");
    }
    for case in 0..4 {
        let mut request = issuance.clone();
        match case {
            0 => request.hosting = None,
            1 => request.path = None,
            2 => {
                let pb::inspect_human_interaction_request::Path::Issuance(path) =
                    request.path.as_mut().unwrap()
                else {
                    unreachable!()
                };
                path.persona = None;
            }
            _ => request
                .hosting
                .as_mut()
                .unwrap()
                .id
                .as_mut()
                .unwrap()
                .value
                .clear(),
        }
        assert_eq!(
            client
                .inspect_human_interaction(authed(request, ROOT_TOKEN))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn observation_reorder_tracks_exact_occurrence_and_fresh_owner_etag() {
    let (mut client, _, request) = fixture().await;
    let original = client
        .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    let pb::inspect_human_interaction_request::Path::Display(path) = request.path.as_ref().unwrap()
    else {
        unreachable!()
    };
    let selection = path.source_read_model.as_ref().unwrap();
    let current = get(&mut client, selection).await;
    let mut proposed = current.entity.unwrap();
    let Some(pb::entity::Kind::UiSlice(slice)) = proposed.kind.as_mut() else {
        unreachable!()
    };
    slice.source_read_models.reverse();
    let changed = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(proposed),
                if_match: current.etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(changed.etag, current.etag);
    let result = client
        .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(result.state, pb::CatalogResolutionState::Resolved as i32);
    let read = result
        .edges
        .iter()
        .find(|edge| edge.role == pb::HumanInteractionRole::ReadModel as i32)
        .unwrap();
    assert_eq!(read.selection.as_ref(), Some(selection));
    assert_eq!(read.field, "ui_slice.source_read_models[0]");
    assert_ne!(
        original.scope.as_ref().unwrap().revision,
        result.scope.as_ref().unwrap().revision
    );
    assert!(result
        .scope
        .as_ref()
        .unwrap()
        .visited
        .iter()
        .any(|row| row.owner == selection.owner && row.etag == changed.etag));
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_can_inspect_but_hosting_and_path_grant_no_write_or_hidden_owner_access() {
    let (mut client, request, _) = fixture().await;
    let before = get(&mut client, request.hosting.as_ref().unwrap()).await;
    let inspected = client
        .inspect_human_interaction(authed(request.clone(), "human-reader-token"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(inspected.state, pb::CatalogResolutionState::Resolved as i32);
    let mut edit = before.entity.clone().unwrap();
    let Some(pb::entity::Kind::Component(component)) = edit.kind.as_mut() else {
        unreachable!()
    };
    component.title = "Denied edit".into();
    assert_eq!(
        client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(edit),
                    if_match: before.etag.clone(),
                    ..Default::default()
                },
                "human-reader-token"
            ))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        get(&mut client, request.hosting.as_ref().unwrap()).await,
        before
    );
    let hidden = client
        .inspect_human_interaction(authed(request.clone(), "human-hidden-token"))
        .await
        .unwrap_err();
    let mut missing = request.clone();
    missing
        .hosting
        .as_mut()
        .unwrap()
        .owner
        .as_mut()
        .unwrap()
        .id
        .as_mut()
        .unwrap()
        .slug = "absent".into();
    let absent = client
        .inspect_human_interaction(authed(missing, "human-hidden-token"))
        .await
        .unwrap_err();
    assert_eq!(hidden.code(), tonic::Code::NotFound);
    assert_eq!(absent.code(), hidden.code());
    assert_eq!(absent.message(), hidden.message());
}

#[tokio::test(flavor = "multi_thread")]
async fn absent_deleted_and_foreign_branches_never_fall_back_to_baseline() {
    let (mut client, request, _) = fixture().await;
    for branch in ["never-created", "@other-owner:hidden"] {
        let mut call = authed(request.clone(), "human-reader-token");
        call.metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
        assert_eq!(
            client
                .inspect_human_interaction(call)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::NotFound
        );
    }
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "removed-inspection".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    client
        .delete_branch(authed(
            pb::DeleteBranchRequest {
                name: "removed-inspection".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut call = authed(request, ROOT_TOKEN);
    call.metadata_mut().insert(
        "x-trogon-atlas-branch",
        "removed-inspection".parse().unwrap(),
    );
    assert_eq!(
        client
            .inspect_human_interaction(call)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_and_missing_endpoints_share_identical_unavailable_receipts() {
    let (mut client, request, _) = fixture().await;
    let hidden_namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "private-ui".into(),
                parent: "private-endpoint-owner".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden_ui = id(&hidden_namespace, "private-review");
    put(
        &mut client,
        pb::entity::Kind::Ui(pb::Ui {
            id: Some(hidden_ui.clone()),
            title: "Private UI canary".into(),
            ..Default::default()
        }),
    )
    .await;
    let component = get(&mut client, request.hosting.as_ref().unwrap()).await;
    let mut owner = component.entity.unwrap();
    let Some(pb::entity::Kind::Component(body)) = owner.kind.as_mut() else {
        unreachable!()
    };
    for hosting in &mut body.ui_hostings {
        hosting.ui = Some(pb::UiRef {
            id: Some(hidden_ui.clone()),
        });
        hosting.connection_id = None;
    }
    let component = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(owner),
                if_match: component.etag,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let pb::inspect_human_interaction_request::Path::Issuance(path) =
        request.path.as_ref().unwrap()
    else {
        unreachable!()
    };
    let slice = get(&mut client, path.ui.as_ref().unwrap()).await;
    let mut owner = slice.entity.unwrap();
    let Some(pb::entity::Kind::CommandSlice(body)) = owner.kind.as_mut() else {
        unreachable!()
    };
    body.persona.as_mut().unwrap().connection_id = None;
    body.ui.as_mut().unwrap().connection_id = None;
    body.command.as_mut().unwrap().connection_id = None;
    body.ui.as_mut().unwrap().ui = Some(pb::UiRef {
        id: Some(hidden_ui.clone()),
    });
    let slice = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(owner),
                if_match: slice.etag,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let selected = pb::InspectHumanInteractionRequest {
        hosting: Some(connection(&component, "component.ui_hostings[0]")),
        path: Some(pb::inspect_human_interaction_request::Path::Issuance(
            pb::HumanIssuanceSelection {
                persona: Some(connection(&slice, "command_slice.persona")),
                ui: Some(connection(&slice, "command_slice.ui")),
                command: Some(connection(&slice, "command_slice.command")),
            },
        )),
    };
    let allowed = client
        .inspect_human_interaction(authed(selected.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(allowed.state, pb::CatalogResolutionState::Resolved as i32);
    assert!(allowed.edges[0]
        .checked_target
        .as_ref()
        .is_some_and(|row| !row.uid.is_empty()));
    let hidden = client
        .inspect_human_interaction(authed(selected.clone(), "human-reader-token"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hidden.state, pb::CatalogResolutionState::Unavailable as i32);
    assert_eq!(
        hidden
            .edges
            .iter()
            .filter(|edge| edge.state == pb::CatalogResolutionState::Unavailable as i32)
            .count(),
        2
    );
    assert!(hidden
        .edges
        .iter()
        .filter(|edge| edge.state == pb::CatalogResolutionState::Unavailable as i32)
        .all(|edge| edge.checked_target.is_none()));
    assert!(!hidden
        .scope
        .as_ref()
        .unwrap()
        .authorized_namespaces
        .contains(&hidden_namespace));
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(hidden_ui),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let missing = client
        .inspect_human_interaction(authed(selected, "human-reader-token"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hidden, missing);
}

#[tokio::test(flavor = "multi_thread")]
async fn endpoint_recreation_follows_exact_key_but_owner_recreation_stales_contained_selection() {
    let (mut client, request, _) = fixture().await;
    let before = client
        .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    let ui_ref = before.edges[0].target.clone().unwrap();
    let old_ui = before.edges[0].checked_target.clone().unwrap();
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: ui_ref.kind,
                id: ui_ref.id.clone(),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let absent = client
        .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(absent.state, pb::CatalogResolutionState::Unavailable as i32);
    let recreated = put(
        &mut client,
        pb::entity::Kind::Ui(pb::Ui {
            id: ui_ref.id,
            title: "Replacement".into(),
            ..Default::default()
        }),
    )
    .await;
    let now = client
        .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(now.state, pb::CatalogResolutionState::Resolved as i32);
    let row = now.edges[0].checked_target.as_ref().unwrap();
    assert_ne!(row.uid, old_ui.uid);
    assert_ne!(row.etag, old_ui.etag);
    assert_eq!(row.etag, recreated.etag);
    assert_eq!(now.request.as_ref(), Some(&request));
    let owner_ref = request.hosting.as_ref().unwrap().owner.clone().unwrap();
    let old_owner = get(&mut client, request.hosting.as_ref().unwrap())
        .await
        .entity
        .unwrap();
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: owner_ref.kind,
                id: owner_ref.id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let new_owner = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(old_owner),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_ne!(
        new_owner.system.as_ref().unwrap().uid,
        request.hosting.as_ref().unwrap().owner_uid
    );
    let stale = client
        .inspect_human_interaction(authed(request, ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(stale.state, pb::CatalogResolutionState::Unresolved as i32);
    assert!(stale.edges.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_owner_inventories_share_one_work_budget_without_global_discovery() {
    let (mut client, issuance, display) = fixture().await;
    let pb::inspect_human_interaction_request::Path::Display(path) = display.path.as_ref().unwrap()
    else {
        unreachable!()
    };
    let current = get(&mut client, path.ui.as_ref().unwrap()).await;
    let mut proposed = current.entity.unwrap();
    let Some(pb::entity::Kind::UiSlice(slice)) = proposed.kind.as_mut() else {
        unreachable!()
    };
    let mut additional = slice.source_read_models[0].clone();
    additional.connection_id = None;
    slice
        .source_read_models
        .extend(std::iter::repeat_n(additional, 9_498));
    let written = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(proposed),
                if_match: current.etag,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let large_slice = written.entity.unwrap();
    assert_eq!(
        trogon_atlas_core::connections::connection_occurrences(&large_slice)
            .unwrap()
            .len(),
        9_502
    );
    let first = client
        .inspect_human_interaction(authed(display.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.state, pb::CatalogResolutionState::Resolved as i32);
    let current = get(&mut client, display.hosting.as_ref().unwrap()).await;
    let mut proposed = current.entity.unwrap();
    let Some(pb::entity::Kind::Component(component)) = proposed.kind.as_mut() else {
        unreachable!()
    };
    let mut additional = component.ui_hostings[0].clone();
    additional.connection_id = None;
    component
        .ui_hostings
        .extend(std::iter::repeat_n(additional, 1_022));
    let written = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(proposed),
                if_match: current.etag,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        trogon_atlas_core::connections::connection_occurrences(written.entity.as_ref().unwrap())
            .unwrap()
            .len(),
        1_024
    );
    let separate = client
        .inspect_human_interaction(authed(issuance, ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(separate.state, pb::CatalogResolutionState::Resolved as i32);
    let before = get(&mut client, display.hosting.as_ref().unwrap()).await;
    let failure = client
        .inspect_human_interaction(authed(display.clone(), ROOT_TOKEN))
        .await
        .unwrap_err();
    assert_eq!(failure.code(), tonic::Code::ResourceExhausted);
    assert!(failure.message().contains("work budget exceeded"));
    assert_eq!(
        get(&mut client, display.hosting.as_ref().unwrap()).await,
        before
    );
}
