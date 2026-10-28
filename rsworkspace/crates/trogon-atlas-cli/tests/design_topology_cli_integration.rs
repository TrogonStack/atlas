#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}
fn cli(endpoint: &str, args: &[&str]) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

const CALLEE: &str = r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {namespace: topology-surfaces, name: callee}
spec:
  title: {value: Callee}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  steps: [{title: {value: Work}}]
";

fn flow(ids: Option<(&str, &str)>) -> String {
    let (start, call, transition) = match ids {
        Some((start, call)) => (
            format!("stepId: {{value: {start}}}, "),
            format!("stepId: {{value: {call}}}, "),
            format!(
                "transitions: [{{label: {{value: next}}, targetStepId: {{value: {call}}}}}, {{label: {{value: next}}, targetStepId: {{value: {call}}}}}], "
            ),
        ),
        None => (String::new(), String::new(), String::new()),
    };
    let entry = ids.map_or(String::new(), |(start, _)| {
        format!("  entrySteps: [{{value: {start}}}]\n")
    });
    format!(
        r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {{namespace: topology-surfaces, name: flow}}
spec:
  title: {{value: Flow}}
  profile: DESIGN_GRAPH_PROFILE_FLOW
{entry}  steps:
    - {{{start}{transition}title: {{value: Start}}}}
    - {{{call}title: {{value: Call}}, target: {{subflow: {{id: {{namespace: topology-surfaces, slug: callee, version: 1}}}}}}}}
"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn design_topology_cli_two_phase_apply_get_and_query() {
    let endpoint = start_server().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("topology.yaml");
    let apply = |manifest: &str| {
        std::fs::write(&path, manifest).unwrap();
        cli(&endpoint, &["apply", "-f", path.to_str().unwrap()]);
    };
    apply(&format!("{CALLEE}---\n{}", flow(None)));
    let exported = cli(&endpoint, &["get", "design_graph/topology-surfaces/flow@1"]);
    let parsed = trogon_atlas_client::manifest::parse_str(&exported, "export.yaml").unwrap();
    let Some(trogon_atlas_proto::entity::Kind::DesignGraph(graph)) = &parsed[0].entity.kind else {
        panic!("design graph missing");
    };
    let start = graph.steps[0].step_id.clone().unwrap().value;
    let call = graph.steps[1].step_id.clone().unwrap().value;
    assert!(!start.is_empty() && start != call);
    apply(&format!("{CALLEE}---\n{}", flow(Some((&start, &call)))));
    let after = cli(&endpoint, &["get", "design_graph/topology-surfaces/flow@1"]);
    let reapplied = trogon_atlas_client::manifest::parse_str(&after, "export.yaml").unwrap();
    let Some(trogon_atlas_proto::entity::Kind::DesignGraph(graph)) = &reapplied[0].entity.kind
    else {
        panic!("design graph missing");
    };
    let transitions: Vec<_> = graph.steps[0]
        .transitions
        .iter()
        .map(|t| t.connection_id.clone().unwrap().value)
        .collect();
    assert_eq!(transitions.len(), 2);
    assert_ne!(transitions[0], transitions[1]);
    apply(&format!("{CALLEE}---\n{}", flow(Some((&start, &call)))));
    assert_eq!(
        after,
        cli(&endpoint, &["get", "design_graph/topology-surfaces/flow@1"])
    );

    let id = |slug| serde_json::json!({"namespace":"topology-surfaces","slug":slug,"version":"1"});
    let subject = serde_json::json!({"entity":{"kind":"ENTITY_KIND_DESIGN_GRAPH","id":id("flow")}});
    for (command, request) in [
        (
            "get-design-graph-detail",
            serde_json::json!({"graph":{"id":id("flow")}}),
        ),
        (
            "list-subject-design-graphs",
            serde_json::json!({"subject":subject}),
        ),
        (
            "traverse-design-graph-topology",
            serde_json::json!({"root":{"id":id("flow")},"followSubflows":true}),
        ),
        (
            "list-incoming-design-graph-subflows",
            serde_json::json!({"graph":{"id":id("callee")}}),
        ),
    ] {
        let response = cli(&endpoint, &[command, "--request", &request.to_string()]);
        assert!(
            response.contains("scope") || response.contains("entity"),
            "{command}: {response}"
        );
    }
}

#[test]
fn malformed_design_topology_query_fails_before_connecting() {
    for (command, request) in [
        ("get-design-graph-detail", "{not json"),
        ("traverse-design-graph-topology", r#"{"maxDepth":"deep"}"#),
        ("list-incoming-design-graph-subflows", r#"{"unknown":true}"#),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
            .args([
                "--endpoint",
                "http://127.0.0.1:1",
                command,
                "--request",
                request,
            ])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(5), "{command}");
    }
}
