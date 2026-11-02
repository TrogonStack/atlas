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
        assert_ne!(scope.revision, "");
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
                    "123e4567-e89b-42d3-a456-426614174000".into();
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
    assert_eq!(stale.edges, [] as [pb::HumanInteractionCheckedEdge; 0]);
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

type Client = EventModelServiceClient<Channel>;

fn namespace_of(request: &pb::InspectHumanInteractionRequest) -> String {
    request
        .hosting
        .as_ref()
        .unwrap()
        .owner
        .as_ref()
        .unwrap()
        .id
        .as_ref()
        .unwrap()
        .namespace
        .clone()
}

fn slice_owner(request: &pb::InspectHumanInteractionRequest) -> &pb::ConnectionRef {
    match request.path.as_ref().unwrap() {
        pb::inspect_human_interaction_request::Path::Issuance(path) => path.ui.as_ref().unwrap(),
        pb::inspect_human_interaction_request::Path::Display(path) => path.ui.as_ref().unwrap(),
    }
}

fn issuance_for(
    hosting: &pb::ConnectionRef,
    slice: &pb::Entity,
) -> pb::InspectHumanInteractionRequest {
    pb::InspectHumanInteractionRequest {
        hosting: Some(hosting.clone()),
        path: Some(pb::inspect_human_interaction_request::Path::Issuance(
            pb::HumanIssuanceSelection {
                persona: Some(connection(slice, "command_slice.persona")),
                ui: Some(connection(slice, "command_slice.ui")),
                command: Some(connection(slice, "command_slice.command")),
            },
        )),
    }
}

fn display_for(
    hosting: &pb::ConnectionRef,
    slice: &pb::Entity,
    observation: usize,
) -> pb::InspectHumanInteractionRequest {
    pb::InspectHumanInteractionRequest {
        hosting: Some(hosting.clone()),
        path: Some(pb::inspect_human_interaction_request::Path::Display(
            pb::HumanDisplaySelection {
                persona: Some(connection(slice, "ui_slice.persona")),
                ui: Some(connection(slice, "ui_slice.ui")),
                source_read_model: Some(connection(
                    slice,
                    &format!("ui_slice.source_read_models[{observation}]"),
                )),
            },
        )),
    }
}

async fn inspect(
    client: &mut Client,
    request: &pb::InspectHumanInteractionRequest,
) -> pb::InspectHumanInteractionResponse {
    client
        .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner()
}

async fn assert_unresolved(client: &mut Client, request: &pb::InspectHumanInteractionRequest) {
    let response = inspect(client, request).await;
    assert_eq!(response.request.as_ref(), Some(request));
    assert_eq!(
        response.state,
        pb::CatalogResolutionState::Unresolved as i32
    );
    assert_eq!(response.edges, [] as [pb::HumanInteractionCheckedEdge; 0]);
}

async fn assert_resolved(
    client: &mut Client,
    request: &pb::InspectHumanInteractionRequest,
) -> pb::InspectHumanInteractionResponse {
    let response = inspect(client, request).await;
    assert_eq!(response.state, pb::CatalogResolutionState::Resolved as i32);
    assert_eq!(response.edges.len(), 4);
    response
}

async fn current(client: &mut Client, selected: &pb::ConnectionRef) -> pb::GetEntityResponse {
    get(client, selected).await
}

/// Writes a proposed owner body. Occurrences whose endpoint or predecessor
/// context changed must drop their retained connection IDs (the server refuses
/// a retained ID on a retargeted occurrence), so renewal is applied against the
/// stored owner first and only clears IDs that genuinely changed meaning.
async fn replace(
    client: &mut Client,
    mut entity: pb::Entity,
    etag: String,
) -> pb::PutEntityResponse {
    let stored = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: trogon_atlas_server::conv::entity_to_ref(&entity)
                    .unwrap()
                    .kind,
                id: Some(component_or_owner_id(&entity)),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_or_else(|status| {
            panic!(
                "replace: get_entity failed: code={:?} message={:?}",
                status.code(),
                status.message()
            )
        })
        .into_inner()
        .entity
        .unwrap();
    trogon_atlas_core::connections::renew_retargeted_connection_ids(&stored, &mut entity).unwrap();
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_or_else(|status| {
            panic!(
                "replace: put_entity failed: code={:?} message={:?} if_match={etag:?}",
                status.code(),
                status.message()
            )
        })
        .into_inner()
}

fn component_or_owner_id(entity: &pb::Entity) -> pb::Id {
    trogon_atlas_server::conv::entity_to_ref(entity)
        .unwrap()
        .id
        .unwrap()
}

fn checked_etag(response: &pb::InspectHumanInteractionResponse, owner: &pb::EntityRef) -> String {
    response
        .scope
        .as_ref()
        .unwrap()
        .visited
        .iter()
        .find(|row| row.owner.as_ref() == Some(owner))
        .unwrap()
        .etag
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_context_retargets_renew_affected_edges_and_old_tuples_never_resolve() {
    let (mut client, issuance, display) = fixture().await;
    let namespace = namespace_of(&issuance);
    let hosting = issuance.hosting.clone().unwrap();
    let alternate_ui = id(&namespace, "other-review");
    put(
        &mut client,
        pb::entity::Kind::Ui(pb::Ui {
            id: Some(alternate_ui.clone()),
            title: "Review".into(),
            ..Default::default()
        }),
    )
    .await;
    let decline = id(&namespace, "decline");
    let approver = id(&namespace, "approver");
    let digest = id(&namespace, "digest");
    put(
        &mut client,
        pb::entity::Kind::Command(pb::Command {
            id: Some(decline.clone()),
            title: "Decline".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Persona(pb::Persona {
            id: Some(approver.clone()),
            title: "Approver".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(digest.clone()),
            title: "Digest".into(),
            ..Default::default()
        }),
    )
    .await;

    // Command retarget: the old command ID is retired, persona/UI IDs remain active.
    let row = current(&mut client, slice_owner(&issuance)).await;
    let mut slice = row.entity.unwrap();
    let Some(pb::entity::Kind::CommandSlice(body)) = slice.kind.as_mut() else {
        unreachable!()
    };
    body.command = Some(pb::CommandEdge {
        command: Some(pb::CommandRef {
            id: Some(decline.clone()),
        }),
        ..Default::default()
    });
    let written = replace(&mut client, slice, row.etag).await;
    let slice = written.entity.unwrap();
    assert_unresolved(&mut client, &issuance).await;
    let fresh = issuance_for(&hosting, &slice);
    let response = assert_resolved(&mut client, &fresh).await;
    let command = response
        .edges
        .iter()
        .find(|edge| edge.role == pb::HumanInteractionRole::Command as i32)
        .unwrap();
    assert_eq!(command.target.as_ref().unwrap().id, Some(decline));
    let (
        pb::inspect_human_interaction_request::Path::Issuance(old),
        pb::inspect_human_interaction_request::Path::Issuance(new),
    ) = (
        issuance.path.as_ref().unwrap(),
        fresh.path.as_ref().unwrap(),
    )
    else {
        unreachable!()
    };
    // Identity is context sensitive: a retarget renews the edited occurrence
    // and every occurrence whose predecessor context it changed, so no
    // component of the old tuple can be mixed with the fresh one.
    assert_ne!(old.command, new.command);
    for (stale, renewed) in [(&old.persona, &new.persona), (&old.ui, &new.ui)] {
        assert!(renewed.is_some() && stale.is_some());
    }
    assert_eq!(
        checked_etag(&response, slice_owner(&fresh).owner.as_ref().unwrap()),
        written.etag
    );

    // Persona retarget invalidates the fresh tuple as well.
    let row = current(&mut client, slice_owner(&fresh)).await;
    let mut slice = row.entity.unwrap();
    let Some(pb::entity::Kind::CommandSlice(body)) = slice.kind.as_mut() else {
        unreachable!()
    };
    body.persona = Some(pb::PersonaEdge {
        persona: Some(pb::PersonaRef {
            id: Some(approver.clone()),
        }),
        ..Default::default()
    });
    let slice = replace(&mut client, slice, row.etag).await.entity.unwrap();
    assert_unresolved(&mut client, &fresh).await;
    let fresh = issuance_for(&hosting, &slice);
    assert_resolved(&mut client, &fresh).await;

    // A slice UI that differs from the hosted UI never resolves, even when
    // titles match; restoring the exact hosted UI needs a fresh occurrence.
    let row = current(&mut client, slice_owner(&fresh)).await;
    let mut slice = row.entity.unwrap();
    let Some(pb::entity::Kind::CommandSlice(body)) = slice.kind.as_mut() else {
        unreachable!()
    };
    let hosted_ui = body.ui.as_ref().unwrap().ui.clone();
    body.ui = Some(pb::UiEdge {
        ui: Some(pb::UiRef {
            id: Some(alternate_ui.clone()),
        }),
        ..Default::default()
    });
    let slice = replace(&mut client, slice, row.etag).await.entity.unwrap();
    assert_unresolved(&mut client, &fresh).await;
    let mismatched = issuance_for(&hosting, &slice);
    assert_unresolved(&mut client, &mismatched).await;
    let row = current(&mut client, slice_owner(&mismatched)).await;
    let mut slice = row.entity.unwrap();
    let Some(pb::entity::Kind::CommandSlice(body)) = slice.kind.as_mut() else {
        unreachable!()
    };
    body.ui = Some(pb::UiEdge {
        ui: hosted_ui,
        ..Default::default()
    });
    let slice = replace(&mut client, slice, row.etag).await.entity.unwrap();
    assert_unresolved(&mut client, &mismatched).await;
    assert_resolved(&mut client, &issuance_for(&hosting, &slice)).await;

    // Observation retarget renews the selected edge; the earlier sibling keeps its ID.
    let row = current(&mut client, slice_owner(&display)).await;
    let mut slice = row.entity.unwrap();
    let Some(pb::entity::Kind::UiSlice(body)) = slice.kind.as_mut() else {
        unreachable!()
    };
    let sibling_id = body.source_read_models[0].connection_id.clone();
    body.source_read_models[1] = pb::ReadModelEdge {
        read_model: Some(pb::ReadModelRef {
            id: Some(digest.clone()),
        }),
        ..Default::default()
    };
    let slice = replace(&mut client, slice, row.etag).await.entity.unwrap();
    assert_unresolved(&mut client, &display).await;
    let fresh = display_for(&display.hosting.clone().unwrap(), &slice, 1);
    let response = assert_resolved(&mut client, &fresh).await;
    let observed = response
        .edges
        .iter()
        .find(|edge| edge.role == pb::HumanInteractionRole::ReadModel as i32)
        .unwrap();
    assert_eq!(observed.target.as_ref().unwrap().id, Some(digest));
    let Some(pb::entity::Kind::UiSlice(body)) = slice.kind.as_ref() else {
        unreachable!()
    };
    assert_eq!(body.source_read_models[0].connection_id, sibling_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn unrelated_slice_edits_keep_resolution_with_the_current_checked_etag_and_events_never_substitute(
) {
    let (mut client, issuance, display) = fixture().await;
    let namespace = namespace_of(&issuance);
    let approved = id(&namespace, "approved");
    put(
        &mut client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(approved.clone()),
            title: "Approved".into(),
            ..Default::default()
        }),
    )
    .await;
    let row = current(&mut client, slice_owner(&issuance)).await;
    let mut slice = row.entity.unwrap();
    let Some(pb::entity::Kind::CommandSlice(body)) = slice.kind.as_mut() else {
        unreachable!()
    };
    body.emitted_events = vec![pb::EventEdge {
        event: Some(pb::EventRef { id: Some(approved) }),
        ..Default::default()
    }];
    let added = replace(&mut client, slice, row.etag).await;
    let slice = added.entity.clone().unwrap();
    let owner = slice_owner(&issuance).owner.clone().unwrap();
    let response = assert_resolved(&mut client, &issuance).await;
    assert_eq!(checked_etag(&response, &owner), added.etag);

    // Wording and emitted-event annotations do not renew the selected edges.
    let mut edited = slice.clone();
    let Some(pb::entity::Kind::CommandSlice(body)) = edited.kind.as_mut() else {
        unreachable!()
    };
    body.title = "Approve from the review screen".into();
    body.doc = "Reworded after review".into();
    body.emitted_events[0].doc = "Emitted once approved".into();
    let reworded = replace(&mut client, edited, added.etag.clone()).await;
    assert_ne!(reworded.etag, added.etag);
    let response = assert_resolved(&mut client, &issuance).await;
    assert_eq!(checked_etag(&response, &owner), reworded.etag);

    // An emitted event edge is not a Persona, UI or Command selection.
    let event = connection(&slice, "command_slice.emitted_events[0]");
    for role in 0..3 {
        let mut request = issuance.clone();
        let Some(pb::inspect_human_interaction_request::Path::Issuance(path)) =
            request.path.as_mut()
        else {
            unreachable!()
        };
        match role {
            0 => path.persona = Some(event.clone()),
            1 => path.ui = Some(event.clone()),
            _ => path.command = Some(event.clone()),
        }
        assert_unresolved(&mut client, &request).await;
    }
    let mut request = issuance.clone();
    request.hosting = Some(event);
    assert_unresolved(&mut client, &request).await;

    // Editing the equal sibling observation leaves the selected one resolved.
    let (_, _, selected) = {
        let pb::inspect_human_interaction_request::Path::Display(path) =
            display.path.as_ref().unwrap()
        else {
            unreachable!()
        };
        (
            path.persona.clone(),
            path.ui.clone(),
            path.source_read_model.clone().unwrap(),
        )
    };
    let row = current(&mut client, &selected).await;
    let mut edited = row.entity.unwrap();
    let Some(pb::entity::Kind::UiSlice(body)) = edited.kind.as_mut() else {
        unreachable!()
    };
    body.source_read_models[0].doc = "Sibling annotation".into();
    body.title = "Reworded display".into();
    let written = replace(&mut client, edited, row.etag).await;
    let response = assert_resolved(&mut client, &display).await;
    assert_eq!(
        checked_etag(&response, selected.owner.as_ref().unwrap()),
        written.etag
    );
    // The unedited sibling is a separate selector with separate evidence.
    let sibling = display_for(
        display.hosting.as_ref().unwrap(),
        written.entity.as_ref().unwrap(),
        0,
    );
    let sibling_response = assert_resolved(&mut client, &sibling).await;
    let field = |response: &pb::InspectHumanInteractionResponse| {
        response
            .edges
            .iter()
            .find(|edge| edge.role == pb::HumanInteractionRole::ReadModel as i32)
            .unwrap()
            .field
            .clone()
    };
    assert_eq!(field(&sibling_response), "ui_slice.source_read_models[0]");
    assert_eq!(
        field(&assert_resolved(&mut client, &display).await),
        "ui_slice.source_read_models[1]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn automation_partial_and_membership_only_selections_never_certify_a_human_path() {
    let (mut client, issuance, display) = fixture().await;
    let namespace = namespace_of(&issuance);
    let hosting = issuance.hosting.clone().unwrap();
    put(
        &mut client,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id(&namespace, "relay")),
            title: "Relay".into(),
            ..Default::default()
        }),
    )
    .await;
    let automation = put(
        &mut client,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id(&namespace, "relay-slice")),
            title: "Relay approvals".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id(&namespace, "summary")),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id(&namespace, "relay")),
                }),
                ..Default::default()
            }),
            emitted_command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id(&namespace, "approve")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let unsupported = pb::InspectHumanInteractionRequest {
        hosting: Some(hosting.clone()),
        path: Some(pb::inspect_human_interaction_request::Path::Issuance(
            pb::HumanIssuanceSelection {
                persona: Some(connection(&automation, "automation_slice.processor")),
                ui: Some(connection(
                    &automation,
                    "automation_slice.source_read_models[0]",
                )),
                command: Some(connection(&automation, "automation_slice.emitted_command")),
            },
        )),
    };
    assert_unresolved(&mut client, &unsupported).await;
    let unsupported_display = pb::InspectHumanInteractionRequest {
        hosting: Some(hosting.clone()),
        path: Some(pb::inspect_human_interaction_request::Path::Display(
            pb::HumanDisplaySelection {
                persona: Some(connection(&automation, "automation_slice.processor")),
                ui: Some(connection(
                    &automation,
                    "automation_slice.source_read_models[0]",
                )),
                source_read_model: Some(connection(
                    &automation,
                    "automation_slice.source_read_models[0]",
                )),
            },
        )),
    };
    assert_unresolved(&mut client, &unsupported_display).await;

    // A CommandSlice missing its command never completes an issuance tuple,
    // and companions from another slice cannot complete it.
    let partial = put(
        &mut client,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(&namespace, "partial")),
            title: "Missing command".into(),
            persona: Some(pb::PersonaEdge {
                persona: Some(pb::PersonaRef {
                    id: Some(id(&namespace, "reviewer")),
                }),
                ..Default::default()
            }),
            ui: Some(pb::UiEdge {
                ui: Some(pb::UiRef {
                    id: Some(id(&namespace, "review")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    assert!(
        trogon_atlas_core::connections::connection_occurrences(&partial)
            .unwrap()
            .iter()
            .all(|item| item.path != "command_slice.command")
    );
    let pb::inspect_human_interaction_request::Path::Issuance(complete) =
        issuance.path.as_ref().unwrap()
    else {
        unreachable!()
    };
    for borrowed in [
        pb::HumanIssuanceSelection {
            persona: Some(connection(&partial, "command_slice.persona")),
            ui: Some(connection(&partial, "command_slice.ui")),
            command: complete.command.clone(),
        },
        pb::HumanIssuanceSelection {
            persona: Some(connection(&partial, "command_slice.persona")),
            ui: Some(connection(&partial, "command_slice.ui")),
            command: Some(connection(&partial, "command_slice.ui")),
        },
    ] {
        let request = pb::InspectHumanInteractionRequest {
            hosting: Some(hosting.clone()),
            path: Some(pb::inspect_human_interaction_request::Path::Issuance(
                borrowed,
            )),
        };
        assert_unresolved(&mut client, &request).await;
    }

    // A persona-only (or otherwise incomplete) tuple is malformed.
    for incomplete in 0..4 {
        let mut request = issuance.clone();
        let Some(pb::inspect_human_interaction_request::Path::Issuance(path)) =
            request.path.as_mut()
        else {
            unreachable!()
        };
        match incomplete {
            0 => {
                path.ui = None;
                path.command = None;
            }
            1 => path.ui = None,
            2 => path.command = None,
            _ => {
                request.path = Some(pb::inspect_human_interaction_request::Path::Display(
                    pb::HumanDisplaySelection {
                        persona: complete.persona.clone(),
                        ui: None,
                        source_read_model: None,
                    },
                ));
            }
        }
        assert_eq!(
            client
                .inspect_human_interaction(authed(request, ROOT_TOKEN))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument,
            "incomplete {incomplete}"
        );
    }

    // Raw members and unrelated declarations on a Component are not hostings.
    let members = put(
        &mut client,
        pb::entity::Kind::Component(pb::Component {
            id: Some(id(&namespace, "members-only")),
            title: "Members only".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(id(&namespace, "review")),
            }],
            relationships: vec![pb::ArchitecturalRelationship {
                target: Some(pb::ArchitecturalEndpoint {
                    target: Some(pb::architectural_endpoint::Target::Component(
                        pb::ComponentRef {
                            id: Some(request_component(&issuance)),
                        },
                    )),
                }),
                label: Some(pb::CatalogTitle {
                    value: "Calls portal".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let occurrences = trogon_atlas_core::connections::connection_occurrences(&members).unwrap();
    assert!(occurrences
        .iter()
        .all(|item| !item.path.contains("members")));
    assert!(occurrences
        .iter()
        .all(|item| !item.path.starts_with("component.ui_hostings[")));
    for request in [&issuance, &display] {
        let mut request = request.clone();
        request.hosting = Some(connection(&members, "component.relationships[0]"));
        assert_unresolved(&mut client, &request).await;
    }
}

fn request_component(request: &pb::InspectHumanInteractionRequest) -> pb::Id {
    request
        .hosting
        .as_ref()
        .unwrap()
        .owner
        .as_ref()
        .unwrap()
        .id
        .clone()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_hostings_stop_resolving_and_readded_assertions_receive_fresh_identity() {
    let (mut client, issuance, display) = fixture().await;
    let hosting = issuance.hosting.clone().unwrap();
    assert_resolved(&mut client, &issuance).await;
    let row = current(&mut client, &hosting).await;
    let mut component = row.entity.clone().unwrap();
    let Some(pb::entity::Kind::Component(body)) = component.kind.as_mut() else {
        unreachable!()
    };
    let retired = body.ui_hostings.remove(1);
    let removed = replace(&mut client, component, row.etag).await;
    assert_unresolved(&mut client, &issuance).await;
    // The surviving equal sibling keeps its own identity and evidence.
    assert_resolved(&mut client, &display).await;
    let mut readded = removed.entity.clone().unwrap();
    let Some(pb::entity::Kind::Component(body)) = readded.kind.as_mut() else {
        unreachable!()
    };
    let mut equal = retired.clone();
    equal.connection_id = None;
    body.ui_hostings.push(equal);
    let readded = replace(&mut client, readded, removed.etag).await;
    let entity = readded.entity.unwrap();
    let Some(pb::entity::Kind::Component(body)) = entity.kind.as_ref() else {
        unreachable!()
    };
    assert_ne!(body.ui_hostings[1].connection_id, retired.connection_id);
    assert_unresolved(&mut client, &issuance).await;
    let pb::inspect_human_interaction_request::Path::Issuance(path) =
        issuance.path.as_ref().unwrap()
    else {
        unreachable!()
    };
    let fresh = pb::InspectHumanInteractionRequest {
        hosting: Some(connection(&entity, "component.ui_hostings[1]")),
        path: Some(pb::inspect_human_interaction_request::Path::Issuance(
            path.clone(),
        )),
    };
    assert_resolved(&mut client, &fresh).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_slice_recreation_stales_old_tuples_despite_equal_content_and_reused_ids() {
    let (mut client, issuance, display) = fixture().await;
    let hosting = issuance.hosting.clone().unwrap();
    let owner = slice_owner(&issuance).clone();
    let before = current(&mut client, &owner).await.entity.unwrap();
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: owner.owner.as_ref().unwrap().kind,
                id: owner.owner.as_ref().unwrap().id.clone(),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    assert_unresolved_or_not_found(&mut client, &issuance).await;
    let recreated = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(before.clone()),
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
        recreated.system.as_ref().unwrap().uid,
        before.system.as_ref().unwrap().uid
    );
    assert_unresolved(&mut client, &issuance).await;
    // The sibling slice kind is a different owner and is unaffected.
    assert_resolved(&mut client, &display).await;
    assert_resolved(&mut client, &issuance_for(&hosting, &recreated)).await;
}

async fn assert_unresolved_or_not_found(
    client: &mut Client,
    request: &pb::InspectHumanInteractionRequest,
) {
    match client
        .inspect_human_interaction(authed(request.clone(), ROOT_TOKEN))
        .await
    {
        Ok(response) => {
            let response = response.into_inner();
            assert_ne!(response.state, pb::CatalogResolutionState::Resolved as i32);
        }
        Err(error) => assert_eq!(error.code(), tonic::Code::NotFound),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn inspection_stores_no_references_or_deletion_guards_for_persona_command_or_read_model() {
    let (mut client, issuance, display) = fixture().await;
    let component = current(&mut client, issuance.hosting.as_ref().unwrap())
        .await
        .entity
        .unwrap();
    let refs = |kind: pb::EntityKind, id: pb::Id| pb::GetReferencesRequest {
        kind: kind as i32,
        id: Some(id),
        ..Default::default()
    };
    let namespace = namespace_of(&issuance);
    let endpoints = [
        (pb::EntityKind::Persona, id(&namespace, "reviewer")),
        (pb::EntityKind::Command, id(&namespace, "approve")),
        (pb::EntityKind::ReadModel, id(&namespace, "summary")),
    ];
    let mut before = Vec::new();
    for (kind, endpoint) in &endpoints {
        before.push(
            client
                .get_incoming_references(authed(refs(*kind, endpoint.clone()), ROOT_TOKEN))
                .await
                .unwrap()
                .into_inner()
                .references,
        );
    }
    let outgoing_before = client
        .get_outgoing_references(authed(
            refs(
                pb::EntityKind::Component,
                trogon_atlas_server::conv::entity_to_ref(&component)
                    .unwrap()
                    .id
                    .unwrap(),
            ),
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .references;
    for request in [&issuance, &display] {
        assert_resolved(&mut client, request).await;
    }
    // Hosting stores only Component-to-UI references.
    assert_eq!(outgoing_before.len(), 2);
    assert!(outgoing_before
        .iter()
        .all(
            |reference| reference.to.as_ref().unwrap().kind == pb::EntityKind::Ui as i32
                && reference.from_field.starts_with("ui_hostings[")
        ));
    let outgoing_after = client
        .get_outgoing_references(authed(
            refs(pb::EntityKind::Component, component_id(&component)),
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert_eq!(outgoing_after, outgoing_before);
    for ((kind, endpoint), expected) in endpoints.iter().zip(before) {
        let after = client
            .get_incoming_references(authed(refs(*kind, endpoint.clone()), ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner()
            .references;
        assert_eq!(after, expected);
        assert_ne!(after, [] as [pb::Reference; 0]);
        assert!(after.iter().all(|reference| {
            let from = reference.from.as_ref().unwrap().kind;
            from == pb::EntityKind::CommandSlice as i32 || from == pb::EntityKind::UiSlice as i32
        }));
        let report = client
            .delete_entity(authed(
                pb::DeleteEntityRequest {
                    kind: *kind as i32,
                    id: Some(endpoint.clone()),
                    mode: pb::delete_entity_request::Mode::DryRun as i32,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_ne!(report.referrers, [] as [pb::Reference; 0]);
        assert!(report
            .referrers
            .iter()
            .all(|reference| reference.from.as_ref().unwrap().kind
                != pb::EntityKind::Component as i32));
    }
}

fn component_id(component: &pb::Entity) -> pb::Id {
    trogon_atlas_server::conv::entity_to_ref(component)
        .unwrap()
        .id
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_hosting_and_slice_mutations_are_denied_without_publication() {
    let (mut client, issuance, display) = fixture().await;
    for request in [&issuance, &display] {
        let hosting = request.hosting.as_ref().unwrap();
        let before = get(&mut client, hosting).await;
        let mut edit = before.entity.clone().unwrap();
        let Some(pb::entity::Kind::Component(body)) = edit.kind.as_mut() else {
            unreachable!()
        };
        let mut added = body.ui_hostings[0].clone();
        added.connection_id = None;
        body.ui_hostings.push(added);
        body.ui_hostings[0].label = Some(pb::CatalogTitle {
            value: "Reader reword".into(),
        });
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
        assert_eq!(get(&mut client, hosting).await, before);
        let slice_before = get(&mut client, slice_owner(request)).await;
        let mut slice = slice_before.entity.clone().unwrap();
        match slice.kind.as_mut() {
            Some(pb::entity::Kind::CommandSlice(body)) => body.title = "Reader edit".into(),
            Some(pb::entity::Kind::UiSlice(body)) => body.title = "Reader edit".into(),
            _ => unreachable!(),
        }
        assert_eq!(
            client
                .put_entity(authed(
                    pb::PutEntityRequest {
                        entity: Some(slice),
                        if_match: slice_before.etag.clone(),
                        ..Default::default()
                    },
                    "human-reader-token"
                ))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(get(&mut client, slice_owner(request)).await, slice_before);
        assert_eq!(
            client
                .inspect_human_interaction(authed(request.clone(), "human-reader-token"))
                .await
                .unwrap()
                .into_inner()
                .state,
            pb::CatalogResolutionState::Resolved as i32
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn contained_ui_transitions_are_plain_endpoint_content_and_are_not_charged_as_examined_occurrences(
) {
    let (mut client, _, display) = fixture().await;
    let namespace = namespace_of(&display);
    let selected_ui = id(&namespace, "review");
    let row = current(&mut client, slice_owner(&display)).await;
    let mut proposed = row.entity.unwrap();
    let Some(pb::entity::Kind::UiSlice(slice)) = proposed.kind.as_mut() else {
        unreachable!()
    };
    let mut additional = slice.source_read_models[0].clone();
    additional.connection_id = None;
    slice
        .source_read_models
        .extend(std::iter::repeat_n(additional, 8_498));
    let slice = replace(&mut client, proposed, row.etag)
        .await
        .entity
        .unwrap();
    let examined = trogon_atlas_core::connections::connection_occurrences(&slice)
        .unwrap()
        .len();
    assert_eq!(examined, 8_502);
    let ui = get(
        &mut client,
        &pb::ConnectionRef {
            owner: Some(pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(selected_ui.clone()),
            }),
            ..Default::default()
        },
    )
    .await;
    let mut proposed = ui.entity.unwrap();
    let Some(pb::entity::Kind::Ui(value)) = proposed.kind.as_mut() else {
        unreachable!()
    };
    value.transitions = vec![
        pb::ui::Transition {
            to: Some(pb::UiRef {
                id: Some(selected_ui),
            }),
            doc: "loop".into(),
            ..Default::default()
        };
        1_800
    ];
    let ui = replace(&mut client, proposed, ui.etag)
        .await
        .entity
        .unwrap();
    assert_eq!(
        trogon_atlas_core::connections::connection_occurrences(&ui)
            .unwrap()
            .len(),
        1_800
    );
    // 8,502 selected slice occurrences, two hosting occurrences and 1,800
    // contained UI transitions would exceed the shared 10,000 ceiling if
    // plain endpoint content were charged as examined occurrences.
    let request = pb::InspectHumanInteractionRequest {
        hosting: display.hosting.clone(),
        path: Some(pb::inspect_human_interaction_request::Path::Display(
            pb::HumanDisplaySelection {
                persona: Some(connection(&slice, "ui_slice.persona")),
                ui: Some(connection(&slice, "ui_slice.ui")),
                source_read_model: Some(connection(&slice, "ui_slice.source_read_models[1]")),
            },
        )),
    };
    let response = assert_resolved(&mut client, &request).await;
    assert_eq!(response.scope.unwrap().visited.len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_subject_declarations_guard_the_hosting_owner_and_force_leaves_selectors_unresolved()
{
    let (mut client, issuance, display) = fixture().await;
    let namespace = namespace_of(&issuance);
    let selected = issuance.hosting.clone().unwrap();
    let subject = pb::SubjectAssociation {
        subject: Some(pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Connection(selected.clone())),
        }),
        ..Default::default()
    };
    put(
        &mut client,
        pb::entity::Kind::Document(pb::Document {
            id: Some(id(&namespace, "guide")),
            title: Some(pb::CatalogTitle {
                value: "Guide".into(),
            }),
            subjects: vec![subject.clone()],
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/guide".into(),
            })),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Decision(pb::Decision {
            id: Some(id(&namespace, "decision")),
            title: Some(pb::CatalogTitle {
                value: "Portal hosting".into(),
            }),
            status: pb::DecisionStatus::Proposed as i32,
            context: Some(pb::CatalogText {
                value: "Review the selected hosting".into(),
            }),
            rationale: Some(pb::CatalogText {
                value: "Keep the declaration exact".into(),
            }),
            options: vec![pb::DecisionOption {
                title: Some(pb::CatalogTitle {
                    value: "Retain".into(),
                }),
                description: Some(pb::CatalogText {
                    value: "Retain it".into(),
                }),
                ..Default::default()
            }],
            subjects: vec![subject],
            ..Default::default()
        }),
    )
    .await;
    let owner = selected.owner.clone().unwrap();
    let delete = |mode: pb::delete_entity_request::Mode| pb::DeleteEntityRequest {
        kind: owner.kind,
        id: owner.id.clone(),
        mode: mode as i32,
        ..Default::default()
    };
    let report = client
        .delete_entity(authed(
            delete(pb::delete_entity_request::Mode::DryRun),
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut kinds: Vec<_> = report
        .referrers
        .iter()
        .map(|reference| reference.from.as_ref().unwrap().kind)
        .collect();
    kinds.sort_unstable();
    assert!(kinds.contains(&(pb::EntityKind::Document as i32)));
    assert!(kinds.contains(&(pb::EntityKind::Decision as i32)));
    // A guarded deletion refuses and publishes nothing; inspection still resolves.
    let before = get(&mut client, &selected).await;
    assert_eq!(
        client
            .delete_entity(authed(
                delete(pb::delete_entity_request::Mode::Unspecified),
                ROOT_TOKEN
            ))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    assert_eq!(get(&mut client, &selected).await, before);
    assert_resolved(&mut client, &issuance).await;
    // Force deletion is explicit and leaves every selector unresolved.
    client
        .delete_entity(authed(
            delete(pb::delete_entity_request::Mode::Force),
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    for request in [&issuance, &display] {
        assert_unresolved_or_not_found(&mut client, request).await;
    }
}
