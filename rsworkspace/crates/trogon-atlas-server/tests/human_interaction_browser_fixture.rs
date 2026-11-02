#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use serde_json::{json, Value};
use tonic::Request;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const TOKEN: &str = "human-interaction-reader";
const INERT: &str = "<script>window.__hostingUnsafe = true</script>\nNative review";

struct Built {
    service: EventModelServiceImpl,
    receipt: Value,
}

async fn put(service: &EventModelServiceImpl, value: Value) -> pb::Entity {
    let entity = TranscodePool::builtin()
        .unwrap()
        .message_from_json_value("trogonatlas.eventmodel.v1alpha1.Entity", &value)
        .unwrap();
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

fn reference(entity: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(entity).unwrap()
}
fn uid(entity: &pb::Entity) -> &str {
    &entity.system.as_ref().unwrap().uid
}
fn selected(entity: &pb::Entity, path: &str) -> Value {
    let occurrence = trogon_atlas_core::connections::connection_occurrences(entity)
        .unwrap()
        .into_iter()
        .find(|item| item.path == path)
        .unwrap();
    TranscodePool::builtin()
        .unwrap()
        .message_json_value(
            "trogonatlas.eventmodel.v1alpha1.ConnectionRef",
            &pb::ConnectionRef {
                owner: Some(reference(entity)),
                owner_uid: uid(entity).into(),
                id: Some(pb::ConnectionId {
                    value: occurrence.id.unwrap().to_string(),
                }),
            },
        )
        .unwrap()
}

async fn build() -> Built {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "human-browser".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let id =
        |slug: &str| json!({"namespace": namespace, "slug": slug, "version": "9007199254740993"});
    let ui = put(
        &service,
        json!({"ui": {"id": id("review"), "title": "Review"}}),
    )
    .await;
    let gone = put(
        &service,
        json!({"ui": {"id": id("gone"), "title": "Gone UI"}}),
    )
    .await;
    let persona = put(
        &service,
        json!({"persona": {"id": id("reviewer"), "title": "Reviewer"}}),
    )
    .await;
    let command = put(
        &service,
        json!({"command": {"id": id("approve"), "title": "Approve"}}),
    )
    .await;
    let summary = put(
        &service,
        json!({"readModel": {"id": id("summary"), "title": "Summary"}}),
    )
    .await;
    let hosting = json!({"ui": {"id": id("review")}, "label": {"value": "Review portal 🦜"}, "doc": {"value": INERT}});
    let component = put(&service, json!({"component": {"id": id("portal"), "title": "Portal", "uiHostings": [hosting.clone(), hosting]}})).await;
    let unavailable_component = put(&service, json!({"component": {"id": id("unavailable-portal"), "title": "Unavailable portal", "uiHostings": [{"ui": {"id": id("gone")}, "label": {"value": "Unavailable target"}}]}})).await;
    let issuance = put(&service, json!({"commandSlice": {"id": id("issue"), "title": "Issue approval", "persona": {"persona": {"id": id("reviewer")}}, "ui": {"ui": {"id": id("review")}}, "command": {"command": {"id": id("approve")}}}})).await;
    let observation = json!({"readModel": {"id": id("summary")}});
    let display = put(&service, json!({"uiSlice": {"id": id("display"), "title": "Display summary", "persona": {"persona": {"id": id("reviewer")}}, "ui": {"ui": {"id": id("review")}}, "sourceReadModels": [observation.clone(), observation]}})).await;
    let unavailable_display = put(&service, json!({"uiSlice": {"id": id("unavailable-display"), "title": "Unavailable display", "persona": {"persona": {"id": id("reviewer")}}, "ui": {"ui": {"id": id("gone")}}, "sourceReadModels": [{"readModel": {"id": id("summary")}}]}})).await;
    let incomplete_display = put(&service, json!({"uiSlice": {"id": id("incomplete-display"), "title": "Incomplete display", "persona": {"persona": {"id": id("reviewer")}}, "sourceReadModels": [{"readModel": {"id": id("summary")}}]}})).await;
    let incomplete_issuance = put(&service, json!({"commandSlice": {"id": id("incomplete-issue"), "title": "Incomplete issuance", "persona": {"persona": {"id": id("reviewer")}}, "command": {"command": {"id": id("approve")}}}})).await;
    let cited_hosting = selected(&component, "component.ui_hostings[1]");
    let retired_body = json!({"component": {"id": id("retired-portal"), "title": "Retired portal", "uiHostings": [{"ui": {"id": id("review")}, "label": {"value": "Retired hosting"}}]}});
    let retired = put(&service, retired_body.clone()).await;
    let stale_hosting = selected(&retired, "component.ui_hostings[0]");
    let subjects = |connection: &Value| json!([{"subject": {"connection": connection}}]);
    let document = put(&service, json!({"document": {"id": id("hosting-guide"), "title": {"value": "Hosting guide"}, "subjects": subjects(&cited_hosting), "link": {"value": "https://example.com/hosting-guide"}}})).await;
    let decision = put(&service, json!({"decision": {"id": id("hosting-decision"), "title": {"value": "Hosting decision"}, "status": "DECISION_STATUS_PROPOSED", "context": {"value": "Review the cited hosting"}, "rationale": {"value": "Keep the sibling distinct"}, "options": [{"title": {"value": "Retain"}, "description": {"value": "Retain the cited hosting"}}], "subjects": subjects(&cited_hosting)}})).await;
    let stale_document = put(&service, json!({"document": {"id": id("stale-hosting-guide"), "title": {"value": "Stale hosting guide"}, "subjects": subjects(&stale_hosting), "link": {"value": "https://example.com/stale-hosting-guide"}}})).await;
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Component as i32,
            id: reference(&retired).id,
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let replacement = put(&service, retired_body).await;
    assert_ne!(uid(&retired), uid(&replacement));
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Ui as i32,
            id: reference(&gone).id,
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let members = [
        &ui,
        &persona,
        &command,
        &summary,
        &component,
        &unavailable_component,
        &issuance,
        &display,
        &unavailable_display,
        &incomplete_display,
        &incomplete_issuance,
        &document,
        &decision,
        &stale_document,
        &replacement,
    ]
    .into_iter()
    .map(reference)
    .collect();
    put(
        &service,
        TranscodePool::builtin()
            .unwrap()
            .entity_json_value(&pb::Entity {
                kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
                    id: Some(pb::Id {
                        namespace: namespace.clone(),
                        slug: "model".into(),
                        version: 1,
                    }),
                    title: "Human interaction browser acceptance".into(),
                    members,
                    ..Default::default()
                })),
                ..Default::default()
            })
            .unwrap(),
    )
    .await;
    let receipt = json!({
        "namespace": namespace, "model": "model", "inert": INERT,
        "hosting": [selected(&component, "component.ui_hostings[0]"), selected(&component, "component.ui_hostings[1]")],
        "issuance": {"persona": selected(&issuance, "command_slice.persona"), "ui": selected(&issuance, "command_slice.ui"), "command": selected(&issuance, "command_slice.command")},
        "display": {"persona": selected(&display, "ui_slice.persona"), "ui": selected(&display, "ui_slice.ui"), "sourceReadModel": selected(&display, "ui_slice.source_read_models[1]")},
        "citations": {"document": "hosting-guide", "decision": "hosting-decision", "stale": "stale-hosting-guide", "ownerUid": uid(&component), "staleOwnerUid": uid(&retired), "replacementOwnerUid": uid(&replacement)},
        "incomplete": {"display": id("incomplete-display"), "issuance": id("incomplete-issue")},
        "unavailable": {"hosting": selected(&unavailable_component, "component.ui_hostings[0]"), "display": {"persona": selected(&unavailable_display, "ui_slice.persona"), "ui": selected(&unavailable_display, "ui_slice.ui"), "sourceReadModel": selected(&unavailable_display, "ui_slice.source_read_models[0]")}},
    });
    Built { service, receipt }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn human_interaction_fixture_keeps_persisted_selectors() {
    let built = Box::pin(build()).await;
    assert_ne!(
        built.receipt["hosting"][0]["id"],
        built.receipt["hosting"][1]["id"]
    );
    let input =
        json!({"hosting": built.receipt["hosting"][1], "display": built.receipt["display"]});
    let request = TranscodePool::builtin()
        .unwrap()
        .message_from_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.InspectHumanInteractionRequest",
            &input,
        )
        .unwrap();
    let response = built
        .service
        .inspect_human_interaction(Request::new(request))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.state, pb::CatalogResolutionState::Resolved as i32);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual authenticated browser fixture; serves loopback for at most ten minutes"]
async fn serve_human_interaction_fixture() {
    let Built {
        service,
        mut receipt,
    } = Box::pin(build()).await;
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(
            json!({"principals": {"native-reader": {"role": "reader", "tokens": [TOKEN]}}}),
        )
        .unwrap(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    receipt["grpc"] = json!(format!("http://{}", listener.local_addr().unwrap()));
    let grpc = data_plane(
        pb::event_model_service_server::EventModelServiceServer::new(service),
        BearerAuth::new(registry),
    );
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let path = std::env::var("ATLAS_BROWSER_FIXTURE_FILE").expect("fixture receipt path required");
    std::fs::write(path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    tokio::select! { () = tokio::time::sleep(Duration::from_secs(600)) => {}, result = tokio::signal::ctrl_c() => result.unwrap() }
    server.abort();
    let _ = server.await;
}
