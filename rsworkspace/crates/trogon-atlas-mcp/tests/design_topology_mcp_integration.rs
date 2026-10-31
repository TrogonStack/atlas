#![allow(clippy::unwrap_used, clippy::too_many_lines)]
#[allow(clippy::expect_used)]
mod support;
use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServiceRunning =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn start_mcp_client() -> (McpClient, McpServiceRunning, String) {
    let grpc_endpoint = start_server().await;
    let mcp_server = support::mcp_server(&grpc_endpoint, None);
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (mcp_service_result, client_result) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (
        client_result.unwrap(),
        mcp_service_result.unwrap(),
        grpc_endpoint,
    )
}

fn args_obj(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    m
}

fn call_tool_result_text(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

const MANIFEST: &str = r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Component
metadata: {namespace: topology-surfaces, name: checkout}
spec: {title: Checkout}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {namespace: topology-surfaces, name: callee}
spec:
  title: {value: Callee}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  steps: [{title: {value: Work}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {namespace: topology-surfaces, name: flow}
spec:
  title: {value: Flow}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  subjects: [{subject: {entity: {kind: ENTITY_KIND_COMPONENT, id: {namespace: topology-surfaces, slug: checkout, version: 1}}}}]
  steps:
    - title: {value: Start}
      target: {component: {kind: ENTITY_KIND_COMPONENT, id: {namespace: topology-surfaces, slug: checkout, version: 1}}}
    - title: {value: Call}
      target: {subflow: {id: {namespace: topology-surfaces, slug: callee, version: 1}}}
";

#[tokio::test(flavor = "multi_thread")]
async fn design_topology_mcp_apply_inspect_traverse_and_incoming() {
    let (client, mcp_service, _endpoint) = start_mcp_client().await;
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("apply_manifests")
                .with_arguments(args_obj(&[("manifests_yaml", serde_json::json!(MANIFEST))])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let id = |slug| serde_json::json!({"namespace":"topology-surfaces","slug":slug,"version":"1"});
    let call = |tool: &'static str, request: serde_json::Value| {
        let client = &client;
        async move {
            let result = client
                .call_tool(
                    rmcp::model::CallToolRequestParams::new(tool)
                        .with_arguments(args_obj(&[("request_json", request)])),
                )
                .await
                .unwrap();
            assert!(!result.is_error.unwrap_or(false), "{tool}: {result:?}");
            call_tool_result_text(&result)
        }
    };

    let detail = call(
        "get_design_graph_detail",
        serde_json::json!({"graph":{"id":id("flow")}}),
    )
    .await;
    let steps = detail["entity"]["designGraph"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{detail}");
    let start = steps[0]["stepId"]["value"].as_str().unwrap();
    let invoke = steps[1]["stepId"]["value"].as_str().unwrap();
    assert_ne!(start, invoke);
    assert_eq!(detail["entity"]["designGraph"]["title"]["value"], "Flow");
    let targets = detail["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2, "{detail}");
    assert_eq!(targets[0]["field"], "steps[0].target");
    assert_eq!(targets[1]["field"], "steps[1].target");
    for target in targets {
        assert_eq!(target["state"], "CATALOG_RESOLUTION_STATE_RESOLVED");
    }
    assert_eq!(detail["declarations"].as_array().unwrap().len(), 1);
    assert_eq!(detail["declarations"][0]["field"], "subjects[0].subject");
    assert!(detail.get("layoutState").is_none());

    let subject =
        serde_json::json!({"entity":{"kind":"ENTITY_KIND_COMPONENT","id":id("checkout")}});
    let listed = call(
        "list_subject_design_graphs",
        serde_json::json!({"subject":subject}),
    )
    .await;
    assert_eq!(
        listed["declarations"].as_array().unwrap().len(),
        1,
        "{listed}"
    );
    assert_eq!(
        listed["declarations"][0]["owner"]["owner"]["id"]["slug"],
        "flow"
    );
    assert_eq!(listed["subjectState"], "CATALOG_RESOLUTION_STATE_RESOLVED");

    let target_only = call(
        "list_subject_design_graphs",
        serde_json::json!({"subject":{"entity":{"kind":"ENTITY_KIND_DESIGN_GRAPH","id":id("callee")}}}),
    )
    .await;
    assert!(target_only.get("declarations").is_none(), "{target_only}");

    let traversal = call(
        "traverse_design_graph_topology",
        serde_json::json!({"root":{"id":id("flow")},"followSubflows":true}),
    )
    .await;
    let nodes = traversal["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3, "{traversal}");
    assert_eq!(nodes[0]["stepId"]["value"], start);
    assert_eq!(nodes[1]["stepId"]["value"], invoke);
    assert_eq!(nodes[2]["depth"], 1);
    assert_eq!(nodes[2]["graph"]["owner"]["id"]["slug"], "callee");
    let edges = traversal["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1, "{traversal}");
    assert_eq!(edges[0]["kind"], "KIND_SUBFLOW");
    assert_eq!(
        traversal["subflows"][0]["state"],
        "TOPOLOGY_SUBFLOW_STATE_EXPANDED"
    );

    let incoming = call(
        "list_incoming_design_graph_subflows",
        serde_json::json!({"graph":{"id":id("callee")}}),
    )
    .await;
    let references = incoming["references"].as_array().unwrap();
    assert_eq!(references.len(), 1, "{incoming}");
    assert_eq!(references[0]["field"], "steps[1].target.subflow");
    assert_eq!(references[0]["stepId"]["value"], invoke);
    assert_eq!(references[0]["owner"]["owner"]["id"]["slug"], "flow");
    let malformed = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_design_graph_detail").with_arguments(
                args_obj(&[("request_json", serde_json::json!({"graph":{},"unknown":1}))]),
            ),
        )
        .await;
    assert!(malformed.is_err());
    client.cancel().await.unwrap();
    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn design_topology_mcp_branch_adapter_forwards_the_branch_to_every_query() {
    use rmcp::ServiceExt;
    let (baseline, baseline_service, endpoint) = start_mcp_client().await;
    let apply = |client: &McpClient, yaml: String| {
        let request = rmcp::model::CallToolRequestParams::new("apply_manifests")
            .with_arguments(args_obj(&[("manifests_yaml", serde_json::json!(yaml))]));
        let client = client.peer().clone();
        async move {
            let result = client.call_tool(request).await.unwrap();
            assert!(!result.is_error.unwrap_or(false), "{result:?}");
        }
    };
    apply(&baseline, MANIFEST.to_owned()).await;
    let created = baseline
        .call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch")
                .with_arguments(args_obj(&[("name", serde_json::json!("topology-review"))])),
        )
        .await
        .unwrap();
    assert!(!created.is_error.unwrap_or(false), "{created:?}");

    let branch_server = support::mcp_server(&endpoint, Some("topology-review".to_owned()));
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    let (branch_service, branch) = tokio::join!(
        branch_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    let (branch_service, branch) = (branch_service.unwrap(), branch.unwrap());
    let exported = branch
        .call_tool(
            rmcp::model::CallToolRequestParams::new("export_namespace").with_arguments(args_obj(
                &[("namespace", serde_json::json!("topology-surfaces"))],
            )),
        )
        .await
        .unwrap();
    let files = call_tool_result_text(&exported)["files"].clone();
    let yaml = files
        .as_object()
        .unwrap()
        .values()
        .map(|file| file.as_str().unwrap().to_owned())
        .collect::<Vec<_>>()
        .join("\n---\n");
    assert!(yaml.contains("value: Flow\n"));
    apply(
        &branch,
        yaml.replacen("value: Flow\n", "value: Flow on branch\n", 1),
    )
    .await;

    let read = |client: rmcp::service::Peer<rmcp::RoleClient>,
                tool: &'static str,
                request: serde_json::Value| async move {
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new(tool)
                    .with_arguments(args_obj(&[("request_json", request)])),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{tool}: {result:?}");
        call_tool_result_text(&result)
    };
    let request = serde_json::json!({"graph":{"id":{"namespace":"topology-surfaces","slug":"flow","version":"1"}}});
    let on_branch = read(
        branch.peer().clone(),
        "get_design_graph_detail",
        request.clone(),
    )
    .await;
    let on_baseline = read(baseline.peer().clone(), "get_design_graph_detail", request).await;
    assert_eq!(
        on_branch["entity"]["designGraph"]["title"]["value"],
        "Flow on branch"
    );
    assert_eq!(
        on_baseline["entity"]["designGraph"]["title"]["value"],
        "Flow"
    );
    assert_eq!(
        on_branch["entity"]["designGraph"]["steps"], on_baseline["entity"]["designGraph"]["steps"],
        "the branch edit keeps every step identity"
    );
    branch.cancel().await.unwrap();
    branch_service.cancel().await.unwrap();
    baseline.cancel().await.unwrap();
    baseline_service.cancel().await.unwrap();
}

mod authenticated {
    use std::{
        sync::{atomic::AtomicI64, Arc},
        time::Duration,
    };

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use rmcp::{
        model::CallToolRequestParams,
        transport::{
            streamable_http_client::StreamableHttpClientTransportConfig,
            StreamableHttpClientTransport,
        },
        ServiceExt,
    };
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tonic::Request;
    use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
    use trogon_atlas_server::{
        auth::{BearerAuth, TokenRegistry},
        mcp::{data_plane, mcp_router},
        metrics::ServerMetrics,
        service::EventModelServiceImpl,
        telemetry::{InflightLayer, RpcMetricsLayer},
    };

    type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

    const WRITER: &str = "topology-mcp-writer";
    const READER: &str = "topology-mcp-reader";
    const HIDDEN: &str = "topology-mcp-hidden";

    async fn server() -> (String, tokio::task::JoinHandle<()>, String) {
        let registry = TokenRegistry::from_file_contents(
            serde_json::from_value(json!({"principals": {
                "writer":{"role":"writer","tokens":[WRITER],"parent":"topology-mcp-owner"},
                "reader":{"role":"reader","tokens":[READER],"parent":"topology-mcp-owner"},
                "hidden":{"role":"reader","tokens":[HIDDEN],"parent":"other-owner"}
            }}))
            .unwrap(),
        )
        .unwrap();
        let service = EventModelServiceImpl::try_new(
            Arc::new(trogon_atlas_testsupport::shared().await.store().await),
            ServerMetrics::default(),
        )
        .unwrap();
        let namespace =
            trogon_atlas_proto::event_model_service_server::EventModelService::register_namespace(
                &service,
                Request::new(pb::RegisterNamespaceRequest {
                    name: "topology-mcp".into(),
                    parent: "topology-mcp-owner".into(),
                }),
            )
            .await
            .unwrap()
            .into_inner()
            .namespace
            .unwrap()
            .id;
        let (drain, _) = tokio::sync::watch::channel(0i64);
        let app = mcp_router(
            data_plane(
                EventModelServiceServer::new(service),
                BearerAuth::new(registry),
            ),
            RpcMetricsLayer::new(ServerMetrics::default()),
            InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
            Some(Duration::from_secs(30)),
            36 * 1024 * 1024,
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}/mcp"), task, namespace)
    }

    async fn connect(uri: &str, token: &str) -> Client {
        ().serve(StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(uri.to_owned()).auth_header(token),
        ))
        .await
        .unwrap()
    }

    async fn invoke(
        client: &Client,
        name: &'static str,
        arguments: Value,
    ) -> Result<Value, String> {
        let result = client
            .call_tool(
                CallToolRequestParams::new(name)
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
            .map_err(|error| format!("{error:?}"))?;
        let text = result
            .content
            .first()
            .and_then(|content| content.as_text())
            .map(|text| text.text.clone())
            .unwrap_or_default();
        if result.is_error.unwrap_or(false) {
            return Err(text);
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    async fn succeed(client: &Client, name: &'static str, arguments: Value) -> Value {
        invoke(client, name, arguments)
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    fn decode<M: prost::Message + Default>(body: &Value) -> M {
        prost::Message::decode(
            STANDARD
                .decode(body["binpb_base64"].as_str().unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap()
    }

    async fn detail_of(client: &Client, id: Value) -> Value {
        succeed(
            client,
            "get_design_graph_detail",
            json!({"request_json": {"graph": {"id": id}}}),
        )
        .await
    }

    fn flow(ns: &str, ids: Option<(&str, &str, &str)>) -> String {
        let (start, call, transition, subject) = match ids {
            Some((start, call, subject)) => (
                format!("stepId: {{value: {start}}}, "),
                format!("stepId: {{value: {call}}}, "),
                format!(
                    "transitions: [{{label: {{value: next}}, targetStepId: {{value: {call}}}}}, {{label: {{value: next}}, targetStepId: {{value: {call}}}}}], "
                ),
                format!("connectionId: {{value: {subject}}}, "),
            ),
            None => Default::default(),
        };
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
  title: {{value: Flow}}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  subjects: [{{{subject}subject: {{entity: {{kind: ENTITY_KIND_COMPONENT, id: {{namespace: {ns}, slug: checkout, version: 1}}}}}}}}]
  steps:
    - {{{start}{transition}title: {{value: Start}}, target: {{component: {{kind: ENTITY_KIND_COMPONENT, id: {{namespace: {ns}, slug: checkout, version: 1}}}}}}}}
    - {{{call}title: {{value: Call}}, target: {{subflow: {{id: {{namespace: {ns}, slug: callee, version: 1}}}}}}}}
"
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_design_topology_mcp_forwards_roles_and_keeps_identities_through_export_and_reapply(
    ) {
        let (uri, task, ns) = server().await;
        let writer = connect(&uri, WRITER).await;
        let reader = connect(&uri, READER).await;
        let hidden = connect(&uri, HIDDEN).await;
        let id = |slug: &str| json!({"namespace":ns,"slug":slug,"version":"1"});
        succeed(
            &writer,
            "apply_manifests",
            json!({"manifests_yaml": flow(&ns, None)}),
        )
        .await;
        let staged = detail_of(&reader, id("flow")).await;
        let ids: Vec<String> = staged["entity"]["designGraph"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["stepId"]["value"].as_str().unwrap().to_owned())
            .collect();
        let subject = staged["entity"]["designGraph"]["subjects"][0]["connectionId"]["value"]
            .as_str()
            .unwrap()
            .to_owned();
        succeed(
            &writer,
            "apply_manifests",
            json!({"manifests_yaml": flow(&ns, Some((&ids[0], &ids[1], &subject)))}),
        )
        .await;
        let wired = detail_of(&reader, id("flow")).await;
        let steps = wired["entity"]["designGraph"]["steps"].as_array().unwrap();
        let connections: Vec<String> = steps[0]["transitions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["connectionId"]["value"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(connections.len(), 2);
        assert_ne!(connections[0], connections[1]);
        assert_eq!(steps[0]["stepId"]["value"], json!(ids[0]));

        let outgoing: pb::GetReferencesResponse = decode(
            &succeed(
                &reader,
                "get_outgoing_references",
                json!({"kind":"design_graph","namespace":ns,"slug":"flow","version":1}),
            )
            .await,
        );
        let rows: Vec<_> = outgoing
            .references
            .iter()
            .map(|r| {
                (
                    r.from_field.clone(),
                    r.step_id.as_ref().map(|s| s.value.clone()),
                    r.connection_id.as_ref().map(|c| c.value.clone()),
                    r.target_step_id.as_ref().map(|s| s.value.clone()),
                )
            })
            .collect();
        let steps = &wired["entity"]["designGraph"]["steps"];
        assert_eq!(
            rows,
            vec![
                (
                    "steps[0].target.component".to_owned(),
                    Some(ids[0].clone()),
                    None,
                    None
                ),
                (
                    "steps[1].target.subflow".to_owned(),
                    Some(ids[1].clone()),
                    None,
                    None
                ),
                (
                    "subjects[0].subject.entity".to_owned(),
                    None,
                    Some(subject.clone()),
                    None
                ),
                (
                    "steps[0].transitions[0]".to_owned(),
                    Some(ids[0].clone()),
                    Some(connections[0].clone()),
                    Some(ids[1].clone())
                ),
                (
                    "steps[0].transitions[1]".to_owned(),
                    Some(ids[0].clone()),
                    Some(connections[1].clone()),
                    Some(ids[1].clone())
                ),
            ],
            "{steps}"
        );
        assert!(outgoing
            .references
            .iter()
            .all(|r| !r.from_uid.is_empty() && r.from_uid == outgoing.references[0].from_uid));
        let incoming: pb::GetReferencesResponse = decode(
            &succeed(
                &reader,
                "get_incoming_references",
                json!({"kind":"design_graph","namespace":ns,"slug":"callee","version":1}),
            )
            .await,
        );
        assert_eq!(incoming.references.len(), 1);
        assert_eq!(incoming.references[0].from_field, "steps[1].target.subflow");
        assert_eq!(
            incoming.references[0].step_id.as_ref().unwrap().value,
            ids[1]
        );

        let impact: pb::GetImpactResponse = decode(
            &succeed(
                &reader,
                "get_impact",
                json!({"kind":"component","namespace":ns,"slug":"checkout","version":1}),
            )
            .await,
        );
        let graph = impact
            .nodes
            .iter()
            .find(|node| {
                let found = node.entity.as_ref().unwrap();
                found.kind == pb::EntityKind::DesignGraph as i32
                    && found.id.as_ref().unwrap().slug == "flow"
            })
            .unwrap();
        assert_eq!(graph.depth, 1);
        assert_eq!(graph.path[0].from_field, "steps[0].target.component");
        assert_eq!(graph.path[0].step_id.as_ref().unwrap().value, ids[0]);

        let exported = succeed(&reader, "export_namespace", json!({"namespace": ns})).await;
        assert_eq!(exported["skipped"], json!([]));
        let yaml = exported["files"]
            .as_object()
            .unwrap()
            .values()
            .map(|file| file.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
            .join("\n---\n");
        for retained in ids.iter().chain(&connections).chain([&subject]) {
            assert!(yaml.contains(retained.as_str()), "export lost {retained}");
        }
        let diff = succeed(&reader, "diff_manifests", json!({"manifests_yaml": yaml})).await;
        assert_eq!(diff["changed"], json!(false), "{diff}");
        let reapplied = succeed(&writer, "apply_manifests", json!({"manifests_yaml": yaml})).await;
        let lines: Vec<_> = reapplied["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap())
            .collect();
        assert_eq!(lines.len(), 3, "{reapplied}");
        assert!(
            lines.iter().all(|line| line.ends_with(" unchanged")),
            "{lines:?}"
        );
        let after = detail_of(&reader, id("flow")).await;
        assert_eq!(after["entity"], wired["entity"]);
        assert_eq!(
            after["etag"], wired["etag"],
            "a no-op reapply writes nothing"
        );

        let edited = yaml.replacen("value: Callee", "value: Reader edit", 1);
        let refused = invoke(
            &reader,
            "apply_manifests",
            json!({"manifests_yaml": edited}),
        )
        .await;
        assert!(
            refused.is_err(),
            "readers cannot change the graph: {refused:?}"
        );
        let applied = succeed(
            &writer,
            "apply_manifests",
            json!({"manifests_yaml": edited}),
        )
        .await;
        assert!(applied["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().ends_with("callee@1 configured")));
        let hidden_view = invoke(
            &hidden,
            "get_design_graph_detail",
            json!({"request_json": {"graph": {"id": id("flow")}}}),
        )
        .await
        .unwrap_err();
        assert!(hidden_view.contains("not_found"), "{hidden_view}");
        assert!(!hidden_view.contains("Flow"));
        task.abort();
        drop(hidden);
    }
}
