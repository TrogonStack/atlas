use std::{collections::HashMap, env, time::Duration};

use async_nats::jetstream::{
    context::GetStreamErrorKind,
    object_store::{GetErrorKind, ObjectStore as NatsObjectStore},
    stream::{DiscardPolicy, RetentionPolicy},
    Context, ErrorCode,
};
use async_trait::async_trait;
use aws_sdk_s3::{
    config::{
        endpoint::{Endpoint, EndpointFuture, Params, ResolveEndpoint},
        Builder as S3ConfigBuilder, Region, RequestChecksumCalculation,
    },
    presigning::PresigningConfig,
    primitives::ByteStream,
    types::{RequestPayer, ServerSideEncryption},
    Client as S3Client,
};
use base64::Engine as _;
use bytes::{Bytes, BytesMut};
use md5::{Digest as _, Md5};
use reqwest::Url;
use tokio::io::AsyncReadExt as _;
use tonic::Status;

pub struct SignedUpload {
    pub url: String,
    pub headers: HashMap<String, String>,
}

#[async_trait]
pub trait BlobStorage: Send + Sync {
    fn backend_name(&self) -> &'static str;
    async fn put(&self, key: &str, data: Bytes) -> Result<(), Status>;
    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status>;
    async fn delete(&self, key: &str) -> Result<(), Status>;
    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        size: u64,
        sha256: &str,
        media_type: &str,
    ) -> Result<Option<SignedUpload>, Status>;
    async fn presign_get(
        &self,
        key: &str,
        ttl: Duration,
        media_type: &str,
    ) -> Result<Option<String>, Status>;
}

pub struct NatsBlobStorage {
    bucket: NatsObjectStore,
}

impl NatsBlobStorage {
    pub async fn open(
        context: Context,
        config: async_nats::jetstream::object_store::Config,
    ) -> Result<Self, Status> {
        if !config.max_age.is_zero() {
            return Err(Status::invalid_argument(
                "artifact object storage cannot expire accepted content",
            ));
        }
        match context.get_stream(format!("OBJ_{}", config.bucket)).await {
            Ok(_) => {}
            Err(error) => match error.kind() {
                GetStreamErrorKind::JetStream(error)
                    if error.error_code() == ErrorCode::STREAM_NOT_FOUND =>
                {
                    if context.create_object_store(config.clone()).await.is_err() {
                        context
                            .get_object_store(&config.bucket)
                            .await
                            .map_err(|_| unavailable())?;
                    }
                }
                _ => return Err(unavailable()),
            },
        }
        let bucket = context
            .get_object_store(&config.bucket)
            .await
            .map_err(|_| unavailable())?;
        let stream = context
            .get_stream(format!("OBJ_{}", config.bucket))
            .await
            .map_err(|_| unavailable())?;
        let stream_config = &stream.cached_info().config;
        if stream_config.subject_transform.is_some() {
            return Err(Status::failed_precondition(
                "artifact object storage cannot transform record subjects",
            ));
        }
        if !stream_config.max_age.is_zero() {
            return Err(Status::failed_precondition(
                "artifact object storage cannot expire accepted content",
            ));
        }
        let global_eviction = stream_config.discard == DiscardPolicy::Old
            && (stream_config.max_bytes > 0 || stream_config.max_messages > 0);
        let chunk_eviction = stream_config.max_messages_per_subject > 0
            && !(stream_config.discard == DiscardPolicy::New
                && stream_config.discard_new_per_subject);
        if stream_config.retention != RetentionPolicy::Limits || global_eviction || chunk_eviction {
            return Err(Status::failed_precondition(
                "artifact object storage must retain accepted content without eviction",
            ));
        }
        Ok(Self { bucket })
    }
}

#[async_trait]
impl BlobStorage for NatsBlobStorage {
    fn backend_name(&self) -> &'static str {
        "nats"
    }

    async fn put(&self, key: &str, data: Bytes) -> Result<(), Status> {
        validate_key(key)?;
        self.bucket
            .put(key, &mut data.as_ref())
            .await
            .map_err(|_| unavailable())?;
        Ok(())
    }

    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status> {
        validate_key(key)?;
        let object = self.bucket.get(key).await.map_err(|error| {
            if error.kind() == GetErrorKind::NotFound {
                not_found()
            } else {
                unavailable()
            }
        })?;
        if object.info.size > max_bytes {
            return Err(too_large());
        }
        let read_limit = u64::try_from(max_bytes)
            .map_err(|_| too_large())?
            .checked_add(1)
            .ok_or_else(too_large)?;
        let mut bytes = Vec::with_capacity(object.info.size);
        object
            .take(read_limit)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| unavailable())?;
        if bytes.len() > max_bytes {
            return Err(too_large());
        }
        Ok(bytes.into())
    }

    async fn delete(&self, key: &str) -> Result<(), Status> {
        validate_key(key)?;
        if let Err(error) = self.bucket.get(key).await {
            if error.kind() == GetErrorKind::NotFound {
                return Ok(());
            }
            return Err(unavailable());
        }
        self.bucket.delete(key).await.map_err(|_| unavailable())
    }

    async fn presign_put(
        &self,
        _key: &str,
        _ttl: Duration,
        _size: u64,
        _sha256: &str,
        _media_type: &str,
    ) -> Result<Option<SignedUpload>, Status> {
        Ok(None)
    }

    async fn presign_get(
        &self,
        _key: &str,
        _ttl: Duration,
        _media_type: &str,
    ) -> Result<Option<String>, Status> {
        Ok(None)
    }
}

pub struct S3StorageConfig {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub force_path_style: bool,
    pub allow_http: bool,
}

const S3_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
struct S3RequestSettings {
    server_side_encryption: Option<ServerSideEncryption>,
    sse_kms_key_id: Option<String>,
    sse_customer_key: Option<String>,
    sse_customer_key_md5: Option<String>,
    request_payer: bool,
}

pub struct S3BlobStorage {
    client: S3Client,
    bucket: String,
    settings: S3RequestSettings,
}

#[derive(Debug)]
struct S3BucketEndpoint(Endpoint);

impl ResolveEndpoint for S3BucketEndpoint {
    fn resolve_endpoint(&self, _params: &Params) -> EndpointFuture<'_> {
        EndpointFuture::ready(Ok(self.0.clone()))
    }
}

impl S3BlobStorage {
    pub async fn new(config: S3StorageConfig) -> Result<Self, Status> {
        let inherited_endpoint = ["AWS_ENDPOINT_URL_S3", "AWS_ENDPOINT_URL", "AWS_ENDPOINT"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok());
        let settings = S3RequestSettings::from_env()?;
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .load()
            .await;
        let credentials = shared.credentials_provider().ok_or_else(unavailable)?;
        Self::with_builder_and_settings(
            config,
            S3ConfigBuilder::new().credentials_provider(credentials),
            inherited_endpoint,
            settings,
        )
    }

    #[cfg(test)]
    fn with_builder(
        config: S3StorageConfig,
        builder: S3ConfigBuilder,
        inherited_endpoint: Option<String>,
    ) -> Result<Self, Status> {
        Self::with_builder_and_settings(
            config,
            builder,
            inherited_endpoint,
            S3RequestSettings::default(),
        )
    }

    fn with_builder_and_settings(
        config: S3StorageConfig,
        builder: S3ConfigBuilder,
        inherited_endpoint: Option<String>,
        settings: S3RequestSettings,
    ) -> Result<Self, Status> {
        if config.bucket.trim().is_empty() || config.region.trim().is_empty() {
            return Err(Status::invalid_argument(
                "artifact S3 bucket and region are required",
            ));
        }
        let mut builder = builder
            .behavior_version_latest()
            .region(Region::new(config.region))
            .force_path_style(config.force_path_style)
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired);
        if let Some(endpoint) = config.endpoint.or(inherited_endpoint) {
            validate_endpoint(&endpoint, config.allow_http)?;
            builder = if config.force_path_style {
                builder.endpoint_url(endpoint)
            } else {
                // A custom virtual-hosted endpoint already identifies the bucket.
                builder
                    .endpoint_resolver(S3BucketEndpoint(Endpoint::builder().url(endpoint).build()))
            };
        }
        Ok(Self {
            client: S3Client::from_conf(builder.build()),
            bucket: config.bucket,
            settings,
        })
    }
}

impl S3RequestSettings {
    fn sse_customer_headers(&self) -> Option<(&str, &str)> {
        self.sse_customer_key
            .as_deref()
            .zip(self.sse_customer_key_md5.as_deref())
    }

    fn from_env() -> Result<Self, Status> {
        let encryption = env::var("AWS_SERVER_SIDE_ENCRYPTION").ok();
        let sse_kms_key_id = env::var("AWS_SSE_KMS_KEY_ID").ok();
        let customer_key = env::var("AWS_SSE_CUSTOMER_KEY_BASE64").ok();
        let (server_side_encryption, sse_customer_key, sse_customer_key_md5) = match encryption
            .as_deref()
        {
            Some(value) if value.eq_ignore_ascii_case("sse-c") => {
                let key = customer_key.ok_or_else(|| {
                    Status::invalid_argument("AWS_SSE_CUSTOMER_KEY_BASE64 is required for SSE-C")
                })?;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(key.as_bytes())
                    .map_err(|_| {
                        Status::invalid_argument("AWS_SSE_CUSTOMER_KEY_BASE64 must be valid base64")
                    })?;
                let digest = Md5::digest(decoded);
                (
                    None,
                    Some(key),
                    Some(base64::engine::general_purpose::STANDARD.encode(digest)),
                )
            }
            Some(value) => {
                (
                    Some(ServerSideEncryption::try_parse(value).map_err(|_| {
                        Status::invalid_argument("invalid AWS_SERVER_SIDE_ENCRYPTION")
                    })?),
                    None,
                    None,
                )
            }
            None => (None, None, None),
        };
        let request_payer = env::var("AWS_REQUEST_PAYER").is_ok_and(|value| {
            value.eq_ignore_ascii_case("requester")
                || value == "1"
                || value.eq_ignore_ascii_case("true")
        });
        Ok(Self {
            server_side_encryption,
            sse_kms_key_id,
            sse_customer_key,
            sse_customer_key_md5,
            request_payer,
        })
    }
}

#[async_trait]
impl BlobStorage for S3BlobStorage {
    fn backend_name(&self) -> &'static str {
        "s3"
    }

    async fn put(&self, key: &str, data: Bytes) -> Result<(), Status> {
        validate_key(key)?;
        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data));
        if let Some(encryption) = &self.settings.server_side_encryption {
            request = request.server_side_encryption(encryption.clone());
            if encryption.as_str() == "aws:kms" {
                if let Some(key_id) = self.settings.sse_kms_key_id.as_deref() {
                    request = request.ssekms_key_id(key_id);
                }
            }
        }
        if let Some((key, md5)) = self.settings.sse_customer_headers() {
            request = request
                .sse_customer_algorithm("AES256")
                .sse_customer_key(key)
                .sse_customer_key_md5(md5);
        }
        if self.settings.request_payer {
            request = request.request_payer(RequestPayer::Requester);
        }
        request.send().await.map_err(|_| unavailable())?;
        Ok(())
    }

    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status> {
        validate_key(key)?;
        tokio::time::timeout(S3_READ_TIMEOUT, async {
            let mut request = self.client.get_object().bucket(&self.bucket).key(key);
            if let Some((key, md5)) = self.settings.sse_customer_headers() {
                request = request
                    .sse_customer_algorithm("AES256")
                    .sse_customer_key(key)
                    .sse_customer_key_md5(md5);
            }
            if self.settings.request_payer {
                request = request.request_payer(RequestPayer::Requester);
            }
            let result = request.send().await.map_err(|error| {
                if error
                    .raw_response()
                    .is_some_and(|r| r.status().as_u16() == 404)
                {
                    not_found()
                } else {
                    unavailable()
                }
            })?;
            let length = result
                .content_length()
                .and_then(|length| usize::try_from(length).ok())
                .ok_or_else(unavailable)?;
            if length > max_bytes {
                return Err(too_large());
            }
            let mut stream = result.body;
            let mut bytes = BytesMut::new();
            while let Some(chunk) = stream.try_next().await.map_err(|_| unavailable())? {
                if chunk.len() > max_bytes.saturating_sub(bytes.len()) {
                    return Err(too_large());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes.freeze())
        })
        .await
        .map_err(|_| unavailable())?
    }

    async fn delete(&self, key: &str) -> Result<(), Status> {
        validate_key(key)?;
        let mut request = self.client.delete_object().bucket(&self.bucket).key(key);
        if self.settings.request_payer {
            request = request.request_payer(RequestPayer::Requester);
        }
        match request.send().await {
            Ok(_) => Ok(()),
            Err(error)
                if error
                    .raw_response()
                    .is_some_and(|r| r.status().as_u16() == 404) =>
            {
                Ok(())
            }
            Err(_) => Err(unavailable()),
        }
    }

    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        size: u64,
        sha256: &str,
        media_type: &str,
    ) -> Result<Option<SignedUpload>, Status> {
        validate_key(key)?;
        validate_ttl(ttl)?;
        let length = i64::try_from(size).map_err(|_| too_large())?;
        let checksum = base64::engine::general_purpose::STANDARD.encode(decode_sha256(sha256)?);
        let expiry = PresigningConfig::expires_in(ttl).map_err(|_| unavailable())?;
        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_length(length)
            .content_type(media_type)
            .checksum_sha256(checksum);
        if let Some(encryption) = &self.settings.server_side_encryption {
            request = request.server_side_encryption(encryption.clone());
            if encryption.as_str() == "aws:kms" {
                if let Some(key_id) = self.settings.sse_kms_key_id.as_deref() {
                    request = request.ssekms_key_id(key_id);
                }
            }
        }
        if let Some((key, md5)) = self.settings.sse_customer_headers() {
            request = request
                .sse_customer_algorithm("AES256")
                .sse_customer_key(key)
                .sse_customer_key_md5(md5);
        }
        if self.settings.request_payer {
            request = request.request_payer(RequestPayer::Requester);
        }
        let request = request.presigned(expiry).await.map_err(|_| unavailable())?;
        Ok(Some(SignedUpload {
            url: request.uri().to_string(),
            headers: request
                .headers()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        }))
    }

    async fn presign_get(
        &self,
        key: &str,
        ttl: Duration,
        media_type: &str,
    ) -> Result<Option<String>, Status> {
        validate_key(key)?;
        validate_ttl(ttl)?;
        media_type
            .parse::<http::HeaderValue>()
            .map_err(|_| Status::invalid_argument("invalid artifact media type"))?;
        if self.settings.sse_customer_headers().is_some() {
            return Ok(None);
        }
        let disposition = if matches!(
            media_type,
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            "inline"
        } else {
            "attachment"
        };
        let expiry = PresigningConfig::expires_in(ttl).map_err(|_| unavailable())?;
        let mut request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .response_content_type(media_type)
            .response_content_disposition(disposition);
        if self.settings.request_payer {
            request = request.request_payer(RequestPayer::Requester);
        }
        request
            .presigned(expiry)
            .await
            .map(|request| Some(request.uri().to_string()))
            .map_err(|_| unavailable())
    }
}

fn validate_key(key: &str) -> Result<(), Status> {
    if key.is_empty()
        || key.len() > 1024
        || key.starts_with('/')
        || key
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !key.bytes().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, b'/' | b'-' | b'_' | b'.')
        })
    {
        return Err(Status::invalid_argument("invalid artifact storage key"));
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str, allow_http: bool) -> Result<(), Status> {
    let url = Url::parse(endpoint)
        .map_err(|_| Status::invalid_argument("invalid artifact S3 endpoint"))?;
    if (url.scheme() != "https" && !(allow_http && url.scheme() == "http"))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Status::invalid_argument("invalid artifact S3 endpoint"));
    }
    Ok(())
}

fn validate_ttl(ttl: Duration) -> Result<(), Status> {
    if ttl.as_secs() == 0 || ttl > Duration::from_hours(168) {
        return Err(Status::invalid_argument("invalid artifact URL lifetime"));
    }
    Ok(())
}

fn decode_sha256(value: &str) -> Result<[u8; 32], Status> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Status::invalid_argument("invalid artifact checksum"));
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| Status::invalid_argument("invalid artifact checksum"))?;
    }
    Ok(digest)
}

fn not_found() -> Status {
    Status::not_found("artifact content not found")
}

fn unavailable() -> Status {
    Status::unavailable("artifact storage is unavailable")
}

fn too_large() -> Status {
    Status::resource_exhausted("artifact content exceeds the configured size limit")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use sha2::Sha256;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use super::*;

    fn fixture_builder() -> S3ConfigBuilder {
        use aws_sdk_s3::config::{retry::RetryConfig, Credentials};
        S3ConfigBuilder::new()
            .credentials_provider(Credentials::new(
                "artifact-fixture-access",
                "artifact-fixture-signing-key",
                None,
                None,
                "fixture",
            ))
            .retry_config(RetryConfig::disabled())
    }

    fn fixture_config(endpoint: Option<String>) -> S3StorageConfig {
        S3StorageConfig {
            bucket: "artifact-test".into(),
            region: "us-east-1".into(),
            endpoint,
            force_path_style: true,
            allow_http: true,
        }
    }

    #[tokio::test]
    async fn explicit_s3_endpoint_overrides_inherited_provider_endpoint() {
        let storage = S3BlobStorage::with_builder(
            fixture_config(Some("https://selected.invalid".into())),
            fixture_builder(),
            Some("https://inherited.invalid".into()),
        )
        .unwrap();
        let signed = storage
            .presign_get("artifacts/capture", Duration::from_secs(60), "image/png")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            Url::parse(&signed).unwrap().host_str(),
            Some("selected.invalid")
        );
    }

    #[tokio::test]
    async fn public_s3_constructor_preserves_environment_endpoint_precedence() {
        const EXPECTED: &str = "ATLAS_TEST_S3_ENDPOINT_HOST";
        if let Ok(expected) = std::env::var(EXPECTED) {
            let endpoint =
                (expected == "explicit.invalid").then(|| "https://explicit.invalid".into());
            let storage = S3BlobStorage::new(fixture_config(endpoint)).await.unwrap();
            let signed = storage
                .presign_get("artifacts/capture", Duration::from_secs(60), "image/png")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                Url::parse(&signed).unwrap().host_str(),
                Some(expected.as_str())
            );
            return;
        }

        for (expected, generic, legacy, service) in [
            (
                "generic.invalid",
                Some("https://generic.invalid"),
                None,
                None,
            ),
            ("legacy.invalid", None, Some("https://legacy.invalid"), None),
            (
                "generic.invalid",
                Some("https://generic.invalid"),
                Some("https://legacy.invalid"),
                None,
            ),
            (
                "service.invalid",
                Some("https://generic.invalid"),
                Some("https://legacy.invalid"),
                Some("https://service.invalid"),
            ),
            (
                "explicit.invalid",
                Some("https://generic.invalid"),
                Some("https://legacy.invalid"),
                Some("https://service.invalid"),
            ),
        ] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "artifacts::storage::tests::public_s3_constructor_preserves_environment_endpoint_precedence"])
                .env_clear()
                .env(EXPECTED, expected)
                .env("AWS_ACCESS_KEY_ID", "artifact-fixture-access")
                .env("AWS_SECRET_ACCESS_KEY", "artifact-fixture-signing-key")
                .env("AWS_EC2_METADATA_DISABLED", "true");
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                command.env("LLVM_PROFILE_FILE", profile);
            }
            for (name, value) in [
                ("AWS_ENDPOINT_URL", generic),
                ("AWS_ENDPOINT", legacy),
                ("AWS_ENDPOINT_URL_S3", service),
            ] {
                if let Some(value) = value {
                    command.env(name, value);
                }
            }
            let output = command.output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("1 passed; 0 failed"),
                "expected one successful endpoint probe for {expected}: {stdout}",
            );
        }
    }

    #[tokio::test]
    async fn s3_virtual_hosted_endpoints_keep_their_bucket_destination() {
        for (endpoint, expected_host, expected_path) in [
            (
                Some("https://artifact-test.objects.invalid/tenant"),
                "artifact-test.objects.invalid",
                "/tenant/artifacts/capture",
            ),
            (
                Some("https://objects.invalid/tenant"),
                "objects.invalid",
                "/tenant/artifacts/capture",
            ),
            (
                None,
                "artifact-test.s3.us-east-1.amazonaws.com",
                "/artifacts/capture",
            ),
        ] {
            let mut config = fixture_config(endpoint.map(str::to_owned));
            config.force_path_style = false;
            let storage = S3BlobStorage::with_builder(config, fixture_builder(), None).unwrap();
            let signed = storage
                .presign_get("artifacts/capture", Duration::from_secs(60), "image/png")
                .await
                .unwrap()
                .unwrap();
            let url = Url::parse(&signed).unwrap();
            assert_eq!(url.host_str(), Some(expected_host));
            assert_eq!(url.path(), expected_path);
        }
    }

    #[tokio::test]
    async fn s3_virtual_hosted_custom_endpoint_handles_direct_and_presigned_writes() {
        let server = MockServer::start().await;
        let mut config = fixture_config(Some(format!("{}/tenant", server.uri())));
        config.force_path_style = false;
        let storage = S3BlobStorage::with_builder(config, fixture_builder(), None).unwrap();
        Mock::given(method("PUT"))
            .and(path("/tenant/artifacts/capture"))
            .respond_with(ResponseTemplate::new(200).insert_header("etag", "\"stored\""))
            .expect(2)
            .mount(&server)
            .await;

        let data = Bytes::from_static(b"artifact bytes");
        let digest = trogon_atlas_client::artifacts::sha256_hex(&data);
        storage
            .put("artifacts/capture", data.clone())
            .await
            .unwrap();
        let signed = storage
            .presign_put(
                "artifacts/capture",
                Duration::from_secs(60),
                u64::try_from(data.len()).unwrap(),
                &digest,
                "image/png",
            )
            .await
            .unwrap()
            .unwrap();
        let mut request = reqwest::Client::new().put(signed.url).body(data);
        for (name, value) in signed.headers {
            request = request.header(name, value);
        }
        assert!(request.send().await.unwrap().status().is_success());
    }

    #[tokio::test]
    async fn s3_reads_and_writes_preserve_request_payer_and_sse_customer_headers() {
        let server = MockServer::start().await;
        let key =
            base64::engine::general_purpose::STANDARD.encode(b"fixture customer encryption key");
        let key_md5 = base64::engine::general_purpose::STANDARD.encode(Md5::digest(
            base64::engine::general_purpose::STANDARD
                .decode(&key)
                .unwrap(),
        ));
        let storage = S3BlobStorage::with_builder_and_settings(
            fixture_config(Some(server.uri())),
            fixture_builder(),
            None,
            S3RequestSettings {
                sse_customer_key: Some(key.clone()),
                sse_customer_key_md5: Some(key_md5.clone()),
                request_payer: true,
                ..Default::default()
            },
        )
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/artifact-test/artifacts/capture"))
            .and(header(
                "x-amz-server-side-encryption-customer-algorithm",
                "AES256",
            ))
            .and(header(
                "x-amz-server-side-encryption-customer-key",
                key.as_str(),
            ))
            .and(header(
                "x-amz-server-side-encryption-customer-key-MD5",
                key_md5.as_str(),
            ))
            .and(header("x-amz-request-payer", "requester"))
            .respond_with(object_response(b"existing artifact"))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/artifact-test/artifacts/capture"))
            .and(header(
                "x-amz-server-side-encryption-customer-algorithm",
                "AES256",
            ))
            .and(header(
                "x-amz-server-side-encryption-customer-key",
                key.as_str(),
            ))
            .and(header(
                "x-amz-server-side-encryption-customer-key-MD5",
                key_md5.as_str(),
            ))
            .and(header("x-amz-request-payer", "requester"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        assert_eq!(
            storage.get("artifacts/capture", 1024).await.unwrap(),
            Bytes::from_static(b"existing artifact")
        );
        storage
            .put("artifacts/capture", Bytes::from_static(b"replacement"))
            .await
            .unwrap();
        assert!(storage
            .presign_get("artifacts/capture", Duration::from_secs(60), "image/png")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn s3_kms_key_is_bound_to_direct_and_presigned_uploads() {
        let server = MockServer::start().await;
        let storage = S3BlobStorage::with_builder_and_settings(
            fixture_config(Some(server.uri())),
            fixture_builder(),
            None,
            S3RequestSettings {
                server_side_encryption: Some(ServerSideEncryption::AwsKms),
                sse_kms_key_id: Some("alias/artifact-fixture".into()),
                ..Default::default()
            },
        )
        .unwrap();
        Mock::given(method("PUT"))
            .and(path("/artifact-test/artifacts/capture"))
            .and(header("x-amz-server-side-encryption", "aws:kms"))
            .and(header(
                "x-amz-server-side-encryption-aws-kms-key-id",
                "alias/artifact-fixture",
            ))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        storage
            .put("artifacts/capture", Bytes::from_static(b"kms artifact"))
            .await
            .unwrap();
        let signed = storage
            .presign_put(
                "artifacts/capture",
                Duration::from_secs(60),
                12,
                &trogon_atlas_client::artifacts::sha256_hex(b"kms artifact"),
                "application/octet-stream",
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            signed
                .headers
                .get("x-amz-server-side-encryption-aws-kms-key-id")
                .map(String::as_str),
            Some("alias/artifact-fixture")
        );
    }

    #[test]
    fn inherited_s3_endpoint_cannot_contain_url_credentials() {
        let result = S3BlobStorage::with_builder(
            fixture_config(None),
            fixture_builder(),
            Some("https://fixture-user:fixture-password@storage.invalid".into()),
        );
        assert_eq!(
            result.err().map(|error| error.code()),
            Some(tonic::Code::InvalidArgument)
        );
    }

    async fn s3_fixture() -> (MockServer, S3BlobStorage) {
        let server = MockServer::start().await;
        let storage = S3BlobStorage::with_builder(
            fixture_config(Some(server.uri())),
            fixture_builder(),
            None,
        )
        .unwrap();
        (server, storage)
    }

    fn object_response(data: &[u8]) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_bytes(data)
            .insert_header("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT")
            .insert_header("etag", "\"artifact-fixture-etag\"")
    }

    #[tokio::test]
    async fn nats_object_store_roundtrip_preserves_bytes_and_rejects_oversize_reads() {
        let server = trogon_atlas_testsupport::shared().await;
        let client = async_nats::connect(&server.url).await.unwrap();
        let context = async_nats::jetstream::new(client);
        let config = async_nats::jetstream::object_store::Config {
            bucket: format!(
                "artifact-{}",
                trogon_atlas_testsupport::unique_suffix_for_test()
            ),
            max_bytes: 2 * 1024 * 1024,
            ..Default::default()
        };
        let storage = NatsBlobStorage::open(context.clone(), config.clone())
            .await
            .unwrap();
        let reopened = NatsBlobStorage::open(context.clone(), config.clone())
            .await
            .unwrap();
        let data = Bytes::from(vec![0xa5; 300 * 1024]);
        storage.put("staging/capture", data.clone()).await.unwrap();
        assert_eq!(
            reopened.get("staging/capture", data.len()).await.unwrap(),
            data
        );
        assert_eq!(
            storage
                .get("staging/capture", data.len() - 1)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::ResourceExhausted
        );
        assert_eq!(
            storage.get("staging/missing", 1).await.unwrap_err().code(),
            tonic::Code::NotFound
        );
        assert!(storage
            .presign_get("staging/capture", Duration::from_secs(60), "image/png")
            .await
            .unwrap()
            .is_none());
        storage.delete("staging/capture").await.unwrap();
        storage.delete("staging/capture").await.unwrap();
        assert_eq!(
            storage
                .get("staging/capture", data.len())
                .await
                .unwrap_err()
                .code(),
            tonic::Code::NotFound
        );
        context.delete_object_store(config.bucket).await.unwrap();
    }

    #[tokio::test]
    async fn s3_provider_roundtrips_and_enforces_read_bounds() {
        let (server, storage) = s3_fixture().await;
        let data = Bytes::from_static(b"artifact bytes");
        Mock::given(method("PUT"))
            .and(path("/artifact-test/staging/capture"))
            .respond_with(ResponseTemplate::new(200).insert_header("etag", "\"stored\""))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/artifact-test/staging/capture"))
            .respond_with(object_response(&data))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/artifact-test/staging/capture"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        storage.put("staging/capture", data.clone()).await.unwrap();
        assert_eq!(
            storage.get("staging/capture", data.len()).await.unwrap(),
            data
        );
        assert_eq!(
            storage
                .get("staging/capture", data.len() - 1)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::ResourceExhausted
        );
        storage.delete("staging/capture").await.unwrap();
    }

    #[tokio::test]
    async fn s3_read_times_out_when_the_provider_stalls_before_response_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let storage =
            S3BlobStorage::with_builder(fixture_config(Some(endpoint)), fixture_builder(), None)
                .unwrap();

        let read = storage.get("staging/capture", 4);
        let request_arrives = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.read_exact(&mut [0]).await.unwrap();
            socket
        };
        tokio::pin!(read);
        let stalled_connection = tokio::select! {
            socket = tokio::time::timeout(Duration::from_secs(5), request_arrives) => {
                socket.expect("provider must receive the request")
            }
            result = &mut read => panic!("read finished before the provider stalled: {result:?}"),
        };
        tokio::time::pause();
        let test_deadline = tokio::time::timeout(Duration::from_secs(31), read);
        tokio::time::advance(Duration::from_secs(31)).await;
        let error = test_deadline
            .await
            .expect("S3 read must return before the test deadline")
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::Unavailable);
        drop(stalled_connection);
    }

    #[tokio::test]
    async fn s3_presigned_upload_binds_length_media_type_and_checksum() {
        let (server, storage) = s3_fixture().await;
        let data = Bytes::from_static(b"artifact bytes");
        let digest = trogon_atlas_client::artifacts::sha256_hex(&data);
        let upload = storage
            .presign_put(
                "staging/capture",
                Duration::from_secs(60),
                data.len() as u64,
                &digest,
                "image/png",
            )
            .await
            .unwrap()
            .unwrap();
        let url = Url::parse(&upload.url).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let signed_headers = query
            .get("X-Amz-SignedHeaders")
            .or_else(|| query.get("x-amz-signedheaders"))
            .unwrap();
        for name in ["content-length", "content-type", "x-amz-checksum-sha256"] {
            assert!(signed_headers.split(';').any(|header| header == name));
        }
        assert_eq!(upload.headers["content-length"], data.len().to_string());
        assert_eq!(upload.headers["content-type"], "image/png");
        assert_eq!(
            upload.headers["x-amz-checksum-sha256"],
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&data))
        );

        Mock::given(method("PUT"))
            .and(path("/artifact-test/staging/capture"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let mut request = reqwest::Client::new().put(upload.url).body(data);
        for (name, value) in upload.headers {
            request = request.header(name, value);
        }
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn s3_presigned_download_binds_media_type_and_safe_disposition() {
        let (_, storage) = s3_fixture().await;
        for (media_type, disposition) in [("image/png", "inline"), ("text/html", "attachment")] {
            let signed = storage
                .presign_get("accepted/capture", Duration::from_secs(60), media_type)
                .await
                .unwrap()
                .unwrap();
            let url = Url::parse(&signed).unwrap();
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(query["response-content-type"], media_type);
            assert_eq!(query["response-content-disposition"], disposition);
        }
    }

    #[tokio::test]
    async fn s3_compatible_server_validates_upload_signatures_and_preserves_bytes() {
        use testcontainers::{
            core::IntoContainerPort, runners::AsyncRunner as _, GenericImage, ImageExt as _,
        };

        let container = GenericImage::new(
            "rustfs/rustfs@sha256",
            "ed9210d6411d77ffceb70d94b964a3cb7d225983368bc25bebe01e995293dcc2",
        )
        .with_exposed_port(9000.tcp())
        .with_env_var("RUSTFS_ACCESS_KEY", "artifact-fixture-access")
        .with_env_var("RUSTFS_SECRET_KEY", "artifact-fixture-signing-key")
        .with_env_var("RUSTFS_CONSOLE_ENABLE", "false")
        .with_cmd(["/data"])
        .start()
        .await
        .expect("start local S3-compatible artifact fixture");
        let port = container.get_host_port_ipv4(9000).await.unwrap();
        let storage = S3BlobStorage::with_builder(
            fixture_config(Some(format!("http://127.0.0.1:{port}"))),
            fixture_builder(),
            None,
        )
        .unwrap();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                if storage
                    .client
                    .create_bucket()
                    .bucket(&storage.bucket)
                    .send()
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("S3-compatible fixture accepts signed bucket creation");

        let storage = std::sync::Arc::new(storage);
        let data = Bytes::from_static(b"captured image bytes");
        let digest = trogon_atlas_client::artifacts::sha256_hex(&data);
        let upload = storage
            .presign_put(
                "staging/capture",
                Duration::from_secs(60),
                data.len() as u64,
                &digest,
                "image/png",
            )
            .await
            .unwrap()
            .unwrap();
        let mut put = http.put(&upload.url).body(data.clone());
        for (name, value) in &upload.headers {
            put = put.header(name, value);
        }
        assert!(put.send().await.unwrap().status().is_success());

        let mut tampered = http.put(&upload.url).body(data.clone());
        for (name, value) in &upload.headers {
            tampered = tampered.header(
                name,
                if name == "content-type" {
                    "image/jpeg"
                } else {
                    value
                },
            );
        }
        assert!(tampered.send().await.unwrap().status().is_client_error());
        let mut wrong_length = http.put(&upload.url).body(vec![0x42; data.len() + 1]);
        for (name, value) in &upload.headers {
            wrong_length = wrong_length.header(
                name,
                if name == "content-length" {
                    (data.len() + 1).to_string()
                } else {
                    value.clone()
                },
            );
        }
        assert!(wrong_length
            .send()
            .await
            .unwrap()
            .status()
            .is_client_error());
        let mut wrong_checksum = http.put(&upload.url).body(vec![0x42; data.len()]);
        for (name, value) in &upload.headers {
            wrong_checksum = wrong_checksum.header(name, value);
        }
        assert!(wrong_checksum
            .send()
            .await
            .unwrap()
            .status()
            .is_client_error());
        assert_eq!(
            storage.get("staging/capture", data.len()).await.unwrap(),
            data
        );
        let download = storage
            .presign_get("staging/capture", Duration::from_secs(60), "image/png")
            .await
            .unwrap()
            .unwrap();
        let response = http.get(download).send().await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.headers()["content-type"], "image/png");
        assert_eq!(response.bytes().await.unwrap(), data);
        storage.delete("staging/capture").await.unwrap();

        let nats = trogon_atlas_testsupport::shared().await;
        let context = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
        let catalog_config = async_nats::jetstream::kv::Config {
            bucket: format!(
                "s3-artifacts-{}",
                trogon_atlas_testsupport::unique_suffix_for_test()
            ),
            ..Default::default()
        };
        let catalog = context
            .create_key_value(catalog_config.clone())
            .await
            .unwrap();
        let store = nats.store().await;
        let service = crate::artifacts::ArtifactService::new(
            storage.clone(),
            catalog,
            1024,
            Duration::from_secs(60),
            trogon_atlas_store::Store::artifact_publication_authority(&store).unwrap(),
        )
        .unwrap();
        let namespace = "artifact-s3-fixture";
        let principal = "artifact-fixture-principal";
        let prepared = service
            .begin(
                trogon_atlas_proto::BeginArtifactUploadRequest {
                    namespace: namespace.into(),
                    filename: "capture.png".into(),
                    media_type: "image/png".into(),
                    size: data.len() as u64,
                    sha256: digest,
                },
                principal,
            )
            .await
            .unwrap();
        assert!(prepared.direct_upload);
        trogon_atlas_client::artifacts::upload_to_presigned_url(
            &prepared.upload_url,
            &prepared.headers,
            data.to_vec(),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let artifact = service
            .complete(namespace, &prepared.upload_id, principal)
            .await
            .unwrap();
        assert_eq!(artifact.artifact_id, prepared.upload_id);
        assert_eq!(
            service
                .content(namespace, &artifact.artifact_id)
                .await
                .unwrap()
                .content,
            data
        );

        let mut delayed_transfer = http.put(&prepared.upload_url).body(data.clone());
        for (name, value) in &prepared.headers {
            delayed_transfer = delayed_transfer.header(name, value);
        }
        assert!(delayed_transfer.send().await.unwrap().status().is_success());
        storage
            .put(
                &format!("staging/{}", prepared.upload_id),
                Bytes::from_static(b"changed staging object"),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .complete(namespace, &prepared.upload_id, principal)
                .await
                .unwrap(),
            artifact
        );
        assert_eq!(
            service.get(namespace, &artifact.artifact_id).await.unwrap(),
            artifact
        );
        assert_eq!(
            service
                .content(namespace, &artifact.artifact_id)
                .await
                .unwrap()
                .content,
            data
        );
        let download = service
            .download(namespace, &artifact.artifact_id)
            .await
            .unwrap();
        let response = http.get(download.url).send().await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.bytes().await.unwrap(), data);
        context
            .delete_key_value(catalog_config.bucket)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn provider_failures_do_not_expose_upstream_details() {
        let (server, storage) = s3_fixture().await;
        Mock::given(method("GET"))
            .and(path("/artifact-test/staging/missing"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_string("upstream private object key and credential context"),
            )
            .mount(&server)
            .await;
        let error = storage.get("staging/missing", 1024).await.unwrap_err();
        assert_eq!(error.code(), tonic::Code::NotFound);
        assert_eq!(error.message(), "artifact content not found");
    }

    #[test]
    fn invalid_keys_and_endpoint_credentials_are_rejected() {
        for key in [
            "",
            "/absolute",
            "../escape",
            "staging/../escape",
            "a//b",
            "a?grant=x",
        ] {
            assert!(validate_key(key).is_err());
        }
        for endpoint in [
            "https://user:password@example.invalid",
            "https://example.invalid?grant=value",
            "https://example.invalid#fragment",
            "http://example.invalid",
            "file:///not-a-storage-endpoint",
        ] {
            assert!(validate_endpoint(endpoint, false).is_err());
        }
        assert!(validate_endpoint("http://127.0.0.1:9000", true).is_ok());
        assert!(validate_ttl(Duration::ZERO).is_err());
        assert!(validate_ttl(Duration::from_millis(500)).is_err());
        assert!(validate_ttl(Duration::from_secs(7 * 24 * 60 * 60 + 1)).is_err());
        assert!(decode_sha256(&"F".repeat(64)).is_err());
    }
}
