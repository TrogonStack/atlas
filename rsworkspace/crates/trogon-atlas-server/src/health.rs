//! Readiness and liveness for every serving port, answered over plain HTTP at
//! `/-/readiness` and `/-/liveness` and over `grpc.health.v1.Health` as the
//! services `readiness`, `liveness` and `""` (readiness).
//!
//! Probes read held state and never do I/O: a background loop records the
//! store check, and the server records its own lifecycle. Both transports
//! read the same state, so they cannot disagree.

use std::{fmt::Write, sync::Arc};

use axum::{
    extract::{RawQuery, State},
    http::StatusCode,
    routing::get,
    Router,
};
use tokio::sync::Mutex;
use tonic_health::{
    pb::health_server::HealthServer,
    server::{HealthReporter, HealthService},
    ServingStatus,
};

pub const READINESS_PATH: &str = "/-/readiness";
pub const LIVENESS_PATH: &str = "/-/liveness";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Readiness,
    Liveness,
}

impl Probe {
    pub fn name(self) -> &'static str {
        match self {
            Self::Readiness => "readiness",
            Self::Liveness => "liveness",
        }
    }

    fn grpc_services(self) -> &'static [&'static str] {
        match self {
            Self::Readiness => &["", "readiness"],
            Self::Liveness => &["liveness"],
        }
    }

    fn failure(self) -> &'static str {
        match self {
            Self::Readiness => "not ready",
            Self::Liveness => "not live",
        }
    }
}

/// The outcome of the most recent store reachability check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreStatus {
    Unchecked,
    Reachable,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Starting,
    Running,
    ShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HealthState {
    lifecycle: Lifecycle,
    store: StoreStatus,
}

/// One named check a probe is made of, listed by `?verbose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub passed: bool,
}

impl HealthState {
    fn checks(self, probe: Probe) -> Vec<Check> {
        let ping = Check {
            name: "ping",
            passed: self.lifecycle != Lifecycle::Starting,
        };
        match probe {
            Probe::Liveness => vec![ping],
            Probe::Readiness => vec![
                ping,
                Check {
                    name: "store",
                    passed: self.store == StoreStatus::Reachable,
                },
                Check {
                    name: "shutdown",
                    passed: self.lifecycle != Lifecycle::ShuttingDown,
                },
            ],
        }
    }
}

#[derive(Debug)]
struct Inner {
    state: Mutex<HealthState>,
    reporter: HealthReporter,
}

/// The server's health, shared by the HTTP routes and the gRPC health service.
#[derive(Debug, Clone)]
pub struct Health {
    inner: Arc<Inner>,
}

impl Health {
    /// Starts not live and not ready, on both transports.
    pub async fn new() -> Self {
        let health = Self {
            inner: Arc::new(Inner {
                state: Mutex::new(HealthState {
                    lifecycle: Lifecycle::Starting,
                    store: StoreStatus::Unchecked,
                }),
                reporter: HealthReporter::new(),
            }),
        };
        health.update(|_| {}).await;
        health
    }

    pub async fn set_store(&self, store: StoreStatus) {
        self.update(|state| state.store = store).await;
    }

    /// The process has initialized and is about to serve.
    pub async fn mark_live(&self) {
        self.update(|state| {
            if state.lifecycle == Lifecycle::Starting {
                state.lifecycle = Lifecycle::Running;
            }
        })
        .await;
    }

    /// Stops advertising readiness for good, before in-flight requests drain.
    pub async fn begin_shutdown(&self) {
        self.update(|state| state.lifecycle = Lifecycle::ShuttingDown)
            .await;
    }

    pub async fn checks(&self, probe: Probe) -> Vec<Check> {
        self.inner.state.lock().await.checks(probe)
    }

    pub fn grpc_service(&self) -> HealthServer<HealthService> {
        HealthServer::new(HealthService::from_health_reporter(
            self.inner.reporter.clone(),
        ))
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route(READINESS_PATH, get(readiness))
            .route(LIVENESS_PATH, get(liveness))
            .with_state(self.clone())
    }

    async fn update(&self, change: impl FnOnce(&mut HealthState)) {
        let mut state = self.inner.state.lock().await;
        change(&mut state);
        for probe in [Probe::Readiness, Probe::Liveness] {
            let status = if state.checks(probe).iter().all(|check| check.passed) {
                ServingStatus::Serving
            } else {
                ServingStatus::NotServing
            };
            for service in probe.grpc_services() {
                self.inner
                    .reporter
                    .set_service_status(service, status)
                    .await;
            }
        }
    }
}

async fn readiness(
    State(health): State<Health>,
    RawQuery(query): RawQuery,
) -> (StatusCode, String) {
    respond(&health, Probe::Readiness, query.as_deref()).await
}

async fn liveness(State(health): State<Health>, RawQuery(query): RawQuery) -> (StatusCode, String) {
    respond(&health, Probe::Liveness, query.as_deref()).await
}

async fn respond(health: &Health, probe: Probe, query: Option<&str>) -> (StatusCode, String) {
    let checks = health.checks(probe).await;
    let passed = checks.iter().all(|check| check.passed);
    let status = if passed {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let verbose = query.is_some_and(|query| {
        query
            .split('&')
            .any(|pair| pair.split('=').next() == Some("verbose"))
    });
    if !verbose {
        let body = if passed { "ok" } else { probe.failure() };
        return (status, format!("{body}\n"));
    }
    let mut body = String::new();
    for check in &checks {
        let (mark, outcome) = if check.passed {
            ('+', "ok")
        } else {
            ('-', "failed")
        };
        let _ = writeln!(body, "[{mark}]{} {outcome}", check.name);
    }
    let outcome = if passed { "passed" } else { "failed" };
    let _ = writeln!(body, "{} check {outcome}", probe.name());
    (status, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn starts_neither_live_nor_ready() {
        let health = Health::new().await;
        assert_eq!(
            respond(&health, Probe::Liveness, None).await,
            (StatusCode::SERVICE_UNAVAILABLE, "not live\n".to_owned())
        );
        assert_eq!(
            respond(&health, Probe::Readiness, None).await,
            (StatusCode::SERVICE_UNAVAILABLE, "not ready\n".to_owned())
        );
    }

    #[tokio::test]
    async fn ready_only_once_live_and_store_reachable() {
        let health = Health::new().await;
        health.set_store(StoreStatus::Reachable).await;
        assert_eq!(
            respond(&health, Probe::Readiness, None).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        health.mark_live().await;
        assert_eq!(
            respond(&health, Probe::Readiness, None).await,
            (StatusCode::OK, "ok\n".to_owned())
        );
    }

    #[tokio::test]
    async fn shutdown_is_final() {
        let health = Health::new().await;
        health.set_store(StoreStatus::Reachable).await;
        health.begin_shutdown().await;
        health.mark_live().await;
        health.set_store(StoreStatus::Reachable).await;
        assert_eq!(
            respond(&health, Probe::Readiness, None).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            respond(&health, Probe::Liveness, None).await.0,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn verbose_lists_each_check() {
        let health = Health::new().await;
        health.mark_live().await;
        health.set_store(StoreStatus::Unreachable).await;
        assert_eq!(
            respond(&health, Probe::Readiness, Some("verbose")).await,
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "[+]ping ok\n[-]store failed\n[+]shutdown ok\nreadiness check failed\n".to_owned()
            )
        );
        assert_eq!(
            respond(&health, Probe::Liveness, Some("a=1&verbose=true")).await,
            (
                StatusCode::OK,
                "[+]ping ok\nliveness check passed\n".to_owned()
            )
        );
        assert_eq!(
            respond(&health, Probe::Liveness, Some("verbosely")).await,
            (StatusCode::OK, "ok\n".to_owned())
        );
    }
}
