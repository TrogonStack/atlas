use async_nats::jetstream::kv::{CreateErrorKind, UpdateErrorKind};
use bytes::Bytes;
use futures::StreamExt;
use prost::Message;
use sha2::{Digest, Sha256};

use super::{kv_entry_timeout, live_entry, now_micros, NatsStore, KV_OP_TIMEOUT};
use crate::{
    source::{
        SourceDocument, SourceDocumentKey, SourceDocumentPage, SOURCE_DOCUMENT_MAX_BYTES,
        SOURCE_REGISTRATION_MAX_COUNT,
    },
    StoreError, StoreResult,
};

const CHUNK_BYTES: usize = 256 * 1024;

#[derive(Clone, PartialEq, Message)]
struct DocumentHeader {
    #[prost(string, tag = "1")]
    identity: String,
    #[prost(string, repeated, tag = "2")]
    chunks: Vec<String>,
    #[prost(uint64, tag = "3")]
    byte_count: u64,
    #[prost(bytes = "vec", tag = "4")]
    digest: Vec<u8>,
    #[prost(int64, tag = "5")]
    created_at_micros: i64,
    #[prost(uint64, tag = "6")]
    writer_epoch: u64,
}

pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}
pub(super) fn registration_hash(key: &SourceDocumentKey) -> String {
    hex(&Sha256::digest(key.as_str().as_bytes()))
}
fn document_key(key: &SourceDocumentKey) -> String {
    format!("r.{}", registration_hash(key))
}
fn failure() -> StoreError {
    StoreError::Unavailable("source persistence unavailable or incomplete".into())
}
fn decode_header(bytes: &[u8]) -> StoreResult<DocumentHeader> {
    let header = DocumentHeader::decode(bytes).map_err(|_| failure())?;
    if header.byte_count > SOURCE_DOCUMENT_MAX_BYTES as u64
        || header.chunks.len() > SOURCE_DOCUMENT_MAX_BYTES / CHUNK_BYTES
        || header.digest.len() != 32
    {
        return Err(failure());
    }
    Ok(header)
}

impl NatsStore {
    pub(super) async fn read_source_document(
        &self,
        key: &SourceDocumentKey,
    ) -> StoreResult<Option<SourceDocument>> {
        let root = document_key(key);
        for _ in 0..3 {
            let Some(entry) = live_entry(kv_entry_timeout(&self.source_kv, root.clone()).await?)
            else {
                return Ok(None);
            };
            let header = decode_header(&entry.value)?;
            if header.identity != key.as_str() {
                return Err(failure());
            }
            let mut bytes =
                Vec::with_capacity(usize::try_from(header.byte_count).map_err(|_| failure())?);
            let mut incomplete = false;
            for chunk in &header.chunks {
                let Some(part) =
                    live_entry(kv_entry_timeout(&self.source_kv, chunk.clone()).await?)
                else {
                    incomplete = true;
                    break;
                };
                if part.value.len() > CHUNK_BYTES
                    || bytes.len().saturating_add(part.value.len()) > SOURCE_DOCUMENT_MAX_BYTES
                {
                    return Err(failure());
                }
                bytes.extend_from_slice(&part.value);
            }
            let current = live_entry(kv_entry_timeout(&self.source_kv, root.clone()).await?);
            if current.as_ref().map(|e| e.revision) != Some(entry.revision) {
                continue;
            }
            if incomplete
                || bytes.len() as u64 != header.byte_count
                || Sha256::digest(&bytes).as_slice() != header.digest.as_slice()
            {
                return Err(failure());
            }
            return Ok(Some(SourceDocument {
                key: key.clone(),
                bytes,
                revision: entry.revision,
                created_at_micros: header.created_at_micros,
            }));
        }
        Err(failure())
    }

    pub(super) async fn write_source_document(
        &self,
        key: &SourceDocumentKey,
        expected: Option<u64>,
        bytes: &[u8],
        storage_limit: u64,
    ) -> StoreResult<SourceDocument> {
        self.require_writer()?;
        if storage_limit == 0
            || storage_limit > SOURCE_DOCUMENT_MAX_BYTES as u64
            || bytes.len() as u64 > storage_limit
        {
            return Err(StoreError::InvalidArgument(
                "source registration storage budget exceeded".into(),
            ));
        }
        let _lock = self.source_document_lock.lock().await;
        let root = document_key(key);
        let mut previous = live_entry(kv_entry_timeout(&self.source_kv, root.clone()).await?);
        if previous.as_ref().map(|e| e.revision) != expected {
            return Err(StoreError::EtagMismatch {
                expected: expected.unwrap_or_default().to_string(),
                found: previous
                    .as_ref()
                    .map(|e| e.revision)
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        let old = previous
            .as_ref()
            .map(|e| decode_header(&e.value))
            .transpose()?;
        let created_at_micros = old
            .as_ref()
            .map_or_else(now_micros, |h| h.created_at_micros);
        let chunks: Vec<String> = bytes
            .chunks(CHUNK_BYTES)
            .map(|part| {
                format!(
                    "c.{}.{}",
                    registration_hash(key),
                    hex(&Sha256::digest(part))
                )
            })
            .collect();
        let header = DocumentHeader {
            identity: key.as_str().to_owned(),
            chunks,
            byte_count: bytes.len() as u64,
            digest: Sha256::digest(bytes).to_vec(),
            created_at_micros,
            writer_epoch: self.lease.current_epoch().get(),
        };
        let encoded = Bytes::from(header.encode_to_vec());
        let mut additional = 0u64;
        let mut unique = std::collections::BTreeSet::new();
        for (name, part) in header.chunks.iter().zip(bytes.chunks(CHUNK_BYTES)) {
            let collected = old
                .as_ref()
                .is_some_and(|header| !header.chunks.contains(name));
            if unique.insert(name.clone())
                && (collected
                    || live_entry(kv_entry_timeout(&self.source_kv, name.clone()).await?).is_none())
            {
                additional = additional.saturating_add(part.len() as u64);
            }
        }
        let old_header_bytes = previous.as_ref().map_or(0, |e| e.value.len());
        let growth = encoded.len().saturating_sub(old_header_bytes) as u64;
        if self
            .source_storage_usage_retaining_document_chunks(
                key,
                old.as_ref().map(|header| header.chunks.as_slice()),
            )
            .await?
            .saturating_add(additional)
            .saturating_add(growth)
            > storage_limit
        {
            return Err(StoreError::Unavailable(
                "source physical storage budget exhausted".into(),
            ));
        }
        if let Some(previous) = &mut previous {
            self.lease
                .verify_epoch(&self.lease_kv, self.lease.current_epoch())
                .await?;
            // Advancing the root fences earlier uncertain CAS attempts before reclaiming their chunks.
            previous.revision = tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.source_kv
                    .update(&root, previous.value.clone(), previous.revision),
            )
            .await
            .map_err(|_| failure())?
            .map_err(|_| failure())?;
            self.collect_source_document_chunks(
                key,
                old.as_ref()
                    .map(|h| h.chunks.as_slice())
                    .unwrap_or_default(),
            )
            .await?;
        }
        let expected = previous.as_ref().map(|e| e.revision);
        for (name, part) in header.chunks.iter().zip(bytes.chunks(CHUNK_BYTES)) {
            self.require_writer()?;
            match tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.source_kv.create(name, Bytes::copy_from_slice(part)),
            )
            .await
            .map_err(|_| failure())?
            {
                Ok(_) => {}
                Err(e) if e.kind() == CreateErrorKind::AlreadyExists => {
                    let existing =
                        live_entry(kv_entry_timeout(&self.source_kv, name.clone()).await?)
                            .ok_or_else(failure)?;
                    if existing.value.as_ref() != part {
                        return Err(failure());
                    }
                }
                Err(_) => return Err(failure()),
            }
        }
        self.lease
            .verify_epoch(&self.lease_kv, self.lease.current_epoch())
            .await?;
        let revision = if let Some(expected) = expected {
            tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.source_kv.update(&root, encoded, expected),
            )
            .await
            .map_err(|_| failure())?
            .map_err(|e| {
                if e.kind() == UpdateErrorKind::WrongLastRevision {
                    StoreError::EtagMismatch {
                        expected: expected.to_string(),
                        found: "changed".into(),
                    }
                } else {
                    failure()
                }
            })?
        } else {
            tokio::time::timeout(KV_OP_TIMEOUT, self.source_kv.create(&root, encoded))
                .await
                .map_err(|_| failure())?
                .map_err(|e| {
                    if e.kind() == CreateErrorKind::AlreadyExists {
                        StoreError::AlreadyExists
                    } else {
                        failure()
                    }
                })?
        };
        self.collect_source_document_chunks(key, &header.chunks)
            .await?;
        Ok(SourceDocument {
            key: key.clone(),
            bytes: bytes.to_vec(),
            revision,
            created_at_micros,
        })
    }

    pub(super) async fn source_registration_keys(
        &self,
        key: &SourceDocumentKey,
    ) -> StoreResult<Vec<String>> {
        let hash = registration_hash(key);
        let prefixes = [
            format!("c.{hash}."),
            format!("t.{hash}."),
            format!("x.t.{hash}."),
        ];
        let root = document_key(key);
        let mut stream = tokio::time::timeout(KV_OP_TIMEOUT, self.source_kv.keys())
            .await
            .map_err(|_| failure())?
            .map_err(|_| failure())?;
        let mut keys = Vec::new();
        while let Some(item) = tokio::time::timeout(KV_OP_TIMEOUT, stream.next())
            .await
            .map_err(|_| failure())?
        {
            let name = item.map_err(|_| failure())?;
            if name == root || prefixes.iter().any(|p| name.starts_with(p)) {
                keys.push(name);
                if keys.len() > 32768 {
                    return Err(failure());
                }
            }
        }
        Ok(keys)
    }
    pub(super) async fn source_storage_usage(&self, key: &SourceDocumentKey) -> StoreResult<u64> {
        self.source_storage_usage_retaining_document_chunks(key, None)
            .await
    }

    async fn source_storage_usage_retaining_document_chunks(
        &self,
        key: &SourceDocumentKey,
        retained: Option<&[String]>,
    ) -> StoreResult<u64> {
        let prefix = format!("c.{}.", registration_hash(key));
        let mut bytes = 0u64;
        for name in self.source_registration_keys(key).await? {
            if retained.is_some_and(|chunks| name.starts_with(&prefix) && !chunks.contains(&name)) {
                continue;
            }
            if let Some(entry) = live_entry(kv_entry_timeout(&self.source_kv, name).await?) {
                bytes = bytes
                    .checked_add(entry.value.len() as u64)
                    .ok_or_else(failure)?;
            }
        }
        Ok(bytes)
    }
    async fn collect_source_document_chunks(
        &self,
        key: &SourceDocumentKey,
        retained: &[String],
    ) -> StoreResult<()> {
        let prefix = format!("c.{}.", registration_hash(key));
        for name in self.source_registration_keys(key).await? {
            if name.starts_with(&prefix) && !retained.contains(&name) {
                self.require_writer()?;
                self.lease
                    .verify_epoch(&self.lease_kv, self.lease.current_epoch())
                    .await?;
                tokio::time::timeout(KV_OP_TIMEOUT, self.source_kv.purge(&name))
                    .await
                    .map_err(|_| failure())?
                    .map_err(|_| failure())?;
            }
        }
        Ok(())
    }

    pub(super) async fn list_source_documents(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> StoreResult<SourceDocumentPage> {
        if limit == 0
            || limit > 500
            || after.is_some_and(|cursor| !cursor.starts_with("r.") || cursor.len() != 66)
        {
            return Err(StoreError::InvalidArgument("invalid source page".into()));
        }
        let mut stream = tokio::time::timeout(KV_OP_TIMEOUT, self.source_kv.keys())
            .await
            .map_err(|_| failure())?
            .map_err(|_| failure())?;
        let mut roots = Vec::new();
        while let Some(item) = tokio::time::timeout(KV_OP_TIMEOUT, stream.next())
            .await
            .map_err(|_| failure())?
        {
            let name = item.map_err(|_| failure())?;
            if name.starts_with("r.") {
                roots.push(name);
                if roots.len() > SOURCE_REGISTRATION_MAX_COUNT {
                    return Err(StoreError::Unavailable(
                        "source registration capacity exceeded".into(),
                    ));
                }
            }
        }
        roots.sort();
        roots.retain(|name| after.is_none_or(|cursor| name.as_str() > cursor));
        let more = roots.len() > limit;
        roots.truncate(limit);
        let next_cursor = if more { roots.last().cloned() } else { None };
        let mut keys = Vec::new();
        for root in roots {
            let entry =
                live_entry(kv_entry_timeout(&self.source_kv, root).await?).ok_or_else(failure)?;
            keys.push(SourceDocumentKey::new(
                decode_header(&entry.value)?.identity,
            )?);
        }
        Ok(SourceDocumentPage { keys, next_cursor })
    }
}
