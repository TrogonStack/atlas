//! TLS termination for the serving port.
//!
//! tonic's built-in TLS advertises only `h2` over ALPN, so rustls rejects
//! every client that offers ALPN without it, such as `curl --http1.1` or the
//! HTTP/1.1 probes and MCP clients the listener serves alongside gRPC. Atlas
//! terminates TLS itself and offers both protocols.

use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use tokio::{net::TcpStream, task::JoinSet};
use tokio_rustls::{
    rustls::{
        self,
        pki_types::{pem::PemObject as _, CertificateDer, PrivateKeyDer},
        ServerConfig,
    },
    server::TlsStream,
};
use tokio_stream::Stream;
use tonic::transport::server::TcpIncoming;

pub const ALPN_H2: &[u8] = b"h2";
pub const ALPN_HTTP_1_1: &[u8] = b"http/1.1";
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_PENDING_HANDSHAKES: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum TlsConfigError {
    #[error("parsing the certificate chain: {0}")]
    CertificateChain(rustls::pki_types::pem::Error),
    #[error("the certificate chain holds no certificate")]
    EmptyCertificateChain,
    #[error("parsing the private key: {0}")]
    PrivateKey(rustls::pki_types::pem::Error),
    #[error("building the TLS configuration: {0}")]
    Rustls(#[from] rustls::Error),
}

#[derive(Clone)]
pub struct TlsAcceptor {
    acceptor: tokio_rustls::TlsAcceptor,
    handshake_timeout: Duration,
}

impl TlsAcceptor {
    pub fn from_pem(cert_chain: &[u8], private_key: &[u8]) -> Result<Self, TlsConfigError> {
        let certs = CertificateDer::pem_slice_iter(cert_chain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(TlsConfigError::CertificateChain)?;
        if certs.is_empty() {
            return Err(TlsConfigError::EmptyCertificateChain);
        }
        let key = PrivateKeyDer::from_pem_slice(private_key).map_err(TlsConfigError::PrivateKey)?;
        let mut config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_single_cert(certs, key)?;
        config.alpn_protocols = vec![ALPN_H2.to_vec(), ALPN_HTTP_1_1.to_vec()];
        Ok(Self {
            acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(config)),
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        })
    }

    #[must_use]
    pub fn with_handshake_timeout(self, handshake_timeout: Duration) -> Self {
        Self {
            handshake_timeout,
            ..self
        }
    }

    pub fn incoming(&self, tcp: TcpIncoming) -> TlsIncoming {
        TlsIncoming {
            tcp,
            acceptor: self.clone(),
            tcp_done: false,
            handshakes: JoinSet::new(),
        }
    }

    async fn handshake(self, tcp: TcpStream) -> Option<TlsStream<TcpStream>> {
        let peer = tcp.peer_addr().ok();
        match tokio::time::timeout(self.handshake_timeout, self.acceptor.accept(tcp)).await {
            Ok(Ok(stream)) => Some(stream),
            Ok(Err(error)) => {
                tracing::debug!(?peer, %error, "TLS handshake failed");
                None
            }
            Err(_) => {
                tracing::debug!(
                    ?peer,
                    timeout = ?self.handshake_timeout,
                    "TLS handshake timed out"
                );
                None
            }
        }
    }
}

impl std::fmt::Debug for TlsAcceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsAcceptor")
            .field("handshake_timeout", &self.handshake_timeout)
            .finish_non_exhaustive()
    }
}

/// Accepted connections that completed the TLS handshake. Each handshake runs
/// on its own task, so a slow or failing client never holds up the next one.
/// At most [`MAX_PENDING_HANDSHAKES`] run at once, so clients that never finish
/// a handshake cannot exhaust sockets and memory.
pub struct TlsIncoming {
    tcp: TcpIncoming,
    tcp_done: bool,
    acceptor: TlsAcceptor,
    handshakes: JoinSet<Option<TlsStream<TcpStream>>>,
}

impl Stream for TlsIncoming {
    type Item = io::Result<TlsStream<TcpStream>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            while !this.tcp_done && this.handshakes.len() < MAX_PENDING_HANDSHAKES {
                match Pin::new(&mut this.tcp).poll_next(cx) {
                    Poll::Ready(Some(Ok(tcp))) => {
                        this.handshakes.spawn(this.acceptor.clone().handshake(tcp));
                    }
                    Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                    Poll::Ready(None) => this.tcp_done = true,
                    Poll::Pending => break,
                }
            }
            match this.handshakes.poll_join_next(cx) {
                Poll::Ready(Some(Ok(Some(stream)))) => return Poll::Ready(Some(Ok(stream))),
                Poll::Ready(Some(Ok(None))) => {}
                Poll::Ready(Some(Err(error))) => {
                    tracing::warn!(%error, "TLS handshake task failed");
                }
                Poll::Ready(None) if this.tcp_done => return Poll::Ready(None),
                Poll::Ready(None) | Poll::Pending => return Poll::Pending,
            }
        }
    }
}
