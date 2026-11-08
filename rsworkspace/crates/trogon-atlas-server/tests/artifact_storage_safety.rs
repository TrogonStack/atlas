#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use async_nats::jetstream::{
    self, object_store,
    stream::{Config as StreamConfig, DiscardPolicy, RetentionPolicy},
    Context,
};
use bytes::Bytes;
use futures::StreamExt;
use trogon_atlas_proto as pb;
use trogon_atlas_server::artifacts::{
    config::{ArtifactBackend, ArtifactConfig},
    storage::{BlobStorage, NatsBlobStorage},
};
use trogon_atlas_store::Store as _;

fn configuration() -> ArtifactConfig {
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    ArtifactConfig {
        artifact_backend: ArtifactBackend::Nats,
        artifact_nats_bucket: format!("safety-bytes-{suffix}"),
        artifact_catalog_bucket: format!("safety-catalog-{suffix}"),
        artifact_max_bytes: 1024,
        artifact_upload_ttl_secs: 300,
        artifact_s3_bucket: None,
        artifact_s3_region: "us-east-1".into(),
        artifact_s3_endpoint: None,
        artifact_s3_force_path_style: false,
        artifact_s3_allow_http: false,
    }
}

async fn context() -> Context {
    let nats = trogon_atlas_testsupport::shared().await;
    jetstream::new(async_nats::connect(&nats.url).await.unwrap())
}

async fn catalog_stream(context: &Context, bucket: &str) -> StreamConfig {
    context
        .create_key_value(jetstream::kv::Config {
            bucket: bucket.into(),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    context
        .get_stream(format!("KV_{bucket}"))
        .await
        .unwrap()
        .cached_info()
        .config
        .clone()
}

async fn object_stream(context: &Context, bucket: &str) -> StreamConfig {
    context
        .create_object_store(object_store::Config {
            bucket: bucket.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    context
        .get_stream(format!("OBJ_{bucket}"))
        .await
        .unwrap()
        .cached_info()
        .config
        .clone()
}

async fn open_object(context: &Context, bucket: &str) -> Result<NatsBlobStorage, tonic::Status> {
    NatsBlobStorage::open(
        context.clone(),
        object_store::Config {
            bucket: bucket.into(),
            ..Default::default()
        },
    )
    .await
}

#[tokio::test]
async fn catalog_refuses_global_discard_old_capacity() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = Arc::new(nats.store().await);
    let context = context().await;
    for (max_bytes, max_messages) in [(4096, -1), (-1, 2)] {
        let config = configuration();
        let mut stream = catalog_stream(&context, &config.artifact_catalog_bucket).await;
        stream.discard = DiscardPolicy::Old;
        stream.max_bytes = max_bytes;
        stream.max_messages = max_messages;
        context.update_stream(stream).await.unwrap();
        let error = config
            .open(&nats.url, 2 * 1024 * 1024, store.clone())
            .await
            .err()
            .expect("catalog with global DiscardOld capacity must be refused");
        assert!(error.to_string().contains("artifact catalog"));
    }
}

#[tokio::test]
async fn catalog_refuses_consumption_based_retention() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = Arc::new(nats.store().await);
    let context = context().await;
    for retention in [RetentionPolicy::Interest, RetentionPolicy::WorkQueue] {
        let config = configuration();
        let bucket = &config.artifact_catalog_bucket;
        context
            .create_stream(StreamConfig {
                name: format!("KV_{bucket}"),
                subjects: vec![format!("$KV.{bucket}.>")],
                max_messages_per_subject: 1,
                retention,
                ..Default::default()
            })
            .await
            .unwrap();
        let error = config
            .open(&nats.url, 2 * 1024 * 1024, store.clone())
            .await
            .err()
            .expect("catalog whose consumers can remove records must be refused");
        assert!(error.to_string().contains("artifact catalog"));
    }
}

#[tokio::test]
async fn catalog_allows_capacity_refusal_and_normal_per_key_history() {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = Arc::new(nats.store().await);
    let context = context().await;
    let config = configuration();
    let mut stream = catalog_stream(&context, &config.artifact_catalog_bucket).await;
    stream.discard = DiscardPolicy::New;
    stream.max_messages = 2;
    assert_eq!(stream.max_messages_per_subject, 1);
    assert!(!stream.discard_new_per_subject);
    context.update_stream(stream).await.unwrap();
    let service = config
        .open(&nats.url, 2 * 1024 * 1024, store.clone())
        .await
        .unwrap()
        .unwrap();
    let namespace = store
        .register_namespace(
            &trogon_atlas_core::NamespaceName::parse("artifact-safety").unwrap(),
            &trogon_atlas_core::OwnerId::parse("artifact-owner").unwrap(),
            "fixture",
        )
        .await
        .unwrap()
        .record
        .id;
    let content = Bytes::from_static(b"retained artifact");
    let request = pb::BeginArtifactUploadRequest {
        namespace: namespace.as_str().into(),
        filename: "capture.bin".into(),
        media_type: "application/octet-stream".into(),
        size: content.len() as u64,
        sha256: trogon_atlas_client::artifacts::sha256_hex(&content),
    };
    let upload = service.begin(request.clone(), "fixture").await.unwrap();
    service
        .upload(
            namespace.as_str(),
            &upload.upload_id,
            "fixture",
            content.clone(),
        )
        .await
        .unwrap();
    let artifact = service
        .complete(namespace.as_str(), &upload.upload_id, "fixture")
        .await
        .unwrap();
    service.begin(request.clone(), "fixture").await.unwrap();
    assert!(service.begin(request, "fixture").await.is_err());
    assert_eq!(
        service
            .get(namespace.as_str(), &artifact.artifact_id)
            .await
            .unwrap(),
        artifact
    );
    assert_eq!(
        service
            .content(namespace.as_str(), &artifact.artifact_id)
            .await
            .unwrap()
            .content,
        content
    );
}

#[tokio::test]
async fn object_store_refuses_eviction_limits() {
    let context = context().await;
    for (max_bytes, max_messages, per_subject, discard) in [
        (4096, -1, -1, DiscardPolicy::Old),
        (-1, 2, -1, DiscardPolicy::Old),
        (-1, -1, 1, DiscardPolicy::Old),
        (-1, -1, 1, DiscardPolicy::New),
    ] {
        let bucket = configuration().artifact_nats_bucket;
        let mut stream = object_stream(&context, &bucket).await;
        stream.max_bytes = max_bytes;
        stream.max_messages = max_messages;
        stream.max_messages_per_subject = per_subject;
        stream.discard = discard;
        context.update_stream(stream).await.unwrap();
        let error = open_object(&context, &bucket)
            .await
            .err()
            .expect("object stream that can evict accepted chunks must be refused");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    }
}

#[tokio::test]
async fn object_store_refuses_consumption_based_retention() {
    let context = context().await;
    for retention in [RetentionPolicy::Interest, RetentionPolicy::WorkQueue] {
        let bucket = configuration().artifact_nats_bucket;
        context
            .create_stream(StreamConfig {
                name: format!("OBJ_{bucket}"),
                subjects: vec![format!("$O.{bucket}.C.>"), format!("$O.{bucket}.M.>")],
                retention,
                ..Default::default()
            })
            .await
            .unwrap();
        let error = open_object(&context, &bucket)
            .await
            .err()
            .expect("object stream whose consumers can remove bytes must be refused");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    }
}

#[tokio::test]
async fn object_store_allows_capacity_refusal_without_losing_accepted_bytes() {
    let context = context().await;
    for per_subject in [false, true] {
        let bucket = configuration().artifact_nats_bucket;
        let mut stream = object_stream(&context, &bucket).await;
        stream.discard = DiscardPolicy::New;
        if per_subject {
            stream.max_messages_per_subject = 1;
            stream.discard_new_per_subject = true;
        } else {
            stream.max_bytes = 4096;
        }
        context.update_stream(stream).await.unwrap();
        let storage = open_object(&context, &bucket).await.unwrap();
        let accepted = Bytes::from_static(b"accepted");
        storage
            .put("artifacts/kept", accepted.clone())
            .await
            .unwrap();
        assert!(storage
            .put("staging/over-capacity", Bytes::from(vec![0x55; 300 * 1024]))
            .await
            .is_err());
        assert_eq!(storage.get("artifacts/kept", 1024).await.unwrap(), accepted);
    }
}

#[derive(Clone, Copy)]
enum CreationRace {
    BeforeCreate,
    AfterCreate,
}

async fn racing_object_context(
    mode: CreationRace,
    bucket: String,
) -> (Context, Arc<AtomicBool>, tokio::task::JoinHandle<()>) {
    let nats = trogon_atlas_testsupport::shared().await;
    let client = async_nats::connect(&nats.url).await.unwrap();
    let real = jetstream::new(client.clone());
    let prefix = format!(
        "safety_api_{}",
        trogon_atlas_testsupport::unique_suffix_for_test()
    );
    let mut requests = client.subscribe(format!("{prefix}.>")).await.unwrap();
    client.flush().await.unwrap();
    let proxied = jetstream::with_prefix(client.clone(), &prefix);
    let injected = Arc::new(AtomicBool::new(false));
    let injected_task = injected.clone();
    let task = tokio::spawn(async move {
        while let Some(request) = requests.next().await {
            let operation = request.subject.strip_prefix(&format!("{prefix}.")).unwrap();
            let creating = operation == format!("STREAM.CREATE.OBJ_{bucket}");
            if creating && matches!(mode, CreationRace::BeforeCreate) {
                real.create_object_store(object_store::Config {
                    bucket: bucket.clone(),
                    max_age: Duration::from_secs(300),
                    ..Default::default()
                })
                .await
                .unwrap();
                injected_task.store(true, Ordering::SeqCst);
            }
            let response = client
                .request(format!("$JS.API.{operation}"), request.payload)
                .await
                .unwrap();
            if creating && matches!(mode, CreationRace::AfterCreate) {
                let mut stream = real
                    .get_stream(format!("OBJ_{bucket}"))
                    .await
                    .unwrap()
                    .cached_info()
                    .config
                    .clone();
                stream.max_age = Duration::from_secs(300);
                real.update_stream(stream).await.unwrap();
                injected_task.store(true, Ordering::SeqCst);
            }
            client
                .publish(request.reply.unwrap(), response.payload)
                .await
                .unwrap();
        }
    });
    (proxied, injected, task)
}

#[tokio::test]
async fn object_store_rechecks_effective_ttl_after_creation_races() {
    for mode in [CreationRace::BeforeCreate, CreationRace::AfterCreate] {
        let bucket = configuration().artifact_nats_bucket;
        let (context, injected, task) = racing_object_context(mode, bucket.clone()).await;
        let result = open_object(&context, &bucket).await;
        task.abort();
        assert!(
            injected.load(Ordering::SeqCst),
            "creation race was not exercised"
        );
        let error = result
            .err()
            .expect("effective TTL introduced during creation must be refused");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        assert!(error.message().contains("expire"));
    }
}
