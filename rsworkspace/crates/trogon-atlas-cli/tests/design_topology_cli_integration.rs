#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "topology-cli-admin";
const WRITER: &str = "topology-cli-writer";
const BRANCH: &str = "@topology-cli-owner:topology-edit";
const READER: &str = "topology-cli-reader";

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

async fn authenticated_server() -> (String, String) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "admin":{"role":"admin","tokens":[ADMIN]},
            "writer":{"role":"writer","kind":"agent","tokens":[WRITER],"parent":"topology-cli-owner"},
            "reader":{"role":"reader","kind":"agent","tokens":[READER],"parent":"topology-cli-owner"}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let secured = data_plane(
        EventModelServiceServer::new(service),
        BearerAuth::new(registry),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(secured)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let endpoint = format!("http://{address}");
    let mut admin = trogon_atlas_client::client::connect(&endpoint, Some(ADMIN), 30)
        .await
        .unwrap();
    let namespace = admin
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "topology-cli".into(),
            parent: "topology-cli-owner".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    admin
        .create_branch(pb::CreateBranchRequest {
            name: BRANCH.into(),
            doc: "topology".into(),
        })
        .await
        .unwrap();
    (endpoint, namespace)
}

fn run(endpoint: &str, token: &str, branch: Option<&str>, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"));
    command.args(["--endpoint", endpoint, "--auth-token", token]);
    if let Some(branch) = branch {
        command.args(["--branch", branch]);
    }
    command.args(args).output().unwrap()
}

fn ok(endpoint: &str, token: &str, branch: Option<&str>, args: &[&str]) -> String {
    let output = run(endpoint, token, branch, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn query(
    endpoint: &str,
    token: &str,
    branch: Option<&str>,
    command: &str,
    request: &Value,
) -> Value {
    let text = ok(
        endpoint,
        token,
        branch,
        &[command, "--request", &request.to_string()],
    );
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{command}: {error}: {text}"))
}

fn authenticated_flow(ns: &str, title: &str, ids: Option<(&str, &str, &str)>) -> String {
    let (start, call, transition) = match ids {
        Some((start, call, _)) => (
            format!("stepId: {{value: {start}}}, "),
            format!("stepId: {{value: {call}}}, "),
            format!(
                "transitions: [{{label: {{value: next}}, targetStepId: {{value: {call}}}}}, {{label: {{value: next}}, targetStepId: {{value: {call}}}}}], "
            ),
        ),
        None => (String::new(), String::new(), String::new()),
    };
    let subject = ids.map_or(String::new(), |(_, _, subject)| {
        format!("connectionId: {{value: {subject}}}, ")
    });
    let entry = ids.map_or(String::new(), |(start, _, _)| {
        format!("  entrySteps: [{{value: {start}}}]\n")
    });
    format!(
        r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Component
metadata: {{namespace: {ns}, name: checkout}}
spec: {{title: Checkout}}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {{namespace: {ns}, name: callee}}
spec:
  title: {{value: Callee}}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  steps: [{{title: {{value: Work}}}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {{namespace: {ns}, name: flow}}
spec:
  title: {{value: {title}}}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  subjects: [{{{subject}subject: {{entity: {{kind: ENTITY_KIND_COMPONENT, id: {{namespace: {ns}, slug: checkout, version: 1}}}}}}}}]
{entry}  steps:
    - {{{start}{transition}title: {{value: Start}}, target: {{component: {{kind: ENTITY_KIND_COMPONENT, id: {{namespace: {ns}, slug: checkout, version: 1}}}}}}}}
    - {{{call}title: {{value: Call}}, target: {{subflow: {{id: {{namespace: {ns}, slug: callee, version: 1}}}}}}}}
"
    )
}

fn step_ids(detail: &Value) -> Vec<String> {
    detail["entity"]["designGraph"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["stepId"]["value"].as_str().unwrap().to_owned())
        .collect()
}

fn connection_ids(detail: &Value) -> Vec<String> {
    detail["entity"]["designGraph"]["steps"][0]["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|transition| {
            transition["connectionId"]["value"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_design_topology_cli_returns_exact_queries_exports_without_drift_and_follows_branches(
) {
    let (endpoint, ns) = authenticated_server().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("flow.yaml");
    let path = path.to_str().unwrap();
    let write = |manifest: String| std::fs::write(path, manifest).unwrap();
    let id = |slug: &str| json!({"namespace":ns,"slug":slug,"version":"1"});
    let detail_request = json!({"graph":{"id":id("flow")}});
    let detail = |token: &str, branch: Option<&str>| {
        query(
            &endpoint,
            token,
            branch,
            "get-design-graph-detail",
            &detail_request,
        )
    };

    write(authenticated_flow(&ns, "Flow", None));
    ok(&endpoint, WRITER, None, &["apply", "-f", path]);
    let staged = detail(READER, None);
    let ids = step_ids(&staged);
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);

    let subject_connection = staged["entity"]["designGraph"]["subjects"][0]["connectionId"]
        ["value"]
        .as_str()
        .unwrap()
        .to_owned();
    write(authenticated_flow(
        &ns,
        "Flow",
        Some((&ids[0], &ids[1], &subject_connection)),
    ));
    ok(&endpoint, WRITER, None, &["apply", "-f", path]);
    let wired = detail(READER, None);
    assert_eq!(step_ids(&wired), ids);
    let connections = connection_ids(&wired);
    assert_eq!(connections.len(), 2);
    assert_ne!(connections[0], connections[1]);
    assert_eq!(
        wired["entity"]["designGraph"]["entrySteps"][0]["value"],
        json!(ids[0])
    );

    let targets = wired["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0]["field"], "steps[0].target");
    assert_eq!(targets[1]["field"], "steps[1].target");
    for target in targets {
        assert_eq!(target["state"], "CATALOG_RESOLUTION_STATE_RESOLVED");
        assert!(target["owner"]["uid"]
            .as_str()
            .is_some_and(|uid| !uid.is_empty()));
    }
    assert_eq!(targets[0]["stepId"]["value"], json!(ids[0]));
    assert_eq!(wired["declarations"].as_array().unwrap().len(), 1);
    assert_eq!(wired["declarations"][0]["field"], "subjects[0].subject");
    assert!(
        wired.get("layoutState").is_none(),
        "absent layout stays absent"
    );
    assert!(wired.get("incomplete").is_none());

    let component = json!({"entity":{"kind":"ENTITY_KIND_COMPONENT","id":id("checkout")}});
    let listed = query(
        &endpoint,
        READER,
        None,
        "list-subject-design-graphs",
        &json!({"subject":component}),
    );
    assert_eq!(listed["declarations"].as_array().unwrap().len(), 1);
    assert_eq!(
        listed["declarations"][0]["owner"]["owner"]["id"]["slug"],
        "flow"
    );
    assert_eq!(listed["subjectState"], "CATALOG_RESOLUTION_STATE_RESOLVED");

    let incoming = query(
        &endpoint,
        READER,
        None,
        "list-incoming-design-graph-subflows",
        &json!({"graph":{"id":id("callee")}}),
    );
    assert_eq!(incoming["references"].as_array().unwrap().len(), 1);
    assert_eq!(
        incoming["references"][0]["field"],
        "steps[1].target.subflow"
    );
    assert_eq!(incoming["references"][0]["stepId"]["value"], json!(ids[1]));
    assert_eq!(
        incoming["references"][0]["owner"]["owner"]["id"]["slug"],
        "flow"
    );

    let described = |entity: &str| -> Value {
        let text = ok(
            &endpoint,
            READER,
            None,
            &["--format", "json", "describe", entity],
        );
        serde_json::from_str::<Value>(&text).unwrap()["data"].clone()
    };
    let rows = |described: &Value, side: &str| -> Vec<(String, String, Option<String>)> {
        described[side]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["field"].as_str().unwrap().to_owned(),
                    row["entity"].as_str().unwrap().to_owned(),
                    row["provenance"]["stepId"]["value"]
                        .as_str()
                        .map(str::to_owned),
                )
            })
            .collect()
    };
    let flow = described(&format!("design_graph/{ns}/flow@1"));
    let flow_entity = format!("design_graph:{ns}/flow@1");
    let outgoing = rows(&flow, "outgoing");
    assert_eq!(
        outgoing,
        vec![
            (
                "steps[0].target.component".to_owned(),
                format!("component:{ns}/checkout@1"),
                Some(ids[0].clone())
            ),
            (
                "steps[1].target.subflow".to_owned(),
                format!("design_graph:{ns}/callee@1"),
                Some(ids[1].clone())
            ),
            (
                "subjects[0].subject.entity".to_owned(),
                format!("component:{ns}/checkout@1"),
                None
            ),
            (
                "steps[0].transitions[0]".to_owned(),
                flow_entity.clone(),
                Some(ids[0].clone())
            ),
            (
                "steps[0].transitions[1]".to_owned(),
                flow_entity.clone(),
                Some(ids[0].clone())
            ),
        ],
        "generic discovery lists external, subject and parallel transition rows"
    );
    let owner_uid = flow["outgoing"][0]["provenance"]["fromUid"]
        .as_str()
        .unwrap();
    assert_ne!(owner_uid, "");
    for row in flow["outgoing"].as_array().unwrap() {
        assert_eq!(row["provenance"]["fromUid"], json!(owner_uid));
    }
    for (index, transition) in flow["outgoing"].as_array().unwrap()[3..].iter().enumerate() {
        assert_eq!(
            transition["provenance"]["connectionId"]["value"],
            json!(connections[index])
        );
        assert_eq!(
            transition["provenance"]["targetStepId"]["value"],
            json!(ids[1])
        );
    }
    assert_eq!(
        flow["outgoing"][2]["provenance"]["connectionId"]["value"],
        json!(subject_connection)
    );
    assert_eq!(
        rows(&flow, "incoming").len(),
        2,
        "incoming holds only the two same-owner transition rows"
    );

    let callee = described(&format!("design_graph/{ns}/callee@1"));
    assert_eq!(callee["outgoing"].as_array().unwrap().len(), 0);
    assert_eq!(
        rows(&callee, "incoming"),
        vec![(
            "steps[1].target.subflow".to_owned(),
            flow_entity,
            Some(ids[1].clone())
        )]
    );
    let checkout = described(&format!("component/{ns}/checkout@1"));
    assert_eq!(
        rows(&checkout, "incoming")
            .iter()
            .map(|row| row.0.as_str())
            .collect::<Vec<_>>(),
        ["steps[0].target.component", "subjects[0].subject.entity"]
    );

    let traversed = query(
        &endpoint,
        READER,
        None,
        "traverse-design-graph-topology",
        &json!({"root":{"id":id("flow")},"followSubflows":true}),
    );
    let nodes = traversed["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0]["stepId"]["value"], json!(ids[0]));
    assert_eq!(nodes[1]["stepId"]["value"], json!(ids[1]));
    assert_eq!(nodes[2]["depth"], 1);
    assert_eq!(nodes[2]["graph"]["owner"]["id"]["slug"], "callee");
    let kinds: Vec<_> = traversed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| edge["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["KIND_TRANSITION", "KIND_TRANSITION", "KIND_SUBFLOW"]
    );
    assert_eq!(
        traversed["edges"][0]["connectionId"]["value"],
        json!(connections[0])
    );
    assert_eq!(
        traversed["edges"][1]["connectionId"]["value"],
        json!(connections[1])
    );
    assert_eq!(
        traversed["subflows"][0]["state"],
        "TOPOLOGY_SUBFLOW_STATE_EXPANDED"
    );
    assert!(traversed.get("incomplete").is_none());

    assert!(!run(&endpoint, READER, None, &["apply", "-f", path])
        .status
        .success());

    let exported = directory.path().join("exported");
    ok(
        &endpoint,
        READER,
        None,
        &[
            "export",
            "--namespace",
            &ns,
            "-o",
            exported.to_str().unwrap(),
        ],
    );
    let exported_text: String = std::fs::read_dir(&exported)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    for id in ids.iter().chain(&connections) {
        assert!(exported_text.contains(id.as_str()), "export drops {id}");
    }
    let drift = run(
        &endpoint,
        WRITER,
        None,
        &["diff", "-f", exported.to_str().unwrap()],
    );
    assert_eq!(
        drift.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&drift.stdout)
    );
    ok(
        &endpoint,
        WRITER,
        None,
        &["apply", "-f", exported.to_str().unwrap()],
    );
    assert_eq!(detail(READER, None), wired);

    let mut retitled = 0;
    for entry in std::fs::read_dir(&exported).unwrap() {
        let file = entry.unwrap().path();
        let text = std::fs::read_to_string(&file).unwrap();
        let renamed = text.replace("value: Flow\n", "value: Flow on branch\n");
        retitled += usize::from(renamed != text);
        std::fs::write(&file, renamed).unwrap();
    }
    assert_eq!(retitled, 1);
    ok(
        &endpoint,
        WRITER,
        Some(BRANCH),
        &["apply", "-f", exported.to_str().unwrap()],
    );
    let branched = detail(WRITER, Some(BRANCH));
    assert_eq!(
        branched["entity"]["designGraph"]["title"]["value"],
        "Flow on branch"
    );
    assert_eq!(step_ids(&branched), ids);
    assert_eq!(connection_ids(&branched), connections);
    assert_eq!(
        detail(READER, None)["entity"]["designGraph"]["title"]["value"],
        "Flow"
    );
    let mut admin = trogon_atlas_client::client::connect(&endpoint, Some(ADMIN), 30)
        .await
        .unwrap();
    let diff = admin
        .diff_branch(pb::DiffBranchRequest {
            name: BRANCH.into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1);
    let merged = admin
        .merge_branch(pb::MergeBranchRequest {
            name: BRANCH.into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    let merged = detail(READER, None);
    assert_eq!(
        merged["entity"]["designGraph"]["title"]["value"],
        "Flow on branch"
    );
    assert_eq!(step_ids(&merged), ids);
    assert_eq!(connection_ids(&merged), connections);
}
