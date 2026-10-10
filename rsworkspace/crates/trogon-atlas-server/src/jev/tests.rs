#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{json, Value};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

use super::*;

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "shop".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn field(name: &str) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        doc: "The customer who purchases the order".into(),
        r#type: Some(pb::FieldType {
            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
        }),
        ..Default::default()
    }
}

fn fixture() -> (pb::AnalysisScope, Vec<StoredEntity>) {
    let entities = vec![
        pb::entity::Kind::Command(pb::Command {
            id: Some(id("purchase")),
            title: "Purchase order".into(),
            doc: "A customer submits an order".into(),
            schema: Some(trogon_atlas_core::schema::pack_fields(vec![field(
                "buyer_id",
            )])),
            ..Default::default()
        }),
        pb::entity::Kind::Event(pb::Event {
            id: Some(id("purchased")),
            title: "Order purchased".into(),
            doc: "Records the purchasing customer".into(),
            schema: Some(trogon_atlas_core::schema::pack_fields(vec![field(
                "customer_id",
            )])),
            ..Default::default()
        }),
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("purchase-flow")),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("purchase")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("purchased")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    ]
    .into_iter()
    .map(|kind| StoredEntity {
        entity: pb::Entity {
            kind: Some(kind),
            ..Default::default()
        },
        etag: "1".into(),
    })
    .collect();
    let scope = pb::AnalysisScope {
        scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(id("purchase-flow")),
        })),
    };
    (scope, entities)
}

fn command(all: &mut [StoredEntity]) -> &mut pb::Command {
    match all[0].entity.kind.as_mut().unwrap() {
        pb::entity::Kind::Command(command) => command,
        _ => panic!("fixture command"),
    }
}

fn event(all: &mut [StoredEntity]) -> &mut pb::Event {
    match all[1].entity.kind.as_mut().unwrap() {
        pb::entity::Kind::Event(event) => event,
        _ => panic!("fixture event"),
    }
}

struct FieldsMut<'a> {
    schema: &'a mut Option<prost_types::Any>,
    fields: Vec<pb::FieldSpec>,
}

impl std::ops::Deref for FieldsMut<'_> {
    type Target = Vec<pb::FieldSpec>;

    fn deref(&self) -> &Self::Target {
        &self.fields
    }
}

impl std::ops::DerefMut for FieldsMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.fields
    }
}

impl Drop for FieldsMut<'_> {
    fn drop(&mut self) {
        *self.schema = Some(trogon_atlas_core::schema::pack_fields(std::mem::take(
            &mut self.fields,
        )));
    }
}

fn fields_mut(schema: &mut Option<prost_types::Any>) -> FieldsMut<'_> {
    let fields = trogon_atlas_core::schema::schema_fields(schema.as_ref());
    FieldsMut { schema, fields }
}

fn answer(probability: f64) -> Value {
    json!({"model":MODEL,"answers":{"q0":{"type":"choice","choice":"c0","confidence":0.83,"probabilities":{"c0":probability,"unknown":1.0-probability}}},"providerMetadata":{"gateway":{"cost":"0"}}})
}

#[test]
fn nested_types_and_type_parameters_must_be_compatible() {
    let source = field("payload");
    let mut target = source.clone();
    target.r#type = Some(pb::FieldType { kind: None });
    assert!(!compatible(&source, &target));
    let mut source = source;
    source.r#type = Some(pb::FieldType {
        kind: Some(pb::field_type::Kind::Object(pb::field_type::ObjectType {
            fields: vec![field("name")],
        })),
    });
    target.r#type = Some(pb::FieldType {
        kind: Some(pb::field_type::Kind::Object(pb::field_type::ObjectType {
            fields: vec![field("address")],
        })),
    });
    assert!(!compatible(&source, &target));
    let mut documented = source.clone();
    if let Some(pb::field_type::Kind::Object(object)) =
        documented.r#type.as_mut().unwrap().kind.as_mut()
    {
        object.fields[0].doc = "Equivalent type with revised documentation".into();
    }
    assert!(compatible(&source, &documented));
}

fn analyzer(server: &MockServer) -> JevAnalyzer {
    let mut config = JevConfig::new("test-key".into());
    config.base_url = server.uri();
    JevAnalyzer::new(config).unwrap()
}

async fn mount(server: &MockServer, value: Value, count: u64) {
    Mock::given(method("POST"))
        .and(path("/v1/evaluate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .expect(count)
        .mount(server)
        .await;
}

#[tokio::test]
async fn semantic_rename_uses_documented_choices_and_scoped_evidence() {
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 1).await;
    let (scope, entities) = fixture();
    let result = analyzer(&server)
        .infer_data_flow(&scope, &entities)
        .await
        .unwrap();
    assert!(result.evaluated);
    assert_eq!(result.mappings.len(), 1);
    let mapping = &result.mappings[0];
    assert_eq!(mapping.target_field, "customer_id");
    assert!((mapping.confidence - 0.99).abs() < f32::EPSILON);
    let Some(pb::inferred_field_mapping::Source::FromField(source)) = &mapping.source else {
        panic!("field source")
    };
    assert_eq!(source.field_path, "buyer_id");
    assert_eq!(
        source
            .from_entity
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "purchase"
    );
    let requests = server.received_requests().await.unwrap();
    let request: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(request["model"], MODEL);
    assert_eq!(
        request["providerOptions"]["gateway"]["zeroDataRetention"],
        true
    );
    assert_eq!(request["questions"]["q0"]["type"], "choice");
    assert_eq!(
        request["state"]["flows"][0]["upstream"][0]["doc"],
        "A customer submits an order"
    );
    assert_eq!(
        request["state"]["flows"][0]["upstream"][0]["fields"][0]["doc"],
        "The customer who purchases the order"
    );
    assert!(!String::from_utf8_lossy(&requests[0].body).contains("test-key"));
}

#[tokio::test]
async fn exact_and_derived_fields_never_call_gateway() {
    let server = MockServer::start().await;
    let (scope, mut entities) = fixture();
    fields_mut(&mut event(&mut entities).schema)[0].name = "buyer_id".into();
    let mut generated = field("created_at");
    generated.metadata.push(prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation"
            .into(),
        value: Vec::new(),
    });
    fields_mut(&mut event(&mut entities).schema).push(generated);
    let result = analyzer(&server)
        .infer_data_flow(&scope, &entities)
        .await
        .unwrap();
    assert!(!result.evaluated);
    assert_eq!(result.mappings.len(), 2);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn independent_fields_share_one_request_and_require_complete_answers() {
    let (scope, mut entities) = fixture();
    fields_mut(&mut event(&mut entities).schema).push(field("purchaser_id"));
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 1).await;
    assert!(matches!(
        analyzer(&server).infer_data_flow(&scope, &entities).await,
        Err(JevError::InvalidResponse)
    ));
    let server = MockServer::start().await;
    let mut complete = answer(0.99);
    complete["answers"]["q1"] = complete["answers"]["q0"].clone();
    mount(&server, complete, 1).await;
    let result = analyzer(&server)
        .infer_data_flow(&scope, &entities)
        .await
        .unwrap();
    assert_eq!(result.mappings.len(), 2);
    assert!(result
        .mappings
        .iter()
        .all(|mapping| mapping.source.is_some()));
    let requests = server.received_requests().await.unwrap();
    let request: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(request["questions"].as_object().unwrap().len(), 2);
}

#[tokio::test]
async fn type_and_cardinality_mismatches_are_never_candidates() {
    let server = MockServer::start().await;
    let analyzer = analyzer(&server);
    let (scope, mut entities) = fixture();
    fields_mut(&mut command(&mut entities).schema)[0].name = "customer_id".into();
    fields_mut(&mut command(&mut entities).schema)[0].r#type = Some(pb::FieldType {
        kind: Some(pb::field_type::Kind::Int(pb::field_type::IntType {})),
    });
    assert!(matches!(
        analyzer.infer_data_flow(&scope, &entities).await,
        Err(JevError::UnknownSource)
    ));
    fields_mut(&mut command(&mut entities).schema)[0] = field("customer_id");
    fields_mut(&mut command(&mut entities).schema)[0].repeated = true;
    assert!(matches!(
        analyzer.infer_data_flow(&scope, &entities).await,
        Err(JevError::UnknownSource)
    ));
    fields_mut(&mut command(&mut entities).schema)[0].repeated = false;
    fields_mut(&mut command(&mut entities).schema)[0].optional = true;
    assert!(matches!(
        analyzer.infer_data_flow(&scope, &entities).await,
        Err(JevError::UnknownSource)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_and_uncertain_answers_trigger_fallback() {
    for (value, expected) in [
        (answer(0.7), JevError::Uncertain),
        (
            json!({"model":MODEL,"answers":{"q0":{"type":"choice","choice":"unknown","probabilities":{"c0":0.01,"unknown":0.99}}}}),
            JevError::UnknownSource,
        ),
    ] {
        let server = MockServer::start().await;
        mount(&server, value, 1).await;
        let (scope, entities) = fixture();
        assert!(
            matches!(analyzer(&server).infer_data_flow(&scope, &entities).await, Err(error) if error == expected)
        );
    }
}

#[tokio::test]
async fn malformed_or_partial_answers_never_become_mappings() {
    let invalid = vec![
        json!({"model":MODEL,"answers":{}}),
        json!({"model":"another-model","answers":answer(0.99)["answers"]}),
        json!({"model":MODEL,"answers":{"q0":{"type":"choice","choice":"invented","probabilities":{"c0":0.99,"unknown":0.01}}}}),
        json!({"model":MODEL,"answers":{"q0":{"type":"choice","choice":"c0","probabilities":{"c0":0.99}}}}),
        json!({"model":MODEL,"answers":{"q0":{"type":"choice","choice":"c0","probabilities":{"c0":0.99,"unknown":0.5}}}}),
        json!({"model":MODEL,"answers":{"q0":{"type":"choice","choice":"c0","probabilities":{"c0":1.01,"unknown":-0.01}}}}),
        json!({"model":MODEL,"answers":{"q0":{"type":"choice","choice":"c0","probabilities":{"c0":0.01,"unknown":0.99}}}}),
        json!({"model":MODEL,"answers":{"q0":{"type":"score","choice":"c0","probabilities":{"c0":0.99,"unknown":0.01}}}}),
    ];
    for value in invalid {
        let server = MockServer::start().await;
        mount(&server, value, 1).await;
        let (scope, entities) = fixture();
        assert!(matches!(
            analyzer(&server).infer_data_flow(&scope, &entities).await,
            Err(JevError::InvalidResponse)
        ));
    }
}

#[tokio::test]
async fn identical_concurrent_calls_share_a_single_evaluation_and_cache_hit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(answer(0.99))
                .set_delay(Duration::from_millis(30)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let analyzer = analyzer(&server);
    let (scope, entities) = fixture();
    let (first, second) = tokio::join!(
        analyzer.infer_data_flow(&scope, &entities),
        analyzer.infer_data_flow(&scope, &entities)
    );
    assert_eq!(first.unwrap().mappings, second.unwrap().mappings);
    assert!(
        analyzer
            .infer_data_flow(&scope, &entities)
            .await
            .unwrap()
            .evaluated
    );
}

#[tokio::test]
async fn changed_evidence_invalidates_cached_evaluation() {
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 3).await;
    let analyzer = analyzer(&server);
    let (scope, mut entities) = fixture();
    analyzer.infer_data_flow(&scope, &entities).await.unwrap();
    event(&mut entities).doc = "Changed business meaning".into();
    analyzer.infer_data_flow(&scope, &entities).await.unwrap();
    fields_mut(&mut command(&mut entities).schema)[0].doc = "Changed field evidence".into();
    analyzer.infer_data_flow(&scope, &entities).await.unwrap();
}

#[tokio::test]
async fn changed_derived_annotation_invalidates_cached_mappings() {
    let server = MockServer::start().await;
    let mut response = answer(0.99);
    response["answers"]["q0"]["probabilities"]["c1"] = json!(0.0);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;
    let analyzer = analyzer(&server);
    let (scope, mut entities) = fixture();
    fields_mut(&mut command(&mut entities).schema).push(field("order_id"));
    fields_mut(&mut event(&mut entities).schema).push(field("order_id"));
    let before = analyzer.infer_data_flow(&scope, &entities).await.unwrap();
    assert!(matches!(
        before.mappings[1].source,
        Some(pb::inferred_field_mapping::Source::FromField(_))
    ));
    fields_mut(&mut event(&mut entities).schema)[1]
        .metadata
        .push(prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation"
                .into(),
            value: Vec::new(),
        });
    let after = analyzer.infer_data_flow(&scope, &entities).await.unwrap();
    assert_eq!(
        after.mappings[1].source,
        Some(pb::inferred_field_mapping::Source::Origin(
            pb::InferredOrigin::Derived as i32
        ))
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[test]
fn extreme_configuration_returns_error_before_runtime_construction() {
    let mut config = JevConfig::new("test-key".into());
    config.timeout = Duration::MAX;
    assert!(matches!(
        JevAnalyzer::new(config),
        Err(JevError::Configuration)
    ));
    let mut config = JevConfig::new("test-key".into());
    config.max_concurrency = usize::MAX;
    assert!(matches!(
        JevAnalyzer::new(config),
        Err(JevError::Configuration)
    ));
}

#[tokio::test]
async fn timeout_includes_provider_latency() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(answer(0.99))
                .set_delay(Duration::from_millis(200)),
        )
        .mount(&server)
        .await;
    let mut config = JevConfig::new("test-key".into());
    config.base_url = server.uri();
    config.timeout = Duration::from_millis(20);
    let analyzer = JevAnalyzer::new(config).unwrap();
    let (scope, entities) = fixture();
    assert!(matches!(
        analyzer.infer_data_flow(&scope, &entities).await,
        Err(JevError::Timeout)
    ));
}

#[tokio::test]
async fn timeout_includes_the_concurrency_queue() {
    let server = MockServer::start().await;
    let mut config = JevConfig::new("test-key".into());
    config.base_url = server.uri();
    config.timeout = Duration::from_millis(20);
    config.max_concurrency = 1;
    let analyzer = JevAnalyzer::new(config).unwrap();
    let _occupied = analyzer.slots.acquire().await.unwrap();
    let (scope, entities) = fixture();
    assert!(matches!(
        analyzer.infer_data_flow(&scope, &entities).await,
        Err(JevError::Timeout)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn cache_expiration_and_eviction_re_evaluate() {
    let server = MockServer::start().await;
    mount(&server, answer(0.99), 4).await;
    let mut config = JevConfig::new("test-key".into());
    config.base_url = server.uri();
    config.cache_capacity = 1;
    config.cache_ttl = Duration::from_millis(15);
    let analyzer = JevAnalyzer::new(config).unwrap();
    let (scope, entities) = fixture();
    analyzer.infer_data_flow(&scope, &entities).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    analyzer.infer_data_flow(&scope, &entities).await.unwrap();
    let mut other = entities.clone();
    event(&mut other).doc = "different meaning".into();
    analyzer.infer_data_flow(&scope, &other).await.unwrap();
    analyzer.infer_data_flow(&scope, &entities).await.unwrap();
    assert_eq!(analyzer.cache.lock().await.len(), 1);
}

#[tokio::test]
async fn request_and_response_size_are_bounded() {
    let server = MockServer::start().await;
    let (scope, mut entities) = fixture();
    event(&mut entities).doc = "x".repeat(MAX_REQUEST_BYTES);
    assert!(matches!(
        analyzer(&server).infer_data_flow(&scope, &entities).await,
        Err(JevError::BudgetExceeded)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
    event(&mut entities).doc.clear();
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("x".repeat(MAX_RESPONSE_BYTES + 1)),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        analyzer(&server).infer_data_flow(&scope, &entities).await,
        Err(JevError::InvalidResponse)
    ));
}

#[tokio::test]
async fn candidate_budget_does_not_silently_drop_evidence() {
    let server = MockServer::start().await;
    let (scope, mut entities) = fixture();
    *fields_mut(&mut command(&mut entities).schema) = (0..=MAX_CANDIDATES)
        .map(|index| field(&format!("source_{index}")))
        .collect();
    assert!(matches!(
        analyzer(&server).infer_data_flow(&scope, &entities).await,
        Err(JevError::BudgetExceeded)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn unsupported_or_malformed_schema_never_claims_empty_success() {
    let server = MockServer::start().await;
    let (scope, mut entities) = fixture();
    for (type_url, value) in [
        ("type.googleapis.com/example.Custom", Vec::new()),
        (
            "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema",
            vec![0xff],
        ),
    ] {
        event(&mut entities).schema = Some(prost_types::Any {
            type_url: type_url.into(),
            value,
        });
        assert!(matches!(
            analyzer(&server).infer_data_flow(&scope, &entities).await,
            Err(JevError::UnsupportedSchema)
        ));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gateway_errors_and_redirects_are_stable_and_do_not_include_body() {
    for status in [302, 401, 429, 500] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string("sensitive-provider-response")
                    .insert_header("location", "https://example.com/"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (scope, entities) = fixture();
        let result = analyzer(&server).infer_data_flow(&scope, &entities).await;
        assert!(matches!(result, Err(JevError::GatewayUnavailable)));
        assert_eq!(result.err().unwrap().reason(), "jev_gateway_unavailable");
    }
}

#[test]
fn configuration_rejects_unsafe_urls_and_redacts_credentials() {
    let secret = "never-print-this-key";
    let config = JevConfig::new(secret.into());
    assert!(!format!("{config:?}").contains(secret));
    for url in [
        "http://example.com",
        "https://user:password@example.com",
        "https://example.com?token=secret",
        "https://example.com/v1",
    ] {
        let mut config = config.clone();
        config.base_url = url.into();
        assert!(matches!(
            JevAnalyzer::new(config),
            Err(JevError::Configuration)
        ));
    }
}
