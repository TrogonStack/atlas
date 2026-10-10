//! Opaque pagination cursor for `ListChanges`' poll.
//!
//! A plain decimal `next_token` discloses nothing beyond what a visible
//! record in the same response already said: a token that happens to equal
//! the seq of an event the caller just received. The bounded lookahead in
//! `list_changes` can also advance the cursor past a run of records the
//! caller cannot see at all, purely so the poll keeps making progress
//! instead of stalling on the same position forever. Handing back that raw
//! sequence would tell an otherwise empty-handed caller how much other
//! tenants have written, which is the leak this module closes: that case
//! mints an opaque, sealed token instead.
//!
//! The seal is a ChaCha20-Poly1305 AEAD, not a lookup key into process
//! state. A `(principal, seq)` pair is sealed directly into the token's
//! bytes, with `principal` bound in as associated data so a token cannot be
//! opened under a different principal even if intercepted. Nothing is kept
//! between `seal` and `resolve` beyond the key, which is what makes this
//! stateless: a token survives the process that minted it restarting, is
//! never evicted under load, and opens on any process holding the same key
//! -- the posture a single-writer deployment with standby and reader roles
//! needs, since the next page of a poll can land on a different process
//! than the one that minted the cursor.
//!
//! A token that fails to open -- tampered, minted for a different
//! principal, or sealed under a key this process does not hold -- is
//! `ResolveError::FailedToOpen`, which `list_changes` reports as the
//! classified `CURSOR_REJECTED` error rather than silently resetting the
//! poll to position 0 or to the live tip. Restarting from either would be
//! its own tenancy risk: a caller who only ever gets `CURSOR_REJECTED` back
//! for a rotated key would otherwise re-poll from the tip and silently miss
//! everything written in between.

use std::fmt;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    ChaCha20Poly1305, Key, Nonce,
};

const TOKEN_PREFIX: &str = "sealed:";
const NONCE_LEN: usize = 12;
const SEQ_LEN: usize = 8;

/// A ChaCha20-Poly1305 key sealing `ListChanges` cursors. Never
/// Debug-printed: anyone holding it can mint a cursor for any principal.
#[derive(Clone)]
pub struct ChangeCursorKey(Key);

impl ChangeCursorKey {
    /// A fresh random key. Generating one per process is the documented
    /// default for a single-process deployment; a deployment that runs more
    /// than one process serving the same `ListChanges` poll (a standby, a
    /// reader role) must configure the same key explicitly instead, or a
    /// cursor minted by one process fails to open on another.
    #[must_use]
    pub fn generate() -> Self {
        Self(ChaCha20Poly1305::generate_key(&mut OsRng))
    }

    /// Parse a standard-base64, 32-byte key -- the form
    /// `--change-cursor-key` / `TROGON_ATLAS_CHANGE_CURSOR_KEY` accepts.
    pub fn from_base64(encoded: &str) -> Result<Self, InvalidChangeCursorKey> {
        use base64::engine::general_purpose::STANDARD;
        let bytes = STANDARD
            .decode(encoded.trim())
            .map_err(|_| InvalidChangeCursorKey)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| InvalidChangeCursorKey)?;
        Ok(Self(bytes.into()))
    }
}

impl fmt::Debug for ChangeCursorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChangeCursorKey(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidChangeCursorKey;

impl fmt::Display for InvalidChangeCursorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("change cursor key must be standard-base64-encoded 32 bytes")
    }
}

impl std::error::Error for InvalidChangeCursorKey {}

/// Why `ChangeCursorSeal::resolve` could not turn a token back into a seq.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    /// Not our prefix at all: a plain decimal or garbage, same as always.
    NotSealed,
    /// Carried our prefix but failed to open: tampered, minted for a
    /// different principal, or sealed under a key this process does not
    /// hold.
    FailedToOpen,
}

/// Seals and opens `ListChanges` cursors. See the module docs for why this
/// is a stateless AEAD rather than a table.
pub struct ChangeCursorSeal {
    cipher: ChaCha20Poly1305,
}

impl ChangeCursorSeal {
    #[must_use]
    pub fn new(key: &ChangeCursorKey) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(&key.0),
        }
    }

    /// Mint an opaque token bound to `principal` and `seq`. The returned
    /// string carries no recoverable information to anyone without the
    /// key; `resolve` is the only way back, and only for the same
    /// principal.
    pub fn seal(&self, principal: &str, seq: u64) -> String {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self.cipher.encrypt(
            &nonce,
            Payload {
                msg: &seq.to_be_bytes(),
                aad: principal.as_bytes(),
            },
        );
        // Safety: the only documented failure mode is an over-length
        // plaintext; an 8-byte seq never approaches it.
        #[allow(clippy::expect_used)]
        let ciphertext = ciphertext.expect("sealing an 8-byte seq cannot fail");
        let mut payload = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&ciphertext);
        format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(payload))
    }

    /// Resolve a token minted by `seal`, only when `principal` matches the
    /// one it was minted for.
    pub fn resolve(&self, principal: &str, token: &str) -> Result<u64, ResolveError> {
        let Some(encoded) = token.strip_prefix(TOKEN_PREFIX) else {
            return Err(ResolveError::NotSealed);
        };
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| ResolveError::FailedToOpen)?;
        if payload.len() < NONCE_LEN {
            return Err(ResolveError::FailedToOpen);
        }
        let (nonce, ciphertext) = payload.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce);
        let plaintext = self
            .cipher
            .decrypt(
                nonce,
                Payload {
                    msg: ciphertext,
                    aad: principal.as_bytes(),
                },
            )
            .map_err(|_| ResolveError::FailedToOpen)?;
        let seq_bytes: [u8; SEQ_LEN] = plaintext
            .as_slice()
            .try_into()
            .map_err(|_| ResolveError::FailedToOpen)?;
        Ok(u64::from_be_bytes(seq_bytes))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn resolves_only_for_the_minting_principal() {
        let seal = ChangeCursorSeal::new(&ChangeCursorKey::generate());
        let token = seal.seal("acme", 42);

        assert_eq!(seal.resolve("acme", &token), Ok(42));
        assert_eq!(
            seal.resolve("beta", &token),
            Err(ResolveError::FailedToOpen),
            "a token minted for one principal must not open under another",
        );
    }

    #[test]
    fn rejects_garbage_and_plain_tokens() {
        let seal = ChangeCursorSeal::new(&ChangeCursorKey::generate());
        assert_eq!(seal.resolve("acme", "42"), Err(ResolveError::NotSealed));
        assert_eq!(
            seal.resolve("acme", "not-a-token"),
            Err(ResolveError::NotSealed)
        );
    }

    #[test]
    fn a_token_sealed_by_one_instance_opens_on_a_second_instance_with_the_same_key() {
        use base64::engine::general_purpose::STANDARD;
        let encoded_key = STANDARD.encode([7u8; 32]);
        let first_instance =
            ChangeCursorSeal::new(&ChangeCursorKey::from_base64(&encoded_key).unwrap());
        let second_instance =
            ChangeCursorSeal::new(&ChangeCursorKey::from_base64(&encoded_key).unwrap());

        let token = first_instance.seal("acme", 7);

        assert_eq!(
            second_instance.resolve("acme", &token),
            Ok(7),
            "a token must resolve on any process holding the same key, not only the one that minted it",
        );
    }

    #[test]
    fn tampered_token_is_refused() {
        let seal = ChangeCursorSeal::new(&ChangeCursorKey::generate());
        let token = seal.seal("acme", 1);
        let encoded = token.strip_prefix(TOKEN_PREFIX).unwrap();
        let mut payload = URL_SAFE_NO_PAD.decode(encoded).unwrap();
        let last = payload.len() - 1;
        payload[last] ^= 0xFF;
        let tampered = format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(payload));

        assert_eq!(
            seal.resolve("acme", &tampered),
            Err(ResolveError::FailedToOpen)
        );
    }
}
