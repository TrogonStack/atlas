#![allow(dead_code)]

use serde_json::{json, Value};
use trogon_atlas_client::client::Client;
use trogon_atlas_core::{connections::connection_occurrences, transcode::TranscodePool};
use trogon_atlas_proto as pb;

pub const VERSION: u64 = 9_007_199_254_740_993;
pub const OWNER: &str = "hosting-owner";
pub const BRANCH: &str = "@hosting-owner:inspection";
pub const ADMIN: &str = "hosting-client-admin";
pub const WRITER: &str = "hosting-client-writer";
pub const READER: &str = "hosting-client-reader";
pub const HIDDEN: &str = "hosting-client-hidden";

pub fn manifest(namespace: &str, kind: &str, name: &str, spec: Value) -> String {
    json!({
        "apiVersion": trogon_atlas_client::manifest::API_VERSION,
        "kind": kind,
        "metadata": {"namespace": namespace, "name": name, "version": VERSION},
        "spec": spec
    })
    .to_string()
}

pub fn manifests(namespace: &str) -> String {
    let target =
        |slug| json!({"id":{"namespace":namespace,"slug":slug,"version":VERSION.to_string()}});
    [
        manifest(namespace, "Ui", "review", json!({"title":"Review"})),
        manifest(namespace, "Persona", "reviewer", json!({"title":"Reviewer"})),
        manifest(namespace, "Command", "approve", json!({"title":"Approve"})),
        manifest(namespace, "ReadModel", "summary", json!({"title":"Summary"})),
        manifest(namespace, "Component", "portal", json!({
            "title":"Portal",
            "uiHostings":[
                {"ui":target("review"),"label":{"value":"Review portal 🦜"}},
                {"ui":target("review"),"label":{"value":"Review portal 🦜"},"doc":{"value":"<script>inert</script>"}}
            ]
        })),
        manifest(namespace, "CommandSlice", "issue", json!({
            "title":"Approve from review",
            "persona":{"persona":target("reviewer")},
            "ui":{"ui":target("review")},
            "command":{"command":target("approve")}
        })),
        manifest(namespace, "UiSlice", "display", json!({
            "title":"Display summary",
            "persona":{"persona":target("reviewer")},
            "ui":{"ui":target("review")},
            "sourceReadModels":[{"readModel":target("summary")},{"readModel":target("summary")}]
        })),
    ].join("\n---\n")
}

pub async fn get(
    client: &mut Client,
    namespace: &str,
    kind: pb::EntityKind,
    slug: &str,
) -> pb::GetEntityResponse {
    client
        .get_entity(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(pb::Id {
                namespace: namespace.into(),
                slug: slug.into(),
                version: VERSION,
            }),
        })
        .await
        .unwrap()
        .into_inner()
}

pub fn connection(entity: &pb::Entity, path: &str) -> pb::ConnectionRef {
    let occurrence = connection_occurrences(entity)
        .unwrap()
        .into_iter()
        .find(|occurrence| occurrence.path == path)
        .unwrap();
    pb::ConnectionRef {
        owner: Some(pb::EntityRef {
            kind: trogon_atlas_core::refs::entity_kind(entity).unwrap() as i32,
            id: trogon_atlas_core::refs::entity_id(entity).cloned(),
        }),
        owner_uid: entity.system.as_ref().unwrap().uid.clone(),
        id: Some(pb::ConnectionId {
            value: occurrence.id.unwrap().to_string(),
        }),
    }
}

pub async fn selections(
    client: &mut Client,
    namespace: &str,
) -> (Value, Value, pb::GetEntityResponse) {
    let component = get(client, namespace, pb::EntityKind::Component, "portal").await;
    let command = get(client, namespace, pb::EntityKind::CommandSlice, "issue")
        .await
        .entity
        .unwrap();
    let display = get(client, namespace, pb::EntityKind::UiSlice, "display")
        .await
        .entity
        .unwrap();
    let owner = component.entity.as_ref().unwrap();
    let issuance = pb::InspectHumanInteractionRequest {
        hosting: Some(connection(owner, "component.ui_hostings[1]")),
        path: Some(pb::inspect_human_interaction_request::Path::Issuance(
            pb::HumanIssuanceSelection {
                persona: Some(connection(&command, "command_slice.persona")),
                ui: Some(connection(&command, "command_slice.ui")),
                command: Some(connection(&command, "command_slice.command")),
            },
        )),
    };
    let display = pb::InspectHumanInteractionRequest {
        hosting: Some(connection(owner, "component.ui_hostings[0]")),
        path: Some(pb::inspect_human_interaction_request::Path::Display(
            pb::HumanDisplaySelection {
                persona: Some(connection(&display, "ui_slice.persona")),
                ui: Some(connection(&display, "ui_slice.ui")),
                source_read_model: Some(connection(&display, "ui_slice.source_read_models[1]")),
            },
        )),
    };
    let pool = TranscodePool::builtin().unwrap();
    let encode = |request: &pb::InspectHumanInteractionRequest| {
        pool.message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.InspectHumanInteractionRequest",
            request,
        )
        .unwrap()
    };
    (encode(&issuance), encode(&display), component)
}

pub fn assert_checked(response: &Value, request: &Value) {
    assert_eq!(&response["request"], request);
    assert_eq!(response["state"], "CATALOG_RESOLUTION_STATE_RESOLVED");
    assert_eq!(
        response["consistency"],
        "HUMAN_INTERACTION_CONSISTENCY_PER_OWNER_CHECKED_READ_SET"
    );
    let scope = &response["scope"];
    assert_eq!(scope["branch"], BRANCH);
    assert!(!scope["revision"].as_str().unwrap().is_empty());
    let edges = response["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 4);
    for edge in edges {
        assert_eq!(edge["state"], "CATALOG_RESOLUTION_STATE_RESOLVED");
        assert_eq!(
            edge["selection"]["owner"]["id"]["version"],
            VERSION.to_string()
        );
        assert_eq!(edge["target"]["id"]["version"], VERSION.to_string());
        assert!(!edge["checkedTarget"]["uid"].as_str().unwrap().is_empty());
        assert!(!edge["checkedTarget"]["etag"].as_str().unwrap().is_empty());
        assert!(!edge["selection"]["ownerUid"].as_str().unwrap().is_empty());
        assert!(!edge["selection"]["id"]["value"]
            .as_str()
            .unwrap()
            .is_empty());
    }
    let (path, terminal_role, terminal_field) = if request.get("issuance").is_some() {
        (
            &request["issuance"],
            "HUMAN_INTERACTION_ROLE_COMMAND",
            "command",
        )
    } else {
        (
            &request["display"],
            "HUMAN_INTERACTION_ROLE_READ_MODEL",
            "sourceReadModel",
        )
    };
    for (role, selection) in [
        ("HUMAN_INTERACTION_ROLE_HOSTED_UI", &request["hosting"]),
        ("HUMAN_INTERACTION_ROLE_PERSONA", &path["persona"]),
        ("HUMAN_INTERACTION_ROLE_SLICE_UI", &path["ui"]),
        (terminal_role, &path[terminal_field]),
    ] {
        let edge = edges.iter().find(|edge| edge["role"] == role).unwrap();
        assert_eq!(&edge["selection"], selection);
        assert_eq!(edge["checkedTarget"]["owner"], edge["target"]);
        assert!(!edge["field"].as_str().unwrap().is_empty());
    }
    assert_eq!(scope["visited"].as_array().unwrap().len(), 5);
}
