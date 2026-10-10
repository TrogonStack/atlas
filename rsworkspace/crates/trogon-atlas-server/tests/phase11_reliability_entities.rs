#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::service::EventModelServiceImpl;

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

async fn svc() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    EventModelServiceImpl::try_new(Arc::new(store)).unwrap()
}

async fn roundtrip(
    svc: &EventModelServiceImpl,
    kind: pb::EntityKind,
    entity: pb::Entity,
    slug: &str,
) {
    let put = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(entity),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(put.etag, "");

    let got = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(id("reliability", slug, 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.etag, put.etag);
    assert!(got.entity.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn service_level_indicator_put_get_roundtrip() {
    let svc = svc().await;
    let entity = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ServiceLevelIndicator(
            pb::ServiceLevelIndicator {
                id: Some(id("reliability", "checkout-latency", 1)),
                title: "Checkout latency".into(),
                ..Default::default()
            },
        )),
    };
    roundtrip(
        &svc,
        pb::EntityKind::ServiceLevelIndicator,
        entity,
        "checkout-latency",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn service_level_objective_put_get_roundtrip() {
    let svc = svc().await;
    let entity = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ServiceLevelObjective(
            pb::ServiceLevelObjective {
                id: Some(id("reliability", "checkout-availability", 1)),
                title: "Checkout availability".into(),
                ..Default::default()
            },
        )),
    };
    roundtrip(
        &svc,
        pb::EntityKind::ServiceLevelObjective,
        entity,
        "checkout-availability",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn alert_policy_put_get_roundtrip() {
    let svc = svc().await;
    let entity = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::AlertPolicy(pb::AlertPolicy {
            id: Some(id("reliability", "checkout-slo-burn", 1)),
            title: "Checkout SLO burn".into(),
            ..Default::default()
        })),
    };
    roundtrip(
        &svc,
        pb::EntityKind::AlertPolicy,
        entity,
        "checkout-slo-burn",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn alert_notification_target_put_get_roundtrip() {
    let svc = svc().await;
    let entity = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::AlertNotificationTarget(
            pb::AlertNotificationTarget {
                id: Some(id("reliability", "oncall-pagerduty", 1)),
                title: "On-call PagerDuty".into(),
                target: "pagerduty:service-123".into(),
                ..Default::default()
            },
        )),
    };
    roundtrip(
        &svc,
        pb::EntityKind::AlertNotificationTarget,
        entity,
        "oncall-pagerduty",
    )
    .await;
}
