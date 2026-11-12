#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_nats::jetstream::kv::{Operation, Store as Registry};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    sync::oneshot,
    task::JoinHandle,
};
use trogon_atlas_core::{NamespaceName, OwnerId};
use trogon_atlas_store::{
    key::{namespace_name_key, namespace_record_key},
    NamespaceRecord, NamespaceTenure, NatsStore, Store, StoreError,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteOperation {
    Put,
    Delete,
}

struct WriteGate {
    subject: Option<String>,
    operation: WriteOperation,
    observed: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

struct WritePause {
    observed: oneshot::Receiver<()>,
    resume: oneshot::Sender<()>,
}

impl WritePause {
    async fn reached(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), &mut self.observed)
            .await
            .expect("request reaches the selected write")
            .expect("proxy remains connected");
    }

    fn resume(self) {
        self.resume.send(()).unwrap();
    }
}

struct KvProxy {
    url: String,
    gate: Arc<Mutex<Option<WriteGate>>>,
    task: JoinHandle<io::Result<()>>,
}

impl KvProxy {
    async fn start(upstream_url: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("nats://{}", listener.local_addr().unwrap());
        let upstream = upstream_url.strip_prefix("nats://").unwrap().to_owned();
        let gate = Arc::new(Mutex::new(None));
        let proxy_gate = gate.clone();
        let task = tokio::spawn(async move {
            let (client, _) = listener.accept().await?;
            let server = TcpStream::connect(upstream).await?;
            let (client_read, mut client_write) = client.into_split();
            let (mut server_read, server_write) = server.into_split();
            tokio::try_join!(
                forward_commands(client_read, server_write, proxy_gate),
                async {
                    tokio::io::copy(&mut server_read, &mut client_write)
                        .await
                        .map(|_| ())
                }
            )?;
            Ok(())
        });
        Self { url, gate, task }
    }

    fn pause_delete(&self, subject: Option<String>) -> WritePause {
        self.pause(WriteOperation::Delete, subject)
    }

    fn pause_put(&self, subject: String) -> WritePause {
        self.pause(WriteOperation::Put, Some(subject))
    }

    fn pause(&self, operation: WriteOperation, subject: Option<String>) -> WritePause {
        let (observed, notification) = oneshot::channel();
        let (resume, continuation) = oneshot::channel();
        let previous = self.gate.lock().unwrap().replace(WriteGate {
            subject,
            operation,
            observed,
            resume: continuation,
        });
        assert!(previous.is_none());
        WritePause {
            observed: notification,
            resume,
        }
    }
}

impl Drop for KvProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward_commands(
    reader: OwnedReadHalf,
    mut writer: OwnedWriteHalf,
    gate: Arc<Mutex<Option<WriteGate>>>,
) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    let mut pending: Option<(Vec<u8>, oneshot::Receiver<()>)> = None;
    loop {
        let read = match pending.as_mut() {
            Some((_, resume)) => tokio::select! {
                read = reader.read_until(b'\n', &mut line) => Some(read?),
                _ = resume => None,
            },
            None => Some(reader.read_until(b'\n', &mut line).await?),
        };
        let Some(read) = read else {
            let (packet, _) = pending.take().unwrap();
            writer.write_all(&packet).await?;
            continue;
        };
        if read == 0 {
            return Ok(());
        }
        let command = std::str::from_utf8(&line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let tokens: Vec<_> = command.split_ascii_whitespace().collect();
        let mut payload = Vec::new();
        if matches!(tokens.first(), Some(&"PUB" | &"HPUB")) {
            let length: usize = tokens
                .last()
                .unwrap()
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            payload.resize(length + 2, 0);
            reader.read_exact(&mut payload).await?;
        }
        let operation = if payload
            .windows(b"KV-Operation: DEL\r\n".len())
            .any(|window| window == b"KV-Operation: DEL\r\n")
        {
            WriteOperation::Delete
        } else {
            WriteOperation::Put
        };
        let selected = if matches!(tokens.first(), Some(&"PUB" | &"HPUB")) {
            gate.lock().unwrap().take_if(|gate| {
                gate.operation == operation
                    && gate
                        .subject
                        .as_deref()
                        .is_none_or(|subject| Some(&subject) == tokens.get(1))
            })
        } else {
            None
        };
        line.extend_from_slice(&payload);
        if let Some(selected) = selected {
            assert!(pending.is_none());
            selected.observed.send(()).unwrap();
            pending = Some((std::mem::take(&mut line), selected.resume));
        } else {
            writer.write_all(&line).await?;
            line.clear();
        }
    }
}

struct Fixture {
    store: Arc<NatsStore>,
    registry: Registry,
    proxy: KvProxy,
    bucket: String,
    name: NamespaceName,
    owner: OwnerId,
}

impl Fixture {
    async fn new() -> Self {
        let nats = trogon_atlas_testsupport::shared().await;
        let proxy = KvProxy::start(&nats.url).await;
        let mut config = nats.store_config();
        config.url.clone_from(&proxy.url);
        let bucket = config.namespaces_bucket.clone();
        let store = Arc::new(NatsStore::connect_with(config).await.unwrap());
        let context = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
        let registry = context.get_key_value(&bucket).await.unwrap();
        Self {
            store,
            registry,
            proxy,
            bucket,
            name: NamespaceName::parse("retained-artifact").unwrap(),
            owner: OwnerId::parse("original-owner").unwrap(),
        }
    }

    async fn provisional(&self) -> NamespaceRecord {
        self.store
            .adopt_namespace(
                &self.name,
                &self.owner,
                &NamespaceTenure::Provisional {
                    branch: "abandoned-upload".into(),
                },
            )
            .await
            .unwrap()
            .record
    }

    fn index_key(&self) -> String {
        namespace_name_key(self.owner.as_str(), self.name.as_str())
    }

    fn release(&self, record: &NamespaceRecord) -> JoinHandle<Result<(), StoreError>> {
        let store = self.store.clone();
        let id = record.id.clone();
        tokio::spawn(async move { store.release_namespace(&id).await })
    }

    async fn assert_retained(&self, record: &NamespaceRecord) {
        assert_eq!(
            self.store.get_namespace(&record.id).await.unwrap(),
            Some(record.clone()),
            "retained namespace must keep its record and owner",
        );
        assert_eq!(
            self.store
                .resolve_namespace(&self.owner, &self.name)
                .await
                .unwrap(),
            Some(record.id.clone()),
            "retained namespace must keep its name index",
        );
    }
}

#[tokio::test]
async fn permanent_namespace_release_refuses_without_releasing_ownership() {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    let permanent = fixture
        .store
        .promote_namespace(&record.id)
        .await
        .unwrap()
        .unwrap();
    let result = fixture.store.release_namespace(&record.id).await;
    fixture.assert_retained(&permanent).await;
    assert!(
        result.is_err(),
        "retained ownership must not report release success"
    );
    assert!(matches!(
        fixture
            .store
            .adopt_namespace(
                &fixture.name,
                &OwnerId::parse("another-owner").unwrap(),
                &NamespaceTenure::Permanent,
            )
            .await,
        Err(StoreError::AlreadyExists)
    ));
}

#[tokio::test]
async fn promotion_winning_after_release_read_preserves_record_and_index() {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    let index_revision = fixture
        .registry
        .entry(fixture.index_key())
        .await
        .unwrap()
        .unwrap()
        .revision;
    let mut pause = fixture.proxy.pause_delete(None);
    let release = fixture.release(&record);
    pause.reached().await;
    let permanent = fixture
        .store
        .promote_namespace(&record.id)
        .await
        .unwrap()
        .unwrap();
    pause.resume();
    let result = release.await.unwrap();
    fixture.assert_retained(&permanent).await;
    assert!(result.is_err(), "a lost record CAS must fail closed");
    assert_eq!(
        fixture
            .registry
            .entry(fixture.index_key())
            .await
            .unwrap()
            .unwrap()
            .revision,
        index_revision,
        "a lost record CAS must not touch its index",
    );
}

#[derive(Clone, Copy)]
enum Recovery {
    Adopt,
    Restore,
}

async fn recover_dangling_index(recovery: Recovery) {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    let original_index_revision = fixture
        .registry
        .entry(fixture.index_key())
        .await
        .unwrap()
        .unwrap()
        .revision;
    fixture
        .registry
        .delete(namespace_record_key(record.id.as_str()))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .registry
            .entry(fixture.index_key())
            .await
            .unwrap()
            .unwrap()
            .operation,
        Operation::Put
    );
    assert!(fixture
        .store
        .resolve_namespace(&fixture.owner, &fixture.name)
        .await
        .unwrap()
        .is_none());
    for expected_created in [true, false] {
        let claim = match recovery {
            Recovery::Adopt => fixture
                .store
                .adopt_namespace(&fixture.name, &fixture.owner, &record.tenure)
                .await
                .unwrap(),
            Recovery::Restore => fixture.store.restore_namespace(&record).await.unwrap(),
        };
        fixture.assert_retained(&claim.record).await;
        assert_eq!(
            claim.created, expected_created,
            "recovery must report whether this attempt recreated the row"
        );
        assert!(
            fixture
                .registry
                .entry(fixture.index_key())
                .await
                .unwrap()
                .unwrap()
                .revision
                > original_index_revision,
            "the recreated record must have a new index incarnation",
        );
    }
}

#[tokio::test]
async fn adoption_repairs_a_dangling_index_without_deleting_its_own_winner() {
    recover_dangling_index(Recovery::Adopt).await;
}

#[tokio::test]
async fn restoration_repairs_a_dangling_index_without_deleting_its_own_winner() {
    recover_dangling_index(Recovery::Restore).await;
}

enum Replacement {
    Registered,
    Adopted,
}

async fn replacement_survives_index_cleanup(replacement: Replacement) {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    let mut pause = fixture.proxy.pause_delete(Some(format!(
        "$KV.{}.{}",
        fixture.bucket,
        fixture.index_key()
    )));
    let release = fixture.release(&record);
    pause.reached().await;
    fixture
        .registry
        .delete(namespace_record_key(record.id.as_str()))
        .await
        .unwrap();
    let replacement = match replacement {
        Replacement::Registered => fixture
            .store
            .register_namespace(&fixture.name, &fixture.owner, "replacement")
            .await
            .unwrap(),
        Replacement::Adopted => fixture
            .store
            .adopt_namespace(&fixture.name, &fixture.owner, &record.tenure)
            .await
            .unwrap(),
    };
    let index_revision = fixture
        .registry
        .entry(fixture.index_key())
        .await
        .unwrap()
        .unwrap()
        .revision;
    pause.resume();
    let result = release.await.unwrap();
    fixture.assert_retained(&replacement.record).await;
    assert!(replacement.created);
    assert!(
        result.is_err(),
        "replacement index conflict must fail closed"
    );
    assert_eq!(
        fixture
            .registry
            .entry(fixture.index_key())
            .await
            .unwrap()
            .unwrap()
            .revision,
        index_revision
    );
}

#[tokio::test]
async fn release_index_cleanup_cannot_delete_a_newly_registered_namespace_index() {
    replacement_survives_index_cleanup(Replacement::Registered).await;
}

#[tokio::test]
async fn release_index_cleanup_cannot_delete_a_readopted_namespace_index() {
    replacement_survives_index_cleanup(Replacement::Adopted).await;
}

#[tokio::test]
async fn retaining_an_unindexed_permanent_registration_fences_orphan_cleanup() {
    let fixture = Fixture::new().await;
    let mut index_pause =
        fixture
            .proxy
            .pause_put(format!("$KV.{}.{}", fixture.bucket, fixture.index_key()));
    let store = fixture.store.clone();
    let name = fixture.name.clone();
    let owner = fixture.owner.clone();
    let losing_registration =
        tokio::spawn(async move { store.register_namespace(&name, &owner, "first").await });
    index_pause.reached().await;
    let winner = fixture
        .store
        .register_namespace(&fixture.name, &fixture.owner, "second")
        .await
        .unwrap();
    let mut orphan_pause = fixture.proxy.pause_delete(None);
    index_pause.resume();
    orphan_pause.reached().await;
    let orphan = fixture
        .store
        .list_namespaces()
        .await
        .unwrap()
        .into_iter()
        .find(|record| record.id != winner.record.id)
        .expect("the losing registration remains visible while cleanup is held");
    let observed = fixture.store.get_namespace(&orphan.id).await.unwrap();
    let creation_revision = fixture
        .registry
        .entry(namespace_record_key(orphan.id.as_str()))
        .await
        .unwrap()
        .unwrap()
        .revision;
    let retained = fixture
        .store
        .promote_namespace(&orphan.id)
        .await
        .unwrap()
        .unwrap();
    let retention_revision = fixture
        .registry
        .entry(namespace_record_key(orphan.id.as_str()))
        .await
        .unwrap()
        .unwrap()
        .revision;
    orphan_pause.resume();
    let settled = losing_registration.await.unwrap().unwrap();
    assert_eq!(
        fixture.store.get_namespace(&retained.id).await.unwrap(),
        Some(retained.clone()),
        "acknowledged retention must fence earlier orphan cleanup",
    );
    assert_eq!(orphan.tenure, NamespaceTenure::Permanent);
    assert_eq!(orphan.parent, fixture.owner);
    assert_eq!(observed, Some(orphan.clone()));
    assert_eq!(retained, orphan, "retention preserves the public record");
    assert!(retention_revision > creation_revision);
    assert_eq!(settled.record, winner.record);
    assert!(!settled.created);
    fixture.assert_retained(&winner.record).await;
}

#[tokio::test]
async fn provisional_release_removes_its_record_and_index_and_is_idempotent() {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    fixture.store.release_namespace(&record.id).await.unwrap();
    assert!(fixture
        .store
        .get_namespace(&record.id)
        .await
        .unwrap()
        .is_none());
    assert!(fixture
        .store
        .resolve_namespace(&fixture.owner, &fixture.name)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        fixture
            .registry
            .entry(fixture.index_key())
            .await
            .unwrap()
            .unwrap()
            .operation,
        Operation::Delete
    );
    fixture.store.release_namespace(&record.id).await.unwrap();
}

#[tokio::test]
async fn delete_removes_a_permanent_namespace_record_and_index() {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    let permanent = fixture
        .store
        .promote_namespace(&record.id)
        .await
        .unwrap()
        .unwrap();
    let removed = fixture.store.delete_namespace(&record.id).await.unwrap();
    assert_eq!(removed, permanent);
    assert!(fixture
        .store
        .get_namespace(&record.id)
        .await
        .unwrap()
        .is_none());
    assert!(fixture
        .store
        .resolve_namespace(&fixture.owner, &fixture.name)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn delete_of_a_missing_namespace_is_not_found() {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    fixture.store.release_namespace(&record.id).await.unwrap();
    assert!(matches!(
        fixture.store.delete_namespace(&record.id).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn delete_loses_to_a_concurrent_record_update_and_keeps_the_index() {
    let fixture = Fixture::new().await;
    let record = fixture.provisional().await;
    let mut pause = fixture.proxy.pause_delete(None);
    let store = fixture.store.clone();
    let id = record.id.clone();
    let delete = tokio::spawn(async move { store.delete_namespace(&id).await });
    pause.reached().await;
    let permanent = fixture
        .store
        .promote_namespace(&record.id)
        .await
        .unwrap()
        .unwrap();
    pause.resume();
    assert!(
        delete.await.unwrap().is_err(),
        "a lost CAS must fail closed"
    );
    fixture.assert_retained(&permanent).await;
}
