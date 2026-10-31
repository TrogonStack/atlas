//! Where the server reads the key it signs attested bundles with.
//!
//! The server never generates or stores a signing key. The operator names
//! one by reference, and every export reads it again, so a rotated key is
//! picked up without a restart and a key that disappears stops signing
//! instead of leaving a stale copy in use.

use std::{fmt, path::PathBuf, str::FromStr};

use trogon_atlas_core::attested_signing::{KeyError, SigningKey};

/// Name of an environment variable holding a PEM private key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVarName(String);

#[derive(Debug, thiserror::Error)]
#[error("environment variable name must be non-empty and contain no '=' or NUL: {0:?}")]
pub struct InvalidEnvVarName(String);

impl FromStr for EnvVarName {
    type Err = InvalidEnvVarName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.contains(['=', '\0']) {
            return Err(InvalidEnvVarName(s.to_string()));
        }
        Ok(Self(s.to_string()))
    }
}

impl fmt::Display for EnvVarName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A reference to the PKCS#8 PEM Ed25519 key attested bundles are signed
/// with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigningKeySource {
    File(PathBuf),
    EnvVar(EnvVarName),
}

/// Why the configured signing key could not be used. The message names the
/// reference, never the key material.
#[derive(Debug, thiserror::Error)]
pub enum SigningKeyUnavailable {
    #[error("reading signing key file {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("signing key environment variable {0} is unset or not UTF-8")]
    Unset(EnvVarName),
    #[error("signing key from {origin}: {source}")]
    Malformed {
        origin: String,
        #[source]
        source: KeyError,
    },
}

impl SigningKeySource {
    /// `file` or `env`, the kind of reference, for status metadata.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::File(_) => "file",
            Self::EnvVar(_) => "env",
        }
    }

    pub fn load(&self) -> Result<SigningKey, SigningKeyUnavailable> {
        let (pem, origin) = match self {
            Self::File(path) => (
                std::fs::read_to_string(path).map_err(|source| {
                    SigningKeyUnavailable::Unreadable {
                        path: path.clone(),
                        source,
                    }
                })?,
                format!("file {}", path.display()),
            ),
            Self::EnvVar(name) => (
                std::env::var(&name.0).map_err(|_| SigningKeyUnavailable::Unset(name.clone()))?,
                format!("environment variable {name}"),
            ),
        };
        SigningKey::from_pkcs8_pem(&pem)
            .map_err(|source| SigningKeyUnavailable::Malformed { origin, source })
    }
}
