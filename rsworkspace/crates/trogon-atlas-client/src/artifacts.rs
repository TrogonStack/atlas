use std::{collections::HashMap, io::Write, path::Path, time::Duration};

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

pub use crate::responsibility::parse_request;
use crate::{client::Client, failure::invalid_input};

pub const MAX_TRANSFER_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 36 * 1024 * 1024;

fn json<M: prost::Message>(name: &str, message: &M) -> Result<Value> {
    Ok(TranscodePool::builtin()?.message_json_value(
        &format!("trogonatlas.api.eventmodel.v1alpha1.{name}"),
        message,
    )?)
}

pub fn artifact_json(artifact: &pb::Artifact) -> Result<Value> {
    json("Artifact", artifact)
}

pub async fn get_artifact_storage_info(
    client: &mut Client,
    request: pb::GetArtifactStorageInfoRequest,
) -> Result<Value> {
    let response = client
        .get_artifact_storage_info(request)
        .await?
        .into_inner();
    json("GetArtifactStorageInfoResponse", &response)
}

pub async fn begin_artifact_upload(
    client: &mut Client,
    request: pb::BeginArtifactUploadRequest,
) -> Result<Value> {
    let response = client.begin_artifact_upload(request).await?.into_inner();
    json("BeginArtifactUploadResponse", &response)
}

pub async fn complete_artifact_upload(
    client: &mut Client,
    request: pb::CompleteArtifactUploadRequest,
) -> Result<Value> {
    let response = client.complete_artifact_upload(request).await?.into_inner();
    json("CompleteArtifactUploadResponse", &response)
}

pub async fn get_artifact(client: &mut Client, request: pb::GetArtifactRequest) -> Result<Value> {
    let response = client.get_artifact(request).await?.into_inner();
    json("GetArtifactResponse", &response)
}

pub async fn get_artifact_download(
    client: &mut Client,
    request: pb::GetArtifactDownloadRequest,
) -> Result<Value> {
    let response = client.get_artifact_download(request).await?.into_inner();
    json("GetArtifactDownloadResponse", &response)
}

pub fn sha256_hex(content: &[u8]) -> String {
    use std::fmt::Write as _;

    Sha256::digest(content)
        .iter()
        .fold(String::with_capacity(64), |mut digest, byte| {
            let _ = write!(digest, "{byte:02x}");
            digest
        })
}

fn media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("png") => "image/png",
        Some(extension)
            if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") =>
        {
            "image/jpeg"
        }
        Some(extension) if extension.eq_ignore_ascii_case("webp") => "image/webp",
        Some(extension) if extension.eq_ignore_ascii_case("gif") => "image/gif",
        Some(extension) if extension.eq_ignore_ascii_case("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

pub async fn upload_file(
    client: &mut Client,
    namespace: &str,
    path: &Path,
    supplied_media_type: Option<&str>,
    timeout: Duration,
) -> Result<pb::Artifact> {
    let info = client
        .get_artifact_storage_info(pb::GetArtifactStorageInfoRequest {})
        .await?
        .into_inner();
    if !info.enabled {
        bail!("artifact storage is disabled on the server");
    }
    let limit = usize::try_from(info.max_bytes)
        .unwrap_or(MAX_TRANSFER_BYTES)
        .min(MAX_TRANSFER_BYTES);
    let content = crate::source_input::read_bounded_file(path, limit)?;
    if content.is_empty() {
        return Err(invalid_input("artifact file must not be empty"));
    }
    let filename = path
        .file_name()
        .and_then(|filename| filename.to_str())
        .ok_or_else(|| invalid_input("artifact filename must be valid UTF-8"))?;
    let size = content.len() as u64;
    let digest = sha256_hex(&content);
    let upload = client
        .begin_artifact_upload(pb::BeginArtifactUploadRequest {
            namespace: namespace.to_owned(),
            filename: filename.to_owned(),
            media_type: supplied_media_type
                .unwrap_or_else(|| media_type(path))
                .to_owned(),
            size,
            sha256: digest.clone(),
        })
        .await?
        .into_inner();
    if upload.direct_upload {
        upload_to_presigned_url(&upload.upload_url, &upload.headers, content, timeout).await?;
    } else {
        client
            .upload_artifact_content(pb::UploadArtifactContentRequest {
                namespace: namespace.to_owned(),
                upload_id: upload.upload_id.clone(),
                content,
            })
            .await?;
    }
    let artifact = client
        .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
            namespace: namespace.to_owned(),
            upload_id: upload.upload_id,
        })
        .await?
        .into_inner()
        .artifact
        .context("server omitted the completed artifact")?;
    if artifact.namespace != namespace || artifact.size != size || artifact.sha256 != digest {
        bail!("completed artifact does not match the uploaded file");
    }
    Ok(artifact)
}

pub async fn upload_to_presigned_url<S: std::hash::BuildHasher>(
    upload_url: &str,
    supplied_headers: &HashMap<String, String, S>,
    content: Vec<u8>,
    timeout: Duration,
) -> Result<()> {
    let url = reqwest::Url::parse(upload_url)
        .map_err(|_| invalid_input("server returned an invalid upload URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_input("server returned an unsupported upload URL"));
    }
    if content.len() > MAX_TRANSFER_BYTES {
        return Err(invalid_input("artifact exceeds the client transfer limit"));
    }
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in supplied_headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| invalid_input("server returned an invalid upload header"))?;
        if matches!(
            name.as_str(),
            "authorization" | "proxy-authorization" | "cookie" | "host"
        ) {
            return Err(invalid_input(
                "server returned an unsupported upload header",
            ));
        }
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| invalid_input("server returned an invalid upload header"))?;
        headers.insert(name, value);
    }
    if let Some(length) = headers.get(reqwest::header::CONTENT_LENGTH) {
        if length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            != Some(content.len())
        {
            return Err(invalid_input(
                "upload Content-Length does not match the file",
            ));
        }
    }
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10).min(timeout))
        .timeout(timeout)
        .build()
        .map_err(|_| anyhow::anyhow!("could not initialize artifact upload transport"))?;
    let response = http
        .put(url)
        .headers(headers)
        .body(content)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("artifact upload transport failed"))?;
    if !response.status().is_success() {
        bail!(
            "artifact upload failed with HTTP status {}",
            response.status().as_u16()
        );
    }
    Ok(())
}

pub async fn download_file(
    client: &mut Client,
    namespace: &str,
    artifact_id: &str,
    destination: &Path,
) -> Result<pb::Artifact> {
    let response = client
        .get_artifact_content(pb::GetArtifactContentRequest {
            namespace: namespace.to_owned(),
            artifact_id: artifact_id.to_owned(),
        })
        .await?
        .into_inner();
    let artifact = response
        .artifact
        .context("server omitted artifact metadata")?;
    if artifact.namespace != namespace || artifact.artifact_id != artifact_id {
        bail!("downloaded artifact identity does not match the request");
    }
    write_verified_download(&artifact, &response.content, destination)?;
    Ok(artifact)
}

pub fn write_verified_download(
    artifact: &pb::Artifact,
    content: &[u8],
    destination: &Path,
) -> Result<()> {
    if content.len() > MAX_TRANSFER_BYTES
        || content.len() as u64 != artifact.size
        || sha256_hex(content) != artifact.sha256
    {
        bail!("downloaded artifact failed size or digest verification");
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| anyhow::anyhow!("could not create artifact download file"))?;
    temporary
        .write_all(content)
        .map_err(|_| anyhow::anyhow!("could not write artifact download file"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| anyhow::anyhow!("could not sync artifact download file"))?;
    temporary.persist_noclobber(destination).map_err(|_| {
        anyhow::anyhow!("could not publish artifact download; destination must not exist")
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn artifact(content: &[u8]) -> pb::Artifact {
        pb::Artifact {
            size: content.len() as u64,
            sha256: sha256_hex(content),
            ..Default::default()
        }
    }

    #[test]
    fn rejected_download_preserves_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("capture.png");
        std::fs::write(&destination, b"existing").unwrap();
        assert!(write_verified_download(&artifact(b"expected"), b"corrupt", &destination).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
        assert!(
            write_verified_download(&artifact(b"replacement"), b"replacement", &destination)
                .is_err()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
    }

    #[test]
    fn valid_download_is_published_after_verification() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("capture.png");
        write_verified_download(&artifact(b"capture"), b"capture", &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"capture");
    }

    #[tokio::test]
    async fn invalid_upload_url_does_not_leak_credentials() {
        let error = upload_to_presigned_url(
            "https://user:private-value@example.invalid/object?secret=private-value",
            &HashMap::new(),
            b"capture".to_vec(),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains("private-value"));
    }

    #[tokio::test]
    async fn upload_rejects_credentials_in_headers() {
        let error = upload_to_presigned_url(
            "https://example.invalid/object",
            &HashMap::from([("Authorization".to_owned(), "private-value".to_owned())]),
            b"capture".to_vec(),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains("private-value"));
    }

    async fn upload_server(response: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/artifact?signature=fixture",
            listener.local_addr().unwrap()
        );
        let task = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let count = connection.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.ends_with(b"capture") {
                    break;
                }
            }
            connection.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, task)
    }

    #[tokio::test]
    async fn direct_upload_sends_bytes_and_required_headers_without_bearer() {
        let (url, server) =
            upload_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        upload_to_presigned_url(
            &url,
            &HashMap::from([("content-type".to_owned(), "image/png".to_owned())]),
            b"capture".to_vec(),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        let request = server.await.unwrap();
        let headers = request
            .split("\r\n\r\n")
            .next()
            .unwrap()
            .to_ascii_lowercase();
        assert!(headers.starts_with("put /artifact?signature=fixture "));
        assert!(headers.contains("content-type: image/png"));
        assert!(!headers.contains("authorization:"));
        assert!(!headers.contains("cookie:"));
        assert!(request.ends_with("capture"));
    }

    #[tokio::test]
    async fn direct_upload_does_not_follow_redirects_or_echo_signed_urls() {
        let (url, server) = upload_server("HTTP/1.1 302 Found\r\nLocation: /redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        let error = upload_to_presigned_url(
            &url,
            &HashMap::new(),
            b"capture".to_vec(),
            Duration::from_secs(3),
        )
        .await
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("302"));
        assert!(!message.contains("signature"));
        assert!(!message.contains(&url));
        server.await.unwrap();
    }
}
