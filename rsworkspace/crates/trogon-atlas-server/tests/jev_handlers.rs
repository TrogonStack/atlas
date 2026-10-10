#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use async_trait::async_trait;
use tonic::Request;
use trogon_atlas_core::OwnerId;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{
    auth::{Principal, PrincipalKind, Role},
    jev::{JevAnalyzer, JevConfig},
    llm::LlmClient,
    llm_analysis::LlmAnalyzer,
    service::EventModelServiceImpl,
};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Respond, ResponseTemplate,
};

#[derive(Default)]
struct FallbackClient {
    calls: AtomicUsize,
    fail: bool,
}

#[async_trait]
impl LlmClient for FallbackClient {
    async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("simulated provider failure");
        }
        Ok(r#"{"mappings":[]}"#.into())
    }

    fn model(&self) -> &'static str {
        "fallback-test"
    }

    fn provider(&self) -> &'static str {
        "mock"
    }
}

async fn service(
    server: &MockServer,
    fallback: Option<Arc<FallbackClient>>,
) -> EventModelServiceImpl {
    let mut config = JevConfig::new("test-gateway-key".into());
    config.base_url = server.uri();
    config.timeout = Duration::from_secs(5);
    service_with_config(config, fallback).await
}

async fn service_with_config(
    config: JevConfig,
    fallback: Option<Arc<FallbackClient>>,
) -> EventModelServiceImpl {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut svc = EventModelServiceImpl::try_new(Arc::new(store))
        .unwrap()
        .with_jev_analyzer(Arc::new(JevAnalyzer::new(config).unwrap()));
    if let Some(client) = fallback {
        svc = svc.with_llm_analyzer(Arc::new(LlmAnalyzer::new(client)));
    }
    svc
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn field(name: &str) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType {
            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
        }),
        doc: "Stable identifier of the customer placing the order".into(),
        ..Default::default()
    }
}

fn command(namespace: &str, field_name: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Command(pb::Command {
            id: Some(id(namespace, "place-order")),
            schema: Some(trogon_atlas_core::schema::pack_fields(vec![field(
                field_name,
            )])),
            ..Default::default()
        })),
    }
}

fn on_branch<T>(body: T, branch: Option<&str>) -> Request<T> {
    let mut req = Request::new(body);
    if let Some(branch) = branch {
        req.metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    req
}

async fn put(svc: &EventModelServiceImpl, entity: pb::Entity, branch: Option<&str>) {
    svc.put_entity(on_branch(
        pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        },
        branch,
    ))
    .await
    .unwrap();
}

async fn seed_flow(svc: &EventModelServiceImpl, namespace: &str, source_field: &str) {
    put(svc, command(namespace, source_field), None).await;
    put(
        svc,
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(namespace, "order-placed")),
                schema: Some(trogon_atlas_core::schema::pack_fields(vec![field(
                    "customer_id",
                )])),
                ..Default::default()
            })),
        },
        None,
    )
    .await;
    put(
        svc,
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(namespace, "checkout")),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(namespace, "place-order")),
                    }),
                    ..Default::default()
                }),
                emitted_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id(namespace, "order-placed")),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        },
        None,
    )
    .await;
}

fn infer_request(namespace: &str) -> pb::InferDataFlowRequest {
    pb::InferDataFlowRequest {
        scope: Some(pb::AnalysisScope {
            scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id(namespace, "checkout")),
            })),
        }),
    }
}

async fn infer(
    svc: &EventModelServiceImpl,
    namespace: &str,
    branch: Option<&str>,
) -> pb::InferDataFlowResponse {
    svc.infer_data_flow(on_branch(infer_request(namespace), branch))
        .await
        .unwrap()
        .into_inner()
}

struct ChoiceAnswer {
    confidence: f64,
}

impl Respond for ChoiceAnswer {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let mut answers = serde_json::Map::new();
        for (question_id, question) in body["questions"].as_object().unwrap() {
            let criteria = question["criteria"].as_object().unwrap();
            assert_eq!(criteria.len(), 2);
            let source = criteria
                .keys()
                .find(|key| key.as_str() != "unknown")
                .unwrap();
            let mut probabilities = serde_json::Map::new();
            probabilities.insert(source.clone(), serde_json::json!(self.confidence));
            probabilities.insert("unknown".into(), serde_json::json!(1.0 - self.confidence));
            answers.insert(
                question_id.clone(),
                serde_json::json!({
                    "type": "choice",
                    "choice": source,
                    "probabilities": probabilities
                }),
            );
        }
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "typesafe-ai/jev",
            "answers": answers
        }))
    }
}

fn answer(confidence: f64) -> ChoiceAnswer {
    ChoiceAnswer { confidence }
}

async fn mount(server: &MockServer, response: impl Respond + 'static, count: u64) {
    Mock::given(method("POST"))
        .and(path("/v1/evaluate"))
        .and(header("authorization", "Bearer test-gateway-key"))
        .respond_with(response)
        .expect(count)
        .mount(server)
        .await;
}

fn assert_source(response: &pb::InferDataFlowResponse, namespace: &str, field_name: &str) {
    assert_eq!(response.mappings.len(), 1);
    let mapping = &response.mappings[0];
    assert_eq!(mapping.target_field, "customer_id");
    assert_eq!(
        mapping.target_entity.as_ref().unwrap().id.as_ref().unwrap(),
        &id(namespace, "order-placed")
    );
    let Some(pb::inferred_field_mapping::Source::FromField(source)) = &mapping.source else {
        panic!("expected a source field: {:?}", mapping.source);
    };
    assert_eq!(source.field_path, field_name);
    assert_eq!(
        source.from_entity.as_ref().unwrap().id.as_ref().unwrap(),
        &id(namespace, "place-order")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn semantic_rename_uses_jev_and_reuses_cached_evaluation() {
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 1).await;
    let fallback = Arc::new(FallbackClient::default());
    let svc = service(&server, Some(fallback.clone())).await;
    seed_flow(&svc, "orders", "buyer_id").await;

    let response = infer(&svc, "orders", None).await;
    assert_eq!(response.provenance, pb::AnalysisProvenance::Llm as i32);
    assert_eq!(response.fallback_reason, "");
    assert_source(&response, "orders", "buyer_id");
    assert_eq!(infer(&svc, "orders", None).await, response);
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 0);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn exact_mapping_avoids_both_providers_and_reports_deterministic_provenance() {
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 0).await;
    let fallback = Arc::new(FallbackClient::default());
    let svc = service(&server, Some(fallback.clone())).await;
    seed_flow(&svc, "orders", "customer_id").await;

    let response = infer(&svc, "orders", None).await;
    assert_eq!(
        response.provenance,
        pb::AnalysisProvenance::Deterministic as i32
    );
    assert_eq!(response.fallback_reason, "");
    assert_source(&response, "orders", "customer_id");
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 0);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_failure_uses_existing_llm_and_preserves_fallback_reason() {
    let server = MockServer::start().await;
    mount(&server, ResponseTemplate::new(503), 1).await;
    let fallback = Arc::new(FallbackClient::default());
    let svc = service(&server, Some(fallback.clone())).await;
    seed_flow(&svc, "orders", "buyer_id").await;

    let response = infer(&svc, "orders", None).await;
    assert_eq!(response.provenance, pb::AnalysisProvenance::Llm as i32);
    assert_eq!(response.fallback_reason, "jev_gateway_unavailable");
    assert_eq!(
        response.mappings,
        [] as [trogon_atlas_proto::InferredFieldMapping; 0]
    );
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 1);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn uncertain_evaluation_uses_existing_llm() {
    let server = MockServer::start().await;
    mount(&server, answer(0.55), 1).await;
    let fallback = Arc::new(FallbackClient::default());
    let svc = service(&server, Some(fallback.clone())).await;
    seed_flow(&svc, "orders", "buyer_id").await;

    let response = infer(&svc, "orders", None).await;
    assert_eq!(response.provenance, pb::AnalysisProvenance::Llm as i32);
    assert_eq!(response.fallback_reason, "jev_uncertain");
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 1);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_failure_without_llm_returns_deterministic_mapping() {
    let server = MockServer::start().await;
    mount(&server, ResponseTemplate::new(503), 1).await;
    let svc = service(&server, None).await;
    seed_flow(&svc, "orders", "buyer_id").await;

    let response = infer(&svc, "orders", None).await;
    assert_eq!(
        response.provenance,
        pb::AnalysisProvenance::Deterministic as i32
    );
    assert_eq!(response.fallback_reason, "jev_gateway_unavailable");
    assert_eq!(response.mappings.len(), 1);
    assert_eq!(response.mappings[0].target_field, "customer_id");
    assert_eq!(
        response.mappings[0].source,
        Some(pb::inferred_field_mapping::Source::Origin(
            pb::InferredOrigin::Unknown as i32
        ))
    );
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn both_provider_failures_report_both_reasons_and_return_deterministic_mapping() {
    let server = MockServer::start().await;
    mount(&server, ResponseTemplate::new(503), 1).await;
    let fallback = Arc::new(FallbackClient {
        fail: true,
        ..Default::default()
    });
    let svc = service(&server, Some(fallback.clone())).await;
    seed_flow(&svc, "orders", "buyer_id").await;

    let response = infer(&svc, "orders", None).await;
    assert_eq!(
        response.provenance,
        pb::AnalysisProvenance::Deterministic as i32
    );
    assert_eq!(
        response.fallback_reason,
        "jev_gateway_unavailable,llm_error"
    );
    assert_eq!(response.mappings.len(), 1);
    assert_eq!(response.mappings[0].target_field, "customer_id");
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 1);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_changes_produce_distinct_evaluations_with_scoped_inputs() {
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 2).await;
    let svc = service(&server, None).await;
    seed_flow(&svc, "orders", "buyer_id").await;
    seed_flow(&svc, "unrelated", "unrelated_marker").await;
    svc.create_branch(Request::new(pb::CreateBranchRequest {
        name: "rename-buyer".into(),
        doc: String::new(),
    }))
    .await
    .unwrap();
    put(
        &svc,
        command("orders", "purchaser_id"),
        Some("rename-buyer"),
    )
    .await;

    let baseline = infer(&svc, "orders", None).await;
    let branch = infer(&svc, "orders", Some("rename-buyer")).await;
    assert_source(&baseline, "orders", "buyer_id");
    assert_source(&branch, "orders", "purchaser_id");
    assert_eq!(infer(&svc, "orders", None).await, baseline);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let baseline_input = String::from_utf8(requests[0].body.clone()).unwrap();
    let branch_input = String::from_utf8(requests[1].body.clone()).unwrap();
    assert!(baseline_input.contains("buyer_id"));
    assert!(!baseline_input.contains("purchaser_id"));
    assert!(branch_input.contains("purchaser_id"));
    assert!(!branch_input.contains("buyer_id"));
    assert!(!baseline_input.contains("unrelated_marker"));
    assert!(!branch_input.contains("unrelated_marker"));
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_evaluation_cannot_reveal_an_invisible_scope() {
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 1).await;
    let svc = service(&server, None).await;
    seed_flow(&svc, "orders", "buyer_id").await;
    assert_source(&infer(&svc, "orders", None).await, "orders", "buyer_id");

    let mut restricted = Request::new(infer_request("orders"));
    restricted.extensions_mut().insert(Principal {
        name: "other-reader".into(),
        role: Role::Reader,
        kind: PrincipalKind::User,
        is_anonymous: false,
        namespaces: Default::default(),
        parent: Some(OwnerId::parse("other-owner").unwrap()),
    });
    let response = svc.infer_data_flow(restricted).await.unwrap().into_inner();
    assert_eq!(
        response.provenance,
        pb::AnalysisProvenance::Deterministic as i32
    );
    assert_eq!(
        response.mappings,
        [] as [trogon_atlas_proto::InferredFieldMapping; 0]
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires AI_GATEWAY_API_KEY and Docker"]
async fn live_jev_semantic_mapping() {
    let api_key = std::env::var("AI_GATEWAY_API_KEY").expect("AI_GATEWAY_API_KEY must be set");
    let svc = service_with_config(JevConfig::new(api_key), None).await;
    seed_flow(&svc, "jev-smoke", "buyer_id").await;
    put(
        &svc,
        pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id("jev-smoke", "order-placed")),
                schema: Some(trogon_atlas_core::schema::pack_fields(vec![pb::FieldSpec {
                    doc: "The customer_id is copied unchanged from buyer_id on the place-order command that emitted this event.".into(),
                    ..field("customer_id")
                }])),
                ..Default::default()
            })),
            ..Default::default()
        },
        None,
    )
    .await;

    let started = Instant::now();
    let cold = infer(&svc, "jev-smoke", None).await;
    let cold_elapsed = started.elapsed();
    assert_eq!(
        cold.provenance,
        pb::AnalysisProvenance::Llm as i32,
        "Jev fallback reason: {}",
        cold.fallback_reason
    );
    assert!(cold.fallback_reason.is_empty(), "{}", cold.fallback_reason);
    assert_source(&cold, "jev-smoke", "buyer_id");

    let started = Instant::now();
    let warm = infer(&svc, "jev-smoke", None).await;
    let warm_elapsed = started.elapsed();
    assert_eq!(warm, cold);
    eprintln!("Jev handler latency: cold={cold_elapsed:?}, warm={warm_elapsed:?}");
}
