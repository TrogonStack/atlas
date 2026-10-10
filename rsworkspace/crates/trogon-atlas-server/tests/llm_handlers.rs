//! Handler-level tests for `InferDataFlow` and `CheckInformationCompleteness`
//! exercised through `EventModelServiceImpl` with a mock `LlmClient`.
//!
//! Three scenarios per handler:
//!   (a) LLM returns valid JSON  => LLM provenance
//!   (b) LLM errors              => falls back to deterministic, succeeds
//!   (c) LLM returns garbage JSON => fallback also engages, succeeds

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::{
    llm::LlmClient, llm_analysis::LlmAnalyzer, service::EventModelServiceImpl,
};

// ---------------------------------------------------------------------------
// Mock LLM client
// ---------------------------------------------------------------------------

struct MockLlmClient {
    response: Result<String>,
}

impl MockLlmClient {
    fn ok(body: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            response: Ok(body.into()),
        })
    }

    fn err(msg: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            response: Err(anyhow::anyhow!("{}", msg.into())),
        })
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
        match &self.response {
            Ok(s) => Ok(s.clone()),
            Err(e) => Err(anyhow::anyhow!("{e}")),
        }
    }

    fn model(&self) -> &'static str {
        "mock"
    }

    fn provider(&self) -> &'static str {
        "mock"
    }
}

// ---------------------------------------------------------------------------
// Test fixture helpers
// ---------------------------------------------------------------------------

async fn svc_with_llm(client: Arc<dyn LlmClient>) -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let analyzer = Arc::new(LlmAnalyzer::new(client));
    EventModelServiceImpl::try_new(Arc::new(store))
        .unwrap()
        .with_llm_analyzer(analyzer)
}

fn slice_scope(ns: &str, slug: &str) -> pb::AnalysisScope {
    pb::AnalysisScope {
        scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 0,
            }),
        })),
    }
}

// A minimal valid JSON response for InferDataFlow with no mappings.
const INFER_EMPTY_JSON: &str = r#"{"mappings":[]}"#;

// A minimal valid JSON response for CheckInformationCompleteness with no gaps.
const COMPLETENESS_EMPTY_JSON: &str = r#"{"gaps":[],"overall_score":1.0}"#;

// ---------------------------------------------------------------------------
// InferDataFlow tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_llm_valid_json_uses_llm_provenance() {
    let client = MockLlmClient::ok(INFER_EMPTY_JSON);
    let svc = svc_with_llm(client).await;

    // The store is empty so resolve_scope returns no links, which means the
    // analyzer short-circuits before calling the LLM. Seed a minimal slice so
    // there is something for the scope to resolve.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice(
            "test", "sl", "test", "cmd", "test", "ev",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(slice_scope("test", "sl")),
        }))
        .await
        .unwrap()
        .into_inner();

    // The LLM returned valid JSON; provenance must be LLM.
    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Llm as i32,
        "expected Llm provenance"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_llm_error_falls_back_to_deterministic() {
    let client = MockLlmClient::err("simulated network error");
    let svc = svc_with_llm(client).await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice(
            "test2", "sl", "test2", "cmd", "test2", "ev",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // The handler must succeed and report deterministic provenance.
    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(slice_scope("test2", "sl")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "expected fallback to deterministic after LLM error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_llm_garbage_json_falls_back_to_deterministic() {
    let client = MockLlmClient::ok("this is not json at all !!!");
    let svc = svc_with_llm(client).await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice(
            "test3", "sl", "test3", "cmd", "test3", "ev",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(slice_scope("test3", "sl")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "expected fallback to deterministic after LLM JSON parse failure"
    );
}

// ---------------------------------------------------------------------------
// CheckInformationCompleteness tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn check_completeness_llm_valid_json_uses_llm_provenance() {
    let client = MockLlmClient::ok(COMPLETENESS_EMPTY_JSON);
    let svc = svc_with_llm(client).await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice(
            "test4", "sl", "test4", "cmd", "test4", "ev",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: Some(slice_scope("test4", "sl")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Llm as i32,
        "expected Llm provenance"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn check_completeness_llm_error_falls_back_to_deterministic() {
    let client = MockLlmClient::err("simulated timeout");
    let svc = svc_with_llm(client).await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice(
            "test5", "sl", "test5", "cmd", "test5", "ev",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: Some(slice_scope("test5", "sl")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "expected fallback to deterministic after LLM error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn check_completeness_llm_garbage_json_falls_back_to_deterministic() {
    let client = MockLlmClient::ok("{not valid json}}}}}}");
    let svc = svc_with_llm(client).await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice(
            "test6", "sl", "test6", "cmd", "test6", "ev",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: Some(slice_scope("test6", "sl")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "expected fallback to deterministic after LLM JSON parse failure"
    );
}

/// Empty scope short-circuits inside the analyzer without calling the LLM.
/// Stamping `AnalysisProvenance::Llm` on that Ok is wrong provenance: no
/// model ran. The handler must fall back to deterministic instead.
#[tokio::test(flavor = "multi_thread")]
async fn infer_empty_scope_must_not_claim_llm_provenance() {
    let client = MockLlmClient::ok(INFER_EMPTY_JSON);
    let svc = svc_with_llm(client).await;

    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(slice_scope("missing-ns", "missing-slice")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.mappings,
        [] as [trogon_atlas_proto::InferredFieldMapping; 0]
    );
    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "empty scope must not claim Llm provenance when no LLM call occurred"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn check_empty_scope_must_not_claim_llm_provenance() {
    let client = MockLlmClient::ok(COMPLETENESS_EMPTY_JSON);
    let svc = svc_with_llm(client).await;

    let resp = svc
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: Some(slice_scope("missing-ns", "missing-slice")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.gaps, [] as [trogon_atlas_proto::CompletenessGap; 0]);
    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "empty scope must not claim Llm provenance when no LLM call occurred"
    );
}

// ---------------------------------------------------------------------------
// Entity builder helper
// ---------------------------------------------------------------------------

fn make_command_slice(
    ns: &str,
    slice_slug: &str,
    cmd_ns: &str,
    cmd_slug: &str,
    ev_ns: &str,
    ev_slug: &str,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slice_slug.into(),
                version: 0,
            }),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(pb::Id {
                        namespace: cmd_ns.into(),
                        slug: cmd_slug.into(),
                        version: 0,
                    }),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(pb::Id {
                        namespace: ev_ns.into(),
                        slug: ev_slug.into(),
                        version: 0,
                    }),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}
