//! `/-/readiness` and `/-/liveness` and `grpc.health.v1.Health`, served on
//! the gRPC listener exactly as `main.rs` mounts them: outside the auth
//! stack, over HTTP/1.1 and HTTP/2, and always in agreement with each other.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use tokio::net::TcpListener;
use tonic::transport::{server::TcpIncoming, Channel, Server};
use tonic_health::pb::{
    health_check_response::ServingStatus, health_client::HealthClient, HealthCheckRequest,
};
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{BearerAuth, RegistryFile, TokenRegistry},
    health::{Health, StoreStatus, LIVENESS_PATH, READINESS_PATH},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const READER_TOKEN: &str = "health-reader-aaaa";

fn authenticated_registry() -> TokenRegistry {
    let file: RegistryFile = toml::from_str(&format!(
        "[principals.reader]\nrole = \"reader\"\ntokens = [\"{READER_TOKEN}\"]\n"
    ))
    .expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

struct HealthHttp {
    addr: std::net::SocketAddr,
    health: Health,
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl HealthHttp {
    async fn start() -> Self {
        let store = trogon_atlas_testsupport::shared().await.store().await;
        let service = EventModelServiceServer::new(
            EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap(),
        );
        let data_plane = data_plane(service, BearerAuth::new(authenticated_registry()));
        let health = Health::new().await;

        let mut routes = tonic::service::Routes::builder();
        routes.add_service(health.grpc_service());
        routes.add_service(data_plane);
        let mut routes = routes.routes();
        let merged = routes.axum_router_mut().clone().merge(health.router());
        *routes.axum_router_mut() = merged;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let incoming = TcpIncoming::from(listener);
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        health.mark_live().await;
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
            health,
            shutdown,
            task,
        }
    }

    async fn channel(&self) -> Channel {
        tonic::transport::Endpoint::from_shared(format!("http://{}", self.addr))
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    async fn get(&self, path_and_query: &str) -> (u16, String) {
        raw_http(
            self.addr,
            &format!(
                "GET {path_and_query} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            ),
        )
        .await
    }

    async fn grpc_status(&self, service: &str) -> ServingStatus {
        HealthClient::new(self.channel().await)
            .check(HealthCheckRequest {
                service: service.to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
            .status
            .try_into()
            .unwrap()
    }

    /// `""`, `readiness` and `liveness` over gRPC, next to what HTTP says.
    async fn assert_agree(&self, ready: bool, live: bool) {
        let expect = |ok: bool| {
            (
                if ok { 200 } else { 503 },
                if ok {
                    ServingStatus::Serving
                } else {
                    ServingStatus::NotServing
                },
            )
        };
        let (readiness_code, readiness_status) = expect(ready);
        let (liveness_code, liveness_status) = expect(live);
        assert_eq!(self.get(READINESS_PATH).await.0, readiness_code);
        assert_eq!(self.get(LIVENESS_PATH).await.0, liveness_code);
        assert_eq!(self.grpc_status("").await, readiness_status);
        assert_eq!(self.grpc_status("readiness").await, readiness_status);
        assert_eq!(self.grpc_status("liveness").await, liveness_status);
    }

    async fn shut_down(self) {
        let _ = self.shutdown.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("the gRPC listener drains on shutdown")
            .unwrap()
            .unwrap();
    }
}

/// Sends one raw HTTP/1.1 request and returns the status code and the body.
async fn raw_http(addr: std::net::SocketAddr, request: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("an HTTP status line");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .expect("an HTTP body");
    (status, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_follows_the_store_and_liveness_does_not() {
    let http = HealthHttp::start().await;

    http.assert_agree(false, true).await;
    assert_eq!(http.get(READINESS_PATH).await, (503, "not ready\n".into()));
    assert_eq!(http.get(LIVENESS_PATH).await, (200, "ok\n".into()));

    http.health.set_store(StoreStatus::Reachable).await;
    http.assert_agree(true, true).await;
    assert_eq!(http.get(READINESS_PATH).await, (200, "ok\n".into()));

    http.health.set_store(StoreStatus::Unreachable).await;
    http.assert_agree(false, true).await;

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn verbose_lists_each_check() {
    let http = HealthHttp::start().await;
    http.health.set_store(StoreStatus::Unreachable).await;

    assert_eq!(
        http.get(&format!("{READINESS_PATH}?verbose")).await,
        (
            503,
            "[+]ping ok\n[-]store failed\n[+]shutdown ok\nreadiness check failed\n".into()
        )
    );
    assert_eq!(
        http.get(&format!("{LIVENESS_PATH}?verbose")).await,
        (200, "[+]ping ok\nliveness check passed\n".into())
    );

    http.health.set_store(StoreStatus::Reachable).await;
    assert_eq!(
        http.get(&format!("{READINESS_PATH}?verbose")).await,
        (
            200,
            "[+]ping ok\n[+]store ok\n[+]shutdown ok\nreadiness check passed\n".into()
        )
    );

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_stops_readiness_but_not_liveness() {
    let http = HealthHttp::start().await;
    http.health.set_store(StoreStatus::Reachable).await;
    http.assert_agree(true, true).await;

    http.health.begin_shutdown().await;
    http.assert_agree(false, true).await;
    assert_eq!(
        http.get(&format!("{READINESS_PATH}?verbose")).await,
        (
            503,
            "[+]ping ok\n[+]store ok\n[-]shutdown failed\nreadiness check failed\n".into()
        )
    );

    http.health.set_store(StoreStatus::Reachable).await;
    http.assert_agree(false, true).await;

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn grpc_watch_on_the_empty_name_tracks_readiness() {
    let http = HealthHttp::start().await;
    let mut watch = HealthClient::new(http.channel().await)
        .watch(HealthCheckRequest {
            service: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut next = async || -> ServingStatus {
        tokio::time::timeout(Duration::from_secs(5), watch.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .status
            .try_into()
            .unwrap()
    };

    assert_eq!(next().await, ServingStatus::NotServing);
    http.health.set_store(StoreStatus::Reachable).await;
    assert_eq!(next().await, ServingStatus::Serving);
    http.health.begin_shutdown().await;
    assert_eq!(next().await, ServingStatus::NotServing);

    drop(watch);
    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_probe_names_are_registered() {
    let http = HealthHttp::start().await;
    let status = HealthClient::new(http.channel().await)
        .check(HealthCheckRequest {
            service: "trogonatlas.api.eventmodel.v1alpha1.EventModelService".to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn probes_need_no_bearer_token_while_the_data_plane_does() {
    let http = HealthHttp::start().await;
    http.health.set_store(StoreStatus::Reachable).await;

    http.assert_agree(true, true).await;
    let refused = EventModelServiceClient::new(http.channel().await)
        .list_entities(trogon_atlas_proto::ListEntitiesRequest::default())
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::Unauthenticated);

    http.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn probes_answer_over_http2() {
    use tower::ServiceExt;

    let http = HealthHttp::start().await;
    http.health.set_store(StoreStatus::Reachable).await;
    for (path, expected) in [
        (READINESS_PATH, http::StatusCode::OK),
        (LIVENESS_PATH, http::StatusCode::OK),
    ] {
        let request = http::Request::get(format!("http://{}{path}", http.addr))
            .body(tonic::body::Body::empty())
            .unwrap();
        let response = http.channel().await.oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected, "{path} over HTTP/2");
        assert_eq!(response.version(), http::Version::HTTP_2);
    }
    http.health.begin_shutdown().await;
    let request = http::Request::get(format!("http://{}{READINESS_PATH}", http.addr))
        .body(tonic::body::Body::empty())
        .unwrap();
    let response = http.channel().await.oneshot(request).await.unwrap();
    assert_eq!(response.status(), http::StatusCode::SERVICE_UNAVAILABLE);

    http.shut_down().await;
}
