use async_nats::{
    header::NATS_MARKER_REASON,
    jetstream::{
        kv::{Entry, Operation, Store as KvStore},
        stream::LastRawMessageErrorKind,
    },
    HeaderMap,
};

use crate::{StoreError, StoreResult};

fn operation(headers: &HeaderMap) -> Operation {
    match headers
        .get("KV-Operation")
        .map(async_nats::HeaderValue::as_str)
    {
        Some("DEL") => Operation::Delete,
        Some("PURGE") => Operation::Purge,
        Some(_) => Operation::Put,
        None => match headers
            .get(NATS_MARKER_REASON)
            .map(async_nats::HeaderValue::as_str)
        {
            Some("MaxAge" | "Purge") => Operation::Purge,
            Some("Remove") => Operation::Delete,
            _ => Operation::Put,
        },
    }
}

pub(super) async fn entry(kv: &KvStore, key: String) -> StoreResult<Option<Entry>> {
    if key.is_empty()
        || key.starts_with('.')
        || key.ends_with('.')
        || !key.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'/' | b'_' | b'=' | b'.')
        })
    {
        return Err(StoreError::Backend(format!("kv.entry {key}: invalid key")));
    }
    let subject = format!("{}{key}", kv.prefix);
    let message = match kv.stream.get_last_raw_message_by_subject(&subject).await {
        Ok(message) => message,
        Err(error) if error.kind() == LastRawMessageErrorKind::NoMessageFound => return Ok(None),
        Err(error) => return Err(StoreError::Backend(format!("kv.entry {key}: {error}"))),
    };
    if message.subject.as_str() != subject {
        return Err(StoreError::Backend("KV response subject mismatch".into()));
    }
    Ok(Some(Entry {
        bucket: kv.name.clone(),
        key,
        operation: operation(&message.headers),
        value: message.payload,
        revision: message.sequence,
        created: message.time,
        delta: 0,
        seen_current: false,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn leader_entries_preserve_sdk_metadata_and_missing_delete_purge_semantics() {
        let nats = trogon_atlas_testsupport::shared().await;
        let client = async_nats::connect(&nats.url).await.unwrap();
        let kv = async_nats::jetstream::new(client)
            .create_key_value(async_nats::jetstream::kv::Config {
                bucket: format!(
                    "leader-entries-{}",
                    trogon_atlas_testsupport::unique_suffix_for_test()
                ),
                history: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        for key in ["put", "delete", "purge", "empty"] {
            kv.put(
                key,
                if key == "empty" {
                    vec![]
                } else {
                    vec![1, 2, 3]
                }
                .into(),
            )
            .await
            .unwrap();
        }
        kv.delete("delete").await.unwrap();
        kv.purge("purge").await.unwrap();
        for key in ["put", "delete", "purge", "empty"] {
            let expected = kv.entry(key).await.unwrap().unwrap();
            let actual = super::entry(&kv, key.into()).await.unwrap().unwrap();
            assert_eq!(actual.bucket, expected.bucket);
            assert_eq!(actual.key, expected.key);
            assert_eq!(actual.value, expected.value);
            assert_eq!(actual.revision, expected.revision);
            assert_eq!(actual.created, expected.created);
            assert_eq!(actual.operation, expected.operation);
            assert_eq!(actual.delta, expected.delta);
            assert_eq!(actual.seen_current, expected.seen_current);
            assert_eq!(
                super::super::live_entry(Some(actual)).is_some(),
                key == "put"
            );
        }
        assert!(super::entry(&kv, "missing".into()).await.unwrap().is_none());
        for key in [
            "",
            ".leading",
            "trailing.",
            "wild*card",
            "bad space",
            "café",
        ] {
            assert!(kv.entry(key).await.is_err());
            assert!(super::entry(&kv, key.into()).await.is_err());
        }
    }

    #[test]
    fn expiry_markers_keep_the_sdk_operation_mapping() {
        for (reason, expected) in [
            ("MaxAge", Operation::Purge),
            ("Purge", Operation::Purge),
            ("Remove", Operation::Delete),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(NATS_MARKER_REASON, reason);
            assert_eq!(operation(&headers), expected);
        }
    }
}
