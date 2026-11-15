#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use trogon_atlas_client::{
    artifacts::sha256_hex,
    client::{connect_with_options, Client, ConnectOptions},
};
use trogon_atlas_proto as pb;

const TOKEN: &str = "retained-artifact-fixture-token";
const MIB: usize = 1024 * 1024;

struct Server {
    child: Child,
    log: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    async fn start(
        nats_url: &str,
        directory: &Path,
        artifact_limit: usize,
        message_limit: usize,
    ) -> (Self, Client) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let log = directory.join(format!("server-{artifact_limit}.log"));
        let child = Command::new(env!("CARGO_BIN_EXE_trogon-atlas-server"))
            .env_clear()
            .env("TROGON_ATLAS_AUTH_TOKEN", TOKEN)
            .env("RUST_LOG", "error")
            .args([
                "--listen",
                &address.to_string(),
                "--nats-url",
                nats_url,
                "--artifact-backend",
                "nats",
                "--artifact-max-bytes",
                &artifact_limit.to_string(),
                "--max-message-bytes",
                &message_limit.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let mut server = Self { child, log };
        let endpoint = format!("http://{address}");
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!(
                    "server exited during startup: {status}: {}",
                    std::fs::read_to_string(&server.log).unwrap()
                );
            }
            let client = tokio::time::timeout(Duration::from_secs(2), async {
                let mut client = connect_with_options(
                    &endpoint,
                    Some(TOKEN),
                    10,
                    ConnectOptions {
                        max_message_bytes: Some(36 * MIB),
                        ..Default::default()
                    },
                )
                .await
                .ok()?;
                let info = client
                    .get_server_info(pb::GetServerInfoRequest {})
                    .await
                    .ok()?
                    .into_inner();
                (info.writer_status?.role
                    == pb::get_server_info_response::WriterRole::Writer as i32)
                    .then_some(client)
            })
            .await;
            if let Ok(Some(client)) = client {
                return (server, client);
            }
            assert!(
                Instant::now() < deadline,
                "server never became writable: {}",
                std::fs::read_to_string(&server.log).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retained_artifact_survives_restart_with_lower_upload_and_decode_limits() {
    let nats = trogon_atlas_testsupport::TestNats::start().await;
    let directory = tempfile::tempdir().unwrap();
    let (first, mut client) = Server::start(&nats.url, directory.path(), 8 * MIB, 12 * MIB).await;
    let namespace = client
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "retained-artifact".into(),
            parent: "retention-fixture".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let content = vec![0x5a; 5 * MIB + 31];
    let begin = pb::BeginArtifactUploadRequest {
        namespace: namespace.clone(),
        filename: "retained.bin".into(),
        media_type: "application/octet-stream".into(),
        size: content.len() as u64,
        sha256: sha256_hex(&content),
    };
    let upload = client
        .begin_artifact_upload(begin.clone())
        .await
        .unwrap()
        .into_inner();
    client
        .upload_artifact_content(pb::UploadArtifactContentRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id.clone(),
            content: content.clone(),
        })
        .await
        .unwrap();
    let artifact = client
        .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id,
        })
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    drop(client);
    drop(first);

    let (_second, mut client) = Server::start(&nats.url, directory.path(), MIB, 4 * MIB).await;
    let storage = client
        .get_artifact_storage_info(pb::GetArtifactStorageInfoRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(storage.max_bytes, MIB as u64);
    let rejected = client.begin_artifact_upload(begin).await.unwrap_err();
    assert_eq!(rejected.code(), tonic::Code::InvalidArgument);
    let rejected = client
        .upload_artifact_content(pb::UploadArtifactContentRequest {
            namespace: namespace.clone(),
            upload_id: artifact.artifact_id.clone(),
            content: content.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), tonic::Code::OutOfRange);
    assert!(rejected
        .message()
        .contains("decoded message length too large"));
    let metadata = client
        .get_artifact(pb::GetArtifactRequest {
            namespace: namespace.clone(),
            artifact_id: artifact.artifact_id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    assert_eq!(metadata, artifact);
    let downloaded = client
        .get_artifact_content(pb::GetArtifactContentRequest {
            namespace,
            artifact_id: artifact.artifact_id,
        })
        .await
        .expect("lowering admission limits must not prevent retained artifact responses")
        .into_inner();
    assert_eq!(downloaded.content, content);
    assert_eq!(
        downloaded.artifact.unwrap().sha256,
        sha256_hex(&downloaded.content)
    );
}
