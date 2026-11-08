#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_nats::jetstream::{
    self,
    stream::{DiscardPolicy, RetentionPolicy},
};
use prost::Message as _;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    sync::Notify,
};
use trogon_atlas_core::WriterRole;
use trogon_atlas_store::{
    artifact_publication::{
        ArtifactPublicationCandidate, ArtifactPublicationState, ArtifactReceiptId,
    },
    NatsStore, NatsStoreConfig, Store, StoreError, StoreMetrics,
};

struct NatsFixture {
    url: String,
    _container: ContainerAsync<GenericImage>,
}

impl NatsFixture {
    async fn start() -> Self {
        let container = GenericImage::new("nats", "2.15.2")
            .with_wait_for(WaitFor::message_on_stderr("Server is ready"))
            .with_exposed_port(4222.tcp())
            .with_copy_to(
                "/etc/atlas-test-nats.conf",
                include_bytes!("../../trogon-atlas-testsupport/nats-server.conf").to_vec(),
            )
            .with_cmd(["-c", "/etc/atlas-test-nats.conf"])
            .start()
            .await
            .unwrap();
        let port = container.get_host_port_ipv4(4222).await.unwrap();
        Self {
            url: format!("nats://127.0.0.1:{port}"),
            _container: container,
        }
    }

    fn store_config(&self) -> NatsStoreConfig {
        let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
        let mut config = NatsStoreConfig::new(&self.url, StoreMetrics::default());
        config.bucket = format!("entities-{suffix}");
        config.branches_bucket = format!("branches-{suffix}");
        config.changesets_bucket = format!("changesets-{suffix}");
        config.revisions_bucket = format!("revisions-{suffix}");
        config.batches_bucket = format!("batches-{suffix}");
        config.namespaces_bucket = format!("namespaces-{suffix}");
        config.operations_bucket = format!("operations-{suffix}");
        config.lease_bucket = format!("lease-{suffix}");
        config.stream = format!("CHANGES_{}", suffix.replace('-', "_"));
        config.subject_root = format!("changes.{suffix}");
        config.changes_max_msgs = Some(1000);
        config.changes_max_age = None;
        config
    }
}

fn candidate(filename: &str, generation: &str) -> ArtifactPublicationCandidate {
    ArtifactPublicationCandidate::new(
        &trogon_atlas_proto::Artifact {
            namespace: "sample".into(),
            artifact_id: uuid::Uuid::from_u128(1).to_string(),
            filename: filename.into(),
            media_type: "image/png".into(),
            size: 42,
            sha256: "a".repeat(64),
        },
        "nats",
        &format!("artifacts/{generation}"),
        "writer",
    )
    .unwrap()
}

fn sample() -> ArtifactPublicationCandidate {
    candidate("state.png", &uuid::Uuid::from_u128(2).to_string())
}

#[derive(prost::Message)]
struct LeaseRecord {
    #[prost(string, tag = "1")]
    holder: String,
    #[prost(uint64, tag = "2")]
    epoch: u64,
    #[prost(int64, tag = "3")]
    expires: i64,
}

async fn expire(context: &jetstream::Context, bucket: &str) {
    let lease = context.get_key_value(bucket).await.unwrap();
    let entry = lease.entry("writer-lease").await.unwrap().unwrap();
    let mut row = LeaseRecord::decode(entry.value).unwrap();
    row.expires = 0;
    lease
        .update("writer-lease", row.encode_to_vec().into(), entry.revision)
        .await
        .unwrap();
}

#[tokio::test]
async fn receipt_is_immutable_binds_exact_candidate_and_survives_takeover() {
    let nats = NatsFixture::start().await;
    let mut config = nats.store_config();
    config.lease_duration = Duration::from_secs(120);
    config.lease_renew_interval = Duration::from_secs(60);
    let owner = NatsStore::connect_with(config.clone()).await.unwrap();
    let authority = owner.artifact_publication_authority().unwrap();
    let permit = authority.capture_permit().await.unwrap();
    let id = ArtifactReceiptId::generate();
    let first = authority.publish(&permit, &sample(), &id).await.unwrap();
    let repeated = authority.publish(&permit, &sample(), &id).await.unwrap();
    assert_eq!(first, repeated);
    assert!(authority
        .verify(
            &candidate("changed.png", &uuid::Uuid::from_u128(2).to_string()),
            &id
        )
        .await
        .is_err());
    assert!(authority
        .verify(
            &candidate("state.png", &uuid::Uuid::from_u128(3).to_string()),
            &id
        )
        .await
        .is_err());
    let context = jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    expire(&context, &config.lease_bucket).await;
    config.role = WriterRole::Standby;
    let standby = NatsStore::connect_with(config).await.unwrap();
    assert!(standby.writer_status().epoch > owner.writer_status().epoch);
    assert!(owner.is_writer());
    assert!(authority
        .publish(&permit, &sample(), &ArtifactReceiptId::generate())
        .await
        .is_err());
    assert_eq!(
        authority.publish(&permit, &sample(), &id).await.unwrap(),
        first
    );
    assert!(matches!(
        standby
            .artifact_publication_authority()
            .unwrap()
            .verify(&sample(), &id)
            .await
            .unwrap(),
        ArtifactPublicationState::Accepted(_)
    ));
}

#[tokio::test]
async fn admission_permit_accepts_same_epoch_renewals() {
    let nats = NatsFixture::start().await;
    let mut config = nats.store_config();
    config.lease_duration = Duration::from_millis(900);
    config.lease_renew_interval = Duration::from_millis(100);
    let store = NatsStore::connect_with(config).await.unwrap();
    let authority = store.artifact_publication_authority().unwrap();
    let permit = authority.capture_permit().await.unwrap();
    let epoch = store.writer_status().epoch;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(store.writer_status().epoch, epoch);
    authority
        .publish(&permit, &sample(), &ArtifactReceiptId::generate())
        .await
        .unwrap();
}

#[tokio::test]
async fn fresh_retention_checks_refuse_expiry_eviction_and_consumption() {
    let nats = NatsFixture::start().await;
    let context = jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    for hazard in ["age", "bytes", "messages", "interest", "ttl", "transform"] {
        let config = nats.store_config();
        let store = NatsStore::connect_with(config.clone()).await.unwrap();
        let authority = store.artifact_publication_authority().unwrap();
        authority.validate().await.unwrap();
        let permit = authority.capture_permit().await.unwrap();
        let id = ArtifactReceiptId::generate();
        authority.publish(&permit, &sample(), &id).await.unwrap();
        let mut stream = context
            .get_stream(format!("KV_{}", config.lease_bucket))
            .await
            .unwrap()
            .cached_info()
            .config
            .clone();
        match hazard {
            "age" => {
                stream.max_age = Duration::from_secs(60);
                stream.duplicate_window = Duration::from_secs(30);
            }
            "bytes" => {
                stream.discard = DiscardPolicy::Old;
                stream.max_bytes = 1024 * 1024;
            }
            "messages" => {
                stream.discard = DiscardPolicy::Old;
                stream.max_messages = 100;
            }
            "interest" => stream.retention = RetentionPolicy::Interest,
            "ttl" => stream.allow_message_ttl = true,
            "transform" => {
                stream.subject_transform = Some(jetstream::stream::SubjectTransform {
                    source: format!("$KV.{}.>", config.lease_bucket),
                    destination: "redirect.>".into(),
                });
            }
            _ => unreachable!(),
        }
        let effective = context.update_stream(stream).await.unwrap().config;
        assert!(
            match hazard {
                "age" => effective.max_age == Duration::from_secs(60),
                "bytes" => effective.discard == DiscardPolicy::Old && effective.max_bytes > 0,
                "messages" => effective.discard == DiscardPolicy::Old && effective.max_messages > 0,
                "interest" => effective.retention == RetentionPolicy::Interest,
                "ttl" => effective.allow_message_ttl,
                "transform" => effective.subject_transform.is_some(),
                _ => false,
            },
            "the {hazard} hazard must actually be installed"
        );
        assert!(authority.validate().await.is_err(), "{hazard}");
        assert!(authority.verify(&sample(), &id).await.is_err(), "{hazard}");
        assert!(authority.capture_permit().await.is_err(), "{hazard}");
    }
}

struct PublishGate {
    url: String,
    armed: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    direct_reads: Arc<AtomicUsize>,
    drop_ack: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for PublishGate {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl PublishGate {
    async fn start(upstream: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("nats://{}", listener.local_addr().unwrap());
        let upstream = upstream.strip_prefix("nats://").unwrap().to_owned();
        let armed = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let direct_reads = Arc::new(AtomicUsize::new(0));
        let drop_ack = Arc::new(AtomicBool::new(false));
        let (task_armed, task_entered, task_release, task_reads) = (
            armed.clone(),
            entered.clone(),
            release.clone(),
            direct_reads.clone(),
        );
        let task_drop_ack = drop_ack.clone();
        let task = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            let server = TcpStream::connect(upstream).await.unwrap();
            let (client_read, mut client_write) = client.into_split();
            let (server_read, mut server_write) = server.into_split();
            let toward_server = async {
                let mut input = BufReader::new(client_read);
                loop {
                    let mut line = Vec::new();
                    if input.read_until(b'\n', &mut line).await.unwrap() == 0 {
                        break;
                    }
                    let text = String::from_utf8_lossy(&line);
                    let tokens: Vec<_> = text.split_whitespace().collect();
                    let mut body = Vec::new();
                    if matches!(tokens.first(), Some(&"PUB" | &"HPUB")) {
                        let length: usize = tokens.last().unwrap().parse().unwrap();
                        body.resize(length + 2, 0);
                        input.read_exact(&mut body).await.unwrap();
                        if tokens[1].contains(".DIRECT.GET.") {
                            task_reads.fetch_add(1, Ordering::SeqCst);
                        }
                        if tokens[1].starts_with("$KV.")
                            && tokens[1].contains(".artifact-receipt.")
                            && task_armed.swap(false, Ordering::SeqCst)
                        {
                            task_entered.notify_one();
                            task_release.notified().await;
                        }
                    }
                    server_write.write_all(&line).await.unwrap();
                    server_write.write_all(&body).await.unwrap();
                }
            };
            let toward_client = async {
                let mut input = BufReader::new(server_read);
                loop {
                    let mut line = Vec::new();
                    if input.read_until(b'\n', &mut line).await.unwrap() == 0 {
                        break;
                    }
                    let text = String::from_utf8_lossy(&line);
                    let tokens: Vec<_> = text.split_whitespace().collect();
                    let mut body = Vec::new();
                    if matches!(tokens.first(), Some(&"MSG" | &"HMSG")) {
                        let length: usize = tokens.last().unwrap().parse().unwrap();
                        body.resize(length + 2, 0);
                        input.read_exact(&mut body).await.unwrap();
                        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) {
                            if value["stream"]
                                .as_str()
                                .is_some_and(|name| name.starts_with("KV_"))
                                && value["seq"].as_u64().is_some()
                                && task_drop_ack.swap(false, Ordering::SeqCst)
                            {
                                continue;
                            }
                        }
                    }
                    client_write.write_all(&line).await.unwrap();
                    client_write.write_all(&body).await.unwrap();
                }
            };
            tokio::select! { () = toward_server => {}, () = toward_client => {} }
        });
        Self {
            url,
            armed,
            entered,
            release,
            direct_reads,
            drop_ack,
            task,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_publication_ack_recovers_the_exact_immutable_receipt() {
    let nats = NatsFixture::start().await;
    let gate = PublishGate::start(&nats.url).await;
    let mut config = nats.store_config();
    config.url = gate.url.clone();
    config.lease_duration = Duration::from_secs(120);
    config.lease_renew_interval = Duration::from_secs(60);
    let store = NatsStore::connect_with(config).await.unwrap();
    let authority = store.artifact_publication_authority().unwrap();
    let permit = authority.capture_permit().await.unwrap();
    let id = ArtifactReceiptId::generate();
    gate.drop_ack.store(true, Ordering::SeqCst);
    let receipt = tokio::time::timeout(
        Duration::from_secs(15),
        authority.publish(&permit, &sample(), &id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        !gate.drop_ack.load(Ordering::SeqCst),
        "the actual acceptance PubAck must have been dropped"
    );
    assert_eq!(
        authority.publish(&permit, &sample(), &id).await.unwrap(),
        receipt
    );
    assert_eq!(
        authority.verify(&sample(), &id).await.unwrap(),
        ArtifactPublicationState::Accepted(receipt)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_timestamp_rejects_a_stored_receipt_after_expiry_without_takeover() {
    let nats = NatsFixture::start().await;
    let gate = PublishGate::start(&nats.url).await;
    let mut config = nats.store_config();
    config.url = gate.url.clone();
    config.lease_duration = Duration::from_secs(2);
    config.lease_renew_interval = Duration::from_secs(1);
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    let authority = store.artifact_publication_authority().unwrap();
    gate.direct_reads.store(0, Ordering::SeqCst);
    let permit = authority.capture_permit().await.unwrap();
    drop(store);
    let context = jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let lease = context.get_key_value(&config.lease_bucket).await.unwrap();
    let entry = lease.entry("writer-lease").await.unwrap().unwrap();
    let row = LeaseRecord::decode(entry.value).unwrap();
    let id = ArtifactReceiptId::generate();
    gate.armed.store(true, Ordering::SeqCst);
    let publishing = {
        let authority = authority.clone();
        let id = id.clone();
        tokio::spawn(async move { authority.publish(&permit, &sample(), &id).await })
    };
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    let sleep_micros =
        u64::try_from((row.expires - chrono::Utc::now().timestamp_micros()).max(0)).unwrap();
    tokio::time::sleep(Duration::from_micros(sleep_micros) + Duration::from_millis(50)).await;
    gate.release.notify_one();
    assert!(publishing.await.unwrap().is_err());
    let receipt = lease
        .entry(format!("artifact-receipt.{}", id.as_str()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.revision,
        entry.revision + 1,
        "global CAS really stored the delayed message with no intervening lease writes"
    );
    assert!(
        receipt.created
            >= async_nats::datetime::from_nanos(i128::from(row.expires) * 1000).unwrap()
    );
    assert_eq!(
        authority.verify(&sample(), &id).await.unwrap(),
        ArtifactPublicationState::Rejected
    );
    assert_eq!(
        gate.direct_reads.load(Ordering::SeqCst),
        0,
        "authority must never use follower-capable DIRECT.GET for lease or receipt reads"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_sequence_fences_takeover_after_snapshot_before_actual_publish() {
    let nats = NatsFixture::start().await;
    let gate = PublishGate::start(&nats.url).await;
    let mut config = nats.store_config();
    config.url = gate.url.clone();
    config.lease_duration = Duration::from_secs(120);
    config.lease_renew_interval = Duration::from_secs(60);
    let store = NatsStore::connect_with(config.clone()).await.unwrap();
    let authority = store.artifact_publication_authority().unwrap();
    let permit = authority.capture_permit().await.unwrap();
    let id = ArtifactReceiptId::generate();
    gate.armed.store(true, Ordering::SeqCst);
    let publishing = {
        let authority = authority.clone();
        let id = id.clone();
        tokio::spawn(async move { authority.publish(&permit, &sample(), &id).await })
    };
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    let context = jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    expire(&context, &config.lease_bucket).await;
    config.url = nats.url.clone();
    config.role = WriterRole::Standby;
    let standby = NatsStore::connect_with(config.clone()).await.unwrap();
    assert!(standby.writer_status().epoch > store.writer_status().epoch);
    assert!(store.is_writer());
    gate.release.notify_one();
    assert!(matches!(
        publishing.await.unwrap(),
        Err(StoreError::NotWriter { .. })
    ));
    let lease = context.get_key_value(&config.lease_bucket).await.unwrap();
    assert!(
        lease
            .entry(format!("artifact-receipt.{}", id.as_str()))
            .await
            .unwrap()
            .is_none(),
        "the stale receipt must never be stored after takeover advanced the same stream"
    );
    assert_eq!(
        authority.verify(&sample(), &id).await.unwrap(),
        ArtifactPublicationState::Missing
    );
}

#[tokio::test]
async fn pre_receipt_schema_marker_cannot_start_a_new_server() {
    let nats = NatsFixture::start().await;
    let config = nats.store_config();
    let context = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let entities = context
        .create_key_value(async_nats::jetstream::kv::Config {
            bucket: config.bucket.clone(),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    entities
        .put(
            "_schema",
            "trogonatlas.eventmodel.v1alpha1+schema-fields".into(),
        )
        .await
        .unwrap();
    assert!(matches!(
        NatsStore::connect_with(config).await,
        Err(StoreError::SchemaMismatch { .. })
    ));
}
