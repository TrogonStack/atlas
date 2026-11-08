use std::{sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use async_nats::jetstream::{
    context::GetStreamErrorKind,
    stream::{DiscardPolicy, RetentionPolicy},
    ErrorCode,
};
use clap::{Args, ValueEnum};

use super::{
    storage::{BlobStorage, NatsBlobStorage, S3BlobStorage, S3StorageConfig},
    ArtifactService, MAX_ARTIFACT_BYTES,
};

#[derive(Clone, Copy, Debug, Default, ValueEnum, PartialEq, Eq)]
pub enum ArtifactBackend {
    #[default]
    Disabled,
    Nats,
    S3,
}

#[derive(Clone, Debug, Args)]
pub struct ArtifactConfig {
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_BACKEND",
        default_value = "disabled"
    )]
    pub artifact_backend: ArtifactBackend,
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_NATS_BUCKET",
        default_value = "atlas_artifacts"
    )]
    pub artifact_nats_bucket: String,
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_CATALOG_BUCKET",
        default_value = "atlas_artifact_catalog"
    )]
    pub artifact_catalog_bucket: String,
    #[arg(long, env = "TROGON_ATLAS_ARTIFACT_MAX_BYTES", default_value_t = 16 * 1024 * 1024)]
    pub artifact_max_bytes: u64,
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_UPLOAD_TTL_SECS",
        default_value_t = 900
    )]
    pub artifact_upload_ttl_secs: u64,
    #[arg(long, env = "TROGON_ATLAS_ARTIFACT_S3_BUCKET")]
    pub artifact_s3_bucket: Option<String>,
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_S3_REGION",
        default_value = "us-east-1"
    )]
    pub artifact_s3_region: String,
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_S3_ENDPOINT",
        hide_env_values = true
    )]
    pub artifact_s3_endpoint: Option<String>,
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_S3_FORCE_PATH_STYLE",
        default_value_t = false
    )]
    pub artifact_s3_force_path_style: bool,
    #[arg(
        long,
        env = "TROGON_ATLAS_ARTIFACT_S3_ALLOW_HTTP",
        default_value_t = false
    )]
    pub artifact_s3_allow_http: bool,
}

impl ArtifactConfig {
    /// Retained files must remain readable after upload limits are lowered.
    pub fn max_encoding_message_bytes(&self, max_message_bytes: usize) -> Result<usize> {
        if self.artifact_backend == ArtifactBackend::Disabled {
            return Ok(max_message_bytes);
        }
        let retained_response_bytes = usize::try_from(MAX_ARTIFACT_BYTES + 65536)
            .context("artifact response limit exceeds platform capacity")?;
        Ok(max_message_bytes.max(retained_response_bytes))
    }

    pub async fn open(
        &self,
        nats_url: &str,
        max_message_bytes: usize,
        store: Arc<dyn trogon_atlas_store::Store>,
    ) -> Result<Option<Arc<ArtifactService>>> {
        if self.artifact_backend == ArtifactBackend::Disabled {
            return Ok(None);
        }
        if self.artifact_max_bytes == 0
            || self.artifact_max_bytes > MAX_ARTIFACT_BYTES
            || self.artifact_max_bytes.saturating_add(65536) > max_message_bytes as u64
        {
            bail!(
                "artifact max bytes must be 1..=32 MiB and fit max-message-bytes with 64 KiB overhead"
            );
        }
        super::validate_configuration(
            self.artifact_max_bytes,
            Duration::from_secs(self.artifact_upload_ttl_secs),
        )?;
        let authority = store.artifact_publication_authority()?;
        authority.validate().await?;
        let connection = trogon_atlas_store::NatsStoreConfig::new(
            nats_url,
            trogon_atlas_store::StoreMetrics::default(),
        );
        let client = connection
            .connection_options()
            .connection_timeout(Duration::from_secs(10))
            .connect(&connection.url)
            .await
            .map_err(|_| anyhow::anyhow!("artifact NATS connection failed"))?;
        let js = async_nats::jetstream::new(client);
        match js
            .get_stream(format!("KV_{}", self.artifact_catalog_bucket))
            .await
        {
            Ok(_) => {}
            Err(error) => match error.kind() {
                GetStreamErrorKind::JetStream(error)
                    if error.error_code() == ErrorCode::STREAM_NOT_FOUND =>
                {
                    let created = js
                        .create_key_value(async_nats::jetstream::kv::Config {
                            bucket: self.artifact_catalog_bucket.clone(),
                            history: 1,
                            max_value_size: 16 * 1024,
                            ..Default::default()
                        })
                        .await;
                    if created.is_err()
                        && js
                            .get_key_value(&self.artifact_catalog_bucket)
                            .await
                            .is_err()
                    {
                        bail!("cannot create artifact catalog");
                    }
                }
                _ => bail!("artifact catalog unavailable"),
            },
        }
        let catalog = js
            .get_key_value(&self.artifact_catalog_bucket)
            .await
            .map_err(|_| anyhow::anyhow!("artifact catalog unavailable"))?;
        let stream = js
            .get_stream(format!("KV_{}", self.artifact_catalog_bucket))
            .await
            .map_err(|_| anyhow::anyhow!("artifact catalog status unavailable"))?;
        let stream_config = &stream.cached_info().config;
        if stream_config.max_messages_per_subject != 1
            || !stream_config.max_age.is_zero()
            || stream_config.retention != RetentionPolicy::Limits
            || (stream_config.discard == DiscardPolicy::Old
                && (stream_config.max_bytes > 0 || stream_config.max_messages > 0))
        {
            bail!("artifact catalog must retain records without expiry or eviction with history=1");
        }
        let blobs: Arc<dyn BlobStorage> = match self.artifact_backend {
            ArtifactBackend::Nats => Arc::new(
                NatsBlobStorage::open(
                    js,
                    async_nats::jetstream::object_store::Config {
                        bucket: self.artifact_nats_bucket.clone(),
                        ..Default::default()
                    },
                )
                .await?,
            ),
            ArtifactBackend::S3 => Arc::new(S3BlobStorage::new(S3StorageConfig {
                bucket: self
                    .artifact_s3_bucket
                    .clone()
                    .context("artifact-s3-bucket is required for S3")?,
                region: self.artifact_s3_region.clone(),
                endpoint: self.artifact_s3_endpoint.clone(),
                force_path_style: self.artifact_s3_force_path_style,
                allow_http: self.artifact_s3_allow_http,
            })?),
            ArtifactBackend::Disabled => return Ok(None),
        };
        Ok(Some(Arc::new(ArtifactService::new(
            blobs,
            catalog,
            self.artifact_max_bytes,
            Duration::from_secs(self.artifact_upload_ttl_secs),
            authority,
        )?)))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use clap::Parser;
    use testcontainers::{
        core::{IntoContainerPort, WaitFor},
        runners::AsyncRunner,
        GenericImage, ImageExt,
    };

    use super::*;

    #[derive(Parser)]
    struct TestArgs {
        #[command(flatten)]
        artifacts: ArtifactConfig,
    }

    fn configuration() -> ArtifactConfig {
        let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
        TestArgs::parse_from([
            "atlas",
            "--artifact-backend",
            "nats",
            "--artifact-max-bytes",
            "1024",
            "--artifact-catalog-bucket",
            &format!("artifact-config-{suffix}"),
            "--artifact-nats-bucket",
            &format!("artifact-config-bytes-{suffix}"),
        ])
        .artifacts
    }

    #[tokio::test]
    async fn authenticated_nats_configuration_and_concurrent_catalog_creation_work() {
        let container = GenericImage::new("nats", "2.15.2")
            .with_wait_for(WaitFor::message_on_stderr("Server is ready"))
            .with_exposed_port(4222.tcp())
            .with_copy_to(
                "/etc/atlas-test-nats.conf",
                include_bytes!("../../../trogon-atlas-testsupport/nats-server.conf").to_vec(),
            )
            .with_cmd([
                "-c",
                "/etc/atlas-test-nats.conf",
                "--user",
                "artifact-fixture-user",
                "--pass",
                "artifact-fixture-password",
            ])
            .start()
            .await
            .unwrap();
        let port = container.get_host_port_ipv4(4222).await.unwrap();
        let url =
            format!("nats://artifact-fixture-user:artifact-fixture-password@127.0.0.1:{port}");
        let store = Arc::new(
            trogon_atlas_store::NatsStore::connect(
                &url,
                trogon_atlas_store::StoreMetrics::default(),
            )
            .await
            .unwrap(),
        );
        let config = configuration();
        let (first, second) = tokio::join!(
            config.open(&url, 2 * 1024 * 1024, store.clone()),
            config.open(&url, 2 * 1024 * 1024, store.clone())
        );
        assert!(first.unwrap().unwrap().info().enabled);
        assert!(second.unwrap().unwrap().info().enabled);
    }

    #[tokio::test]
    async fn invalid_configuration_refuses_before_connecting_to_storage() {
        let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
        for ttl in [0, 3601] {
            let mut config = configuration();
            config.artifact_upload_ttl_secs = ttl;
            let error = config
                .open("nats://127.0.0.1:1", 2 * 1024 * 1024, store.clone())
                .await
                .err()
                .unwrap();
            assert!(error
                .to_string()
                .contains("invalid artifact service configuration"));
        }
    }
}
