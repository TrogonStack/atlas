//! `trogon-atlas protection` against a real server behind the auth stack:
//! who may protect and lift, that a protected identity refuses a write with
//! `ENTITY_PROTECTED`, and that a listing the server could not finish exits
//! as incomplete instead of passing as the whole registry.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::NatsStore;

const ADMIN: &str = "protection-cli-admin";
const WRITER: &str = "protection-cli-writer";
const READER: &str = "protection-cli-reader";

async fn serve(store: Arc<NatsStore>) -> String {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "protection-admin":{"role":"admin","tokens":[ADMIN]},
            "protection-writer":{"role":"writer","tokens":[WRITER]},
            "protection-reader":{"role":"reader","tokens":[READER]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
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
    format!("http://{address}")
}

struct Run {
    code: i32,
    outcome: Value,
}

fn run(endpoint: &str, token: &str, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            endpoint,
            "--auth-token",
            token,
            "--format",
            "json",
        ])
        .args(args)
        .output()
        .unwrap();
    let outcome = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{args:?}: {e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    Run {
        code: output.status.code().unwrap(),
        outcome,
    }
}

fn data(run: &Run) -> &Value {
    assert_eq!(run.code, 0, "{}", run.outcome);
    &run.outcome["data"]
}

fn event_manifest(dir: &Path, namespace: &str, title: &str) -> String {
    let path = dir.join(format!("{title}.yaml"));
    std::fs::write(
        &path,
        format!(
            "apiVersion: {}\nkind: Event\nmetadata: {{namespace: {namespace}, name: placed, version: 1}}\nspec: {{title: {title}}}\n",
            trogon_atlas_client::manifest::API_VERSION
        ),
    )
    .unwrap();
    path.to_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protected_identity_refuses_writes_until_an_admin_lifts_it() {
    let endpoint = serve(Arc::new(
        trogon_atlas_testsupport::shared().await.store().await,
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let identity = "event/protection-cli/placed";
    data(&run(
        &endpoint,
        WRITER,
        &[
            "apply",
            "-f",
            &event_manifest(dir.path(), "protection-cli", "Placed"),
        ],
    ));

    let refused = run(
        &endpoint,
        READER,
        &["protection", "protect", identity, "--reason", "published"],
    );
    assert_eq!(refused.code, 7, "{}", refused.outcome);
    assert_eq!(refused.outcome["failure"]["category"], "unauthorized");

    for bad in [
        vec![
            "protection",
            "protect",
            "event/protection-cli/placed@1",
            "--reason",
            "published",
        ],
        vec!["protection", "protect", identity, "--reason", "  "],
        vec![
            "--branch",
            "draft",
            "protection",
            "protect",
            identity,
            "--reason",
            "published",
        ],
        vec!["protection", "unprotect", identity, "--if-match", "0"],
    ] {
        let invalid = run(&endpoint, ADMIN, &bad);
        assert_eq!(invalid.code, 5, "{bad:?}: {}", invalid.outcome);
        assert_eq!(invalid.outcome["failure"]["category"], "validation");
    }

    let dry = run(
        &endpoint,
        ADMIN,
        &[
            "protection",
            "protect",
            identity,
            "--reason",
            "published",
            "--validate-only",
        ],
    );
    assert_eq!(data(&dry)["validate_only"], true);
    assert_eq!(
        data(&run(&endpoint, READER, &["protection", "get", identity]))["state"],
        "unprotected"
    );

    let protected = run(
        &endpoint,
        ADMIN,
        &["protection", "protect", identity, "--reason", "published"],
    );
    let protected = data(&protected).clone();
    assert_eq!(protected["identity"], identity);
    assert_eq!(protected["changed"], true);
    assert_eq!(protected["protection"]["reason"], "published");
    let revision = protected["protection"]["revision"].as_u64().unwrap();
    assert!(revision > 0);

    let again = run(
        &endpoint,
        ADMIN,
        &["protection", "protect", identity, "--reason", "another"],
    );
    assert_eq!(data(&again)["changed"], false);
    assert_eq!(data(&again)["protection"]["reason"], "published");

    let write = run(
        &endpoint,
        WRITER,
        &[
            "apply",
            "-f",
            &event_manifest(dir.path(), "protection-cli", "Edited"),
        ],
    );
    assert_eq!(write.code, 4, "{}", write.outcome);
    assert_eq!(write.outcome["failure"]["code"], "ENTITY_PROTECTED");
    assert_eq!(write.outcome["failure"]["next_action"], "request_unprotect");
    assert_eq!(write.outcome["failure"]["details"]["identity"], identity);

    let got = run(&endpoint, READER, &["protection", "get", identity]);
    assert_eq!(data(&got)["state"], "protected");
    assert_eq!(data(&got)["protection"]["revision"], revision);

    let listed = run(
        &endpoint,
        READER,
        &["protection", "list", "--namespace", "protection-cli"],
    );
    let listed = data(&listed);
    assert_eq!(listed["verdict"], "complete");
    assert_eq!(listed["complete"], true);
    assert_eq!(listed["protections"][0]["identity"], identity);
    assert_eq!(listed["unknown"], json!([]));

    let writer_lift = run(&endpoint, WRITER, &["protection", "unprotect", identity]);
    assert_eq!(writer_lift.code, 7, "{}", writer_lift.outcome);

    let stale = run(
        &endpoint,
        ADMIN,
        &[
            "protection",
            "unprotect",
            identity,
            "--if-match",
            &(revision + 1000).to_string(),
        ],
    );
    assert_eq!(stale.code, 3, "{}", stale.outcome);
    assert_eq!(stale.outcome["failure"]["code"], "STALE_PRECONDITION");

    let lifted = run(
        &endpoint,
        ADMIN,
        &[
            "protection",
            "unprotect",
            identity,
            "--if-match",
            &revision.to_string(),
        ],
    );
    assert_eq!(data(&lifted)["changed"], true);
    assert!(data(&lifted)["changeset_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));

    let unchanged = run(&endpoint, ADMIN, &["protection", "unprotect", identity]);
    assert_eq!(data(&unchanged)["changed"], false);
    assert_eq!(data(&unchanged)["changeset_id"], Value::Null);

    data(&run(
        &endpoint,
        WRITER,
        &[
            "apply",
            "-f",
            &event_manifest(dir.path(), "protection-cli", "Edited"),
        ],
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unfinished_protection_listing_exits_incomplete_and_names_unreadable_rows() {
    let nats = trogon_atlas_testsupport::shared().await;
    let (store, registry) = Box::pin(nats.store_with_protection_registry()).await;
    let endpoint = serve(Arc::new(store)).await;
    let dir = tempfile::tempdir().unwrap();
    let identity = "event/protection-partial/placed";
    data(&run(
        &endpoint,
        WRITER,
        &[
            "apply",
            "-f",
            &event_manifest(dir.path(), "protection-partial", "Placed"),
        ],
    ));
    data(&run(
        &endpoint,
        ADMIN,
        &["protection", "protect", identity, "--reason", "published"],
    ));
    let revision = registry.corrupt_every_row().await[0];
    registry.add_unparseable_key().await;

    let listed = run(&endpoint, READER, &["protection", "list"]);
    assert_eq!(listed.code, 11, "{}", listed.outcome);
    assert_eq!(listed.outcome["ok"], true);
    let listing = &listed.outcome["data"];
    assert_eq!(listing["verdict"], "partial");
    assert_eq!(listing["complete"], false);
    assert_eq!(listing["protections"], json!([]));
    assert_eq!(listing["unknown"][0]["identity"], identity);
    assert_eq!(listing["unknown"][0]["revision"], revision);
    assert_eq!(listing["unknown"][0]["liftable"], true);

    let text = Command::new(env!("CARGO_BIN_EXE_trogon-atlas"))
        .args([
            "--endpoint",
            &endpoint,
            "--auth-token",
            READER,
            "protection",
            "list",
        ])
        .output()
        .unwrap();
    assert_eq!(text.status.code(), Some(11));
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains(identity), "{text}");
    assert!(text.contains("could not be enumerated"), "{text}");
    assert!(text.contains("could not be read"), "{text}");

    let got = run(&endpoint, READER, &["protection", "get", identity]);
    assert_eq!(got.code, 11, "{}", got.outcome);
    assert_eq!(got.outcome["data"]["state"], "unknown");
    assert_eq!(got.outcome["data"]["unknown"]["revision"], revision);

    let write = run(
        &endpoint,
        WRITER,
        &[
            "apply",
            "-f",
            &event_manifest(dir.path(), "protection-partial", "Edited"),
        ],
    );
    assert_eq!(write.code, 75, "{}", write.outcome);
    assert_eq!(write.outcome["failure"]["code"], "PROTECTION_STATE_UNKNOWN");

    data(&run(
        &endpoint,
        ADMIN,
        &[
            "protection",
            "unprotect",
            identity,
            "--if-match",
            &revision.to_string(),
        ],
    ));
    let after = run(&endpoint, READER, &["protection", "list"]);
    assert_eq!(after.code, 11, "{}", after.outcome);
    assert_eq!(after.outcome["data"]["complete"], false);
    assert_eq!(after.outcome["data"]["unknown"], json!([]));
}
