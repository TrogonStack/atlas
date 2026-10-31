//! `ClassifyImpact` through the public RPC: breaking versus bump-only per
//! dependent, compared with the manual GetImpact + DiffEntities procedure in
//! `docs/how-to/assess-change-impact.md`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeMap, sync::Arc};

use prost::Message as _;
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
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ACME_TOKEN: &str = "impact-acme-token";
const BETA_TOKEN: &str = "impact-beta-token";
const UNBOUND_TOKEN: &str = "impact-unbound-token";

const NS: &str = "shop";

use pb::{DependentClassification as C, ImpactIncompleteness as I};

fn registry() -> TokenRegistry {
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

[principals.unbound-writer]
role = "writer"
tokens = ["{UNBOUND_TOKEN}"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

struct Client {
    inner: EventModelServiceClient<Channel>,
    token: &'static str,
    branch: Option<String>,
}

impl Client {
    fn as_token(&mut self, token: &'static str) -> &mut Self {
        self.token = token;
        self
    }

    fn on_branch(&mut self, branch: Option<&str>) -> &mut Self {
        self.branch = branch.map(str::to_owned);
        self
    }

    fn request<T>(&self, body: T) -> tonic::Request<T> {
        let mut req = tonic::Request::new(body);
        req.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {}", self.token)).unwrap(),
        );
        if let Some(branch) = &self.branch {
            req.metadata_mut().insert(
                "x-trogon-atlas-branch",
                MetadataValue::try_from(branch.as_str()).unwrap(),
            );
        }
        req
    }

    async fn put(&mut self, entity: pb::Entity) {
        let req = self.request(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        });
        self.inner.put_entity(req).await.expect("put entity");
    }

    async fn classify_page(
        &mut self,
        current: pb::EntityRef,
        candidate: pb::EntityRef,
        max_depth: u32,
        page_size: i32,
        page_token: String,
    ) -> Result<pb::ClassifyImpactResponse, tonic::Status> {
        let req = self.request(pb::ClassifyImpactRequest {
            current: Some(current),
            candidate: Some(candidate),
            max_depth,
            page_size,
            page_token,
        });
        self.inner
            .classify_impact(req)
            .await
            .map(tonic::Response::into_inner)
    }

    /// Every page, with the dependents of later pages appended to the first.
    async fn classify_with(
        &mut self,
        current: pb::EntityRef,
        candidate: pb::EntityRef,
        max_depth: u32,
        page_size: i32,
    ) -> pb::ClassifyImpactResponse {
        let mut first: Option<pb::ClassifyImpactResponse> = None;
        let mut token = String::new();
        loop {
            let mut page = self
                .classify_page(
                    current.clone(),
                    candidate.clone(),
                    max_depth,
                    page_size,
                    token,
                )
                .await
                .expect("classify impact");
            token = std::mem::take(&mut page.next_page_token);
            match first.as_mut() {
                None => first = Some(page),
                Some(first) => first.dependents.extend(page.dependents),
            }
            if token.is_empty() {
                break;
            }
        }
        first.unwrap()
    }

    async fn classify(
        &mut self,
        current: pb::EntityRef,
        candidate: pb::EntityRef,
    ) -> pb::ClassifyImpactResponse {
        self.classify_with(current, candidate, 0, 0).await
    }
}

async fn client_with(cap: Option<usize>) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut svc =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    if let Some(cap) = cap {
        svc = svc.with_snapshot_cap(cap);
    }
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(svc)),
        BearerAuth::new(Arc::new(registry())),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Client {
        inner: EventModelServiceClient::connect(format!("http://{addr}"))
            .await
            .unwrap(),
        token: UNBOUND_TOKEN,
        branch: None,
    }
}

async fn client() -> Client {
    client_with(None).await
}

fn id_in(namespace: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version,
    }
}

fn id(slug: &str, version: u64) -> pb::Id {
    id_in(NS, slug, version)
}

fn eref(kind: pb::EntityKind, slug: &str, version: u64) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(slug, version)),
    }
}

fn field(name: &str) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType {
            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
        }),
        ..Default::default()
    }
}

fn optional(name: &str) -> pb::FieldSpec {
    pb::FieldSpec {
        optional: true,
        ..field(name)
    }
}

fn fields(names: &[&str]) -> Vec<pb::FieldSpec> {
    names.iter().map(|name| field(name)).collect()
}

fn event_in(id: pb::Id, schema: Vec<pb::FieldSpec>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            title: id.slug.clone(),
            id: Some(id),
            schema: Some(trogon_atlas_core::schema::pack_fields(schema)),
            ..Default::default()
        })),
    }
}

fn event(slug: &str, version: u64, schema: Vec<pb::FieldSpec>) -> pb::Entity {
    event_in(id(slug, version), schema)
}

fn superseding_event(slug: &str, version: u64, schema: Vec<pb::FieldSpec>) -> pb::Entity {
    let mut entity = event(slug, version, schema);
    if let Some(pb::entity::Kind::Event(e)) = entity.kind.as_mut() {
        e.supersedes = Some(id(slug, version - 1));
    }
    entity
}

fn command(slug: &str, version: u64, schema: Vec<pb::FieldSpec>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Command(pb::Command {
            id: Some(id(slug, version)),
            title: slug.into(),
            schema: Some(trogon_atlas_core::schema::pack_fields(schema)),
            ..Default::default()
        })),
    }
}

fn read_model(
    slug: &str,
    version: u64,
    schema: Vec<pb::FieldSpec>,
    source_events: &[(&str, u64)],
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id(slug, version)),
            title: slug.into(),
            schema: Some(trogon_atlas_core::schema::pack_fields(schema)),
            source_events: source_events
                .iter()
                .map(|(slug, version)| pb::EventRef {
                    id: Some(id(slug, *version)),
                })
                .collect(),
            ..Default::default()
        })),
    }
}

fn event_edge(slug: &str, version: u64) -> pb::EventEdge {
    pb::EventEdge {
        event: Some(pb::EventRef {
            id: Some(id(slug, version)),
        }),
        ..Default::default()
    }
}

fn read_model_edge(slug: &str, version: u64) -> pb::ReadModelEdge {
    pb::ReadModelEdge {
        read_model: Some(pb::ReadModelRef {
            id: Some(id(slug, version)),
        }),
        ..Default::default()
    }
}

fn command_slice_in(slice: pb::Id, command: (&str, u64), emitted: &[(&str, u64)]) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            title: slice.slug.clone(),
            id: Some(slice),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id(command.0, command.1)),
                }),
                ..Default::default()
            }),
            emitted_events: emitted
                .iter()
                .map(|(slug, version)| event_edge(slug, *version))
                .collect(),
            ..Default::default()
        })),
    }
}

fn command_slice(slug: &str, command: (&str, u64), emitted: &[(&str, u64)]) -> pb::Entity {
    command_slice_in(id(slug, 1), command, emitted)
}

fn struct_payload(keys: &[&str]) -> prost_types::Any {
    let value = prost_types::Struct {
        fields: keys
            .iter()
            .map(|key| {
                (
                    (*key).to_owned(),
                    prost_types::Value {
                        kind: Some(prost_types::value::Kind::StringValue("x".into())),
                    },
                )
            })
            .collect(),
    };
    prost_types::Any {
        type_url: "type.googleapis.com/google.protobuf.Struct".into(),
        value: value.encode_to_vec(),
    }
}

fn projection_slice(
    slug: &str,
    source_events: &[(&str, u64)],
    read_model: (&str, u64),
    when: Vec<pb::EventExample>,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
            id: Some(id(slug, 1)),
            title: slug.into(),
            source_events: source_events
                .iter()
                .map(|(slug, version)| event_edge(slug, *version))
                .collect(),
            read_model: Some(read_model_edge(read_model.0, read_model.1)),
            scenarios: if when.is_empty() {
                Vec::new()
            } else {
                vec![pb::ReadModelScenario {
                    id: "s1".into(),
                    title: "projects".into(),
                    when,
                    then: Some(pb::ReadModelExample {
                        read_model: Some(pb::ReadModelRef {
                            id: Some(id(read_model.0, read_model.1)),
                        }),
                        payload: None,
                    }),
                    ..Default::default()
                }]
            },
            ..Default::default()
        })),
    }
}

fn ui_slice(slug: &str, read_models: &[(&str, u64)]) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::UiSlice(pb::UiSlice {
            id: Some(id(slug, 1)),
            title: slug.into(),
            source_read_models: read_models
                .iter()
                .map(|(slug, version)| read_model_edge(slug, *version))
                .collect(),
            ..Default::default()
        })),
    }
}

fn event_model(slug: &str, members: Vec<pb::EntityRef>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(slug, 1)),
            title: slug.into(),
            members,
            ..Default::default()
        })),
    }
}

fn dependent<'a>(response: &'a pb::ClassifyImpactResponse, slug: &str) -> &'a pb::DependentImpact {
    response
        .dependents
        .iter()
        .find(|d| {
            d.dependent
                .as_ref()
                .and_then(|e| e.id.as_ref())
                .is_some_and(|id| id.slug == slug)
        })
        .unwrap_or_else(|| panic!("{slug} is not a dependent: {response:#?}"))
}

fn slugs(response: &pb::ClassifyImpactResponse) -> Vec<String> {
    let mut slugs: Vec<String> = response
        .dependents
        .iter()
        .filter_map(|d| Some(d.dependent.as_ref()?.id.as_ref()?.slug.clone()))
        .collect();
    slugs.sort();
    slugs
}

fn classification(d: &pb::DependentImpact) -> C {
    C::try_from(d.classification).unwrap()
}

fn occurrence_sites(d: &pb::DependentImpact) -> Vec<&str> {
    d.occurrences
        .iter()
        .map(|o| o.reference.as_ref().unwrap().from_field.as_str())
        .collect()
}

fn causal_paths(o: &pb::ConsumedOccurrence) -> Vec<&str> {
    o.causal_ops.iter().map(|op| op.path.as_str()).collect()
}

fn reasons(d: &pb::DependentImpact) -> Vec<pb::ClassificationUnknownReason> {
    d.unknown_reasons
        .iter()
        .map(|r| pb::ClassificationUnknownReason::try_from(*r).unwrap())
        .collect()
}

fn incompleteness(response: &pb::ClassifyImpactResponse) -> Vec<pb::ImpactIncompleteness> {
    response
        .incompleteness
        .iter()
        .map(|r| pb::ImpactIncompleteness::try_from(*r).unwrap())
        .collect()
}

fn verdict(response: &pb::ClassifyImpactResponse) -> C {
    C::try_from(response.verdict).unwrap()
}

fn placed(version: u64) -> pb::EntityRef {
    eref(pb::EntityKind::Event, "placed", version)
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_consumed_field_breaks_the_projection_that_reads_it() {
    let mut c = client().await;
    c.put(event("placed", 1, fields(&["order_id", "amount", "note"])))
        .await;
    c.put(superseding_event(
        "placed",
        2,
        fields(&["order_id", "note"]),
    ))
    .await;
    c.put(read_model(
        "totals",
        1,
        fields(&["order_id", "amount"]),
        &[("placed", 1)],
    ))
    .await;

    let response = c.classify(placed(1), placed(2)).await;

    assert_eq!(verdict(&response), C::Breaking);
    assert_eq!(incompleteness(&response), Vec::<I>::new());
    let totals = dependent(&response, "totals");
    assert_eq!(classification(totals), C::Breaking);
    let occurrence = &totals.occurrences[0];
    assert_eq!(
        occurrence.reference.as_ref().unwrap().from_field,
        "source_events[0]"
    );
    assert_eq!(
        occurrence.consumed_paths,
        ["schema.fields[amount]", "schema.fields[order_id]"]
    );
    assert_eq!(causal_paths(occurrence), ["schema.fields[amount]"]);
    assert_eq!(
        occurrence.causal_ops[0].kind,
        pb::DiffOpKind::Removed as i32
    );
    assert_eq!(response.counts.as_ref().unwrap().breaking, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_unused_field_is_bump_only_and_the_candidate_is_not_a_dependent() {
    let mut c = client().await;
    c.put(event("placed", 1, fields(&["order_id", "amount", "note"])))
        .await;
    c.put(superseding_event(
        "placed",
        2,
        fields(&["order_id", "amount"]),
    ))
    .await;
    c.put(read_model(
        "totals",
        1,
        fields(&["order_id", "amount"]),
        &[("placed", 1)],
    ))
    .await;

    let response = c.classify(placed(1), placed(2)).await;

    assert_eq!(
        slugs(&response),
        ["totals"],
        "the superseding candidate is excluded"
    );
    assert_eq!(verdict(&response), C::BumpOnly);
    let totals = dependent(&response, "totals");
    assert_eq!(classification(totals), C::BumpOnly);
    assert_eq!(totals.occurrences[0].causal_ops.len(), 0);
    let mut paths: Vec<&str> = response.ops.iter().map(|op| op.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["schema.fields[note]", "supersedes"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn additions_break_only_the_consumers_that_must_handle_them() {
    let mut c = client().await;
    c.put(event("placed", 1, fields(&["order_id"]))).await;
    c.put(event("placed", 2, fields(&["order_id", "customer"])))
        .await;
    c.put(command("place", 1, fields(&["order_id", "customer"])))
        .await;
    c.put(command("import", 1, fields(&["order_id"]))).await;
    c.put(command_slice("place-order", ("place", 1), &[("placed", 1)]))
        .await;
    c.put(command_slice(
        "import-order",
        ("import", 1),
        &[("placed", 1)],
    ))
    .await;
    c.put(read_model(
        "totals",
        1,
        fields(&["order_id"]),
        &[("placed", 1)],
    ))
    .await;

    let response = c.classify(placed(1), placed(2)).await;

    assert_eq!(
        classification(dependent(&response, "place-order")),
        C::BumpOnly,
        "the command already carries the added field"
    );
    let import = dependent(&response, "import-order");
    assert_eq!(
        classification(import),
        C::Breaking,
        "nothing sources the new field"
    );
    assert_eq!(
        causal_paths(&import.occurrences[0]),
        ["schema.fields[customer]"]
    );
    assert_eq!(
        classification(dependent(&response, "totals")),
        C::Breaking,
        "a projection must project every field of its source event"
    );

    c.put(command(
        "place",
        2,
        vec![field("order_id"), field("customer"), optional("coupon")],
    ))
    .await;
    let response = c
        .classify(
            eref(pb::EntityKind::Command, "place", 1),
            eref(pb::EntityKind::Command, "place", 2),
        )
        .await;
    let place = dependent(&response, "place-order");
    assert_eq!(classification(place), C::BumpOnly, "{place:#?}");
    assert_eq!(
        place.occurrences[0].consumed_paths,
        ["schema.fields[order_id]"],
        "the slice reads only what its emitted events carry"
    );
    assert_eq!(verdict(&response), C::BumpOnly);
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_references_report_every_occurrence_at_its_pinned_version() {
    let mut c = client().await;
    c.put(event("placed", 1, fields(&["order_id", "amount"])))
        .await;
    c.put(event("placed", 2, fields(&["order_id"]))).await;
    c.put(event("paid", 1, fields(&["order_id"]))).await;
    c.put(read_model(
        "totals",
        1,
        fields(&["order_id"]),
        &[("placed", 1), ("paid", 1)],
    ))
    .await;
    c.put(projection_slice(
        "project-totals",
        &[("placed", 1), ("paid", 1), ("placed", 1)],
        ("totals", 1),
        vec![
            pb::EventExample {
                event: Some(pb::EventRef {
                    id: Some(id("placed", 1)),
                }),
                payload: Some(struct_payload(&["order_id", "amount"])),
            },
            pb::EventExample {
                event: Some(pb::EventRef {
                    id: Some(id("placed", 1)),
                }),
                payload: None,
            },
        ],
    ))
    .await;
    c.put(read_model(
        "archive",
        1,
        fields(&["order_id"]),
        &[("placed", 2)],
    ))
    .await;
    c.put(projection_slice(
        "already-moved",
        &[("placed", 2)],
        ("archive", 1),
        Vec::new(),
    ))
    .await;

    let response = c.classify(placed(1), placed(2)).await;

    assert!(
        !slugs(&response).contains(&"already-moved".to_owned()),
        "a referrer of another version is not a dependent"
    );
    let slice = dependent(&response, "project-totals");
    assert_eq!(
        occurrence_sites(slice),
        [
            "source_events[0]",
            "source_events[2]",
            "scenarios[0].when[0].event",
            "scenarios[0].when[1].event",
        ]
    );
    for occurrence in &slice.occurrences {
        assert_eq!(
            occurrence.reference.as_ref().unwrap().to.as_ref().unwrap(),
            &placed(1),
            "each occurrence keeps its pinned target"
        );
    }
    let by_site: BTreeMap<&str, C> = slice
        .occurrences
        .iter()
        .map(|o| {
            (
                o.reference.as_ref().unwrap().from_field.as_str(),
                C::try_from(o.classification).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        by_site["source_events[0]"],
        C::BumpOnly,
        "totals never read amount"
    );
    assert_eq!(by_site["source_events[2]"], C::BumpOnly);
    assert_eq!(
        by_site["scenarios[0].when[0].event"],
        C::Breaking,
        "the example payload names amount"
    );
    assert_eq!(
        by_site["scenarios[0].when[1].event"],
        C::BumpOnly,
        "no payload"
    );
    assert_eq!(classification(slice), C::Breaking);
}

/// Event `placed` -> read models -> UI slices -> event model, plus an
/// outgoing reference from `placed` to its swimlane.
async fn layered(c: &mut Client) {
    c.put(pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
            id: Some(id("orders", 1)),
            title: "orders".into(),
            ..Default::default()
        })),
    })
    .await;
    let mut placed_v1 = event("placed", 1, fields(&["order_id", "amount", "note"]));
    if let Some(pb::entity::Kind::Event(e)) = placed_v1.kind.as_mut() {
        e.swimlane = Some(pb::SwimlaneRef {
            id: Some(id("orders", 1)),
        });
    }
    c.put(placed_v1.clone()).await;
    let mut placed_v2 = placed_v1;
    if let Some(pb::entity::Kind::Event(e)) = placed_v2.kind.as_mut() {
        e.id = Some(id("placed", 2));
        e.schema = Some(trogon_atlas_core::schema::pack_fields(fields(&[
            "order_id", "note",
        ])));
    }
    c.put(placed_v2).await;
    c.put(read_model(
        "totals",
        1,
        fields(&["order_id", "amount"]),
        &[("placed", 1)],
    ))
    .await;
    c.put(read_model(
        "ledger",
        1,
        fields(&["order_id", "note"]),
        &[("placed", 1)],
    ))
    .await;
    c.put(ui_slice("dashboard", &[("totals", 1)])).await;
    c.put(ui_slice("ledger-view", &[("ledger", 1)])).await;
    c.put(event_model(
        "model",
        vec![eref(pb::EntityKind::UiSlice, "ledger-view", 1)],
    ))
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transitive_dependents_inherit_through_the_dependent_they_reference() {
    let mut c = client().await;
    layered(&mut c).await;

    let response = c.classify(placed(1), placed(2)).await;

    assert_eq!(
        slugs(&response),
        ["dashboard", "ledger", "ledger-view", "model", "totals"],
        "the swimlane placed points at is not a dependent"
    );
    assert_eq!(classification(dependent(&response, "totals")), C::Breaking);
    assert_eq!(classification(dependent(&response, "ledger")), C::BumpOnly);
    let dashboard = dependent(&response, "dashboard");
    assert_eq!(dashboard.depth, 2);
    assert_eq!(classification(dashboard), C::Unknown);
    assert_eq!(
        reasons(dashboard),
        [pb::ClassificationUnknownReason::TransitiveThroughAffected]
    );
    assert_eq!(
        dashboard.through[0].reference.as_ref().unwrap().from_field,
        "source_read_models[0]"
    );
    assert_eq!(
        dashboard.through[0].through_classification,
        C::Breaking as i32
    );
    assert_eq!(
        classification(dependent(&response, "ledger-view")),
        C::BumpOnly
    );
    let model = dependent(&response, "model");
    assert_eq!(model.depth, 3);
    assert_eq!(classification(model), C::BumpOnly);
    assert_eq!(verdict(&response), C::Breaking);
}

/// The documented manual procedure walks GetImpact and keeps only nodes
/// reached by incoming hops. ClassifyImpact must name exactly that set, and
/// classify the overlap the procedure computes by hand.
#[tokio::test(flavor = "multi_thread")]
async fn agrees_with_the_manual_procedure_on_which_entities_are_dependents() {
    let mut c = client().await;
    layered(&mut c).await;

    let req = c.request(pb::GetImpactRequest {
        root: Some(placed(1)),
        ..Default::default()
    });
    let impact = c.inner.get_impact(req).await.unwrap().into_inner();
    let mut manual: Vec<String> = impact
        .nodes
        .iter()
        .filter(|node| node.depth > 0)
        .filter(|node| {
            let mut toward = placed(1);
            node.path.iter().all(|hop| {
                let incoming = hop.to.as_ref() == Some(&toward);
                toward = hop.from.clone().unwrap_or_default();
                incoming
            })
        })
        .filter_map(|node| Some(node.entity.as_ref()?.id.as_ref()?.slug.clone()))
        .filter(|slug| slug != "placed")
        .collect();
    manual.sort();
    let outgoing: Vec<&str> = impact
        .nodes
        .iter()
        .filter_map(|node| Some(node.entity.as_ref()?.id.as_ref()?.slug.as_str()))
        .filter(|slug| *slug == "orders")
        .collect();
    assert_eq!(
        outgoing,
        ["orders"],
        "GetImpact also walks outgoing references"
    );

    let response = c.classify(placed(1), placed(2)).await;
    assert_eq!(slugs(&response), manual);

    let req = c.request(pb::DiffEntitiesRequest {
        a: Some(placed(1)),
        b: Some(placed(2)),
    });
    let diff = c.inner.diff_entities(req).await.unwrap().into_inner();
    assert_eq!(
        response.ops, diff.ops,
        "the causal vocabulary is DiffEntities"
    );
    let removed: Vec<&str> = diff
        .ops
        .iter()
        .filter(|op| op.kind == pb::DiffOpKind::Removed as i32)
        .map(|op| op.path.as_str())
        .collect();
    for slug in ["totals", "ledger"] {
        let d = dependent(&response, slug);
        let overlap = d.occurrences[0]
            .consumed_paths
            .iter()
            .any(|path| removed.contains(&path.as_str()));
        let expected = if overlap { C::Breaking } else { C::BumpOnly };
        assert_eq!(classification(d), expected, "{slug}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_depth_limit_that_cuts_off_dependents_never_reports_bump_only() {
    let mut c = client().await;
    layered(&mut c).await;

    let response = c.classify_with(placed(1), placed(2), 1, 0).await;

    assert_eq!(slugs(&response), ["ledger", "totals"]);
    assert_eq!(
        incompleteness(&response),
        [pb::ImpactIncompleteness::DepthLimited]
    );
    let ledger = dependent(&response, "ledger");
    assert_eq!(classification(ledger), C::Unknown);
    assert_eq!(
        reasons(ledger),
        [pb::ClassificationUnknownReason::TraversalIncomplete]
    );
    assert_eq!(classification(dependent(&response, "totals")), C::Breaking);
    assert_eq!(verdict(&response), C::Breaking);
    assert_eq!(response.counts.as_ref().unwrap().bump_only, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_snapshot_never_reports_bump_only() {
    let mut c = client_with(Some(2)).await;
    c.put(event("placed", 1, fields(&["order_id", "note"])))
        .await;
    c.put(event("placed", 2, fields(&["order_id"]))).await;
    c.put(read_model(
        "ledger",
        1,
        fields(&["order_id"]),
        &[("placed", 1)],
    ))
    .await;
    c.put(read_model(
        "audit",
        1,
        fields(&["order_id"]),
        &[("placed", 1)],
    ))
    .await;

    let response = c.classify(placed(1), placed(2)).await;

    assert!(incompleteness(&response).contains(&pb::ImpactIncompleteness::SnapshotTruncated));
    assert_eq!(verdict(&response), C::Unknown);
    assert!(
        response
            .dependents
            .iter()
            .all(|d| classification(d) != C::BumpOnly),
        "{response:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_dependents_make_the_verdict_unknown_without_naming_them() {
    let mut c = client().await;
    let acme = |slug: &str, version| id_in("acme-orders", slug, version);
    c.as_token(ACME_TOKEN);
    c.put(event_in(acme("placed", 1), fields(&["order_id", "note"])))
        .await;
    c.put(event_in(acme("placed", 2), fields(&["order_id"])))
        .await;
    c.as_token(BETA_TOKEN);
    c.put(event_in(id_in("beta-orders", "claimed", 1), Vec::new()))
        .await;
    c.as_token(UNBOUND_TOKEN);
    c.put(pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id_in("beta-orders", "mirror", 1)),
            title: "mirror".into(),
            schema: Some(trogon_atlas_core::schema::pack_fields(fields(&[
                "order_id",
            ]))),
            source_events: vec![pb::EventRef {
                id: Some(acme("placed", 1)),
            }],
            ..Default::default()
        })),
    })
    .await;
    let acme_ref = |version| pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(acme("placed", version)),
    };

    let everyone = c.classify(acme_ref(1), acme_ref(2)).await;
    assert_eq!(slugs(&everyone), ["mirror"]);
    assert_eq!(verdict(&everyone), C::BumpOnly);
    assert_eq!(incompleteness(&everyone), Vec::<I>::new());

    let owner = c
        .as_token(ACME_TOKEN)
        .classify(acme_ref(1), acme_ref(2))
        .await;
    assert!(owner.dependents.is_empty(), "{owner:#?}");
    assert_eq!(
        incompleteness(&owner),
        [pb::ImpactIncompleteness::HiddenDependents]
    );
    assert_eq!(verdict(&owner), C::Unknown);
}

#[tokio::test(flavor = "multi_thread")]
async fn sites_without_a_consumption_model_are_unknown_unless_only_docs_change() {
    let mut c = client().await;
    c.put(event("placed", 1, fields(&["order_id"]))).await;
    c.put(read_model(
        "totals",
        1,
        fields(&["order_id", "amount"]),
        &[("placed", 1)],
    ))
    .await;
    let mut documented = read_model(
        "totals",
        2,
        fields(&["order_id", "amount"]),
        &[("placed", 1)],
    );
    if let Some(pb::entity::Kind::ReadModel(rm)) = documented.kind.as_mut() {
        rm.doc = "Running totals per order.".into();
    }
    c.put(documented).await;
    let mut rekeyed = read_model(
        "totals",
        3,
        fields(&["order_id", "amount"]),
        &[("placed", 1)],
    );
    if let Some(pb::entity::Kind::ReadModel(rm)) = rekeyed.kind.as_mut() {
        rm.key = vec!["order_id".into()];
    }
    c.put(rekeyed).await;
    c.put(ui_slice("dashboard", &[("totals", 1)])).await;
    let totals = |version| eref(pb::EntityKind::ReadModel, "totals", version);

    let docs_only = c.classify(totals(1), totals(2)).await;
    assert_eq!(
        classification(dependent(&docs_only, "dashboard")),
        C::BumpOnly
    );

    let rekeyed = c.classify(totals(1), totals(3)).await;
    let dashboard = dependent(&rekeyed, "dashboard");
    assert_eq!(classification(dashboard), C::Unknown);
    assert_eq!(
        reasons(dashboard),
        [pb::ClassificationUnknownReason::NoConsumptionModel]
    );
    assert_eq!(rekeyed.uncovered_changes, ["key"]);
    assert_eq!(verdict(&rekeyed), C::Unknown);
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_partition_the_dependents_and_repeat_the_summary() {
    let mut c = client().await;
    layered(&mut c).await;

    let whole = c.classify(placed(1), placed(2)).await;
    let paged = c.classify_with(placed(1), placed(2), 0, 2).await;

    assert_eq!(paged.dependents, whole.dependents);
    assert_eq!(paged.counts, whole.counts);
    assert_eq!(paged.verdict, whole.verdict);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_page_names_the_snapshot_it_was_computed_from() {
    let mut c = client().await;
    layered(&mut c).await;
    let req = c.request(pb::GetSnapshotIdRequest::default());
    let expected = c.inner.get_snapshot_id(req).await.unwrap().into_inner();

    let first = c
        .classify_page(placed(1), placed(2), 0, 2, String::new())
        .await
        .unwrap();
    let second = c
        .classify_page(placed(1), placed(2), 0, 2, first.next_page_token.clone())
        .await
        .unwrap();
    assert_eq!(first.snapshot_id, expected.snapshot_id);
    assert_eq!(second.snapshot_id, first.snapshot_id);

    c.put(event("unrelated", 1, fields(&["id"]))).await;
    let moved = c.classify(placed(1), placed(2)).await;
    assert_ne!(moved.snapshot_id, first.snapshot_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_classifies_against_its_own_merged_view() {
    let mut c = client().await;
    c.put(event("placed", 1, fields(&["order_id", "amount"])))
        .await;
    let req = c.request(pb::CreateBranchRequest {
        name: "drop-amount".into(),
        doc: String::new(),
    });
    c.inner.create_branch(req).await.unwrap();
    c.on_branch(Some("drop-amount"));
    c.put(event("placed", 2, fields(&["order_id"]))).await;
    c.put(read_model(
        "totals",
        1,
        fields(&["order_id", "amount"]),
        &[("placed", 1)],
    ))
    .await;

    let branch = c.classify(placed(1), placed(2)).await;
    assert_eq!(classification(dependent(&branch, "totals")), C::Breaking);

    c.on_branch(None);
    let status = c
        .classify_page(placed(1), placed(2), 0, 0, String::new())
        .await
        .expect_err("the candidate exists only on the branch");
    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_a_candidate_of_another_kind() {
    let mut c = client().await;
    c.put(event("placed", 1, fields(&["order_id"]))).await;
    c.put(command("place", 1, fields(&["order_id"]))).await;

    let status = c
        .classify_page(
            placed(1),
            eref(pb::EntityKind::Command, "place", 1),
            0,
            0,
            String::new(),
        )
        .await
        .expect_err("kinds differ");
    assert_eq!(status.code(), Code::InvalidArgument);
}
