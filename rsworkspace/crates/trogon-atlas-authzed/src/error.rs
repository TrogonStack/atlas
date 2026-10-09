use std::fmt;

/// Everything that can go wrong between this process and SpiceDB.
///
/// [`Malformed`](AuthzedError::Malformed) is separate from
/// [`Rpc`](AuthzedError::Rpc) on purpose: the first says a value never had a
/// chance of being accepted and is a bug here, the second says the remote
/// refused or was unreachable and may be worth retrying.
#[derive(Debug, thiserror::Error)]
pub enum AuthzedError {
    #[error("invalid {kind}: {reason}")]
    Malformed { kind: &'static str, reason: String },

    #[error("cannot reach SpiceDB at {endpoint}: {source}")]
    Connect {
        endpoint: String,
        #[source]
        source: tonic::transport::Error,
    },

    #[error("SpiceDB {operation} failed: {source}")]
    Rpc {
        operation: &'static str,
        #[source]
        source: Box<tonic::Status>,
    },

    /// A single lookup produced more resources than the configured ceiling.
    #[error(
        "SpiceDB returned more than {limit} resources for one lookup; refusing to buffer more"
    )]
    Overflow { limit: usize },

    /// SpiceDB answered, but with something the API says cannot happen, such
    /// as an unrecognised `Permissionship` or a response with no `ZedToken`.
    /// Treated as an error rather than defaulted, because every sensible
    /// default here is either a silent grant or a silent staleness.
    #[error("SpiceDB {operation} returned an uninterpretable response: {detail}")]
    Protocol {
        operation: &'static str,
        detail: String,
    },
}

impl AuthzedError {
    pub(crate) fn malformed(kind: &'static str, reason: impl fmt::Display) -> Self {
        Self::Malformed {
            kind,
            reason: reason.to_string(),
        }
    }

    pub(crate) fn rpc(operation: &'static str, source: tonic::Status) -> Self {
        Self::Rpc {
            operation,
            source: Box::new(source),
        }
    }

    pub(crate) fn protocol(operation: &'static str, detail: impl fmt::Display) -> Self {
        Self::Protocol {
            operation,
            detail: detail.to_string(),
        }
    }

    /// Whether a retry could plausibly succeed. Callers that fail closed use
    /// this to decide between refusing the request and refusing the process.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Connect { .. } => true,
            Self::Rpc { source, .. } => matches!(
                source.code(),
                tonic::Code::Unavailable
                    | tonic::Code::DeadlineExceeded
                    | tonic::Code::ResourceExhausted
                    | tonic::Code::Aborted
            ),
            Self::Malformed { .. } | Self::Protocol { .. } | Self::Overflow { .. } => false,
        }
    }
}
