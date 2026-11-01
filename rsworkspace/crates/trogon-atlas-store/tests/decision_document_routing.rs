#![allow(clippy::unwrap_used)]

use serde_json::json;
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{Store, StoreError, WriteContext};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "catalog-routing".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn entity(value: &serde_json::Value) -> pb::Entity {
    TranscodePool::builtin()
        .unwrap()
        .message_from_json_value("trogonatlas.eventmodel.v1alpha1.Entity", value)
        .unwrap()
}
#[tokio::test]
async fn catalog_store_routes_kinds_and_complete_nested_owner_references_across_branch_tombstones()
{
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let document = entity(
        &json!({"document":{"id":{"namespace":"catalog-routing","slug":"note","version":"1"},"title":{"value":"Note"},"subjects":[{"subject":{"entity":{"kind":"ENTITY_KIND_READ_MODEL","id":{"namespace":"catalog-routing","slug":"receipt","version":"1"}}}}],"link":{"value":"https://example.test/note"}}}),
    );
    store
        .create(
            pb::EntityKind::Document,
            &document,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let decision = entity(
        &json!({"decision":{"id":{"namespace":"catalog-routing","slug":"choice","version":"1"},"title":{"value":"Choice"},"authoredOn":{"value":"2024-02-29"},"decisionMakers":[{"person":{"id":{"namespace":"catalog-routing","slug":"maker","version":"1"}}},{"team":{"id":{"namespace":"catalog-routing","slug":"makers","version":"2"}}}],"amends":[{"decision":{"id":{"namespace":"catalog-routing","slug":"original","version":"1"}}}],"status":"DECISION_STATUS_PROPOSED","context":{"value":"Question"},"rationale":{"value":"Reason"},"options":[{"title":{"value":"Option"},"description":{"value":"Description"}}],"subjects":[{"subject":{"connection":{"owner":{"kind":"ENTITY_KIND_AUTOMATION_SLICE","id":{"namespace":"catalog-routing","slug":"observe","version":"1"}},"ownerUid":"01963410-1000-7000-8000-000000000001","id":{"value":"01963410-1000-7000-8000-000000000002"}}}},{"subject":{"member":{"specification":{"id":{"namespace":"catalog-routing","slug":"contract","version":"1"}},"ownerUid":"01963410-1000-7000-8000-000000000003","kind":"CONTRACT_MEMBER_KIND_MESSAGE","memberId":{"value":"01963410-1000-7000-8000-000000000004"}}}},{"subject":{"port":{"product":{"id":{"namespace":"catalog-routing","slug":"product","version":"1"}},"ownerUid":"01963410-1000-7000-8000-000000000005","direction":"DATA_PORT_DIRECTION_OUTPUT","portId":{"value":"01963410-1000-7000-8000-000000000006"}}}}],"documents":[{"document":{"document":{"id":{"namespace":"catalog-routing","slug":"note","version":"1"}}}}]}}),
    );
    store
        .create(
            pb::EntityKind::Decision,
            &decision,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    for (kind, slug) in [
        (pb::EntityKind::Decision, "choice"),
        (pb::EntityKind::Document, "note"),
    ] {
        let stored = store.get(kind, &id(slug), None).await.unwrap();
        assert_eq!(
            trogon_atlas_core::refs::entity_kind(&stored.entity),
            Some(kind)
        );
        assert!(matches!(
            store
                .get(
                    if kind == pb::EntityKind::Decision {
                        pb::EntityKind::Document
                    } else {
                        pb::EntityKind::Decision
                    },
                    &id(slug),
                    None
                )
                .await,
            Err(StoreError::NotFound)
        ));
    }
    let targets = [
        (pb::EntityKind::AutomationSlice, "observe"),
        (pb::EntityKind::Specification, "contract"),
        (pb::EntityKind::DataProduct, "product"),
        (pb::EntityKind::Document, "note"),
        (pb::EntityKind::Person, "maker"),
        (pb::EntityKind::Decision, "original"),
    ];
    for (kind, slug) in targets {
        assert!(
            store
                .has_incoming_reference(kind, &id(slug), None)
                .await
                .unwrap(),
            "{kind:?}"
        );
    }
    assert!(store
        .has_incoming_reference(pb::EntityKind::ReadModel, &id("receipt"), None)
        .await
        .unwrap());
    let mut team_id = id("makers");
    team_id.version = 2;
    assert!(store
        .has_incoming_reference(pb::EntityKind::Team, &team_id, None)
        .await
        .unwrap());
    store.create_branch("catalog-edit", "").await.unwrap();
    store
        .delete(
            pb::EntityKind::Decision,
            &id("choice"),
            None,
            WriteContext::branch("catalog-edit"),
        )
        .await
        .unwrap();
    for (kind, slug) in targets {
        assert!(!store
            .has_incoming_reference(kind, &id(slug), Some("catalog-edit"))
            .await
            .unwrap());
        assert!(store
            .has_incoming_reference(kind, &id(slug), None)
            .await
            .unwrap());
    }
    assert!(store
        .get(pb::EntityKind::Document, &id("note"), Some("catalog-edit"))
        .await
        .is_ok());
}
