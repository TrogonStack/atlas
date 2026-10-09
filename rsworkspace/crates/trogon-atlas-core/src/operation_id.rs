//! Idempotency-key primitives shared by `trogon-atlas-store` (claim/settle)
//! and `trogon-atlas-server` (request-time validation). See `OperationReceipt`
//! in `service.proto` for the contract these implement.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::content_hash::{canonical_json, ContentHashError};

/// `operation_id` must match `[A-Za-z0-9_-]{1,128}`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("operation_id must match [A-Za-z0-9_-]{{1,128}}, got {0:?}")]
pub struct InvalidOperationId(pub String);

/// Mint a fresh `operation_id`: a UUIDv7 (time-ordered, so claims and
/// receipts sort the way they were issued), rendered in the lowercase
/// hyphenated form that always satisfies `validate_operation_id`.
#[must_use]
pub fn new_operation_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub fn validate_operation_id(id: &str) -> Result<(), InvalidOperationId> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(InvalidOperationId(id.to_owned()))
    }
}

/// The KV key one principal's claim on one `operation_id` lives under:
/// `sha256(principal)[:16].operation_id`. Hashing the principal keeps the
/// key free of whatever bytes a principal name contains and bounds its
/// length; the id itself is kept in clear text so the key stays
/// human-greppable. The hash need not be collision-resistant against a
/// malicious principal: colliding two principals only lets them share an
/// operation_id namespace, never read or write each other's entities.
#[must_use]
pub fn operation_claim_key(principal: &str, operation_id: &str) -> String {
    let digest = Sha256::digest(principal.as_bytes());
    let mut prefix = String::with_capacity(16);
    for byte in &digest[..8] {
        let _ = write!(prefix, "{byte:02x}");
    }
    format!("{prefix}.{operation_id}")
}

/// Canonical digest of a mutation request:
/// `sha256(canonical_json({rpc, branch, request}))`, hex-encoded.
/// `request_json` must already have `operation_id` cleared: the digest must
/// not depend on the very field that selects which digest to compare
/// against, or every retry would mint a new digest instead of matching the
/// first one.
pub fn operation_digest(
    rpc: &str,
    branch: Option<&str>,
    request_json: &serde_json::Value,
) -> Result<String, ContentHashError> {
    let envelope = serde_json::json!({
        "rpc": rpc,
        "branch": branch.unwrap_or(""),
        "request": request_json,
    });
    let canonical = canonical_json(&envelope)?;
    let digest = Sha256::digest(canonical.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn minted_ids_are_distinct_and_always_valid() {
        let a = new_operation_id();
        let b = new_operation_id();
        assert_ne!(a, b);
        assert!(validate_operation_id(&a).is_ok(), "{a}");
        assert!(validate_operation_id(&b).is_ok(), "{b}");
    }

    #[test]
    fn accepts_the_documented_alphabet() {
        assert!(validate_operation_id("abcXYZ09_-").is_ok());
        assert!(validate_operation_id(&"a".repeat(128)).is_ok());
    }

    #[test]
    fn rejects_empty_too_long_and_foreign_characters() {
        assert!(validate_operation_id("").is_err());
        assert!(validate_operation_id(&"a".repeat(129)).is_err());
        assert!(validate_operation_id("has space").is_err());
        assert!(validate_operation_id("has/slash").is_err());
    }

    #[test]
    fn claim_key_is_stable_and_scoped_by_principal() {
        let a = operation_claim_key("alice", "op-1");
        let b = operation_claim_key("bob", "op-1");
        assert_ne!(a, b);
        assert_eq!(a, operation_claim_key("alice", "op-1"));
        assert!(a.ends_with(".op-1"));
    }

    #[test]
    fn digest_ignores_key_order_but_not_value() {
        let one = serde_json::json!({"a": 1, "b": 2});
        let two = serde_json::json!({"b": 2, "a": 1});
        assert_eq!(
            operation_digest("PutEntity", None, &one).unwrap(),
            operation_digest("PutEntity", None, &two).unwrap()
        );
        let three = serde_json::json!({"a": 1, "b": 3});
        assert_ne!(
            operation_digest("PutEntity", None, &one).unwrap(),
            operation_digest("PutEntity", None, &three).unwrap()
        );
        assert_ne!(
            operation_digest("PutEntity", None, &one).unwrap(),
            operation_digest("PutEntity", Some("feature-x"), &one).unwrap()
        );
    }
}
