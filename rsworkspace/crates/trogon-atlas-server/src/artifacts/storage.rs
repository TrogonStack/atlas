use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

use async_nats::jetstream::{
    context::GetStreamErrorKind,
    object_store::{GetErrorKind, ObjectStore as NatsObjectStore},
    stream::{DiscardPolicy, RetentionPolicy},
    Context, ErrorCode,
};
use async_trait::async_trait;
use aws_sigv4::{
    http_request::{
        sign, PercentEncodingMode, SignableBody, SignableRequest, SignatureLocation,
        SigningSettings, UriPathNormalizationMode,
    },
    sign::v4,
};
use base64::Engine as _;
use bytes::{Bytes, BytesMut};
use futures::TryStreamExt as _;
use object_store::{
    aws::{AmazonS3, AmazonS3Builder, AmazonS3ConfigKey},
    path::Path,
    signer::{Method, SignedUrlOptions, Signer, Url},
    ObjectStoreExt as _,
};
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

pub struct S3BlobStorage {
    store: AmazonS3,
    region: String,
}

impl S3BlobStorage {
    pub fn new(config: S3StorageConfig) -> Result<Self, Status> {
        Self::with_builder(config, AmazonS3Builder::from_env())
    }

    fn with_builder(config: S3StorageConfig, builder: AmazonS3Builder) -> Result<Self, Status> {
        if config.bucket.trim().is_empty() || config.region.trim().is_empty() {
            return Err(Status::invalid_argument(
                "artifact S3 bucket and region are required",
            ));
        }
        let mut builder = builder
            .with_bucket_name(config.bucket)
            .with_region(&config.region)
            .with_allow_http(config.allow_http)
            .with_skip_signature(false)
            .with_virtual_hosted_style_request(!config.force_path_style)
            .with_disable_bulk_delete(true);
        let endpoint = config
            .endpoint
            .or_else(|| builder.get_config_value(&AmazonS3ConfigKey::S3Endpoint))
            .or_else(|| builder.get_config_value(&AmazonS3ConfigKey::Endpoint));
        if let Some(endpoint) = endpoint {
            validate_endpoint(&endpoint, config.allow_http)?;
            builder = builder
                .with_config(AmazonS3ConfigKey::S3Endpoint, &endpoint)
                .with_endpoint(endpoint);
        }
        let store = builder
            .build()
            .map_err(|_| Status::failed_precondition("artifact S3 configuration is invalid"))?;
        Ok(Self {
            store,
            region: config.region,
        })
    }
}

#[async_trait]
impl BlobStorage for S3BlobStorage {
    fn backend_name(&self) -> &'static str {
        "s3"
    }

    async fn put(&self, key: &str, data: Bytes) -> Result<(), Status> {
        let path = storage_path(key)?;
        self.store
            .put(&path, data.into())
            .await
            .map_err(|error| storage_error(&error))?;
        Ok(())
    }

    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status> {
        let path = storage_path(key)?;
        let result = self
            .store
            .get(&path)
            .await
            .map_err(|error| storage_error(&error))?;
        if result.meta.size > u64::try_from(max_bytes).map_err(|_| too_large())? {
            return Err(too_large());
        }
        let mut stream = result.into_stream();
        let mut bytes = BytesMut::new();
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|error| storage_error(&error))?
        {
            if chunk.len() > max_bytes.saturating_sub(bytes.len()) {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes.freeze())
    }

    async fn delete(&self, key: &str) -> Result<(), Status> {
        let path = storage_path(key)?;
        match self.store.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(storage_error(&error)),
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
        let path = storage_path(key)?;
        validate_ttl(ttl)?;
        let checksum = base64::engine::general_purpose::STANDARD.encode(decode_sha256(sha256)?);
        let headers = HashMap::from([
            ("content-length".to_owned(), size.to_string()),
            ("content-type".to_owned(), media_type.to_owned()),
            ("x-amz-checksum-sha256".to_owned(), checksum),
        ]);
        // object_store excludes Content-Length from its signer. The upload grant
        // must bind it, so use AWS's signer over the provider-resolved object URL.
        let mut url = self
            .store
            .signed_url(Method::PUT, &path, ttl)
            .await
            .map_err(|error| storage_error(&error))?;
        let query: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(name, _)| {
                !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "x-amz-algorithm"
                        | "x-amz-credential"
                        | "x-amz-date"
                        | "x-amz-expires"
                        | "x-amz-signedheaders"
                        | "x-amz-signature"
                        | "x-amz-security-token"
                )
            })
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        url.set_query(None);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let credential = self
            .store
            .credentials()
            .get_credential()
            .await
            .map_err(|error| storage_error(&error))?;
        let identity = aws_credential_types::Credentials::new(
            credential.key_id.clone(),
            credential.secret_key.clone(),
            credential.token.clone(),
            None,
            "atlas-artifacts",
        )
        .into();
        let mut settings = SigningSettings::default();
        settings.signature_location = SignatureLocation::QueryParams;
        settings.expires_in = Some(ttl);
        settings.excluded_headers = Some(Vec::new());
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name("s3")
            .time(SystemTime::now())
            .settings(settings)
            .build()
            .map_err(|_| unavailable())?
            .into();
        let signable = SignableRequest::new(
            "PUT",
            url.as_str(),
            headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
            SignableBody::UnsignedPayload,
        )
        .map_err(|_| unavailable())?;
        let (instructions, _) = sign(signable, &params)
            .map_err(|_| unavailable())?
            .into_parts();
        let mut request = http::Request::builder()
            .method("PUT")
            .uri(url.as_str())
            .body(())
            .map_err(|_| unavailable())?;
        instructions.apply_to_request_http1x(&mut request);
        Ok(Some(SignedUpload {
            url: request.uri().to_string(),
            headers,
        }))
    }

    async fn presign_get(
        &self,
        key: &str,
        ttl: Duration,
        media_type: &str,
    ) -> Result<Option<String>, Status> {
        let path = storage_path(key)?;
        validate_ttl(ttl)?;
        media_type
            .parse::<http::HeaderValue>()
            .map_err(|_| Status::invalid_argument("invalid artifact media type"))?;
        let disposition = if matches!(
            media_type,
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            "inline"
        } else {
            "attachment"
        };
        let options = SignedUrlOptions::default().with_query([
            ("response-content-type", media_type),
            ("response-content-disposition", disposition),
        ]);
        self.store
            .signed_url_opts(Method::GET, &path, ttl, &options)
            .await
            .map(|url| Some(url.to_string()))
            .map_err(|error| storage_error(&error))
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

fn storage_path(key: &str) -> Result<Path, Status> {
    validate_key(key)?;
    Path::parse(key).map_err(|_| Status::invalid_argument("invalid artifact storage key"))
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

fn storage_error(error: &object_store::Error) -> Status {
    match error {
        object_store::Error::NotFound { .. } => not_found(),
        _ => unavailable(),
    }
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

    use sha2::{Digest as _, Sha256};
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use super::*;

    #[tokio::test]
    async fn explicit_s3_endpoint_overrides_inherited_provider_endpoint() {
        let builder = AmazonS3Builder::new()
            .with_config(
                object_store::aws::AmazonS3ConfigKey::S3Endpoint,
                "https://inherited.invalid",
            )
            .with_access_key_id("artifact-fixture-access")
            .with_secret_access_key("artifact-fixture-signing-key");
        let storage = S3BlobStorage::with_builder(
            S3StorageConfig {
                bucket: "artifact-test".into(),
                region: "us-east-1".into(),
                endpoint: Some("https://selected.invalid".into()),
                force_path_style: true,
                allow_http: false,
            },
            builder,
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

    #[test]
    fn inherited_s3_endpoint_cannot_contain_url_credentials() {
        let builder = AmazonS3Builder::new()
            .with_config(
                object_store::aws::AmazonS3ConfigKey::S3Endpoint,
                "https://fixture-user:fixture-password@storage.invalid",
            )
            .with_access_key_id("artifact-fixture-access")
            .with_secret_access_key("artifact-fixture-signing-key");
        let result = S3BlobStorage::with_builder(
            S3StorageConfig {
                bucket: "artifact-test".into(),
                region: "us-east-1".into(),
                endpoint: None,
                force_path_style: true,
                allow_http: false,
            },
            builder,
        );
        assert_eq!(
            result.err().map(|error| error.code()),
            Some(tonic::Code::InvalidArgument)
        );
    }

    async fn s3_fixture() -> (MockServer, S3BlobStorage) {
        let server = MockServer::start().await;
        let store = AmazonS3Builder::new()
            .with_bucket_name("artifact-test")
            .with_region("us-east-1")
            .with_endpoint(server.uri())
            .with_allow_http(true)
            .with_virtual_hosted_style_request(false)
            .with_disable_bulk_delete(true)
            .with_access_key_id("artifact-fixture-access")
            .with_secret_access_key("artifact-fixture-signing-key")
            .with_retry(object_store::RetryConfig {
                max_retries: 0,
                ..Default::default()
            })
            .build()
            .unwrap();
        (
            server,
            S3BlobStorage {
                store,
                region: "us-east-1".into(),
            },
        )
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
        let store = AmazonS3Builder::new()
            .with_bucket_name("artifact-test")
            .with_region("us-east-1")
            .with_endpoint(format!("http://127.0.0.1:{port}"))
            .with_allow_http(true)
            .with_virtual_hosted_style_request(false)
            .with_disable_bulk_delete(true)
            .with_access_key_id("artifact-fixture-access")
            .with_secret_access_key("artifact-fixture-signing-key")
            .with_retry(object_store::RetryConfig {
                max_retries: 0,
                ..Default::default()
            })
            .build()
            .unwrap();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let create_bucket = store
            .signed_url(Method::PUT, &Path::from(""), Duration::from_secs(120))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                if let Ok(response) = http.put(create_bucket.clone()).send().await {
                    if response.status().is_success() {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("S3-compatible fixture accepts signed bucket creation");

        let storage = std::sync::Arc::new(S3BlobStorage {
            store,
            region: "us-east-1".into(),
        });
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
