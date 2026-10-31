#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use support::two_tenants::{
    authed, make_registry, serve_with_registry, ACME_TOKEN, BETA_TOKEN, ROOT_TOKEN,
};
use tonic::{transport::Channel, Code};
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type Client = EventModelServiceClient<Channel>;

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 9_007_199_254_740_993,
    }
}

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

fn hosting(ui: &pb::Id) -> pb::UiHosting {
    pb::UiHosting {
        ui: Some(pb::UiRef {
            id: Some(ui.clone()),
        }),
        label: Some(pb::CatalogTitle {
            value: "Review portal ☕".into(),
        }),
        ..Default::default()
    }
}

fn component(owner: &pb::Id, hostings: Vec<pb::UiHosting>) -> pb::Entity {
    entity(pb::entity::Kind::Component(pb::Component {
        id: Some(owner.clone()),
        title: "Portal".into(),
        ui_hostings: hostings,
        ..Default::default()
    }))
}

fn ui(owner: &pb::Id) -> pb::Entity {
    entity(pb::entity::Kind::Ui(pb::Ui {
        id: Some(owner.clone()),
        title: "Review".into(),
        ..Default::default()
    }))
}

fn hostings(value: &pb::Entity) -> &[pb::UiHosting] {
    let Some(pb::entity::Kind::Component(value)) = &value.kind else {
        panic!("component")
    };
    &value.ui_hostings
}

async fn serve() -> Client {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    serve_with_registry(service, make_registry()).await
}

async fn register(client: &mut Client, token: &str) -> String {
    client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "hosting".into(),
                ..Default::default()
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id
}

async fn put(
    client: &mut Client,
    token: &str,
    value: pb::Entity,
    etag: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(value),
                if_match: etag.into(),
                ..Default::default()
            },
            token,
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn get(
    client: &mut Client,
    kind: pb::EntityKind,
    id: &pb::Id,
) -> Result<pb::GetEntityResponse, tonic::Status> {
    client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(id.clone()),
            },
            ROOT_TOKEN,
        ))
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_requires_a_readable_existing_ui_and_refuses_without_publication() {
    let mut client = serve().await;
    let visible = register(&mut client, ACME_TOKEN).await;
    let hidden = register(&mut client, BETA_TOKEN).await;
    let target = id(&hidden, "private-review");
    put(&mut client, BETA_TOKEN, ui(&target), "").await.unwrap();
    let missing = id(&hidden, "missing-review");
    let mut refusals = Vec::new();
    for (slug, target) in [("hidden", target), ("missing", missing)] {
        let owner = id(&visible, slug);
        let error = put(
            &mut client,
            ACME_TOKEN,
            component(&owner, vec![hosting(&target)]),
            "",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(error
            .message()
            .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
        assert!(!error.message().contains("private-review"));
        assert_eq!(
            get(&mut client, pb::EntityKind::Component, &owner)
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
        refusals.push((error.code(), error.message().to_owned()));
    }
    assert_eq!(refusals[0], refusals[1]);
    let target = id(&visible, "review");
    put(&mut client, ACME_TOKEN, ui(&target), "").await.unwrap();
    let saved = put(
        &mut client,
        ACME_TOKEN,
        component(
            &id(&visible, "portal"),
            vec![hosting(&target), hosting(&target)],
        ),
        "",
    )
    .await
    .unwrap();
    let owner = saved.entity.unwrap();
    assert_ne!(
        hostings(&owner)[0].connection_id,
        hostings(&owner)[1].connection_id
    );
    assert_eq!(
        hostings(&owner)[0].label.as_ref().unwrap().value,
        "Review portal ☕"
    );
    assert_eq!(hostings(&owner)[0].doc, None);
}

fn put_op(value: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            entity: Some(value),
            ..Default::default()
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn untouched_dangling_hosting_survives_but_changed_wording_is_strict() {
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    let owner = id(&namespace, "portal");
    put(&mut client, ACME_TOKEN, ui(&target), "").await.unwrap();
    let saved = put(
        &mut client,
        ACME_TOKEN,
        component(&owner, vec![hosting(&target)]),
        "",
    )
    .await
    .unwrap();
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(target.clone()),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut authored = saved.entity.unwrap();
    let Some(pb::entity::Kind::Component(value)) = authored.kind.as_mut() else {
        panic!("component")
    };
    value.title = "Renamed portal".into();
    let renamed = put(&mut client, ACME_TOKEN, authored, &saved.etag)
        .await
        .unwrap();
    let before = get(&mut client, pb::EntityKind::Component, &owner)
        .await
        .unwrap();
    let mut changed = renamed.entity.unwrap();
    let Some(pb::entity::Kind::Component(value)) = changed.kind.as_mut() else {
        panic!("component")
    };
    value.ui_hostings[0].doc = Some(pb::CatalogText {
        value: "A newly asserted claim".into(),
    });
    let error = put(&mut client, ACME_TOKEN, changed, &renamed.etag)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error
        .message()
        .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner)
            .await
            .unwrap(),
        before
    );
    put(&mut client, ACME_TOKEN, ui(&target), "").await.unwrap();
    let mut resolving = before.entity.clone().unwrap();
    let Some(pb::entity::Kind::Component(value)) = resolving.kind.as_mut() else {
        panic!("component")
    };
    value.ui_hostings[0].doc = Some(pb::CatalogText {
        value: "A newly asserted claim".into(),
    });
    let resolved = put(&mut client, ACME_TOKEN, resolving, &before.etag)
        .await
        .unwrap();
    assert_eq!(
        hostings(resolved.entity.as_ref().unwrap())[0].connection_id,
        hostings(before.entity.as_ref().unwrap())[0].connection_id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_admission_uses_final_batch_overlay_and_publishes_nothing_on_refusal() {
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    let owner = id(&namespace, "portal");
    client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: vec![
                    put_op(component(&owner, vec![hosting(&target)])),
                    put_op(ui(&target)),
                ],
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap();
    let before = get(&mut client, pb::EntityKind::Component, &owner)
        .await
        .unwrap();
    let target_before = get(&mut client, pb::EntityKind::Ui, &target).await.unwrap();
    let sibling = id(&namespace, "unpublished-sibling");
    let mut changed = before.entity.clone().unwrap();
    let Some(pb::entity::Kind::Component(value)) = changed.kind.as_mut() else {
        panic!("component")
    };
    value.ui_hostings.push(hosting(&target));
    let deletion = pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: Some(target.clone()),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        })),
    };
    let error = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: vec![
                    put_op(component(&sibling, Vec::new())),
                    put_op(changed),
                    deletion,
                ],
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error
        .message()
        .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        get(&mut client, pb::EntityKind::Ui, &target).await.unwrap(),
        target_before
    );
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &sibling)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

fn reference(owner: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(owner).unwrap()
}

async fn source_registration(client: &mut Client, namespace: &str) -> pb::SourceRegistration {
    client
        .create_source_registration(authed(
            pb::CreateSourceRegistrationRequest {
                registration: Some(pb::SourceRegistration {
                    id: "hosting-source".into(),
                    administration_namespace: namespace.into(),
                    destination_namespaces: vec![namespace.into()],
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
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn source_cannot_adopt_a_hosting_occurrence_even_without_write_ownership() {
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    let owner_id = id(&namespace, "portal");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let saved = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner_id, vec![hosting(&target)]),
        "",
    )
    .await
    .unwrap();
    let before = get(&mut client, pb::EntityKind::Component, &owner_id)
        .await
        .unwrap();
    let owner = saved.entity.as_ref().unwrap();
    let registration = source_registration(&mut client, &namespace).await;
    let selection = pb::SourceSelection {
        selection: Some(pb::source_selection::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(reference(owner)),
                owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                id: hostings(owner)[0].connection_id.clone(),
            },
        )),
    };
    let error = client
        .create_source_mapping(authed(
            pb::CreateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: "forbidden-hosting".into(),
                    source: Some(pb::SourceResourceKey {
                        registration_id: registration.id.clone(),
                        category: "model".into(),
                        key: "hosting".into(),
                    }),
                    selection: Some(selection),
                    ownership: Some(pb::SourceFieldOwnership {
                        policy: Some(pb::source_field_ownership::Policy::None(
                            pb::SourceNoOwnership {},
                        )),
                    }),
                    adoption: Some(pb::SourceAdoption {
                        observed: Some(pb::SourceObservedOwner {
                            owner: Some(reference(owner)),
                            uid: owner.system.as_ref().unwrap().uid.clone(),
                            etag: saved.etag,
                        }),
                        authorized_adoption: true,
                    }),
                    ..Default::default()
                }),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        client
            .get_source_mapping(authed(
                pb::GetSourceMappingRequest {
                    registration_id: registration.id,
                    mapping_id: "forbidden-hosting".into()
                },
                ROOT_TOKEN
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}
