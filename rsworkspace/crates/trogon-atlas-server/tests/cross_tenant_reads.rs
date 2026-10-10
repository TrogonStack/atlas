//! Cross-tenant read isolation, one test per analysis and history RPC.
//!
//! `tests/namespace_ownership.rs` proves the boundary on the list and
//! key-shaped reads. The RPCs here are the rest of the read surface: graph
//! walks, projections, validators, and the per-entity revision log. Each
//! reaches entities by a different route, and a boundary is only as good as
//! its weakest route, so each one gets its own test rather than trusting
//! that they all funnel through the same helper. They do not: some take a
//! lens-filtered snapshot, some check one namespace up front, and the
//! difference is invisible from the outside.
//!
//! Every test writes as `acme` and reads as `beta`. The assertion is always
//! the same: `beta` learns nothing about `acme`'s entity, whether by being
//! told it is missing or by being handed an empty result.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    service::EventModelServiceImpl,
};

const ACME_TOKEN: &str = "xt-acme-token";
const BETA_TOKEN: &str = "xt-beta-token";

/// The namespace `acme` owns and `beta` must never see into.
const NS: &str = "xt-private";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.acme-writer]
role = "writer"
tokens = ["{ACME_TOKEN}"]
parent = "acme"

[principals.beta-writer]
role = "writer"
tokens = ["{BETA_TOKEN}"]
parent = "beta"
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

async fn client() -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc = EventModelServiceServer::new(svc);

    let auth = BearerAuth::new(Arc::new(make_registry()));
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
        auth,
    );

    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

fn authed<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    req
}

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: NS.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event_entity(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(slug)),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

/// A command slice that emits `event_slug`, so the graph has one edge to
/// walk and the projections have something to project.
fn slice_entity(slug: &str, event_slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(slug)),
            title: slug.into(),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(event_slug)),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}

/// A storyboard naming the slice, so its projection has something to inline.
fn storyboard_entity(slug: &str, slice_slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id(slug)),
            title: slug.into(),
            slices: vec![pb::SliceRef {
                id: Some(id(slice_slug)),
            }],
            ..Default::default()
        })),
    }
}

/// An event model whose members are the slice and the event.
fn event_model_entity(slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(slug)),
            title: slug.into(),
            members: vec![
                pb::EntityRef {
                    kind: pb::EntityKind::CommandSlice as i32,
                    id: Some(id("place")),
                },
                pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("placed")),
                },
            ],
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }
}

fn eref(kind: pb::EntityKind, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(slug)),
    }
}

/// Writes `acme`'s private graph and hands back a client. Every test starts
/// here so the data `beta` is probing for genuinely exists.
async fn world() -> EventModelServiceClient<Channel> {
    let mut c = client().await;
    c.put_entity(authed(put_req(event_entity("placed")), ACME_TOKEN))
        .await
        .expect("acme writes its event");
    c.put_entity(authed(put_req(slice_entity("place", "placed")), ACME_TOKEN))
        .await
        .expect("acme writes its slice");
    c.put_entity(authed(
        put_req(storyboard_entity("checkout", "place")),
        ACME_TOKEN,
    ))
    .await
    .expect("acme writes its storyboard");
    c.put_entity(authed(put_req(event_model_entity("model")), ACME_TOKEN))
        .await
        .expect("acme writes its event model");
    c
}

/// A cross-tenant read must not distinguish "yours, but hidden" from
/// "absent". `NOT_FOUND` and an empty result both satisfy that; anything
/// else, including `PERMISSION_DENIED`, confirms the entity exists.
fn assert_hidden<T>(
    what: &str,
    result: Result<tonic::Response<T>, tonic::Status>,
    empty: impl Fn(&T) -> bool,
) {
    match result {
        Ok(resp) => {
            let body = resp.into_inner();
            assert!(
                empty(&body),
                "{what} handed beta something belonging to acme",
            );
        }
        Err(status) => assert_eq!(
            status.code(),
            Code::NotFound,
            "{what} answered {:?}, which tells beta the entity exists: {}",
            status.code(),
            status.message(),
        ),
    }
}

/// `get_impact` echoes the root ref it was handed back as a depth-0 node
/// whether or not the entity exists, so the property that matters is not an
/// empty response but an *indistinguishable* one: beta must get the same
/// answer for acme's entity as for one that was never written.
#[tokio::test(flavor = "multi_thread")]
async fn get_impact_answers_the_same_for_a_hidden_entity_and_a_missing_one() {
    let mut c = world().await;

    async fn nodes_for(
        c: &mut EventModelServiceClient<Channel>,
        slug: &str,
    ) -> Result<usize, Code> {
        c.get_impact(authed(
            pb::GetImpactRequest {
                root: Some(eref(pb::EntityKind::Event, slug)),
                max_depth: 5,
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await
        .map(|r| r.into_inner().nodes.len())
        .map_err(|s| s.code())
    }

    assert_eq!(
        nodes_for(&mut c, "placed").await,
        nodes_for(&mut c, "never-written").await,
        "get_impact distinguishes acme's entity from one that does not exist",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outgoing_references_does_not_cross_the_owner_boundary() {
    let mut c = world().await;
    let out = c
        .get_outgoing_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id("place")),
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("get_outgoing_references", out, |r| r.references.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_supersession_chain_does_not_cross_the_owner_boundary() {
    let mut c = world().await;
    let out = c
        .get_supersession_chain(authed(
            pb::GetSupersessionChainRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("placed")),
                direction: pb::get_supersession_chain_request::Direction::Both as i32,
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("get_supersession_chain", out, |r| r.chain.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn list_versions_does_not_enumerate_another_owners_versions() {
    let mut c = world().await;
    let out = c
        .list_versions(authed(
            pb::ListVersionsRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: NS.into(),
                slug: "placed".into(),
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("list_versions", out, |r| r.versions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_latest_version_does_not_confirm_another_owners_entity() {
    let mut c = world().await;
    let out = c
        .get_latest_version(authed(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: NS.into(),
                slug: "placed".into(),
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("get_latest_version", out, |r| r.entity.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_entity_history_does_not_expose_another_owners_revisions() {
    let mut c = world().await;
    let out = c
        .get_entity_history(authed(
            pb::GetEntityHistoryRequest {
                r#ref: Some(eref(pb::EntityKind::Event, "placed")),
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("get_entity_history", out, |r| r.revisions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_does_not_read_another_owners_entities() {
    let mut c = world().await;
    let out = c
        .diff_entities(authed(
            pb::DiffEntitiesRequest {
                a: Some(eref(pb::EntityKind::Event, "placed")),
                b: Some(eref(pb::EntityKind::CommandSlice, "place")),
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("diff_entities", out, |_| false);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_slice_projection_does_not_project_another_owners_slice() {
    let mut c = world().await;
    let out = c
        .get_slice_projection(authed(
            pb::GetSliceProjectionRequest {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id("place")),
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("get_slice_projection", out, |r| r.projection.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn extract_subgraph_does_not_extract_another_owners_subgraph() {
    let mut c = world().await;
    let out = c
        .extract_subgraph(authed(
            pb::ExtractSubgraphRequest {
                scope: Some(pb::AnalysisScope {
                    scope: Some(pb::analysis_scope::Scope::Slice(eref(
                        pb::EntityKind::CommandSlice,
                        "place",
                    ))),
                }),
                new_event_model_id: Some(pb::Id {
                    namespace: "beta-ns".into(),
                    slug: "stolen".into(),
                    version: 1,
                }),
                include_cross_model: true,
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("extract_subgraph", out, |r| r.entities.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_does_not_analyze_another_owners_slice() {
    let mut c = world().await;
    let out = c
        .infer_data_flow(authed(
            pb::InferDataFlowRequest {
                scope: Some(pb::AnalysisScope {
                    scope: Some(pb::analysis_scope::Scope::Slice(eref(
                        pb::EntityKind::CommandSlice,
                        "place",
                    ))),
                }),
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("infer_data_flow", out, |r| r.mappings.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_does_not_destroy_another_owners_entity() {
    let mut c = world().await;
    let err = c
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                operation_id: String::new(),
                kind: pb::EntityKind::Event as i32,
                id: Some(id("placed")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                if_match: String::new(),
            },
            BETA_TOKEN,
        ))
        .await
        .expect_err("a cross-owner delete must be refused");
    assert!(
        matches!(err.code(), Code::NotFound | Code::PermissionDenied),
        "delete answered {:?}",
        err.code(),
    );

    c.get_entity(authed(
        pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("placed")),
        },
        ACME_TOKEN,
    ))
    .await
    .expect("acme's entity must still be there");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_storyboard_projection_does_not_project_another_owners_storyboard() {
    let mut c = world().await;
    let out = c
        .get_storyboard_projection(authed(
            pb::GetStoryboardProjectionRequest {
                id: Some(id("checkout")),
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("get_storyboard_projection", out, |r| r.projection.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_event_model_projection_does_not_project_another_owners_model() {
    let mut c = world().await;
    let out = c
        .get_event_model_projection(authed(
            pb::GetEventModelProjectionRequest {
                id: Some(id("model")),
                include_cross_model: true,
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("get_event_model_projection", out, |r| {
        r.projection.is_none()
    });
}

/// Validating by id loads the stored model. The issues it reports name the
/// model's members, so answering at all describes acme's graph to beta.
#[tokio::test(flavor = "multi_thread")]
async fn validate_event_model_does_not_load_another_owners_model() {
    let mut c = world().await;
    let out = c
        .validate_event_model(authed(
            pb::ValidateEventModelRequest {
                event_model: None,
                event_model_id: Some(id("model")),
            },
            BETA_TOKEN,
        ))
        .await;
    assert_hidden("validate_event_model", out, |r| r.issues.is_empty());
}
