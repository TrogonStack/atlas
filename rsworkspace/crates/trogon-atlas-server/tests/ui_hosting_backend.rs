#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use async_trait::async_trait;
use support::two_tenants::{
    authed, make_registry, serve_with_registry, ACME_TOKEN, BETA_TOKEN, ROOT_TOKEN,
};
use tonic::{transport::Channel, Code};
use trogon_atlas_core::{NamespaceId, OwnerId};
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{
    metrics::ServerMetrics,
    ownership::{Lens, NamespaceAuthorizer, RegistryAuthorizer, Visibility, WriteVerdict},
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NamespaceRecord, Store};

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

async fn serve_with_store() -> (Client, Arc<trogon_atlas_store::NatsStore>) {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    (serve_with_registry(service, make_registry()).await, store)
}
async fn serve() -> Client {
    serve_with_store().await.0
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

fn component_mut(value: &mut pb::Entity) -> &mut pb::Component {
    let Some(pb::entity::Kind::Component(value)) = value.kind.as_mut() else {
        panic!("component")
    };
    value
}

fn resize_owner(value: &mut pb::Entity, bytes: usize) {
    use prost::Message;
    loop {
        let size = value.encoded_len();
        if size == bytes {
            return;
        }
        let doc = &mut component_mut(value).doc;
        if size < bytes {
            doc.push_str(&"x".repeat(bytes - size));
        } else {
            doc.truncate(doc.len() - (size - bytes));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_collection_and_complete_prepared_bytes_reject_atomically_at_the_next_value() {
    use prost::Message;
    let (mut client, store) = serve_with_store().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let owner = id(&namespace, "collection");
    let exact = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner, vec![hosting(&target); 1_024]),
        "",
    )
    .await
    .unwrap();
    assert_eq!(hostings(exact.entity.as_ref().unwrap()).len(), 1_024);
    let ids: std::collections::HashSet<_> = hostings(exact.entity.as_ref().unwrap())
        .iter()
        .map(|value| value.connection_id.clone().unwrap())
        .collect();
    assert_eq!(ids.len(), 1_024);
    let before = get(&mut client, pb::EntityKind::Component, &owner)
        .await
        .unwrap();
    let sequence = store.current_change_seq().await.unwrap();
    let mut next = exact.entity.unwrap();
    component_mut(&mut next).ui_hostings.push(hosting(&target));
    let error = put(&mut client, ROOT_TOKEN, next, &before.etag)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains("UI hosting owner exceeds limits"));
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner)
            .await
            .unwrap(),
        before
    );
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);

    let owner = id(&namespace, "bytes");
    let mut bounded = hosting(&target);
    bounded.doc = Some(pb::CatalogText {
        value: "x".repeat(15_500),
    });
    let initial = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner, vec![bounded; 64]),
        "",
    )
    .await
    .unwrap();
    let mut exact = initial.entity.unwrap();
    resize_owner(&mut exact, 1_048_576);
    assert!(component_mut(&mut exact).doc.len() <= 65_536);
    assert!(hostings(&exact)
        .iter()
        .all(|hosting| hosting.doc.as_ref().unwrap().value.len() <= 16_384));
    let exact = put(&mut client, ROOT_TOKEN, exact, &initial.etag)
        .await
        .unwrap();
    assert_eq!(exact.entity.as_ref().unwrap().encoded_len(), 1_048_576);
    let before = get(&mut client, pb::EntityKind::Component, &owner)
        .await
        .unwrap();
    let sequence = store.current_change_seq().await.unwrap();
    let stored = store
        .get(pb::EntityKind::Component, &owner, None)
        .await
        .unwrap();
    assert_eq!(stored.entity, before.entity.clone().unwrap());
    assert_eq!(stored.etag, before.etag);
    for strip_system in [false, true] {
        let mut next = exact.entity.clone().unwrap();
        resize_owner(&mut next, 1_048_577);
        if strip_system {
            next.system = None;
            assert!(next.encoded_len() < 1_048_576);
            assert_eq!(
                trogon_atlas_core::stamp_system(before.entity.as_ref(), &next).encoded_len(),
                1_048_577
            );
        }
        let error = put(&mut client, ROOT_TOKEN, next, &before.etag)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert_eq!(
            get(&mut client, pb::EntityKind::Component, &owner)
                .await
                .unwrap(),
            before
        );
        assert_eq!(store.current_change_seq().await.unwrap(), sequence);
        assert_eq!(
            store
                .get(pb::EntityKind::Component, &owner, None)
                .await
                .unwrap()
                .entity,
            stored.entity
        );
    }
    let mut next = exact.entity.unwrap();
    component_mut(&mut next).ui_hostings.push(hosting(&target));
    let mut prepared = next.clone();
    trogon_atlas_core::connections::reconcile_connection_ids(
        before.entity.as_ref().unwrap(),
        &mut prepared,
    )
    .unwrap();
    let assigned_growth = prepared.encoded_len() - next.encoded_len();
    resize_owner(&mut next, 1_048_577 - assigned_growth);
    next.system = None;
    assert!(
        trogon_atlas_core::stamp_system(before.entity.as_ref(), &next).encoded_len() <= 1_048_576
    );
    let mut prepared = next.clone();
    trogon_atlas_core::connections::reconcile_connection_ids(
        before.entity.as_ref().unwrap(),
        &mut prepared,
    )
    .unwrap();
    assert_eq!(
        trogon_atlas_core::stamp_system(before.entity.as_ref(), &prepared).encoded_len(),
        1_048_577
    );
    let error = put(&mut client, ROOT_TOKEN, next, &before.etag)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains("UI hosting owner exceeds limits"));
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner)
            .await
            .unwrap(),
        before
    );
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
    assert_eq!(
        store
            .get(pb::EntityKind::Component, &owner, None)
            .await
            .unwrap()
            .entity,
        stored.entity
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_batches_share_one_exact_inventory_work_budget() {
    let (mut client, store) = serve_with_store().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let owners: Vec<_> = (0..10)
        .map(|index| id(&namespace, &format!("portal-{index}")))
        .collect();
    let values: Vec<_> = owners
        .iter()
        .enumerate()
        .map(|(index, owner)| {
            component(
                owner,
                vec![hosting(&target); if index == 9 { 784 } else { 1_024 }],
            )
        })
        .collect();
    client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: values.into_iter().map(put_op).collect(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut before = Vec::new();
    for owner in &owners {
        before.push(
            get(&mut client, pb::EntityKind::Component, owner)
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        before
            .iter()
            .map(|row| hostings(row.entity.as_ref().unwrap()).len())
            .sum::<usize>(),
        10_000
    );
    let mut values: Vec<_> = before
        .iter()
        .map(|row| row.entity.clone().unwrap())
        .collect();
    component_mut(&mut values[9])
        .ui_hostings
        .push(hosting(&target));
    let sequence = store.current_change_seq().await.unwrap();
    let error = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: values.into_iter().map(put_op).collect(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
    assert!(error.message().contains("work budget exceeded"));
    for (owner, before) in owners.iter().zip(before) {
        assert_eq!(
            get(&mut client, pb::EntityKind::Component, owner)
                .await
                .unwrap(),
            before
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_reference_occurrences_and_subjects_keep_exact_siblings_and_hidden_delete_guard() {
    let mut client = serve().await;
    let owner_ns = register(&mut client, ACME_TOKEN).await;
    let target_ns = register(&mut client, BETA_TOKEN).await;
    let target = id(&target_ns, "review");
    let owner_id = id(&owner_ns, "portal");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let saved = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner_id, vec![hosting(&target), hosting(&target)]),
        "",
    )
    .await
    .unwrap();
    let owner = saved.entity.as_ref().unwrap();
    let selected = |index: usize| pb::ConnectionRef {
        owner: Some(reference(owner)),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id: hostings(owner)[index].connection_id.clone(),
    };
    let first = selected(0);
    let second = selected(1);
    let incoming = client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(target.clone()),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.references.len(), 2);
    for (index, hit) in incoming.references.iter().enumerate() {
        assert_eq!(hit.from, Some(reference(owner)));
        assert_eq!(hit.from_uid, owner.system.as_ref().unwrap().uid);
        assert_eq!(hit.connection_id, hostings(owner)[index].connection_id);
        assert_eq!(hit.from_field, format!("ui_hostings[{index}].ui"));
        assert_eq!(
            hit.to,
            Some(pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(target.clone())
            })
        );
    }
    put(
        &mut client,
        ROOT_TOKEN,
        entity(pb::entity::Kind::Document(pb::Document {
            id: Some(id(&owner_ns, "guide")),
            title: Some(pb::CatalogTitle {
                value: "Guide".into(),
            }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(first.clone())),
                }),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/guide".into(),
            })),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap();
    put(
        &mut client,
        ROOT_TOKEN,
        entity(pb::entity::Kind::Decision(pb::Decision {
            id: Some(id(&owner_ns, "decision")),
            title: Some(pb::CatalogTitle {
                value: "Portal hosting".into(),
            }),
            status: pb::DecisionStatus::Proposed as i32,
            context: Some(pb::CatalogText {
                value: "Review the selected hosting assertion".into(),
            }),
            rationale: Some(pb::CatalogText {
                value: "Keep this declaration distinct from its sibling".into(),
            }),
            options: vec![pb::DecisionOption {
                title: Some(pb::CatalogTitle {
                    value: "Retain".into(),
                }),
                description: Some(pb::CatalogText {
                    value: "Retain the selected hosting".into(),
                }),
                ..Default::default()
            }],
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(first.clone())),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap();
    put(
        &mut client,
        ROOT_TOKEN,
        entity(pb::entity::Kind::Person(pb::Person {
            id: Some(id(&owner_ns, "maintainer")),
            title: Some(pb::DirectoryDisplayName {
                value: "Maintainer".into(),
            }),
            responsibilities: vec![pb::Responsibility {
                kind: pb::ResponsibilityKind::Maintainer as i32,
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Connection(
                        first.clone(),
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap();
    for (selected, expected) in [(&first, 1), (&second, 0)] {
        let subject = pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Connection(selected.clone())),
        };
        let documents = client
            .list_subject_documents(authed(
                pb::ListSubjectDocumentsRequest {
                    subject: Some(subject.clone()),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        let decisions = client
            .list_subject_decisions(authed(
                pb::ListSubjectDecisionsRequest {
                    subject: Some(subject),
                    status: None,
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        let assignments = client
            .list_subject_assignments(authed(
                pb::ListSubjectAssignmentsRequest {
                    subject: Some(pb::ResponsibilitySubject {
                        selection: Some(pb::responsibility_subject::Selection::Connection(
                            selected.clone(),
                        )),
                    }),
                    kind: None,
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            (
                documents.declarations.len(),
                decisions.declarations.len(),
                assignments.assignments.len()
            ),
            (expected, expected, expected)
        );
        assert!(!documents.incomplete && !decisions.incomplete && !assignments.incomplete);
    }
    let visible_report = client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(target.clone()),
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(visible_report.referrers.is_empty());
    let before = get(&mut client, pb::EntityKind::Ui, &target).await.unwrap();
    let error = client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(target.clone()),
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert!(!error.message().contains(&owner_ns));
    assert!(!error.message().contains("portal"));
    assert_eq!(
        get(&mut client, pb::EntityKind::Ui, &target).await.unwrap(),
        before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_identity_preview_reorder_retarget_retirement_and_new_incarnations_are_explicit() {
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    let other = id(&namespace, "other-review");
    for selected in [&target, &other] {
        put(&mut client, ROOT_TOKEN, ui(selected), "")
            .await
            .unwrap();
    }
    let owner_id = id(&namespace, "portal");
    let input = component(&owner_id, vec![hosting(&target), hosting(&target)]);
    let preview = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(input.clone()),
                validate_only: true,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let created = put(&mut client, ROOT_TOKEN, input, "").await.unwrap();
    let original = created.entity.as_ref().unwrap();
    assert_ne!(
        original.system.as_ref().unwrap().uid,
        preview
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
    );
    assert_ne!(
        hostings(original)[0].connection_id,
        hostings(preview.entity.as_ref().unwrap())[0].connection_id
    );
    let mut reordered = original.clone();
    let component = component_mut(&mut reordered);
    component.ui_hostings.reverse();
    component.ui_hostings[0].label = Some(pb::CatalogTitle {
        value: "New label".into(),
    });
    component.ui_hostings[0].doc = Some(pb::CatalogText {
        value: "Descriptive edit".into(),
    });
    let edited = put(&mut client, ROOT_TOKEN, reordered, &created.etag)
        .await
        .unwrap();
    assert_eq!(
        hostings(edited.entity.as_ref().unwrap())[0].connection_id,
        hostings(original)[1].connection_id
    );
    assert_eq!(
        hostings(edited.entity.as_ref().unwrap())[1].connection_id,
        hostings(original)[0].connection_id
    );
    let before = get(&mut client, pb::EntityKind::Component, &owner_id)
        .await
        .unwrap();
    let stale = put(
        &mut client,
        ROOT_TOKEN,
        edited.entity.clone().unwrap(),
        &created.etag,
    )
    .await
    .unwrap_err();
    assert_eq!(stale.code(), Code::Aborted);
    let mut retargeted = edited.entity.unwrap();
    component_mut(&mut retargeted).ui_hostings[0].ui = Some(pb::UiRef { id: Some(other) });
    let error = put(&mut client, ROOT_TOKEN, retargeted.clone(), &before.etag)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains("changed role or endpoint"));
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap(),
        before
    );
    component_mut(&mut retargeted).ui_hostings[0].connection_id = None;
    let retargeted = put(&mut client, ROOT_TOKEN, retargeted, &before.etag)
        .await
        .unwrap();
    assert_ne!(
        hostings(retargeted.entity.as_ref().unwrap())[0].connection_id,
        hostings(before.entity.as_ref().unwrap())[0].connection_id
    );
    assert_eq!(
        hostings(retargeted.entity.as_ref().unwrap())[1].connection_id,
        hostings(before.entity.as_ref().unwrap())[1].connection_id
    );
    let mut removed = retargeted.entity.clone().unwrap();
    let retired = component_mut(&mut removed).ui_hostings.remove(0);
    let removed = put(&mut client, ROOT_TOKEN, removed, &retargeted.etag)
        .await
        .unwrap();
    let mut restored = removed.entity.clone().unwrap();
    component_mut(&mut restored)
        .ui_hostings
        .push(retired.clone());
    assert_eq!(
        put(&mut client, ROOT_TOKEN, restored.clone(), &removed.etag)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    component_mut(&mut restored).ui_hostings[1].connection_id = None;
    let restored = put(&mut client, ROOT_TOKEN, restored, &removed.etag)
        .await
        .unwrap();
    assert_ne!(
        hostings(restored.entity.as_ref().unwrap())[1].connection_id,
        retired.connection_id
    );
    let mut idless = restored.entity.clone().unwrap();
    for hosting in &mut component_mut(&mut idless).ui_hostings {
        hosting.connection_id = None;
    }
    let noop = put(&mut client, ROOT_TOKEN, idless, &restored.etag)
        .await
        .unwrap();
    assert!(noop.no_op);
    assert_eq!(noop.entity, restored.entity);
    assert_eq!(noop.etag, restored.etag);
    let retained = restored.entity.unwrap();
    for (slug, version) in [
        ("clone", owner_id.version),
        ("portal", owner_id.version + 1),
    ] {
        let mut cloned = retained.clone();
        component_mut(&mut cloned).id = Some(pb::Id {
            namespace: namespace.clone(),
            slug: slug.into(),
            version,
        });
        let cloned = put(&mut client, ROOT_TOKEN, cloned, "")
            .await
            .unwrap()
            .entity
            .unwrap();
        assert_ne!(
            cloned.system.as_ref().unwrap().uid,
            retained.system.as_ref().unwrap().uid
        );
    }
    let old_selector = pb::ConnectionRef {
        owner: Some(reference(&retained)),
        owner_uid: retained.system.as_ref().unwrap().uid.clone(),
        id: hostings(&retained)[0].connection_id.clone(),
    };
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(owner_id),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let recreated = put(&mut client, ROOT_TOKEN, retained.clone(), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    assert_ne!(
        recreated.system.as_ref().unwrap().uid,
        old_selector.owner_uid
    );
    let state = client
        .list_subject_documents(authed(
            pb::ListSubjectDocumentsRequest {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(old_selector)),
                }),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        state.subject_state,
        pb::CatalogResolutionState::Unresolved as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_branch_restore_renews_only_the_opposing_retired_hosting() {
    use support::two_tenants::authed_branch;
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    let owner_id = id(&namespace, "portal");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let saved = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner_id, vec![hosting(&target), hosting(&target)]),
        "",
    )
    .await
    .unwrap();
    let before = saved.entity.clone().unwrap();
    let ids: Vec<_> = hostings(&before)
        .iter()
        .map(|value| value.connection_id.clone())
        .collect();
    let branch = format!("hosting-{}", uuid::Uuid::now_v7().simple());
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: branch.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut branch_owner = before.clone();
    component_mut(&mut branch_owner).title = "Authored on branch".into();
    client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(branch_owner),
                if_match: saved.etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap();
    let mut baseline = before.clone();
    component_mut(&mut baseline).ui_hostings.pop();
    let baseline = put(&mut client, ROOT_TOKEN, baseline, &saved.etag)
        .await
        .unwrap();
    let conflict = client
        .merge_branch(authed(
            pb::MergeBranchRequest {
                name: branch.clone(),
                dry_run: true,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        conflict.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    client
        .resolve_branch_entry(authed(
            pb::ResolveBranchEntryRequest {
                name: branch.clone(),
                r#ref: Some(reference(&before)),
                resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let resolved = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(owner_id.clone()),
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        hostings(resolved.entity.as_ref().unwrap())[0].connection_id,
        ids[0]
    );
    assert_ne!(
        hostings(resolved.entity.as_ref().unwrap())[1].connection_id,
        ids[1]
    );
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap()
            .entity,
        baseline.entity
    );
    let merged = client
        .merge_branch(authed(
            pb::MergeBranchRequest {
                name: branch,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap()
            .entity,
        resolved.entity
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_row_budget_includes_plain_ui_dependencies_and_declaring_owners_together() {
    let (mut client, store) = serve_with_store().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let targets: Vec<_> = (0..4_996)
        .map(|index| id(&namespace, &format!("review-{index}")))
        .collect();
    for chunk in targets.chunks(500) {
        client
            .batch_mutate(authed(
                pb::BatchMutateRequest {
                    ops: chunk.iter().map(|target| put_op(ui(target))).collect(),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap();
    }
    let owners: Vec<_> = (0..5)
        .map(|index| id(&namespace, &format!("portal-{index}")))
        .collect();
    let values: Vec<_> = owners
        .iter()
        .enumerate()
        .map(|(index, owner)| {
            component(
                owner,
                targets[index * 999..(index + 1) * 999]
                    .iter()
                    .map(hosting)
                    .collect(),
            )
        })
        .collect();
    client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: values.into_iter().map(put_op).collect(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut before = Vec::new();
    for owner in &owners {
        before.push(
            get(&mut client, pb::EntityKind::Component, owner)
                .await
                .unwrap(),
        );
    }
    let mut values: Vec<_> = before
        .iter()
        .map(|row| row.entity.clone().unwrap())
        .collect();
    for value in &mut values {
        for hosting in &mut component_mut(value).ui_hostings {
            hosting.doc = Some(pb::CatalogText {
                value: "Recheck this authored assertion".into(),
            });
        }
    }
    component_mut(&mut values[4])
        .ui_hostings
        .push(hosting(&targets[4_995]));
    let sequence = store.current_change_seq().await.unwrap();
    let error = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: values.into_iter().map(put_op).collect(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
    assert!(error.message().contains("hosting resource budget exceeded"));
    for (owner, before) in owners.iter().zip(before) {
        assert_eq!(
            get(&mut client, pb::EntityKind::Component, owner)
                .await
                .unwrap(),
            before
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_admission_uses_branch_targets_and_tombstones_without_baseline_fallthrough() {
    use support::two_tenants::authed_branch;
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "baseline-review");
    let branch_target = id(&namespace, "branch-review");
    let owner_id = id(&namespace, "portal");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let saved = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner_id, vec![hosting(&target), hosting(&target)]),
        "",
    )
    .await
    .unwrap();
    let branch = format!("hosting-target-{}", uuid::Uuid::now_v7().simple());
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: branch.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(ui(&branch_target)),
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap();
    assert_eq!(
        get(&mut client, pb::EntityKind::Ui, &branch_target)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let mut branch_owner = saved.entity.clone().unwrap();
    component_mut(&mut branch_owner).ui_hostings[0].ui = Some(pb::UiRef {
        id: Some(branch_target.clone()),
    });
    component_mut(&mut branch_owner).ui_hostings[0].connection_id = None;
    let branch_saved = client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(branch_owner),
                if_match: saved.etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        hostings(branch_saved.entity.as_ref().unwrap())[1].connection_id,
        hostings(saved.entity.as_ref().unwrap())[1].connection_id
    );
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap()
            .entity,
        saved.entity
    );
    client
        .delete_entity(authed_branch(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(target.clone()),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap();
    let mut changed = branch_saved.entity.clone().unwrap();
    component_mut(&mut changed)
        .ui_hostings
        .push(hosting(&target));
    let error = client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(changed),
                if_match: branch_saved.etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error
        .message()
        .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
    let after = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(owner_id.clone()),
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after.entity, branch_saved.entity);
    assert_eq!(after.etag, branch_saved.etag);
    assert_eq!(
        get(&mut client, pb::EntityKind::Ui, &target)
            .await
            .unwrap()
            .entity
            .unwrap()
            .kind,
        ui(&target).kind
    );
    let mut title_only = branch_saved.entity.unwrap();
    component_mut(&mut title_only).title = "Still authored on branch".into();
    client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(title_only),
                if_match: branch_saved.etag,
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap();
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap()
            .entity,
        saved.entity
    );
}

#[derive(Debug)]
struct HostingAuthorizer {
    inner: RegistryAuthorizer,
    readable: std::sync::Mutex<Option<NamespaceId>>,
    failure: std::sync::Mutex<Option<Code>>,
}
#[async_trait]
impl NamespaceAuthorizer for HostingAuthorizer {
    async fn admits(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<bool, tonic::Status> {
        if self.readable.lock().unwrap().as_ref() == Some(namespace) {
            if let Some(code) = *self.failure.lock().unwrap() {
                return Err(tonic::Status::new(code, "injected authorization outage"));
            }
            return Ok(true);
        }
        self.inner.admits(visibility, namespace).await
    }
    async fn lens(&self, visibility: &Visibility) -> Result<Lens, tonic::Status> {
        let mut lens = self.inner.lens(visibility).await?;
        if let (Lens::Only(namespaces), Some(readable)) =
            (&mut lens, self.readable.lock().unwrap().as_ref())
        {
            Arc::make_mut(namespaces).insert(readable.clone());
        }
        Ok(lens)
    }
    async fn may_write(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<WriteVerdict, tonic::Status> {
        self.inner.may_write(visibility, namespace).await
    }
    async fn on_registered(&self, record: &NamespaceRecord) -> Result<(), tonic::Status> {
        self.inner.on_registered(record).await
    }
    async fn on_moved(
        &self,
        record: &NamespaceRecord,
        from: &OwnerId,
    ) -> Result<(), tonic::Status> {
        self.inner.on_moved(record, from).await
    }
    async fn on_released(&self, record: &NamespaceRecord) -> Result<(), tonic::Status> {
        self.inner.on_released(record).await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hosting_uses_reference_read_authority_without_granting_endpoint_writes_and_propagates_auth_failures(
) {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let authorizer = Arc::new(HostingAuthorizer {
        inner: RegistryAuthorizer::new(service.directory(), store),
        readable: std::sync::Mutex::new(None),
        failure: std::sync::Mutex::new(None),
    });
    let mut client =
        serve_with_registry(service.with_authorizer(authorizer.clone()), make_registry()).await;
    let home = register(&mut client, ACME_TOKEN).await;
    let remote = register(&mut client, BETA_TOKEN).await;
    let target = id(&remote, "review");
    let owner = id(&home, "portal");
    put(&mut client, BETA_TOKEN, ui(&target), "").await.unwrap();
    *authorizer.readable.lock().unwrap() = Some(NamespaceId::parse(&remote).unwrap());
    let target_before = get(&mut client, pb::EntityKind::Ui, &target).await.unwrap();
    let created = put(
        &mut client,
        ACME_TOKEN,
        component(&owner, vec![hosting(&target)]),
        "",
    )
    .await
    .unwrap();
    let error = put(&mut client, ACME_TOKEN, ui(&target), "")
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(
        get(&mut client, pb::EntityKind::Ui, &target).await.unwrap(),
        target_before
    );
    let before = get(&mut client, pb::EntityKind::Component, &owner)
        .await
        .unwrap();
    for kind in [
        pb::entity::Kind::Persona(pb::Persona {
            id: Some(id(&home, "reviewer")),
            title: "Reviewer".into(),
            ..Default::default()
        }),
        pb::entity::Kind::Command(pb::Command {
            id: Some(id(&home, "approve")),
            title: "Approve".into(),
            ..Default::default()
        }),
    ] {
        put(&mut client, ROOT_TOKEN, entity(kind), "")
            .await
            .unwrap();
    }
    let slice = put(
        &mut client,
        ROOT_TOKEN,
        entity(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(&home, "issue")),
            title: "Approve".into(),
            persona: Some(pb::PersonaEdge {
                persona: Some(pb::PersonaRef {
                    id: Some(id(&home, "reviewer")),
                }),
                ..Default::default()
            }),
            ui: Some(pb::UiEdge {
                ui: Some(pb::UiRef {
                    id: Some(target.clone()),
                }),
                ..Default::default()
            }),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id(&home, "approve")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    let selected = |owner: &pb::Entity, path: &str| {
        let occurrence = trogon_atlas_core::connections::connection_occurrences(owner)
            .unwrap()
            .into_iter()
            .find(|value| value.path == path)
            .unwrap();
        pb::ConnectionRef {
            owner: Some(reference(owner)),
            owner_uid: owner.system.as_ref().unwrap().uid.clone(),
            id: Some(pb::ConnectionId {
                value: occurrence.id.unwrap().to_string(),
            }),
        }
    };
    let inspection = pb::InspectHumanInteractionRequest {
        hosting: Some(selected(
            created.entity.as_ref().unwrap(),
            "component.ui_hostings[0]",
        )),
        path: Some(pb::inspect_human_interaction_request::Path::Issuance(
            pb::HumanIssuanceSelection {
                persona: Some(selected(&slice, "command_slice.persona")),
                ui: Some(selected(&slice, "command_slice.ui")),
                command: Some(selected(&slice, "command_slice.command")),
            },
        )),
    };
    let checked = client
        .inspect_human_interaction(authed(inspection.clone(), ACME_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(checked.state, pb::CatalogResolutionState::Resolved as i32);
    for code in [Code::Unavailable, Code::DeadlineExceeded] {
        *authorizer.failure.lock().unwrap() = Some(code);
        let mut changed = created.entity.clone().unwrap();
        component_mut(&mut changed).ui_hostings[0].doc = Some(pb::CatalogText {
            value: "Changed assertion requires a fresh authorized target read".into(),
        });
        let error = put(&mut client, ACME_TOKEN, changed, &created.etag)
            .await
            .unwrap_err();
        assert_eq!(error.code(), code);
        assert!(!error
            .message()
            .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
        let query_error = client
            .inspect_human_interaction(authed(inspection.clone(), ACME_TOKEN))
            .await
            .unwrap_err();
        assert_eq!(query_error.code(), code);

        *authorizer.failure.lock().unwrap() = None;
        assert_eq!(
            get(&mut client, pb::EntityKind::Component, &owner)
                .await
                .unwrap(),
            before
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_and_duplicate_hosting_ids_are_refused_before_publication_including_cross_role_collisions(
) {
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let peer_id = id(&namespace, "peer");
    put(&mut client, ROOT_TOKEN, component(&peer_id, Vec::new()), "")
        .await
        .unwrap();
    for (index, malformed) in ["", "broken-id", "00000000-0000-0000-0000-000000000000"]
        .into_iter()
        .enumerate()
    {
        let owner_id = id(&namespace, &format!("invalid-{index}"));
        let mut input = component(&owner_id, vec![hosting(&target)]);
        component_mut(&mut input).ui_hostings[0].connection_id = Some(pb::ConnectionId {
            value: malformed.into(),
        });
        for preview in [false, true] {
            let error = client
                .put_entity(authed(
                    pb::PutEntityRequest {
                        entity: Some(input.clone()),
                        validate_only: preview,
                        ..Default::default()
                    },
                    ROOT_TOKEN,
                ))
                .await
                .unwrap_err();
            assert_eq!(error.code(), Code::InvalidArgument);
            assert_eq!(
                get(&mut client, pb::EntityKind::Component, &owner_id)
                    .await
                    .unwrap_err()
                    .code(),
                Code::NotFound
            );
        }
    }
    for cross_role in [false, true] {
        let owner_id = id(
            &namespace,
            if cross_role {
                "duplicate-role"
            } else {
                "duplicate-hosting"
            },
        );
        let mut input = component(&owner_id, vec![hosting(&target)]);
        let duplicate = Some(pb::ConnectionId {
            value: "11111111-1111-4111-8111-111111111111".into(),
        });
        let value = component_mut(&mut input);
        value.ui_hostings[0].connection_id = duplicate.clone();
        if cross_role {
            value.relationships.push(pb::ArchitecturalRelationship {
                connection_id: duplicate,
                target: Some(pb::ArchitecturalEndpoint {
                    target: Some(pb::architectural_endpoint::Target::Component(
                        pb::ComponentRef {
                            id: Some(peer_id.clone()),
                        },
                    )),
                }),
                label: Some(pb::CatalogTitle {
                    value: "Peer".into(),
                }),
                ..Default::default()
            });
        } else {
            value.ui_hostings.push(value.ui_hostings[0].clone());
        }
        let error = put(&mut client, ROOT_TOKEN, input, "").await.unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(error.message().contains("duplicate"));
        assert_eq!(
            get(&mut client, pb::EntityKind::Component, &owner_id)
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn global_ui_retarget_renews_hosting_occurrences_and_preserves_parallel_provenance() {
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let from = id(&namespace, "review");
    let to = id(&namespace, "other-review");
    for selected in [&from, &to] {
        put(&mut client, ROOT_TOKEN, ui(selected), "")
            .await
            .unwrap();
    }
    let owner_id = id(&namespace, "portal");
    let created = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner_id, vec![hosting(&from), hosting(&from)]),
        "",
    )
    .await
    .unwrap();
    let before = get(&mut client, pb::EntityKind::Component, &owner_id)
        .await
        .unwrap();
    let request = pb::RetargetReferencesRequest {
        from: Some(pb::EntityRef {
            kind: pb::EntityKind::Ui as i32,
            id: Some(from.clone()),
        }),
        to: Some(pb::EntityRef {
            kind: pb::EntityKind::Ui as i32,
            id: Some(to.clone()),
        }),
        filter_kinds: vec![pb::EntityKind::Component as i32],
        ..Default::default()
    };
    let mut preview = request.clone();
    preview.dry_run = true;
    let preview = client
        .retarget_references(authed(preview, ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((preview.rewritten, preview.failed), (1, 0));
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap(),
        before
    );
    let result = client
        .retarget_references(authed(request, ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((result.rewritten, result.failed), (1, 0));
    assert_eq!(
        result.results[0].fields,
        vec!["ui_hostings[0].ui", "ui_hostings[1].ui"]
    );
    let after = get(&mut client, pb::EntityKind::Component, &owner_id)
        .await
        .unwrap()
        .entity
        .unwrap();
    assert_eq!(
        after.system.as_ref().unwrap().uid,
        created
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
    );
    for (index, value) in hostings(&after).iter().enumerate() {
        assert_eq!(value.ui.as_ref().unwrap().id, Some(to.clone()));
        assert_ne!(
            value.connection_id,
            hostings(created.entity.as_ref().unwrap())[index].connection_id
        );
        assert_eq!(
            value.label,
            hostings(created.entity.as_ref().unwrap())[index].label
        );
    }
    let old = client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(from),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(old.references.is_empty());
    let new = client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(to),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(new.references.len(), 2);
    assert!(new
        .references
        .iter()
        .all(|reference| reference.from_uid == after.system.as_ref().unwrap().uid));
}

#[tokio::test(flavor = "multi_thread")]
async fn landing_revalidates_new_hostings_while_metadata_rebase_preserves_dangling_branch_assertions(
) {
    use support::two_tenants::authed_branch;
    let mut client = serve().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    let owner_id = id(&namespace, "portal");
    put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let baseline = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner_id, Vec::new()),
        "",
    )
    .await
    .unwrap();
    let branch = format!("hosting-land-{}", uuid::Uuid::now_v7().simple());
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: branch.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut changed = baseline.entity.clone().unwrap();
    component_mut(&mut changed)
        .ui_hostings
        .push(hosting(&target));
    let branched = client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(changed),
                if_match: baseline.etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap()
        .into_inner();
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Ui as i32,
                id: Some(target),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    for dry_run in [true, false] {
        let error = client
            .merge_branch(authed(
                pb::MergeBranchRequest {
                    name: branch.clone(),
                    dry_run,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(error
            .message()
            .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
        assert_eq!(
            get(&mut client, pb::EntityKind::Component, &owner_id)
                .await
                .unwrap()
                .entity,
            baseline.entity
        );
    }
    let rebased = client
        .update_branch(authed(
            pb::UpdateBranchRequest {
                name: branch.clone(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(rebased.rebased_count, 1);
    let retained = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(owner_id.clone()),
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(retained.entity, branched.entity);
    assert!(!retained.etag.is_empty());
    let still_invalid = client
        .merge_branch(authed(
            pb::MergeBranchRequest {
                name: branch,
                dry_run: true,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(still_invalid.code(), Code::InvalidArgument);
    assert!(still_invalid
        .message()
        .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap()
            .entity,
        baseline.entity
    );
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap()
            .etag,
        baseline.etag
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_retired_hosting_requires_its_ui_again_before_resolution_can_publish() {
    use support::two_tenants::authed_branch;
    let (mut client, store) = serve_with_store().await;
    let namespace = register(&mut client, ACME_TOKEN).await;
    let target = id(&namespace, "review");
    let owner_id = id(&namespace, "portal");
    let original_ui = put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    let saved = put(
        &mut client,
        ROOT_TOKEN,
        component(&owner_id, vec![hosting(&target), hosting(&target)]),
        "",
    )
    .await
    .unwrap();
    let branch = format!("hosting-restore-{}", uuid::Uuid::now_v7().simple());
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: branch.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut selected = saved.entity.clone().unwrap();
    component_mut(&mut selected).title = "Retain authored hostings".into();
    client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(selected),
                if_match: saved.etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap();
    let mut removed = saved.entity.clone().unwrap();
    component_mut(&mut removed).ui_hostings.pop();
    put(&mut client, ROOT_TOKEN, removed, &saved.etag)
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
    assert_eq!(
        get(&mut client, pb::EntityKind::Ui, &target)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let baseline_before = get(&mut client, pb::EntityKind::Component, &owner_id)
        .await
        .unwrap();
    let branch_before = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(owner_id.clone()),
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap()
        .into_inner();
    let sequence = store.current_change_seq().await.unwrap();
    let resolution = pb::ResolveBranchEntryRequest {
        name: branch.clone(),
        r#ref: Some(reference(branch_before.entity.as_ref().unwrap())),
        resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
        ..Default::default()
    };
    let error = client
        .resolve_branch_entry(authed(resolution.clone(), ROOT_TOKEN))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error
        .message()
        .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
    assert_eq!(store.current_change_seq().await.unwrap(), sequence);
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap(),
        baseline_before
    );
    assert_eq!(
        client
            .get_entity(authed_branch(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(owner_id.clone())
                },
                ROOT_TOKEN,
                &branch
            ))
            .await
            .unwrap()
            .into_inner(),
        branch_before
    );
    let replacement_ui = put(&mut client, ROOT_TOKEN, ui(&target), "").await.unwrap();
    assert_ne!(
        replacement_ui
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        original_ui
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
    );
    client
        .resolve_branch_entry(authed(resolution, ROOT_TOKEN))
        .await
        .unwrap();
    let branch_after = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(owner_id.clone()),
            },
            ROOT_TOKEN,
            &branch,
        ))
        .await
        .unwrap()
        .into_inner();
    let before = branch_before.entity.as_ref().unwrap();
    let after = branch_after.entity.as_ref().unwrap();
    assert_eq!(
        hostings(after)[0].connection_id,
        hostings(before)[0].connection_id
    );
    assert_ne!(
        hostings(after)[1].connection_id,
        hostings(before)[1].connection_id
    );
    let mut expected = before.clone();
    component_mut(&mut expected).ui_hostings[1].connection_id =
        hostings(after)[1].connection_id.clone();
    assert_eq!(*after, expected);
    assert_eq!(
        get(&mut client, pb::EntityKind::Component, &owner_id)
            .await
            .unwrap(),
        baseline_before
    );
}
