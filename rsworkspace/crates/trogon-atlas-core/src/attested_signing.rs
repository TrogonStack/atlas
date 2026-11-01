//! Detached Ed25519 signatures over an attested bundle's digest.
//!
//! The signer never sees the bundle, only its digest behind a domain tag of
//! its own, so a signature cannot be replayed as a signature over anything
//! else. The digest already covers completeness, so a signature binds an
//! incomplete bundle's reasons exactly as it binds its manifests.

use std::{collections::BTreeMap, fmt, str::FromStr};

use ed25519_dalek::{
    pkcs8::{DecodePrivateKey, DecodePublicKey},
    Signer as _,
};
use sha2::{Digest as _, Sha256};
use trogon_atlas_proto as pb;

use crate::content_hash::{ContentHash, ContentHashError};

const SIGNATURE_DOMAIN: &[u8] = b"trogonatlas.attested-model.signature.v1\0";

/// Width of an Ed25519 signature in bytes.
pub const SIGNATURE_LEN: usize = ed25519_dalek::SIGNATURE_LENGTH;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("not a PKCS#8 PEM Ed25519 private key")]
    PrivateKey,
    #[error("not an SPKI PEM Ed25519 public key")]
    PublicKey,
    #[error("no trusted key was given")]
    NoTrustedKeys,
}

/// Identifies a signing key: lowercase hex SHA-256 of its raw 32-byte
/// public key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(ContentHash);

impl KeyId {
    fn of(key: &ed25519_dalek::VerifyingKey) -> Self {
        Self(ContentHash::from_bytes(
            Sha256::digest(key.as_bytes()).into(),
        ))
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({})", self.0)
    }
}

impl FromStr for KeyId {
    type Err = ContentHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ContentHash::from_str(s).map(Self)
    }
}

/// A private key that seals signed bundles. Its `Debug` form names only
/// the key id.
pub struct SigningKey {
    key: ed25519_dalek::SigningKey,
    id: KeyId,
}

impl SigningKey {
    /// Read a PKCS#8 PEM Ed25519 private key, as `openssl genpkey
    /// -algorithm ed25519` writes it.
    pub fn from_pkcs8_pem(pem: &str) -> Result<Self, KeyError> {
        let key =
            ed25519_dalek::SigningKey::from_pkcs8_pem(pem).map_err(|_| KeyError::PrivateKey)?;
        let id = KeyId::of(&key.verifying_key());
        Ok(Self { key, id })
    }

    #[must_use]
    pub fn key_id(&self) -> KeyId {
        self.id
    }

    /// The public half, as a verifier would trust it.
    #[must_use]
    pub fn trusted_key(&self) -> TrustedKey {
        TrustedKey {
            key: self.key.verifying_key(),
            id: self.id,
        }
    }

    pub(crate) fn sign(&self, digest: &ContentHash) -> pb::BundleSignature {
        pb::BundleSignature {
            algorithm: pb::bundle_signature::Algorithm::Ed25519 as i32,
            key_id: self.id.to_hex(),
            value: self.key.sign(&statement(digest)).to_bytes().to_vec(),
        }
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A public key a verifier accepts signatures from.
#[derive(Clone, PartialEq, Eq)]
pub struct TrustedKey {
    key: ed25519_dalek::VerifyingKey,
    id: KeyId,
}

impl TrustedKey {
    /// Read an SPKI PEM Ed25519 public key, as `openssl pkey -pubout`
    /// writes it.
    pub fn from_public_key_pem(pem: &str) -> Result<Self, KeyError> {
        let key = ed25519_dalek::VerifyingKey::from_public_key_pem(pem)
            .map_err(|_| KeyError::PublicKey)?;
        Ok(Self {
            id: KeyId::of(&key),
            key,
        })
    }

    #[must_use]
    pub fn key_id(&self) -> KeyId {
        self.id
    }

    pub(crate) fn verifies(&self, digest: &ContentHash, signature: &[u8; SIGNATURE_LEN]) -> bool {
        self.key
            .verify_strict(
                &statement(digest),
                &ed25519_dalek::Signature::from_bytes(signature),
            )
            .is_ok()
    }
}

impl fmt::Debug for TrustedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrustedKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The keys a verifier accepts signatures from. Never empty: verifying
/// against no keys is [`verify`](crate::attested_bundle::verify).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKeys(BTreeMap<KeyId, TrustedKey>);

impl TrustedKeys {
    pub fn new(keys: impl IntoIterator<Item = TrustedKey>) -> Result<Self, KeyError> {
        let keys: BTreeMap<_, _> = keys.into_iter().map(|k| (k.id, k)).collect();
        if keys.is_empty() {
            return Err(KeyError::NoTrustedKeys);
        }
        Ok(Self(keys))
    }

    pub(crate) fn get(&self, id: &KeyId) -> Option<&TrustedKey> {
        self.0.get(id)
    }
}

fn statement(digest: &ContentHash) -> Vec<u8> {
    let mut out = Vec::with_capacity(SIGNATURE_DOMAIN.len() + digest.as_bytes().len());
    out.extend_from_slice(SIGNATURE_DOMAIN);
    out.extend_from_slice(digest.as_bytes());
    out
}
