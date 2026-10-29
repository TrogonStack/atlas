#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    process::Command,
    sync::{Arc, Mutex},
};

use sha2::{Digest, Sha256};
use tonic::service::interceptor::InterceptedService;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::Store;

const SOURCE: &str = "416a6c7e02034390c53e96f459439e7e1f913dfe";
const TOKEN: &str = "historical-acceptance-admin";
const DISCOVERY: &str = "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetServerInfo";

async fn apply(
    binary: &std::path::Path,
    endpoint: &str,
    path: &std::path::Path,
) -> std::process::Output {
    let mut command = Command::new(binary);
    command
        .args(["--endpoint", endpoint, "--auth-token", TOKEN, "apply", "-f"])
        .arg(path);
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a verified frozen revision 18 CLI; see docs/how-to/test-historical-native-client.md"]
async fn historical_18_apply_refuses_native_19_without_writes() {
    let binary = std::path::PathBuf::from(
        std::env::var("ATLAS_HISTORICAL_CLI").expect("ATLAS_HISTORICAL_CLI required"),
    );
    assert_eq!(std::env::var("ATLAS_HISTORICAL_SOURCE").unwrap(), SOURCE);
    let expected =
        std::env::var("ATLAS_HISTORICAL_SHA256").expect("ATLAS_HISTORICAL_SHA256 required");
    assert_eq!(expected.len(), 64);
    assert_eq!(
        Sha256::digest(std::fs::read(&binary).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        expected,
        "artifact hash mismatch"
    );
    assert_eq!(
        (pb::CONTRACT_REVISION, pb::MIN_CLIENT_CONTRACT_REVISION),
        (19, 19)
    );
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            "[principals.admin]\nrole = 'admin'\ntokens = ['{TOKEN}']\n"
        ))
        .unwrap(),
    )
    .unwrap();
    let grpc = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(pb::event_model_service_server::EventModelServiceServer::new(service)),
        BearerAuth::new(Arc::new(registry)),
    );
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .layer(tower::layer::layer_fn(move |inner| {
                let recorded = recorded.clone();
                tower::util::MapRequest::new(
                    inner,
                    move |req: tonic::codegen::http::Request<tonic::body::Body>| {
                        recorded.lock().unwrap().push(req.uri().path().to_string());
                        req
                    },
                )
            }))
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("native.yaml");
    let manifest = "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Schema\nmetadata: {namespace: acceptance, name: payload}\nspec: {title: Payload}\n---\napiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Specification\nmetadata: {namespace: acceptance, name: contract}\nspec:\n  title: Before\n  messages:\n    - name: {value: Message}\n      schemas:\n        - role: ROLE_PAYLOAD\n          schema: payload\n          environments: [{value: Blue}, {value: green}]\n          isDefault: false\n";
    std::fs::write(&path, manifest).unwrap();
    let current = std::path::Path::new(env!("CARGO_BIN_EXE_trogon-atlas"));
    let seeded = apply(current, &endpoint, &path).await;
    assert!(
        seeded.status.success(),
        "{}",
        String::from_utf8_lossy(&seeded.stderr)
    );
    let id = pb::Id {
        namespace: "acceptance".into(),
        slug: "contract".into(),
        version: 1,
    };
    let before = store
        .get(pb::EntityKind::Specification, &id, None)
        .await
        .unwrap();
    let Some(pb::entity::Kind::Specification(spec)) = &before.entity.kind else {
        panic!("specification missing")
    };
    assert_eq!(
        spec.messages[0].schemas[0]
            .environments
            .iter()
            .map(|v| v.value.as_str())
            .collect::<Vec<_>>(),
        ["Blue", "green"]
    );
    assert_eq!(spec.messages[0].schemas[0].is_default, Some(false));
    let seq = store.current_change_seq().await.unwrap();
    let exported = Command::new(current)
        .args([
            "--endpoint",
            &endpoint,
            "--auth-token",
            TOKEN,
            "get",
            "specification/acceptance/contract@1",
        ])
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let native = String::from_utf8(exported.stdout).unwrap();
    assert!(native.contains("Before"));
    std::fs::write(&path, native.replace("Before", "After")).unwrap();
    calls.lock().unwrap().clear();
    let refused = apply(&binary, &endpoint, &path).await;
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success());
    assert!(stderr.contains("client contract revision 18 is older than revision 19 the server requires; upgrade this client before mutating"), "{stderr}");
    assert_eq!(*calls.lock().unwrap(), [DISCOVERY]);
    let after = store
        .get(pb::EntityKind::Specification, &id, None)
        .await
        .unwrap();
    assert_eq!(after.entity, before.entity);
    assert_eq!(after.etag, before.etag);
    assert_eq!(store.current_change_seq().await.unwrap(), seq);
    eprintln!("historical source={SOURCE} sha256={expected}; refusal={stderr}; RPCs={:?}; unchanged entity/UID/etag/variants and sequence={seq}", calls.lock().unwrap());
    calls.lock().unwrap().clear();
    let accepted = apply(current, &endpoint, &path).await;
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let changed = store
        .get(pb::EntityKind::Specification, &id, None)
        .await
        .unwrap();
    let Some(pb::entity::Kind::Specification(spec)) = &changed.entity.kind else {
        panic!("specification missing")
    };
    assert_eq!(spec.title, "After");
    let mut expected_entity = before.entity.clone();
    if let Some(pb::entity::Kind::Specification(value)) = &mut expected_entity.kind {
        value.title = "After".into();
    }
    assert_eq!(changed.entity.kind, expected_entity.kind);
    assert_eq!(
        changed.entity.system.as_ref().unwrap().uid,
        before.entity.system.as_ref().unwrap().uid
    );
    assert_ne!(changed.etag, before.etag);
    assert!(store.current_change_seq().await.unwrap() > seq);
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .any(|path| path.ends_with("/BatchMutate")));
    eprintln!(
        "current19 control persisted title and preserved variants/UID; RPCs={:?}",
        calls.lock().unwrap()
    );
    server.abort();
}
