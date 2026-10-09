//! A gRPC server that answers `GetServerInfo` (and, when scripted,
//! `BatchGetEntities`) with fixed responses and records every call, so tests
//! can pose as a server of another version and prove a client refused before
//! sending a mutation, or that a read still works against it.

use std::{
    convert::Infallible,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
};

use tonic::{
    body::BoxBody,
    codegen::{http, Body, BoxFuture, Service, StdError},
};
use trogon_atlas_proto as pb;

const GET_SERVER_INFO_PATH: &str =
    "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetServerInfo";
const BATCH_GET_ENTITIES_PATH: &str =
    "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchGetEntities";

#[derive(Clone)]
struct DiscoveryOnlyService {
    info: pb::GetServerInfoResponse,
    entities: Option<pb::BatchGetEntitiesResponse>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl tonic::server::NamedService for DiscoveryOnlyService {
    const NAME: &'static str = "trogonatlas.api.eventmodel.v1alpha1.EventModelService";
}

impl<B> Service<http::Request<B>> for DiscoveryOnlyService
where
    B: Body + Send + 'static,
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<BoxBody>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        let path = req.uri().path().to_string();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.clone());
        match (path.as_str(), &self.entities) {
            (GET_SERVER_INFO_PATH, _) => {
                let info = self.info.clone();
                Box::pin(async move {
                    let mut grpc = tonic::server::Grpc::new(tonic::codec::ProstCodec::default());
                    Ok(grpc
                        .unary(Scripted::<pb::GetServerInfoRequest, _>::new(info), req)
                        .await)
                })
            }
            (BATCH_GET_ENTITIES_PATH, Some(entities)) => {
                let entities = entities.clone();
                Box::pin(async move {
                    let mut grpc = tonic::server::Grpc::new(tonic::codec::ProstCodec::default());
                    Ok(grpc
                        .unary(
                            Scripted::<pb::BatchGetEntitiesRequest, _>::new(entities),
                            req,
                        )
                        .await)
                })
            }
            _ => {
                let path = path.clone();
                Box::pin(async move { Ok(tonic::Status::unimplemented(path).into_http()) })
            }
        }
    }
}

struct Scripted<Req, Resp> {
    response: Resp,
    request: std::marker::PhantomData<fn(Req)>,
}

impl<Req, Resp> Scripted<Req, Resp> {
    fn new(response: Resp) -> Self {
        Self {
            response,
            request: std::marker::PhantomData,
        }
    }
}

impl<Req, Resp> tonic::server::UnaryService<Req> for Scripted<Req, Resp>
where
    Req: Send + 'static,
    Resp: Clone + Send + 'static,
{
    type Response = Resp;
    type Future = BoxFuture<tonic::Response<Self::Response>, tonic::Status>;

    fn call(&mut self, _req: tonic::Request<Req>) -> Self::Future {
        let response = self.response.clone();
        Box::pin(async move { Ok(tonic::Response::new(response)) })
    }
}

pub struct DiscoveryOnlyServer {
    pub endpoint: String,
    calls: Arc<Mutex<Vec<String>>>,
}

impl DiscoveryOnlyServer {
    /// Bind an ephemeral port and serve `info` from `GetServerInfo`. Every
    /// other RPC answers UNIMPLEMENTED and is recorded.
    pub async fn start(info: pb::GetServerInfoResponse) -> Result<Self, String> {
        Self::start_with_entities(info, None).await
    }

    /// Like [`Self::start`], but also answer `BatchGetEntities` with
    /// `entities`, so a read-only plan can run against the posed version.
    pub async fn start_serving_entities(
        info: pb::GetServerInfoResponse,
        entities: pb::BatchGetEntitiesResponse,
    ) -> Result<Self, String> {
        Self::start_with_entities(info, Some(entities)).await
    }

    async fn start_with_entities(
        info: pb::GetServerInfoResponse,
        entities: Option<pb::BatchGetEntitiesResponse>,
    ) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("bind: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("local_addr: {e}"))?;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let service = DiscoveryOnlyService {
            info,
            entities,
            calls: Arc::clone(&calls),
        };
        tokio::spawn(async move {
            let _ = tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await;
        });
        Ok(Self {
            endpoint: format!("http://{addr}"),
            calls,
        })
    }

    /// RPC paths received so far, in arrival order.
    pub fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// RPC paths received other than `GetServerInfo`.
    pub fn calls_other_than_discovery(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|path| path != GET_SERVER_INFO_PATH)
            .collect()
    }
}

/// Discovery response of a server built before contract revisions existed:
/// every pre-existing feature on, every gating flag and revision zero.
pub fn pre_revision_server_info() -> pb::GetServerInfoResponse {
    pb::GetServerInfoResponse {
        schema_version: pb::SCHEMA_VERSION.into(),
        server_version: "0.0.0-pre-revision".into(),
        features: Some(pb::get_server_info_response::Features {
            search: true,
            change_feed: true,
            infer_data_flow: true,
            check_information_completeness: true,
            diff_entities: true,
            impact_analysis: true,
            extract_subgraph: true,
            supersession_descendants: true,
            ..Default::default()
        }),
        limits: None,
        contract_revision: 0,
        min_client_contract_revision: 0,
        writer_status: None,
    }
}
