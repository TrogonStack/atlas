//! The TLS serving path accepts HTTP/1.1 and HTTP/2 on the same port,
//! whichever ALPN protocol the client offers, and a failed handshake never
//! stops the listener from taking the next connection.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    net::SocketAddr,
    sync::{Arc, LazyLock},
    time::Duration,
};

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{
    rustls::{
        self,
        pki_types::{pem::PemObject as _, CertificateDer, ServerName},
        ClientConfig, RootCertStore,
    },
    TlsConnector,
};
use tonic::transport::{server::TcpIncoming, Certificate, Channel, ClientTlsConfig, Server};
use tonic_health::pb::{
    health_check_response::ServingStatus, health_client::HealthClient, HealthCheckRequest,
};
use tower::ServiceExt as _;
use trogon_atlas_server::{
    health::{Health, StoreStatus, LIVENESS_PATH, READINESS_PATH},
    tls::{TlsAcceptor, ALPN_HTTP_1_1},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(500);

/// A throwaway CA and a `localhost` / `127.0.0.1` leaf it signed, generated
/// per test run so no private key lives in the repository.
struct TestPem {
    ca: String,
    server_cert: String,
    server_key: String,
}

static PEM: LazyLock<TestPem> = LazyLock::new(|| {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Atlas Test CA");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();

    let mut server_params =
        CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()]).unwrap();
    server_params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_key = KeyPair::generate().unwrap();
    let server = server_params.signed_by(&server_key, &ca).unwrap();

    TestPem {
        ca: ca.pem(),
        server_cert: server.pem(),
        server_key: server_key.serialize_pem(),
    }
});

struct TlsServer {
    addr: SocketAddr,
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl TlsServer {
    async fn start() -> Self {
        let health = Health::new().await;
        health.set_store(StoreStatus::Reachable).await;
        health.mark_live().await;

        let mut routes = tonic::service::Routes::builder();
        routes.add_service(health.grpc_service());
        let mut routes = routes.routes();
        let merged = routes.axum_router_mut().clone().merge(health.router());
        *routes.axum_router_mut() = merged;

        let acceptor = TlsAcceptor::from_pem(PEM.server_cert.as_bytes(), PEM.server_key.as_bytes())
            .unwrap()
            .with_handshake_timeout(HANDSHAKE_TIMEOUT);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = acceptor.incoming(TcpIncoming::from(listener));
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(
            Server::builder()
                .accept_http1(true)
                .add_routes(routes)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                }),
        );
        Self {
            addr,
            shutdown,
            task,
        }
    }

    async fn channel(&self) -> Channel {
        tonic::transport::Endpoint::from_shared(format!("https://{}", self.addr))
            .unwrap()
            .tls_config(
                ClientTlsConfig::new()
                    .ca_certificate(Certificate::from_pem(&PEM.ca))
                    .domain_name("localhost"),
            )
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    /// Sends one HTTP/1.1 request over TLS offering `alpn`, and returns the
    /// negotiated protocol and the response status code.
    async fn http1_get(&self, alpn: &[&[u8]], path: &str) -> (Option<Vec<u8>>, u16) {
        let mut roots = RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(PEM.ca.as_bytes()) {
            roots.add(cert.unwrap()).unwrap();
        }
        let mut config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();

        let tcp = TcpStream::connect(self.addr).await.unwrap();
        let mut tls = TlsConnector::from(Arc::new(config))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        let negotiated = tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
        tls.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
        let mut response = Vec::new();
        let _ = tls.read_to_end(&mut response).await;
        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 "),
            "an HTTP/1.1 response, got {response:?}"
        );
        let status = response
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("an HTTP status line");
        (negotiated, status)
    }

    async fn shut_down(self) {
        let _ = self.shutdown.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("the TLS listener drains on shutdown")
            .unwrap()
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn http1_client_offering_only_http1_alpn_is_served() {
    let server = TlsServer::start().await;
    assert_eq!(
        server.http1_get(&[ALPN_HTTP_1_1], LIVENESS_PATH).await,
        (Some(ALPN_HTTP_1_1.to_vec()), 200)
    );
    server.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn client_offering_no_alpn_is_served_over_http1() {
    let server = TlsServer::start().await;
    assert_eq!(server.http1_get(&[], LIVENESS_PATH).await, (None, 200));
    server.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn h2_client_is_served_over_http2() {
    let server = TlsServer::start().await;
    let request = http::Request::get(READINESS_PATH)
        .body(tonic::body::Body::empty())
        .unwrap();
    let response = server.channel().await.oneshot(request).await.unwrap();
    assert_eq!(response.status(), http::StatusCode::OK);
    assert_eq!(response.version(), http::Version::HTTP_2);
    server.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn grpc_health_check_works_over_tls() {
    let server = TlsServer::start().await;
    let status: ServingStatus = HealthClient::new(server.channel().await)
        .check(HealthCheckRequest {
            service: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .status
        .try_into()
        .unwrap();
    assert_eq!(status, ServingStatus::Serving);
    server.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_and_stalled_handshakes_do_not_stop_the_listener() {
    let server = TlsServer::start().await;

    let mut plaintext = TcpStream::connect(server.addr).await.unwrap();
    plaintext
        .write_all(b"GET /-/liveness HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut sink = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), plaintext.read_to_end(&mut sink))
        .await
        .expect("the server drops a client that does not speak TLS");

    let mut stalled = TcpStream::connect(server.addr).await.unwrap();
    let read = tokio::time::timeout(HANDSHAKE_TIMEOUT * 10, stalled.read_to_end(&mut Vec::new()))
        .await
        .expect("the server drops a client that stalls the handshake");
    assert!(matches!(read, Ok(0) | Err(_)));

    assert_eq!(
        server.http1_get(&[ALPN_HTTP_1_1], LIVENESS_PATH).await,
        (Some(ALPN_HTTP_1_1.to_vec()), 200),
    );
    server.shut_down().await;
}
